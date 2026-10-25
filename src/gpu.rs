use std::sync::Arc;

use cudarc::driver::{CudaContext, CudaFunction, CudaModule, CudaSlice, CudaStream, CudaView, DevicePtr, DriverError, LaunchConfig, PushKernelArg};

macro_rules! klaunch {
    ($self:expr, $count:expr, $func:expr $(, $arg:expr)* $(,)?) => {{
        let count: u32 = $count;
        if count > 0 {
            unsafe {
                $self
                    .stream
                    .launch_builder($func)
                    $(.arg($arg))*
                    .launch(LaunchConfig::for_num_elems(count))
                    .map_err(map_gpu)?;
            }
        }
    }};
}
use cudarc::nvrtc::Ptx;
use dubdsr_kernels::filter::gaussian_kernel;

use crate::error::PipelineError;
use crate::pipeline::crop::CropBox;

pub fn probe_gpu() -> Result<(String, String), String> {
    let ctx = CudaContext::new(0).map_err(|err| err.to_string())?;
    let name = ctx.name().map_err(|err| err.to_string())?;
    let bytes = ctx.total_mem().map_err(|err| err.to_string())?;
    let gib = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    Ok((name, format!("{gib:.1}")))
}

pub struct Gpu {
    stream: Arc<CudaStream>,
    _module: Arc<CudaModule>,
    convolve_h: CudaFunction,
    convolve_v: CudaFunction,
    row_max: CudaFunction,
    threshold_u8: CudaFunction,
    row_moments: CudaFunction,
    masked_row_sum: CudaFunction,
    demosaic_u16: CudaFunction,
    f32_to_u16: CudaFunction,
    sub_f32: CudaFunction,
    crop_f32: CudaFunction,
    bilinear_accum: CudaFunction,
    kernel_fine: CudaSlice<f32>,
    fine_len: i32,
    kernel_wide: CudaSlice<f32>,
    wide_len: i32,
}

impl Gpu {
    pub fn new() -> Result<Self, PipelineError> {
        let ctx = CudaContext::new(0).map_err(map_gpu)?;
        let stream = ctx.default_stream();
        let ptx = Ptx::from_src(include_str!(env!("DUBDSR_PTX")));
        let module = ctx.load_module(ptx).map_err(map_gpu)?;
        let load = |name: &str| module.load_function(name).map_err(|err| PipelineError::with("error.gpu", format!("{name}: {err}")));
        let (kernel_fine, fine_len) = upload_kernel(&stream, 1.5)?;
        let (kernel_wide, wide_len) = upload_kernel(&stream, 6.0)?;
        Ok(Self {
            stream,
            convolve_h: load("convolve_h")?,
            convolve_v: load("convolve_v")?,
            row_max: load("row_max")?,
            threshold_u8: load("threshold_u8")?,
            row_moments: load("row_moments")?,
            masked_row_sum: load("masked_row_sum")?,
            demosaic_u16: load("demosaic_u16")?,
            f32_to_u16: load("f32_to_u16")?,
            sub_f32: load("sub_f32")?,
            crop_f32: load("crop_f32")?,
            bilinear_accum: load("bilinear_accum")?,
            _module: module,
            kernel_fine,
            fine_len,
            kernel_wide,
            wide_len,
        })
    }

    pub fn upload(&self, frames: &[f32]) -> Result<CudaSlice<f32>, PipelineError> {
        self.stream.clone_htod(frames).map_err(map_gpu)
    }

    pub fn free_bytes(&self) -> Result<usize, PipelineError> {
        self.stream.context().mem_get_info().map(|(free, _)| free).map_err(map_gpu)
    }

    pub fn download(&self, src: &CudaSlice<f32>, offset: usize, len: usize) -> Result<Vec<f32>, PipelineError> {
        let view = src.slice(offset..offset + len);
        pull(&self.stream, &view)
    }

    pub fn sharpness(&self, volume: &CudaSlice<f32>, count: usize, height: i32, width: i32) -> Result<Vec<f32>, PipelineError> {
        let hw = plane_len(height, width)?;
        let mut tmp = zeros::<f32>(&self.stream, hw)?;
        let mut blur = zeros::<f32>(&self.stream, hw)?;
        let mut high = zeros::<f32>(&self.stream, hw)?;
        let mut mask = zeros::<u8>(&self.stream, hw)?;
        let mut sums = zeros::<f32>(&self.stream, height as usize)?;
        let mut counts = zeros::<i32>(&self.stream, height as usize)?;
        let mut maxima = zeros::<f32>(&self.stream, height as usize)?;
        let mut columns = zeros::<i32>(&self.stream, height as usize)?;
        let mut scores = Vec::with_capacity(count);
        let n = hw as i32;
        let launch_n = hw as u32;
        for index in 0..count {
            let frame = volume.slice(index * hw..(index + 1) * hw);
            self.blur(&frame, &mut tmp, &mut blur, height, width, false)?;
            klaunch!(self, launch_n, &self.sub_f32, &frame, &blur, &mut high, &n);
            self.blur(&frame, &mut tmp, &mut blur, height, width, true)?;
            klaunch!(self, height as u32, &self.row_max, &blur, &height, &width, &mut maxima, &mut columns);
            let peak = pull(&self.stream, &maxima)?.into_iter().fold(0.0f32, f32::max);
            let score = if peak <= 0.0 {
                self.variance(&high, &mask, height, width, 0, &mut sums, &mut counts)?.0
            } else {
                let threshold = peak * 0.6;
                klaunch!(self, launch_n, &self.threshold_u8, &blur, &mut mask, &n, &threshold);
                let (value, kept) = self.variance(&high, &mask, height, width, 1, &mut sums, &mut counts)?;
                if kept > 16 {
                    value
                } else {
                    self.variance(&high, &mask, height, width, 0, &mut sums, &mut counts)?.0
                }
            };
            scores.push(score);
        }
        Ok(scores)
    }

    pub fn gather(&self, volume: &CudaSlice<f32>, indices: &[usize], src_h: i32, src_w: i32, crop: CropBox) -> Result<CudaSlice<f32>, PipelineError> {
        let crop_h = crop.y1 - crop.y0;
        let crop_w = crop.x1 - crop.x0;
        let crop_n = plane_len(crop_h, crop_w)?;
        let src_n = plane_len(src_h, src_w)?;
        let mut packed = zeros::<f32>(&self.stream, indices.len() * crop_n)?;
        let y0 = crop.y0;
        let x0 = crop.x0;
        for (slot, index) in indices.iter().enumerate() {
            let src = volume.slice(index * src_n..(index + 1) * src_n);
            let mut dst = packed.slice_mut(slot * crop_n..(slot + 1) * crop_n);
            klaunch!(self, (crop_n as u32).max(1), &self.crop_f32, &src, &mut dst, &src_w, &y0, &x0, &crop_h, &crop_w);
        }
        Ok(packed)
    }

    pub fn superres(
        &self,
        packed: &CudaSlice<f32>,
        count: usize,
        height: i32,
        width: i32,
        shifts: &[(f32, f32)],
        weights: &[f32],
        scale: i32,
        pattern: Option<i32>,
    ) -> Result<Vec<f32>, PipelineError> {
        if scale < 1 {
            return Err(PipelineError::new("error.scale_min"));
        }
        let hw = plane_len(height, width)?;
        let out_h = height.checked_mul(scale).ok_or_else(|| PipelineError::new("error.scale_min"))?;
        let out_w = width.checked_mul(scale).ok_or_else(|| PipelineError::new("error.scale_min"))?;
        let channels = if pattern.is_some() { 3 } else { 1 };
        let out_n = plane_len(out_h, out_w)?.checked_mul(channels).ok_or_else(|| PipelineError::new("error.vram"))?;
        let mut accum = zeros::<f32>(&self.stream, out_n)?;
        let mut raw16 = zeros::<u16>(&self.stream, hw)?;
        let mut rgb = if pattern.is_some() {
            Some(zeros::<f32>(&self.stream, hw * 3)?)
        } else {
            None
        };
        let normal = weights.iter().sum::<f32>().max(1.0e-8);
        let scale_f = scale as f32;
        let n = hw as i32;
        for index in 0..count {
            let frame = packed.slice(index * hw..(index + 1) * hw);
            let weight = weights.get(index).copied().unwrap_or(0.0) / normal;
            let (dy, dx) = shifts.get(index).copied().unwrap_or((0.0, 0.0));
            let pixels = (out_h as u32).saturating_mul(out_w as u32);
            if let Some(code) = pattern {
                klaunch!(self, hw as u32, &self.f32_to_u16, &frame, &mut raw16, &n);
                let color = rgb.as_mut().expect("tampon couleur");
                klaunch!(self, hw as u32, &self.demosaic_u16, &raw16, &mut *color, &height, &width, &code);
                klaunch!(
                    self,
                    pixels,
                    &self.bilinear_accum,
                    &*color,
                    &mut accum,
                    &height,
                    &width,
                    &out_h,
                    &out_w,
                    &scale_f,
                    &dy,
                    &dx,
                    &weight,
                    &channels
                );
            } else {
                klaunch!(
                    self,
                    pixels,
                    &self.bilinear_accum,
                    &frame,
                    &mut accum,
                    &height,
                    &width,
                    &out_h,
                    &out_w,
                    &scale_f,
                    &dy,
                    &dx,
                    &weight,
                    &channels
                );
            }
        }
        pull(&self.stream, &accum)
    }

    fn blur(&self, src: &CudaView<'_, f32>, tmp: &mut CudaSlice<f32>, dst: &mut CudaSlice<f32>, height: i32, width: i32, wide: bool) -> Result<(), PipelineError> {
        let n = (height as u32).saturating_mul(width as u32);
        let (kernel, klen) = if wide {
            (&self.kernel_wide, self.wide_len)
        } else {
            (&self.kernel_fine, self.fine_len)
        };
        klaunch!(self, n, &self.convolve_h, src, &mut *tmp, &height, &width, kernel, &klen);
        klaunch!(self, n, &self.convolve_v, &*tmp, dst, &height, &width, kernel, &klen);
        Ok(())
    }

    fn variance(
        &self,
        src: &CudaSlice<f32>,
        mask: &CudaSlice<u8>,
        height: i32,
        width: i32,
        use_mask: i32,
        sums: &mut CudaSlice<f32>,
        counts: &mut CudaSlice<i32>,
    ) -> Result<(f32, i32), PipelineError> {
        klaunch!(self, height as u32, &self.masked_row_sum, src, mask, &height, &width, &use_mask, &mut *sums, &mut *counts);
        let total: f32 = pull(&self.stream, &*sums)?.into_iter().sum();
        let count: i32 = pull(&self.stream, &*counts)?.into_iter().sum();
        if count <= 0 {
            return Ok((0.0, 0));
        }
        let mean = total / count as f32;
        klaunch!(self, height as u32, &self.row_moments, src, mask, &height, &width, &mean, &use_mask, &mut *sums, &mut *counts);
        let sumsq: f32 = pull(&self.stream, &*sums)?.into_iter().sum();
        Ok((sumsq / count as f32, count))
    }
}

fn upload_kernel(stream: &Arc<CudaStream>, sigma: f64) -> Result<(CudaSlice<f32>, i32), PipelineError> {
    let kernel: Vec<f32> = gaussian_kernel(sigma).into_iter().map(|value| value as f32).collect();
    let len = kernel.len() as i32;
    let device = stream.clone_htod(&kernel).map_err(map_gpu)?;
    Ok((device, len))
}

fn zeros<T: cudarc::driver::DeviceRepr + cudarc::driver::ValidAsZeroBits>(stream: &Arc<CudaStream>, len: usize) -> Result<CudaSlice<T>, PipelineError> {
    stream.alloc_zeros::<T>(len).map_err(map_gpu)
}

fn pull<T: cudarc::driver::DeviceRepr>(stream: &Arc<CudaStream>, src: &impl DevicePtr<T>) -> Result<Vec<T>, PipelineError> {
    let host = stream.clone_dtoh(src).map_err(map_gpu)?;
    stream.synchronize().map_err(map_gpu)?;
    Ok(host)
}

fn plane_len(height: i32, width: i32) -> Result<usize, PipelineError> {
    if height <= 0 || width <= 0 {
        return Err(PipelineError::new("error.no_frames"));
    }
    usize::try_from(height as u64 * width as u64).map_err(|_| PipelineError::with("error.gpu", "dimensions"))
}

fn map_gpu(err: DriverError) -> PipelineError {
    let text = err.to_string();
    let lower = text.to_ascii_lowercase();
    if lower.contains("memory") {
        PipelineError::with("error.vram", text)
    } else {
        PipelineError::with("error.gpu", text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::align::phase_shift;

    fn blob(height: usize, width: usize, cy: f32, cx: f32) -> Vec<f32> {
        let mut image = vec![0.0; height * width];
        for y in 0..height {
            for x in 0..width {
                let dy = y as f32 - cy;
                let dx = x as f32 - cx;
                image[y * width + x] = (-0.15 * (dy * dy + dx * dx)).exp();
            }
        }
        image
    }

    #[test]
    fn superres_stacks_a_shifted_blob_on_the_reference() {
        let gpu = match Gpu::new() {
            Ok(gpu) => gpu,
            Err(err) => {
                eprintln!("GPU ignoré: {err}");
                return;
            }
        };
        let height = 32usize;
        let width = 32usize;
        let reference = blob(height, width, 16.0, 16.0);
        let moved = blob(height, width, 21.0, 13.0);
        let (dy, dx) = phase_shift(&reference, &moved, height, width);
        assert!((dy - (-5.0)).abs() < 0.35, "dy={dy}");
        assert!((dx - 3.0).abs() < 0.35, "dx={dx}");
        let mut packed = reference.clone();
        packed.extend_from_slice(&moved);
        let device = gpu.upload(&packed).unwrap();
        let image = gpu
            .superres(&device, 2, height as i32, width as i32, &[(0.0, 0.0), (dy, dx)], &[1.0, 1.0], 1, None)
            .unwrap();
        let center = image[16 * width + 16];
        let stray = image[21 * width + 13];
        assert!(center > 0.7, "centre {center}");
        assert!(stray < 0.25, "fantôme {stray} dy={dy} dx={dx}");
    }
}

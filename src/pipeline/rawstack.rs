use std::path::{Path, PathBuf};

use crate::error::PipelineError;
use crate::gpu::{report_card, Gpu};

use super::align::PhaseReference;
use super::bayer::{normalize_pattern, pattern_code};
use super::budget::{blend_passes, packet_len, scale_average};
use super::crop::{crop_plane, detect_disc_crop, select_top_frames, CropBox};
use super::rawset::{decode_raw, list_raw_files};
use super::stacker::write_result;
use super::StackSettings;

pub fn stack_raw_folder(input: &Path, settings: &StackSettings, on_progress: &mut impl FnMut(&str, &[&str], f32)) -> Result<super::StackResult, PipelineError> {
    let files = list_raw_files(input)?;
    let files = window(&files, settings);
    if files.is_empty() {
        return Err(PipelineError::new("error.raw_empty"));
    }
    let frame_count = files.len();

    let gpu = Gpu::new()?;
    report_card(&gpu.info, on_progress, 0.0);
    on_progress("stage.raw_read", &["1", &frame_count.to_string()], 0.0);
    let mut scores = Vec::with_capacity(frame_count);
    let mut geometry: Option<(i32, i32, Option<String>)> = None;
    for (index, path) in files.iter().enumerate() {
        let frame = decode_raw(path)?;
        match &geometry {
            None => geometry = Some((frame.width, frame.height, frame.bayer.clone())),
            Some((width, height, bayer)) => {
                if frame.width != *width || frame.height != *height || frame.bayer != *bayer {
                    let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("raw");
                    return Err(PipelineError::with("error.raw_mismatch", name));
                }
            }
        }
        let device = gpu.upload(&frame.pixels)?;
        let score = gpu.sharpness(&device, 1, frame.height, frame.width)?;
        scores.push(score.first().copied().unwrap_or(0.0));
        drop(device);
        let done = (index + 1).to_string();
        let total = frame_count.to_string();
        on_progress("stage.raw_read", &[&done, &total], (index + 1) as f32 / frame_count as f32 * 0.25);
    }
    let (width, height, series_bayer) = geometry.ok_or_else(|| PipelineError::new("error.raw_empty"))?;

    on_progress("stage.frame_select", &[], 0.30);
    let selected = select_top_frames(&scores, settings.keep_ratio, 3);
    if selected.is_empty() {
        return Err(PipelineError::new("error.raw_empty"));
    }
    let chosen: Vec<(usize, f32)> = selected.iter().map(|&index| (index, scores[index])).collect();
    let reference_index = chosen[0].0;
    let reference = decode_raw(files[reference_index])?;

    on_progress("stage.crop", &[], 0.34);
    let crop = if settings.auto_crop {
        detect_disc_crop(&reference.pixels, height, width, settings.crop_margin_ratio)
    } else {
        CropBox {
            y0: 0,
            y1: height,
            x0: 0,
            x1: width,
        }
    };
    let crop_h = crop.y1 - crop.y0;
    let crop_w = crop.x1 - crop.x0;
    if crop_h <= 0 || crop_w <= 0 {
        return Err(PipelineError::new("error.no_frames"));
    }

    let (pattern, pattern_name) = resolve_pattern(settings, series_bayer.as_deref())?;
    let channels = if pattern.is_some() { 3 } else { 1 };
    let free = gpu.free_bytes()?;
    let packet = packet_len(free, crop_h, crop_w, settings.drizzle_scale, channels).ok_or_else(|| {
        let gib = free as f64 / (1024.0 * 1024.0 * 1024.0);
        PipelineError::with("error.vram", format!("{gib:.1} Go"))
    })?;

    let phase = PhaseReference::from_frame(&reference.pixels, height as usize, width as usize);
    let passes = chosen.len().div_ceil(packet);
    let out_h = (crop_h as usize) * (settings.drizzle_scale.max(1) as usize);
    let out_w = (crop_w as usize) * (settings.drizzle_scale.max(1) as usize);
    let mut accum = vec![0.0f32; out_h * out_w * channels];
    let mut total_weight = 0.0f32;

    for (pass, chunk) in chosen.chunks(packet).enumerate() {
        let pass_no = (pass + 1).to_string();
        let pass_count = passes.to_string();
        let fraction = 0.36 + 0.56 * ((pass as f32) / (passes as f32));
        on_progress("stage.raw_pass", &[&pass_no, &pass_count], fraction);
        let mut packed = Vec::new();
        let mut shifts = Vec::with_capacity(chunk.len());
        let mut weights = Vec::with_capacity(chunk.len());
        for &(index, score) in chunk {
            let owned = if index == reference_index { None } else { Some(decode_raw(files[index])?) };
            let frame = owned.as_ref().unwrap_or(&reference);
            if frame.width != width || frame.height != height || frame.bayer != series_bayer {
                let name = files[index].file_name().and_then(|value| value.to_str()).unwrap_or("raw");
                return Err(PipelineError::with("error.raw_mismatch", name));
            }
            let shift = if index == reference_index {
                (0.0, 0.0)
            } else {
                phase.shift_of(&frame.pixels)
            };
            packed.extend(crop_plane(&frame.pixels, width as usize, crop));
            shifts.push(shift);
            weights.push(score);
        }
        let device = gpu.upload(&packed)?;
        let average = gpu.superres(&device, chunk.len(), crop_h, crop_w, &shifts, &weights, settings.drizzle_scale, pattern)?;
        drop(device);
        let normal = weights.iter().copied().sum::<f32>().max(1.0e-8);
        blend_passes(&mut accum, &average, normal);
        total_weight += normal;
    }
    scale_average(&mut accum, total_weight);

    let output = stacked_dir_path(input);
    let result = write_result(
        output,
        settings,
        accum,
        out_h,
        out_w,
        pattern.is_some(),
        pattern_name,
        frame_count,
        chosen.len(),
    )?;
    on_progress("stage.done", &[], 1.0);
    Ok(result)
}

fn window<'a>(files: &'a [std::path::PathBuf], settings: &StackSettings) -> Vec<&'a Path> {
    if files.is_empty() {
        return Vec::new();
    }
    let last = settings.end_frame.unwrap_or(files.len() - 1).min(files.len() - 1);
    let start = settings.start_frame.min(last);
    let mut chosen: Vec<&Path> = files[start..=last].iter().map(PathBuf::as_path).collect();
    if settings.max_frames > 0 {
        chosen.truncate(settings.max_frames as usize);
    }
    chosen
}

fn resolve_pattern(settings: &StackSettings, series_bayer: Option<&str>) -> Result<(Option<i32>, Option<String>), PipelineError> {
    if !settings.debayer {
        return Ok((None, None));
    }
    let key = normalize_pattern(&settings.bayer_pattern);
    let name = if key == "AUTO" {
        match series_bayer {
            Some(name) => name.to_string(),
            None => return Ok((None, None)),
        }
    } else {
        key
    };
    let code = pattern_code(&name).ok_or_else(|| PipelineError::with("error.unknown_bayer", name.clone()))?;
    Ok((Some(code), Some(name)))
}

fn stacked_dir_path(dir: &Path) -> std::path::PathBuf {
    let stem = dir.file_name().and_then(|value| value.to_str()).unwrap_or("stacked");
    dir.join(format!("{stem}_stacked.tiff"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::stack_video;

    #[test]
    #[ignore]
    fn stack_photos_moon_cr3() {
        let dir = Path::new("photos");
        if !dir.join("_DR_0548.cr3").is_file() {
            return;
        }
        let mut settings = StackSettings::default();
        settings.keep_ratio = 0.10;
        settings.white_balance = true;
        settings.flatten_strength = 0.0;
        settings.debayer = true;
        settings.bayer_pattern = "AUTO".to_string();
        let result = stack_video(dir, &settings, |stage, args, fraction| {
            eprintln!("{fraction:.2} {stage} {}", args.join(" "));
        })
        .expect("empilement CR3");
        eprintln!(
            "{} {}x{} gardées {}/{} motif {:?}",
            result.output_path.display(),
            result.width,
            result.height,
            result.kept_frames,
            result.frame_count,
            result.bayer_pattern
        );
        assert!(result.output_path.is_file());
        assert!(result.kept_frames >= 3);
        assert_eq!(result.frame_count, 20);
    }
}

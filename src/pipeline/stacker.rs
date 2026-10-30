use std::path::{Path, PathBuf};

use crate::error::PipelineError;
use crate::gpu::{report_card, Gpu};

use super::align::align_frames;
use super::avi::iter_frames;
use super::bayer::{detect_bayer_pattern, gray_world, normalize_pattern, pattern_code};
use super::crop::{detect_disc_crop, select_top_frames, CropBox};
use super::rawstack::stack_raw_folder;
use super::tiff::write_tiff;
use super::tone::{normalize_mono, normalize_rgb, ToneSettings};

#[derive(Clone, Debug)]
pub struct StackSettings {
    pub keep_ratio: f32,
    pub debayer: bool,
    pub white_balance: bool,
    pub auto_crop: bool,
    pub crop_margin_ratio: f32,
    pub drizzle_scale: i32,
    pub bayer_pattern: String,
    pub black_point: f32,
    pub white_point: f32,
    pub gamma: f32,
    pub subtract_background: bool,
    pub solar_stretch: f32,
    pub sharpen_amount: f32,
    pub sharpen_radius: f32,
    pub flatten_strength: f32,
    pub max_frames: u32,
    pub start_frame: usize,
    pub end_frame: Option<usize>,
}

impl Default for StackSettings {
    fn default() -> Self {
        Self {
            keep_ratio: 0.10,
            debayer: true,
            white_balance: false,
            auto_crop: true,
            crop_margin_ratio: 0.05,
            drizzle_scale: 3,
            bayer_pattern: "AUTO".to_string(),
            black_point: 0.1,
            white_point: 100.0,
            gamma: 1.2,
            subtract_background: true,
            solar_stretch: 0.0,
            sharpen_amount: 2.0,
            sharpen_radius: 1.0,
            flatten_strength: 0.70,
            max_frames: 0,
            start_frame: 0,
            end_frame: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct StackResult {
    pub output_path: PathBuf,
    pub frame_count: usize,
    pub kept_frames: usize,
    pub width: usize,
    pub height: usize,
    pub rgb: bool,
    pub bayer_pattern: Option<String>,
    pub pixels: Vec<u16>,
}

pub fn stack_video(input: &Path, settings: &StackSettings, mut report: impl FnMut(&str, &[&str], f32)) -> Result<StackResult, PipelineError> {
    if input.is_dir() {
        return stack_raw_folder(input, settings, &mut report);
    }
    let (width, height, packed) = {
        let mut on_progress = |key: &str, fraction: f32| report(key, &[], fraction);
        on_progress("stage.read_avi", 0.0);
        let max_frames = if settings.max_frames == 0 { None } else { Some(settings.max_frames as usize) };
        let mut report_read = |stage: &str, fraction: f32| on_progress(stage, fraction * 0.25);
        iter_frames(input, max_frames, settings.start_frame, settings.end_frame, Some(&mut report_read))?
    };
    let width_i = width as i32;
    let height_i = height as i32;
    let frame_pixels = (width as usize).saturating_mul(height as usize);
    if frame_pixels == 0 || packed.len() % frame_pixels != 0 {
        return Err(PipelineError::new("error.no_frames"));
    }
    let frame_count = packed.len() / frame_pixels;

    report("stage.gpu_transfer", &[], 0.25);
    let gpu = Gpu::new()?;
    report_card(&gpu.info, &mut report, 0.25);
    let mut on_progress = |key: &str, fraction: f32| report(key, &[], fraction);
    let volume = gpu.upload(&packed)?;
    drop(packed);

    on_progress("stage.sharpness", 0.35);
    let scores = gpu.sharpness(&volume, frame_count, height_i, width_i)?;

    on_progress("stage.frame_select", 0.45);
    let selected = select_top_frames(&scores, settings.keep_ratio, 3);
    let selected_scores: Vec<f32> = selected.iter().map(|index| scores[*index]).collect();

    on_progress("stage.align", 0.55);
    let mut selected_frames = Vec::with_capacity(selected.len());
    for index in &selected {
        selected_frames.push(gpu.download(&volume, index * frame_pixels, frame_pixels)?);
    }
    let shifts = align_frames(&selected_frames, height as usize, width as usize, |done, total| {
        let fraction = 0.55 + 0.07 * (done as f32 / total.max(1) as f32);
        on_progress("stage.align", fraction.min(0.62));
    });

    on_progress("stage.crop", 0.62);
    let crop = if settings.auto_crop {
        detect_disc_crop(&selected_frames[0], height_i, width_i, settings.crop_margin_ratio)
    } else {
        CropBox {
            y0: 0,
            y1: height_i,
            x0: 0,
            x1: width_i,
        }
    };
    drop(selected_frames);
    let packed_gpu = gpu.gather(&volume, &selected, height_i, width_i, crop)?;
    drop(volume);

    let crop_h = crop.y1 - crop.y0;
    let crop_w = crop.x1 - crop.x0;
    on_progress("stage.superres", 0.72);
    let mut resolved = None;
    let pattern = if settings.debayer {
        let key = normalize_pattern(&settings.bayer_pattern);
        let name = if key == "AUTO" {
            on_progress("stage.bayer_detect", 0.68);
            let frame = gpu.download(&packed_gpu, 0, (crop_h as usize) * (crop_w as usize))?;
            detect_bayer_pattern(&frame, crop_h, crop_w)
        } else {
            key
        };
        let code = pattern_code(&name).ok_or_else(|| PipelineError::with("error.unknown_bayer", name.clone()))?;
        resolved = Some(name);
        Some(code)
    } else {
        None
    };

    let linear = gpu.superres(&packed_gpu, selected.len(), crop_h, crop_w, &shifts, &selected_scores, settings.drizzle_scale, pattern)?;
    drop(packed_gpu);

    let out_h = (crop_h as usize) * (settings.drizzle_scale.max(1) as usize);
    let out_w = (crop_w as usize) * (settings.drizzle_scale.max(1) as usize);
    let output_path = stacked_path(input);
    let result = write_result(output_path, settings, linear, out_h, out_w, settings.debayer, resolved, frame_count, selected.len())?;
    on_progress("stage.done", 1.0);
    Ok(result)
}

pub(crate) fn write_result(
    output_path: PathBuf,
    settings: &StackSettings,
    mut linear: Vec<f32>,
    out_h: usize,
    out_w: usize,
    rgb: bool,
    bayer_pattern: Option<String>,
    frame_count: usize,
    kept_frames: usize,
) -> Result<StackResult, PipelineError> {
    let tone = ToneSettings {
        black_point: settings.black_point,
        white_point: settings.white_point,
        gamma: settings.gamma,
        subtract_background: settings.subtract_background,
        solar_stretch: settings.solar_stretch,
        sharpen_amount: settings.sharpen_amount,
        sharpen_radius: settings.sharpen_radius,
        flatten_strength: settings.flatten_strength,
    };
    let pixels = if rgb {
        if settings.white_balance {
            gray_world(&mut linear);
        }
        normalize_rgb(&linear, out_h, out_w, &tone)
    } else {
        normalize_mono(&linear, out_h, out_w, &tone)
    };
    write_tiff(&output_path, out_w as u32, out_h as u32, rgb, &pixels)?;
    Ok(StackResult {
        output_path,
        frame_count,
        kept_frames,
        width: out_w,
        height: out_h,
        rgb,
        bayer_pattern,
        pixels,
    })
}

fn stacked_path(input: &Path) -> PathBuf {
    let stem = input.file_stem().and_then(|value| value.to_str()).unwrap_or("stacked");
    input.with_file_name(format!("{stem}_stacked.tiff"))
}

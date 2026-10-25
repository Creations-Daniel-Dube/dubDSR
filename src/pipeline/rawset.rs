use std::fs;
use std::path::{Path, PathBuf};

use jpeg_decoder::PixelFormat;

use super::bayer::pattern_code;
use crate::error::PipelineError;

const RAW_EXTENSIONS: &[&str] = &["cr2", "cr3", "nef", "nrw", "orf", "pef", "arw", "dng", "raf", "rw2"];

#[derive(Clone, Debug)]
pub struct RawFolder {
    pub files: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct RawFrame {
    pub width: i32,
    pub height: i32,
    pub bayer: Option<String>,
    pub pixels: Vec<f32>,
}

pub enum PreviewImage {
    Gray { width: usize, height: usize, pixels: Vec<u8> },
    Rgb { width: usize, height: usize, pixels: Vec<u8> },
}

impl RawFolder {
    pub fn open(dir: &Path) -> Result<Self, PipelineError> {
        Ok(Self {
            files: list_raw_files(dir)?,
        })
    }

    pub fn preview(&self, index: usize) -> Result<PreviewImage, PipelineError> {
        let path = self.files.get(index).ok_or_else(|| PipelineError::new("error.no_frames"))?;
        if let Some(image) = thumb_preview(path) {
            return Ok(image);
        }
        let frame = decode_raw(path)?;
        Ok(downscale_gray(&frame.pixels, frame.width as usize, frame.height as usize, 1280))
    }
}

pub fn list_raw_files(dir: &Path) -> Result<Vec<PathBuf>, PipelineError> {
    let entries = fs::read_dir(dir).map_err(|_| PipelineError::with("error.raw_open", dir.display().to_string()))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| PipelineError::with("error.raw_open", dir.display().to_string()))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
        if RAW_EXTENSIONS.contains(&ext.as_str()) {
            files.push(path);
        }
    }
    files.sort_by(|left, right| {
        let left = left.file_name().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
        let right = right.file_name().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
        left.cmp(&right)
    });
    Ok(files)
}

pub fn decode_raw(path: &Path) -> Result<RawFrame, PipelineError> {
    let guard = decode_buffer(path, true);
    if guard.0.status != 0 || guard.0.pixels.is_null() {
        return Err(raw_error(path, &guard.0));
    }
    let width = guard.0.width;
    let height = guard.0.height;
    let count = (width as usize).saturating_mul(height as usize);
    if width < 2 || height < 2 || count == 0 {
        return Err(PipelineError::with("error.raw_unsupported", file_label(path)));
    }
    let pixels = unsafe { std::slice::from_raw_parts(guard.0.pixels, count) }.to_vec();
    let bayer = if guard.0.bayer < 0 { None } else { pattern_name(guard.0.bayer).map(str::to_string) };
    if guard.0.bayer >= 0 && bayer.is_none() {
        return Err(PipelineError::with("error.raw_unsupported", file_label(path)));
    }
    let (pixels, width, height, bayer) = orient_mosaic(&pixels, width, height, guard.0.flip, bayer.as_deref())
        .map_err(|_| PipelineError::with("error.raw_unsupported", file_label(path)))?;
    Ok(RawFrame {
        width,
        height,
        bayer,
        pixels,
    })
}

pub fn orient_mosaic(pixels: &[f32], width: i32, height: i32, flip: i32, bayer: Option<&str>) -> Result<(Vec<f32>, i32, i32, Option<String>), ()> {
    let flip = if (0..8).contains(&flip) { flip } else { 0 };
    if width < 2 || height < 2 || pixels.len() != (width as usize) * (height as usize) {
        return Err(());
    }
    let out_w = if flip & 4 != 0 { height } else { width };
    let out_h = if flip & 4 != 0 { width } else { height };
    let mut out = vec![0.0f32; (out_w as usize) * (out_h as usize)];
    for row in 0..out_h {
        for col in 0..out_w {
            let (src_row, src_col) = source_xy(flip, width, height, row, col);
            out[(row * out_w + col) as usize] = pixels[(src_row * width + src_col) as usize];
        }
    }
    let pattern = match bayer {
        None => None,
        Some(name) => Some(pattern_after_flip(name, flip, width, height)?),
    };
    Ok((out, out_w, out_h, pattern))
}

fn pattern_after_flip(name: &str, flip: i32, width: i32, height: i32) -> Result<String, ()> {
    let mut chars = ['X'; 4];
    for (index, (row, col)) in [(0, 0), (0, 1), (1, 0), (1, 1)].into_iter().enumerate() {
        let (src_row, src_col) = source_xy(flip, width, height, row, col);
        chars[index] = color_at(name, src_row, src_col).ok_or(())?;
    }
    let text: String = chars.iter().collect();
    if pattern_code(&text).is_none() {
        return Err(());
    }
    Ok(text)
}

fn source_xy(flip: i32, width: i32, height: i32, row: i32, col: i32) -> (i32, i32) {
    let mut src_row = row;
    let mut src_col = col;
    if flip & 4 != 0 {
        std::mem::swap(&mut src_row, &mut src_col);
    }
    if flip & 2 != 0 {
        src_row = height - 1 - src_row;
    }
    if flip & 1 != 0 {
        src_col = width - 1 - src_col;
    }
    (src_row, src_col)
}

fn color_at(pattern: &str, row: i32, col: i32) -> Option<char> {
    let bytes = pattern.as_bytes();
    if bytes.len() != 4 {
        return None;
    }
    let index = (row.rem_euclid(2) * 2 + col.rem_euclid(2)) as usize;
    Some(bytes[index] as char)
}

fn pattern_name(code: i32) -> Option<&'static str> {
    match code {
        0 => Some("BGGR"),
        1 => Some("RGGB"),
        2 => Some("GRBG"),
        3 => Some("GBRG"),
        _ => None,
    }
}

fn thumb_preview(path: &Path) -> Option<PreviewImage> {
    let guard = decode_buffer(path, false);
    if guard.0.status != 0 || guard.0.thumb.is_null() || guard.0.thumb_len <= 0 {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(guard.0.thumb, guard.0.thumb_len as usize) };
    match guard.0.thumb_kind {
        1 => preview_jpeg(bytes),
        2 => Some(downscale_rgb(bytes, guard.0.thumb_w as usize, guard.0.thumb_h as usize, 1280)),
        3 => {
            let gray: Vec<f32> = bytes.iter().map(|value| f32::from(*value) * (65535.0 / 255.0)).collect();
            Some(downscale_gray(&gray, guard.0.thumb_w as usize, guard.0.thumb_h as usize, 1280))
        }
        _ => None,
    }
}

fn preview_jpeg(bytes: &[u8]) -> Option<PreviewImage> {
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    let width = info.width as usize;
    let height = info.height as usize;
    match info.pixel_format {
        PixelFormat::RGB24 if pixels.len() == width * height * 3 => Some(downscale_rgb(&pixels, width, height, 1280)),
        PixelFormat::L8 if pixels.len() == width * height => {
            let gray: Vec<f32> = pixels.iter().map(|value| f32::from(*value) * (65535.0 / 255.0)).collect();
            Some(downscale_gray(&gray, width, height, 1280))
        }
        _ => None,
    }
}

fn downscale_gray(pixels: &[f32], width: usize, height: usize, max_edge: usize) -> PreviewImage {
    let (out_w, out_h, step) = preview_grid(width, height, max_edge);
    let peak = pixels.iter().copied().fold(0.0f32, f32::max).max(1.0);
    let mut out = Vec::with_capacity(out_w * out_h);
    for row in 0..out_h {
        for col in 0..out_w {
            let value = pixels[(row * step) * width + col * step];
            out.push(((value / peak) * 255.0).clamp(0.0, 255.0) as u8);
        }
    }
    PreviewImage::Gray {
        width: out_w,
        height: out_h,
        pixels: out,
    }
}

fn downscale_rgb(pixels: &[u8], width: usize, height: usize, max_edge: usize) -> PreviewImage {
    let (out_w, out_h, step) = preview_grid(width, height, max_edge);
    let mut out = Vec::with_capacity(out_w * out_h * 3);
    for row in 0..out_h {
        for col in 0..out_w {
            let index = ((row * step) * width + col * step) * 3;
            out.extend_from_slice(&pixels[index..index + 3]);
        }
    }
    PreviewImage::Rgb {
        width: out_w,
        height: out_h,
        pixels: out,
    }
}

fn preview_grid(width: usize, height: usize, max_edge: usize) -> (usize, usize, usize) {
    let long = width.max(height).max(1);
    let step = (long / max_edge.max(1)).max(1);
    let out_w = (width / step).max(1);
    let out_h = (height / step).max(1);
    (out_w, out_h, step)
}

fn file_label(path: &Path) -> String {
    path.file_name().and_then(|value| value.to_str()).unwrap_or("raw").to_string()
}

fn raw_error(path: &Path, buffer: &MosaicBuffer) -> PipelineError {
    let mut label = file_label(path);
    let message = buffer.message.split(|byte| *byte == 0).next().unwrap_or(&[]);
    let text = String::from_utf8_lossy(message);
    let text = text.trim();
    if !text.is_empty() {
        label = format!("{label} ({text})");
    }
    let key = if buffer.status == 3 { "error.raw_unsupported" } else { "error.raw_open" };
    PipelineError::with(key, label)
}

#[repr(C)]
struct MosaicBuffer {
    width: i32,
    height: i32,
    bayer: i32,
    flip: i32,
    status: i32,
    pixels: *mut f32,
    thumb: *mut u8,
    thumb_len: i32,
    thumb_w: i32,
    thumb_h: i32,
    thumb_kind: i32,
    message: [u8; 192],
}

struct DecodeGuard(MosaicBuffer);

impl Drop for DecodeGuard {
    fn drop(&mut self) {
        unsafe { mosaic_free(&mut self.0) }
    }
}

fn decode_buffer(path: &Path, full: bool) -> DecodeGuard {
    let mut buffer = MosaicBuffer {
        width: 0,
        height: 0,
        bayer: -1,
        flip: 0,
        status: 1,
        pixels: std::ptr::null_mut(),
        thumb: std::ptr::null_mut(),
        thumb_len: 0,
        thumb_w: 0,
        thumb_h: 0,
        thumb_kind: 0,
        message: [0; 192],
    };
    unsafe {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            mosaic_decode_utf16(wide.as_ptr(), i32::from(full), &mut buffer);
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::ffi::OsStrExt;
            let mut bytes = path.as_os_str().as_bytes().to_vec();
            bytes.push(0);
            mosaic_decode_utf8(bytes.as_ptr().cast(), i32::from(full), &mut buffer);
        }
    }
    DecodeGuard(buffer)
}

unsafe extern "C" {
    #[cfg(not(windows))]
    fn mosaic_decode_utf8(path: *const u8, full: i32, out: *mut MosaicBuffer) -> i32;
    #[cfg(windows)]
    fn mosaic_decode_utf16(path: *const u16, full: i32, out: *mut MosaicBuffer) -> i32;
    fn mosaic_free(out: *mut MosaicBuffer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orient_keeps_rggb_without_rotation() {
        let pixels = vec![1.0, 2.0, 3.0, 4.0];
        let (out, width, height, pattern) = orient_mosaic(&pixels, 2, 2, 0, Some("RGGB")).unwrap();
        assert_eq!((width, height), (2, 2));
        assert_eq!(out, pixels);
        assert_eq!(pattern.as_deref(), Some("RGGB"));
    }

    #[test]
    fn half_turn_of_rggb_becomes_bggr() {
        let pixels = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let (out, width, height, pattern) = orient_mosaic(&pixels, 4, 2, 3, Some("RGGB")).unwrap();
        assert_eq!((width, height), (4, 2));
        assert_eq!(out[0], 8.0);
        assert_eq!(pattern.as_deref(), Some("BGGR"));
    }

    #[test]
    fn quarter_turn_of_rggb_becomes_grbg() {
        let pixels = vec![0.0; 16];
        let (_, _, _, pattern) = orient_mosaic(&pixels, 4, 4, 6, Some("RGGB")).unwrap();
        assert_eq!(pattern.as_deref(), Some("GRBG"));
    }

    #[test]
    fn list_keeps_only_raw_names_sorted() {
        let dir = std::env::temp_dir().join(format!("dubdsr-raw-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("b.CR3"), b"x").unwrap();
        fs::write(dir.join("a.nef"), b"x").unwrap();
        fs::write(dir.join("note.xmp"), b"x").unwrap();
        fs::write(dir.join("preview.jpg"), b"x").unwrap();
        let files = list_raw_files(&dir).unwrap();
        let names: Vec<_> = files.iter().map(|path| path.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["a.nef", "b.CR3"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_raw_reports_an_open_error() {
        let err = decode_raw(Path::new("dubdsr-missing-file.cr3")).unwrap_err();
        assert_eq!(err.key, "error.raw_open");
    }

    #[test]
    fn photos_cr3_decodes_a_bayer_mosaic() {
        let path = Path::new("photos/_DR_0548.cr3");
        if !path.is_file() {
            return;
        }
        let frame = decode_raw(path).expect("décodage CR3");
        assert!(frame.width > 1000 && frame.height > 1000, "{}x{}", frame.width, frame.height);
        assert_eq!(frame.pixels.len(), (frame.width as usize) * (frame.height as usize));
        let peak = frame.pixels.iter().copied().fold(0.0f32, f32::max);
        assert!(peak > 1000.0, "pic {peak} motif {:?}", frame.bayer);
        let corner = frame.pixels[0];
        eprintln!("CR3 {}x{} motif {:?} pic {peak:.0} coin {corner:.0}", frame.width, frame.height, frame.bayer);
    }
}

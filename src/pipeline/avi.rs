use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::error::PipelineError;

#[derive(Debug, Clone)]
pub struct VideoProbe {
    pub frame_count: usize,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone)]
struct FrameLoc {
    offset: u64,
    size: u32,
}

#[derive(Clone)]
pub struct AviVideo {
    path: std::path::PathBuf,
    width: u32,
    height: u32,
    fps: f64,
    bit_count: u16,
    #[allow(dead_code)]
    compression: u32,
    top_down: bool,
    stride: usize,
    frames: Vec<FrameLoc>,
}

impl AviVideo {
    pub fn open(path: &Path) -> Result<Self, PipelineError> {
        let mut file = File::open(path).map_err(|_| PipelineError::with("error.video_open", path.display().to_string()))?;
        let mut header = [0u8; 12];
        file.read_exact(&mut header)
            .map_err(|_| PipelineError::with("error.video_open", path.display().to_string()))?;
        if &header[0..4] != b"RIFF" || &header[8..12] != b"AVI " {
            return Err(PipelineError::with("error.unsupported_frame", "RIFF/AVI"));
        }

        let mut width = 0u32;
        let mut height = 0i32;
        let mut bit_count = 0u16;
        let mut compression = 0u32;
        let mut fps = 0.0f64;
        let mut frames = Vec::new();
        let file_len = file.seek(SeekFrom::End(0)).unwrap_or(0);
        file.seek(SeekFrom::Start(12)).ok();
        parse_lists(
            &mut file,
            12,
            file_len,
            &mut width,
            &mut height,
            &mut bit_count,
            &mut compression,
            &mut fps,
            &mut frames,
        )?;

        if width == 0 || height == 0 || frames.is_empty() {
            return Err(PipelineError::new("error.no_frames"));
        }
        if !supported_compression(compression, bit_count) {
            return Err(PipelineError::with(
                "error.unsupported_frame",
                format!("compression {compression:#x} {bit_count} bits"),
            ));
        }

        let top_down = height < 0;
        let height_abs = height.unsigned_abs();
        let stride = (((width as usize) * (bit_count as usize) + 31) / 32) * 4;
        Ok(Self {
            path: path.to_path_buf(),
            width,
            height: height_abs,
            fps,
            bit_count,
            compression,
            top_down,
            stride,
            frames,
        })
    }

    pub fn probe(&self) -> VideoProbe {
        VideoProbe {
            frame_count: self.frames.len(),
            fps: self.fps,
            width: self.width,
            height: self.height,
        }
    }

    pub fn read_frame(&self, index: usize) -> Result<Vec<f32>, PipelineError> {
        let loc = self
            .frames
            .get(index)
            .ok_or_else(|| PipelineError::new("error.no_frames"))?;
        let mut file = File::open(&self.path).map_err(|_| PipelineError::with("error.video_open", self.path.display().to_string()))?;
        file.seek(SeekFrom::Start(loc.offset))
            .map_err(|_| PipelineError::with("error.video_open", self.path.display().to_string()))?;
        let mut bytes = vec![0u8; loc.size as usize];
        file.read_exact(&mut bytes)
            .map_err(|_| PipelineError::with("error.video_open", self.path.display().to_string()))?;
        decode_frame(
            &bytes,
            self.width,
            self.height,
            self.bit_count,
            self.stride,
            self.top_down,
        )
    }
}

fn supported_compression(compression: u32, bit_count: u16) -> bool {
    let fourcc = compression.to_le_bytes();
    let text = [fourcc[0], fourcc[1], fourcc[2], fourcc[3]];
    if compression == 0 || compression == 3 {
        return matches!(bit_count, 8 | 16 | 24 | 48);
    }
    matches!(&text, b"Y800" | b"GREY" | b"Y8  " | b"Y16 " | b"DIB ")
}

fn parse_lists(
    file: &mut File,
    mut offset: u64,
    end: u64,
    width: &mut u32,
    height: &mut i32,
    bit_count: &mut u16,
    compression: &mut u32,
    fps: &mut f64,
    frames: &mut Vec<FrameLoc>,
) -> Result<(), PipelineError> {
    while offset + 8 <= end {
        file.seek(SeekFrom::Start(offset)).ok();
        let mut hdr = [0u8; 8];
        if file.read_exact(&mut hdr).is_err() {
            break;
        }
        let id = [hdr[0], hdr[1], hdr[2], hdr[3]];
        let size = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as u64;
        let data = offset + 8;
        let padded = size + (size & 1);
        if &id == b"LIST" {
            let mut kind = [0u8; 4];
            if file.seek(SeekFrom::Start(data)).is_err() || file.read_exact(&mut kind).is_err() {
                break;
            }
            if &kind == b"movi" {
                let movi_end = data.saturating_add(size).min(end);
                scan_movi(file, data + 4, movi_end, frames);
            } else {
                parse_lists(file, data + 4, data + size, width, height, bit_count, compression, fps, frames)?;
            }
        } else if &id == b"avih" && size >= 40 {
            let mut buf = vec![0u8; size as usize];
            file.seek(SeekFrom::Start(data)).ok();
            if file.read_exact(&mut buf).is_ok() {
                let usec = u32::from_le_bytes(buf[0..4].try_into().unwrap());
                if usec > 0 {
                    *fps = 1_000_000.0 / f64::from(usec);
                }
                if *width == 0 {
                    *width = u32::from_le_bytes(buf[32..36].try_into().unwrap());
                    *height = i32::from_le_bytes(buf[36..40].try_into().unwrap());
                }
            }
        } else if &id == b"strh" && size >= 48 {
            let mut buf = vec![0u8; size as usize];
            file.seek(SeekFrom::Start(data)).ok();
            if file.read_exact(&mut buf).is_ok() && &buf[0..4] == b"vids" {
                let scale = u32::from_le_bytes(buf[20..24].try_into().unwrap());
                let rate = u32::from_le_bytes(buf[24..28].try_into().unwrap());
                if scale > 0 && rate > 0 {
                    *fps = f64::from(rate) / f64::from(scale);
                }
            }
        } else if &id == b"RIFF" {
            // OpenDML : les images au-delà du premier giga-octet sont dans un RIFF AVIX.
            let mut kind = [0u8; 4];
            let child_end = data.saturating_add(size).min(end);
            if file.seek(SeekFrom::Start(data)).is_ok()
                && file.read_exact(&mut kind).is_ok()
                && (&kind == b"AVIX" || &kind == b"AVI ")
                && data + 4 < child_end
            {
                parse_lists(
                    file,
                    data + 4,
                    child_end,
                    width,
                    height,
                    bit_count,
                    compression,
                    fps,
                    frames,
                )?;
            }
        } else if &id == b"strf" && size >= 20 {
            let mut buf = vec![0u8; size as usize];
            file.seek(SeekFrom::Start(data)).ok();
            if file.read_exact(&mut buf).is_ok() {
                *width = i32::from_le_bytes(buf[4..8].try_into().unwrap()).unsigned_abs();
                *height = i32::from_le_bytes(buf[8..12].try_into().unwrap());
                *bit_count = u16::from_le_bytes(buf[14..16].try_into().unwrap());
                *compression = u32::from_le_bytes(buf[16..20].try_into().unwrap());
            }
        }
        let next = data.saturating_add(padded);
        if next <= offset {
            break;
        }
        offset = next;
    }
    Ok(())
}

fn scan_movi(file: &mut File, mut offset: u64, end: u64, frames: &mut Vec<FrameLoc>) {
    while offset + 8 <= end {
        file.seek(SeekFrom::Start(offset)).ok();
        let mut hdr = [0u8; 8];
        if file.read_exact(&mut hdr).is_err() {
            break;
        }
        let id = [hdr[0], hdr[1], hdr[2], hdr[3]];
        let size = u32::from_le_bytes(hdr[4..8].try_into().unwrap());
        let data = offset + 8;
        let is_frame = id[2] == b'd' && (id[3] == b'b' || id[3] == b'c');
        if is_frame && size > 0 {
            frames.push(FrameLoc {
                offset: data,
                size,
            });
        }
        let padded = u64::from(size) + (u64::from(size) & 1);
        offset = data + padded;
    }
}

fn decode_frame(bytes: &[u8], width: u32, height: u32, bit_count: u16, stride: usize, top_down: bool) -> Result<Vec<f32>, PipelineError> {
    let width = width as usize;
    let height = height as usize;
    let mut plane = vec![0.0f32; width * height];
    for row in 0..height {
        let src_row = if top_down { row } else { height - 1 - row };
        let start = src_row * stride;
        if start >= bytes.len() {
            return Err(PipelineError::with("error.unsupported_frame", format!("{width}x{height}x{bit_count}")));
        }
        let row_bytes = &bytes[start..];
        for x in 0..width {
            plane[row * width + x] = pixel_to_f32(row_bytes, x, bit_count)?;
        }
    }
    Ok(plane)
}

fn pixel_to_f32(row: &[u8], x: usize, bit_count: u16) -> Result<f32, PipelineError> {
    match bit_count {
        8 => {
            let i = x;
            if i >= row.len() {
                return Err(PipelineError::new("error.unsupported_frame"));
            }
            Ok(f32::from(row[i]) * (65535.0 / 255.0))
        }
        16 => {
            let i = x * 2;
            if i + 1 >= row.len() {
                return Err(PipelineError::new("error.unsupported_frame"));
            }
            Ok(f32::from(u16::from_le_bytes([row[i], row[i + 1]])))
        }
        24 => {
            let i = x * 3;
            if i + 2 >= row.len() {
                return Err(PipelineError::new("error.unsupported_frame"));
            }
            let b = u32::from(row[i]);
            let g = u32::from(row[i + 1]);
            let r = u32::from(row[i + 2]);
            if b == g && g == r {
                Ok(f32::from(row[i]) * (65535.0 / 255.0))
            } else {
                Ok(f32::from(gray_u8(r, g, b)) * (65535.0 / 255.0))
            }
        }
        48 => {
            let i = x * 6;
            if i + 5 >= row.len() {
                return Err(PipelineError::new("error.unsupported_frame"));
            }
            let b = u16::from_le_bytes([row[i], row[i + 1]]);
            let g = u16::from_le_bytes([row[i + 2], row[i + 3]]);
            let r = u16::from_le_bytes([row[i + 4], row[i + 5]]);
            if b == g && g == r {
                Ok(f32::from(b))
            } else {
                let y = (u32::from(r) * 9798 + u32::from(g) * 19235 + u32::from(b) * 3735 + 16384) >> 15;
                Ok(y as f32)
            }
        }
        _ => Err(PipelineError::with("error.unsupported_frame", bit_count.to_string())),
    }
}

fn gray_u8(r: u32, g: u32, b: u32) -> u8 {
    ((r * 9798 + g * 19235 + b * 3735 + 16384) >> 15) as u8
}

pub fn iter_frames(
    path: &Path,
    max_frames: Option<usize>,
    start_frame: usize,
    end_frame: Option<usize>,
    mut on_progress: Option<&mut dyn FnMut(&str, f32)>,
) -> Result<(u32, u32, Vec<f32>), PipelineError> {
    let video = AviVideo::open(path)?;
    let probe = video.probe();
    let last = end_frame
        .map(|end| end.min(probe.frame_count.saturating_sub(1)))
        .unwrap_or(probe.frame_count.saturating_sub(1));
    let start = start_frame.min(last);
    let span = last - start + 1;
    let mut frames = Vec::new();
    for index in start..=last {
        if max_frames.is_some_and(|limit| frames.len() >= limit) {
            break;
        }
        let frame = video.read_frame(index)?;
        frames.push(frame);
        if let Some(callback) = on_progress.as_mut() {
            if frames.len() % 25 == 0 {
                callback("stage.read_frames", (frames.len() as f32 / span as f32).min(1.0));
            }
        }
    }
    if frames.is_empty() {
        return Err(PipelineError::new("error.no_frames"));
    }
    if let Some(callback) = on_progress.as_mut() {
        callback("stage.read_done", 1.0);
    }
    let height = probe.height as usize;
    let width = probe.width as usize;
    let mut packed = Vec::with_capacity(frames.len() * height * width);
    for frame in frames {
        packed.extend_from_slice(&frame);
    }
    Ok((probe.width, probe.height, packed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn push_u32(buf: &mut Vec<u8>, value: u32) {
        buf.extend_from_slice(&value.to_le_bytes());
    }

    fn write_chunk(buf: &mut Vec<u8>, id: &[u8; 4], payload: &[u8]) {
        buf.extend_from_slice(id);
        push_u32(buf, payload.len() as u32);
        buf.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            buf.push(0);
        }
    }

    #[test]
    fn roundtrip_uncompressed_16bit_frame() {
        let width = 4u32;
        let height = 2u32;
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let value = (1000 + y * 10 + x) as u16;
                pixels.extend_from_slice(&value.to_le_bytes());
            }
        }
        let mut movi_payload = Vec::new();
        movi_payload.extend_from_slice(b"movi");
        let mut frame_chunk = Vec::new();
        write_chunk(&mut frame_chunk, b"00db", &pixels);
        movi_payload.extend_from_slice(&frame_chunk);

        let mut strf = Vec::new();
        push_u32(&mut strf, 40);
        push_u32(&mut strf, width);
        strf.extend_from_slice(&(-2i32).to_le_bytes());
        strf.extend_from_slice(&1u16.to_le_bytes());
        strf.extend_from_slice(&16u16.to_le_bytes());
        push_u32(&mut strf, 0);
        push_u32(&mut strf, pixels.len() as u32);
        push_u32(&mut strf, 0);
        push_u32(&mut strf, 0);
        push_u32(&mut strf, 0);
        push_u32(&mut strf, 0);

        let mut avih = vec![0u8; 56];
        avih[0..4].copy_from_slice(&40_000u32.to_le_bytes());
        avih[16..20].copy_from_slice(&1u32.to_le_bytes());
        avih[24..28].copy_from_slice(&1u32.to_le_bytes());
        avih[32..36].copy_from_slice(&width.to_le_bytes());
        avih[36..40].copy_from_slice(&(-2i32).to_le_bytes());

        let mut hdrl = Vec::new();
        hdrl.extend_from_slice(b"hdrl");
        write_chunk(&mut hdrl, b"avih", &avih);
        let mut strl = Vec::new();
        strl.extend_from_slice(b"strl");
        write_chunk(&mut strl, b"strf", &strf);
        write_chunk(&mut hdrl, b"LIST", &strl);

        let mut riff = Vec::new();
        riff.extend_from_slice(b"AVI ");
        write_chunk(&mut riff, b"LIST", &hdrl);
        write_chunk(&mut riff, b"LIST", &movi_payload);
        let mut file_bytes = Vec::new();
        write_chunk(&mut file_bytes, b"RIFF", &riff);

        let path = std::env::temp_dir().join("dubdsr-avi-test.avi");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(&file_bytes).unwrap();
        let video = AviVideo::open(&path).unwrap();
        assert_eq!(video.probe().frame_count, 1);
        assert_eq!(video.probe().width, 4);
        let frame = video.read_frame(0).unwrap();
        assert!((frame[0] - 1000.0).abs() < 1e-3);
        assert!((frame[4] - 1010.0).abs() < 1e-3);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn avix_frames_follow_the_first_riff() {
        let width = 2u32;
        let _height = 2u32;
        let mut pixels_a = Vec::new();
        let mut pixels_b = Vec::new();
        for value in [10u16, 20, 30, 40] {
            pixels_a.extend_from_slice(&value.to_le_bytes());
        }
        for value in [50u16, 60, 70, 80] {
            pixels_b.extend_from_slice(&value.to_le_bytes());
        }
        let mut movi_a = Vec::new();
        movi_a.extend_from_slice(b"movi");
        let mut chunk = Vec::new();
        write_chunk(&mut chunk, b"00db", &pixels_a);
        movi_a.extend_from_slice(&chunk);
        let mut movi_b = Vec::new();
        movi_b.extend_from_slice(b"movi");
        chunk.clear();
        write_chunk(&mut chunk, b"00db", &pixels_b);
        movi_b.extend_from_slice(&chunk);

        let mut strf = Vec::new();
        push_u32(&mut strf, 40);
        push_u32(&mut strf, width);
        strf.extend_from_slice(&(-2i32).to_le_bytes());
        strf.extend_from_slice(&1u16.to_le_bytes());
        strf.extend_from_slice(&16u16.to_le_bytes());
        push_u32(&mut strf, 0);
        push_u32(&mut strf, pixels_a.len() as u32);
        push_u32(&mut strf, 0);
        push_u32(&mut strf, 0);
        push_u32(&mut strf, 0);
        push_u32(&mut strf, 0);
        let mut avih = vec![0u8; 56];
        avih[16..20].copy_from_slice(&2u32.to_le_bytes());
        avih[32..36].copy_from_slice(&width.to_le_bytes());
        avih[36..40].copy_from_slice(&(-2i32).to_le_bytes());
        let mut hdrl = Vec::new();
        hdrl.extend_from_slice(b"hdrl");
        write_chunk(&mut hdrl, b"avih", &avih);
        let mut strl = Vec::new();
        strl.extend_from_slice(b"strl");
        write_chunk(&mut strl, b"strf", &strf);
        write_chunk(&mut hdrl, b"LIST", &strl);

        let mut riff = Vec::new();
        riff.extend_from_slice(b"AVI ");
        write_chunk(&mut riff, b"LIST", &hdrl);
        write_chunk(&mut riff, b"LIST", &movi_a);
        let mut file_bytes = Vec::new();
        write_chunk(&mut file_bytes, b"RIFF", &riff);
        let mut avix = Vec::new();
        avix.extend_from_slice(b"AVIX");
        write_chunk(&mut avix, b"LIST", &movi_b);
        write_chunk(&mut file_bytes, b"RIFF", &avix);

        let path = std::env::temp_dir().join("dubdsr-avix-test.avi");
        std::fs::write(&path, &file_bytes).unwrap();
        let video = AviVideo::open(&path).unwrap();
        assert_eq!(video.probe().frame_count, 2);
        let second = video.read_frame(1).unwrap();
        assert!((second[0] - 50.0).abs() < 1e-3);
        let _ = std::fs::remove_file(path);
    }
}

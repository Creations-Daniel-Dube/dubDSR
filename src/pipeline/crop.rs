#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CropBox {
    pub y0: i32,
    pub y1: i32,
    pub x0: i32,
    pub x1: i32,
}

pub fn detect_disc_crop(image: &[f32], height: i32, width: i32, margin_ratio: f32) -> CropBox {
    let full = CropBox {
        y0: 0,
        y1: height,
        x0: 0,
        x1: width,
    };
    let peak = image.iter().copied().fold(0.0f32, f32::max);
    if peak <= 0.0 {
        return full;
    }
    let threshold = peak * 0.08;
    let mut count = 0i32;
    let mut ys = Vec::new();
    let mut xs = Vec::new();
    for y in 0..height {
        for x in 0..width {
            if !above(image, height, width, y, x, threshold) {
                continue;
            }
            let connected = above(image, height, width, y - 1, x, threshold)
                || above(image, height, width, y + 1, x, threshold)
                || above(image, height, width, y, x - 1, threshold)
                || above(image, height, width, y, x + 1, threshold);
            if connected {
                count += 1;
                ys.push(y as f32);
                xs.push(x as f32);
            }
        }
    }
    if count < 16 {
        return full;
    }
    // Le centre de luminosité d'un croissant est décalé vers le limbe clair.
    // Un carré autour de ce centre coupe le bord opposé. On englobe toute
    // l'étendue du disque, sans les pixels chauds isolés.
    let y_lo = ys.iter().copied().fold(f32::MAX, f32::min);
    let y_hi = ys.iter().copied().fold(f32::MIN, f32::max);
    let x_lo = xs.iter().copied().fold(f32::MAX, f32::min);
    let x_hi = xs.iter().copied().fold(f32::MIN, f32::max);
    let cy = (y_lo + y_hi) * 0.5;
    let cx = (x_lo + x_hi) * 0.5;
    let half = (y_hi - y_lo).max(x_hi - x_lo) * 0.5 * (1.0 + margin_ratio.max(0.0));
    let y0 = (cy - half).floor().max(0.0) as i32;
    let y1 = (cy + half).ceil().min(height as f32) as i32;
    let x0 = (cx - half).floor().max(0.0) as i32;
    let x1 = (cx + half).ceil().min(width as f32) as i32;
    snap_even(
        CropBox {
            y0,
            y1,
            x0,
            x1,
        },
        height,
        width,
    )
}

fn above(image: &[f32], height: i32, width: i32, y: i32, x: i32, threshold: f32) -> bool {
    y >= 0 && x >= 0 && y < height && x < width && image[(y * width + x) as usize] > threshold
}

pub fn crop_plane(src: &[f32], width: usize, crop: CropBox) -> Vec<f32> {
    let out_h = (crop.y1 - crop.y0).max(0) as usize;
    let out_w = (crop.x1 - crop.x0).max(0) as usize;
    let mut out = Vec::with_capacity(out_h.saturating_mul(out_w));
    if width == 0 || out_w == 0 || out_h == 0 {
        return out;
    }
    for y in crop.y0..crop.y1 {
        let start = (y as usize).saturating_mul(width).saturating_add(crop.x0.max(0) as usize);
        let end = start.saturating_add(out_w);
        if end <= src.len() {
            out.extend_from_slice(&src[start..end]);
        }
    }
    out
}

pub fn snap_even(box_: CropBox, max_h: i32, max_w: i32) -> CropBox {
    let y0 = box_.y0 - (box_.y0.rem_euclid(2));
    let x0 = box_.x0 - (box_.x0.rem_euclid(2));
    let mut y1 = max_h.min(box_.y1 + (box_.y1.rem_euclid(2)));
    let mut x1 = max_w.min(box_.x1 + (box_.x1.rem_euclid(2)));
    if (y1 - y0).rem_euclid(2) != 0 {
        y1 -= 1;
    }
    if (x1 - x0).rem_euclid(2) != 0 {
        x1 -= 1;
    }
    if y1 <= y0 || x1 <= x0 {
        return CropBox {
            y0: 0,
            y1: max_h - max_h.rem_euclid(2),
            x0: 0,
            x1: max_w - max_w.rem_euclid(2),
        };
    }
    CropBox { y0, y1, x0, x1 }
}

pub fn percentile(values: &mut [f32], q: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if values.len() == 1 {
        return values[0];
    }
    let last = values.len() - 1;
    let pos = ((q / 100.0) * last as f32).clamp(0.0, last as f32);
    let lo = (pos.floor() as usize).min(last);
    let hi = (pos.ceil() as usize).min(last);
    let weight = pos - lo as f32;
    values[lo] * (1.0 - weight) + values[hi] * weight
}

pub fn select_top_frames(scores: &[f32], keep_ratio: f32, min_frames: usize) -> Vec<usize> {
    if scores.is_empty() {
        return Vec::new();
    }
    let ratio = keep_ratio.clamp(0.05, 1.0);
    let mut count = (scores.len() as f32 * ratio).ceil() as usize;
    count = count.max(min_frames).min(scores.len());
    let mut order: Vec<usize> = (0..scores.len()).collect();
    order.sort_by(|&a, &b| {
        scores[b]
            .partial_cmp(&scores[a])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(&b))
    });
    order.truncate(count);
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_at_100_stays_on_the_last_sample() {
        let mut values = vec![1.0f32; 16_777_220];
        for value in values.iter_mut().rev().take(32) {
            *value = 9.0;
        }
        let value = percentile(&mut values, 100.0);
        assert!((value - 9.0).abs() < 1e-4, "{value}");
    }

    #[test]
    fn percentile_is_linear() {
        let mut values = vec![1.0, 2.0, 3.0, 4.0];
        let value = percentile(&mut values, 98.0);
        assert!((value - 3.94).abs() < 1e-5);
    }

    #[test]
    fn snap_even_keeps_bayer_phase() {
        let snapped = snap_even(
            CropBox {
                y0: 3,
                y1: 7,
                x0: 1,
                x1: 6,
            },
            20,
            20,
        );
        assert_eq!(snapped.y0 % 2, 0);
        assert_eq!(snapped.x0 % 2, 0);
        assert_eq!((snapped.y1 - snapped.y0) % 2, 0);
        assert_eq!((snapped.x1 - snapped.x0) % 2, 0);
    }

    #[test]
    fn hot_pixel_does_not_stretch_the_crop() {
        let (height, width) = (200i32, 200i32);
        let mut image = vec![0.0f32; 200 * 200];
        image[0] = 65535.0;
        for y in 0..200 {
            for x in 0..200 {
                let dy = y as f32 - 100.0;
                let dx = x as f32 - 100.0;
                if dy * dy + dx * dx <= 40.0 * 40.0 {
                    image[y * 200 + x] = 20000.0;
                }
            }
        }
        let crop = detect_disc_crop(&image, height, width, 0.05);
        assert!(crop.y0 > 20, "y0={}", crop.y0);
        assert!(crop.x0 > 20, "x0={}", crop.x0);
    }

    #[test]
    fn full_disc_stays_inside_the_margin() {
        let (height, width) = (200i32, 200i32);
        let mut image = vec![0.0f32; 200 * 200];
        for y in 0..200 {
            for x in 0..200 {
                let dy = y as f32 - 100.0;
                let dx = x as f32 - 100.0;
                if dy * dy + dx * dx <= 60.0 * 60.0 {
                    image[y * 200 + x] = 1000.0;
                }
            }
        }
        let crop = detect_disc_crop(&image, height, width, 0.05);
        assert!(crop.y0 <= 40, "y0={}", crop.y0);
        assert!(crop.y1 >= 160, "y1={}", crop.y1);
        assert!(crop.x0 <= 40, "x0={}", crop.x0);
        assert!(crop.x1 >= 160, "x1={}", crop.x1);
    }

    #[test]
    fn crescent_keeps_the_upper_limb() {
        let (height, width) = (220i32, 220i32);
        let mut image = vec![0.0f32; 220 * 220];
        let mut y_min = height;
        let mut y_max = 0i32;
        for y in 0..220 {
            for x in 0..220 {
                let dy = y as f32 - 100.0;
                let dx = x as f32 - 130.0;
                if dy * dy + dx * dx <= 70.0 * 70.0 && x > 120 {
                    let weight = 0.2 + 0.8 * (y as f32 / 220.0);
                    image[(y * 220 + x) as usize] = 4000.0 * weight;
                    y_min = y_min.min(y);
                    y_max = y_max.max(y);
                }
            }
        }
        let crop = detect_disc_crop(&image, height, width, 0.05);
        assert!(crop.y0 <= y_min, "y0={} limbe={y_min}", crop.y0);
        assert!(crop.y1 >= y_max + 1, "y1={} limbe={y_max}", crop.y1);
    }

    #[test]
    fn select_keeps_the_sharpest_first() {
        let picked = select_top_frames(&[1.0, 5.0, 3.0, 4.0], 0.5, 3);
        assert_eq!(picked[0], 1);
        assert_eq!(picked.len(), 3);
    }
}

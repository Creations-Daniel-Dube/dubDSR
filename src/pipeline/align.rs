use dubdsr_kernels::complex::C32;
use dubdsr_kernels::fft::fft2;

#[cfg_attr(not(test), allow(dead_code))]
pub fn phase_shift(reference: &[f32], frame: &[f32], height: usize, width: usize) -> (f32, f32) {
    let mut reference = to_complex(reference);
    let mut frame = to_complex(frame);
    fft2(&mut reference, height, width, false);
    fft2(&mut frame, height, width, false);
    for (left, right) in reference.iter_mut().zip(&frame) {
        let value = left.mul(right.conj());
        let magnitude = value.norm().max(1.0e-8);
        *left = value.scale(1.0 / magnitude);
    }
    fft2(&mut reference, height, width, true);
    let corr: Vec<f32> = reference.iter().map(|value| value.re).collect();
    let (peak_y, peak_x) = argmax_2d(&corr, width as i32);
    refine_peak(&corr, height as i32, width as i32, peak_y, peak_x)
}

/// Spectre de la référence, calculé une fois et réutilisé d'un paquet à l'autre.
pub struct PhaseReference {
    spectrum: Vec<C32>,
    height: usize,
    width: usize,
}

impl PhaseReference {
    pub fn from_frame(frame: &[f32], height: usize, width: usize) -> Self {
        let mut spectrum = to_complex(frame);
        if height > 0 && width > 0 && frame.len() == height * width {
            fft2(&mut spectrum, height, width, false);
        }
        Self { spectrum, height, width }
    }

    pub fn shift_of(&self, frame: &[f32]) -> (f32, f32) {
        if self.height == 0 || self.width == 0 || frame.len() != self.height * self.width {
            return (0.0, 0.0);
        }
        correlate(&self.spectrum, frame, self.height, self.width)
    }
}

/// Décalages (dy, dx) de chaque image vers la première, qui reste à (0, 0).
/// La référence n'est transformée qu'une fois. `on_done(fait, total)` est appelé après chaque image.
pub fn align_frames(frames: &[Vec<f32>], height: usize, width: usize, mut on_done: impl FnMut(usize, usize)) -> Vec<(f32, f32)> {
    let total = frames.len();
    let mut shifts = vec![(0.0f32, 0.0f32); total];
    if total <= 1 || height == 0 || width == 0 {
        on_done(total, total.max(1));
        return shifts;
    }
    let reference = PhaseReference::from_frame(&frames[0], height, width);
    on_done(1, total);
    for (slot, frame) in frames.iter().enumerate().skip(1) {
        shifts[slot] = reference.shift_of(frame);
        on_done(slot + 1, total);
    }
    shifts
}

fn correlate(reference: &[C32], frame: &[f32], height: usize, width: usize) -> (f32, f32) {
    let mut image = to_complex(frame);
    fft2(&mut image, height, width, false);
    for (bin, right) in image.iter_mut().enumerate() {
        let value = reference[bin].mul(right.conj());
        let magnitude = value.norm().max(1.0e-8);
        *right = value.scale(1.0 / magnitude);
    }
    fft2(&mut image, height, width, true);
    let corr: Vec<f32> = image.iter().map(|value| value.re).collect();
    let (peak_y, peak_x) = argmax_2d(&corr, width as i32);
    refine_peak(&corr, height as i32, width as i32, peak_y, peak_x)
}

pub fn argmax_2d(values: &[f32], width: i32) -> (i32, i32) {
    let mut best_index = 0usize;
    let mut best = f32::NEG_INFINITY;
    for (index, value) in values.iter().enumerate() {
        if *value > best {
            best = *value;
            best_index = index;
        }
    }
    let width = width.max(1) as usize;
    ((best_index / width) as i32, (best_index % width) as i32)
}

pub fn refine_peak(corr: &[f32], height: i32, width: i32, peak_y: i32, peak_x: i32) -> (f32, f32) {
    let mut dy = peak_y;
    let mut dx = peak_x;
    if dy > height / 2 {
        dy -= height;
    }
    if dx > width / 2 {
        dx -= width;
    }
    let mut sub_y = dy as f32;
    let mut sub_x = dx as f32;
    if dy > 0 && dy < height - 1 {
        let column = dx.rem_euclid(width);
        let up = sample(corr, width, (dy - 1).rem_euclid(height), column);
        let center = sample(corr, width, dy.rem_euclid(height), column);
        let down = sample(corr, width, (dy + 1).rem_euclid(height), column);
        let denom = up - 2.0 * center + down;
        if denom.abs() > 1.0e-8 {
            sub_y += 0.5 * (up - down) / denom;
        }
    }
    if dx > 0 && dx < width - 1 {
        let row = dy.rem_euclid(height);
        let left = sample(corr, width, row, (dx - 1).rem_euclid(width));
        let center = sample(corr, width, row, dx.rem_euclid(width));
        let right = sample(corr, width, row, (dx + 1).rem_euclid(width));
        let denom = left - 2.0 * center + right;
        if denom.abs() > 1.0e-8 {
            sub_x += 0.5 * (left - right) / denom;
        }
    }
    (sub_y, sub_x)
}

fn sample(corr: &[f32], width: i32, y: i32, x: i32) -> f32 {
    corr[(y * width + x) as usize]
}

fn to_complex(values: &[f32]) -> Vec<C32> {
    let mean = values.iter().sum::<f32>() / values.len().max(1) as f32;
    values.iter().map(|value| C32::new(value - mean, 0.0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subpixel_skips_negative_peaks() {
        let corr = vec![0.0, 0.0, 0.0, 0.0, 5.0, 0.0, 0.0, 0.0, 0.0];
        let (dy, dx) = refine_peak(&corr, 3, 3, 2, 2);
        assert_eq!(dy, -1.0);
        assert_eq!(dx, -1.0);
    }

    #[test]
    fn phase_shift_finds_a_blob_offset() {
        let height = 32;
        let width = 32;
        let reference = blob(height, width, 12.0, 14.0);
        let frame = blob(height, width, 15.0, 12.0);
        let (dy, dx) = phase_shift(&reference, &frame, height, width);
        // Pic opposé au déplacement du contenu : l'empilement échantillonne y/scale - dy.
        assert!((dy - (-3.0)).abs() < 0.35, "dy={dy}");
        assert!((dx - 2.0).abs() < 0.35, "dx={dx}");
    }

    #[test]
    fn phase_shift_finds_the_offset_off_powers_of_two() {
        for (height, width, dy0, dx0) in [(45usize, 60, 4.0f32, -3.0), (135, 180, 7.0, 5.0), (270, 480, -6.0, 9.0), (40, 1080, 3.0, -8.0), (36, 1920, -4.0, 11.0)] {
            let reference = blob(height, width, height as f32 * 0.45, width as f32 * 0.40);
            let frame = blob(height, width, height as f32 * 0.45 + dy0, width as f32 * 0.40 + dx0);
            let (dy, dx) = phase_shift(&reference, &frame, height, width);
            assert!((dy - (-dy0)).abs() < 0.35, "{height}x{width} dy={dy} attendu {}", -dy0);
            assert!((dx - (-dx0)).abs() < 0.35, "{height}x{width} dx={dx} attendu {}", -dx0);
        }
    }

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
    fn align_frames_keeps_the_reference_and_the_offset() {
        let height = 45usize;
        let width = 60usize;
        let reference = blob(height, width, 18.0, 22.0);
        let moved = blob(height, width, 23.0, 19.0);
        let shifts = align_frames(&[reference, moved], height, width, |_, _| {});
        assert_eq!(shifts[0], (0.0, 0.0));
        assert!((shifts[1].0 - (-5.0)).abs() < 0.35, "dy={}", shifts[1].0);
        assert!((shifts[1].1 - 3.0).abs() < 0.35, "dx={}", shifts[1].1);
    }

    #[test]
    #[ignore]
    fn align_hd_pair_finds_the_shift() {
        let height = 1080usize;
        let width = 1920usize;
        let reference = blob(height, width, 400.0, 700.0);
        let frame = blob(height, width, 412.0, 688.0);
        let started = std::time::Instant::now();
        let (dy, dx) = phase_shift(&reference, &frame, height, width);
        let elapsed = started.elapsed();
        assert!((dy - (-12.0)).abs() < 0.35, "dy={dy}");
        assert!((dx - 12.0).abs() < 0.35, "dx={dx}");
        assert!(elapsed.as_secs() < 20, "{elapsed:?}");
        eprintln!("paire HD: {elapsed:?} dy={dy} dx={dx}");
    }
}

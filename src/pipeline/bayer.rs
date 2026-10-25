use dubdsr_kernels::demosaic::demosaic_image;

pub fn normalize_pattern(pattern: &str) -> String {
    match pattern.trim().to_ascii_uppercase().as_str() {
        "GRGB" => "GRBG".to_string(),
        "AUTO" => "AUTO".to_string(),
        other => other.to_string(),
    }
}

pub fn pattern_code(pattern: &str) -> Option<i32> {
    match pattern {
        "BGGR" => Some(0),
        "RGGB" => Some(1),
        "GRBG" => Some(2),
        "GBRG" => Some(3),
        _ => None,
    }
}

pub fn detect_bayer_pattern(raw: &[f32], height: i32, width: i32) -> String {
    let pixels: Vec<u16> = raw.iter().map(|value| value.clamp(0.0, 65535.0) as u16).collect();
    let mut best = "GRBG".to_string();
    let mut best_score = f32::INFINITY;
    for (name, code) in [("GRBG", 2), ("GBRG", 3), ("BGGR", 0), ("RGGB", 1)] {
        let rgb = demosaic_image(&pixels, height, width, code);
        let score = quality_score(&rgb, height, width);
        if score < best_score {
            best_score = score;
            best = name.to_string();
        }
    }
    best
}

fn quality_score(rgb: &[f32], height: i32, width: i32) -> f32 {
    let n = (height * width) as usize;
    if n == 0 || rgb.len() < n * 3 {
        return f32::INFINITY;
    }
    let mut lum = vec![0.0f32; n];
    for i in 0..n {
        lum[i] = (rgb[i * 3] + rgb[i * 3 + 1] + rgb[i * 3 + 2]) / 3.0;
    }
    let peak = lum.iter().copied().fold(0.0f32, f32::max);
    if peak <= 0.0 {
        return f32::INFINITY;
    }
    let mut mask = mask_over(&lum, peak * 0.12);
    if mask.iter().filter(|on| **on).count() < 64 {
        mask = mask_over(&lum, peak * 0.05);
    }
    if mask.iter().filter(|on| **on).count() < 16 {
        return f32::INFINITY;
    }
    let signal = masked_mean(&lum, &mask).max(1.0);
    let red = channel(rgb, 0);
    let green = channel(rgb, 1);
    let blue = channel(rgb, 2);
    let mut chroma_sum = 0.0f32;
    let mut chroma_count = 0.0f32;
    for i in 0..n {
        if mask[i] {
            chroma_sum += (red[i] - green[i]).abs() + (green[i] - blue[i]).abs();
            chroma_count += 1.0;
        }
    }
    let chroma = (chroma_sum / chroma_count.max(1.0)) / signal;
    let mut diff = vec![0.0f32; n];
    for i in 0..n {
        diff[i] = (red[i] - green[i]).abs();
    }
    let mut grid_sum = 0.0f32;
    let mut grid_count = 0.0f32;
    if height > 2 && width > 2 {
        for y in 1..height - 1 {
            for x in 1..width - 1 {
                if !mask[(y * width + x) as usize] {
                    continue;
                }
                let center = diff[(y * width + x) as usize];
                let lap = center
                    - 0.25
                        * (diff[((y - 1) * width + x) as usize]
                            + diff[((y + 1) * width + x) as usize]
                            + diff[(y * width + x - 1) as usize]
                            + diff[(y * width + x + 1) as usize]);
                grid_sum += lap.abs();
                grid_count += 1.0;
            }
        }
    }
    let grid = if grid_count > 0.0 { (grid_sum / grid_count) / signal } else { 0.0 };
    let means = [
        masked_mean(&red, &mask),
        masked_mean(&green, &mask),
        masked_mean(&blue, &mask),
    ];
    let mean = (means[0] + means[1] + means[2]) / 3.0;
    let variance = means.iter().map(|value| (value - mean).powi(2)).sum::<f32>() / 3.0;
    let cast = variance.sqrt() / mean.max(1.0);
    chroma + grid * 2.0 + cast * 0.5
}

fn channel(rgb: &[f32], which: usize) -> Vec<f32> {
    (0..rgb.len() / 3).map(|i| rgb[i * 3 + which]).collect()
}

fn mask_over(values: &[f32], threshold: f32) -> Vec<bool> {
    values.iter().map(|value| *value > threshold).collect()
}

fn masked_mean(values: &[f32], mask: &[bool]) -> f32 {
    let mut sum = 0.0f32;
    let mut count = 0.0f32;
    for (value, on) in values.iter().zip(mask) {
        if *on {
            sum += *value;
            count += 1.0;
        }
    }
    if count == 0.0 { 0.0 } else { sum / count }
}

pub fn gray_world(rgb: &mut [f32]) {
    let n = rgb.len() / 3;
    if n == 0 {
        return;
    }
    let lum: Vec<f32> = (0..n)
        .map(|i| (rgb[i * 3] + rgb[i * 3 + 1] + rgb[i * 3 + 2]) / 3.0)
        .collect();
    let peak = lum.iter().copied().fold(0.0f32, f32::max);
    let mut mask: Vec<bool> = lum.iter().map(|value| *value > peak * 0.05).collect();
    if mask.iter().filter(|on| **on).count() < 16 {
        mask = lum.iter().map(|value| *value > 0.0).collect();
    }
    let mut means = [0.0f32; 3];
    let mut count = 0.0f32;
    for i in 0..n {
        if mask[i] {
            count += 1.0;
            for channel in 0..3 {
                means[channel] += rgb[i * 3 + channel];
            }
        }
    }
    if count == 0.0 {
        return;
    }
    for mean in &mut means {
        *mean = (*mean / count).max(1.0e-6);
    }
    let target = (means[0] + means[1] + means[2]) / 3.0;
    let gains = [target / means[0], target / means[1], target / means[2]];
    for i in 0..n {
        for channel in 0..3 {
            rgb[i * 3 + channel] *= gains[channel];
        }
    }
}

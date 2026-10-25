use dubdsr_kernels::filter::gaussian_filter;

use super::crop::percentile;

pub struct ToneSettings {
    pub black_point: f32,
    pub white_point: f32,
    pub gamma: f32,
    pub subtract_background: bool,
    pub solar_stretch: f32,
    pub sharpen_amount: f32,
    pub sharpen_radius: f32,
    pub flatten_strength: f32,
}

pub fn normalize_rgb(rgb: &[f32], height: usize, width: usize, settings: &ToneSettings) -> Vec<u16> {
    let mut linear = rgb.to_vec();
    if settings.subtract_background {
        let mut lows = [0.0f32; 3];
        for channel in 0..3 {
            let plane = channel_of(&linear, channel);
            lows[channel] = percentile(&mut corners(&plane, height, width, 0.14), 90.0);
        }
        if settings.black_point > 0.0 {
            let mut lum = luminance(&linear);
            let floor = percentile(&mut lum, settings.black_point);
            for low in &mut lows {
                *low = low.max(floor);
            }
        }
        for i in 0..height * width {
            for channel in 0..3 {
                linear[i * 3 + channel] = (linear[i * 3 + channel] - lows[channel]).max(0.0);
            }
        }
    } else {
        let mut lum = luminance(&linear);
        let low = percentile(&mut lum, settings.black_point);
        for value in &mut linear {
            *value = (*value - low).max(0.0);
        }
    }
    let mask_lum = luminance(&linear);
    let flattened = settings.flatten_strength > 0.0;
    if flattened {
        linear = flatten_illumination(&linear, height, width, settings.flatten_strength);
    }
    let lum = luminance(&linear);
    let mut stretched = stretch(&linear, &lum, height, width, settings, flattened);
    stretched = finalize(&stretched, &mask_lum, height, width, settings);
    to_u16(&stretched)
}

pub fn normalize_mono(image: &[f32], height: usize, width: usize, settings: &ToneSettings) -> Vec<u16> {
    let mut linear = image.to_vec();
    if settings.subtract_background {
        let mut low = percentile(&mut corners(&linear, height, width, 0.14), 90.0);
        if settings.black_point > 0.0 {
            low = low.max(percentile(&mut linear.clone(), settings.black_point));
        }
        for value in &mut linear {
            *value = (*value - low).max(0.0);
        }
    } else {
        let low = percentile(&mut linear.clone(), settings.black_point);
        for value in &mut linear {
            *value = (*value - low).max(0.0);
        }
    }
    let mask_lum = linear.clone();
    let flattened = settings.flatten_strength > 0.0;
    if flattened {
        linear = flatten_mono(&linear, height, width, settings.flatten_strength);
    }
    let mut stretched = stretch_mono(&linear, height, width, settings, flattened);
    if settings.sharpen_amount > 0.0 {
        stretched = sharpen_mono(&stretched, height, width, settings.sharpen_amount, settings.sharpen_radius);
    }
    let disk = feathered_disk(&mask_lum);
    for (value, mask) in stretched.iter_mut().zip(&disk) {
        *value *= mask;
    }
    to_u16(&stretched)
}

fn stretch(linear: &[f32], luminance: &[f32], height: usize, width: usize, settings: &ToneSettings, prescaled: bool) -> Vec<f32> {
    let mut stretched = if prescaled {
        linear.iter().map(|value| value.clamp(0.0, 1.0)).collect()
    } else {
        let high = percentile(&mut luminance.to_vec(), settings.white_point);
        let span = high.max(1.0);
        if settings.solar_stretch > 0.0 {
            let base: Vec<f32> = linear.iter().map(|value| (value / span).clamp(0.0, 1.0) * 65535.0).collect();
            asinh_rgb(&base, settings.solar_stretch)
        } else {
            linear.iter().map(|value| (value / span).clamp(0.0, 1.0)).collect()
        }
    };
    apply_gamma(&mut stretched, settings.gamma);
    let _ = (height, width);
    stretched
}

fn stretch_mono(linear: &[f32], _height: usize, _width: usize, settings: &ToneSettings, prescaled: bool) -> Vec<f32> {
    let mut stretched = if prescaled {
        linear.iter().map(|value| value.clamp(0.0, 1.0)).collect()
    } else {
        let high = percentile(&mut linear.to_vec(), settings.white_point);
        let span = high.max(1.0);
        if settings.solar_stretch > 0.0 {
            let base: Vec<f32> = linear.iter().map(|value| (value / span).clamp(0.0, 1.0) * 65535.0).collect();
            asinh_mono(&base, settings.solar_stretch)
        } else {
            linear.iter().map(|value| (value / span).clamp(0.0, 1.0)).collect()
        }
    };
    apply_gamma(&mut stretched, settings.gamma);
    stretched
}

fn finalize(stretched: &[f32], mask_lum: &[f32], height: usize, width: usize, settings: &ToneSettings) -> Vec<f32> {
    let mut image = if settings.sharpen_amount > 0.0 {
        sharpen_rgb(stretched, height, width, settings.sharpen_amount, settings.sharpen_radius)
    } else {
        stretched.to_vec()
    };
    let disk = feathered_disk(mask_lum);
    for i in 0..height * width {
        for channel in 0..3 {
            image[i * 3 + channel] *= disk[i];
        }
    }
    image
}

fn apply_gamma(values: &mut [f32], gamma: f32) {
    if (gamma - 1.0).abs() > 1.0e-3 {
        let exponent = 1.0 / gamma;
        for value in values {
            *value = value.max(0.0).powf(exponent);
        }
    }
}

fn asinh_rgb(img: &[f32], stretch: f32) -> Vec<f32> {
    if stretch <= 0.0 {
        return img.to_vec();
    }
    let data: Vec<f32> = img.iter().map(|value| value.max(0.0)).collect();
    let lum = luminance(&data);
    let peak = lum.iter().copied().fold(0.0f32, f32::max);
    if peak <= 0.0 {
        return data;
    }
    let factor = stretch / peak;
    let reference = (peak * factor).asinh();
    let mut result = data;
    if reference > 0.0 {
        for i in 0..lum.len() {
            let compressed = (lum[i] * factor).asinh() / reference;
            let scale = compressed / lum[i].max(1.0e-6);
            for channel in 0..3 {
                result[i * 3 + channel] = (result[i * 3 + channel] * scale).clamp(0.0, 1.0);
            }
        }
    }
    result
}

fn asinh_mono(img: &[f32], stretch: f32) -> Vec<f32> {
    if stretch <= 0.0 {
        return img.to_vec();
    }
    let data: Vec<f32> = img.iter().map(|value| value.max(0.0)).collect();
    let peak = data.iter().copied().fold(0.0f32, f32::max);
    if peak <= 0.0 || (peak * stretch / peak).asinh() <= 0.0 {
        return data;
    }
    let reference = stretch.asinh();
    data.into_iter()
        .map(|value| {
            let compressed = (value / peak * stretch).asinh() / reference;
            let scale = compressed / value.max(1.0e-6);
            (value * scale).clamp(0.0, 1.0)
        })
        .collect()
}

fn sharpen_rgb(image: &[f32], height: usize, width: usize, amount: f32, radius: f32) -> Vec<f32> {
    if amount <= 0.0 || radius <= 0.0 {
        return image.to_vec();
    }
    let mut residual = luminance(image);
    let mut detail = vec![0.0f32; residual.len()];
    let mut sigma = f64::from(radius);
    for level in 0..4 {
        let blurred = gaussian_filter(&residual, height, width, sigma);
        let weight = amount * 0.6f32.powi(level);
        for i in 0..residual.len() {
            detail[i] += weight * (residual[i] - blurred[i]);
        }
        residual = blurred;
        sigma *= 2.0;
    }
    image
        .iter()
        .enumerate()
        .map(|(index, value)| (value + detail[index / 3]).clamp(0.0, 1.0))
        .collect()
}

fn sharpen_mono(image: &[f32], height: usize, width: usize, amount: f32, radius: f32) -> Vec<f32> {
    if amount <= 0.0 || radius <= 0.0 {
        return image.to_vec();
    }
    let mut residual = image.to_vec();
    let mut detail = vec![0.0f32; residual.len()];
    let mut sigma = f64::from(radius);
    for level in 0..4 {
        let blurred = gaussian_filter(&residual, height, width, sigma);
        let weight = amount * 0.6f32.powi(level);
        for i in 0..residual.len() {
            detail[i] += weight * (residual[i] - blurred[i]);
        }
        residual = blurred;
        sigma *= 2.0;
    }
    image.iter().zip(detail).map(|(value, extra)| (value + extra).clamp(0.0, 1.0)).collect()
}

fn flatten_illumination(linear: &[f32], height: usize, width: usize, strength: f32) -> Vec<f32> {
    let lum = luminance(linear);
    flatten_with_luminance(linear, &lum, height, width, strength, true)
}

fn flatten_mono(linear: &[f32], height: usize, width: usize, strength: f32) -> Vec<f32> {
    flatten_with_luminance(linear, linear, height, width, strength, false)
}

fn flatten_with_luminance(linear: &[f32], luminance: &[f32], height: usize, width: usize, strength: f32, rgb: bool) -> Vec<f32> {
    let peak = luminance.iter().copied().fold(0.0f32, f32::max);
    if peak <= 0.0 {
        return linear.to_vec();
    }
    let sigma = f64::from(height.max(width) as f32) * 0.16;
    let illum = gaussian_filter(luminance, height, width, sigma);
    let disk: Vec<f32> = illum
        .iter()
        .zip(luminance)
        .filter(|(_, value)| **value > peak * 0.05)
        .map(|(value, _)| *value)
        .collect();
    let target = if disk.is_empty() {
        illum.iter().copied().fold(0.0f32, f32::max)
    } else {
        percentile(&mut disk.clone(), 50.0)
    };
    if target <= 0.0 {
        return linear.to_vec();
    }
    let base = 0.55f32;
    let neutral = base / target;
    let cap = base / (target * 0.15);
    let channels = if rgb { 3 } else { 1 };
    let mut out = linear.to_vec();
    for i in 0..height * width {
        let denom = illum[i].max(target * 0.15);
        let flat = base / denom;
        let mut gain = neutral + strength * (flat - neutral);
        gain = gain.clamp(0.0, cap);
        for channel in 0..channels {
            out[i * channels + channel] *= gain;
        }
    }
    out
}

fn feathered_disk(luminance: &[f32]) -> Vec<f32> {
    let peak = luminance.iter().copied().fold(0.0f32, f32::max);
    if peak <= 0.0 {
        return vec![1.0; luminance.len()];
    }
    let threshold = peak * 0.04;
    luminance
        .iter()
        .map(|value| ((value - threshold * 0.5) / (threshold + 1.0e-6)).clamp(0.0, 1.0))
        .collect()
}

fn luminance(rgb: &[f32]) -> Vec<f32> {
    (0..rgb.len() / 3)
        .map(|i| (rgb[i * 3] + rgb[i * 3 + 1] + rgb[i * 3 + 2]) / 3.0)
        .collect()
}

fn channel_of(rgb: &[f32], channel: usize) -> Vec<f32> {
    (0..rgb.len() / 3).map(|i| rgb[i * 3 + channel]).collect()
}

fn corners(channel: &[f32], height: usize, width: usize, frac: f64) -> Vec<f32> {
    let ch = ((height as f64) * frac).max(1.0) as usize;
    let cw = ((width as f64) * frac).max(1.0) as usize;
    let mut values = Vec::new();
    for y in 0..ch {
        for x in 0..cw {
            values.push(channel[y * width + x]);
            values.push(channel[y * width + (width - cw + x)]);
            values.push(channel[(height - ch + y) * width + x]);
            values.push(channel[(height - ch + y) * width + (width - cw + x)]);
        }
    }
    values
}

fn to_u16(values: &[f32]) -> Vec<u16> {
    values.iter().map(|value| (value.clamp(0.0, 1.0) * 65535.0) as u16).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gamma_lifts_midtones() {
        let settings = ToneSettings {
            black_point: 0.0,
            white_point: 100.0,
            gamma: 2.0,
            subtract_background: false,
            solar_stretch: 0.0,
            sharpen_amount: 0.0,
            sharpen_radius: 1.0,
            flatten_strength: 0.0,
        };
        let image = vec![0.0f32, 32767.5, 65535.0];
        let out = normalize_mono(&image, 1, 3, &settings);
        assert!(out[1] > 40000);
        assert_eq!(out[2], 65535);
    }
}

/// Indice replié comme `scipy.ndimage` mode `reflect` (le bord est répété).
pub fn reflect_index(index: i32, length: i32) -> i32 {
    if length <= 1 {
        return 0;
    }
    let period = length * 2;
    let mut index = index % period;
    if index < 0 {
        index += period;
    }
    if index >= length {
        index = period - index - 1;
    }
    index
}

#[cfg(not(target_arch = "nvptx64"))]
pub fn gaussian_kernel(sigma: f64) -> Vec<f64> {
    if sigma <= 0.0 {
        return vec![1.0];
    }
    let radius = (4.0 * sigma + 0.5) as i32;
    let mut kernel = Vec::with_capacity((radius * 2 + 1) as usize);
    let mut sum = 0.0;
    for offset in -radius..=radius {
        let x = offset as f64;
        let weight = (-0.5 * (x / sigma) * (x / sigma)).exp();
        kernel.push(weight);
        sum += weight;
    }
    if sum != 0.0 {
        for weight in &mut kernel {
            *weight /= sum;
        }
    }
    kernel
}

#[cfg(not(target_arch = "nvptx64"))]
fn fill_rows(dst: &mut [f32], src: &[f32], y0: usize, rows: usize, width: usize, kernel: &[f64], horizontal: bool) {
    let radius = (kernel.len() / 2) as i32;
    let height = src.len() / width.max(1);
    for row in 0..rows {
        let y = (y0 + row) as i32;
        for x in 0..width as i32 {
            let mut acc = 0.0f64;
            for (k, weight) in kernel.iter().enumerate() {
                let delta = k as i32 - radius;
                let (sy, sx) = if horizontal {
                    (y, reflect_index(x + delta, width as i32))
                } else {
                    (reflect_index(y + delta, height as i32), x)
                };
                acc += *weight * f64::from(src[(sy as usize) * width + sx as usize]);
            }
            dst[row * width + x as usize] = acc as f32;
        }
    }
}

#[cfg(not(target_arch = "nvptx64"))]
fn convolve_axis(src: &[f32], height: usize, width: usize, kernel: &[f64], horizontal: bool) -> Vec<f32> {
    let mut dst = vec![0.0f32; height * width];
    let workers = std::thread::available_parallelism().map(|count| count.get()).unwrap_or(1);
    if workers <= 1 || height < 48 {
        fill_rows(&mut dst, src, 0, height, width, kernel, horizontal);
        return dst;
    }
    let rows_per = height.div_ceil(workers.min(height));
    let span = rows_per.saturating_mul(width).max(width);
    std::thread::scope(|scope| {
        for (band, chunk) in dst.chunks_mut(span).enumerate() {
            let y0 = band * rows_per;
            let rows = chunk.len() / width;
            scope.spawn(move || fill_rows(chunk, src, y0, rows, width, kernel, horizontal));
        }
    });
    dst
}

#[cfg(not(target_arch = "nvptx64"))]
fn gaussian_exact(image: &[f32], height: usize, width: usize, sigma: f64) -> Vec<f32> {
    let kernel = gaussian_kernel(sigma);
    if kernel.len() <= 1 {
        return image.to_vec();
    }
    let horizontal = convolve_axis(image, height, width, &kernel, true);
    convolve_axis(&horizontal, height, width, &kernel, false)
}

/// Le flou d'aplatissement a un rayon plus grand que l'image en Drizzle ×3.
/// On le calcule sur une image réduite, puis on le réétale : les fréquences
/// qu'un tel sigma efface ne sont pas dans la version réduite.
#[cfg(not(target_arch = "nvptx64"))]
fn gaussian_reduced(image: &[f32], height: usize, width: usize, sigma: f64) -> Vec<f32> {
    let mut factor = 1usize;
    let mut sigma_now = sigma;
    while sigma_now > 8.0 && height / (factor * 2) >= 8 && width / (factor * 2) >= 8 {
        factor *= 2;
        sigma_now = sigma / factor as f64;
    }
    if factor == 1 {
        return gaussian_exact(image, height, width, sigma);
    }
    let small_h = height.div_ceil(factor);
    let small_w = width.div_ceil(factor);
    let mut small = vec![0.0f64; small_h * small_w];
    let mut counts = vec![0u32; small_h * small_w];
    for y in 0..height {
        for x in 0..width {
            let index = (y / factor) * small_w + (x / factor);
            small[index] += f64::from(image[y * width + x]);
            counts[index] += 1;
        }
    }
    let small: Vec<f32> = small
        .iter()
        .zip(&counts)
        .map(|(sum, count)| if *count == 0 { 0.0 } else { (*sum / f64::from(*count)) as f32 })
        .collect();
    let blurred = gaussian_exact(&small, small_h, small_w, sigma_now);
    let mut out = vec![0.0f32; height * width];
    for y in 0..height {
        let sy = (y as f32 + 0.5) * small_h as f32 / height as f32 - 0.5;
        let y0 = sy.floor() as i32;
        let fy = sy - y0 as f32;
        for x in 0..width {
            let sx = (x as f32 + 0.5) * small_w as f32 / width as f32 - 0.5;
            let x0 = sx.floor() as i32;
            let fx = sx - x0 as f32;
            let sample = |iy: i32, ix: i32| {
                let iy = iy.clamp(0, small_h as i32 - 1) as usize;
                let ix = ix.clamp(0, small_w as i32 - 1) as usize;
                blurred[iy * small_w + ix]
            };
            let top = sample(y0, x0) * (1.0 - fx) + sample(y0, x0 + 1) * fx;
            let bottom = sample(y0 + 1, x0) * (1.0 - fx) + sample(y0 + 1, x0 + 1) * fx;
            out[y * width + x] = top * (1.0 - fy) + bottom * fy;
        }
    }
    out
}

#[cfg(not(target_arch = "nvptx64"))]
pub fn gaussian_filter(image: &[f32], height: usize, width: usize, sigma: f64) -> Vec<f32> {
    if height == 0 || width == 0 || image.len() < height * width {
        return image.to_vec();
    }
    if sigma <= 8.0 {
        gaussian_exact(image, height, width, sigma)
    } else {
        gaussian_reduced(image, height, width, sigma)
    }
}

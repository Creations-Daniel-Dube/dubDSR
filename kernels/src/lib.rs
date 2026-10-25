#![cfg_attr(target_arch = "nvptx64", no_std)]
#![cfg_attr(target_arch = "nvptx64", feature(abi_ptx, stdarch_nvptx))]

pub mod complex;
pub mod demosaic;
pub mod fft;
pub mod filter;
pub mod fmath;
pub mod sample;
pub mod trig;

#[cfg(target_arch = "nvptx64")]
mod device {
    use crate::complex::C32;
    use crate::demosaic::demosaic_pixel;
    use crate::fft::{fft_1d_raw, is_pow2};
    use crate::filter::reflect_index;
    use crate::sample::bilinear_strided;
    use core::arch::nvptx::{_block_dim_x, _block_idx_x, _thread_idx_x};

    fn global_x() -> u32 {
        let thread = unsafe { _thread_idx_x() };
        let block = unsafe { _block_idx_x() };
        let width = unsafe { _block_dim_x() };
        block.wrapping_mul(width).wrapping_add(thread)
    }

    unsafe fn f32_at(ptr: *const f32, index: i32) -> f32 {
        unsafe { *ptr.add(index as usize) }
    }

    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! {
        loop {}
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn convolve_h(
        src: *const f32,
        dst: *mut f32,
        height: i32,
        width: i32,
        kernel: *const f32,
        klen: i32,
    ) {
        let gid = global_x() as i32;
        let n = height.wrapping_mul(width);
        if gid >= n || width <= 0 {
            return;
        }
        let y = gid / width;
        let x = gid - y * width;
        let radius = klen / 2;
        let mut acc = 0.0f32;
        let mut k = 0i32;
        while k < klen {
            let sx = reflect_index(x + k - radius, width);
            let sample = unsafe { f32_at(src, y * width + sx) };
            let weight = unsafe { f32_at(kernel, k) };
            acc += weight * sample;
            k += 1;
        }
        unsafe {
            *dst.add(gid as usize) = acc;
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn convolve_v(
        src: *const f32,
        dst: *mut f32,
        height: i32,
        width: i32,
        kernel: *const f32,
        klen: i32,
    ) {
        let gid = global_x() as i32;
        let n = height.wrapping_mul(width);
        if gid >= n || width <= 0 || height <= 0 {
            return;
        }
        let y = gid / width;
        let x = gid - y * width;
        let radius = klen / 2;
        let mut acc = 0.0f32;
        let mut k = 0i32;
        while k < klen {
            let sy = reflect_index(y + k - radius, height);
            let sample = unsafe { f32_at(src, sy * width + x) };
            let weight = unsafe { f32_at(kernel, k) };
            acc += weight * sample;
            k += 1;
        }
        unsafe {
            *dst.add(gid as usize) = acc;
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn row_sum(src: *const f32, height: i32, width: i32, sums: *mut f32) {
        let row = global_x() as i32;
        if row >= height || width <= 0 {
            return;
        }
        let mut acc = 0.0f32;
        let mut x = 0i32;
        while x < width {
            acc += unsafe { f32_at(src, row * width + x) };
            x += 1;
        }
        unsafe {
            *sums.add(row as usize) = acc;
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn row_max(
        src: *const f32,
        height: i32,
        width: i32,
        max_out: *mut f32,
        col_out: *mut i32,
    ) {
        let row = global_x() as i32;
        if row >= height || width <= 0 {
            return;
        }
        let mut best = unsafe { f32_at(src, row * width) };
        let mut best_x = 0i32;
        let mut x = 1i32;
        while x < width {
            let value = unsafe { f32_at(src, row * width + x) };
            if value > best {
                best = value;
                best_x = x;
            }
            x += 1;
        }
        unsafe {
            *max_out.add(row as usize) = best;
            *col_out.add(row as usize) = best_x;
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn threshold_u8(src: *const f32, dst: *mut u8, n: i32, threshold: f32) {
        let gid = global_x() as i32;
        if gid >= n {
            return;
        }
        let value = unsafe { f32_at(src, gid) };
        unsafe {
            *dst.add(gid as usize) = if value > threshold { 1 } else { 0 };
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn row_moments(
        src: *const f32,
        mask: *const u8,
        height: i32,
        width: i32,
        mean: f32,
        use_mask: i32,
        sumsq: *mut f32,
        counts: *mut i32,
    ) {
        let row = global_x() as i32;
        if row >= height || width <= 0 {
            return;
        }
        let mut acc = 0.0f32;
        let mut count = 0i32;
        let mut x = 0i32;
        while x < width {
            let index = row * width + x;
            let keep = if use_mask == 0 {
                true
            } else {
                unsafe { *mask.add(index as usize) != 0 }
            };
            if keep {
                let delta = unsafe { f32_at(src, index) } - mean;
                acc += delta * delta;
                count += 1;
            }
            x += 1;
        }
        unsafe {
            *sumsq.add(row as usize) = acc;
            *counts.add(row as usize) = count;
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn masked_row_sum(
        src: *const f32,
        mask: *const u8,
        height: i32,
        width: i32,
        use_mask: i32,
        sums: *mut f32,
        counts: *mut i32,
    ) {
        let row = global_x() as i32;
        if row >= height || width <= 0 {
            return;
        }
        let mut acc = 0.0f32;
        let mut count = 0i32;
        let mut x = 0i32;
        while x < width {
            let index = row * width + x;
            let keep = if use_mask == 0 {
                true
            } else {
                unsafe { *mask.add(index as usize) != 0 }
            };
            if keep {
                acc += unsafe { f32_at(src, index) };
                count += 1;
            }
            x += 1;
        }
        unsafe {
            *sums.add(row as usize) = acc;
            *counts.add(row as usize) = count;
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn sub_mean_complex(src: *const f32, dst: *mut f32, n: i32, mean: f32) {
        let gid = global_x() as i32;
        if gid >= n {
            return;
        }
        unsafe {
            *dst.add((gid as usize) * 2) = f32_at(src, gid) - mean;
            *dst.add((gid as usize) * 2 + 1) = 0.0;
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn fft_rows(
        data: *mut f32,
        rows: i32,
        cols: i32,
        scratch: *mut f32,
        b_fft: *const f32,
        m: i32,
        inverse: i32,
    ) {
        let row = global_x() as i32;
        if row >= rows || cols <= 0 {
            return;
        }
        let n = cols as usize;
        let input = unsafe { data.add((row as usize) * n * 2) as *mut C32 };
        let scratch_row = unsafe { scratch.add((row as usize) * (m as usize) * 2) as *mut C32 };
        let b = b_fft as *const C32;
        if is_pow2(n) {
            unsafe {
                fft_1d_raw(input, n, input, scratch_row, b, m as usize, inverse != 0);
            }
        } else {
            let mut out_holder = input;
            unsafe {
                fft_1d_raw(input, n, out_holder, scratch_row, b, m as usize, inverse != 0);
            }
            let _ = out_holder;
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn transpose_complex(src: *const f32, dst: *mut f32, height: i32, width: i32) {
        let gid = global_x() as i32;
        let n = height.wrapping_mul(width);
        if gid >= n || width <= 0 {
            return;
        }
        let y = gid / width;
        let x = gid - y * width;
        let src_i = ((y * width + x) * 2) as usize;
        let dst_i = ((x * height + y) * 2) as usize;
        unsafe {
            *dst.add(dst_i) = *src.add(src_i);
            *dst.add(dst_i + 1) = *src.add(src_i + 1);
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn cross_power(a: *const f32, b: *const f32, dst: *mut f32, n: i32) {
        let gid = global_x() as i32;
        if gid >= n {
            return;
        }
        let i = (gid as usize) * 2;
        let ar = unsafe { *a.add(i) };
        let ai = unsafe { *a.add(i + 1) };
        let br = unsafe { *b.add(i) };
        let bi = unsafe { *b.add(i + 1) };
        let mut re = ar * br + ai * bi;
        let mut im = ai * br - ar * bi;
        let mag = crate::fmath::sqrt_f32(re * re + im * im);
        let denom = if mag > 1.0e-8 { mag } else { 1.0e-8 };
        re /= denom;
        im /= denom;
        unsafe {
            *dst.add(i) = re;
            *dst.add(i + 1) = im;
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn complex_real(src: *const f32, dst: *mut f32, n: i32) {
        let gid = global_x() as i32;
        if gid >= n {
            return;
        }
        unsafe {
            *dst.add(gid as usize) = *src.add((gid as usize) * 2);
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn demosaic_u16(
        raw: *const u16,
        dst: *mut f32,
        height: i32,
        width: i32,
        pattern: i32,
    ) {
        let gid = global_x() as i32;
        let n = height.wrapping_mul(width);
        if gid >= n || width <= 0 {
            return;
        }
        let y = gid / width;
        let x = gid - y * width;
        let (r, g, b) = unsafe { demosaic_pixel(raw, height, width, y, x, pattern) };
        let i = (gid as usize) * 3;
        unsafe {
            *dst.add(i) = r;
            *dst.add(i + 1) = g;
            *dst.add(i + 2) = b;
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn f32_to_u16(src: *const f32, dst: *mut u16, n: i32) {
        let gid = global_x() as i32;
        if gid >= n {
            return;
        }
        let mut value = unsafe { f32_at(src, gid) };
        if value < 0.0 {
            value = 0.0;
        }
        if value > 65535.0 {
            value = 65535.0;
        }
        unsafe {
            *dst.add(gid as usize) = value as u16;
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn sub_f32(a: *const f32, b: *const f32, dst: *mut f32, n: i32) {
        let gid = global_x() as i32;
        if gid >= n {
            return;
        }
        unsafe {
            *dst.add(gid as usize) = f32_at(a, gid) - f32_at(b, gid);
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn crop_f32(
        src: *const f32,
        dst: *mut f32,
        src_w: i32,
        y0: i32,
        x0: i32,
        crop_h: i32,
        crop_w: i32,
    ) {
        let gid = global_x() as i32;
        let n = crop_h.wrapping_mul(crop_w);
        if gid >= n || crop_w <= 0 {
            return;
        }
        let y = gid / crop_w;
        let x = gid - y * crop_w;
        let src_i = (y + y0) * src_w + (x + x0);
        unsafe {
            *dst.add(gid as usize) = f32_at(src, src_i);
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "ptx-kernel" fn bilinear_accum(
        src: *const f32,
        accum: *mut f32,
        height: i32,
        width: i32,
        out_h: i32,
        out_w: i32,
        scale: f32,
        dy: f32,
        dx: f32,
        weight: f32,
        channels: i32,
    ) {
        let gid = global_x() as i32;
        let n = out_h.wrapping_mul(out_w);
        if gid >= n || out_w <= 0 || scale == 0.0 {
            return;
        }
        let y = gid / out_w;
        let x = gid - y * out_w;
        let sy = (y as f32) / scale - dy;
        let sx = (x as f32) / scale - dx;
        let stride = if channels <= 1 { 1 } else { channels };
        let mut c = 0i32;
        while c < channels {
            let sample = unsafe { bilinear_strided(src.add(c as usize), height, width, stride, sy, sx) };
            let index = if channels <= 1 {
                gid as usize
            } else {
                (gid as usize) * (channels as usize) + c as usize
            };
            unsafe {
                *accum.add(index) += sample * weight;
            }
            c += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::complex::C32;
    use super::demosaic::demosaic_image;
    use super::fft::{fft2, fft_1d_slice};
    use super::filter::{gaussian_filter, gaussian_kernel, reflect_index};
    use super::sample::bilinear_strided;
    use super::trig::sin_approx;

    fn dft(input: &[C32], inverse: bool) -> Vec<C32> {
        let n = input.len();
        let mut out = vec![C32::zero(); n];
        for k in 0..n {
            let mut sum = C32::zero();
            for (n_i, value) in input.iter().enumerate() {
                let mut ang = core::f32::consts::PI * 2.0 * (k * n_i) as f32 / n as f32;
                if !inverse {
                    ang = -ang;
                }
                let twiddle = C32::new(ang.cos(), ang.sin());
                sum = sum.add(value.mul(twiddle));
            }
            if inverse && n > 0 {
                sum = sum.scale(1.0 / n as f32);
            }
            out[k] = sum;
        }
        out
    }

    fn assert_close(a: &[C32], b: &[C32]) {
        assert_eq!(a.len(), b.len());
        for (left, right) in a.iter().zip(b) {
            let dr = (left.re - right.re).abs();
            let di = (left.im - right.im).abs();
            assert!(dr < 2.0e-3 && di < 2.0e-3, "{left:?} != {right:?}");
        }
    }

    #[test]
    fn sqrt_matches_std() {
        for step in 1..200 {
            let x = step as f32 * 0.37;
            let err = (super::fmath::sqrt_f32(x) - x.sqrt()).abs();
            assert!(err < 1e-4, "x={x} err={err}");
        }
    }

    #[test]
    fn sin_matches_std_on_a_few_turns() {
        for step in 0..400 {
            let x = -20.0 + step as f32 * 0.1;
            let err = (sin_approx(x) - x.sin()).abs();
            assert!(err < 2.0e-5, "x={x} err={err}");
        }
    }

    #[test]
    fn fft_matches_dft_for_small_lengths() {
        for n in [1usize, 2, 3, 4, 5, 6, 7, 8, 15, 16] {
            let input: Vec<C32> = (0..n)
                .map(|i| C32::new(i as f32 + 0.25, (n - i) as f32 * 0.5))
                .collect();
            let mut got = input.clone();
            fft_1d_slice(&mut got, false);
            assert_close(&got, &dft(&input, false));
            fft_1d_slice(&mut got, true);
            assert_close(&got, &input);
        }
    }

    #[test]
    fn fft2_roundtrip() {
        let h = 5;
        let w = 6;
        let mut data: Vec<C32> = (0..h * w).map(|i| C32::new(i as f32, 0.0)).collect();
        let original = data.clone();
        fft2(&mut data, h, w, false);
        fft2(&mut data, h, w, true);
        assert_close(&data, &original);
    }

    #[test]
    fn fft_matches_dft_at_video_lengths() {
        for n in [48usize, 96, 192, 384, 1080, 1920] {
            let input: Vec<C32> = (0..n).map(|i| C32::new(((i * 17) % 23) as f32 - 8.0, 0.0)).collect();
            let mut got = input.clone();
            fft_1d_slice(&mut got, false);
            let expect = dft(&input, false);
            let mut worst = 0.0f32;
            let mut peak = 0.0f32;
            for (left, right) in got.iter().zip(&expect) {
                worst = worst.max((left.re - right.re).abs()).max((left.im - right.im).abs());
                peak = peak.max(right.re.abs()).max(right.im.abs());
            }
            assert!(worst <= peak * 5.0e-4 + 0.02, "n={n} pire écart {worst} pic {peak}");
            fft_1d_slice(&mut got, true);
            let mut back = 0.0f32;
            for (left, right) in got.iter().zip(&input) {
                back = back.max((left.re - right.re).abs()).max((left.im - right.im).abs());
            }
            assert!(back < 0.05, "n={n} retour {back}");
        }
    }

    #[test]
    fn reflect_repeats_the_edge() {
        let n = 4;
        let values = [1, 2, 3, 4];
        let mut padded = Vec::new();
        for index in -6..10 {
            padded.push(values[reflect_index(index, n) as usize]);
        }
        assert_eq!(padded, vec![3, 4, 4, 3, 2, 1, 1, 2, 3, 4, 4, 3, 2, 1, 1, 2]);
    }

    #[test]
    fn gaussian_kernel_sigma_1_5_sums_to_one() {
        let kernel = gaussian_kernel(1.5);
        assert_eq!(kernel.len(), 13);
        let sum: f64 = kernel.iter().sum();
        assert!((sum - 1.0).abs() < 1e-12);
        assert!((kernel[0] - 8.922106863539e-5).abs() < 1e-15);
    }

    #[test]
    fn large_gaussian_keeps_a_constant_and_the_peak() {
        let image = vec![4.0f32; 64 * 48];
        let flat = gaussian_filter(&image, 48, 64, 18.0);
        for value in &flat {
            assert!((value - 4.0).abs() < 1e-3, "{value}");
        }
        let mut spot = vec![0.0f32; 64 * 48];
        spot[24 * 64 + 32] = 100.0;
        let blurred = gaussian_filter(&spot, 48, 64, 12.0);
        let peak_at = blurred.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap().0;
        let peak_y = peak_at / 64;
        let peak_x = peak_at % 64;
        assert!((peak_y as i32 - 24).abs() <= 1, "y={peak_y}");
        assert!((peak_x as i32 - 32).abs() <= 1, "x={peak_x}");
    }

    #[test]
    fn gaussian_filter_matches_reference_4x4() {
        let image: Vec<f32> = (0..16).map(|v| v as f32).collect();
        let out = gaussian_filter(&image, 4, 4, 1.5);
        let expected = [
            3.8597796, 4.2864947, 4.889154, 5.315868, 5.566637, 5.9933515, 6.596011, 7.0227256, 7.9772744, 8.40399,
            9.006649, 9.433363, 9.684132, 10.1108465, 10.713507, 11.140221,
        ];
        for (got, exp) in out.iter().zip(expected) {
            assert!((got - exp).abs() < 1e-4, "{got} != {exp}");
        }
    }

    #[test]
    fn demosaic_bggr_lattice_matches_opencv_edges() {
        let mut raw = vec![0u16; 36];
        for y in 0..6 {
            for x in 0..6 {
                let value = match ((y & 1), (x & 1)) {
                    (0, 0) => 1000,
                    (0, 1) => 2000,
                    (1, 0) => 3000,
                    _ => 4000,
                };
                raw[y * 6 + x] = value;
            }
        }
        let rgb = demosaic_image(&raw, 6, 6, 0);
        let g = |y: usize, x: usize| rgb[(y * 6 + x) * 3 + 1];
        assert_eq!(g(1, 1), 2500.0);
        assert_eq!(g(1, 2), 3000.0);
        assert_eq!(g(2, 2), 2500.0);
        assert_eq!(g(0, 2), g(1, 2));
        assert_eq!(g(2, 0), g(2, 1));
        assert_eq!(rgb[(2 * 6 + 2) * 3], 1000.0);
        assert_eq!(rgb[(2 * 6 + 2) * 3 + 2], 4000.0);
    }

    #[test]
    fn bilinear_is_zero_outside_and_exact_on_pixels() {
        let img = [1.0f32, 2.0, 3.0, 4.0];
        unsafe {
            assert_eq!(bilinear_strided(img.as_ptr(), 2, 2, 1, 0.0, 0.0), 1.0);
            assert_eq!(bilinear_strided(img.as_ptr(), 2, 2, 1, 0.0, 1.0), 2.0);
            let mid = bilinear_strided(img.as_ptr(), 2, 2, 1, 0.5, 0.5);
            assert!((mid - 2.5).abs() < 1e-5);
            assert_eq!(bilinear_strided(img.as_ptr(), 2, 2, 1, -1.0, 0.0), 0.0);
        }
    }
}

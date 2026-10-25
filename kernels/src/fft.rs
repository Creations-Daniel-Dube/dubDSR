use crate::complex::C32;

const PI: f32 = 3.14159265358979323846;

pub fn is_pow2(n: usize) -> bool {
    n != 0 && n.is_power_of_two()
}

pub fn bluestein_m(n: usize) -> usize {
    if n <= 1 {
        return 1;
    }
    (2 * n - 1).next_power_of_two()
}

/// FFT radix-2 sur place. `inverse` divise par `n` (convention NumPy).
pub unsafe fn fft_pow2_raw(data: *mut C32, n: usize, inverse: bool) {
    if n <= 1 {
        return;
    }
    let mut j = 0usize;
    let mut i = 1usize;
    while i < n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            let a = unsafe { *data.add(i) };
            let b = unsafe { *data.add(j) };
            unsafe {
                *data.add(i) = b;
                *data.add(j) = a;
            }
        }
        i += 1;
    }

    let mut len = 2usize;
    while len <= n {
        let mut ang = PI * 2.0 / (len as f32);
        if !inverse {
            ang = -ang;
        }
        let wlen = C32::from_angle(ang);
        let half = len >> 1;
        let mut base = 0usize;
        while base < n {
            let mut w = C32::new(1.0, 0.0);
            let mut k = 0usize;
            while k < half {
                let u = unsafe { *data.add(base + k) };
                let v = unsafe { (*data.add(base + k + half)).mul(w) };
                unsafe {
                    *data.add(base + k) = u.add(v);
                    *data.add(base + k + half) = u.sub(v);
                }
                w = w.mul(wlen);
                k += 1;
            }
            base += len;
        }
        len <<= 1;
    }

    if inverse {
        let scale = 1.0 / (n as f32);
        let mut i = 0usize;
        while i < n {
            unsafe {
                *data.add(i) = (*data.add(i)).scale(scale);
            }
            i += 1;
        }
    }
}

/// Chirp de Bluestein, longueur `n`. Signe avant pour une DFT directe.
/// L'angle π k²/n dépasse le millier de radians en HD : la réduction se fait en f64,
/// sinon la simple précision avale la fraction et la corrélation retombe à zéro.
pub fn chirp_at(k: usize, n: usize, inverse: bool) -> C32 {
    if n == 0 {
        return C32::new(1.0, 0.0);
    }
    let mut ang = core::f64::consts::PI * (k as f64) * (k as f64) / (n as f64);
    if !inverse {
        ang = -ang;
    }
    let tau = core::f64::consts::TAU;
    ang -= tau * round_nearest_f64(ang / tau);
    C32::from_angle(ang as f32)
}

fn round_nearest_f64(x: f64) -> f64 {
    if !x.is_finite() {
        return 0.0;
    }
    if x >= 0.0 {
        let shifted = x + 0.5;
        if shifted >= i64::MAX as f64 {
            return x;
        }
        shifted as i64 as f64
    } else {
        let shifted = -x + 0.5;
        if shifted >= i64::MAX as f64 {
            return x;
        }
        -(shifted as i64 as f64)
    }
}

/// Convolution chirp. `scratch` et `b_fft` ont la longueur `m` (puissance de 2).
pub unsafe fn bluestein_raw(
    input: *const C32,
    n: usize,
    output: *mut C32,
    scratch: *mut C32,
    b_fft: *const C32,
    m: usize,
    inverse: bool,
) {
    let mut i = 0usize;
    while i < m {
        let value = if i < n {
            let x = unsafe { *input.add(i) };
            x.mul(chirp_at(i, n, inverse))
        } else {
            C32::zero()
        };
        unsafe {
            *scratch.add(i) = value;
        }
        i += 1;
    }
    unsafe {
        fft_pow2_raw(scratch, m, false);
    }
    i = 0;
    while i < m {
        let a = unsafe { *scratch.add(i) };
        let b = unsafe { *b_fft.add(i) };
        unsafe {
            *scratch.add(i) = a.mul(b);
        }
        i += 1;
    }
    unsafe {
        fft_pow2_raw(scratch, m, true);
    }
    i = 0;
    while i < n {
        let c = unsafe { *scratch.add(i) };
        let mut y = c.mul(chirp_at(i, n, inverse));
        if inverse && n > 0 {
            y = y.scale(1.0 / (n as f32));
        }
        unsafe {
            *output.add(i) = y;
        }
        i += 1;
    }
}

pub unsafe fn fft_1d_raw(
    input: *const C32,
    n: usize,
    output: *mut C32,
    scratch: *mut C32,
    b_fft: *const C32,
    m: usize,
    inverse: bool,
) {
    if n == 0 {
        return;
    }
    if n == 1 {
        unsafe {
            *output = *input;
        }
        return;
    }
    if is_pow2(n) {
        let mut i = 0usize;
        while i < n {
            unsafe {
                *output.add(i) = *input.add(i);
            }
            i += 1;
        }
        unsafe {
            fft_pow2_raw(output, n, inverse);
        }
        return;
    }
    unsafe {
        bluestein_raw(input, n, output, scratch, b_fft, m, inverse);
    }
}

#[cfg(not(target_arch = "nvptx64"))]
pub fn make_b_fft(n: usize, inverse: bool) -> Vec<C32> {
    let m = bluestein_m(n);
    let mut b = vec![C32::zero(); m];
    if n == 0 {
        return b;
    }
    b[0] = chirp_at(0, n, inverse).conj();
    let mut k = 1usize;
    while k < n {
        let bk = chirp_at(k, n, inverse).conj();
        b[k] = bk;
        b[m - k] = bk;
        k += 1;
    }
    unsafe {
        fft_pow2_raw(b.as_mut_ptr(), m, false);
    }
    b
}

#[cfg(not(target_arch = "nvptx64"))]
pub fn fft_1d_slice(row: &mut [C32], inverse: bool) {
    let n = row.len();
    if n <= 1 {
        return;
    }
    let m = bluestein_m(n);
    let b_fft = if is_pow2(n) {
        Vec::new()
    } else {
        make_b_fft(n, inverse)
    };
    let mut scratch = vec![C32::zero(); m];
    let mut out = vec![C32::zero(); n];
    unsafe {
        fft_1d_raw(
            row.as_ptr(),
            n,
            out.as_mut_ptr(),
            scratch.as_mut_ptr(),
            b_fft.as_ptr(),
            m,
            inverse,
        );
    }
    row.copy_from_slice(&out);
}

#[cfg(not(target_arch = "nvptx64"))]
struct AxisPlan {
    m: usize,
    b_forward: Vec<C32>,
    b_inverse: Vec<C32>,
    chirp_forward: Vec<C32>,
    chirp_inverse: Vec<C32>,
}

#[cfg(not(target_arch = "nvptx64"))]
impl AxisPlan {
    fn new(n: usize) -> Self {
        if n <= 1 || is_pow2(n) {
            return Self {
                m: 0,
                b_forward: Vec::new(),
                b_inverse: Vec::new(),
                chirp_forward: Vec::new(),
                chirp_inverse: Vec::new(),
            };
        }
        Self {
            m: bluestein_m(n),
            b_forward: make_b_fft(n, false),
            b_inverse: make_b_fft(n, true),
            chirp_forward: (0..n).map(|k| chirp_at(k, n, false)).collect(),
            chirp_inverse: (0..n).map(|k| chirp_at(k, n, true)).collect(),
        }
    }
}

#[cfg(not(target_arch = "nvptx64"))]
fn transform_rows_serial(data: &mut [C32], rows: usize, cols: usize, plan: &AxisPlan, inverse: bool) {
    if cols <= 1 || plan.m == 0 {
        for row in 0..rows {
            unsafe {
                fft_pow2_raw(data.as_mut_ptr().add(row * cols), cols, inverse);
            }
        }
        return;
    }
    let b = if inverse { &plan.b_inverse } else { &plan.b_forward };
    let chirp = if inverse { &plan.chirp_inverse } else { &plan.chirp_forward };
    let scale = if inverse { 1.0 / cols as f32 } else { 1.0 };
    let mut scratch = vec![C32::zero(); plan.m];
    for row in 0..rows {
        let line = &mut data[row * cols..(row + 1) * cols];
        for i in 0..plan.m {
            scratch[i] = if i < cols { line[i].mul(chirp[i]) } else { C32::zero() };
        }
        unsafe {
            fft_pow2_raw(scratch.as_mut_ptr(), plan.m, false);
        }
        for (bin, coeff) in scratch.iter_mut().zip(b) {
            *bin = bin.mul(*coeff);
        }
        unsafe {
            fft_pow2_raw(scratch.as_mut_ptr(), plan.m, true);
        }
        for i in 0..cols {
            line[i] = scratch[i].mul(chirp[i]).scale(scale);
        }
    }
}

#[cfg(not(target_arch = "nvptx64"))]
fn transform_rows(data: &mut [C32], rows: usize, cols: usize, plan: &AxisPlan, inverse: bool) {
    let workers = std::thread::available_parallelism().map(|count| count.get()).unwrap_or(1);
    if workers <= 1 || rows < 8 {
        transform_rows_serial(data, rows, cols, plan, inverse);
        return;
    }
    let rows_per = rows.div_ceil(workers.min(rows));
    let span = rows_per.saturating_mul(cols).max(cols);
    std::thread::scope(|scope| {
        for chunk in data[..rows * cols].chunks_mut(span) {
            let chunk_rows = chunk.len() / cols;
            scope.spawn(move || transform_rows_serial(chunk, chunk_rows, cols, plan, inverse));
        }
    });
}

#[cfg(not(target_arch = "nvptx64"))]
pub fn fft2(data: &mut [C32], height: usize, width: usize, inverse: bool) {
    if height == 0 || width == 0 || data.len() < height * width {
        return;
    }
    let row_plan = AxisPlan::new(width);
    let col_plan = AxisPlan::new(height);
    transform_rows(data, height, width, &row_plan, inverse);
    let mut transposed = vec![C32::zero(); height * width];
    for y in 0..height {
        for x in 0..width {
            transposed[x * height + y] = data[y * width + x];
        }
    }
    transform_rows(&mut transposed, width, height, &col_plan, inverse);
    for y in 0..height {
        for x in 0..width {
            data[y * width + x] = transposed[x * height + y];
        }
    }
}

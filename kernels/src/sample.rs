use crate::fmath::floor_f32;

pub unsafe fn bilinear_strided(img: *const f32, height: i32, width: i32, stride: i32, y: f32, x: f32) -> f32 {
    if height <= 0 || width <= 0 {
        return 0.0;
    }
    let y0f = floor_f32(y);
    let x0f = floor_f32(x);
    let y0 = y0f as i32;
    let x0 = x0f as i32;
    let wy = y - y0f;
    let wx = x - x0f;
    let v00 = unsafe { sample_at(img, height, width, stride, y0, x0) };
    let v01 = unsafe { sample_at(img, height, width, stride, y0, x0 + 1) };
    let v10 = unsafe { sample_at(img, height, width, stride, y0 + 1, x0) };
    let v11 = unsafe { sample_at(img, height, width, stride, y0 + 1, x0 + 1) };
    let top = v00 * (1.0 - wx) + v01 * wx;
    let bottom = v10 * (1.0 - wx) + v11 * wx;
    top * (1.0 - wy) + bottom * wy
}

unsafe fn sample_at(img: *const f32, height: i32, width: i32, stride: i32, y: i32, x: i32) -> f32 {
    if y < 0 || x < 0 || y >= height || x >= width {
        return 0.0;
    }
    unsafe { *img.add(((y * width + x) * stride) as usize) }
}

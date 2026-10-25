fn avg2(a: u32, b: u32) -> u16 {
    ((a + b + 1) >> 1) as u16
}

fn avg4(a: u32, b: u32, c: u32, d: u32) -> u16 {
    ((a + b + c + d + 2) >> 2) as u16
}

/// 0 BGGR, 1 RGGB, 2 GRBG, 3 GBRG — codes passés à OpenCV par l'ancien logiciel.
fn red_parity(pattern: i32) -> (i32, i32) {
    match pattern {
        1 => (1, 1),
        2 => (1, 0),
        3 => (0, 1),
        _ => (0, 0),
    }
}

unsafe fn raw_at(raw: *const u16, width: i32, y: i32, x: i32) -> u32 {
    unsafe { *raw.add((y * width + x) as usize) as u32 }
}

/// Un pixel RGB float, bords copiés depuis la ligne et la colonne intérieures.
pub unsafe fn demosaic_pixel(
    raw: *const u16,
    height: i32,
    width: i32,
    y: i32,
    x: i32,
    pattern: i32,
) -> (f32, f32, f32) {
    if height <= 0 || width <= 0 {
        return (0.0, 0.0, 0.0);
    }
    let mut sy = y;
    let mut sx = x;
    if height > 2 {
        if sy == 0 {
            sy = 1;
        } else if sy == height - 1 {
            sy = height - 2;
        }
    }
    if width > 2 {
        if sx == 0 {
            sx = 1;
        } else if sx == width - 1 {
            sx = width - 2;
        }
    }
    sy = sy.clamp(0, height - 1);
    sx = sx.clamp(0, width - 1);

    let (ry, rx) = red_parity(pattern);
    let y_even = (sy & 1) == ry;
    let x_even = (sx & 1) == rx;
    let is_red = y_even && x_even;
    let is_blue = !y_even && !x_even;

    let center = unsafe { raw_at(raw, width, sy, sx) };
    let up = if sy > 0 {
        unsafe { raw_at(raw, width, sy - 1, sx) }
    } else {
        center
    };
    let down = if sy + 1 < height {
        unsafe { raw_at(raw, width, sy + 1, sx) }
    } else {
        center
    };
    let left = if sx > 0 {
        unsafe { raw_at(raw, width, sy, sx - 1) }
    } else {
        center
    };
    let right = if sx + 1 < width {
        unsafe { raw_at(raw, width, sy, sx + 1) }
    } else {
        center
    };
    let ul = if sy > 0 && sx > 0 {
        unsafe { raw_at(raw, width, sy - 1, sx - 1) }
    } else {
        center
    };
    let ur = if sy > 0 && sx + 1 < width {
        unsafe { raw_at(raw, width, sy - 1, sx + 1) }
    } else {
        center
    };
    let dl = if sy + 1 < height && sx > 0 {
        unsafe { raw_at(raw, width, sy + 1, sx - 1) }
    } else {
        center
    };
    let dr = if sy + 1 < height && sx + 1 < width {
        unsafe { raw_at(raw, width, sy + 1, sx + 1) }
    } else {
        center
    };

    let green = avg4(up, down, left, right) as f32;
    let diag = avg4(ul, ur, dl, dr) as f32;
    let horiz = avg2(left, right) as f32;
    let vert = avg2(up, down) as f32;
    let here = center as f32;

    let (r, g, b) = if is_red {
        (here, green, diag)
    } else if is_blue {
        (diag, green, here)
    } else if y_even {
        (horiz, here, vert)
    } else {
        (vert, here, horiz)
    };
    (r, g, b)
}

#[cfg(not(target_arch = "nvptx64"))]
pub fn demosaic_image(raw: &[u16], height: i32, width: i32, pattern: i32) -> Vec<f32> {
    let mut out = vec![0.0f32; (height * width * 3) as usize];
    for y in 0..height {
        for x in 0..width {
            let (r, g, b) = unsafe { demosaic_pixel(raw.as_ptr(), height, width, y, x, pattern) };
            let i = ((y * width + x) * 3) as usize;
            out[i] = r;
            out[i + 1] = g;
            out[i + 2] = b;
        }
    }
    out
}

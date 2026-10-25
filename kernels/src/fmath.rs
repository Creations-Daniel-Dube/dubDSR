pub fn floor_f32(x: f32) -> f32 {
    if !x.is_finite() {
        return x;
    }
    let truncated = x as i32;
    let truncated_f = truncated as f32;
    if x < 0.0 && truncated_f != x {
        (truncated.wrapping_sub(1)) as f32
    } else {
        truncated_f
    }
}

pub fn round_f32(x: f32) -> f32 {
    if x >= 0.0 {
        floor_f32(x + 0.5)
    } else {
        -floor_f32(-x + 0.5)
    }
}

pub fn sqrt_f32(x: f32) -> f32 {
    if !(x > 0.0) {
        return 0.0;
    }
    let mut guess = f32::from_bits(0x1FBD1DF5 + (x.to_bits() >> 1));
    guess = 0.5 * (guess + x / guess);
    guess = 0.5 * (guess + x / guess);
    guess = 0.5 * (guess + x / guess);
    0.5 * (guess + x / guess)
}

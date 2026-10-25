//! Sinus / cosinus sans libm, utilisables dans un noyau PTX.

use crate::fmath::round_f32;

const PI: f32 = 3.14159265358979323846;
const HALF_PI: f32 = 1.57079632679489661923;
const TWO_PI: f32 = 6.28318530717958647692;
const INV_TWO_PI: f32 = 0.15915494309189533577;

fn wrap_pi(x: f32) -> f32 {
    let k = round_f32(x * INV_TWO_PI);
    x - k * TWO_PI
}

pub fn sin_approx(x: f32) -> f32 {
    let mut x = wrap_pi(x);
    if x > HALF_PI {
        x = PI - x;
    } else if x < -HALF_PI {
        x = -PI - x;
    }
    let x2 = x * x;
    x * (1.0
        + x2 * (-1.6666667163e-1
            + x2 * (8.3333337680e-3
                + x2 * (-1.9841270114e-4
                    + x2 * (2.7557314297e-6 + x2 * (-2.5050759689e-8 + x2 * 1.5896910177e-10))))))
}

pub fn cos_approx(x: f32) -> f32 {
    sin_approx(x + HALF_PI)
}

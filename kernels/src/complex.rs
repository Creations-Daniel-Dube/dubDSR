use crate::fmath::sqrt_f32;
use crate::trig::{cos_approx, sin_approx};

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct C32 {
    pub re: f32,
    pub im: f32,
}

impl C32 {
    pub fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }

    pub fn zero() -> Self {
        Self { re: 0.0, im: 0.0 }
    }

    pub fn from_angle(angle: f32) -> Self {
        Self {
            re: cos_approx(angle),
            im: sin_approx(angle),
        }
    }

    pub fn add(self, other: Self) -> Self {
        Self {
            re: self.re + other.re,
            im: self.im + other.im,
        }
    }

    pub fn sub(self, other: Self) -> Self {
        Self {
            re: self.re - other.re,
            im: self.im - other.im,
        }
    }

    pub fn mul(self, other: Self) -> Self {
        Self {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }

    pub fn scale(self, factor: f32) -> Self {
        Self {
            re: self.re * factor,
            im: self.im * factor,
        }
    }

    pub fn conj(self) -> Self {
        Self {
            re: self.re,
            im: -self.im,
        }
    }

    pub fn norm(self) -> f32 {
        sqrt_f32(self.re * self.re + self.im * self.im)
    }
}

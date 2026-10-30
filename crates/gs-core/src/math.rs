//! Deterministic math. Everything here gives bit-identical results on the server (native) and
//! in the browser (wasm32): basic IEEE ops (+ - * / sqrt) are exact everywhere, and every
//! transcendental goes through pure-Rust `libm` instead of the platform's.
//!
//! `glam` is compiled with `libm` + `scalar-math`, so its methods are safe too.
//! `clippy.toml` forbids `f32::sin` & friends inside sim crates.

pub use glam::{Quat, Vec2, Vec3, Vec4, vec2, vec3};

pub const PI: f32 = core::f32::consts::PI;
pub const TAU: f32 = core::f32::consts::TAU;

#[inline]
pub fn sin(x: f32) -> f32 {
    libm::sinf(x)
}
#[inline]
pub fn cos(x: f32) -> f32 {
    libm::cosf(x)
}
#[inline]
pub fn sin_cos(x: f32) -> (f32, f32) {
    libm::sincosf(x)
}
#[inline]
pub fn tan(x: f32) -> f32 {
    libm::tanf(x)
}
#[inline]
pub fn asin(x: f32) -> f32 {
    libm::asinf(x)
}
#[inline]
pub fn acos(x: f32) -> f32 {
    libm::acosf(x)
}
#[inline]
pub fn atan(x: f32) -> f32 {
    libm::atanf(x)
}
#[inline]
pub fn atan2(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}
#[inline]
pub fn exp(x: f32) -> f32 {
    libm::expf(x)
}
#[inline]
pub fn ln(x: f32) -> f32 {
    libm::logf(x)
}
#[inline]
pub fn powf(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}
#[inline]
pub fn hypot(x: f32, y: f32) -> f32 {
    libm::hypotf(x, y)
}
/// IEEE sqrt is correctly rounded on every target, so this is just the intrinsic.
#[inline]
pub fn sqrt(x: f32) -> f32 {
    x.sqrt()
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Move `cur` toward `target` by at most `max_delta`.
#[inline]
pub fn approach(cur: f32, target: f32, max_delta: f32) -> f32 {
    if cur < target { (cur + max_delta).min(target) } else { (cur - max_delta).max(target) }
}

/// Wrap an angle into (-PI, PI].
#[inline]
pub fn wrap_angle(a: f32) -> f32 {
    let mut a = a % TAU;
    if a <= -PI {
        a += TAU;
    } else if a > PI {
        a -= TAU;
    }
    a
}

/// Rotation about +Y (yaw), the usual "facing" for ground characters.
#[inline]
pub fn yaw_quat(yaw: f32) -> Quat {
    let (s, c) = sin_cos(yaw * 0.5);
    Quat::from_xyzw(0.0, s, 0.0, c)
}

/// Unit vector on the XZ plane for a yaw angle (yaw 0 faces -Z, three.js "forward").
#[inline]
pub fn yaw_dir(yaw: f32) -> Vec3 {
    let (s, c) = sin_cos(yaw);
    Vec3::new(-s, 0.0, -c)
}

/// Yaw that faces along `dir` on the XZ plane (inverse of [`yaw_dir`]).
#[inline]
pub fn dir_yaw(dir: Vec3) -> f32 {
    atan2(-dir.x, -dir.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn yaw_roundtrip() {
        for i in -10..10 {
            let y = i as f32 * 0.3;
            assert!((wrap_angle(dir_yaw(yaw_dir(y))) - wrap_angle(y)).abs() < 1e-5);
        }
    }
}

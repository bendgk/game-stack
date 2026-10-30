//! The one abstraction that makes the engine dimension-agnostic: every algorithm is written
//! against [`Vector`], implemented for `glam::Vec2` (2D mode) and `glam::Vec3` (3D mode).
//! Axis 1 (Y) is "up" in both modes, so gravity, grounding and floors read the same.

use std::fmt::Debug;
use std::ops::{Add, AddAssign, Div, Mul, MulAssign, Neg, Sub, SubAssign};

use glam::{Vec2, Vec3};
use gs_wire::Wire;

pub trait Vector:
    Copy
    + Debug
    + Default
    + PartialEq
    + Wire
    + Add<Output = Self>
    + Sub<Output = Self>
    + Neg<Output = Self>
    + Mul<f32, Output = Self>
    + Div<f32, Output = Self>
    + AddAssign
    + SubAssign
    + MulAssign<f32>
    + 'static
{
    /// Number of axes (2 or 3).
    const DIM: usize;
    const ZERO: Self;
    /// +Y.
    const UP: Self;

    fn splat(v: f32) -> Self;
    fn get(self, axis: usize) -> f32;
    fn with(self, axis: usize, v: f32) -> Self;
    fn dot(self, o: Self) -> f32;
    fn abs(self) -> Self;
    fn min(self, o: Self) -> Self;
    fn max(self, o: Self) -> Self;

    /// Unit vector along `axis`.
    fn axis(axis: usize) -> Self {
        Self::ZERO.with(axis, 1.0)
    }
    fn length_squared(self) -> f32 {
        self.dot(self)
    }
    fn length(self) -> f32 {
        self.length_squared().sqrt()
    }
    fn clamp(self, lo: Self, hi: Self) -> Self {
        self.max(lo).min(hi)
    }
    fn normalize_or(self, fallback: Self) -> Self {
        let l = self.length();
        if l > 1e-12 { self / l } else { fallback }
    }
    /// Component of `self` perpendicular to unit `n`.
    fn reject(self, n: Self) -> Self {
        self - n * self.dot(n)
    }
}

impl Vector for Vec2 {
    const DIM: usize = 2;
    const ZERO: Self = Vec2::ZERO;
    const UP: Self = Vec2::Y;
    fn splat(v: f32) -> Self {
        Vec2::splat(v)
    }
    fn get(self, axis: usize) -> f32 {
        self[axis]
    }
    fn with(mut self, axis: usize, v: f32) -> Self {
        self[axis] = v;
        self
    }
    fn dot(self, o: Self) -> f32 {
        Vec2::dot(self, o)
    }
    fn abs(self) -> Self {
        Vec2::abs(self)
    }
    fn min(self, o: Self) -> Self {
        Vec2::min(self, o)
    }
    fn max(self, o: Self) -> Self {
        Vec2::max(self, o)
    }
}

impl Vector for Vec3 {
    const DIM: usize = 3;
    const ZERO: Self = Vec3::ZERO;
    const UP: Self = Vec3::Y;
    fn splat(v: f32) -> Self {
        Vec3::splat(v)
    }
    fn get(self, axis: usize) -> f32 {
        self[axis]
    }
    fn with(mut self, axis: usize, v: f32) -> Self {
        self[axis] = v;
        self
    }
    fn dot(self, o: Self) -> f32 {
        Vec3::dot(self, o)
    }
    fn abs(self) -> Self {
        Vec3::abs(self)
    }
    fn min(self, o: Self) -> Self {
        Vec3::min(self, o)
    }
    fn max(self, o: Self) -> Self {
        Vec3::max(self, o)
    }
}

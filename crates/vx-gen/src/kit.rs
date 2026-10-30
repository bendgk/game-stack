//! The modelling kit: primitives with fast `classify`, transforms, and a builder that turns a
//! list of parts into a prefab DAG. Props, facade modules, tiles and trees are all built here.

use std::sync::Arc;

use vx_core::compose::{Aabb, BoxShape, CARVE, Class, Shape};
use vx_core::{AIR, Dag, IVec3, Mat, Palette, Vec3, prefab};

/// A solid cylinder along an axis (0 = x, 1 = y, 2 = z).
pub struct Cyl {
    pub axis: usize,
    pub center: Vec3,
    pub r: f32,
    pub lo: f32,
    pub hi: f32,
    pub mat: Mat,
}

impl Shape for Cyl {
    fn bounds(&self) -> Aabb {
        let mut min = self.center - Vec3::splat(self.r);
        let mut max = self.center + Vec3::splat(self.r);
        min[self.axis] = self.lo;
        max[self.axis] = self.hi;
        Aabb::new(min.floor().as_ivec3(), max.ceil().as_ivec3() + IVec3::ONE)
    }
    fn classify(&self, cube: Aabb) -> Class {
        let b = self.bounds();
        if !b.overlaps(&cube) { Class::Empty } else { Class::Mixed }
    }
    fn sample(&self, p: IVec3) -> Mat {
        let q = p.as_vec3() + Vec3::splat(0.5);
        let a = q[self.axis];
        if a < self.lo || a >= self.hi {
            return AIR;
        }
        let mut d = q - self.center;
        d[self.axis] = 0.0;
        if d.length_squared() <= self.r * self.r { self.mat } else { AIR }
    }
}

/// A solid ellipsoid; with `noise > 0` its surface is lumpy (foliage, rocks).
pub struct Blob {
    pub center: Vec3,
    pub radii: Vec3,
    pub mat: Mat,
    pub noise: f32,
    pub seed: u64,
    /// Fraction of interior voxels kept (< 1 = airy foliage).
    pub fill: f32,
    pub alt: Option<(Mat, f32)>,
}

impl Shape for Blob {
    fn bounds(&self) -> Aabb {
        let r = self.radii * (1.0 + self.noise);
        Aabb::new((self.center - r).floor().as_ivec3(), (self.center + r).ceil().as_ivec3() + IVec3::ONE)
    }
    fn sample(&self, p: IVec3) -> Mat {
        let q = (p.as_vec3() + Vec3::splat(0.5) - self.center) / self.radii;
        let mut d = q.length();
        if self.noise > 0.0 {
            let n = crate::rng::noise2(q.x * 3.0 + q.y * 1.7, q.z * 3.0 - q.y * 2.3, self.seed) - 0.5;
            d += n * self.noise * 2.0;
        }
        if d > 1.0 {
            return AIR;
        }
        let h = crate::rng::hash01(p.x, p.y, p.z, self.seed);
        if self.fill < 1.0 && d < 0.85 && h > self.fill {
            return AIR;
        }
        if let Some((m, share)) = self.alt
            && crate::rng::hash01(p.x / 2, p.y / 2, p.z / 2, self.seed ^ 77) < share
        {
            return m;
        }
        self.mat
    }
}

/// Rotate a part by `quarter_turns` × 90° about +Y around the centre of a `size`-wide cube.
pub struct RotY {
    pub inner: Box<dyn Shape>,
    pub turns: u8,
    pub size: i32,
}

impl RotY {
    fn to_local(&self, p: IVec3) -> IVec3 {
        let s = self.size - 1;
        match self.turns % 4 {
            0 => p,
            1 => IVec3::new(p.z, p.y, s - p.x),
            2 => IVec3::new(s - p.x, p.y, s - p.z),
            _ => IVec3::new(s - p.z, p.y, p.x),
        }
    }
    fn to_world(&self, p: IVec3) -> IVec3 {
        let s = self.size - 1;
        match self.turns % 4 {
            0 => p,
            1 => IVec3::new(s - p.z, p.y, p.x),
            2 => IVec3::new(s - p.x, p.y, s - p.z),
            _ => IVec3::new(p.z, p.y, s - p.x),
        }
    }
}

impl Shape for RotY {
    fn bounds(&self) -> Aabb {
        let b = self.inner.bounds();
        let a = self.to_world(b.min);
        let c = self.to_world(b.max - IVec3::ONE);
        Aabb::new(a.min(c), a.max(c) + IVec3::ONE)
    }
    fn classify(&self, cube: Aabb) -> Class {
        if !self.bounds().overlaps(&cube) { Class::Empty } else { Class::Mixed }
    }
    fn sample(&self, p: IVec3) -> Mat {
        self.inner.sample(self.to_local(p))
    }
}

/// A shape moved by an offset.
pub struct Moved {
    pub inner: Arc<dyn Shape>,
    pub by: IVec3,
}

impl Shape for Moved {
    fn bounds(&self) -> Aabb {
        let b = self.inner.bounds();
        Aabb::new(b.min + self.by, b.max + self.by)
    }
    fn classify(&self, cube: Aabb) -> Class {
        self.inner.classify(Aabb::new(cube.min - self.by, cube.max - self.by))
    }
    fn sample(&self, p: IVec3) -> Mat {
        self.inner.sample(p - self.by)
    }
}

/// Collects parts (in local voxel coordinates) and builds them into a prefab.
#[derive(Default)]
pub struct Kit {
    pub parts: Vec<Box<dyn Shape>>,
}

impl Kit {
    pub fn new() -> Self {
        Kit { parts: Vec::new() }
    }
    pub fn push(&mut self, s: impl Shape + 'static) -> &mut Self {
        self.parts.push(Box::new(s));
        self
    }
    /// Box from `min` (inclusive) to `max` (exclusive).
    pub fn bx(&mut self, min: [i32; 3], max: [i32; 3], mat: Mat) -> &mut Self {
        let (a, b) = (IVec3::from_array(min), IVec3::from_array(max));
        self.push(BoxShape { b: Aabb::new(a.min(b), a.max(b)), mat })
    }
    pub fn carve(&mut self, min: [i32; 3], max: [i32; 3]) -> &mut Self {
        self.bx(min, max, CARVE)
    }
    pub fn cyl(&mut self, axis: usize, center: [f32; 3], r: f32, lo: f32, hi: f32, mat: Mat) -> &mut Self {
        self.push(Cyl { axis, center: Vec3::from_array(center), r, lo, hi, mat })
    }
    pub fn blob(&mut self, center: [f32; 3], radii: [f32; 3], mat: Mat, noise: f32, seed: u64) -> &mut Self {
        self.push(Blob {
            center: Vec3::from_array(center),
            radii: Vec3::from_array(radii),
            mat,
            noise,
            seed,
            fill: 1.0,
            alt: None,
        })
    }
    /// Rotate every part so far by `turns` quarter turns inside a `size` cube.
    pub fn rotate(mut self, turns: u8, size: i32) -> Self {
        if turns.is_multiple_of(4) {
            return self;
        }
        self.parts =
            self.parts.into_iter().map(|p| Box::new(RotY { inner: p, turns, size }) as Box<dyn Shape>).collect();
        self
    }
    pub fn build(&self, palette: &Palette, levels: u32) -> Arc<Dag> {
        let refs: Vec<&dyn Shape> = self.parts.iter().map(|p| p.as_ref()).collect();
        prefab(palette, levels, &refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_is_a_bijection_and_rotates_four_times_back() {
        let r =
            RotY { inner: Box::new(BoxShape { b: Aabb::new(IVec3::ZERO, IVec3::ONE), mat: 1 }), turns: 1, size: 64 };
        for p in [IVec3::new(0, 0, 0), IVec3::new(5, 3, 60), IVec3::new(63, 1, 2)] {
            assert_eq!(r.to_local(r.to_world(p)), p);
        }
        let pal = Palette::default_city();
        let mut k = Kit::new();
        k.bx([0, 0, 0], [10, 4, 2], 1);
        let base = k.build(&pal, 3);
        let mut k4 = Kit::new();
        k4.bx([0, 0, 0], [10, 4, 2], 1);
        let rot = k4.rotate(1, 64).build(&pal, 3);
        assert_eq!(base.voxel_count(), rot.voxel_count());
        assert_eq!(rot.get(IVec3::new(63, 0, 0)), 1);
    }
}

//! Heightmaps: generators, a min/max pyramid for fast voxel classification, and derived maps.
//!
//! Heights are in voxels. One sample per `cell` voxels (e.g. 64 = one sample per 2 m). The
//! voxel shape interpolates bilinearly and adds fine detail noise.

use crate::noise::{billow2, fbm2, ridged2, terrace, warp2};

#[derive(Clone, Debug)]
pub struct Heightmap {
    pub w: usize,
    pub h: usize,
    /// Voxels between samples.
    pub cell: i32,
    pub data: Vec<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Relief {
    /// Gentle fbm hills.
    Hills,
    /// Ridged mountains with valleys.
    Mountains,
    /// An island: land falls off to the sea at the edges.
    Island,
    /// Warped canyons cut into a plateau.
    Canyons,
    /// Stepped mesas (desert).
    Mesas,
    /// Flat (a base for cities).
    Plains,
}

impl Heightmap {
    pub fn new(w: usize, h: usize, cell: i32) -> Self {
        Heightmap { w, h, cell, data: vec![0.0; w * h] }
    }

    #[inline]
    pub fn at(&self, x: usize, z: usize) -> f32 {
        self.data[z.min(self.h - 1) * self.w + x.min(self.w - 1)]
    }

    /// Bilinear height at voxel (x, z).
    pub fn sample(&self, x: f32, z: f32) -> f32 {
        let fx = (x / self.cell as f32).clamp(0.0, (self.w - 1) as f32);
        let fz = (z / self.cell as f32).clamp(0.0, (self.h - 1) as f32);
        let (x0, z0) = (fx.floor() as usize, fz.floor() as usize);
        let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
        let a = self.at(x0, z0) + (self.at(x0 + 1, z0) - self.at(x0, z0)) * tx;
        let b = self.at(x0, z0 + 1) + (self.at(x0 + 1, z0 + 1) - self.at(x0, z0 + 1)) * tx;
        a + (b - a) * tz
    }

    /// Slope (rise over run, voxels per voxel) at voxel (x, z).
    pub fn slope(&self, x: f32, z: f32) -> f32 {
        let c = self.cell as f32;
        let dx = self.sample(x + c, z) - self.sample(x - c, z);
        let dz = self.sample(x, z + c) - self.sample(x, z - c);
        (dx * dx + dz * dz).sqrt() / (2.0 * c)
    }

    /// Suggested height range for a relief, as a fraction of the map's width.
    pub fn suggested_relief(relief: Relief) -> f32 {
        match relief {
            Relief::Mountains => 0.3,
            Relief::Hills => 0.08,
            Relief::Island => 0.14,
            Relief::Canyons => 0.12,
            Relief::Mesas => 0.12,
            Relief::Plains => 0.02,
        }
    }

    /// Generate a relief. `amp` = height range (voxels), `base` = height of the lowest ground.
    pub fn generate(w: usize, h: usize, cell: i32, relief: Relief, seed: u32, base: f32, amp: f32) -> Self {
        let mut hm = Heightmap::new(w, h, cell);
        // Base feature size: about a third of the map.
        let scale = (w.max(h) as f32 / 3.0).max(8.0);
        let fill = |x: usize, z: usize| -> f32 {
            let (u, v) = (x as f32 / scale, z as f32 / scale);
            let t = match relief {
                Relief::Hills => fbm2(u, v, seed, 6) * 0.5 + 0.5,
                Relief::Plains => fbm2(u * 0.5, v * 0.5, seed, 3) * 0.04 + 0.02,
                Relief::Mountains => {
                    let (wu, wv) = warp2(u, v, seed, 0.25);
                    let r = ridged2(wu, wv, seed, 6);
                    let mask = (fbm2(u * 0.4, v * 0.4, seed ^ 9, 3) * 0.6 + 0.6).clamp(0.15, 1.0);
                    // Broad valleys, sharp crests.
                    (r * mask).powf(2.0)
                }
                Relief::Island => {
                    let (nx, nz) = (x as f32 / w as f32 * 2.0 - 1.0, z as f32 / h as f32 * 2.0 - 1.0);
                    let d = (nx * nx + nz * nz).sqrt();
                    let falloff = (1.0 - d.powf(2.2)).clamp(0.0, 1.0);
                    let n = fbm2(u, v, seed, 6) * 0.5 + 0.5;
                    let r = ridged2(u * 0.7, v * 0.7, seed ^ 3, 5);
                    (n * 0.6 + r * 0.6) * falloff - 0.12
                }
                Relief::Canyons => {
                    let (wu, wv) = warp2(u, v, seed, 1.2);
                    let c = billow2(wu * 0.8, wv * 0.8, seed, 4);
                    let plateau = 0.75 + fbm2(u, v, seed ^ 5, 4) * 0.08;
                    if c < 0.18 { plateau * (c / 0.18).powf(0.6) * 0.75 } else { plateau }
                }
                Relief::Mesas => {
                    let n = fbm2(u, v, seed, 5) * 0.5 + 0.5;
                    terrace(n.powf(1.5), 0.12, 0.85)
                }
            };
            base + t * amp
        };
        #[cfg(feature = "parallel")]
        {
            use rayon::prelude::*;
            hm.data.par_chunks_mut(w).enumerate().for_each(|(z, row)| {
                for (x, v) in row.iter_mut().enumerate() {
                    *v = fill(x, z);
                }
            });
        }
        #[cfg(not(feature = "parallel"))]
        for z in 0..h {
            for x in 0..w {
                hm.data[z * w + x] = fill(x, z);
            }
        }
        hm
    }

    /// Blend toward `flat_height` inside a rectangle (with a smooth margin) — carve a plateau
    /// for a city or a level.
    pub fn flatten(&mut self, x0: usize, z0: usize, x1: usize, z1: usize, flat_height: f32, margin: usize) {
        for z in 0..self.h {
            for x in 0..self.w {
                let dx = (x0 as i64 - x as i64).max(x as i64 - x1 as i64).max(0) as f32;
                let dz = (z0 as i64 - z as i64).max(z as i64 - z1 as i64).max(0) as f32;
                let d = (dx * dx + dz * dz).sqrt();
                let t = (d / margin.max(1) as f32).clamp(0.0, 1.0);
                let t = t * t * (3.0 - 2.0 * t);
                let i = z * self.w + x;
                self.data[i] = flat_height + (self.data[i] - flat_height) * t;
            }
        }
    }
}

/// Min/max heights over power-of-two blocks of samples, for classifying voxel cubes.
pub struct MinMax {
    levels: Vec<(usize, usize, Vec<(f32, f32)>)>,
    cell: i32,
}

impl MinMax {
    pub fn new(hm: &Heightmap) -> Self {
        let mut levels = vec![(hm.w, hm.h, hm.data.iter().map(|&v| (v, v)).collect::<Vec<_>>())];
        loop {
            let (w, h, prev) = levels.last().unwrap();
            if *w <= 1 && *h <= 1 {
                break;
            }
            let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
            let mut next = vec![(f32::MAX, f32::MIN); nw * nh];
            for z in 0..*h {
                for x in 0..*w {
                    let (a, b) = prev[z * w + x];
                    let n = &mut next[(z / 2) * nw + x / 2];
                    n.0 = n.0.min(a);
                    n.1 = n.1.max(b);
                }
            }
            levels.push((nw, nh, next));
        }
        MinMax { levels, cell: hm.cell }
    }

    /// Conservative (min, max) height over voxel rect [x0, x1) × [z0, x1).
    pub fn range(&self, x0: i32, z0: i32, x1: i32, z1: i32) -> (f32, f32) {
        let c = self.cell;
        // Sample range covering the rect (+1 for bilinear interpolation).
        let sx0 = (x0 / c).max(0) as usize;
        let sz0 = (z0 / c).max(0) as usize;
        let sx1 = ((x1 + c - 1) / c + 1).max(0) as usize;
        let sz1 = ((z1 + c - 1) / c + 1).max(0) as usize;
        let span = (sx1 - sx0).max(sz1 - sz0).max(1);
        let mut lvl = 0;
        while lvl + 1 < self.levels.len() && (1usize << (lvl + 1)) <= span / 2 {
            lvl += 1;
        }
        let (w, h, data) = &self.levels[lvl];
        let (a0, b0) = ((sx0 >> lvl).min(w - 1), (sz0 >> lvl).min(h - 1));
        let (a1, b1) = ((sx1 >> lvl).min(w - 1), (sz1 >> lvl).min(h - 1));
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for z in b0..=b1 {
            for x in a0..=a1 {
                let (l, u) = data[z * w + x];
                lo = lo.min(l);
                hi = hi.max(u);
            }
        }
        (lo, hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minmax_bounds_every_sample() {
        let hm = Heightmap::generate(130, 97, 64, Relief::Mountains, 4, 100.0, 2000.0);
        let mm = MinMax::new(&hm);
        for (x0, z0, x1, z1) in [(0, 0, 640, 640), (1000, 2000, 1500, 2600), (64 * 120, 64 * 90, 64 * 130, 64 * 97)] {
            let (lo, hi) = mm.range(x0, z0, x1, z1);
            for z in (z0..z1).step_by(16) {
                for x in (x0..x1).step_by(16) {
                    let v = hm.sample(x as f32, z as f32);
                    assert!(v >= lo - 1e-3 && v <= hi + 1e-3, "{v} not in [{lo}, {hi}]");
                }
            }
        }
    }
}

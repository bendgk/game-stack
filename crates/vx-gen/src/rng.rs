//! Deterministic randomness (SplitMix64) and value noise. Same seed → same world, everywhere.

#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9e37_79b9_7f4a_7c15)
    }
    /// An independent stream derived from this seed and a salt (e.g. a lot id).
    pub fn fork(&self, salt: u64) -> Rng {
        Rng::new(mix(self.0 ^ mix(salt)))
    }
    pub fn u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mix(self.0)
    }
    pub fn f32(&mut self) -> f32 {
        (self.u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }
    pub fn int(&mut self, lo: i32, hi_incl: i32) -> i32 {
        if hi_incl <= lo {
            return lo;
        }
        lo + (self.u64() % (hi_incl - lo + 1) as u64) as i32
    }
    pub fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }
    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.u64() % xs.len() as u64) as usize]
    }
}

pub fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Hash of integer coordinates → [0, 1).
pub fn hash01(x: i32, y: i32, z: i32, seed: u64) -> f32 {
    let h = mix(seed ^ mix((x as u64) ^ mix((y as u64) << 21 ^ (z as u64) << 42)));
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Smooth value noise in 2D, [0, 1).
pub fn noise2(x: f32, z: f32, seed: u64) -> f32 {
    let (x0, z0) = (x.floor(), z.floor());
    let (fx, fz) = (x - x0, z - z0);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let h = |a: f32, b: f32| hash01(a as i32, 0, b as i32, seed);
    let a = h(x0, z0) + (h(x0 + 1.0, z0) - h(x0, z0)) * s(fx);
    let b = h(x0, z0 + 1.0) + (h(x0 + 1.0, z0 + 1.0) - h(x0, z0 + 1.0)) * s(fx);
    a + (b - a) * s(fz)
}

/// Fractal noise (octaves of value noise), [0, 1).
pub fn fbm2(x: f32, z: f32, seed: u64, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut f, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += noise2(x * f, z * f, seed.wrapping_add(o as u64 * 101)) * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm
}

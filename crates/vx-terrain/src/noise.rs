//! Fast, deterministic noise primitives. Pure integer hashing + polynomial interpolation (no
//! trig, no tables), so results are identical on every machine and in wasm. Mirrors
//! `shaders/noise.wgsl` for GPU use.
//!
//! - [`perlin2`] / [`perlin3`]: gradient noise in [-1, 1]
//! - [`fbm2`] / [`fbm3`]: fractal sums
//! - [`ridged2`]: ridged multifractal (mountain ridges)
//! - [`billow2`]: |noise| clouds / rolling hills
//! - [`warp2`]: domain warping (organic, eroded-looking shapes)
//! - [`worley2`] / [`worley3`]: cellular (F1, F2) for rocks, cracks, cells
//! - [`terrace`]: quantise into stepped mesas

#[inline]
fn hash(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mut h = seed.wrapping_mul(0x27d4_eb2d)
        ^ (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ (z as u32).wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^ (h >> 15)
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
fn grad2(h: u32, x: f32, y: f32) -> f32 {
    // 8 gradient directions.
    match h & 7 {
        0 => x + y,
        1 => x - y,
        2 => -x + y,
        3 => -x - y,
        4 => x,
        5 => -x,
        6 => y,
        _ => -y,
    }
}

#[inline]
fn grad3(h: u32, x: f32, y: f32, z: f32) -> f32 {
    // Perlin's 12 edge gradients (+4 repeats).
    match h & 15 {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        3 => -x - y,
        4 => x + z,
        5 => -x + z,
        6 => x - z,
        7 => -x - z,
        8 => y + z,
        9 => -y + z,
        10 => y - z,
        11 => -y - z,
        12 => x + y,
        13 => -x + y,
        14 => -y + z,
        _ => -y - z,
    }
}

/// 2D gradient noise in roughly [-1, 1].
pub fn perlin2(x: f32, y: f32, seed: u32) -> f32 {
    let (xf, yf) = (x.floor(), y.floor());
    let (xi, yi) = (xf as i32, yf as i32);
    let (dx, dy) = (x - xf, y - yf);
    let (u, v) = (fade(dx), fade(dy));
    let a = grad2(hash(xi, yi, 0, seed), dx, dy);
    let b = grad2(hash(xi + 1, yi, 0, seed), dx - 1.0, dy);
    let c = grad2(hash(xi, yi + 1, 0, seed), dx, dy - 1.0);
    let d = grad2(hash(xi + 1, yi + 1, 0, seed), dx - 1.0, dy - 1.0);
    lerp(lerp(a, b, u), lerp(c, d, u), v) * 0.70710677
}

/// 3D gradient noise in roughly [-1, 1].
pub fn perlin3(x: f32, y: f32, z: f32, seed: u32) -> f32 {
    let (xf, yf, zf) = (x.floor(), y.floor(), z.floor());
    let (xi, yi, zi) = (xf as i32, yf as i32, zf as i32);
    let (dx, dy, dz) = (x - xf, y - yf, z - zf);
    let (u, v, w) = (fade(dx), fade(dy), fade(dz));
    let g = |ox: i32, oy: i32, oz: i32| {
        grad3(hash(xi + ox, yi + oy, zi + oz, seed), dx - ox as f32, dy - oy as f32, dz - oz as f32)
    };
    let x00 = lerp(g(0, 0, 0), g(1, 0, 0), u);
    let x10 = lerp(g(0, 1, 0), g(1, 1, 0), u);
    let x01 = lerp(g(0, 0, 1), g(1, 0, 1), u);
    let x11 = lerp(g(0, 1, 1), g(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// Fractal Brownian motion: `octaves` of perlin2, lacunarity 2, gain 0.5. ~[-1, 1].
pub fn fbm2(x: f32, y: f32, seed: u32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut f, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += perlin2(x * f, y * f, seed.wrapping_add(o * 1013)) * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm
}

pub fn fbm3(x: f32, y: f32, z: f32, seed: u32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut f, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += perlin3(x * f, y * f, z * f, seed.wrapping_add(o * 1013)) * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm
}

/// Ridged multifractal (Musgrave): sharp crests, eroded-looking valleys. [0, 1].
pub fn ridged2(x: f32, y: f32, seed: u32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut f, mut norm, mut weight) = (0.0, 1.0, 1.0, 0.0, 1.0f32);
    for o in 0..octaves {
        let mut n = 1.0 - perlin2(x * f, y * f, seed.wrapping_add(o * 7919)).abs();
        n *= n;
        n *= weight;
        weight = (n * 2.0).clamp(0.0, 1.0);
        sum += n * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm
}

/// Billowy noise: |fbm|. [0, 1].
pub fn billow2(x: f32, y: f32, seed: u32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut f, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += perlin2(x * f, y * f, seed.wrapping_add(o * 3571)).abs() * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm
}

/// Domain warping: offset the lookup by fbm, `strength` in input units.
pub fn warp2(x: f32, y: f32, seed: u32, strength: f32) -> (f32, f32) {
    let wx = fbm2(x + 5.2, y + 1.3, seed ^ 0x51, 3);
    let wy = fbm2(x - 1.7, y + 9.2, seed ^ 0xa3, 3);
    (x + wx * strength, y + wy * strength)
}

/// Cellular noise: distances to the nearest and second-nearest feature point (F1, F2).
pub fn worley2(x: f32, y: f32, seed: u32) -> (f32, f32) {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (mut f1, mut f2) = (9.0f32, 9.0f32);
    for oy in -1..=1 {
        for ox in -1..=1 {
            let h = hash(xi + ox, yi + oy, 7, seed);
            let px = (xi + ox) as f32 + (h & 0xffff) as f32 / 65536.0;
            let py = (yi + oy) as f32 + (h >> 16) as f32 / 65536.0;
            let d = (px - x) * (px - x) + (py - y) * (py - y);
            if d < f1 {
                f2 = f1;
                f1 = d;
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    (f1.sqrt(), f2.sqrt())
}

pub fn worley3(x: f32, y: f32, z: f32, seed: u32) -> f32 {
    let (xi, yi, zi) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
    let mut f1 = 9.0f32;
    for oz in -1..=1 {
        for oy in -1..=1 {
            for ox in -1..=1 {
                let h = hash(xi + ox, yi + oy, zi + oz, seed);
                let p = [
                    (xi + ox) as f32 + (h & 0x3ff) as f32 / 1024.0,
                    (yi + oy) as f32 + ((h >> 10) & 0x3ff) as f32 / 1024.0,
                    (zi + oz) as f32 + ((h >> 20) & 0x3ff) as f32 / 1024.0,
                ];
                let d = (p[0] - x).powi(2) + (p[1] - y).powi(2) + (p[2] - z).powi(2);
                f1 = f1.min(d);
            }
        }
    }
    f1.sqrt()
}

/// Quantise a height into terraces of `step`, with `sharpness` (0 = smooth, 1 = hard steps).
pub fn terrace(h: f32, step: f32, sharpness: f32) -> f32 {
    let k = h / step;
    let base = k.floor();
    let f = k - base;
    let s = f.powf(1.0 + sharpness * 8.0);
    (base + s) * step
}

/// Hash → [0, 1) for scattering.
pub fn rand01(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    (hash(x, y, z, seed) >> 8) as f32 / (1u32 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_and_determinism() {
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for i in 0..20000 {
            let (x, y) = (i as f32 * 0.137, (i * 7 % 1000) as f32 * 0.071);
            let n = perlin2(x, y, 3);
            lo = lo.min(n);
            hi = hi.max(n);
            assert_eq!(n, perlin2(x, y, 3));
            let r = ridged2(x, y, 1, 5);
            assert!((0.0..=1.0).contains(&r));
            let n3 = perlin3(x, y, x * 0.5, 9);
            assert!(n3.abs() <= 1.01, "{n3}");
        }
        assert!(lo < -0.4 && hi > 0.4, "noise covers its range: {lo} {hi}");
        let (f1, f2) = worley2(3.3, 4.4, 1);
        assert!(f1 <= f2);
        assert!((terrace(10.4, 5.0, 1.0) - 10.0).abs() < 0.1);
    }
}

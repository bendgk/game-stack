//! Hydraulic + thermal erosion — the virtual-pipe shallow-water model of
//! Mei, Decaudin & Hu (2007) as used by Šťava et al., "Interactive Terrain Modeling Using
//! Hydraulic Erosion" (SCA 2008).
//!
//! Each step:
//! 1. **rain** adds water (`d`);
//! 2. **flux**: virtual pipes between 4-neighbours carry water downhill (outflow `f`, scaled so a
//!    cell never drains more than it holds);
//! 3. **water + velocity**: net flow updates depth; the velocity field follows from the fluxes;
//! 4. **erosion / deposition**: transport capacity `C = Kc · sin(tilt) · |v|`; below capacity the
//!    flow picks up sediment (`Ks`), above it drops it (`Kd`);
//! 5. **transport**: sediment moves with the water through the pipes (mass-conserving, unlike
//!    the paper's semi-Lagrangian advection, which leaks a few % per hundred steps);
//! 6. **evaporation**; and **thermal erosion** slumps slopes steeper than the talus angle.
//!
//! The simulation works in cell units (pipe length 1, heights in cells) so the usual parameter
//! sets apply at any map size; [`erode`] converts from/to voxel heights. `erosion_gpu` runs the same steps as
//! WGSL compute shaders; this is the CPU reference.

use crate::heightmap::Heightmap;

#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub iterations: u32,
    pub dt: f32,
    pub rain: f32,
    pub pipe_area: f32,
    pub gravity: f32,
    pub capacity: f32,
    pub suspension: f32,
    pub deposition: f32,
    pub evaporation: f32,
    pub min_tilt: f32,
    /// Water depth (cells) at which flow reaches full carrying capacity.
    pub max_depth: f32,
    /// Thermal erosion: talus slope (tan of the angle) and rate.
    pub talus: f32,
    pub thermal: f32,
    /// Rain varies across the map with noise (0 = uniform).
    pub rain_variation: f32,
    pub seed: u32,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            iterations: 300,
            dt: 0.02,
            rain: 0.012,
            pipe_area: 20.0,
            gravity: 9.81,
            capacity: 1.0,
            suspension: 0.5,
            deposition: 1.0,
            evaporation: 0.015,
            min_tilt: 0.05,
            max_depth: 0.3,
            talus: 0.9,
            thermal: 0.15,
            rain_variation: 0.6,
            seed: 1,
        }
    }
}

/// Simulation state (normalised units). `out` maps for materials after the run.
#[derive(Clone, Debug)]
pub struct State {
    pub w: usize,
    pub h: usize,
    pub b: Vec<f32>,
    pub d: Vec<f32>,
    pub s: Vec<f32>,
    /// Outflow flux: [left, right, top, bottom] per cell.
    pub f: Vec<[f32; 4]>,
    pub v: Vec<[f32; 2]>,
    /// Accumulated |eroded| and |deposited| amounts (for material rules).
    pub eroded: Vec<f32>,
    pub deposited: Vec<f32>,
}

impl State {
    pub fn new(b: Vec<f32>, w: usize, h: usize) -> Self {
        let n = w * h;
        State {
            w,
            h,
            b,
            d: vec![0.0; n],
            s: vec![0.0; n],
            f: vec![[0.0; 4]; n],
            v: vec![[0.0; 2]; n],
            eroded: vec![0.0; n],
            deposited: vec![0.0; n],
        }
    }
}

#[inline]
fn idx(w: usize, x: usize, y: usize) -> usize {
    y * w + x
}

fn par_rows<T: Send>(out: &mut [T], w: usize, f: impl Fn(usize, usize, &mut T) + Sync + Send) {
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, v) in row.iter_mut().enumerate() {
                f(x, y, v);
            }
        });
    }
    #[cfg(not(feature = "parallel"))]
    for (i, v) in out.iter_mut().enumerate() {
        f(i % w, i / w, v);
    }
}

pub fn rain_at(x: usize, y: usize, w: usize, p: &Params) -> f32 {
    let n = crate::noise::fbm2(x as f32 / w as f32 * 4.0, y as f32 / w as f32 * 4.0, p.seed ^ 0x5a1, 3) * 0.5 + 0.5;
    p.rain * (1.0 - p.rain_variation + p.rain_variation * 2.0 * n)
}

/// One simulation step.
pub fn step(st: &mut State, p: &Params, iteration: u32) {
    let (w, h) = (st.w, st.h);
    // Cell units: pipes are one cell long, heights are measured in cells.
    let l = 1.0;
    let dt = p.dt;
    // 1. Rain (stopping for the last 10% so the water settles and evaporates).
    let raining = iteration < p.iterations * 9 / 10;
    let mut d1 = st.d.clone();
    if raining {
        par_rows(&mut d1, w, |x, y, v| *v += dt * rain_at(x, y, w, p));
    }
    // 2. Outflow flux.
    let (b, f_old) = (&st.b, &st.f);
    let mut f = vec![[0.0f32; 4]; w * h];
    par_rows(&mut f, w, |x, y, out| {
        let i = idx(w, x, y);
        let hc = b[i] + d1[i];
        let nb = [
            if x > 0 { Some(idx(w, x - 1, y)) } else { None },
            if x + 1 < w { Some(idx(w, x + 1, y)) } else { None },
            if y > 0 { Some(idx(w, x, y - 1)) } else { None },
            if y + 1 < h { Some(idx(w, x, y + 1)) } else { None },
        ];
        let mut sum = 0.0;
        for k in 0..4 {
            out[k] = match nb[k] {
                Some(j) => (f_old[i][k] + dt * p.pipe_area * p.gravity * (hc - b[j] - d1[j]) / l).max(0.0),
                None => 0.0,
            };
            sum += out[k];
        }
        if sum > 0.0 {
            let kf = (d1[i] * l * l / (sum * dt)).min(1.0);
            for o in out.iter_mut() {
                *o *= kf;
            }
        }
    });
    // 3. Water depth + velocity.
    let mut d2 = vec![0.0f32; w * h];
    let mut v = vec![[0.0f32; 2]; w * h];
    {
        let fr = &f;
        let get = |x: i64, y: i64, k: usize| -> f32 {
            if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 { 0.0 } else { fr[idx(w, x as usize, y as usize)][k] }
        };
        let mut dv: Vec<(f32, [f32; 2])> = vec![(0.0, [0.0; 2]); w * h];
        par_rows(&mut dv, w, |x, y, out| {
            let (xi, yi) = (x as i64, y as i64);
            let i = idx(w, x, y);
            let inflow = get(xi - 1, yi, 1) + get(xi + 1, yi, 0) + get(xi, yi - 1, 3) + get(xi, yi + 1, 2);
            let outflow: f32 = fr[i].iter().sum();
            let dvol = dt * (inflow - outflow);
            let wx = (get(xi - 1, yi, 1) - fr[i][0] + fr[i][1] - get(xi + 1, yi, 0)) * 0.5;
            let wy = (get(xi, yi - 1, 3) - fr[i][2] + fr[i][3] - get(xi, yi + 1, 2)) * 0.5;
            *out = (dvol, [wx, wy]);
        });
        for i in 0..w * h {
            d2[i] = (d1[i] + dv[i].0 / (l * l)).max(0.0);
            let dm = 0.5 * (d1[i] + d2[i]);
            if dm > 1e-5 {
                v[i] = [dv[i].1[0] / (l * dm), dv[i].1[1] / (l * dm)];
            }
        }
    }
    // 4. Erosion / deposition.
    let mut b2 = st.b.clone();
    let mut s2 = st.s.clone();
    {
        let (b, s) = (&st.b, &st.s);
        let mut res: Vec<(f32, f32, f32)> = vec![(0.0, 0.0, 0.0); w * h];
        par_rows(&mut res, w, |x, y, out| {
            let i = idx(w, x, y);
            let bl = b[idx(w, x.saturating_sub(1), y)];
            let br = b[idx(w, (x + 1).min(w - 1), y)];
            let bt = b[idx(w, x, y.saturating_sub(1))];
            let bb = b[idx(w, x, (y + 1).min(h - 1))];
            let gx = (br - bl) / (2.0 * l);
            let gy = (bb - bt) / (2.0 * l);
            let g2 = gx * gx + gy * gy;
            let sin_a = (g2 / (1.0 + g2)).sqrt().max(p.min_tilt);
            let speed = (v[i][0] * v[i][0] + v[i][1] * v[i][1]).sqrt();
            // Shallow flows can't carry much (limits erosion in thin films).
            let depth_lim = (d2[i] / p.max_depth).min(1.0);
            let cap = p.capacity * sin_a * speed * depth_lim;
            let sed = s[i];
            if cap > sed {
                // Never dig below the lowest neighbour: that's what turns channels into trenches.
                let floor = (b[i] - bl.min(br).min(bt).min(bb)).max(0.0);
                let a = (dt * p.suspension * (cap - sed)).min(floor * 0.5);
                *out = (-a, a, a);
            } else {
                let a = dt * p.deposition * (sed - cap);
                *out = (a, -a, -a);
            }
        });
        for i in 0..w * h {
            b2[i] += res[i].0;
            s2[i] = (s2[i] + res[i].1).max(0.0);
            if res[i].2 > 0.0 {
                st.eroded[i] += res[i].2;
            } else {
                st.deposited[i] -= res[i].2;
            }
        }
    }
    // 5. Sediment transport, conservative: sediment leaves through each pipe in the same
    //    proportion as the water does (flux · dt / volume), + 6. evaporation.
    let mut s3 = vec![0.0f32; w * h];
    {
        let frac =
            |j: usize, k: usize| -> f32 { if d1[j] > 1e-6 { (f[j][k] * dt / (d1[j] * l * l)).min(1.0) } else { 0.0 } };
        par_rows(&mut s3, w, |x, y, out| {
            let i = idx(w, x, y);
            let leave: f32 = (0..4).map(|k| frac(i, k)).sum::<f32>().min(1.0);
            let mut v = s2[i] * (1.0 - leave);
            if x > 0 {
                let j = idx(w, x - 1, y);
                v += s2[j] * frac(j, 1);
            }
            if x + 1 < w {
                let j = idx(w, x + 1, y);
                v += s2[j] * frac(j, 0);
            }
            if y > 0 {
                let j = idx(w, x, y - 1);
                v += s2[j] * frac(j, 3);
            }
            if y + 1 < h {
                let j = idx(w, x, y + 1);
                v += s2[j] * frac(j, 2);
            }
            *out = v;
        });
    }
    for x in d2.iter_mut() {
        *x *= 1.0 - p.evaporation * dt;
    }
    // 7. Thermal erosion: move material down slopes steeper than the talus slope.
    let talus_h = p.talus * l;
    let mut moved = vec![0.0f32; w * h];
    {
        let b = &b2;
        let mut give: Vec<[f32; 4]> = vec![[0.0; 4]; w * h];
        par_rows(&mut give, w, |x, y, out| {
            let i = idx(w, x, y);
            let nb = [
                if x > 0 { Some(idx(w, x - 1, y)) } else { None },
                if x + 1 < w { Some(idx(w, x + 1, y)) } else { None },
                if y > 0 { Some(idx(w, x, y - 1)) } else { None },
                if y + 1 < h { Some(idx(w, x, y + 1)) } else { None },
            ];
            let mut total = 0.0;
            let mut ex = [0.0f32; 4];
            let mut maxd = 0.0f32;
            for k in 0..4 {
                if let Some(j) = nb[k] {
                    let dh = b[i] - b[j];
                    if dh > talus_h {
                        ex[k] = dh - talus_h;
                        total += ex[k];
                        maxd = maxd.max(dh);
                    }
                }
            }
            if total > 0.0 {
                let amount = dt * p.thermal * (maxd - talus_h) * 0.5;
                for k in 0..4 {
                    out[k] = amount * ex[k] / total;
                }
            }
        });
        for y in 0..h {
            for x in 0..w {
                let i = idx(w, x, y);
                let out: f32 = give[i].iter().sum();
                moved[i] -= out;
                if x > 0 {
                    moved[idx(w, x - 1, y)] += give[i][0];
                }
                if x + 1 < w {
                    moved[idx(w, x + 1, y)] += give[i][1];
                }
                if y > 0 {
                    moved[idx(w, x, y - 1)] += give[i][2];
                }
                if y + 1 < h {
                    moved[idx(w, x, y + 1)] += give[i][3];
                }
            }
        }
    }
    for i in 0..w * h {
        b2[i] += moved[i];
    }
    st.b = b2;
    st.d = d2;
    st.s = s3;
    st.f = f;
    st.v = v;
}

/// Erode a voxel heightmap in place (CPU). Returns the final state (water, sediment, eroded/deposited maps for materials).
pub fn erode(hm: &mut Heightmap, p: &Params) -> State {
    let (lo, hi) = hm.data.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let _ = hi;
    let range = hm.cell as f32;
    let b: Vec<f32> = hm.data.iter().map(|v| (v - lo) / range).collect();
    let mut st = State::new(b, hm.w, hm.h);
    for it in 0..p.iterations {
        step(&mut st, p, it);
    }
    for (o, b) in hm.data.iter_mut().zip(&st.b) {
        *o = lo + b * range;
    }
    st
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heightmap::Relief;

    #[test]
    fn erosion_carves_and_conserves_roughly() {
        let mut hm = Heightmap::generate(96, 96, 64, Relief::Mountains, 7, 0.0, 3000.0);
        let before = hm.clone();
        let p = Params { iterations: 120, ..Default::default() };
        let st = erode(&mut hm, &p);
        assert!(hm.data.iter().all(|v| v.is_finite()));
        let changed = hm.data.iter().zip(&before.data).filter(|(a, b)| (*a - *b).abs() > 1.0).count();
        assert!(changed > 96 * 96 / 10, "erosion changed {changed} cells");
        let eroded: f32 = st.eroded.iter().sum();
        assert!(eroded > 0.0);
        // Mass: terrain + suspended sediment stays within a few % of the start (semi-Lagrangian
        // transport is not exactly conservative).
        let (lo, hi) = before.data.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        let m0: f32 = before.data.iter().sum::<f32>();
        let _ = (lo, hi);
        let m1: f32 = hm.data.iter().sum::<f32>() + st.s.iter().sum::<f32>() * hm.cell as f32;
        assert!((m1 - m0).abs() / m0.abs().max(1.0) < 0.05, "mass drift {m0} → {m1}");
    }
}

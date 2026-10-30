//! Wave Function Collapse (simple tiled model), deterministic.
//!
//! Every cell starts in a superposition of all tiles (a bitset). Repeatedly: *observe* the
//! lowest-entropy undecided cell (Shannon entropy over the remaining tiles' weights, ties broken
//! by seeded noise), collapse it to one tile drawn by weight, then *propagate*: remove from every
//! neighbour each tile that no remaining tile here allows on that side, until nothing changes.
//! A contradiction (a cell with no options) restarts with the next seed.
//!
//! Tiles are described by four edge labels (N, E, S, W); two tiles may touch when the shared
//! edges carry the same label. Weights are per-cell (a closure), so a field — a density map, a
//! model-chosen preference, a coastline — can steer the collapse without breaking its rules.
//! Cell options can also be pre-constrained (forced roads on the border, water from a noise
//! mask) before solving.

use crate::rng::Rng;

/// Up to 128 tiles.
type Bits = u128;

#[derive(Clone, Debug)]
pub struct Tile {
    pub name: &'static str,
    /// Edge labels: [north (-z), east (+x), south (+z), west (-x)].
    pub edges: [u8; 4],
    pub weight: f32,
}

pub struct Wfc {
    pub w: usize,
    pub h: usize,
    pub tiles: Vec<Tile>,
    /// compat[d][t] = tiles allowed on side d of tile t.
    compat: [Vec<Bits>; 4],
    pub cells: Vec<Bits>,
}

const DX: [i32; 4] = [0, 1, 0, -1];
const DZ: [i32; 4] = [-1, 0, 1, 0];

impl Wfc {
    pub fn new(w: usize, h: usize, tiles: Vec<Tile>) -> Self {
        assert!(tiles.len() <= 128 && !tiles.is_empty());
        let n = tiles.len();
        let mut compat: [Vec<Bits>; 4] = std::array::from_fn(|_| vec![0; n]);
        for (a, ta) in tiles.iter().enumerate() {
            for (b, tb) in tiles.iter().enumerate() {
                for d in 0..4 {
                    if ta.edges[d] == tb.edges[(d + 2) % 4] {
                        compat[d][a] |= 1 << b;
                    }
                }
            }
        }
        let all: Bits = if n == 128 { !0 } else { (1 << n) - 1 };
        Wfc { w, h, tiles, compat, cells: vec![all; w * h] }
    }

    pub fn tile_id(&self, name: &str) -> Option<usize> {
        self.tiles.iter().position(|t| t.name == name)
    }

    /// Tiles allowed on side `d` (N, E, S, W) of tile `t`.
    pub fn compat(&self, d: usize, t: usize) -> u128 {
        self.compat[d][t]
    }

    /// Does `tiles` (one per cell) obey every restriction and adjacency rule?
    pub fn check(&self, tiles: &[usize]) -> bool {
        if tiles.len() != self.w * self.h {
            return false;
        }
        for z in 0..self.h {
            for x in 0..self.w {
                let t = tiles[z * self.w + x];
                if t >= self.tiles.len() || self.cells[z * self.w + x] & (1 << t) == 0 {
                    return false;
                }
                if x + 1 < self.w && self.compat[1][t] & (1 << tiles[z * self.w + x + 1]) == 0 {
                    return false;
                }
                if z + 1 < self.h && self.compat[2][t] & (1 << tiles[(z + 1) * self.w + x]) == 0 {
                    return false;
                }
            }
        }
        true
    }

    /// Restrict a cell to a set of tiles (before solving).
    pub fn restrict(&mut self, x: usize, z: usize, allowed: Bits) {
        self.cells[z * self.w + x] &= allowed;
    }

    fn propagate(&mut self, stack: &mut Vec<usize>) -> bool {
        while let Some(i) = stack.pop() {
            let (x, z) = ((i % self.w) as i32, (i / self.w) as i32);
            for d in 0..4 {
                let (nx, nz) = (x + DX[d], z + DZ[d]);
                if nx < 0 || nz < 0 || nx >= self.w as i32 || nz >= self.h as i32 {
                    continue;
                }
                let mut allowed: Bits = 0;
                let mut bits = self.cells[i];
                while bits != 0 {
                    let t = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    allowed |= self.compat[d][t];
                }
                let j = nz as usize * self.w + nx as usize;
                let before = self.cells[j];
                let after = before & allowed;
                if after != before {
                    if after == 0 {
                        return false;
                    }
                    self.cells[j] = after;
                    stack.push(j);
                }
            }
        }
        true
    }

    /// Solve. `weight(x, z, tile)` scales tile weights per cell. Returns the chosen tile per cell,
    /// or None after `attempts` contradictions.
    pub fn solve(&self, seed: u64, attempts: u32, weight: &dyn Fn(usize, usize, usize) -> f32) -> Option<Vec<usize>> {
        for a in 0..attempts {
            let mut s = Wfc {
                w: self.w,
                h: self.h,
                tiles: self.tiles.clone(),
                compat: self.compat.clone(),
                cells: self.cells.clone(),
            };
            let mut rng = Rng::new(seed.wrapping_add(a as u64 * 7919));
            // Initial propagation of pre-constraints.
            let mut stack: Vec<usize> = (0..s.cells.len()).collect();
            if !s.propagate(&mut stack) {
                return None; // constraints themselves are contradictory
            }
            let mut ok = true;
            loop {
                // Observe: minimum entropy undecided cell.
                let mut best = None;
                let mut best_e = f32::INFINITY;
                for (i, &c) in s.cells.iter().enumerate() {
                    if c.count_ones() <= 1 {
                        continue;
                    }
                    let (x, z) = (i % s.w, i / s.w);
                    let (mut sw, mut swl) = (0.0f32, 0.0f32);
                    let mut bits = c;
                    while bits != 0 {
                        let t = bits.trailing_zeros() as usize;
                        bits &= bits - 1;
                        let w = (s.tiles[t].weight * weight(x, z, t)).max(1e-6);
                        sw += w;
                        swl += w * w.ln();
                    }
                    let e = sw.ln() - swl / sw + rng.f32() * 1e-3;
                    if e < best_e {
                        best_e = e;
                        best = Some(i);
                    }
                }
                let Some(i) = best else { break };
                // Collapse by weight.
                let (x, z) = (i % s.w, i / s.w);
                let opts: Vec<(usize, f32)> = {
                    let mut v = Vec::new();
                    let mut bits = s.cells[i];
                    while bits != 0 {
                        let t = bits.trailing_zeros() as usize;
                        bits &= bits - 1;
                        v.push((t, (s.tiles[t].weight * weight(x, z, t)).max(1e-6)));
                    }
                    v
                };
                let total: f32 = opts.iter().map(|o| o.1).sum();
                let mut r = rng.f32() * total;
                let mut pick = opts[0].0;
                for (t, w) in &opts {
                    if r < *w {
                        pick = *t;
                        break;
                    }
                    r -= w;
                }
                s.cells[i] = 1 << pick;
                if !s.propagate(&mut vec![i]) {
                    ok = false;
                    break;
                }
            }
            if ok {
                return Some(s.cells.iter().map(|c| c.trailing_zeros() as usize).collect());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roads() -> Vec<Tile> {
        // Edge label 1 = road crosses this edge, 0 = no road.
        vec![
            Tile { name: "lot", edges: [0, 0, 0, 0], weight: 3.0 },
            Tile { name: "ew", edges: [0, 1, 0, 1], weight: 1.0 },
            Tile { name: "ns", edges: [1, 0, 1, 0], weight: 1.0 },
            Tile { name: "cross", edges: [1, 1, 1, 1], weight: 0.5 },
        ]
    }

    #[test]
    fn solutions_respect_adjacency_and_are_deterministic() {
        let w = Wfc::new(24, 24, roads());
        let a = w.solve(42, 20, &|_, _, _| 1.0).expect("solvable");
        let b = w.solve(42, 20, &|_, _, _| 1.0).unwrap();
        assert_eq!(a, b);
        for z in 0..24 {
            for x in 0..24 {
                let t = &w.tiles[a[z * 24 + x]];
                if x + 1 < 24 {
                    assert_eq!(t.edges[1], w.tiles[a[z * 24 + x + 1]].edges[3]);
                }
                if z + 1 < 24 {
                    assert_eq!(t.edges[2], w.tiles[a[(z + 1) * 24 + x]].edges[0]);
                }
            }
        }
    }

    #[test]
    fn weights_steer_the_collapse() {
        let w = Wfc::new(16, 16, roads());
        let lots_heavy = w.solve(1, 20, &|_, _, t| if t == 0 { 50.0 } else { 1.0 }).unwrap();
        let roads_heavy = w.solve(1, 20, &|_, _, t| if t == 0 { 0.02 } else { 1.0 }).unwrap();
        let count = |v: &[usize]| v.iter().filter(|&&t| t == 0).count();
        assert!(count(&lots_heavy) > count(&roads_heavy) + 50);
    }
}

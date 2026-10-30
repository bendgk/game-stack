//! The city: a WFC street network → ground tiles → blocks → buildings → props.
//!
//! Layout runs on a coarse grid of cells (each `cm` × `cm` 2 m modules). Every cell collapses to
//! a road piece (straight, corner, T, cross — edges labelled road / no road so streets always
//! connect where they meet), a building lot, a park, a plaza, or water. The model's preferences
//! (density, parks, water, grid-ness) and a downtown field weight the collapse; a regular grid
//! of avenues can be pre-imposed for Manhattan-like plans. Everything is then expanded to 2 m
//! ground tiles, buildings and props — all prefab instances.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use vx_core::compose::{Aabb, BoxShape, Shape};
use vx_core::{Dag, IVec3, Instance, MODULE, Palette};

use crate::building::{self, BuildingSpec, Modules};
use crate::props;
use crate::rng::{Rng, fbm2, hash01};
use crate::spec::{SceneSpec, Style};
use crate::style::Mats;
use crate::tiles::{self, ROAD_TOP, Tile, Tiles, WALK_TOP};
use crate::wfc::{Tile as WTile, Wfc};

/// Voxel y of the bottom of the ground-tile layer; the walking surface is at GROUND + 64.
pub const GROUND: i32 = 512;
pub const WALK_Y: i32 = GROUND + WALK_TOP;
pub const ROAD_Y: i32 = GROUND + ROAD_TOP;
const S: i32 = MODULE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Road([bool; 4]),
    Lot,
    Park,
    Plaza,
    Water,
}

fn wfc_tiles() -> Vec<WTile> {
    let r = |name, e: [u8; 4], w| WTile { name, edges: e, weight: w };
    vec![
        r("lot", [0, 0, 0, 0], 1.0),
        r("park", [0, 0, 0, 0], 1.0),
        r("plaza", [0, 0, 0, 0], 1.0),
        r("water", [0, 0, 0, 0], 1.0),
        r("ns", [1, 0, 1, 0], 1.0),
        r("ew", [0, 1, 0, 1], 1.0),
        r("cross", [1, 1, 1, 1], 0.35),
        r("t_n", [0, 1, 1, 1], 0.25),
        r("t_e", [1, 0, 1, 1], 0.25),
        r("t_s", [1, 1, 0, 1], 0.25),
        r("t_w", [1, 1, 1, 0], 0.25),
        r("ne", [1, 1, 0, 0], 0.15),
        r("es", [0, 1, 1, 0], 0.15),
        r("sw", [0, 0, 1, 1], 0.15),
        r("wn", [1, 0, 0, 1], 0.15),
    ]
}

/// The planned city (before voxels).
pub struct Plan {
    pub n: usize,
    /// Modules per cell.
    pub cm: i32,
    pub cells: Vec<Cell>,
}

impl Plan {
    pub fn cell(&self, x: i32, z: i32) -> Option<Cell> {
        if x < 0 || z < 0 || x >= self.n as i32 || z >= self.n as i32 {
            None
        } else {
            Some(self.cells[z as usize * self.n + x as usize])
        }
    }
    pub fn modules(&self) -> i32 {
        self.n as i32 * self.cm
    }
}

pub fn plan(spec: &SceneSpec) -> Plan {
    plan_with(spec, false)
}

/// Plan the street network. `gpu_wfc` solves it with the GPU collapse (falls back to the CPU
/// solver without a GPU). CPU and GPU are each deterministic, but give different cities.
pub fn plan_with(spec: &SceneSpec, gpu_wfc: bool) -> Plan {
    let modules = (spec.size_m / 2.0) as i32;
    let n = 48usize.min((modules / 4) as usize).max(8);
    let cm = (modules / n as i32).max(4);
    let tiles = wfc_tiles();
    let mut w = Wfc::new(n, n, tiles.clone());
    let id = |s: &str| tiles.iter().position(|t| t.name == s).unwrap();
    let road_mask: u128 = (4..tiles.len()).fold(0, |a, i| a | 1 << i);
    let water_bit: u128 = 1 << id("water");
    let seed = spec.seed;
    let nf = n as f32;
    // Water: low areas of a noise field become water-only; elsewhere water is forbidden.
    let water_thresh = 1.0 - spec.water * 0.55;
    for z in 0..n {
        for x in 0..n {
            let h = fbm2(x as f32 / nf * 3.0, z as f32 / nf * 3.0, seed ^ 0xa11, 4);
            let river = (fbm2(x as f32 / nf * 1.5 + 7.0, z as f32 / nf * 1.5, seed ^ 0xb22, 3) - 0.5).abs();
            let is_water = spec.water > 0.02 && (h > water_thresh || river < spec.water * 0.07);
            if is_water {
                w.restrict(x, z, water_bit);
            } else {
                w.restrict(x, z, !water_bit);
            }
        }
    }
    // A regular avenue grid (Manhattan-ness).
    if spec.grid > 0.35 {
        let period = (3.0 + (1.0 - spec.grid) * 4.0) as usize;
        let has = |e: [u8; 4], d: usize| e[d] == 1;
        for z in 0..n {
            for x in 0..n {
                let mut allowed: u128 = !0;
                let cell = w.cells[z * n + x];
                if cell == water_bit {
                    continue;
                }
                if x % period == period / 2 {
                    allowed &= tiles
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| has(t.edges, 0) && has(t.edges, 2))
                        .fold(0, |a, (i, _)| a | 1 << i);
                }
                if z % period == period / 2 {
                    allowed &= tiles
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| has(t.edges, 1) && has(t.edges, 3))
                        .fold(0, |a, (i, _)| a | 1 << i);
                }
                if allowed != !0 && cell & allowed != 0 {
                    w.restrict(x, z, allowed);
                }
            }
        }
    }
    let center = nf / 2.0;
    let weight = |x: usize, z: usize, t: usize| -> f32 {
        let dx = (x as f32 - center) / center;
        let dz = (z as f32 - center) / center;
        let dist = (dx * dx + dz * dz).sqrt();
        let park_field = fbm2(x as f32 / nf * 4.0 + 3.0, z as f32 / nf * 4.0, seed ^ 0xc33, 3);
        match tiles[t].name {
            "lot" => 0.6 + spec.density * 3.0 * (1.2 - dist * 0.5),
            "park" => spec.parks * 6.0 * park_field * park_field + 0.01,
            "plaza" => spec.plazas * 3.0 * (1.2 - dist).max(0.1) + 0.01,
            "water" => 1.0,
            n if n == "ns" || n == "ew" => 0.7 + spec.grid,
            _ => 1.0 - spec.grid * 0.6,
        }
    };
    #[cfg(feature = "gpu")]
    let gpu = if gpu_wfc { crate::wfc_gpu::device().ok() } else { None };
    #[cfg(not(feature = "gpu"))]
    let _ = gpu_wfc;
    let solve = |w: &Wfc, seed: u64| -> Option<Vec<usize>> {
        #[cfg(feature = "gpu")]
        if let Some((d, q)) = &gpu
            && let Some(s) = w.solve_gpu(d, q, seed, &weight).0
        {
            return Some(s);
        }
        w.solve(seed, 40, &weight)
    };
    let solved = solve(&w, seed).or_else(|| {
        // Over-constrained: drop the avenue grid and retry.
        let mut w2 = Wfc::new(n, n, tiles.clone());
        for (i, c) in w.cells.iter().enumerate() {
            if *c == water_bit {
                w2.restrict(i % n, i / n, water_bit);
            }
        }
        solve(&w2, seed ^ 1)
    });
    let cells = match solved {
        Some(s) => s
            .into_iter()
            .map(|t| match tiles[t].name {
                "lot" => Cell::Lot,
                "park" => Cell::Park,
                "plaza" => Cell::Plaza,
                "water" => Cell::Water,
                _ => {
                    let e = tiles[t].edges;
                    Cell::Road([e[0] == 1, e[1] == 1, e[2] == 1, e[3] == 1])
                }
            })
            .collect(),
        None => {
            // Last resort: a plain grid (never fails).
            (0..n * n)
                .map(|i| {
                    let (x, z) = (i % n, i / n);
                    match (x % 4 == 2, z % 4 == 2) {
                        (true, true) => Cell::Road([true; 4]),
                        (true, false) => Cell::Road([true, false, true, false]),
                        (false, true) => Cell::Road([false, true, false, true]),
                        _ => Cell::Lot,
                    }
                })
                .collect()
        }
    };
    let _ = road_mask;
    Plan { n, cm, cells }
}

/// Prop prefab cache: (name, variant, turns, base) → DAG.
pub struct PropCache<'a> {
    m: &'a Mats,
    palette: &'a Palette,
    cache: Mutex<HashMap<(&'static str, u8, u8, i32), Arc<Dag>>>,
}

impl<'a> PropCache<'a> {
    pub fn new(m: &'a Mats, palette: &'a Palette) -> Self {
        PropCache { m, palette, cache: Mutex::new(HashMap::new()) }
    }
    pub fn get(&self, name: &'static str, variant: u8, turns: u8, base: i32) -> Option<Arc<Dag>> {
        let def = props::find(name)?;
        let key = (def.name, variant, turns % 4, base);
        if let Some(d) = self.cache.lock().unwrap().get(&key) {
            return Some(d.clone());
        }
        let d = props::build(def, variant as u64 * 1_000_003 + 17, turns, base, self.m, self.palette);
        self.cache.lock().unwrap().insert(key, d.clone());
        Some(d)
    }
    pub fn unique(&self) -> usize {
        self.cache.lock().unwrap().len()
    }
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct CityStats {
    pub cells: usize,
    pub roads: usize,
    pub buildings: usize,
    pub floors: i64,
    pub props: usize,
    pub items: usize,
    pub unique_modules: usize,
    pub unique_tiles: usize,
    pub unique_props: usize,
}

/// Emit every shape of the city into `out`. Returns (stats, spawn position in voxels).
pub fn emit(
    spec: &SceneSpec,
    plan: &Plan,
    m: &Mats,
    palette: &Palette,
    out: &mut Vec<Box<dyn Shape>>,
) -> (CityStats, vx_core::Vec3) {
    let tiles = Tiles::new(m, palette);
    let mods = Modules::new(spec.style, m, palette);
    let pcache = PropCache::new(m, palette);
    let mut st = CityStats { cells: plan.cells.len(), ..Default::default() };
    let cm = plan.cm;
    let seed = spec.seed;
    let night_share = spec.lit_windows;
    let mut props_out: Vec<Box<dyn Shape>> = Vec::new();
    let place = |props_out: &mut Vec<Box<dyn Shape>>,
                 name: &'static str,
                 mx: i32,
                 mz: i32,
                 turns: u8,
                 on_road: bool,
                 var_seed: u64,
                 st: &mut CityStats| {
        let Some(def) = props::find(name) else { return };
        let variant = (var_seed % 6) as u8;
        let (y, base) = if def.level == 3 {
            (WALK_Y, 0)
        } else if on_road {
            (GROUND, ROAD_TOP)
        } else {
            (GROUND, WALK_TOP)
        };
        if let Some(d) = pcache.get(name, variant, turns, base) {
            // 2 m props fill module (mx, mz); 8 m props are centred at (160,160) in their cube,
            // so the cube starts two modules back — every origin stays 2 m-aligned.
            let back = if def.level == 3 { 0 } else { 2 * S };
            let origin = IVec3::new(mx * S - back, y, mz * S - back);
            props_out.push(Box::new(Instance { dag: d, origin }));
            st.props += 1;
        }
    };
    let pedestrian_floor = match spec.style {
        Style::Desert => Tile::Sand,
        _ => Tile::Plaza,
    };
    let park_ground = tiles::outside_ground(spec.style);

    for cz in 0..plan.n as i32 {
        for cx in 0..plan.n as i32 {
            let cell = plan.cell(cx, cz).unwrap();
            let (bx, bz) = (cx * cm, cz * cm);
            let road_nb = [
                matches!(plan.cell(cx, cz - 1), Some(Cell::Road(_))),
                matches!(plan.cell(cx + 1, cz), Some(Cell::Road(_))),
                matches!(plan.cell(cx, cz + 1), Some(Cell::Road(_))),
                matches!(plan.cell(cx - 1, cz), Some(Cell::Road(_))),
            ];
            let mut set_tile = |i: i32, j: i32, t: Tile| {
                let (mx, mz) = (bx + i, bz + j);
                let v = (hash01(mx, 0, mz, seed) * 8.0) as u8;
                let v = if matches!(t, Tile::LaneLine { .. } | Tile::Crosswalk { .. } | Tile::Water) { 0 } else { v };
                out.push(Box::new(Instance { dag: tiles.get(t, v), origin: IVec3::new(mx * S, GROUND, mz * S) }));
            };
            match cell {
                Cell::Road(e) => {
                    st.roads += 1;
                    let n_edges = e.iter().filter(|x| **x).count();
                    for j in 0..cm {
                        for i in 0..cm {
                            let edge_n = j == 0;
                            let edge_s = j == cm - 1;
                            let edge_w = i == 0;
                            let edge_e = i == cm - 1;
                            let corner = (edge_n || edge_s) && (edge_w || edge_e);
                            let walk = (edge_w && !e[3])
                                || (edge_e && !e[1])
                                || (edge_n && !e[0])
                                || (edge_s && !e[2])
                                || corner;
                            let t = if walk {
                                Tile::Sidewalk
                            } else if n_edges >= 3 && ((edge_n && e[0]) || (edge_s && e[2])) && !(edge_w || edge_e) {
                                Tile::Crosswalk { axis_x: false }
                            } else if n_edges >= 3 && ((edge_w && e[3]) || (edge_e && e[1])) && !(edge_n || edge_s) {
                                Tile::Crosswalk { axis_x: true }
                            } else if n_edges == 2 && e[0] && e[2] && i == cm / 2 && j % 2 == 0 {
                                Tile::LaneLine { axis_x: false }
                            } else if n_edges == 2 && e[1] && e[3] && j == cm / 2 && i % 2 == 0 {
                                Tile::LaneLine { axis_x: true }
                            } else {
                                Tile::Asphalt
                            };
                            set_tile(i, j, t);
                            let (mx, mz) = (bx + i, bz + j);
                            let h = hash01(mx, 7, mz, seed);
                            if walk && !corner {
                                // Street furniture along the sidewalk, facing the road.
                                let facing: u8 = if edge_w {
                                    1
                                } else if edge_e {
                                    3
                                } else if edge_n {
                                    2
                                } else {
                                    0
                                };
                                let along = if edge_w || edge_e { mz } else { mx };
                                if along % 5 == 0 && h < 0.9 {
                                    place(
                                        &mut props_out,
                                        "street_lamp",
                                        mx,
                                        mz,
                                        facing,
                                        false,
                                        mx as u64 ^ mz as u64,
                                        &mut st,
                                    );
                                } else if along % 5 == 2 && h < spec.trees * 0.9 {
                                    place(
                                        &mut props_out,
                                        "tree",
                                        mx,
                                        mz,
                                        (h * 4.0) as u8,
                                        false,
                                        (h * 1e6) as u64,
                                        &mut st,
                                    );
                                } else if h < spec.props * 0.25 {
                                    let pick = [
                                        "bench",
                                        "trash_bin",
                                        "hydrant",
                                        "mailbox",
                                        "bollard",
                                        "planter",
                                        "bus_shelter",
                                    ];
                                    let name =
                                        pick[(hash01(mx, 3, mz, seed) * pick.len() as f32) as usize % pick.len()];
                                    place(&mut props_out, name, mx, mz, facing, false, (h * 1e7) as u64, &mut st);
                                }
                            } else if !walk && n_edges == 2 && h < spec.props * 0.14 {
                                // Parked / stopped cars, aligned with the street.
                                let ns = e[0] && e[2];
                                let lane_ok = if ns { i == 1 || i == cm - 2 } else { j == 1 || j == cm - 2 };
                                if lane_ok && (if ns { j } else { i }) % 3 == 0 {
                                    let turns = if ns {
                                        if i == 1 { 0 } else { 2 }
                                    } else if j == 1 {
                                        3
                                    } else {
                                        1
                                    };
                                    place(&mut props_out, "car", mx, mz, turns, true, (h * 1e7) as u64, &mut st);
                                }
                            }
                            if corner && n_edges >= 3 && h < 0.7 {
                                place(
                                    &mut props_out,
                                    "traffic_light",
                                    mx,
                                    mz,
                                    (i / (cm - 1) + 2 * (j / (cm - 1))) as u8,
                                    false,
                                    3,
                                    &mut st,
                                );
                            }
                        }
                    }
                }
                Cell::Water => {
                    for j in 0..cm {
                        for i in 0..cm {
                            set_tile(i, j, Tile::Water);
                        }
                    }
                }
                Cell::Park | Cell::Plaza | Cell::Lot => {
                    // Sidewalk ring on road-facing sides.
                    let ring = |i: i32, j: i32| {
                        (j == 0 && road_nb[0])
                            || (i == cm - 1 && road_nb[1])
                            || (j == cm - 1 && road_nb[2])
                            || (i == 0 && road_nb[3])
                    };
                    let (mut x0, mut z0, mut x1, mut z1) = (0, 0, cm, cm);
                    if road_nb[0] {
                        z0 = 1;
                    }
                    if road_nb[2] {
                        z1 = cm - 1;
                    }
                    if road_nb[3] {
                        x0 = 1;
                    }
                    if road_nb[1] {
                        x1 = cm - 1;
                    }
                    let mut footprints: Vec<(i32, i32, i32, i32)> = Vec::new();
                    if cell == Cell::Lot {
                        // Split the interior into 1–3 buildings with 1-module alleys.
                        let mut r = Rng::new(seed ^ (cx as u64) << 20 ^ cz as u64);
                        let (w, d) = (x1 - x0, z1 - z0);
                        let splits = if w >= 10 && r.chance(0.6) { 2 } else { 1 };
                        let bw = (w - (splits - 1)) / splits;
                        for k in 0..splits {
                            let fx = x0 + k * (bw + 1);
                            if bw >= 3 && d >= 3 {
                                footprints.push((fx, z0, bw, d));
                            }
                        }
                    }
                    let in_fp = |i: i32, j: i32| {
                        footprints.iter().any(|&(fx, fz, fw, fd)| i >= fx && i < fx + fw && j >= fz && j < fz + fd)
                    };
                    for j in 0..cm {
                        for i in 0..cm {
                            let t = if ring(i, j) {
                                Tile::Sidewalk
                            } else if cell == Cell::Park {
                                park_ground
                            } else if cell == Cell::Plaza || !in_fp(i, j) {
                                pedestrian_floor
                            } else {
                                Tile::Plaza
                            };
                            set_tile(i, j, t);
                            let (mx, mz) = (bx + i, bz + j);
                            let h = hash01(mx, 11, mz, seed);
                            if cell == Cell::Park && !ring(i, j) {
                                if h < spec.trees * 0.18 && i % 2 == 0 && j % 2 == 0 {
                                    place(
                                        &mut props_out,
                                        "tree",
                                        mx,
                                        mz,
                                        (h * 40.0) as u8,
                                        false,
                                        (h * 1e6) as u64,
                                        &mut st,
                                    );
                                } else if h > 0.97 {
                                    place(
                                        &mut props_out,
                                        if h > 0.985 { "rock" } else { "bench" },
                                        mx,
                                        mz,
                                        (h * 400.0) as u8,
                                        false,
                                        (h * 1e7) as u64,
                                        &mut st,
                                    );
                                } else if h > 0.92 {
                                    place(&mut props_out, "bush", mx, mz, 0, false, (h * 1e7) as u64, &mut st);
                                }
                            }
                            if cell == Cell::Plaza && !ring(i, j) {
                                if i == cm / 2 && j == cm / 2 {
                                    place(&mut props_out, "fountain", mx, mz, 0, false, seed, &mut st);
                                } else if h < spec.props * 0.08 && (i - cm / 2).abs() > 2 {
                                    let pick = ["umbrella_table", "planter", "bench", "tree"];
                                    let name = pick[(h * 1000.0) as usize % pick.len()];
                                    place(
                                        &mut props_out,
                                        name,
                                        mx,
                                        mz,
                                        (h * 77.0) as u8,
                                        false,
                                        (h * 1e7) as u64,
                                        &mut st,
                                    );
                                }
                            }
                            if cell == Cell::Lot && !ring(i, j) && !in_fp(i, j) && h < spec.props * 0.3 {
                                // Back-alley junk: the GMod classics.
                                let pick = [
                                    "dumpster",
                                    "crate_wood",
                                    "barrel_oil",
                                    "barrel_explosive",
                                    "pallet",
                                    "cardboard_box",
                                    "tire",
                                    "traffic_cone",
                                    "crate_metal",
                                    "propane_tank",
                                ];
                                let name = pick[(h * 7919.0) as usize % pick.len()];
                                place(&mut props_out, name, mx, mz, (h * 13.0) as u8, false, (h * 1e7) as u64, &mut st);
                            }
                        }
                    }
                    // Buildings.
                    for (k, &(fx, fz, fw, fd)) in footprints.iter().enumerate() {
                        let (gx, gz) = (bx + fx, bz + fz);
                        let mut r = Rng::new(seed ^ ((gx as u64) << 32) ^ gz as u64 ^ k as u64);
                        let c = plan.modules() as f32 / 2.0;
                        let dist = (((gx as f32 - c).powi(2) + (gz as f32 - c).powi(2)).sqrt() / c).min(1.0);
                        let core = (1.0 - dist).powf(1.5);
                        let noise = fbm2(gx as f32 / 40.0, gz as f32 / 40.0, seed ^ 0x77, 3);
                        let t = (core * spec.downtown + noise * (1.0 - spec.downtown)).clamp(0.0, 1.0);
                        let mut hmax = spec.height_max_m;
                        if matches!(spec.style, Style::Medieval) {
                            hmax = hmax.min(18.0);
                        }
                        let h = spec.height_min_m + (hmax - spec.height_min_m) * t * t * r.range(0.55, 1.0);
                        let floors = (h / 4.0).round().max(1.0) as i32;
                        st.buildings += 1;
                        st.floors += floors as i64;
                        let b = BuildingSpec {
                            x: gx,
                            z: gz,
                            w: fw,
                            d: fd,
                            floors,
                            seed: r.u64(),
                            commercial: road_nb.iter().any(|x| *x) && r.chance(0.6),
                        };
                        building::emit(&b, WALK_Y, &mods, night_share, out);
                        // Rooftop clutter.
                        let top_y = WALK_Y + floors * 2 * S;
                        for q in 0..r.int(0, 3) {
                            let (mx, mz) = (gx + r.int(1, fw - 2), gz + r.int(1, fd - 2));
                            let name = if spec.style == Style::Brick && q == 0 && floors > 3 && fw >= 5 && fd >= 5 {
                                "water_tower"
                            } else {
                                "ac_unit"
                            };
                            if let Some(def) = props::find(name)
                                && let Some(dd) = pcache.get(name, q as u8, r.int(0, 3) as u8, 0)
                            {
                                let back = if def.level == 3 { 0 } else { 2 * S };
                                props_out.push(Box::new(Instance {
                                    dag: dd,
                                    origin: IVec3::new(mx * S - back, top_y, mz * S - back),
                                }));
                                st.props += 1;
                            }
                        }
                        // Ruins: knock holes in the upper floors.
                        if spec.style == Style::Ruins {
                            for _ in 0..r.int(1, 4) {
                                let p = IVec3::new(
                                    (gx + r.int(0, fw)) * S,
                                    WALK_Y + r.int(1, floors.max(2)) * 2 * S,
                                    (gz + r.int(0, fd)) * S,
                                );
                                out.push(Box::new(crate::kit::Blob {
                                    center: p.as_vec3(),
                                    radii: vx_core::Vec3::splat(r.range(60.0, 160.0)),
                                    mat: vx_core::CARVE,
                                    noise: 0.3,
                                    seed: r.u64(),
                                    fill: 1.0,
                                    alt: None,
                                }));
                            }
                        }
                    }
                }
            }
        }
    }
    // Extra props asked for by the prompt: a few of each near the centre plaza/sidewalks.
    let mut r = Rng::new(seed ^ 0xe7);
    for name in &spec.extra_props {
        let Some(def) = props::find(name) else { continue };
        for _ in 0..12 {
            let c = plan.modules() / 2;
            let (mx, mz) = (c + r.int(-30, 30), c + r.int(-30, 30));
            if let Some(Cell::Road(_)) = plan.cell(mx / cm, mz / cm) {
                continue;
            }
            place(&mut props_out, def.name, mx, mz, r.int(0, 3) as u8, false, r.u64(), &mut st);
        }
    }
    out.extend(props_out);
    st.unique_modules = mods.unique();
    st.unique_tiles = tiles.unique();
    st.unique_props = pcache.unique();
    st.items = out.len();
    // Spawn: above the most central road cell.
    let mut spawn =
        vx_core::Vec3::new(plan.modules() as f32 * 32.0, (WALK_Y + 96) as f32, plan.modules() as f32 * 32.0);
    'find: for rad in 0..plan.n as i32 {
        for dz in -rad..=rad {
            for dx in -rad..=rad {
                let (x, z) = (plan.n as i32 / 2 + dx, plan.n as i32 / 2 + dz);
                if let Some(Cell::Road(_)) = plan.cell(x, z) {
                    spawn = vx_core::Vec3::new(
                        ((x * cm + cm / 2) * S) as f32,
                        (WALK_Y + 64) as f32,
                        ((z * cm + cm / 2) * S) as f32,
                    );
                    break 'find;
                }
            }
        }
    }
    (st, spawn)
}

/// Bedrock and the ground outside the city.
pub fn emit_base(world_side: i32, city_side: i32, m: &Mats, style: Style, out: &mut Vec<Box<dyn Shape>>) {
    out.push(Box::new(BoxShape {
        b: Aabb::new(IVec3::ZERO, IVec3::new(world_side, GROUND, world_side)),
        mat: m.bedrock,
    }));
    let top = match style {
        Style::Desert => m.sand,
        Style::Snow => m.snow,
        _ => m.grass,
    };
    let y = IVec3::new(0, GROUND, 0);
    let h = IVec3::new(0, WALK_Y, 0);
    if city_side < world_side {
        out.push(Box::new(BoxShape {
            b: Aabb::new(y + IVec3::new(city_side, 0, 0), h + IVec3::new(world_side, 0, world_side)),
            mat: top,
        }));
        out.push(Box::new(BoxShape {
            b: Aabb::new(y + IVec3::new(0, 0, city_side), h + IVec3::new(city_side, 0, world_side)),
            mat: top,
        }));
    }
}

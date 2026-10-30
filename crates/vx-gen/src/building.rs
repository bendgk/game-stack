//! Buildings assembled from 2 m facade modules.
//!
//! A facade module is one 2 m × 2 m × 2 m prefab cube with a wall slab on its outer face. A
//! storey is two modules tall (4 m): a lower and an upper half. Modules are generated
//! procedurally per style and kind (window / wall / storefront / door / balcony / pilaster) and
//! orientation, a few variants each, then *linked* across the whole city — tens of thousands
//! of windows cost a few hundred unique nodes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use vx_core::compose::{Aabb, BoxShape, Shape};
use vx_core::{Dag, IVec3, Instance, MODULE, Mat, Palette};

use crate::kit::Kit;
use crate::rng::{Rng, hash01};
use crate::spec::Style;
use crate::style::Mats;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Module {
    Window,
    WindowWide,
    Wall,
    Storefront,
    Door,
    Balcony,
    Pilaster,
    Parapet,
}

/// Key for the module prefab cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    kind: Module,
    upper: bool,
    face: u8,
    variant: u8,
    lit: u8,
    wall: Mat,
}

/// Generates and caches facade module prefabs.
pub struct Modules<'a> {
    pub style: Style,
    pub m: &'a Mats,
    pub palette: &'a Palette,
    cache: Mutex<HashMap<Key, Arc<Dag>>>,
}

const S: i32 = MODULE; // 64 voxels = 2 m
const T: i32 = 10; // wall thickness ≈ 31 cm

impl<'a> Modules<'a> {
    pub fn new(style: Style, m: &'a Mats, palette: &'a Palette) -> Self {
        Modules { style, m, palette, cache: Mutex::new(HashMap::new()) }
    }

    /// Prefab for a module on facade `face` (0 = −z/north, 1 = +x, 2 = +z, 3 = −x).
    /// `lit`: 0 dark glass, 1 warm lit, 2 cool lit.
    pub fn get(&self, kind: Module, upper: bool, face: u8, variant: u8, lit: u8, wall: Mat) -> Arc<Dag> {
        let key = Key { kind, upper, face, variant, lit, wall };
        if let Some(d) = self.cache.lock().unwrap().get(&key) {
            return d.clone();
        }
        let d = self.build(key);
        self.cache.lock().unwrap().insert(key, d.clone());
        d
    }

    pub fn unique(&self) -> usize {
        self.cache.lock().unwrap().len()
    }

    /// Canonical module: facade on the −z face (z ∈ [0, T)), rotated afterwards.
    fn build(&self, key: Key) -> Arc<Dag> {
        let m = self.m;
        let mut k = Kit::new();
        let wall = key.wall;
        let glass = match key.lit {
            1 => m.glass_lit,
            2 => m.glass_lit_cool,
            _ => {
                if self.style == Style::Futuristic {
                    m.glass
                } else {
                    m.glass_dark
                }
            }
        };
        let style = self.style;
        let v = key.variant as i32;
        // Wall slab (+ a recessed inner layer so windows have depth).
        let wall_slab = |k: &mut Kit| {
            k.bx([0, 0, 0], [S, S, T], wall);
        };
        match key.kind {
            Module::Wall => {
                wall_slab(&mut k);
                if style == Style::Brick || style == Style::Medieval {
                    // Coursing: thin darker joints every 8 voxels (25 cm).
                    for y in (0..S).step_by(8) {
                        k.bx([0, y, 0], [S, y + 1, 1], m.trim);
                    }
                }
            }
            Module::Pilaster => {
                wall_slab(&mut k);
                k.bx([20, 0, -6], [44, S, 0], m.trim);
            }
            Module::Window | Module::WindowWide => {
                wall_slab(&mut k);
                let (x0, x1) = if key.kind == Module::WindowWide { (6, 58) } else { (14 - v % 3 * 2, 50 + v % 3 * 2) };
                if !key.upper {
                    // Lower half: sill + glass from 1/3 up.
                    let y0 = 24 - v % 2 * 6;
                    k.carve([x0, y0, 0], [x1, S, T]);
                    k.bx([x0, y0, 6], [x1, S, 7], glass);
                    k.bx([x0 - 2, y0 - 3, -3], [x1 + 2, y0, T], m.trim); // sill
                    if style == Style::Modern || style == Style::Futuristic {
                        k.bx([31, y0, 4], [33, S, 6], m.steel_dark); // mullion
                    }
                } else {
                    // Upper half: glass up to the lintel, then spandrel.
                    let y1 = 40 + v % 2 * 6;
                    k.carve([x0, 0, 0], [x1, y1, T]);
                    k.bx([x0, 0, 6], [x1, y1, 7], glass);
                    k.bx([x0 - 2, y1, 0], [x1 + 2, y1 + 4, 2], m.trim); // lintel
                    if style == Style::Modern || style == Style::Futuristic {
                        k.bx([31, 0, 4], [33, y1, 6], m.steel_dark);
                        k.bx([x0, y1 - 1, 4], [x1, y1, 6], m.steel_dark);
                    }
                    if style == Style::Cyberpunk && v % 3 == 0 {
                        let neon = m.neon[(v as usize / 3) % 4];
                        k.bx([0, S - 4, -2], [S, S - 2, 0], neon);
                    }
                    if style == Style::Brick && v % 2 == 1 {
                        // Arched top: carve a half-disc.
                        k.cyl(2, [32.0, y1 as f32, 0.0], (x1 - x0) as f32 * 0.5, 0.0, T as f32, vx_core::CARVE);
                    }
                }
            }
            Module::Storefront => {
                wall_slab(&mut k);
                if !key.upper {
                    k.carve([2, 0, 0], [62, S, T]);
                    k.bx([2, 2, 5], [62, S, 6], if key.lit > 0 { m.glass_lit } else { m.glass });
                    k.bx([2, 0, 0], [62, 2, T], m.trim);
                    k.bx([30, 2, 3], [34, S, 5], m.steel_dark);
                } else {
                    k.carve([2, 0, 0], [62, 20, T]);
                    k.bx([2, 0, 5], [62, 20, 6], if key.lit > 0 { m.glass_lit } else { m.glass });
                    // Sign band + awning.
                    k.bx([0, 22, -2], [S, 34, 0], m.accent);
                    if style == Style::Cyberpunk {
                        k.bx([6, 26, -3], [58, 30, -2], m.neon[(v as usize) % 4]);
                    }
                    if v % 2 == 0 {
                        for z in 0..24 {
                            let y = 20 - z / 3;
                            k.bx([0, y, -z - 1], [S, y + 1, -z], m.accent);
                        }
                    }
                }
            }
            Module::Door => {
                wall_slab(&mut k);
                if !key.upper {
                    k.carve([18, 0, 0], [46, S, T]);
                    k.bx([18, 0, 7], [46, S, 8], m.wood_dark);
                    k.bx([40, 28, 5], [42, 34, 7], m.chrome);
                    k.bx([14, 0, -2], [50, 3, T], m.concrete); // step
                } else {
                    k.carve([18, 0, 0], [46, 14, T]);
                    k.bx([18, 0, 6], [46, 14, 7], glass);
                    k.bx([14, 14, -1], [50, 18, 1], m.trim);
                    k.bx([16, 24, -8], [48, 26, 0], m.trim); // canopy
                }
            }
            Module::Balcony => {
                // Window lower half behind a balcony slab + railing.
                wall_slab(&mut k);
                if !key.upper {
                    k.carve([10, 6, 0], [54, S, T]);
                    k.bx([10, 6, 6], [54, S, 7], glass);
                    k.bx([0, 0, -26], [S, 4, 0], m.concrete); // slab
                    k.bx([0, 4, -26], [S, 6, -24], m.steel_dark);
                    k.bx([0, 26, -26], [S, 28, -24], m.steel_dark);
                    for x in (2..S).step_by(6) {
                        k.bx([x, 4, -26], [x + 1, 26, -25], m.steel_dark);
                    }
                } else {
                    k.carve([10, 0, 0], [54, 44, T]);
                    k.bx([10, 0, 6], [54, 44, 7], glass);
                }
            }
            Module::Parapet => {
                k.bx([0, 0, 0], [S, 18, 8], wall);
                k.bx([0, 18, -1], [S, 22, 9], m.trim);
                if style == Style::Medieval || style == Style::Ruins {
                    for x in (0..S).step_by(16) {
                        k.carve([x, 12, 0], [x + 8, 22, 9]); // crenels
                    }
                }
            }
        }
        // The module cube sits just *outside* the building, on the 2 m grid. Canonical facade:
        // the wall hugs the cube's +z face (the side touching the building), so shift the parts
        // (authored with the wall at z ∈ [0, T) and outward details at z < 0) by S − T.
        let shift = S - T;
        let parts = std::mem::take(&mut k.parts);
        for p in parts {
            k.parts.push(Box::new(crate::kit::Moved { inner: Arc::from(p), by: IVec3::new(0, 0, shift) }));
        }
        // Rotate so the building side (+z canonical) faces the building: face 0 = north (−z).
        let turns = match key.face {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 3,
        };
        k.rotate(turns, S).build(self.palette, vx_core::MODULE_LEVEL)
    }
}

/// The 2 m-aligned origin of facade module `i` on `face`, just outside the footprint.
fn cube_origin(face: u8, i: i32, x0: i32, z0: i32, w: i32, d: i32) -> (i32, i32) {
    match face {
        0 => (x0 + i * S, z0 - S),
        1 => (x0 + w * S, z0 + i * S),
        2 => (x0 + (w - 1 - i) * S, z0 + d * S),
        _ => (x0 - S, z0 + (d - 1 - i) * S),
    }
}

/// One building to place.
pub struct BuildingSpec {
    /// Footprint origin (module units) and size (modules, ≥ 2).
    pub x: i32,
    pub z: i32,
    pub w: i32,
    pub d: i32,
    /// Storeys (each 2 modules = 4 m).
    pub floors: i32,
    pub seed: u64,
    pub commercial: bool,
}

/// Emit the shapes for a building whose ground floor sits at voxel height `ground`.
/// Facade modules sit *outside* the core by 26 voxels (the canonical shift), so the outer
/// wall surface is flush with the footprint edge.
pub fn emit(b: &BuildingSpec, ground: i32, mods: &Modules, lit_share: f32, out: &mut Vec<Box<dyn Shape>>) {
    let m = mods.m;
    let mut r = Rng::new(b.seed);
    let wall = *r.pick(&m.walls);
    let x0 = b.x * S;
    let z0 = b.z * S;
    let (w, d) = (b.w, b.d);
    let style = mods.style;
    let has_balconies =
        matches!(style, Style::Modern | Style::Tropical | Style::Desert | Style::Brick) && r.chance(0.35);
    let wide = matches!(style, Style::Modern | Style::Futuristic) && r.chance(0.5);
    let pilaster_every = if matches!(style, Style::Brick | Style::Medieval) { r.int(3, 5) } else { 0 };
    let floors = b.floors.max(1);
    let top = ground + floors * 2 * S;
    // Interior: floor slabs over the whole footprint (facade modules sit outside it, so they
    // never overlap and both stay prefab-linked) and a dark core seen through the windows.
    for f in 1..=floors {
        let y = ground + f * 2 * S;
        out.push(Box::new(BoxShape {
            b: Aabb::new(IVec3::new(x0, y - 8, z0), IVec3::new(x0 + w * S, y, z0 + d * S)),
            mat: if f == floors { m.roof } else { m.concrete },
        }));
    }
    if w > 4 && d > 4 {
        out.push(Box::new(BoxShape {
            b: Aabb::new(
                IVec3::new(x0 + 2 * S, ground, z0 + 2 * S),
                IVec3::new(x0 + (w - 2) * S, top - 8, z0 + (d - 2) * S),
            ),
            mat: m.concrete_dark,
        }));
    }
    // Corner columns where two facades meet.
    for (cx, cz) in [(x0 - T, z0 - T), (x0 + w * S, z0 - T), (x0 - T, z0 + d * S), (x0 + w * S, z0 + d * S)] {
        out.push(Box::new(BoxShape {
            b: Aabb::new(IVec3::new(cx, ground, cz), IVec3::new(cx + T, top + 22, cz + T)),
            mat: wall,
        }));
    }
    // Facades. For each face, the modules along it; module cubes are placed so the canonical
    // facade (at local z = 26) lands on the footprint edge.
    for face in 0..4u8 {
        let n = if face % 2 == 0 { w } else { d };
        for i in 0..n {
            for f in 0..floors {
                for upper in [false, true] {
                    let y = ground + f * 2 * S + if upper { S } else { 0 };
                    let mut kind = if pilaster_every > 0 && i % pilaster_every == 0 {
                        Module::Pilaster
                    } else if wide {
                        Module::WindowWide
                    } else {
                        Module::Window
                    };
                    if f == 0 {
                        kind = if b.commercial {
                            if i == n / 2 && face == 0 { Module::Door } else { Module::Storefront }
                        } else if i == n / 2 && face == 0 {
                            Module::Door
                        } else {
                            kind
                        };
                    } else if has_balconies && kind == Module::Window && (i + f) % 3 == 0 {
                        kind = Module::Balcony;
                    }
                    if i == 0 || i == n - 1 {
                        kind = if f == 0 && b.commercial { Module::Storefront } else { Module::Wall };
                    }
                    let variant = (hash01(i, f, face as i32, b.seed) * 6.0) as u8;
                    // Lit state is per window, so both halves of a window agree.
                    let lit = {
                        let h = hash01(i * 7 + 1, f, face as i32 + 11, b.seed);
                        if h < lit_share * 0.75 {
                            1
                        } else if h < lit_share {
                            2
                        } else {
                            0
                        }
                    };
                    let pre = mods.get(kind, upper, face, variant, lit, wall);
                    let (ox, oz) = cube_origin(face, i, x0, z0, w, d);
                    out.push(Box::new(Instance { dag: pre, origin: IVec3::new(ox, y, oz) }));
                }
            }
        }
        // Parapet on the roof edge.
        for i in 0..n {
            let pre = mods.get(Module::Parapet, false, face, 0, 0, wall);
            let (ox, oz) = cube_origin(face, i, x0, z0, w, d);
            out.push(Box::new(Instance { dag: pre, origin: IVec3::new(ox, top, oz) }));
        }
    }
    // Medieval: a pitched roof instead of the flat one.
    if style == Style::Medieval || style == Style::Snow {
        let (cx, cz) = (x0 + w * S / 2, z0 + d * S / 2);
        let half = (w.min(d) * S / 2) as f32;
        let along_x = w >= d;
        let roof_m = m.roof_tile;
        let snow = if style == Style::Snow { Some(m.snow) } else { None };
        let bb = Aabb::new(
            IVec3::new(x0 - 16, top, z0 - 16),
            IVec3::new(x0 + w * S + 16, top + half as i32 + 8, z0 + d * S + 16),
        );
        out.push(Box::new(vx_core::FnShape {
            b: bb,
            sample: move |p: IVec3| {
                let dist = if along_x { (p.z - cz).abs() } else { (p.x - cx).abs() } as f32;
                let h = (half + 16.0 - dist) * 0.9;
                let y = (p.y - top) as f32;
                if y < h && y > h - 10.0 {
                    if let Some(s) = snow
                        && y > h - 3.0
                    {
                        return s;
                    }
                    roof_m
                } else {
                    vx_core::AIR
                }
            },
            // The roof is a 10-voxel shell under the gable line: decide whole cubes from the
            // range of that line over the cube, so only voxels on the shell's surface are sampled.
            classify: move |c: Aabb| {
                let (lo, hi) =
                    if along_x { (c.min.z - cz, c.max.z - 1 - cz) } else { (c.min.x - cx, c.max.x - 1 - cx) };
                let dmin = if lo <= 0 && hi >= 0 { 0 } else { lo.abs().min(hi.abs()) } as f32;
                let dmax = lo.abs().max(hi.abs()) as f32;
                let (hmax, hmin) = ((half + 16.0 - dmin) * 0.9, (half + 16.0 - dmax) * 0.9);
                let (y0, y1) = ((c.min.y - top) as f32, (c.max.y - 1 - top) as f32);
                if y0 >= hmax || y1 <= hmin - 10.0 {
                    vx_core::Class::Empty
                } else if snow.is_none() && y1 < hmin && y0 > hmax - 10.0 {
                    vx_core::Class::Solid(roof_m)
                } else {
                    vx_core::Class::Mixed
                }
            },
        }));
    }
}

//! Sculpt: a small modelling language for voxel models, written by people and agents.
//!
//! A [`ModelSpec`] is a list of operations in **metres**, Y up, with the model's origin at the
//! centre of its floor (x = z = 0, y = 0) and its front facing −z. Operations paint in order;
//! a later one covers earlier ones, and `"mat": "air"` carves. Groups move, turn, mirror and
//! repeat other operations, so a chair is a seat, a back and one leg mirrored twice.
//!
//! ```json
//! { "name": "stool", "ops": [
//!   { "op": "cylinder", "base": [0, 0.45, 0], "r": 0.2, "h": 0.05, "mat": "oak" },
//!   { "op": "group", "mirror": ["x", "z"], "ops": [
//!     { "op": "line", "a": [0.14, 0, 0.14], "b": [0.12, 0.45, 0.12], "r": 0.025, "mat": "dark_oak" } ] } ] }
//! ```
//!
//! A spec builds into a prefab DAG ([`build`]) of 2 m, 8 m, 32 m or 128 m, whichever fits, and
//! is addressed by the hash of its canonical JSON, so the same spec is the same model
//! everywhere. Models are built where they are made (the server) and shipped as voxels, so
//! the floating-point maths here never has to agree across machines.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use vx_core::compose::{Aabb, CARVE, Class, Shape};
use vx_core::glam::{Mat3, Vec3};
use vx_core::{AIR, Dag, IVec3, Mat, Palette, prefab};

use crate::blocks;

/// Voxels per metre.
const VPM: f32 = 32.0;
/// Hard limits, so a runaway spec can't take the server down.
const MAX_OPS: usize = 4000;
const MAX_LEVEL: u32 = 6;
const MAX_GRID: usize = 256 * 256;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelSpec {
    pub name: String,
    #[serde(default = "default_category")]
    pub category: String,
    pub ops: Vec<Op>,
}

fn default_category() -> String {
    "custom".into()
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    X,
    #[default]
    Y,
    Z,
}

impl Axis {
    fn idx(self) -> usize {
        match self {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Axis-aligned box from corner `min` to corner `max` (or `at` = centre + `size`).
    Box {
        #[serde(default)]
        min: Option<[f32; 3]>,
        #[serde(default)]
        max: Option<[f32; 3]>,
        #[serde(default)]
        at: Option<[f32; 3]>,
        #[serde(default)]
        size: Option<[f32; 3]>,
        mat: String,
    },
    /// Ellipsoid (sphere if `r` has one value) centred on `c`.
    Sphere { c: [f32; 3], r: Radius, mat: String },
    /// Cylinder or cone frustum: from `base` (centre of the bottom disc) `h` metres along
    /// `axis`, radius `r` at the base and `r2` (default `r`) at the top.
    Cylinder {
        base: [f32; 3],
        r: f32,
        #[serde(default)]
        r2: Option<f32>,
        h: f32,
        #[serde(default)]
        axis: Axis,
        mat: String,
    },
    /// Capsule (rounded rod) from `a` to `b` of radius `r`: legs, pipes, branches, rails.
    Line { a: [f32; 3], b: [f32; 3], r: f32, mat: String },
    /// Ring around `axis` through `c`: radius `big` to the tube centre, tube radius `small`.
    Torus {
        c: [f32; 3],
        big: f32,
        small: f32,
        #[serde(default)]
        axis: Axis,
        mat: String,
    },
    /// Gable roof filling the box `min`..`max`: the ridge runs along `ridge` ("x" or "z") at the
    /// top, sloping down to the eaves on the other two sides. `thickness` > 0 makes it a shell.
    Roof {
        min: [f32; 3],
        max: [f32; 3],
        #[serde(default = "default_ridge")]
        ridge: Axis,
        #[serde(default)]
        thickness: f32,
        mat: String,
        /// Close the two gable ends with this material (a wall 0.25 m thick, set `ends_inset`
        /// in from the roof's edge — usually the overhang, so it lines up with the walls).
        #[serde(default)]
        ends: Option<String>,
        #[serde(default)]
        ends_inset: Option<f32>,
    },
    /// Wedge (ramp, stairs' stringer, shed roof) in the box: full height at the low side of
    /// `rise` axis ("x" or "z"), zero at the high side; `flip` reverses it.
    Wedge {
        min: [f32; 3],
        max: [f32; 3],
        #[serde(default = "default_ridge")]
        rise: Axis,
        #[serde(default)]
        flip: bool,
        mat: String,
    },
    /// Pixel art in 3D: `layers` bottom-up, each a list of rows (−z → +z), each row a string
    /// of characters (−x → +x) looked up in `key` (" " or "." = nothing). Each cell is `cell`
    /// metres (default one voxel, 1/32 m); `at` is the min corner.
    Voxels {
        at: [f32; 3],
        #[serde(default)]
        cell: Option<f32>,
        layers: Vec<Vec<String>>,
        key: BTreeMap<String, String>,
    },
    /// Transform other operations: mirror (copies across x = 0 / z = 0), turn `rot` degrees
    /// about +y, move by `at`, and repeat `count` times stepping by `step`.
    Group {
        ops: Vec<Op>,
        #[serde(default)]
        at: Option<[f32; 3]>,
        #[serde(default)]
        rot: Option<f32>,
        #[serde(default)]
        mirror: Vec<Axis>,
        #[serde(default)]
        count: Option<u32>,
        #[serde(default)]
        step: Option<[f32; 3]>,
    },
}

fn default_ridge() -> Axis {
    Axis::X
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Radius {
    One(f32),
    Three([f32; 3]),
}

impl Radius {
    fn v(&self) -> Vec3 {
        match self {
            Radius::One(r) => Vec3::splat(*r),
            Radius::Three(r) => Vec3::from_array(*r),
        }
    }
}

/// A primitive in model space (metres) after group transforms. `inv` maps model → local.
#[derive(Clone)]
enum Kind {
    Box { lo: Vec3, hi: Vec3 },
    Ellipsoid { c: Vec3, r: Vec3 },
    Cone { base: Vec3, axis: usize, r0: f32, r1: f32, h: f32 },
    Capsule { a: Vec3, b: Vec3, r: f32 },
    Torus { c: Vec3, axis: usize, big: f32, small: f32 },
    Roof { lo: Vec3, hi: Vec3, ridge: usize, thick: f32 },
    Wedge { lo: Vec3, hi: Vec3, rise: usize, flip: bool },
    Grid { at: Vec3, cell: f32, dims: [usize; 3], cells: Vec<Mat> },
}

#[derive(Clone)]
struct Prim {
    kind: Kind,
    /// model → local: `inv * (p - t)`.
    inv: Mat3,
    t: Vec3,
    mat: Mat,
    /// Voxel bounds in the prefab cube.
    vb: Aabb,
    convex: bool,
}

#[derive(Clone, Copy)]
struct Xf {
    m: Mat3,
    t: Vec3,
}

impl Xf {
    const ID: Xf = Xf { m: Mat3::IDENTITY, t: Vec3::ZERO };
    fn then(self, inner: Xf) -> Xf {
        // self ∘ inner
        Xf { m: self.m * inner.m, t: self.m * inner.t + self.t }
    }
}

fn finite(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite() && x.abs() < 1000.0)
}

impl Kind {
    /// Local bounds (metres).
    fn bounds(&self) -> (Vec3, Vec3) {
        match self {
            Kind::Box { lo, hi } | Kind::Roof { lo, hi, .. } | Kind::Wedge { lo, hi, .. } => (*lo, *hi),
            Kind::Ellipsoid { c, r } => (*c - *r, *c + *r),
            Kind::Cone { base, axis, r0, r1, h } => {
                let r = r0.max(*r1);
                let mut lo = *base - Vec3::splat(r);
                let mut hi = *base + Vec3::splat(r);
                lo[*axis] = base[*axis] + h.min(0.0);
                hi[*axis] = base[*axis] + h.max(0.0);
                (lo, hi)
            }
            Kind::Capsule { a, b, r } => (a.min(*b) - Vec3::splat(*r), a.max(*b) + Vec3::splat(*r)),
            Kind::Torus { c, axis, big, small } => {
                let mut e = Vec3::splat(big + small);
                e[*axis] = *small;
                (*c - e, *c + e)
            }
            Kind::Grid { at, cell, dims, .. } => {
                (*at, *at + Vec3::new(dims[0] as f32, dims[1] as f32, dims[2] as f32) * *cell)
            }
        }
    }

    /// Is local point `q` inside? Returns the grid's material for grids.
    fn inside(&self, q: Vec3) -> Option<Mat> {
        let yes = |b: bool| if b { Some(AIR) } else { None };
        match self {
            Kind::Box { lo, hi } => yes(q.cmpge(*lo).all() && q.cmplt(*hi).all()),
            Kind::Ellipsoid { c, r } => yes(((q - *c) / r.max(Vec3::splat(1e-4))).length_squared() <= 1.0),
            Kind::Cone { base, axis, r0, r1, h } => {
                let d = q - *base;
                let along = d[*axis];
                let t = if h.abs() < 1e-6 { -1.0 } else { along / h };
                if !(0.0..1.0).contains(&t) {
                    return None;
                }
                let mut rad = d;
                rad[*axis] = 0.0;
                let r = r0 + (r1 - r0) * t;
                yes(rad.length_squared() <= r * r)
            }
            Kind::Capsule { a, b, r } => {
                let ab = *b - *a;
                let t = ((q - *a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
                yes((q - (*a + ab * t)).length_squared() <= r * r)
            }
            Kind::Torus { c, axis, big, small } => {
                let d = q - *c;
                let along = d[*axis];
                let mut rad = d;
                rad[*axis] = 0.0;
                let ring = rad.length() - big;
                yes(ring * ring + along * along <= small * small)
            }
            Kind::Roof { lo, hi, ridge, thick } => {
                if !(q.cmpge(*lo).all() && q.cmplt(*hi).all()) {
                    return None;
                }
                let across = if *ridge == 0 { 2 } else { 0 };
                let w = (hi[across] - lo[across]).max(1e-4);
                let u = (q[across] - lo[across]) / w;
                let top = lo.y + (hi.y - lo.y) * (1.0 - (2.0 * u - 1.0).abs());
                if q.y >= top {
                    return None;
                }
                // Shell: within `thick` (measured vertically) under the slope.
                yes(*thick <= 0.0 || q.y >= top - thick)
            }
            Kind::Wedge { lo, hi, rise, flip } => {
                if !(q.cmpge(*lo).all() && q.cmplt(*hi).all()) {
                    return None;
                }
                let u = (q[*rise] - lo[*rise]) / (hi[*rise] - lo[*rise]).max(1e-4);
                let u = if *flip { u } else { 1.0 - u };
                yes(q.y < lo.y + (hi.y - lo.y) * u)
            }
            Kind::Grid { at, cell, dims, cells } => {
                let g = ((q - *at) / *cell).floor();
                if g.cmplt(Vec3::ZERO).any() {
                    return None;
                }
                let (x, y, z) = (g.x as usize, g.y as usize, g.z as usize);
                if x >= dims[0] || y >= dims[1] || z >= dims[2] {
                    return None;
                }
                let m = cells[x + z * dims[0] + y * dims[0] * dims[2]];
                if m == AIR { None } else { Some(m) }
            }
        }
    }
}

/// Model metres → prefab voxels: origin at (half, 0, half).
fn to_vox(p: Vec3, half: f32) -> Vec3 {
    p * VPM + Vec3::new(half, 0.0, half)
}

impl Prim {
    fn new(kind: Kind, xf: Xf, mat: Mat, half: f32) -> Option<Prim> {
        let inv = xf.m.inverse();
        if !inv.is_finite() {
            return None;
        }
        let (lo, hi) = kind.bounds();
        let mut a = Vec3::splat(f32::MAX);
        let mut b = Vec3::splat(f32::MIN);
        for i in 0..8 {
            let c = Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            );
            let w = to_vox(xf.m * c + xf.t, half);
            a = a.min(w);
            b = b.max(w);
        }
        if !a.is_finite() || !b.is_finite() {
            return None;
        }
        let vb = Aabb::new(a.floor().as_ivec3() - IVec3::ONE, b.ceil().as_ivec3() + IVec3::ONE);
        let convex = !matches!(kind, Kind::Torus { .. } | Kind::Grid { .. });
        Some(Prim { kind, inv, t: xf.t, mat, vb, convex })
    }

    fn local(&self, p: Vec3, half: f32) -> Vec3 {
        let model = (p - Vec3::new(half, 0.0, half)) / VPM;
        self.inv * (model - self.t)
    }
}

struct Placed<'a> {
    p: &'a Prim,
    half: f32,
}

impl Shape for Placed<'_> {
    fn bounds(&self) -> Aabb {
        self.p.vb
    }
    fn classify(&self, cube: Aabb) -> Class {
        if !self.p.vb.overlaps(&cube) {
            return Class::Empty;
        }
        if self.p.convex {
            // A convex shape holding all 8 corner voxels holds the whole cube.
            let (a, b) = (cube.min.as_vec3() + 0.5, cube.max.as_vec3() - 0.5);
            let all = (0..8).all(|i| {
                let c = Vec3::new(
                    if i & 1 == 0 { a.x } else { b.x },
                    if i & 2 == 0 { a.y } else { b.y },
                    if i & 4 == 0 { a.z } else { b.z },
                );
                self.p.kind.inside(self.p.local(c, self.half)).is_some()
            });
            if all {
                return Class::Solid(if self.p.mat == AIR { CARVE } else { self.p.mat });
            }
        }
        Class::Mixed
    }
    fn sample(&self, p: IVec3) -> Mat {
        let q = self.p.local(p.as_vec3() + Vec3::splat(0.5), self.half);
        match self.p.kind.inside(q) {
            None => AIR,
            Some(AIR) => {
                if self.p.mat == AIR {
                    CARVE
                } else {
                    self.p.mat
                }
            }
            Some(m) => m,
        }
    }
}

/// A built model.
pub struct Model {
    pub dag: Dag,
    /// Bounds of the solid voxels in the prefab cube: [min, max).
    pub min: [i32; 3],
    pub max: [i32; 3],
    pub voxels: u64,
    pub warnings: Vec<String>,
}

fn rotation_y(deg: f32) -> Mat3 {
    // Quarter turns exactly (no 1e-8 drift between mirrored halves).
    let q = deg / 90.0;
    if (q - q.round()).abs() < 1e-6 {
        let t = (q.round() as i32).rem_euclid(4);
        let (s, c) = [(0.0, 1.0), (1.0, 0.0), (0.0, -1.0), (-1.0, 0.0)][t as usize];
        return Mat3::from_cols(Vec3::new(c, 0.0, -s), Vec3::Y, Vec3::new(s, 0.0, c));
    }
    Mat3::from_rotation_y(deg.to_radians())
}

fn mat_of(pal: &Palette, name: &str, warn: &mut Vec<String>) -> Mat {
    match blocks::resolve(pal, name) {
        Some(m) => m,
        None => {
            if warn.len() < 20 {
                warn.push(format!("unknown material '{name}' (used concrete)"));
            }
            blocks::resolve(pal, "concrete").unwrap_or(1)
        }
    }
}

fn v3(a: [f32; 3]) -> Vec3 {
    Vec3::from_array(a)
}

fn flatten(op: &Op, xf: Xf, pal: &Palette, out: &mut Vec<(Kind, Xf, Mat)>, warn: &mut Vec<String>, depth: u32) {
    if out.len() >= MAX_OPS || depth > 12 {
        return;
    }
    fn push(out: &mut Vec<(Kind, Xf, Mat)>, warn: &mut Vec<String>, pal: &Palette, xf: Xf, k: Kind, m: &str, ok: bool) {
        if ok {
            out.push((k, xf, mat_of(pal, m, warn)));
        } else if warn.len() < 20 {
            warn.push("skipped an op with bad numbers".into());
        }
    }
    let push = |out: &mut Vec<_>, warn: &mut Vec<_>, k: Kind, m: &str, ok: bool| push(out, warn, pal, xf, k, m, ok);
    match op {
        Op::Box { min, max, at, size, mat } => {
            let (lo, hi) = match (min, max, at, size) {
                (Some(a), Some(b), _, _) => (v3(*a).min(v3(*b)), v3(*a).max(v3(*b))),
                (_, _, Some(c), Some(s)) => (v3(*c) - v3(*s).abs() / 2.0, v3(*c) + v3(*s).abs() / 2.0),
                _ => {
                    warn.push("box needs min+max or at+size".into());
                    return;
                }
            };
            push(out, warn, Kind::Box { lo, hi }, mat, finite(&lo.to_array()) && finite(&hi.to_array()));
        }
        Op::Sphere { c, r, mat } => {
            let r = r.v().abs();
            push(out, warn, Kind::Ellipsoid { c: v3(*c), r }, mat, finite(c) && finite(&r.to_array()));
        }
        Op::Cylinder { base, r, r2, h, axis, mat } => {
            let r1 = r2.unwrap_or(*r);
            push(
                out,
                warn,
                Kind::Cone { base: v3(*base), axis: axis.idx(), r0: r.abs(), r1: r1.abs(), h: *h },
                mat,
                finite(base) && finite(&[*r, r1, *h]),
            );
        }
        Op::Line { a, b, r, mat } => push(
            out,
            warn,
            Kind::Capsule { a: v3(*a), b: v3(*b), r: r.abs() },
            mat,
            finite(a) && finite(b) && finite(&[*r]),
        ),
        Op::Torus { c, big, small, axis, mat } => push(
            out,
            warn,
            Kind::Torus { c: v3(*c), axis: axis.idx(), big: big.abs(), small: small.abs() },
            mat,
            finite(c) && finite(&[*big, *small]),
        ),
        Op::Roof { min, max, ridge, thickness, mat, ends, ends_inset } => {
            let (lo, hi) = (v3(*min).min(v3(*max)), v3(*min).max(v3(*max)));
            let r = if *ridge == Axis::Z { 2 } else { 0 };
            let ok = finite(min) && finite(max) && finite(&[*thickness]);
            if let Some(end) = ends
                && ok
            {
                // Solid gable triangles just inside each end, under the slopes.
                let inset = ends_inset.unwrap_or(0.3).clamp(0.0, (hi[r] - lo[r]) / 2.0);
                for side in [0, 1] {
                    let (mut a, mut b) = (lo, hi);
                    if side == 0 {
                        a[r] = lo[r] + inset;
                        b[r] = a[r] + 0.25;
                    } else {
                        b[r] = hi[r] - inset;
                        a[r] = b[r] - 0.25;
                    }
                    out.push((Kind::Roof { lo: a, hi: b, ridge: r, thick: 0.0 }, xf, mat_of(pal, end, warn)));
                }
            }
            push(out, warn, Kind::Roof { lo, hi, ridge: r, thick: thickness.max(0.0) }, mat, ok)
        }
        Op::Wedge { min, max, rise, flip, mat } => push(
            out,
            warn,
            Kind::Wedge {
                lo: v3(*min).min(v3(*max)),
                hi: v3(*min).max(v3(*max)),
                rise: if *rise == Axis::Z { 2 } else { 0 },
                flip: *flip,
            },
            mat,
            finite(min) && finite(max),
        ),
        Op::Voxels { at, cell, layers, key } => {
            let cell = cell.unwrap_or(1.0 / VPM).clamp(1.0 / VPM, 4.0);
            let dy = layers.len();
            let dz = layers.iter().map(|l| l.len()).max().unwrap_or(0);
            let dx = layers.iter().flatten().map(|r| r.chars().count()).max().unwrap_or(0);
            if dx * dy * dz == 0 || dx * dy * dz > MAX_GRID * 16 || !finite(at) {
                warn.push("voxels grid empty or too big".into());
                return;
            }
            let keys: BTreeMap<char, Mat> =
                key.iter().filter_map(|(k, v)| Some((k.chars().next()?, mat_of(pal, v, warn)))).collect();
            let mut cells = vec![AIR; dx * dy * dz];
            for (y, layer) in layers.iter().enumerate() {
                for (z, row) in layer.iter().enumerate() {
                    for (x, ch) in row.chars().enumerate() {
                        if let Some(&m) = keys.get(&ch) {
                            cells[x + z * dx + y * dx * dz] = m;
                        }
                    }
                }
            }
            out.push((Kind::Grid { at: v3(*at), cell, dims: [dx, dy, dz], cells }, xf, 1));
        }
        Op::Group { ops, at, rot, mirror, count, step } => {
            let local = Xf { m: rotation_y(rot.unwrap_or(0.0)), t: at.map(v3).unwrap_or(Vec3::ZERO) };
            let n = count.unwrap_or(1).clamp(1, 256);
            let step = step.map(v3).unwrap_or(Vec3::ZERO);
            // Mirror copies: every combination of the listed axes.
            let mut flips = vec![Vec3::ONE];
            for ax in mirror {
                let i = ax.idx();
                let more: Vec<Vec3> = flips
                    .iter()
                    .map(|f| {
                        let mut g = *f;
                        g[i] = -g[i];
                        g
                    })
                    .collect();
                flips.extend(more);
            }
            for k in 0..n {
                for f in &flips {
                    let rep = Xf { m: Mat3::IDENTITY, t: step * k as f32 };
                    let flip = Xf { m: Mat3::from_diagonal(*f), t: Vec3::ZERO };
                    let x = xf.then(rep).then(flip).then(local);
                    for o in ops {
                        flatten(o, x, pal, out, warn, depth + 1);
                    }
                }
            }
        }
    }
}

/// Canonical id of a spec: FNV-1a of its JSON.
pub fn hash(spec: &ModelSpec) -> u64 {
    let s = serde_json::to_string(spec).unwrap_or_default();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Build `spec` against the builder palette.
pub fn build(spec: &ModelSpec) -> Result<Model, String> {
    let pal = blocks::palette();
    let mut warn = Vec::new();
    let mut flat = Vec::new();
    for op in &spec.ops {
        flatten(op, Xf::ID, &pal, &mut flat, &mut warn, 0);
    }
    if flat.is_empty() {
        return Err("the model has no operations".into());
    }
    if flat.len() >= MAX_OPS {
        warn.push(format!("stopped at {MAX_OPS} operations"));
    }
    // Pick the smallest cube that holds every painted primitive (origin at floor centre).
    let mut ext = 0.0f32;
    let mut top = 0.0f32;
    for (k, xf, _) in &flat {
        let (lo, hi) = k.bounds();
        for i in 0..8 {
            let c = Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            );
            let w = xf.m * c + xf.t;
            ext = ext.max(w.x.abs()).max(w.z.abs());
            top = top.max(w.y);
            if w.y < -0.1 && warn.len() < 20 && !warn.iter().any(|s| s.contains("below")) {
                warn.push("parts below y = 0 are cut off (the floor is y = 0)".into());
            }
        }
    }
    let need = (ext * 2.0).max(top) * VPM;
    let level = (3..=MAX_LEVEL).find(|l| (vx_core::side(*l) as f32) >= need).unwrap_or(MAX_LEVEL);
    if (vx_core::side(level) as f32) < need {
        warn.push(format!("model is bigger than {} m; clipped", vx_core::side(MAX_LEVEL) / 32));
    }
    let half = (vx_core::side(level) / 2) as f32;
    let prims: Vec<Prim> = flat.into_iter().filter_map(|(k, xf, m)| Prim::new(k, xf, m, half)).collect();
    let placed: Vec<Placed> = prims.iter().map(|p| Placed { p, half }).collect();
    let refs: Vec<&dyn Shape> = placed.iter().map(|p| p as &dyn Shape).collect();
    let dag = prefab(&pal, level, &refs);
    let dag = std::sync::Arc::try_unwrap(dag).unwrap_or_else(|a| (*a).clone());
    let (lo, hi) = dag.bounds_of(dag.root, level).ok_or("the model is empty (everything carved or off-grid)")?;
    let voxels = dag.voxel_count();
    Ok(Model { dag, min: lo.to_array(), max: hi.to_array(), voxels, warnings: warn })
}

/// Copy a model built against the builder palette into `world`'s palette ids.
pub fn remap(model: &Dag, world: &Palette) -> Dag {
    let src = blocks::palette();
    let map: Vec<Mat> =
        src.materials.iter().map(|m| if m.name == "air" { AIR } else { world.id(&m.name).unwrap_or(1) }).collect();
    if map.iter().enumerate().all(|(i, &m)| i as Mat == m || i == 0) {
        return model.clone();
    }
    let mut b = vx_core::DagBuilder::new(world);
    let mut memo = Default::default();
    let root = remap_rec(model, &mut b, model.root, model.levels, &map, &mut memo);
    b.finish(root, model.levels)
}

fn remap_rec(
    d: &Dag,
    b: &mut vx_core::DagBuilder,
    n: vx_core::NodeRef,
    level: u32,
    map: &[Mat],
    memo: &mut BTreeMap<vx_core::NodeRef, vx_core::NodeRef>,
) -> vx_core::NodeRef {
    use vx_core::Pool;
    if n == vx_core::EMPTY {
        return n;
    }
    if let Some(&r) = memo.get(&n) {
        return r;
    }
    let out = if level == 1 {
        let mut mats = [AIR; 64];
        for (i, m) in mats.iter_mut().enumerate() {
            let v = d.leaf_mat(n, i);
            *m = if v == AIR { AIR } else { map.get(v as usize).copied().unwrap_or(1) };
        }
        b.leaf(&mats)
    } else {
        let mut ch = [vx_core::EMPTY; 64];
        let mut bits = d.mask(n);
        while bits != 0 {
            let i = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            ch[i] = remap_rec(d, b, d.child(n, i), level - 1, map, memo);
        }
        b.interior(&ch)
    };
    memo.insert(n, out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(json: &str) -> ModelSpec {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_stool_builds_with_mirrored_legs() {
        let s = spec(
            r#"{ "name": "stool", "ops": [
              { "op": "cylinder", "base": [0, 0.45, 0], "r": 0.2, "h": 0.05, "mat": "oak" },
              { "op": "group", "mirror": ["x", "z"], "ops": [
                { "op": "line", "a": [0.14, 0, 0.14], "b": [0.12, 0.45, 0.12], "r": 0.025, "mat": "dark_oak" } ] } ] }"#,
        );
        let m = build(&s).unwrap();
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
        assert_eq!(m.dag.levels, 3, "fits in 2 m");
        let pal = blocks::palette();
        let oak = blocks::resolve(&pal, "oak").unwrap();
        let leg = blocks::resolve(&pal, "dark_oak").unwrap();
        // Seat top centre, and all four legs near the floor.
        assert_eq!(m.dag.get(IVec3::new(32, 15, 32)), oak);
        for (x, z) in [(1, 1), (-1, 1), (1, -1), (-1, -1)] {
            let at = |s: i32| (32.0 + s as f32 * 0.14 * VPM).floor() as i32;
            let p = IVec3::new(at(x), 1, at(z));
            assert_eq!(m.dag.get(p), leg, "leg {x} {z}");
        }
        assert_eq!(m.min[1], 0);
        assert_eq!(hash(&s), hash(&s.clone()));
    }

    #[test]
    fn a_house_with_a_carved_door_and_a_roof() {
        let s = spec(
            r#"{ "name": "cottage", "category": "building", "ops": [
              { "op": "box", "min": [-4, 0, -3], "max": [4, 3, 3], "mat": "plaster" },
              { "op": "box", "min": [-3.8, 0.1, -2.8], "max": [3.8, 2.9, 2.8], "mat": "air" },
              { "op": "box", "at": [0, 1.05, -3], "size": [1.0, 2.1, 0.5], "mat": "air" },
              { "op": "roof", "min": [-4.4, 3, -3.4], "max": [4.4, 5.5, 3.4], "ridge": "x", "thickness": 0.3, "mat": "roof_tile" },
              { "op": "group", "count": 3, "step": [2.5, 0, 0], "at": [-2.5, 0, 0], "ops": [
                { "op": "box", "at": [0, 1.6, 3], "size": [0.9, 0.9, 0.3], "mat": "glass" } ] } ] }"#,
        );
        let m = build(&s).unwrap();
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
        assert_eq!(m.dag.levels, 5, "8 m wide house → 32 m cube");
        let pal = blocks::palette();
        let c = 512;
        let at = |x: f32, y: f32, z: f32| {
            m.dag.get(IVec3::new(c + (x * VPM) as i32, (y * VPM) as i32, c + (z * VPM) as i32))
        };
        assert_eq!(at(0.0, 1.0, -2.95), AIR, "door is open");
        assert_eq!(at(2.0, 1.0, -2.95), blocks::resolve(&pal, "plaster").unwrap(), "wall");
        assert_eq!(at(0.0, 1.5, 0.0), AIR, "hollow inside");
        assert_eq!(at(0.0, 5.3, 0.0), blocks::resolve(&pal, "roof_tile").unwrap(), "ridge");
        assert_eq!(at(0.0, 4.0, 0.0), AIR, "roof is a shell");
        assert_eq!(at(2.5, 1.6, 2.95), blocks::resolve(&pal, "glass").unwrap(), "window");
        assert!(m.dag.bytes() < 4 << 20);
    }

    #[test]
    fn pixel_layers_and_remap() {
        let s = spec(
            r##"{ "name": "sign", "ops": [
              { "op": "voxels", "at": [-0.5, 0, 0], "cell": 0.25, "key": { "X": "brick", "o": "#ffcc00" },
                "layers": [ ["XXXX"], ["XooX"], ["XXXX"] ] } ] }"##,
        );
        let m = build(&s).unwrap();
        let pal = blocks::palette();
        assert_eq!(m.dag.get(IVec3::new(32 - 16 + 2, 1, 34)), blocks::resolve(&pal, "brick").unwrap());
        let mut world = Palette::default_city();
        crate::blocks::add_to(&mut world);
        let r = remap(&m.dag, &world);
        assert_eq!(world.get(r.get(IVec3::new(32 - 16 + 2, 1, 34))).name, "brick");
        assert_eq!(r.voxel_count(), m.dag.voxel_count());
    }

    #[test]
    fn hostile_specs_are_rejected_or_clamped() {
        assert!(build(&spec(r#"{ "name": "x", "ops": [] }"#)).is_err());
        let s = spec(r#"{ "name": "x", "ops": [ { "op": "sphere", "c": [0, 1, 0], "r": 1e30, "mat": "stone" } ] }"#);
        assert!(build(&s).is_err() || !build(&s).unwrap().warnings.is_empty());
        let deep = r#"{ "op": "group", "count": 256, "step": [0.01, 0, 0], "ops": [ { "op": "box", "at": [0, 0.5, 0], "size": [0.1, 0.1, 0.1], "mat": "stone" } ] }"#;
        let s = spec(&format!(r#"{{ "name": "x", "ops": [ {{ "op": "group", "count": 256, "ops": [ {deep} ] }} ] }}"#));
        let m = build(&s).unwrap();
        assert!(m.warnings.iter().any(|w| w.contains("stopped")));
    }
}

/// What travels with a built model.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelMeta {
    /// Hex [`hash`] of the spec.
    pub hash: String,
    pub name: String,
    pub category: String,
    pub levels: u32,
    pub min: [i32; 3],
    pub max: [i32; 3],
    pub voxels: u64,
    pub spec: ModelSpec,
}

/// `"VXM1" | u32 LE meta length | meta JSON | DAG bytes` (builder palette ids).
pub fn encode(spec: &ModelSpec, m: &Model) -> (Vec<u8>, ModelMeta) {
    let meta = ModelMeta {
        hash: format!("{:016x}", hash(spec)),
        name: spec.name.chars().take(48).collect(),
        category: spec.category.chars().take(24).collect(),
        levels: m.dag.levels,
        min: m.min,
        max: m.max,
        voxels: m.voxels,
        spec: spec.clone(),
    };
    let json = serde_json::to_vec(&meta).unwrap_or_default();
    let mut out = Vec::with_capacity(8 + json.len() + m.dag.bytes());
    out.extend_from_slice(b"VXM1");
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(&m.dag.to_bytes());
    (out, meta)
}

pub fn decode(bytes: &[u8]) -> Result<(ModelMeta, Dag), String> {
    if bytes.len() < 8 || &bytes[..4] != b"VXM1" {
        return Err("not a voxelscape model".into());
    }
    let n = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let json = bytes.get(8..8 + n).ok_or("truncated model")?;
    let meta: ModelMeta = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    let dag = Dag::from_bytes(&bytes[8 + n..])?;
    if dag.levels > MAX_LEVEL || dag.levels < 1 {
        return Err("model size out of range".into());
    }
    Ok((meta, dag))
}

/// The modelling guide for agents (and people): units, every op, scale references, craft.
pub fn guide() -> String {
    let mats = blocks::names().join(", ");
    format!(
        r####"# Sculpting voxel models

A model is JSON: {{ "name": "...", "category": "building|furniture|vehicle|nature|prop|...", "ops": [ ... ] }}.
Units are METRES. One voxel is 1/32 m (3.1 cm), so a 1 m wall is 32 voxels: details down to
~5 cm read up close. Y is up; the origin is the centre of the model's floor (x = z = 0, y = 0);
the model's FRONT faces −z (doors, faces, headlights go on the −z side). Nothing below y = 0.
Ops paint in order (later covers earlier); "mat": "air" carves. Max 128 m, 4000 ops.

## Ops
- box: {{"op":"box","min":[x,y,z],"max":[x,y,z],"mat":"brick"}} or {{"op":"box","at":[cx,cy,cz],"size":[w,h,d],"mat":..}}
- sphere: {{"op":"sphere","c":[x,y,z],"r":0.5}} or "r":[rx,ry,rz] (ellipsoid)
- cylinder: {{"op":"cylinder","base":[x,y,z],"r":0.3,"h":2,"axis":"y"}}; "r2" = top radius (cones, lampshades, tree trunks); axis x|y|z
- line: {{"op":"line","a":[x,y,z],"b":[x,y,z],"r":0.03}} rounded rod: legs, pipes, railings, branches, cables
- torus: {{"op":"torus","c":[x,y,z],"big":0.35,"small":0.1,"axis":"x"}} wheels, tyres, rings
- roof: {{"op":"roof","min":[..],"max":[..],"ridge":"x","thickness":0.25,"ends":"plaster","ends_inset":0.4}} gable roof filling the box, ridge along x or z;
  thickness makes it a shell (hollow attic); "ends" closes both gable triangles (inset by the overhang so they meet the walls)
- wedge: {{"op":"wedge","min":[..],"max":[..],"rise":"z","flip":false}} ramp / shed roof: full height at the low end of the axis
- voxels: {{"op":"voxels","at":[minx,miny,minz],"cell":0.0625,"key":{{"#":"brick","o":"#ffcc00"}},"layers":[["#o#","###"]]}}
  pixel art: layers bottom-up, rows −z→+z, characters −x→+x, " " or "." empty. Signs, faces, emblems, patterns.
- group: {{"op":"group","ops":[..],"at":[x,y,z],"rot":90,"mirror":["x"],"count":4,"step":[2,0,0]}}
  moves/turns (degrees about +y) its ops, mirror copies across x=0 and/or z=0, repeats count times.
  Use mirror for symmetry (chair legs, car wheels, facade halves) and count for windows, columns, fence posts, stairs.

## Materials
{mats}
or any colour as "#rrggbb" (matched to a plain paint). Lights: lamp, lamp_warm, candle, neon_*, screen, fire, lava glow at night.
Glass and water are see-through. "team" takes the colour of whoever owns it.

## Scale (match these or it looks wrong next to players)
player 1.8 m tall, 0.6 m wide, eyes 1.6 m · door 1.0 × 2.1 m (≥ 0.8 m wide to walk through) · storey 3 m
(floor-to-floor), ceiling ≥ 2.4 m · stairs rise 0.18 m, tread 0.28 m (players step up ≤ 0.5 m) · railing 1.0 m ·
table 0.75 m · chair seat 0.45 m · counter 0.9 m · bed 2.0 × 1.4 × 0.5 m · sofa 2.0 × 0.9 × 0.85 m ·
car 4.5 × 1.8 × 1.5 m · street lamp 5 m · tree 5–15 m · window sill 0.9 m, window 1.0–1.4 m tall.

## Craft (what makes voxel art beautiful, Minecraft-builder style)
- Never leave a big flat wall: frame it. Plinth/foundation at the base, a trim band at each floor, a cornice
  under the roof; corner pillars or quoins in a second material; windows RECESSED 0.1–0.15 m (carve, then a glass
  pane) with frames, sills and lintels; shutters, awnings, flower boxes.
- Depth beats colour: offset things by 1–4 voxels (3–12 cm). Beams that stick out, overhanging roofs (0.3–0.6 m),
  stepped edges, layered roofs, chimneys, dormers.
- 2–3 related materials plus one accent (e.g. plaster + dark_oak beams + roof_tile, accent paint_green door).
  Darker at the base, lighter up top. Use colour variety across copies (several houses, not clones).
- Silhouette first (the big shapes read from far), then medium detail (windows, trims), then small detail
  (handles, hinges, lamps, signs, plants) for up close.
- Hollow buildings so players can walk in: carve the inside, carve the door, add floors, stairs, furniture.
  Walls 0.2–0.3 m thick. Interiors make worlds feel real.
- Organic things (trees, rocks, food): overlapping spheres/ellipsoids of slightly different colours; trunks as
  cones (r2 < r); branches as lines.
- Vehicles and furniture: build one side and mirror it.

## Workflow
1. sculpt_model with your spec → LOOK at the four renders (front-left, back-right, side, top) with the 1 m grid
   and the 1.8 m player for scale. Check proportions, that doors/windows read, nothing floats or is missing.
2. Fix and sculpt again (get_model returns a spec to edit). Iterate until it is genuinely good.
3. place_model: "build" stamps it into the world as solid voxels (walkable buildings, terrain features);
   "prop" spawns a physics prop players can grab. Then preview the spot in the world.
4. Big scenes: sculpt a few variants (sizes, colours, roofs) and place many with different turns.
"####
    )
}

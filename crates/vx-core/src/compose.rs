//! Composition: turn a description of a world (shapes, prefab instances) into a DAG fast.
//!
//! The trick that makes 10¹¹ voxels cheap is to never visit most of them:
//! - each item can *classify* a cube as Empty / Solid(mat) / Mixed. A cube that the
//!   topmost item fills solid becomes one cached node; a cube no item touches is EMPTY;
//! - a prefab [`Instance`] that is the only item in an aligned cube is *linked*, not
//!   rasterised: its subtree is imported once (memoised) and shared everywhere;
//! - only cubes that stay Mixed down to level 1 are sampled voxel by voxel;
//! - top-level subtrees build in parallel (rayon), then merge (hash-consing dedups across
//!   threads).
//!
//! Items paint in order: a later item's non-air voxels cover earlier ones.

use std::sync::Arc;

use glam::IVec3;
use rustc_hash::FxHashMap;

use crate::dag::{Dag, DagBuilder, EMPTY, NodeRef, Pool, side};
use crate::mat::{AIR, Mat, Palette};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Empty,
    Solid(Mat),
    Mixed,
}

/// Axis-aligned voxel box `[min, max)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Aabb {
    pub min: IVec3,
    pub max: IVec3,
}

impl Aabb {
    pub fn new(min: IVec3, max: IVec3) -> Self {
        Aabb { min, max }
    }
    pub fn cube(min: IVec3, size: i32) -> Self {
        Aabb { min, max: min + IVec3::splat(size) }
    }
    pub fn overlaps(&self, o: &Aabb) -> bool {
        self.min.cmplt(o.max).all() && o.min.cmplt(self.max).all()
    }
    pub fn contains(&self, o: &Aabb) -> bool {
        self.min.cmple(o.min).all() && o.max.cmple(self.max).all()
    }
    pub fn is_empty(&self) -> bool {
        self.min.cmpge(self.max).any()
    }
}

/// Something that paints voxels.
pub trait Shape: Send + Sync {
    fn bounds(&self) -> Aabb;
    /// Classify the cube `[min, min+size)`. Conservative: `Mixed` is always allowed.
    fn classify(&self, cube: Aabb) -> Class {
        let b = self.bounds();
        if !b.overlaps(&cube) { Class::Empty } else { Class::Mixed }
    }
    /// Material at voxel `p` (AIR = no paint).
    fn sample(&self, p: IVec3) -> Mat;
    /// If this shape is a prefab whose node covers `cube` exactly, its (pool, node).
    fn as_instance(&self, _cube: Aabb, _level: u32) -> Option<(&Arc<Dag>, NodeRef)> {
        None
    }
}

/// A solid box.
pub struct BoxShape {
    pub b: Aabb,
    pub mat: Mat,
}

impl Shape for BoxShape {
    fn bounds(&self) -> Aabb {
        self.b
    }
    fn classify(&self, cube: Aabb) -> Class {
        if !self.b.overlaps(&cube) {
            Class::Empty
        } else if self.b.contains(&cube) {
            if self.mat == AIR { Class::Empty } else { Class::Solid(self.mat) }
        } else {
            Class::Mixed
        }
    }
    fn sample(&self, p: IVec3) -> Mat {
        if p.cmpge(self.b.min).all() && p.cmplt(self.b.max).all() { self.mat } else { AIR }
    }
}

/// A solid ball of voxels whose centres are within `r` of `c` (integer maths: the same on
/// every machine, so edits made of balls replay bit-exactly).
pub struct Ball {
    pub c: IVec3,
    pub r: i32,
    pub mat: Mat,
}

impl Ball {
    fn inside(&self, p: IVec3) -> bool {
        // Doubled coordinates keep the voxel centre (p + ½) integral.
        let d = (p * 2 + IVec3::ONE) - self.c * 2;
        (d.x as i64).pow(2) + (d.y as i64).pow(2) + (d.z as i64).pow(2) <= 4 * (self.r as i64).pow(2)
    }
}

impl Shape for Ball {
    fn bounds(&self) -> Aabb {
        Aabb::new(self.c - IVec3::splat(self.r), self.c + IVec3::splat(self.r + 1))
    }
    fn classify(&self, cube: Aabb) -> Class {
        if !self.bounds().overlaps(&cube) {
            return Class::Empty;
        }
        // Nearest voxel centre of the cube to the centre: none inside → the cube is empty.
        // (Without this every cube in the bounding box would be sampled voxel by voxel.)
        let near = self.c.clamp(cube.min, cube.max - IVec3::ONE);
        if !self.inside(near) {
            return Class::Empty;
        }
        let (a, b) = (cube.min, cube.max - IVec3::ONE);
        let all = (0..8).all(|i| {
            self.inside(IVec3::new(
                if i & 1 == 0 { a.x } else { b.x },
                if i & 2 == 0 { a.y } else { b.y },
                if i & 4 == 0 { a.z } else { b.z },
            ))
        });
        if all { Class::Solid(if self.mat == AIR { CARVE } else { self.mat }) } else { Class::Mixed }
    }
    fn sample(&self, p: IVec3) -> Mat {
        if !self.inside(p) {
            AIR
        } else if self.mat == AIR {
            CARVE
        } else {
            self.mat
        }
    }
}

/// A box that carves (paints air) — boxes are painted in order, so a later `Carve`
/// removes what earlier items placed. Represented with a sentinel material.
pub const CARVE: Mat = u16::MAX;

/// Any closure (with optional classifier) as a shape.
pub struct FnShape<S: Fn(IVec3) -> Mat + Send + Sync, C: Fn(Aabb) -> Class + Send + Sync> {
    pub b: Aabb,
    pub sample: S,
    pub classify: C,
}

impl<S: Fn(IVec3) -> Mat + Send + Sync, C: Fn(Aabb) -> Class + Send + Sync> Shape for FnShape<S, C> {
    fn bounds(&self) -> Aabb {
        self.b
    }
    fn classify(&self, cube: Aabb) -> Class {
        if !self.b.overlaps(&cube) { Class::Empty } else { (self.classify)(cube) }
    }
    fn sample(&self, p: IVec3) -> Mat {
        if p.cmpge(self.b.min).all() && p.cmplt(self.b.max).all() { (self.sample)(p) } else { AIR }
    }
}

/// A small dense voxel grid (procedural props, imported models) placed anywhere.
pub struct Dense {
    pub origin: IVec3,
    pub size: IVec3,
    /// x-fastest, then z, then y: `x + z*sx + y*sx*sz`.
    pub data: Arc<[Mat]>,
}

impl Shape for Dense {
    fn bounds(&self) -> Aabb {
        Aabb::new(self.origin, self.origin + self.size)
    }
    fn sample(&self, p: IVec3) -> Mat {
        let l = p - self.origin;
        if l.cmplt(IVec3::ZERO).any() || l.cmpge(self.size).any() {
            return AIR;
        }
        self.data[(l.x + l.z * self.size.x + l.y * self.size.x * self.size.z) as usize]
    }
}

/// A prefab placed at a position. Placed at a multiple of its node size it is *linked*
/// (free); otherwise it is sampled (still correct, just slower).
pub struct Instance {
    pub dag: Arc<Dag>,
    pub origin: IVec3,
}

impl Shape for Instance {
    fn bounds(&self) -> Aabb {
        Aabb::cube(self.origin, self.dag.side())
    }
    fn classify(&self, cube: Aabb) -> Class {
        let b = self.bounds();
        if !b.overlaps(&cube) {
            return Class::Empty;
        }
        // A sub-cube the prefab leaves empty (aligned or not): skip it without sampling.
        let rel = cube.min - self.origin;
        if !self.dag.any_in(self.dag.root, self.dag.levels, rel, cube.max - self.origin) {
            return Class::Empty;
        }
        Class::Mixed
    }
    fn sample(&self, p: IVec3) -> Mat {
        let l = p - self.origin;
        let s = self.dag.side();
        if l.cmplt(IVec3::ZERO).any() || l.cmpge(IVec3::splat(s)).any() {
            return AIR;
        }
        self.dag.get_in(self.dag.root, self.dag.levels, l)
    }
    fn as_instance(&self, cube: Aabb, level: u32) -> Option<(&Arc<Dag>, NodeRef)> {
        if level > self.dag.levels || !self.bounds().contains(&cube) {
            return None;
        }
        let rel = cube.min - self.origin;
        let size = side(level);
        if rel % size != IVec3::ZERO {
            return None;
        }
        Some((&self.dag, self.dag.descend(self.dag.root, self.dag.levels, rel, level)))
    }
}

/// Build statistics.
#[derive(Clone, Debug, Default)]
pub struct BuildStats {
    pub sampled_leaves: u64,
    pub solid_shortcuts: u64,
    pub linked_instances: u64,
    pub millis: u128,
}

struct Ctx<'a> {
    b: DagBuilder,
    imports: FxHashMap<(usize, NodeRef), NodeRef>,
    stats: BuildStats,
    _p: std::marker::PhantomData<&'a ()>,
}

/// Build the node for cube `(origin, level)` from `items` painted over `base` (an existing node
/// of the same pool covering that cube, or EMPTY). Untouched cubes keep their base node, so an
/// edit only adds the paths it changes.
fn build_node(cx: &mut Ctx, level: u32, origin: IVec3, items: &[&dyn Shape], base: NodeRef) -> NodeRef {
    let size = side(level);
    let cube = Aabb::cube(origin, size);
    // Walk from the top item down: drop items that are empty here; stop at a solid one.
    let mut live: Vec<&dyn Shape> = Vec::with_capacity(items.len());
    let mut floor: Option<Mat> = None;
    for it in items.iter().rev() {
        match it.classify(cube) {
            Class::Empty => {}
            Class::Solid(m) => {
                floor = Some(m);
                break;
            }
            Class::Mixed => live.push(*it),
        }
    }
    live.reverse();
    if live.is_empty() {
        return match floor {
            None => base,
            Some(CARVE) => EMPTY,
            Some(m) => {
                cx.stats.solid_shortcuts += 1;
                cx.b.solid(m, level)
            }
        };
    }
    // Link a prefab that is alone in an aligned cube.
    if floor.is_none()
        && base == EMPTY
        && live.len() == 1
        && let Some((dag, n)) = live[0].as_instance(cube, level)
    {
        cx.stats.linked_instances += 1;
        let key = (Arc::as_ptr(dag) as usize, n);
        if let Some(&r) = cx.imports.get(&key) {
            return r;
        }
        // Memo per prefab DAG: a node's import depends only on (dag, node).
        let mut memo = FxHashMap::default();
        let r = cx.b.import(dag.as_ref(), n, level, &mut memo);
        for (src, dst) in memo {
            cx.imports.insert((key.0, src), dst);
        }
        return r;
    }
    if level == 1 {
        cx.stats.sampled_leaves += 1;
        let mut mats = [AIR; 64];
        for (i, m) in mats.iter_mut().enumerate() {
            let p = origin + crate::dag::child_xyz(i);
            let mut v = match floor {
                Some(f) => f,
                None if base != EMPTY => cx.b.leaf_mat(base, i),
                None => AIR,
            };
            for it in live.iter().rev() {
                let s = it.sample(p);
                if s != AIR {
                    v = s;
                    break;
                }
            }
            *m = if v == CARVE { AIR } else { v };
        }
        return cx.b.leaf(&mats);
    }
    // Bucket items into the children they overlap.
    let cs = side(level - 1);
    let mut buckets: Vec<Vec<&dyn Shape>> = vec![Vec::new(); 64];
    let mut floor_shape: Option<BoxShape> = None;
    if let Some(m) = floor {
        floor_shape = Some(BoxShape { b: cube, mat: m });
    }
    let floor_ref: Option<&dyn Shape> = floor_shape.as_ref().map(|s| s as &dyn Shape);
    if let Some(f) = floor_ref {
        for bk in buckets.iter_mut() {
            bk.push(f);
        }
    }
    for it in &live {
        let b = it.bounds();
        let lo = ((b.min - origin).max(IVec3::ZERO)) / cs;
        let hi = ((b.max - origin - IVec3::ONE).min(IVec3::splat(size - 1))) / cs;
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    buckets[crate::dag::child_index(x, y, z)].push(*it);
                }
            }
        }
    }
    let mut ch = [EMPTY; 64];
    for (i, bk) in buckets.iter().enumerate() {
        // Below a solid floor the base is covered, so it only matters without one.
        let below = if base != EMPTY && floor.is_none() { cx.b.child(base, i) } else { EMPTY };
        if bk.is_empty() {
            ch[i] = below;
            continue;
        }
        let o = origin + crate::dag::child_xyz(i) * cs;
        ch[i] = build_node(cx, level - 1, o, bk, below);
    }
    cx.b.interior(&ch)
}

/// Build a world of `4^levels` voxels per axis from `items` (painted in order).
/// With the `parallel` feature, the root's 64 children (or 4096 grandchildren for deep
/// worlds) are built on all cores and merged.
pub fn build(palette: &Palette, levels: u32, items: &[&dyn Shape]) -> (Dag, BuildStats) {
    let t0 = std::time::Instant::now();
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        if levels >= 4 {
            // Split two levels down: 4096 tasks keeps every core busy on uneven cities.
            let split = levels - 2;
            let cs = side(split);
            let world = Aabb::cube(IVec3::ZERO, side(levels));
            let tasks: Vec<IVec3> = (0..4096)
                .map(|i| {
                    let a = crate::dag::child_xyz(i & 63);
                    let b = crate::dag::child_xyz(i >> 6);
                    (b * 4 + a) * cs
                })
                .collect();
            let parts: Vec<(IVec3, Option<(DagBuilder, NodeRef, BuildStats)>)> = tasks
                .par_iter()
                .map(|&o| {
                    let cube = Aabb::cube(o, cs);
                    let mine: Vec<&dyn Shape> =
                        items.iter().copied().filter(|it| it.bounds().overlaps(&cube)).collect();
                    if mine.is_empty() || !world.contains(&cube) {
                        return (o, None);
                    }
                    let mut cx = Ctx {
                        b: DagBuilder::new(palette),
                        imports: FxHashMap::default(),
                        stats: BuildStats::default(),
                        _p: Default::default(),
                    };
                    let r = build_node(&mut cx, split, o, &mine, EMPTY);
                    (o, if r == EMPTY { None } else { Some((cx.b, r, cx.stats)) })
                })
                .collect();
            // Merge into one pool (dedup across threads), then assemble the top two levels.
            let mut main = DagBuilder::new(palette);
            let mut stats = BuildStats::default();
            let mut at: FxHashMap<IVec3, NodeRef> = FxHashMap::default();
            for (o, part) in parts {
                if let Some((b, r, s)) = part {
                    let mut memo = FxHashMap::default();
                    let n = main.import(&b, r, split, &mut memo);
                    at.insert(o, n);
                    stats.sampled_leaves += s.sampled_leaves;
                    stats.solid_shortcuts += s.solid_shortcuts;
                    stats.linked_instances += s.linked_instances;
                }
            }
            let mut mid = [EMPTY; 64];
            for (bi, slot) in mid.iter_mut().enumerate() {
                let mut ch = [EMPTY; 64];
                for (ai, c) in ch.iter_mut().enumerate() {
                    let o = (crate::dag::child_xyz(bi) * 4 + crate::dag::child_xyz(ai)) * cs;
                    *c = at.get(&o).copied().unwrap_or(EMPTY);
                }
                *slot = main.interior(&ch);
            }
            let root = main.interior(&mid);
            stats.millis = t0.elapsed().as_millis();
            return (main.finish(root, levels), stats);
        }
    }
    let mut cx = Ctx {
        b: DagBuilder::new(palette),
        imports: FxHashMap::default(),
        stats: BuildStats::default(),
        _p: Default::default(),
    };
    let root = build_node(&mut cx, levels, IVec3::ZERO, items, EMPTY);
    let mut stats = cx.stats;
    stats.millis = t0.elapsed().as_millis();
    (cx.b.finish(root, levels), stats)
}

/// Build a small prefab (a module / prop) as its own DAG.
pub fn prefab(palette: &Palette, levels: u32, items: &[&dyn Shape]) -> Arc<Dag> {
    let mut cx = Ctx {
        b: DagBuilder::new(palette),
        imports: FxHashMap::default(),
        stats: BuildStats::default(),
        _p: Default::default(),
    };
    let root = build_node(&mut cx, levels, IVec3::ZERO, items, EMPTY);
    Arc::new(cx.b.finish(root, levels))
}

/// Paint `items` over `dag` in place and return the new root. *Append-only*: existing words
/// are never changed, only the changed paths are added after them, so every earlier root
/// stays a valid world (undo, rollback, and the GPU only has to upload the new tail).
pub fn edit(dag: &mut Dag, palette: &Palette, items: &[&dyn Shape]) -> NodeRef {
    let words = std::mem::take(&mut dag.words);
    let mut cx = Ctx {
        b: DagBuilder::over(words, palette),
        imports: FxHashMap::default(),
        stats: BuildStats::default(),
        _p: Default::default(),
    };
    let world = Aabb::cube(IVec3::ZERO, dag.side());
    let mine: Vec<&dyn Shape> = items.iter().copied().filter(|it| it.bounds().overlaps(&world)).collect();
    let root = if mine.is_empty() { dag.root } else { build_node(&mut cx, dag.levels, IVec3::ZERO, &mine, dag.root) };
    dag.words = cx.b.words;
    dag.root = root;
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pal() -> Palette {
        Palette::default_city()
    }

    #[test]
    fn boxes_paint_in_order_and_carve() {
        let p = pal();
        let ground = BoxShape { b: Aabb::new(IVec3::ZERO, IVec3::new(256, 10, 256)), mat: 1 };
        let block = BoxShape { b: Aabb::new(IVec3::new(5, 5, 5), IVec3::new(20, 30, 21)), mat: 3 };
        let hole = BoxShape { b: Aabb::new(IVec3::new(8, 0, 8), IVec3::new(10, 40, 10)), mat: CARVE };
        let (d, s) = build(&p, 4, &[&ground, &block, &hole]);
        assert_eq!(d.get(IVec3::new(100, 3, 100)), 1);
        assert_eq!(d.get(IVec3::new(6, 6, 6)), 3, "block over ground");
        assert_eq!(d.get(IVec3::new(9, 2, 9)), AIR, "carved");
        assert_eq!(d.get(IVec3::new(6, 25, 6)), 3);
        assert_eq!(d.get(IVec3::new(100, 12, 100)), AIR);
        let expected = 256 * 10 * 256 + (15 * 25 * 16 - 15 * 5 * 16) - 2 * 2 * 30 - 2 * 2 * 10 + 2 * 2 * 5;
        // ground minus hole column inside ground, plus block above/inside ground region…
        // (exact accounting is easier against a brute-force count:)
        let mut brute = 0u64;
        for y in 0..64 {
            for z in 0..256 {
                for x in 0..256 {
                    if d.get(IVec3::new(x, y, z)) != AIR {
                        brute += 1;
                    }
                }
            }
        }
        assert_eq!(d.voxel_count(), brute);
        let _ = expected;
        assert!(s.solid_shortcuts > 0);
    }

    #[test]
    fn prefabs_link_by_pointer_and_dedup() {
        let p = pal();
        // A 16³ prefab (level 2): a frame with a hole.
        let frame = BoxShape { b: Aabb::new(IVec3::ZERO, IVec3::splat(16)), mat: 4 };
        let hole = BoxShape { b: Aabb::new(IVec3::new(4, 4, 0), IVec3::new(12, 12, 16)), mat: CARVE };
        let pre = prefab(&p, 2, &[&frame, &hole]);
        // Place it 4096 times in a 256³ world, aligned.
        let inst: Vec<Instance> = (0..16)
            .flat_map(|z| (0..16).flat_map(move |y| (0..16).map(move |x| (x, y, z))))
            .map(|(x, y, z)| Instance { dag: pre.clone(), origin: IVec3::new(x, y, z) * 16 })
            .collect();
        let refs: Vec<&dyn Shape> = inst.iter().map(|i| i as &dyn Shape).collect();
        let (d, s) = build(&p, 4, &refs);
        assert_eq!(s.sampled_leaves, 0, "nothing rasterised: every module is linked");
        assert_eq!(d.voxel_count(), 4096 * (16 * 16 * 16 - 8 * 8 * 16));
        assert_eq!(d.get(IVec3::new(16 * 3 + 6, 16 * 7 + 6, 5)), AIR);
        assert_eq!(d.get(IVec3::new(16 * 3 + 1, 16 * 7 + 6, 5)), 4);
        assert!(d.bytes() < 4096, "4096 instances share one subtree: {} bytes", d.bytes());
    }

    #[test]
    fn edits_append_and_keep_old_roots() {
        let p = pal();
        let ground = BoxShape { b: Aabb::new(IVec3::ZERO, IVec3::new(256, 10, 256)), mat: 1 };
        let (mut d, _) = build(&p, 4, &[&ground]);
        let (r0, len0) = (d.root, d.words.len());
        let before = d.words.clone();
        let hole = BoxShape { b: Aabb::new(IVec3::new(8, 0, 8), IVec3::new(10, 40, 10)), mat: CARVE };
        let wall = BoxShape { b: Aabb::new(IVec3::new(100, 10, 3), IVec3::new(103, 30, 50)), mat: 3 };
        let r1 = edit(&mut d, &p, &[&hole, &wall]);
        assert_eq!(&d.words[..len0], &before[..], "old words untouched");
        assert!(d.words.len() - len0 < 4000, "only the changed paths: {}", d.words.len() - len0);
        assert_eq!(d.get(IVec3::new(9, 2, 9)), AIR, "carved");
        assert_eq!(d.get(IVec3::new(101, 20, 10)), 3, "wall");
        assert_eq!(d.get(IVec3::new(50, 5, 50)), 1, "ground kept");
        // The old root still reads the old world.
        assert_eq!(d.get_in(r0, 4, IVec3::new(9, 2, 9)), 1);
        assert_eq!(d.get_in(r0, 4, IVec3::new(101, 20, 10)), AIR);
        // Same as building everything at once.
        let (all, _) = build(&p, 4, &[&ground, &hole, &wall]);
        assert_eq!(d.voxel_count(), all.voxel_count());
        assert_eq!(d.avg(r1), all.avg(all.root), "averages (LOD) match a full rebuild");
        // A prefab stamped into the edited world.
        let pre = prefab(&p, 2, &[&BoxShape { b: Aabb::new(IVec3::ZERO, IVec3::splat(16)), mat: 5 }]);
        edit(&mut d, &p, &[&Instance { dag: pre, origin: IVec3::new(64, 16, 64) }]);
        assert_eq!(d.get(IVec3::new(70, 20, 70)), 5);
        assert_eq!(d.get(IVec3::new(9, 2, 9)), AIR, "earlier edit kept");
    }

    #[test]
    fn misaligned_instances_still_render_correctly() {
        let p = pal();
        let b = BoxShape { b: Aabb::new(IVec3::new(1, 2, 3), IVec3::new(10, 11, 12)), mat: 2 };
        let pre = prefab(&p, 2, &[&b]);
        let i = Instance { dag: pre.clone(), origin: IVec3::new(7, 5, 3) };
        let (d, _) = build(&p, 3, &[&i]);
        for z in 0..64 {
            for y in 0..64 {
                for x in 0..64 {
                    let q = IVec3::new(x, y, z);
                    assert_eq!(d.get(q), pre.get(q - IVec3::new(7, 5, 3)), "{q}");
                }
            }
        }
    }
}

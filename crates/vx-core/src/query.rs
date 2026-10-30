//! Queries on a finished DAG: bounds, appending other DAGs (prefabs) into the same pool, and a
//! box collider for physics.
//!
//! Everything here is integer-only, so a physics step that uses [`Collider::blocked`] gives the
//! same answer natively and in wasm.

use glam::IVec3;
use rustc_hash::FxHashMap;

use crate::dag::{Dag, EMPTY, HEADER, NodeRef, Pool, child_xyz, side};
use crate::mat::{Mat, Palette, flag};

impl Dag {
    /// Copy `other` into this pool (relocating its child offsets) and return its root here.
    /// The renderer traces prefabs (players, props) from the same buffer as the world.
    pub fn append(&mut self, other: &Dag) -> NodeRef {
        let mut memo = FxHashMap::default();
        append_node(&mut self.words, other, other.root, other.levels, &mut memo)
    }

    /// The box `[min, max)` around every non-empty voxel under `n` (a node of `level`), or None.
    pub fn bounds_of(&self, n: NodeRef, level: u32) -> Option<(IVec3, IVec3)> {
        if n == EMPTY {
            return None;
        }
        let mut lo = IVec3::splat(i32::MAX);
        let mut hi = IVec3::splat(i32::MIN);
        bounds_rec(self, n, level, IVec3::ZERO, &mut lo, &mut hi);
        (lo.x <= hi.x).then_some((lo, hi))
    }
}

fn append_node(
    out: &mut Vec<u32>,
    src: &Dag,
    n: NodeRef,
    level: u32,
    memo: &mut FxHashMap<NodeRef, NodeRef>,
) -> NodeRef {
    if n == EMPTY {
        return EMPTY;
    }
    if let Some(&m) = memo.get(&n) {
        return m;
    }
    let len = src.node_len(n, level);
    let mut node: Vec<u32> = src.words()[n as usize..n as usize + len].to_vec();
    if level > 1 {
        for w in node[HEADER..].iter_mut() {
            *w = append_node(out, src, *w, level - 1, memo);
        }
    }
    let at = out.len() as NodeRef;
    out.extend_from_slice(&node);
    memo.insert(n, at);
    at
}

fn bounds_rec(d: &Dag, n: NodeRef, level: u32, origin: IVec3, lo: &mut IVec3, hi: &mut IVec3) {
    let mask = d.mask(n);
    let s = side(level - 1);
    for i in 0..64 {
        if mask >> i & 1 == 0 {
            continue;
        }
        let o = origin + child_xyz(i) * s;
        // Skip children that can't grow the box.
        if o.cmpge(*lo).all() && (o + s).cmple(*hi).all() {
            continue;
        }
        if level == 1 {
            *lo = lo.min(o);
            *hi = hi.max(o + 1);
        } else {
            bounds_rec(d, d.child(n, i), level - 1, o, lo, hi);
        }
    }
}

const ANY: u8 = 1;
const ALL: u8 = 2;
const DONE: u8 = 4;

/// Answers "is any blocking voxel inside this box?" in a few node visits.
///
/// Built once per world: every node gets two bits — *contains a blocking voxel* and *is all
/// blocking* — so a query skips empty space and stops at solid space without descending.
/// Water (and anything without the SOLID flag) doesn't block.
pub struct Collider {
    flags: Vec<u8>,
    root: NodeRef,
    levels: u32,
    blocks: Vec<bool>,
}

impl Collider {
    pub fn new(dag: &Dag, palette: &Palette) -> Self {
        let blocks: Vec<bool> = (0..palette.materials.len().max(1))
            .map(|i| {
                let m = palette.get(i as Mat);
                m.flags & flag::SOLID != 0 && m.flags & flag::WATER == 0
            })
            .collect();
        Self::with_blocking(dag, blocks)
    }

    /// `blocks[mat]` says whether material `mat` stops movement.
    pub fn with_blocking(dag: &Dag, blocks: Vec<bool>) -> Self {
        let mut c = Collider { flags: vec![0; dag.words.len()], root: dag.root, levels: dag.levels, blocks };
        if dag.root != EMPTY {
            c.compute(dag, dag.root, dag.levels);
        }
        c
    }

    fn blocking(&self, m: Mat) -> bool {
        self.blocks.get(m as usize).copied().unwrap_or(true)
    }

    fn compute(&mut self, d: &Dag, n: NodeRef, level: u32) -> u8 {
        let f = self.flags[n as usize];
        if f & DONE != 0 {
            return f;
        }
        let mask = d.mask(n);
        let mut any = false;
        let mut all = mask == u64::MAX;
        for i in 0..64 {
            if mask >> i & 1 == 0 {
                continue;
            }
            let (a, l) = if level == 1 {
                let b = self.blocking(d.leaf_mat(n, i));
                (b, b)
            } else {
                let cf = self.compute(d, d.child(n, i), level - 1);
                (cf & ANY != 0, cf & ALL != 0)
            };
            any |= a;
            all &= l;
        }
        let f = DONE | if any { ANY } else { 0 } | if all { ALL } else { 0 };
        self.flags[n as usize] = f;
        f
    }

    /// Any blocking voxel in the integer box `[min, max)`? Outside the world is empty.
    pub fn blocked(&self, d: &Dag, min: IVec3, max: IVec3) -> bool {
        if self.root == EMPTY || min.cmpge(max).any() {
            return false;
        }
        let s = side(self.levels);
        let (min, max) = (min.max(IVec3::ZERO), max.min(IVec3::splat(s)));
        if min.cmpge(max).any() {
            return false;
        }
        self.query(d, self.root, self.levels, IVec3::ZERO, min, max)
    }

    fn query(&self, d: &Dag, n: NodeRef, level: u32, origin: IVec3, min: IVec3, max: IVec3) -> bool {
        let f = self.flags[n as usize];
        if f & ANY == 0 {
            return false;
        }
        let s = side(level);
        // The box covers this whole node and it's all blocking.
        if f & ALL != 0 {
            return true;
        }
        let cs = side(level - 1);
        // Child index range overlapped by the box.
        let a = ((min - origin) / cs).clamp(IVec3::ZERO, IVec3::splat(3));
        let b = ((max - origin - 1) / cs).clamp(IVec3::ZERO, IVec3::splat(3));
        let _ = s;
        let mask = d.mask(n);
        for z in a.z..=b.z {
            for y in a.y..=b.y {
                for x in a.x..=b.x {
                    let i = (x | y << 2 | z << 4) as usize;
                    if mask >> i & 1 == 0 {
                        continue;
                    }
                    if level == 1 {
                        if self.blocking(d.leaf_mat(n, i)) {
                            return true;
                        }
                    } else {
                        let o = origin + IVec3::new(x, y, z) * cs;
                        if self.query(d, d.child(n, i), level - 1, o, min.max(o), max.min(o + cs)) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Aabb, BoxShape, Material, build};

    #[test]
    fn collider_matches_brute_force() {
        let mut p = Palette::default();
        let stone = p.add(Material::new("stone", 0x888888, 0.8));
        let water = p.add(Material::new("water", 0x2255aa, 0.1).water());
        let a = BoxShape { b: Aabb::new(IVec3::new(10, 0, 10), IVec3::new(200, 40, 90)), mat: stone };
        let w = BoxShape { b: Aabb::new(IVec3::new(0, 40, 0), IVec3::new(256, 60, 256)), mat: water };
        let c = BoxShape { b: Aabb::new(IVec3::new(100, 60, 100), IVec3::new(103, 70, 101)), mat: stone };
        let (dag, _) = build(&p, 4, &[&a, &w, &c]);
        let col = Collider::new(&dag, &p);
        let brute = |min: IVec3, max: IVec3| {
            for z in min.z.max(0)..max.z.min(256) {
                for y in min.y.max(0)..max.y.min(256) {
                    for x in min.x.max(0)..max.x.min(256) {
                        if dag.get(IVec3::new(x, y, z)) == stone {
                            return true;
                        }
                    }
                }
            }
            false
        };
        let mut seed = 7u32;
        let mut r = |n: i32| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as i32 % n
        };
        for _ in 0..400 {
            let min = IVec3::new(r(260) - 2, r(100) - 2, r(260) - 2);
            let max = min + IVec3::new(r(30) + 1, r(30) + 1, r(30) + 1);
            assert_eq!(col.blocked(&dag, min, max), brute(min, max), "{min} {max}");
        }
    }

    #[test]
    fn append_and_bounds() {
        let mut p = Palette::default();
        let stone = p.add(Material::new("stone", 0x888888, 0.8));
        let a = BoxShape { b: Aabb::new(IVec3::new(3, 4, 5), IVec3::new(20, 9, 11)), mat: stone };
        let (prefab, _) = build(&p, 3, &[&a]);
        let (mut world, _) =
            build(&p, 4, &[&BoxShape { b: Aabb::new(IVec3::ZERO, IVec3::new(256, 8, 256)), mat: stone }]);
        let before = world.voxel_count();
        let r = world.append(&prefab);
        assert_eq!(world.voxel_count(), before, "world unchanged");
        assert_eq!(world.bounds_of(r, 3), Some((IVec3::new(3, 4, 5), IVec3::new(20, 9, 11))));
        assert_eq!(world.get_in(r, 3, IVec3::new(10, 5, 6)), stone);
    }
}

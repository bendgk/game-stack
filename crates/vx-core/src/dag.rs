//! The voxel DAG: a sparse 64-tree whose identical subtrees are stored once.
//!
//! Every node covers a cube of 4^level voxels split into a 4×4×4 grid of children
//! (child index `i = x | y << 2 | z << 4`). Nodes live in one flat `u32` pool:
//!
//! ```text
//! [mask_lo, mask_hi, avg, payload…]
//!   mask    64-bit occupancy: bit i set ⇔ child i is non-empty
//!   avg     RGBA8 — mean albedo of the subtree (rgb) and its solid coverage (a);
//!           the renderer shades distant nodes with it instead of descending (LOD)
//!   payload level ≥ 2: one u32 pool offset per set bit, in bit order
//!           level = 1: one u16 material per set bit (packed two per word)
//! ```
//!
//! Nodes are *hash-consed*: building a node whose words already exist returns the
//! existing offset. A solid concrete slab collapses to one node per level; a window
//! module used 40 000 times across a city is stored once. That is what lets a scene
//! address 10¹¹–10¹² voxels in a few hundred MB (the GPU walks the same pool).
//!
//! Offset 0 is reserved and means "empty".

use glam::{IVec3, Vec3};
use hashbrown::HashTable;
use rustc_hash::{FxHashMap, FxHasher};
use std::hash::Hasher;

use crate::mat::{AIR, Mat, Palette};

pub type NodeRef = u32;
pub const EMPTY: NodeRef = 0;
pub const HEADER: usize = 3;

#[inline]
pub fn child_index(x: i32, y: i32, z: i32) -> usize {
    (x | (y << 2) | (z << 4)) as usize
}

#[inline]
pub fn child_xyz(i: usize) -> IVec3 {
    IVec3::new((i & 3) as i32, ((i >> 2) & 3) as i32, ((i >> 4) & 3) as i32)
}

/// Voxels per side of a node at `level`.
#[inline]
pub fn side(level: u32) -> i32 {
    1 << (2 * level)
}

/// An immutable, shareable DAG (the result of a build).
#[derive(Clone, Debug, Default)]
pub struct Dag {
    pub words: Vec<u32>,
    pub root: NodeRef,
    /// Level of the root: the world spans `4^levels` voxels per axis.
    pub levels: u32,
}

/// Node-pool reading, shared by [`Dag`] and [`DagBuilder`].
pub trait Pool {
    fn words(&self) -> &[u32];

    #[inline]
    fn mask(&self, n: NodeRef) -> u64 {
        let w = self.words();
        w[n as usize] as u64 | (w[n as usize + 1] as u64) << 32
    }
    #[inline]
    fn avg(&self, n: NodeRef) -> u32 {
        if n == EMPTY { 0 } else { self.words()[n as usize + 2] }
    }
    /// Child `i` of an interior node (level ≥ 2), or EMPTY.
    #[inline]
    fn child(&self, n: NodeRef, i: usize) -> NodeRef {
        let m = self.mask(n);
        if m >> i & 1 == 0 {
            return EMPTY;
        }
        let rank = (m & ((1u64 << i) - 1)).count_ones() as usize;
        self.words()[n as usize + HEADER + rank]
    }
    /// Voxel `i` of a leaf node (level 1).
    #[inline]
    fn leaf_mat(&self, n: NodeRef, i: usize) -> Mat {
        let m = self.mask(n);
        if m >> i & 1 == 0 {
            return AIR;
        }
        let rank = (m & ((1u64 << i) - 1)).count_ones() as usize;
        let w = self.words()[n as usize + HEADER + rank / 2];
        (if rank & 1 == 0 { w & 0xffff } else { w >> 16 }) as Mat
    }
    /// Words a node occupies.
    #[inline]
    fn node_len(&self, n: NodeRef, level: u32) -> usize {
        let c = self.mask(n).count_ones() as usize;
        HEADER + if level == 1 { c.div_ceil(2) } else { c }
    }
    /// The material at voxel `p` inside node `n` (level `level`, origin at 0).
    fn get_in(&self, mut n: NodeRef, mut level: u32, p: IVec3) -> Mat {
        while n != EMPTY {
            let s = 2 * (level - 1);
            let c = child_index((p.x >> s) & 3, (p.y >> s) & 3, (p.z >> s) & 3);
            if level == 1 {
                return self.leaf_mat(n, c);
            }
            n = self.child(n, c);
            level -= 1;
        }
        AIR
    }
    /// Descend to the node covering the aligned region `(p, level)` — or EMPTY.
    fn descend(&self, mut n: NodeRef, mut level: u32, p: IVec3, target: u32) -> NodeRef {
        while n != EMPTY && level > target {
            let s = 2 * (level - 1);
            n = self.child(n, child_index((p.x >> s) & 3, (p.y >> s) & 3, (p.z >> s) & 3));
            level -= 1;
        }
        n
    }
}

impl Pool for Dag {
    fn words(&self) -> &[u32] {
        &self.words
    }
}

impl Dag {
    pub fn side(&self) -> i32 {
        side(self.levels)
    }
    pub fn get(&self, p: IVec3) -> Mat {
        let s = self.side();
        if p.cmplt(IVec3::ZERO).any() || p.cmpge(IVec3::splat(s)).any() {
            return AIR;
        }
        self.get_in(self.root, self.levels, p)
    }
    pub fn bytes(&self) -> usize {
        self.words.len() * 4
    }

    /// Solid voxels in the scene (counting every instance of every shared subtree).
    pub fn voxel_count(&self) -> u64 {
        fn go(d: &Dag, n: NodeRef, level: u32, memo: &mut FxHashMap<(NodeRef, u32), u64>) -> u64 {
            if n == EMPTY {
                return 0;
            }
            if level == 1 {
                return d.mask(n).count_ones() as u64;
            }
            if let Some(v) = memo.get(&(n, level)) {
                return *v;
            }
            let m = d.mask(n);
            let mut total = 0u64;
            let mut bits = m;
            while bits != 0 {
                let i = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                total += go(d, d.child(n, i), level - 1, memo);
            }
            memo.insert((n, level), total);
            total
        }
        go(self, self.root, self.levels, &mut FxHashMap::default())
    }

    /// Unique nodes per level (index = level).
    pub fn node_counts(&self) -> Vec<usize> {
        let mut seen = vec![rustc_hash::FxHashSet::default(); self.levels as usize + 1];
        fn go(d: &Dag, n: NodeRef, level: u32, seen: &mut [rustc_hash::FxHashSet<NodeRef>]) {
            if n == EMPTY || !seen[level as usize].insert(n) || level == 1 {
                return;
            }
            let mut bits = d.mask(n);
            while bits != 0 {
                let i = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                go(d, d.child(n, i), level - 1, seen);
            }
        }
        go(self, self.root, self.levels, &mut seen);
        seen.iter().map(|s| s.len()).collect()
    }

    /// First solid voxel along a ray (voxel units). Returns (voxel, t, face normal).
    /// Hierarchical: empty space is skipped one empty node at a time.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<RayHit> {
        let world = self.side() as f32;
        let dir = dir.normalize_or_zero();
        if dir == Vec3::ZERO {
            return None;
        }
        let inv = dir.recip();
        // Clip to the world cube.
        let t0 = (Vec3::ZERO - origin) * inv;
        let t1 = (Vec3::splat(world) - origin) * inv;
        let tmin = t0.min(t1).max_element().max(0.0);
        let tmax = t0.max(t1).min_element().min(max_t);
        if tmin > tmax {
            return None;
        }
        let mut t = tmin;
        let mut normal = IVec3::ZERO;
        if tmin > 0.0 {
            let ax =
                (t0.min(t1)).to_array().iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i).unwrap();
            normal[ax] = if dir[ax] > 0.0 { -1 } else { 1 };
        }
        let step = dir.signum().as_ivec3();
        for _ in 0..100_000 {
            if t > tmax {
                return None;
            }
            // Point slightly inside the current cell, biased along the ray.
            let p = origin + dir * (t + 1e-4);
            let v = p.floor().as_ivec3();
            if v.cmplt(IVec3::ZERO).any() || v.cmpge(IVec3::splat(self.side())).any() {
                return None;
            }
            // Descend until we hit a solid voxel or an empty node.
            let mut n = self.root;
            let mut level = self.levels;
            let empty_size;
            loop {
                let s = 2 * (level - 1);
                let c = child_index((v.x >> s) & 3, (v.y >> s) & 3, (v.z >> s) & 3);
                if level == 1 {
                    let m = self.leaf_mat(n, c);
                    if m != AIR {
                        return Some(RayHit { voxel: v, t, normal, mat: m });
                    }
                    empty_size = 1;
                    break;
                }
                let ch = self.child(n, c);
                if ch == EMPTY {
                    empty_size = side(level - 1);
                    break;
                }
                n = ch;
                level -= 1;
            }
            // Leave the empty cube of `empty_size` containing v.
            let lo = (v / empty_size) * empty_size;
            let lo = lo.as_vec3();
            let hi = lo + Vec3::splat(empty_size as f32);
            // Exit distance per axis; an axis the ray doesn't move along never exits.
            let exit = |d: f32, lo: f32, hi: f32, o: f32, inv: f32| {
                if d > 0.0 {
                    (hi - o) * inv
                } else if d < 0.0 {
                    (lo - o) * inv
                } else {
                    f32::INFINITY
                }
            };
            let tx = exit(dir.x, lo.x, hi.x, origin.x, inv.x);
            let ty = exit(dir.y, lo.y, hi.y, origin.y, inv.y);
            let tz = exit(dir.z, lo.z, hi.z, origin.z, inv.z);
            let (mut tn, mut ax) = (tx, 0);
            if ty < tn {
                tn = ty;
                ax = 1;
            }
            if tz < tn {
                tn = tz;
                ax = 2;
            }
            normal = IVec3::ZERO;
            normal[ax] = -step[ax];
            t = tn.max(t);
        }
        None
    }

    /// Serialise: magic, levels, root, word count, words (little endian).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.words.len() * 4);
        out.extend_from_slice(b"VXD1");
        out.extend_from_slice(&self.levels.to_le_bytes());
        out.extend_from_slice(&self.root.to_le_bytes());
        out.extend_from_slice(&(self.words.len() as u32).to_le_bytes());
        out.extend_from_slice(bytemuck::cast_slice(&self.words));
        out
    }

    pub fn from_bytes(b: &[u8]) -> Result<Dag, String> {
        if b.len() < 16 || &b[0..4] != b"VXD1" {
            return Err("not a VXD1 DAG".into());
        }
        let rd = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let (levels, root, n) = (rd(4), rd(8), rd(12) as usize);
        if b.len() != 16 + n * 4 || levels > 15 || root as usize >= n.max(1) {
            return Err("corrupt DAG".into());
        }
        let words = b[16..].as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        Ok(Dag { words, root, levels })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub voxel: IVec3,
    pub t: f32,
    pub normal: IVec3,
    pub mat: Mat,
}

/// Builds hash-consed nodes into a pool.
pub struct DagBuilder {
    pub words: Vec<u32>,
    table: HashTable<(u32, u32)>,
    /// RGBA8 albedo per material (for node averages).
    colors: Vec<u32>,
    solid_cache: FxHashMap<(Mat, u32), NodeRef>,
    scratch: Vec<u32>,
}

impl Pool for DagBuilder {
    fn words(&self) -> &[u32] {
        &self.words
    }
}

fn hash_words(w: &[u32]) -> u64 {
    let mut h = FxHasher::default();
    for x in w {
        h.write_u32(*x);
    }
    h.finish()
}

#[inline]
fn unpack(c: u32) -> [u32; 4] {
    [c & 255, (c >> 8) & 255, (c >> 16) & 255, c >> 24]
}

impl DagBuilder {
    pub fn new(palette: &Palette) -> Self {
        DagBuilder {
            words: vec![0, 0, 0], // offset 0 = EMPTY sentinel
            table: HashTable::new(),
            colors: palette.albedo_rgba(),
            solid_cache: FxHashMap::default(),
            scratch: Vec::with_capacity(70),
        }
    }

    pub fn unique_nodes(&self) -> usize {
        self.table.len()
    }

    fn intern(&mut self, node: &[u32]) -> NodeRef {
        let h = hash_words(node);
        let words = &self.words;
        if let Some(&(off, _)) = self.table.find(h, |&(off, len)| {
            len as usize == node.len() && &words[off as usize..off as usize + len as usize] == node
        }) {
            return off;
        }
        let off = self.words.len() as u32;
        self.words.extend_from_slice(node);
        let words = &self.words;
        self.table
            .insert_unique(h, (off, node.len() as u32), |&(o, l)| hash_words(&words[o as usize..(o + l) as usize]));
        off
    }

    /// A leaf (4×4×4 voxels, index `x | y<<2 | z<<4`). EMPTY if all air.
    pub fn leaf(&mut self, mats: &[Mat; 64]) -> NodeRef {
        let mut mask = 0u64;
        let mut acc = [0u32; 3];
        let mut n = 0u32;
        let mut node = std::mem::take(&mut self.scratch);
        node.clear();
        node.extend_from_slice(&[0, 0, 0]);
        let mut pending: Option<u32> = None;
        for (i, &m) in mats.iter().enumerate() {
            if m == AIR {
                continue;
            }
            mask |= 1 << i;
            let c = unpack(self.colors[m as usize]);
            acc[0] += c[0];
            acc[1] += c[1];
            acc[2] += c[2];
            n += 1;
            match pending.take() {
                None => pending = Some(m as u32),
                Some(lo) => node.push(lo | (m as u32) << 16),
            }
        }
        if let Some(lo) = pending {
            node.push(lo);
        }
        if mask == 0 {
            self.scratch = node;
            return EMPTY;
        }
        node[0] = mask as u32;
        node[1] = (mask >> 32) as u32;
        node[2] = (acc[0] / n) | (acc[1] / n) << 8 | (acc[2] / n) << 16 | ((n * 255 + 32) / 64) << 24;
        let r = self.intern(&node);
        self.scratch = node;
        r
    }

    /// An interior node from 64 children (EMPTY entries allowed). EMPTY if all empty.
    pub fn interior(&mut self, children: &[NodeRef; 64]) -> NodeRef {
        let mut mask = 0u64;
        let mut node = std::mem::take(&mut self.scratch);
        node.clear();
        node.extend_from_slice(&[0, 0, 0]);
        let (mut r, mut g, mut b, mut cov) = (0u64, 0u64, 0u64, 0u64);
        for (i, &c) in children.iter().enumerate() {
            if c == EMPTY {
                continue;
            }
            mask |= 1 << i;
            node.push(c);
            let a = unpack(self.words[c as usize + 2]);
            let w = a[3] as u64;
            r += a[0] as u64 * w;
            g += a[1] as u64 * w;
            b += a[2] as u64 * w;
            cov += w;
        }
        if mask == 0 {
            self.scratch = node;
            return EMPTY;
        }
        node[0] = mask as u32;
        node[1] = (mask >> 32) as u32;
        let wsum = cov.max(1);
        node[2] = (r / wsum) as u32
            | ((g / wsum) as u32) << 8
            | ((b / wsum) as u32) << 16
            | (((cov + 32) / 64) as u32).min(255) << 24;
        let out = self.intern(&node);
        self.scratch = node;
        out
    }

    /// The node for a cube of `level` entirely filled with `m` (cached).
    pub fn solid(&mut self, m: Mat, level: u32) -> NodeRef {
        if m == AIR {
            return EMPTY;
        }
        if let Some(&n) = self.solid_cache.get(&(m, level)) {
            return n;
        }
        let n = if level == 1 {
            self.leaf(&[m; 64])
        } else {
            let c = self.solid(m, level - 1);
            self.interior(&[c; 64])
        };
        self.solid_cache.insert((m, level), n);
        n
    }

    /// Copy (and dedup) node `n` of another pool into this one.
    pub fn import(
        &mut self,
        src: &impl Pool,
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
        let out = if level == 1 {
            let len = src.node_len(n, 1);
            let words = src.words()[n as usize..n as usize + len].to_vec();
            self.intern(&words)
        } else {
            let mut ch = [EMPTY; 64];
            let mut bits = src.mask(n);
            while bits != 0 {
                let i = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                ch[i] = self.import(src, src.child(n, i), level - 1, memo);
            }
            self.interior(&ch)
        };
        memo.insert(n, out);
        out
    }

    pub fn finish(self, root: NodeRef, levels: u32) -> Dag {
        Dag { words: self.words, root, levels }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mat::Palette;

    #[test]
    fn hash_consing_dedups_identical_nodes() {
        let pal = Palette::default_city();
        let mut b = DagBuilder::new(&pal);
        let mut m = [AIR; 64];
        m[5] = 3;
        m[63] = 7;
        let a = b.leaf(&m);
        let a2 = b.leaf(&m);
        assert_eq!(a, a2);
        assert_eq!(b.leaf_mat(a, 5), 3);
        assert_eq!(b.leaf_mat(a, 63), 7);
        assert_eq!(b.leaf_mat(a, 6), AIR);
        let s1 = b.solid(2, 4);
        let s2 = b.solid(2, 4);
        assert_eq!(s1, s2);
        assert_eq!(b.unique_nodes(), 1 + 4, "one leaf + one node per solid level");
    }

    #[test]
    fn solid_world_counts_every_voxel() {
        let pal = Palette::default_city();
        let mut b = DagBuilder::new(&pal);
        let root = b.solid(1, 8);
        let d = b.finish(root, 8);
        assert_eq!(d.voxel_count(), 65536u64.pow(3));
        assert!(d.bytes() < 4096, "2.8e14 voxels in {} bytes", d.bytes());
        assert_eq!(d.get(IVec3::new(12345, 60000, 3)), 1);
    }

    #[test]
    fn raycast_hits_and_skips_empty_space() {
        let pal = Palette::default_city();
        let mut b = DagBuilder::new(&pal);
        // Level-3 world (64³) with a single solid level-2 block at child (2,0,2) of the root.
        let block = b.solid(5, 2);
        let mut ch = [EMPTY; 64];
        ch[child_index(2, 0, 2)] = block;
        let root = b.interior(&ch);
        let d = b.finish(root, 3);
        let hit = d.raycast(Vec3::new(40.0, 8.0, -10.0), Vec3::new(0.0, 0.0, 1.0), 1000.0).unwrap();
        assert_eq!(hit.voxel, IVec3::new(40, 8, 32));
        assert_eq!(hit.normal, IVec3::new(0, 0, -1));
        assert!((hit.t - 42.0).abs() < 1e-3);
        assert!(d.raycast(Vec3::new(5.0, 8.0, -10.0), Vec3::Z, 1000.0).is_none());
        // Round-trip.
        let back = Dag::from_bytes(&d.to_bytes()).unwrap();
        assert_eq!(back.get(IVec3::new(33, 3, 40)), 5);
    }
}

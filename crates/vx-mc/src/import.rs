//! A Minecraft world → a voxelscape world.
//!
//! Every distinct block state is voxelized once (32³, with its textures) and its eight 16³
//! octants become nodes of the DAG; the world is then assembled from those nodes. Hash-consing
//! does the compression: every stone block is the same eight nodes, every 2×2×2 of stone is
//! one node, a whole stone hill is a handful. A 768 × 768 block area (≈ 7 × 10¹¹ voxels) comes
//! out at tens of MB, and travels and renders like any other world bundle.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use rayon::prelude::*;
use vx_core::{AIR, Dag, DagBuilder, EMPTY, Mat, Material, NodeRef, Palette, Vec3, side};

use crate::anvil::{self, Chunk, State};
use crate::assets::Assets;
use crate::nbt::{self, Tag};
use crate::voxelize::{self, Grid, Kind, N};
use crate::zip::Zip;

/// Where a world's files come from: a folder or a zip of one.
pub enum WorldFiles {
    Dir(std::path::PathBuf),
    Zip(Zip),
}

impl WorldFiles {
    pub fn open(path: &Path) -> Result<WorldFiles, String> {
        if path.is_dir() {
            Ok(WorldFiles::Dir(path.to_path_buf()))
        } else {
            Ok(WorldFiles::Zip(Zip::new(std::fs::read(path).map_err(|e| e.to_string())?)?))
        }
    }

    /// Every file path (relative, `/`-separated).
    fn list(&self) -> Vec<String> {
        match self {
            WorldFiles::Zip(z) => z.names().cloned().collect(),
            WorldFiles::Dir(d) => {
                let mut out = Vec::new();
                let mut stack = vec![d.clone()];
                while let Some(p) = stack.pop() {
                    for e in std::fs::read_dir(&p).into_iter().flatten().flatten() {
                        let path = e.path();
                        if path.is_dir() {
                            if out.len() < 200_000 {
                                stack.push(path);
                            }
                        } else if let Ok(rel) = path.strip_prefix(d) {
                            out.push(rel.to_string_lossy().replace('\\', "/"));
                        }
                    }
                }
                out
            }
        }
    }

    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        match self {
            WorldFiles::Zip(z) => z.read(rel),
            WorldFiles::Dir(d) => std::fs::read(d.join(rel)).ok(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    /// Blocks around the spawn point to import (the area is a square 2·radius wide).
    pub radius: i32,
    /// Import around this block (x, z) instead of the world spawn.
    pub center: Option<[i32; 2]>,
    pub title: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options { radius: 384, center: None, title: None }
    }
}

pub struct Imported {
    pub world: vx_gen::World,
    pub chunks: usize,
    pub states: usize,
    pub materials: usize,
    pub ms: u128,
}

fn read_level(files: &WorldFiles, root: &str) -> (String, [i32; 3]) {
    let t = files.read(&format!("{root}level.dat")).and_then(|b| nbt::parse_gz(&b).ok());
    let data = t.as_ref().and_then(|t| t.get("Data"));
    let name = data.and_then(|d| d.get("LevelName")).and_then(Tag::str).unwrap_or("Minecraft world").to_string();
    let int = |k: &str| data.and_then(|d| d.get(k)).and_then(Tag::int).map(|v| v as i32);
    let spawn = match (int("SpawnX"), int("SpawnY"), int("SpawnZ")) {
        (Some(x), Some(y), Some(z)) => [x, y, z],
        // Newer saves keep it in a compound.
        _ => data
            .and_then(|d| d.at(&["spawn", "pos"]))
            .and_then(Tag::ints)
            .filter(|p| p.len() == 3)
            .map(|p| [p[0], p[1], p[2]])
            .unwrap_or([0, 64, 0]),
    };
    (name, spawn)
}

/// Lower y bound of the world (blocks) and its height.
const Y0: i32 = -64;
const HEIGHT: i32 = 384;

struct Area {
    x0: i32,
    z0: i32,
    w: i32,
    /// Chunk grid over the area: index (cz - cz0) * cw + (cx - cx0).
    cols: Vec<Option<Column>>,
    cx0: i32,
    cz0: i32,
    cw: i32,
}

/// A section: palette (global ids), its 4096 indices, and for biome-tinted states one global
/// id per section biome, with the 64 biome cell indices.
struct Sec {
    pal: Vec<u32>,
    idx: Arc<Vec<u16>>,
    tinted: Vec<Option<Vec<u32>>>,
    biome_idx: Vec<u8>,
}

struct Column {
    /// Sections by index (y - Y0/16).
    secs: Vec<Option<Sec>>,
}

impl Area {
    /// Global state id at block (ax, ay, az) relative to the area's corner (0 = air).
    fn at(&self, ax: i32, ay: i32, az: i32) -> u32 {
        if ax < 0 || az < 0 || ay < 0 || ax >= self.w || az >= self.w || ay >= HEIGHT {
            return 0;
        }
        let (bx, bz) = (self.x0 + ax, self.z0 + az);
        let (cx, cz) = (bx.div_euclid(16) - self.cx0, bz.div_euclid(16) - self.cz0);
        let Some(Some(col)) = self.cols.get((cz * self.cw + cx) as usize) else { return 0 };
        let Some(Some(sec)) = col.secs.get((ay / 16) as usize) else { return 0 };
        let (lx, ly, lz) = (bx.rem_euclid(16), ay % 16, bz.rem_euclid(16));
        let i = if sec.idx.is_empty() { 0 } else { sec.idx[(ly * 256 + lz * 16 + lx) as usize] as usize };
        if let Some(Some(by_biome)) = sec.tinted.get(i) {
            let cell = ((ly / 4) * 16 + (lz / 4) * 4 + lx / 4) as usize;
            let b = sec.biome_idx.get(cell).copied().unwrap_or(0) as usize;
            return by_biome.get(b).copied().unwrap_or(0);
        }
        sec.pal.get(i).copied().unwrap_or(0)
    }

    /// Any non-air section touching the block box?
    fn any(&self, a: [i32; 3], size: i32) -> bool {
        let y0 = a[1].max(0) / 16;
        let y1 = ((a[1] + size).min(HEIGHT) - 1) / 16;
        for bz in (a[2].max(0)..(a[2] + size).min(self.w)).step_by(16).chain([(a[2] + size - 1).min(self.w - 1)]) {
            for bx in (a[0].max(0)..(a[0] + size).min(self.w)).step_by(16).chain([(a[0] + size - 1).min(self.w - 1)]) {
                let (cx, cz) = ((self.x0 + bx).div_euclid(16) - self.cx0, (self.z0 + bz).div_euclid(16) - self.cz0);
                if let Some(Some(col)) = self.cols.get((cz * self.cw + cx) as usize) {
                    for sy in y0..=y1 {
                        if col.secs.get(sy as usize).is_some_and(|s| s.is_some()) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

/// Voxel grids by global state id (None = air), in palette material ids.
struct Looks {
    grids: Vec<Option<Vec<Mat>>>,
}

fn material(rgb: [u8; 3], kind: Kind) -> Material {
    // 5 bits per channel: textures stay crisp, the palette stays small (only colours used).
    let q = |c: u8| (c as u32 >> 3) << 3 | (c as u32 >> 5);
    let color = q(rgb[0]) << 16 | q(rgb[1]) << 8 | q(rgb[2]);
    let name = format!("mc{:?}{color:06x}", kind);
    let m = Material::new(&name, color, 0.85).grain(0.02);
    match kind {
        Kind::Solid => m,
        Kind::Passable => {
            let mut m = m.foliage();
            m.flags &= !vx_core::flag::SOLID;
            m
        }
        Kind::Foliage => m.foliage(),
        Kind::Glass => Material { roughness: 0.08, ..m.glass() },
        Kind::Water => Material { roughness: 0.05, ..m.water() },
        Kind::Light => m.emit(5.0),
    }
}

fn build_looks(assets: &Assets, states: &[(State, Option<String>)], pal: &mut Palette) -> Looks {
    let grids: Vec<Option<Grid>> = states
        .par_iter()
        .map(|(s, biome)| match biome {
            Some(b) => voxelize::block_in(assets, s, voxelize::biome_tint(b)),
            None => voxelize::block(assets, s),
        })
        .collect();
    let mut ids: HashMap<([u8; 3], Kind), Mat> = HashMap::new();
    let grids = grids
        .into_iter()
        .map(|g| {
            g.map(|g| {
                g.iter()
                    .map(|v| match v {
                        None => AIR,
                        Some(v) => {
                            let key = ([v.rgb[0] & !7, v.rgb[1] & !7, v.rgb[2] & !7], v.kind);
                            *ids.entry(key).or_insert_with(|| pal.add(material(key.0, key.1)))
                        }
                    })
                    .collect()
            })
        })
        .collect();
    Looks { grids }
}

/// Per-thread DAG building with caches of block octants.
struct Ctx<'a> {
    b: DagBuilder,
    looks: &'a Looks,
    area: &'a Area,
    oct: HashMap<(u32, u8), NodeRef>,
    uniform3: HashMap<u32, NodeRef>,
}

impl Ctx<'_> {
    /// The 16³ node for octant `o` of block state `gid`.
    fn octant(&mut self, gid: u32, o: u8) -> NodeRef {
        if let Some(&n) = self.oct.get(&(gid, o)) {
            return n;
        }
        let n = match self.looks.grids.get(gid as usize).and_then(|g| g.as_ref()) {
            None => EMPTY,
            Some(g) => {
                let (ox, oy, oz) = ((o & 1) as usize * 16, (o >> 1 & 1) as usize * 16, (o >> 2 & 1) as usize * 16);
                let mut leaves = [EMPTY; 64];
                for (li, leaf) in leaves.iter_mut().enumerate() {
                    let (lx, ly, lz) = (li & 3, li >> 2 & 3, li >> 4 & 3);
                    let mut m = [AIR; 64];
                    for (vi, v) in m.iter_mut().enumerate() {
                        let (x, y, z) =
                            (ox + lx * 4 + (vi & 3), oy + ly * 4 + (vi >> 2 & 3), oz + lz * 4 + (vi >> 4 & 3));
                        *v = g[x + z * N + y * N * N];
                    }
                    *leaf = self.b.leaf(&m);
                }
                self.b.interior(&leaves)
            }
        };
        self.oct.insert((gid, o), n);
        n
    }

    /// Node of `level` (≥ 3) whose min corner is block `a` (relative to the area).
    fn node(&mut self, level: u32, a: [i32; 3]) -> NodeRef {
        let blocks = side(level) / 32;
        if !self.area.any(a, blocks) {
            return EMPTY;
        }
        let mut ch = [EMPTY; 64];
        if level == 3 {
            // 2×2×2 blocks; children are their octants.
            let mut ids = [0u32; 8];
            for (k, id) in ids.iter_mut().enumerate() {
                *id = self.area.at(a[0] + (k & 1) as i32, a[1] + (k >> 1 & 1) as i32, a[2] + (k >> 2 & 1) as i32);
            }
            if ids.iter().all(|i| *i == ids[0]) {
                if ids[0] == 0 {
                    return EMPTY;
                }
                if let Some(&n) = self.uniform3.get(&ids[0]) {
                    return n;
                }
            }
            for (i, c) in ch.iter_mut().enumerate() {
                let (cx, cy, cz) = (i & 3, i >> 2 & 3, i >> 4 & 3);
                let gid = ids[(cx / 2) | (cy / 2) << 1 | (cz / 2) << 2];
                if gid != 0 {
                    *c = self.octant(gid, ((cx % 2) | (cy % 2) << 1 | (cz % 2) << 2) as u8);
                }
            }
            let n = self.b.interior(&ch);
            if ids.iter().all(|i| *i == ids[0]) {
                self.uniform3.insert(ids[0], n);
            }
            return n;
        }
        let step = blocks / 4;
        for (i, c) in ch.iter_mut().enumerate() {
            let o =
                [a[0] + (i & 3) as i32 * step, a[1] + (i >> 2 & 3) as i32 * step, a[2] + (i >> 4 & 3) as i32 * step];
            *c = self.node(level - 1, o);
        }
        self.b.interior(&ch)
    }
}

/// Import a Minecraft Java world (a save folder, or a zip of one) around its spawn.
pub fn import(files: &WorldFiles, assets: &Assets, opt: &Options) -> Result<Imported, String> {
    let t0 = std::time::Instant::now();
    let names = files.list();
    // The save root: wherever level.dat is (zips often wrap the folder).
    let root = names
        .iter()
        .filter(|n| n.ends_with("level.dat"))
        .min_by_key(|n| n.len())
        .map(|n| n.trim_end_matches("level.dat").to_string())
        .unwrap_or_default();
    let (level_name, spawn) = read_level(files, &root);
    let regions: Vec<(i32, i32, String)> = names
        .iter()
        .filter(|n| n.starts_with(&root) && n.ends_with(".mca"))
        .filter(|n| {
            let rel = &n[root.len()..];
            rel.starts_with("region/") || rel.starts_with("dimensions/minecraft/overworld/region/")
        })
        .filter_map(|n| {
            let f = n.rsplit('/').next()?;
            let mut it = f.trim_end_matches(".mca").split('.').skip(1);
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?, n.clone()))
        })
        .collect();
    if regions.is_empty() {
        return Err("no overworld region files (.mca) found: pick a Minecraft Java save folder".into());
    }
    let r = opt.radius.clamp(32, 1024);
    let [cx, cz] = opt.center.unwrap_or([spawn[0], spawn[2]]);
    let (x0, z0, w) = (cx - r, cz - r, 2 * r);
    let (cx0, cz0) = (x0.div_euclid(16), z0.div_euclid(16));
    let cw = (x0 + w - 1).div_euclid(16) - cx0 + 1;
    let ch = (z0 + w - 1).div_euclid(16) - cz0 + 1;
    // Read the regions that overlap the area, in parallel.
    let wanted: Vec<&(i32, i32, String)> = regions
        .iter()
        .filter(|(rx, rz, _)| rx * 32 + 31 >= cx0 && rx * 32 < cx0 + cw && rz * 32 + 31 >= cz0 && rz * 32 < cz0 + ch)
        .collect();
    let chunks: Vec<Chunk> = wanted
        .par_iter()
        .flat_map(|(_, _, path)| files.read(path).map(|b| anvil::region(&b)).unwrap_or_default())
        .filter(|c| c.x >= cx0 && c.x < cx0 + cw && c.z >= cz0 && c.z < cz0 + ch)
        .collect();
    if chunks.is_empty() {
        return Err(format!("no chunks around ({cx}, {cz}); try another center"));
    }
    // Global block states.
    // Global ids: (state, biome tint) — the biome only for blocks that take its colour.
    let mut gids: BTreeMap<(State, Option<String>), u32> = BTreeMap::new();
    let mut states: Vec<(State, Option<String>)> =
        vec![(State { name: "minecraft:air".into(), props: Default::default() }, None)];
    let mut gid = |key: (State, Option<String>)| -> u32 {
        *gids.entry(key.clone()).or_insert_with(|| {
            states.push(key);
            (states.len() - 1) as u32
        })
    };
    let mut cols: Vec<Option<Column>> = (0..cw * ch).map(|_| None).collect();
    let nsec = (HEIGHT / 16) as usize;
    for c in &chunks {
        let mut col = Column { secs: (0..nsec).map(|_| None).collect() };
        for s in &c.sections {
            let si = s.y - Y0 / 16;
            if si < 0 || si as usize >= nsec {
                continue;
            }
            let biomes: Vec<String> =
                if s.biomes.is_empty() { vec!["minecraft:plains".into()] } else { s.biomes.clone() };
            let mut pal = Vec::with_capacity(s.palette.len());
            let mut tinted = Vec::with_capacity(s.palette.len());
            for st in &s.palette {
                if st.is_air() {
                    pal.push(0);
                    tinted.push(None);
                } else if voxelize::tinted(st) {
                    pal.push(gid((st.clone(), Some(biomes[0].clone()))));
                    tinted.push(Some(biomes.iter().map(|b| gid((st.clone(), Some(b.clone())))).collect()));
                } else {
                    pal.push(gid((st.clone(), None)));
                    tinted.push(None);
                }
            }
            col.secs[si as usize] =
                Some(Sec { pal, idx: Arc::new(s.idx.clone()), tinted, biome_idx: s.biome_idx.clone() });
        }
        cols[((c.z - cz0) * cw + (c.x - cx0)) as usize] = Some(col);
    }
    if std::env::var_os("VX_MC_DEBUG").is_some() {
        let secs: usize = chunks.iter().map(|c| c.sections.len()).sum();
        eprintln!(
            "spawn {spawn:?} root {root:?} regions {} wanted {} chunks {} sections {secs} states {}",
            regions.len(),
            wanted.len(),
            chunks.len(),
            states.len()
        );
        let mut biomes: BTreeMap<&str, usize> = BTreeMap::new();
        for c in &chunks {
            for s in &c.sections {
                for b in &s.biomes {
                    *biomes.entry(b.as_str()).or_default() += 1;
                }
            }
        }
        eprintln!("biomes {biomes:?}");
    }
    let area = Area { x0, z0, w, cols, cx0, cz0, cw };
    let mut palette = Palette::default();
    palette.add(Material::new("mc_base", 0x808080, 0.9));
    let looks = build_looks(assets, &states, &mut palette);
    // Assemble: level-5 cubes (32 blocks) in parallel, then the levels above.
    let span = (w.max(HEIGHT) * 32) as u32;
    let levels = (6..=10).find(|l| side(*l) as u32 >= span).unwrap_or(10);
    let n5 = side(levels) / side(5);
    let tasks: Vec<[i32; 3]> = (0..n5)
        .flat_map(|z| (0..n5).flat_map(move |y| (0..n5).map(move |x| [x * 32, y * 32, z * 32])))
        .filter(|a| a[0] < w && a[2] < w && a[1] < HEIGHT && area.any(*a, 32))
        .collect();
    let parts: Vec<([i32; 3], DagBuilder, NodeRef)> = tasks
        .par_iter()
        .map(|a| {
            let mut cx = Ctx {
                b: DagBuilder::new(&palette),
                looks: &looks,
                area: &area,
                oct: HashMap::new(),
                uniform3: HashMap::new(),
            };
            let n = cx.node(5, *a);
            (*a, cx.b, n)
        })
        .collect();
    let mut main = DagBuilder::new(&palette);
    let mut at: HashMap<[i32; 3], NodeRef> = HashMap::new();
    for (a, b, n) in parts {
        if n != EMPTY {
            let mut memo = Default::default();
            at.insert(a, main.import(&b, n, 5, &mut memo));
        }
    }
    fn up(main: &mut DagBuilder, at: &HashMap<[i32; 3], NodeRef>, level: u32, a: [i32; 3]) -> NodeRef {
        if level == 5 {
            return at.get(&a).copied().unwrap_or(EMPTY);
        }
        let step = side(level) / 32 / 4;
        let mut ch = [EMPTY; 64];
        for (i, c) in ch.iter_mut().enumerate() {
            *c = up(
                main,
                at,
                level - 1,
                [a[0] + (i & 3) as i32 * step, a[1] + (i >> 2 & 3) as i32 * step, a[2] + (i >> 4 & 3) as i32 * step],
            );
        }
        main.interior(&ch)
    }
    let root = up(&mut main, &at, levels, [0, 0, 0]);
    if root == EMPTY {
        return Err("the area is empty".into());
    }
    let dag: Dag = main.finish(root, levels);
    // Spawn: above the highest block at the spawn column.
    let (sx, sz) = (spawn[0] - x0, spawn[2] - z0);
    let top =
        (0..HEIGHT).rev().find(|y| area.at(sx.clamp(0, w - 1), *y, sz.clamp(0, w - 1)) != 0).unwrap_or(spawn[1] - Y0);
    let spawn_v = Vec3::new(sx as f32 * 32.0 + 16.0, (top + 1) as f32 * 32.0 + 8.0, sz as f32 * 32.0 + 16.0);
    let title = opt.title.clone().unwrap_or_else(|| format!("{level_name} (Minecraft)"));
    let spec =
        vx_gen::SceneSpec { title, size_m: w as f32, city: false, time_of_day: 13.0, fog: 0.15, ..Default::default() };
    let stats = vx_gen::GenStats { voxels: dag.voxel_count(), bytes: dag.bytes(), levels, ..Default::default() };
    let materials = palette.materials.len();
    let world = vx_gen::World { spec, dag, palette, spawn: spawn_v, stats };
    Ok(Imported { world, chunks: chunks.len(), states: states.len(), materials, ms: t0.elapsed().as_millis() })
}

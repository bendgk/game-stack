//! Blocks → voxels. A Minecraft block is 1 m, so 32 voxels per side: every texel of a 16×16
//! texture becomes 2×2 voxels on the face it belongs to, and the model's real shape is kept
//! (stairs, slabs, fences, torches, flowers, panes). Cutout texels (leaves, glass panes,
//! flowers) become air, so foliage is genuinely see-through voxels.
//!
//! This works for any game's block-and-texture content, not only Minecraft: give it boxes
//! (elements) with a texture per face.

use crate::anvil::State;
use crate::assets::{Assets, Element, Placed, Tex};

pub const N: usize = 32;

/// How a voxel behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Solid,
    /// No collision (flowers, grass, torches, rails…).
    Passable,
    /// Leaves and plants: wind and wrap lighting in the renderer.
    Foliage,
    /// See-through (glass, ice).
    Glass,
    Water,
    /// Emits light (glowstone, lanterns, lava…).
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Voxel {
    pub rgb: [u8; 3],
    pub kind: Kind,
}

/// 32³ voxels, index `x + z*32 + y*1024`.
pub type Grid = Vec<Option<Voxel>>;

fn has(name: &str, keys: &[&str]) -> bool {
    keys.iter().any(|k| name.contains(k))
}

/// How a block behaves, from its name.
pub fn kind_of(s: &State) -> Kind {
    let n = s.short();
    if has(n, &["water", "bubble_column"]) {
        return Kind::Water;
    }
    let lit = s.prop("lit") != Some("false");
    if has(
        n,
        &[
            "glowstone",
            "lantern",
            "froglight",
            "shroomlight",
            "lava",
            "magma",
            "end_rod",
            "beacon",
            "sea_pickle",
            "fire",
        ],
    ) || (n.contains("torch") && !n.contains("redstone"))
        || (lit && has(n, &["redstone_lamp", "campfire", "furnace", "smoker"]) && s.prop("lit") == Some("true"))
    {
        return Kind::Light;
    }
    if has(n, &["glass", "ice"]) && !n.contains("packed") {
        return Kind::Glass;
    }
    if n.ends_with("leaves") || n.contains("azalea") {
        return Kind::Foliage;
    }
    if has(
        n,
        &[
            "flower",
            "sapling",
            "fern",
            "short_grass",
            "tall_grass",
            "dead_bush",
            "vine",
            "kelp",
            "seagrass",
            "rail",
            "button",
            "pressure_plate",
            "sign",
            "banner",
            "cobweb",
            "lever",
            "redstone_wire",
            "tripwire",
            "ladder",
            "sugar_cane",
            "wheat",
            "carrots",
            "potatoes",
            "beetroots",
            "mushroom",
            "dandelion",
            "poppy",
            "orchid",
            "allium",
            "tulip",
            "daisy",
            "cornflower",
            "lily",
            "rose",
            "peony",
            "lilac",
            "sunflower",
            "bamboo_sapling",
            "sweet_berry",
            "roots",
            "sprouts",
            "torchflower",
            "pitcher",
            "spore",
            "glow_lichen",
            "moss_carpet",
            "snow",
            "carpet",
        ],
    ) && !n.ends_with("_block")
    {
        return Kind::Passable;
    }
    Kind::Solid
}

/// Biome colours for grey textures: (grass, foliage, water).
pub type Tint = ([u8; 3], [u8; 3], [u8; 3]);

pub const PLAINS: Tint = ([145, 189, 89], [119, 171, 47], [63, 118, 228]);

/// The tint of a biome (by name, without `minecraft:`). Close to the game's colour maps.
pub fn biome_tint(biome: &str) -> Tint {
    let b = biome.strip_prefix("minecraft:").unwrap_or(biome);
    let has = |k: &str| b.contains(k);
    if has("swamp") {
        ([106, 112, 57], [106, 112, 57], [97, 123, 100])
    } else if has("jungle") {
        ([89, 201, 60], [48, 187, 11], [20, 160, 200])
    } else if has("badlands") {
        ([144, 129, 77], [158, 129, 77], [63, 118, 228])
    } else if has("desert") || has("savanna") {
        ([191, 183, 85], [174, 164, 42], [50, 160, 200])
    } else if has("dark_forest") {
        ([80, 122, 50], [89, 174, 48], [63, 118, 228])
    } else if has("birch") {
        ([136, 187, 103], [107, 169, 65], [63, 118, 228])
    } else if has("taiga") || has("grove") || has("snowy") || has("frozen") || has("ice") {
        ([128, 180, 151], [96, 161, 123], [61, 87, 214])
    } else if has("cherry") {
        ([182, 219, 97], [182, 219, 97], [93, 183, 239])
    } else if has("mangrove") {
        ([106, 112, 57], [141, 177, 39], [58, 122, 106])
    } else if has("warm_ocean") {
        ([142, 185, 113], [113, 167, 77], [67, 213, 238])
    } else if has("cold_ocean") || has("deep") {
        ([142, 185, 113], [113, 167, 77], [61, 87, 214])
    } else if has("forest") || has("flower") {
        ([121, 192, 90], [89, 174, 48], [63, 118, 228])
    } else if has("mountain") || has("peak") || has("windswept") || has("stony") {
        ([138, 182, 137], [109, 163, 107], [63, 118, 228])
    } else {
        PLAINS
    }
}

/// The colour a block's grey texture is multiplied by, in a biome.
fn tint(name: &str, t: Tint) -> [u8; 3] {
    match name {
        n if n.contains("water") || n == "bubble_column" => t.2,
        "birch_leaves" => [128, 167, 85],
        "spruce_leaves" => [97, 153, 97],
        n if n.ends_with("leaves") || n.contains("vine") => t.1,
        "lily_pad" => [32, 128, 48],
        _ => t.0,
    }
}

/// Does this block take the biome's colour?
pub fn tinted(s: &State) -> bool {
    let n = s.short();
    (n.ends_with("leaves") && !matches!(n, "birch_leaves" | "spruce_leaves" | "cherry_leaves" | "azalea_leaves"))
        || matches!(
            n,
            "grass_block"
                | "short_grass"
                | "tall_grass"
                | "fern"
                | "large_fern"
                | "vine"
                | "sugar_cane"
                | "water"
                | "bubble_column"
        )
}

/// Turn `turns` quarter turns of (a, b) about the centre of a 16 × 16 square.
fn quarter(mut a: f32, mut b: f32, turns: i32) -> (f32, f32) {
    for _ in 0..turns.rem_euclid(4) {
        (a, b) = (16.0 - b, a);
    }
    (a, b)
}

/// Inverse of the blockstate's x then y rotation (degrees), on a point in 0..16 units.
fn unrotate(p: [f32; 3], x: i32, y: i32) -> [f32; 3] {
    let [mut px, mut py, mut pz] = p;
    // Undo y (clockwise from above: north → east), then x.
    for _ in 0..(y / 90).rem_euclid(4) {
        (px, pz) = (pz, 16.0 - px);
    }
    for _ in 0..(x / 90).rem_euclid(4) {
        (py, pz) = (16.0 - pz, py);
    }
    [px, py, pz]
}

fn sample(e: &Element, face: usize, q: [f32; 3], t: &Tex, rotation: i32) -> [u8; 4] {
    let (f, to) = (e.from, e.to);
    let d = |a: usize| (to[a] - f[a]).abs().max(1e-3);
    // Position across the face, 0..1 (a → u, b → v), Minecraft's default UV layout.
    let (a, b) = match face {
        0 => ((q[0] - f[0]) / d(0), (to[2] - q[2]) / d(2)),
        1 => ((q[0] - f[0]) / d(0), (q[2] - f[2]) / d(2)),
        2 => ((to[0] - q[0]) / d(0), (to[1] - q[1]) / d(1)),
        3 => ((q[0] - f[0]) / d(0), (to[1] - q[1]) / d(1)),
        4 => ((q[2] - f[2]) / d(2), (to[1] - q[1]) / d(1)),
        _ => ((to[2] - q[2]) / d(2), (to[1] - q[1]) / d(1)),
    };
    let uv = e.faces[face].as_ref().and_then(|x| x.uv).unwrap_or_else(|| match face {
        0 => [f[0], 16.0 - to[2], to[0], 16.0 - f[2]],
        1 => [f[0], f[2], to[0], to[2]],
        2 => [16.0 - to[0], 16.0 - to[1], 16.0 - f[0], 16.0 - f[1]],
        3 => [f[0], 16.0 - to[1], to[0], 16.0 - f[1]],
        4 => [f[2], 16.0 - to[1], to[2], 16.0 - f[1]],
        _ => [16.0 - to[2], 16.0 - to[1], 16.0 - f[2], 16.0 - f[1]],
    });
    let (a, b) = (a.clamp(0.0, 0.999), b.clamp(0.0, 0.999));
    let (u, v) = (uv[0] + a * (uv[2] - uv[0]), uv[1] + b * (uv[3] - uv[1]));
    let (u, v) = quarter(u, v, rotation / 90);
    t.at(u, v)
}

fn rotate_about(q: [f32; 3], rot: (usize, f32, [f32; 3])) -> [f32; 3] {
    let (axis, deg, o) = rot;
    let (s, c) = (-deg.to_radians()).sin_cos();
    let p = [q[0] - o[0], q[1] - o[1], q[2] - o[2]];
    let (i, j) = match axis {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let mut r = p;
    r[i] = p[i] * c - p[j] * s;
    r[j] = p[i] * s + p[j] * c;
    [r[0] + o[0], r[1] + o[1], r[2] + o[2]]
}

/// The voxel at `q` (0..16 units) of one element, if it covers it with an opaque texel.
fn element_at(e: &Element, q: [f32; 3], tint_rgb: [u8; 3]) -> Option<[u8; 4]> {
    let q = match e.rot {
        Some(r) => rotate_about(q, r),
        None => q,
    };
    // Flat elements (flower planes, pane centres) get at least one voxel of thickness.
    let mut lo = [0.0; 3];
    let mut hi = [0.0; 3];
    for a in 0..3 {
        let (mut l, mut h) = (e.from[a].min(e.to[a]), e.from[a].max(e.to[a]));
        if h - l < 0.5 {
            let pad = (0.5 - (h - l)) / 2.0;
            l -= pad;
            h += pad;
        }
        lo[a] = l;
        hi[a] = h;
        if q[a] < l || q[a] > h {
            return None;
        }
    }
    // The face this voxel belongs to: the nearest one the element has.
    let dist = [q[1] - lo[1], hi[1] - q[1], q[2] - lo[2], hi[2] - q[2], q[0] - lo[0], hi[0] - q[0]];
    let mut order: Vec<usize> = (0..6).filter(|i| e.faces[*i].is_some()).collect();
    order.sort_by(|a, b| dist[*a].total_cmp(&dist[*b]));
    let face = *order.first()?;
    let f = e.faces[face].as_ref()?;
    let t = f.tex.as_ref()?;
    let mut c = sample(e, face, q, t, f.rotation);
    if c[3] < 100 {
        return None;
    }
    if f.tint {
        for k in 0..3 {
            c[k] = ((c[k] as u32 * tint_rgb[k] as u32) / 255) as u8;
        }
    }
    Some(c)
}

/// Approximate colours for when there are no textures (a few common blocks; others hashed).
fn fallback_colour(n: &str) -> [u8; 3] {
    let table: &[(&str, [u8; 3])] = &[
        ("grass_block", [95, 159, 53]),
        ("dirt", [134, 96, 67]),
        ("deepslate", [80, 80, 82]),
        ("stone", [125, 125, 125]),
        ("cobble", [110, 110, 110]),
        ("sand", [219, 207, 163]),
        ("gravel", [131, 127, 126]),
        ("water", [63, 118, 228]),
        ("lava", [207, 92, 20]),
        ("snow", [249, 254, 254]),
        ("ice", [145, 183, 253]),
        ("leaves", [72, 132, 40]),
        ("log", [102, 81, 50]),
        ("planks", [162, 130, 78]),
        ("glass", [200, 220, 230]),
        ("brick", [150, 97, 83]),
        ("terracotta", [152, 94, 67]),
        ("clay", [160, 166, 179]),
        ("netherrack", [97, 38, 38]),
        ("ore", [140, 140, 140]),
        ("wool", [233, 236, 236]),
        ("chest", [160, 110, 45]),
        ("barrel", [120, 85, 50]),
        ("grass", [100, 160, 60]),
        ("flower", [200, 60, 60]),
    ];
    table.iter().find(|(k, _)| n.contains(k)).map(|x| x.1).unwrap_or_else(|| {
        let h = n.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
        [(96 + (h & 63)) as u8, (96 + (h >> 8 & 63)) as u8, (96 + (h >> 16 & 63)) as u8]
    })
}

/// Voxelize one block state. `None` = all air.
pub fn block(assets: &Assets, s: &State) -> Option<Grid> {
    block_in(assets, s, PLAINS)
}

/// Voxelize one block state with a biome tint. `None` = all air.
pub fn block_in(assets: &Assets, s: &State, t: Tint) -> Option<Grid> {
    if s.is_air() {
        return None;
    }
    let kind = kind_of(s);
    let name = s.short();
    let placed: Vec<Placed> = if assets.has_textures() { assets.block(s) } else { Vec::new() };
    let mut g: Grid = vec![None; N * N * N];
    let fluid = matches!(name, "water" | "lava" | "bubble_column");
    if placed.is_empty() || fluid {
        // Fluids (their models are empty) and blocks without a model: a coloured volume;
        // fluids follow their level, a little grain keeps them from looking flat.
        let tex = if fluid {
            assets.texture(&format!("minecraft:block/{}_still", if name == "lava" { "lava" } else { "water" }))
        } else {
            None
        };
        let level = s.prop("level").and_then(|l| l.parse::<u32>().ok()).unwrap_or(0);
        let top = if fluid && level > 0 && level < 8 { N - (level as usize * N / 9).min(N - 2) } else { N };
        let base = fallback_colour(name);
        let sparse = kind == Kind::Passable;
        for y in 0..top {
            for z in 0..N {
                for x in 0..N {
                    let h = ((x * 73856093) ^ (y * 19349663) ^ (z * 83492791)) as u32;
                    if sparse && !(x.abs_diff(16) < 3 && z.abs_diff(16) < 3 && y < 24) {
                        continue;
                    }
                    let rgb = match &tex {
                        Some(tx) => {
                            let c = tx.at(x as f32 / 2.0, z as f32 / 2.0);
                            if name == "lava" {
                                [c[0], c[1], c[2]]
                            } else {
                                let tw = tint(name, t);
                                [
                                    (c[0] as u32 * tw[0] as u32 / 255) as u8,
                                    (c[1] as u32 * tw[1] as u32 / 255) as u8,
                                    (c[2] as u32 * tw[2] as u32 / 255) as u8,
                                ]
                            }
                        }
                        None => {
                            let j = ((h >> 7) % 17) as i32 - 8;
                            [
                                (base[0] as i32 + j).clamp(0, 255) as u8,
                                (base[1] as i32 + j).clamp(0, 255) as u8,
                                (base[2] as i32 + j).clamp(0, 255) as u8,
                            ]
                        }
                    };
                    g[x + z * N + y * N * N] = Some(Voxel { rgb, kind });
                }
            }
        }
        return Some(g);
    }
    let tint_rgb = tint(name, t);
    let mut any = false;
    for y in 0..N {
        for z in 0..N {
            for x in 0..N {
                let p = [(x as f32 + 0.5) / 2.0, (y as f32 + 0.5) / 2.0, (z as f32 + 0.5) / 2.0];
                let hit = placed.iter().find_map(|pl| {
                    let q = unrotate(p, pl.x, pl.y);
                    pl.elements.iter().find_map(|e| element_at(e, q, tint_rgb))
                });
                if let Some(c) = hit {
                    // Light blocks glow where they're bright (the flame, not the lantern's iron).
                    let k = if kind == Kind::Light && (c[0] as u32 + c[1] as u32 + c[2] as u32) < 330 {
                        Kind::Solid
                    } else {
                        kind
                    };
                    g[x + z * N + y * N * N] = Some(Voxel { rgb: [c[0], c[1], c[2]], kind: k });
                    any = true;
                }
            }
        }
    }
    any.then_some(g)
}

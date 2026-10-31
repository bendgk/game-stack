//! Anvil region files (`r.X.Z.mca`): 32×32 chunks of 16×16 columns, each a compressed NBT
//! document whose sections hold paletted 16³ block states (Java Edition 1.16+ packing; the
//! pre-1.18 `Level` wrapper is understood too).

use std::collections::BTreeMap;
use std::io::Read;

use crate::nbt::{self, Tag};

/// A block state as written in the world: `minecraft:oak_stairs` + properties.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct State {
    pub name: String,
    pub props: BTreeMap<String, String>,
}

impl State {
    /// Without the `minecraft:` prefix.
    pub fn short(&self) -> &str {
        self.name.strip_prefix("minecraft:").unwrap_or(&self.name)
    }
    pub fn prop(&self, k: &str) -> Option<&str> {
        self.props.get(k).map(String::as_str)
    }
    pub fn is_air(&self) -> bool {
        matches!(self.short(), "air" | "cave_air" | "void_air" | "structure_void" | "barrier" | "light")
    }
}

/// One 16³ section: a palette and 4096 indices (`y*256 + z*16 + x`), or one state.
#[derive(Clone, Debug)]
pub struct Section {
    /// Section index (block y / 16).
    pub y: i32,
    pub palette: Vec<State>,
    /// Empty = every block is `palette[0]`.
    pub idx: Vec<u16>,
}

impl Section {
    pub fn at(&self, x: usize, y: usize, z: usize) -> u16 {
        if self.idx.is_empty() { 0 } else { self.idx[y * 256 + z * 16 + x] }
    }
}

#[derive(Clone, Debug)]
pub struct Chunk {
    pub x: i32,
    pub z: i32,
    pub sections: Vec<Section>,
}

fn unpack(data: &[i64], bits: usize) -> Vec<u16> {
    let per = 64 / bits;
    let mask = (1u64 << bits) - 1;
    let mut out = Vec::with_capacity(4096);
    for i in 0..4096 {
        let Some(w) = data.get(i / per) else { break };
        out.push(((*w as u64 >> ((i % per) * bits)) & mask) as u16);
    }
    out.resize(4096, 0);
    out
}

fn states(palette: &[Tag]) -> Vec<State> {
    palette
        .iter()
        .map(|p| State {
            // Newer saves write property-less states as a bare string.
            name: p.str().or_else(|| p.get("Name").and_then(Tag::str)).unwrap_or("minecraft:air").to_string(),
            props: p
                .get("Properties")
                .and_then(Tag::compound)
                .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.str()?.to_string()))).collect())
                .unwrap_or_default(),
        })
        .collect()
}

/// Decode one chunk's NBT.
pub fn chunk(t: &Tag) -> Option<Chunk> {
    let root = t.get("Level").unwrap_or(t);
    let x = root.get("xPos")?.int()? as i32;
    let z = root.get("zPos")?.int()? as i32;
    let secs = root.get("sections").or_else(|| root.get("Sections"))?;
    let mut sections = Vec::new();
    for s in secs.list() {
        let y = s.get("Y").and_then(Tag::int).unwrap_or(0) as i32;
        // 1.18+: block_states { palette, data }; 1.16–1.17: Palette + BlockStates.
        let (pal, data) = match s.get("block_states") {
            Some(bs) => {
                (bs.get("palette").map(Tag::list).unwrap_or(&[]), bs.get("data").map(Tag::longs).unwrap_or(&[]))
            }
            None => {
                (s.get("Palette").map(Tag::list).unwrap_or(&[]), s.get("BlockStates").map(Tag::longs).unwrap_or(&[]))
            }
        };
        if pal.is_empty() {
            continue;
        }
        let palette = states(pal);
        if palette.len() == 1 && palette[0].is_air() {
            continue;
        }
        let idx = if palette.len() == 1 || data.is_empty() {
            Vec::new()
        } else {
            let bits = (usize::BITS - (palette.len() - 1).leading_zeros()).max(4) as usize;
            let mut v = unpack(data, bits);
            let n = palette.len() as u16;
            v.iter_mut().for_each(|i| {
                if *i >= n {
                    *i = 0;
                }
            });
            v
        };
        sections.push(Section { y, palette, idx });
    }
    Some(Chunk { x, z, sections })
}

/// Every chunk in a region file.
pub fn region(bytes: &[u8]) -> Vec<Chunk> {
    let mut out = Vec::new();
    if bytes.len() < 8192 {
        return out;
    }
    for i in 0..1024 {
        let loc = u32::from_be_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
        let (off, sectors) = ((loc >> 8) as usize * 4096, (loc & 255) as usize);
        if off == 0 || sectors == 0 || off + 5 > bytes.len() {
            continue;
        }
        let len = u32::from_be_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
        let kind = bytes[off + 4];
        let Some(body) = bytes.get(off + 5..off + 4 + len) else { continue };
        let mut raw = Vec::new();
        let ok = match kind {
            1 => flate2::read::GzDecoder::new(body).take(64 << 20).read_to_end(&mut raw).is_ok(),
            2 => flate2::read::ZlibDecoder::new(body).take(64 << 20).read_to_end(&mut raw).is_ok(),
            3 => {
                raw = body.to_vec();
                true
            }
            _ => false, // LZ4 / external .mcc: skipped
        };
        if ok
            && let Ok(t) = nbt::parse(&raw)
            && let Some(c) = chunk(&t)
        {
            out.push(c);
        }
    }
    out
}

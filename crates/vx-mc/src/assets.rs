//! Resource packs: blockstates → models → textures, read from the game's own jar or any
//! resource pack (zip or folder), layered (later packs override earlier ones).
//!
//! The textures are never shipped with voxelscape: they come from the user's own Minecraft
//! install (or a pack they point at). Without any, blocks fall back to flat colours.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::anvil::State;
use crate::zip::Zip;

pub enum Source {
    Zip(Zip),
    Dir(PathBuf),
}

impl Source {
    fn get(&self, path: &str) -> Option<Vec<u8>> {
        match self {
            Source::Zip(z) => z.read(path),
            Source::Dir(d) => std::fs::read(d.join(path)).ok(),
        }
    }
}

/// An RGBA texture (first animation frame).
pub struct Tex {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<[u8; 4]>,
}

impl Tex {
    /// Texel at (u, v) in Minecraft's 0..16 units.
    pub fn at(&self, u: f32, v: f32) -> [u8; 4] {
        let x = ((u / 16.0 * self.w as f32).floor() as isize).clamp(0, self.w as isize - 1) as usize;
        let y = ((v / 16.0 * self.h as f32).floor() as isize).clamp(0, self.h as isize - 1) as usize;
        self.rgba[y * self.w + x]
    }
}

/// Face order: down, up, north (−z), south (+z), west (−x), east (+x).
pub const FACES: [&str; 6] = ["down", "up", "north", "south", "west", "east"];

#[derive(Clone)]
pub struct Face {
    pub tex: Option<Arc<Tex>>,
    pub uv: Option<[f32; 4]>,
    pub rotation: i32,
    pub tint: bool,
}

#[derive(Clone)]
pub struct Element {
    pub from: [f32; 3],
    pub to: [f32; 3],
    /// (axis 0..3, degrees, origin)
    pub rot: Option<(usize, f32, [f32; 3])>,
    pub faces: [Option<Face>; 6],
}

/// A model placed by a blockstate: its elements and the variant's rotation (degrees).
#[derive(Clone)]
pub struct Placed {
    pub elements: Arc<Vec<Element>>,
    pub x: i32,
    pub y: i32,
}

struct RawModel {
    parent: Option<String>,
    textures: BTreeMap<String, String>,
    elements: Option<Value>,
}

pub struct Assets {
    layers: Vec<Source>,
    raw: Mutex<HashMap<String, Option<Arc<RawModel>>>>,
    tex: Mutex<HashMap<String, Option<Arc<Tex>>>>,
    pub label: String,
}

fn decode_png(b: &[u8]) -> Option<Tex> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(b));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let ch = info.color_type.samples();
    // Animated textures are frames stacked vertically: keep the first (square) one.
    let fh = h.min(w).max(1);
    let mut rgba = Vec::with_capacity(w * fh);
    for y in 0..fh {
        for x in 0..w {
            let p = &buf[(y * w + x) * ch..(y * w + x) * ch + ch];
            rgba.push(match ch {
                1 => [p[0], p[0], p[0], 255],
                2 => [p[0], p[0], p[0], p[1]],
                3 => [p[0], p[1], p[2], 255],
                _ => [p[0], p[1], p[2], p[3]],
            });
        }
    }
    Some(Tex { w, h: fh, rgba })
}

fn resource(id: &str, kind: &str, ext: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("assets/{ns}/{kind}/{path}{ext}")
}

impl Assets {
    pub fn none() -> Assets {
        Assets { layers: Vec::new(), raw: Default::default(), tex: Default::default(), label: "no textures".into() }
    }

    /// Layer a jar, a resource pack zip, or an unpacked pack folder on top.
    pub fn add(&mut self, path: &Path) -> Result<(), String> {
        let src = if path.is_dir() {
            Source::Dir(path.to_path_buf())
        } else {
            Source::Zip(Zip::new(std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)?)
        };
        self.add_source(src, &path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        Ok(())
    }

    pub fn add_source(&mut self, src: Source, label: &str) {
        self.layers.push(src);
        self.label = if self.layers.len() == 1 { label.to_string() } else { format!("{} + {label}", self.label) };
    }

    pub fn has_textures(&self) -> bool {
        !self.layers.is_empty()
    }

    /// Find the newest installed Minecraft client jar (the user's own copy).
    pub fn find_installed() -> Option<PathBuf> {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)?;
        let mut roots =
            vec![home.join("Library/Application Support/minecraft/versions"), home.join(".minecraft/versions")];
        if let Some(a) = std::env::var_os("APPDATA") {
            roots.push(PathBuf::from(a).join(".minecraft/versions"));
        }
        let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
        for r in roots {
            for v in std::fs::read_dir(r).into_iter().flatten().flatten() {
                let jar = v.path().join(format!("{}.jar", v.file_name().to_string_lossy()));
                if let Ok(m) = jar.metadata().and_then(|m| m.modified())
                    && best.as_ref().is_none_or(|b| m > b.0)
                {
                    best = Some((m, jar));
                }
            }
        }
        best.map(|b| b.1)
    }

    fn get(&self, path: &str) -> Option<Vec<u8>> {
        self.layers.iter().rev().find_map(|l| l.get(path))
    }

    pub fn texture(&self, id: &str) -> Option<Arc<Tex>> {
        if let Some(t) = self.tex.lock().unwrap_or_else(|e| e.into_inner()).get(id) {
            return t.clone();
        }
        let t = self.get(&resource(id, "textures", ".png")).and_then(|b| decode_png(&b)).map(Arc::new);
        self.tex.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), t.clone());
        t
    }

    fn raw_model(&self, id: &str) -> Option<Arc<RawModel>> {
        if let Some(m) = self.raw.lock().unwrap_or_else(|e| e.into_inner()).get(id) {
            return m.clone();
        }
        let m = self.get(&resource(id, "models", ".json")).and_then(|b| serde_json::from_slice::<Value>(&b).ok()).map(
            |v| {
                Arc::new(RawModel {
                    parent: v["parent"].as_str().map(str::to_string),
                    textures: v["textures"]
                        .as_object()
                        // A texture is a string, or (newer packs) { "sprite": "...", ... }.
                        .map(|o| {
                            o.iter()
                                .filter_map(|(k, v)| {
                                    Some((k.clone(), v.as_str().or_else(|| v["sprite"].as_str())?.to_string()))
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    elements: v.get("elements").cloned(),
                })
            },
        );
        self.raw.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), m.clone());
        m
    }

    /// A model with its parents resolved: elements and their face textures.
    pub fn model(&self, id: &str) -> Option<Vec<Element>> {
        let mut textures: BTreeMap<String, String> = BTreeMap::new();
        let mut elements: Option<Value> = None;
        let mut cur = Some(id.to_string());
        let mut depth = 0;
        while let Some(m) = cur.take().and_then(|i| self.raw_model(&i)) {
            for (k, v) in &m.textures {
                textures.entry(k.clone()).or_insert_with(|| v.clone());
            }
            if elements.is_none() {
                elements = m.elements.clone();
            }
            depth += 1;
            if depth > 16 {
                break;
            }
            cur = m.parent.clone();
        }
        let resolve = |mut t: String| -> Option<String> {
            for _ in 0..8 {
                match t.strip_prefix('#') {
                    Some(var) => t = textures.get(var)?.clone(),
                    None => return Some(t),
                }
            }
            None
        };
        let els = elements?;
        let f3 = |v: &Value| -> Option<[f32; 3]> {
            let a = v.as_array()?;
            Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32])
        };
        let mut out = Vec::new();
        for e in els.as_array()? {
            let (Some(from), Some(to)) = (f3(&e["from"]), f3(&e["to"])) else { continue };
            let rot = e.get("rotation").and_then(|r| {
                let axis = match r["axis"].as_str()? {
                    "x" => 0,
                    "y" => 1,
                    _ => 2,
                };
                Some((axis, r["angle"].as_f64()? as f32, f3(&r["origin"]).unwrap_or([8.0; 3])))
            });
            let mut faces: [Option<Face>; 6] = Default::default();
            for (i, name) in FACES.iter().enumerate() {
                let Some(f) = e["faces"].get(*name) else { continue };
                let tex = f["texture"].as_str().map(str::to_string).and_then(resolve).and_then(|t| self.texture(&t));
                let uv = f["uv"].as_array().and_then(|a| {
                    Some([
                        a.first()?.as_f64()? as f32,
                        a.get(1)?.as_f64()? as f32,
                        a.get(2)?.as_f64()? as f32,
                        a.get(3)?.as_f64()? as f32,
                    ])
                });
                faces[i] = Some(Face {
                    tex,
                    uv,
                    rotation: f["rotation"].as_i64().unwrap_or(0) as i32,
                    tint: f.get("tintindex").is_some(),
                });
            }
            out.push(Element { from, to, rot, faces });
        }
        Some(out)
    }

    /// The models a block state shows (variants or multipart), already resolved.
    pub fn block(&self, s: &State) -> Vec<Placed> {
        let Some(bs) =
            self.get(&resource(&s.name, "blockstates", ".json")).and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        else {
            return Vec::new();
        };
        let matches = |cond: &str| -> bool {
            cond.is_empty()
                || cond.split(',').all(|kv| match kv.split_once('=') {
                    // A property the state doesn't carry matches anything (partial states
                    // from other games / tools pick the first fitting variant).
                    Some((k, v)) => s.prop(k).is_none_or(|x| x == v),
                    None => true,
                })
        };
        let mut picks: Vec<&Value> = Vec::new();
        if let Some(vars) = bs["variants"].as_object() {
            if let Some((_, v)) = vars.iter().find(|(k, _)| matches(k)) {
                picks.push(v);
            }
        } else if let Some(parts) = bs["multipart"].as_array() {
            for p in parts {
                if when(&p["when"], s) {
                    picks.push(&p["apply"]);
                }
            }
        }
        picks
            .into_iter()
            .filter_map(|v| {
                let v = if v.is_array() { v.get(0)? } else { v };
                let els = self.model(v["model"].as_str()?)?;
                Some(Placed {
                    elements: Arc::new(els),
                    x: v["x"].as_i64().unwrap_or(0) as i32,
                    y: v["y"].as_i64().unwrap_or(0) as i32,
                })
            })
            .collect()
    }
}

/// A multipart `when` condition.
fn when(w: &Value, s: &State) -> bool {
    match w {
        Value::Null => true,
        Value::Object(o) => {
            if let Some(any) = o.get("OR").and_then(Value::as_array) {
                return any.iter().any(|c| when(c, s));
            }
            if let Some(all) = o.get("AND").and_then(Value::as_array) {
                return all.iter().all(|c| when(c, s));
            }
            o.iter().all(|(k, v)| {
                let want = match v {
                    Value::String(x) => x.clone(),
                    other => other.to_string(),
                };
                want.split('|').any(|alt| s.prop(k) == Some(alt))
            })
        }
        _ => true,
    }
}

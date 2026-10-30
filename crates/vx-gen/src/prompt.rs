//! Prompt → SceneSpec without a language model.
//!
//! The world explorer asks a local model first (MLX on Mac, Ollama / llama.cpp on Windows and
//! Linux). When none is running — or you just don't want one — this keyword parser produces a
//! sensible spec from the same prompt, so every other part of the engine works model-free.
//! It's also the safety net if a model returns something unusable.

use crate::spec::{Landscape, SceneSpec, Style};

fn has(p: &str, words: &[&str]) -> bool {
    words.iter().any(|w| p.contains(w))
}

pub fn spec_from_prompt(prompt: &str) -> SceneSpec {
    let p = prompt.to_lowercase();
    let mut s = SceneSpec { title: prompt.trim().chars().take(80).collect(), ..Default::default() };
    // Seed: stable per prompt (same prompt → same city).
    s.seed = p.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));

    s.style = if has(&p, &["cyberpunk", "neon", "blade runner", "dystopia", "synthwave"]) {
        Style::Cyberpunk
    } else if has(&p, &["medieval", "castle", "fantasy", "village", "tudor", "old town", "knight"]) {
        Style::Medieval
    } else if has(&p, &["desert", "sand", "dune", "arab", "marrakech", "mesa", "egypt"]) {
        Style::Desert
    } else if has(&p, &["ruin", "abandoned", "post-apoc", "apocalyp", "overgrown", "war", "destroyed"]) {
        Style::Ruins
    } else if has(&p, &["future", "sci-fi", "scifi", "utopia", "solarpunk", "space"]) {
        Style::Futuristic
    } else if has(&p, &["tropical", "beach", "island", "miami", "caribbean", "resort"]) {
        Style::Tropical
    } else if has(&p, &["snow", "winter", "arctic", "frozen", "christmas", "alpine"]) {
        Style::Snow
    } else if has(&p, &["brick", "brooklyn", "london", "boston", "industrial", "victorian", "1920"]) {
        Style::Brick
    } else {
        Style::Modern
    };

    if has(&p, &["huge", "massive", "megacity", "metropolis", "sprawling", "enormous"]) {
        s.size_m = 1536.0;
    } else if has(&p, &["small", "tiny", "village", "hamlet", "town"]) {
        s.size_m = 384.0;
    }
    if has(&p, &["skyscraper", "tower", "metropolis", "megacity", "manhattan", "downtown", "dense"]) {
        s.height_max_m = 260.0;
        s.density = 0.85;
        s.downtown = 0.8;
    }
    if has(&p, &["low-rise", "suburb", "village", "town", "cottage"]) {
        s.height_max_m = 16.0;
        s.density = 0.55;
    }
    if has(&p, &["manhattan", "grid"]) {
        s.grid = 1.0;
    }
    if has(&p, &["organic", "winding", "maze", "old town"]) {
        s.grid = 0.2;
    }
    if has(&p, &["river", "canal", "harbor", "harbour", "lake", "venice", "waterfront", "bay"]) {
        s.water = 0.45;
    }
    if has(&p, &["dry", "no water", "desert"]) {
        s.water = 0.0;
    }
    if has(&p, &["park", "green", "garden", "forest", "trees", "solarpunk", "overgrown"]) {
        s.parks = 0.35;
        s.trees = 0.95;
    }
    if has(&p, &["plaza", "square", "market"]) {
        s.plazas = 0.25;
    }
    if has(&p, &["busy", "cluttered", "messy", "junk", "props", "garry", "gmod", "sandbox"]) {
        s.props = 1.0;
        s.extra_props = vec![
            "barrel_explosive".into(),
            "crate_wood".into(),
            "watermelon".into(),
            "traffic_cone".into(),
            "tire".into(),
        ];
    }
    if has(&p, &["empty", "clean", "minimal", "sparse"]) {
        s.props = 0.2;
    }
    // Time of day / weather.
    s.time_of_day = if has(&p, &["night", "midnight", "neon"]) {
        22.5
    } else if has(&p, &["sunset", "dusk", "golden hour", "evening"]) {
        18.6
    } else if has(&p, &["sunrise", "dawn", "morning"]) {
        6.6
    } else if has(&p, &["noon", "midday"]) {
        12.5
    } else {
        15.5
    };
    if s.style == Style::Cyberpunk && !has(&p, &["day", "noon", "morning"]) {
        s.time_of_day = 22.5;
        s.lit_windows = 0.7;
    }
    if has(&p, &["fog", "mist", "haze", "smog", "rain", "storm"]) {
        s.fog = 0.85;
    }
    if has(&p, &["clear", "crisp"]) {
        s.fog = 0.1;
    }
    // The land.
    s.landscape = if has(&p, &["minecraft", "blocky", "block world", "voxel terrain", "cubes"]) {
        Landscape::Blocky
    } else if has(&p, &["island", "archipelago", "atoll"]) {
        Landscape::Island
    } else if has(&p, &["mountain", "alps", "alpine", "peak", "himalaya", "rockies", "valley"]) {
        Landscape::Mountains
    } else if has(&p, &["canyon", "gorge", "ravine"]) {
        Landscape::Canyons
    } else if has(&p, &["mesa", "badlands", "plateau", "monument valley"]) {
        Landscape::Mesas
    } else if has(
        &p,
        &[
            "hill",
            "countryside",
            "rolling",
            "meadow",
            "forest",
            "woods",
            "nature",
            "wilderness",
            "landscape",
            "terrain",
        ],
    ) {
        Landscape::Hills
    } else {
        Landscape::Flat
    };
    let wild = has(
        &p,
        &[
            "wilderness",
            "untouched",
            "no city",
            "no buildings",
            "nature only",
            "empty",
            "landscape",
            "terrain",
            "forest",
            "woods",
        ],
    );
    let urban = has(&p, &["city", "town", "village", "street", "downtown", "buildings", "metropolis", "settlement"]);
    s.city = s.landscape == Landscape::Flat || urban || !wild;
    if s.landscape == Landscape::Blocky && !urban {
        s.city = false;
    }
    if has(&p, &["forest", "woods", "jungle", "taiga", "trees"]) {
        s.forest = 0.9;
    }
    if has(&p, &["eroded", "rivers", "gullies", "weathered"]) {
        s.erosion = 0.9;
    }
    if has(&p, &["huge mountains", "towering", "epic", "dramatic", "jagged"]) {
        s.relief = 0.9;
    }
    s.sanitized()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_map_to_sensible_specs() {
        let a = spec_from_prompt("A neon cyberpunk megacity at night in the rain");
        assert_eq!(a.style, Style::Cyberpunk);
        assert!(a.size_m > 1000.0 && a.time_of_day > 20.0 && a.fog > 0.5);
        let b = spec_from_prompt("small medieval village by a river at sunset");
        assert_eq!(b.style, Style::Medieval);
        assert!(b.water > 0.3 && b.height_max_m <= 20.0 && (17.0..20.0).contains(&b.time_of_day));
        assert_eq!(spec_from_prompt("x").seed, spec_from_prompt("x").seed);
        assert_ne!(spec_from_prompt("x").seed, spec_from_prompt("y").seed);
    }
}

/// The scene-writing guide given to language models (local, Claude Code, or an MCP client):
/// what a `SceneSpec` is, every field and its range, and examples. Ask for JSON only.
pub fn guide() -> String {
    let styles: Vec<&str> = Style::ALL.iter().map(|s| s.name()).collect();
    let lands: Vec<&str> = Landscape::ALL.iter().map(|l| l.name()).collect();
    let props: Vec<&str> = crate::props::CATALOG.iter().map(|p| p.name).collect();
    format!(
        r##"You design scenes for voxelscape, a procedural voxel world generator. Turn the user's request into ONE JSON object (a SceneSpec). Output only the JSON, no prose, no code fences.

Fields (all optional; anything missing gets a sensible default; values are clamped):
- "title": short name for the scene (string)
- "seed": integer; change it for a different layout of the same idea
- "landscape": one of {lands:?}. "flat" = only a city on flat ground; the others generate eroded natural terrain ("blocky" = Minecraft-style blocks)
- "city": true/false: build a city (on a plateau when the landscape isn't flat). Use false for pure nature
- "style": one of {styles:?} (architecture and materials)
- "size_m": 256..2048, side of the city (or of the terrain when city=false), metres
- "density": 0..1 how built-up; "downtown": 0..1 how strongly towers cluster in the centre
- "height_min_m", "height_max_m": building heights in metres (4..480)
- "parks", "plazas", "water", "grid", "props", "trees": 0..1 (grid 1 = Manhattan, 0 = organic streets)
- "relief": 0..1 terrain height; "erosion": 0..1 rivers and gullies; "forest": 0..1 tree cover on terrain
- "time_of_day": 0..24 hours (sun position; at night windows and lamps light up)
- "fog": 0..1 haze; "lit_windows": 0..1 share of lit windows at night
- "wall_colors": up to 8 "#rrggbb" facade colours; "accent_color": "#rrggbb"
- "extra_props": prop names to scatter, from: {props:?}

Guidance: match the mood (time_of_day, fog, style) and the scale (size_m, heights) to the request. Small towns: size_m 256-512, height_max_m 12-30. Big cities: 1024-2048 with tall towers. Nature scenes: city false, pick a landscape, raise forest, erosion and relief as described.

Example: "a rainy neon megacity at night" -> {{"title":"Neon Megacity","landscape":"flat","city":true,"style":"cyberpunk","size_m":1536,"density":0.9,"downtown":0.9,"height_min_m":20,"height_max_m":400,"time_of_day":22.5,"fog":0.7,"lit_windows":0.8,"water":0.15,"grid":0.85}}
Example: "a quiet alpine village in the mountains at sunrise" -> {{"title":"Alpine Village","landscape":"mountains","city":true,"style":"snow","size_m":384,"density":0.5,"height_min_m":6,"height_max_m":14,"relief":0.8,"erosion":0.8,"forest":0.7,"time_of_day":6.8,"fog":0.35}}
Example: "untouched eroded canyon wilderness at golden hour" -> {{"title":"Canyon Wilds","landscape":"canyons","city":false,"size_m":2048,"relief":0.7,"erosion":0.9,"forest":0.3,"time_of_day":18.2,"fog":0.25}}"##
    )
}

/// Merge a model's JSON reply onto a base spec (the keyword parser's spec for the same prompt,
/// or the current scene when refining): fields the model set win, everything else keeps the
/// base. Tolerates prose, code fences and `<think>` blocks around the object, and ignores
/// unknown or badly typed fields.
pub fn merge_reply(prompt: &str, reply: &str, base: Option<&SceneSpec>) -> Result<SceneSpec, String> {
    let start = match base {
        Some(b) => b.clone(),
        None => spec_from_prompt(prompt),
    };
    let text = match reply.rfind("</think>") {
        Some(e) => &reply[e + 8..],
        None => reply,
    };
    let obj = extract_object(text).ok_or("no JSON object in the reply")?;
    let patch: serde_json::Value = serde_json::from_str(obj).map_err(|e| e.to_string())?;
    let serde_json::Value::Object(patch) = patch else { return Err("reply is not an object".into()) };
    let serde_json::Value::Object(mut fields) = serde_json::to_value(&start).map_err(|e| e.to_string())? else {
        return Err("spec is not an object".into());
    };
    for (k, val) in patch {
        if !fields.contains_key(&k) {
            continue;
        }
        let mut trial = fields.clone();
        trial.insert(k.clone(), val.clone());
        if serde_json::from_value::<SceneSpec>(serde_json::Value::Object(trial)).is_ok() {
            fields.insert(k, val);
        }
    }
    serde_json::from_value::<SceneSpec>(serde_json::Value::Object(fields))
        .map(|s| s.sanitized())
        .map_err(|e| e.to_string())
}

/// The first balanced `{...}` in `s` (aware of strings).
fn extract_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let (mut depth, mut in_str, mut esc) = (0i32, false, false);
    for (i, c) in s[start..].char_indices() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod harness_tests {
    use super::*;

    #[test]
    fn replies_merge_onto_prompt_defaults() {
        let r = "<think>hmm {not json}</think>Sure! ```json\n{\"style\": \"snow\", \"landscape\": \"mountains\", \"height_max_m\": \"tall\", \"title\": \"A {braced} \\\"name\\\"\", \"bogus\": 1}\n```";
        let s = merge_reply("a village", r, None).unwrap();
        assert_eq!(s.style, Style::Snow);
        assert_eq!(s.landscape, Landscape::Mountains);
        assert_eq!(s.title, "A {braced} \"name\"");
        assert_eq!(s.height_max_m, spec_from_prompt("a village").height_max_m, "badly typed field ignored");
        assert!(merge_reply("x", "no json here", None).is_err());
        assert!(guide().contains("crate_wood"));
    }

    #[test]
    fn nature_prompts_skip_the_city() {
        let s = spec_from_prompt("an untouched eroded mountain wilderness with forests");
        assert_eq!(s.landscape, Landscape::Mountains);
        assert!(!s.city);
        let s = spec_from_prompt("a medieval town in the hills");
        assert_eq!(s.landscape, Landscape::Hills);
        assert!(s.city);
    }
}

//! Prompt → SceneSpec without a language model.
//!
//! The world explorer asks a local model first (MLX on Mac, Ollama / llama.cpp on Windows and
//! Linux). When none is running — or you just don't want one — this keyword parser produces a
//! sensible spec from the same prompt, so every other part of the engine works model-free.
//! It's also the safety net if a model returns something unusable.

use crate::spec::{SceneSpec, Style};

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

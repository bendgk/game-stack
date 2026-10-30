//! `SceneSpec`: everything a scene is generated from. The world explorer's local model turns a
//! prompt into one of these (JSON); the generator turns it into voxels, deterministically.
//! Unknown / missing fields fall back to defaults and every value is clamped, so a model can't
//! produce an invalid scene.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    #[default]
    Modern,
    Brick,
    Cyberpunk,
    Medieval,
    Desert,
    Ruins,
    Futuristic,
    Tropical,
    Snow,
}

impl Style {
    pub const ALL: [Style; 9] = [
        Style::Modern,
        Style::Brick,
        Style::Cyberpunk,
        Style::Medieval,
        Style::Desert,
        Style::Ruins,
        Style::Futuristic,
        Style::Tropical,
        Style::Snow,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Style::Modern => "modern",
            Style::Brick => "brick",
            Style::Cyberpunk => "cyberpunk",
            Style::Medieval => "medieval",
            Style::Desert => "desert",
            Style::Ruins => "ruins",
            Style::Futuristic => "futuristic",
            Style::Tropical => "tropical",
            Style::Snow => "snow",
        }
    }
}

/// The land under (or instead of) the city.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Landscape {
    /// Flat ground: just the city.
    #[default]
    Flat,
    Hills,
    Mountains,
    Island,
    Canyons,
    Mesas,
    /// Minecraft-style block terrain.
    Blocky,
}

impl Landscape {
    pub const ALL: [Landscape; 7] = [
        Landscape::Flat,
        Landscape::Hills,
        Landscape::Mountains,
        Landscape::Island,
        Landscape::Canyons,
        Landscape::Mesas,
        Landscape::Blocky,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Landscape::Flat => "flat",
            Landscape::Hills => "hills",
            Landscape::Mountains => "mountains",
            Landscape::Island => "island",
            Landscape::Canyons => "canyons",
            Landscape::Mesas => "mesas",
            Landscape::Blocky => "blocky",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneSpec {
    pub title: String,
    /// The land: `flat` (city only) or a generated, eroded terrain around / instead of it.
    pub landscape: Landscape,
    /// Build a city (on a plateau when the landscape isn't flat).
    pub city: bool,
    /// 0..1: terrain height, relative to the landscape's natural relief.
    pub relief: f32,
    /// 0..1: hydraulic erosion strength (gullies, river valleys, sediment fans).
    pub erosion: f32,
    /// 0..1: forest cover on natural terrain.
    pub forest: f32,
    pub seed: u64,
    pub style: Style,
    /// City extent (metres, square). 256–2048.
    pub size_m: f32,
    /// 0..1: how much of the land is built on (vs parks/plazas).
    pub density: f32,
    /// Building heights (metres).
    pub height_min_m: f32,
    pub height_max_m: f32,
    /// 0..1: how strongly tall buildings cluster downtown (0 = uniform).
    pub downtown: f32,
    /// 0..1 shares of the non-road land.
    pub parks: f32,
    pub plazas: f32,
    /// 0..1: how much water (rivers / lakes / harbour).
    pub water: f32,
    /// 0..1: how grid-like the streets are (1 = Manhattan, 0 = organic).
    pub grid: f32,
    /// 0..1: street furniture / props / cars.
    pub props: f32,
    /// 0..1: trees along streets and in parks.
    pub trees: f32,
    /// Hours 0..24 (sun position; night lights the windows).
    pub time_of_day: f32,
    /// 0..1 atmosphere haze.
    pub fog: f32,
    /// 0..1 share of windows lit at night.
    pub lit_windows: f32,
    /// Optional colour overrides (#rrggbb): facade colours and an accent colour.
    pub wall_colors: Vec<String>,
    pub accent_color: Option<String>,
    /// Optional extra props to scatter (names from the prop catalog).
    pub extra_props: Vec<String>,
}

impl Default for SceneSpec {
    fn default() -> Self {
        SceneSpec {
            title: "Untitled city".into(),
            landscape: Landscape::Flat,
            city: true,
            relief: 0.5,
            erosion: 0.6,
            forest: 0.5,
            seed: 1,
            style: Style::Modern,
            size_m: 768.0,
            density: 0.7,
            height_min_m: 8.0,
            height_max_m: 120.0,
            downtown: 0.6,
            parks: 0.12,
            plazas: 0.06,
            water: 0.1,
            grid: 0.8,
            props: 0.6,
            trees: 0.6,
            time_of_day: 16.5,
            fog: 0.3,
            lit_windows: 0.45,
            wall_colors: vec![],
            accent_color: None,
            extra_props: vec![],
        }
    }
}

fn c01(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.5 }
}

impl SceneSpec {
    /// Clamp everything into the generator's valid range.
    pub fn sanitized(mut self) -> Self {
        self.size_m = if self.size_m.is_finite() { self.size_m.clamp(256.0, 2048.0) } else { 768.0 };
        for v in [
            &mut self.density,
            &mut self.downtown,
            &mut self.parks,
            &mut self.plazas,
            &mut self.water,
            &mut self.grid,
            &mut self.props,
            &mut self.trees,
            &mut self.fog,
            &mut self.lit_windows,
            &mut self.relief,
            &mut self.erosion,
            &mut self.forest,
        ] {
            *v = c01(*v);
        }
        self.height_min_m = if self.height_min_m.is_finite() { self.height_min_m.clamp(4.0, 200.0) } else { 8.0 };
        self.height_max_m =
            if self.height_max_m.is_finite() { self.height_max_m.clamp(self.height_min_m, 480.0) } else { 120.0 };
        self.time_of_day = if self.time_of_day.is_finite() { self.time_of_day.rem_euclid(24.0) } else { 16.5 };
        self.title.truncate(80);
        self.wall_colors.retain(|c| parse_hex(c).is_some());
        self.wall_colors.truncate(8);
        if self.accent_color.as_deref().and_then(parse_hex).is_none() {
            self.accent_color = None;
        }
        self.extra_props.truncate(16);
        if self.landscape == Landscape::Flat {
            self.city = true; // a flat empty world has nothing in it
        }
        self
    }

    /// Parse JSON leniently (missing fields default; values clamped).
    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str::<SceneSpec>(s).map(|s| s.sanitized()).map_err(|e| e.to_string())
    }

    /// Sun elevation/azimuth (radians) for the time of day.
    pub fn sun(&self) -> (f32, f32) {
        let day = (self.time_of_day - 6.0) / 12.0; // 0 at 6:00, 1 at 18:00
        let elev = (day * std::f32::consts::PI).sin() * 1.15 - 0.1;
        let az = 0.6 + day * 2.4;
        (elev, az)
    }
}

pub fn parse_hex(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    if h.len() == 6 { u32::from_str_radix(h, 16).ok() } else { None }
}

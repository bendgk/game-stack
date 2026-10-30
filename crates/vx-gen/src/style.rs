//! Materials and per-style art direction.

use vx_core::{Mat, Material, Palette};

use crate::spec::{SceneSpec, Style, parse_hex};

/// Every material the generators use, by role.
#[derive(Clone, Debug)]
pub struct Mats {
    pub bedrock: Mat,
    pub concrete: Mat,
    pub concrete_dark: Mat,
    pub asphalt: Mat,
    pub asphalt_dark: Mat,
    pub line_white: Mat,
    pub line_yellow: Mat,
    pub sidewalk: Mat,
    pub sidewalk_joint: Mat,
    pub curb: Mat,
    pub grass: Mat,
    pub grass_light: Mat,
    pub dirt: Mat,
    pub flowers: [Mat; 3],
    pub water: Mat,
    pub plaza: Mat,
    pub plaza_alt: Mat,
    pub sand: Mat,
    pub snow: Mat,
    pub glass: Mat,
    pub glass_dark: Mat,
    pub glass_lit: Mat,
    pub glass_lit_cool: Mat,
    pub steel: Mat,
    pub steel_dark: Mat,
    pub chrome: Mat,
    pub wood: Mat,
    pub wood_dark: Mat,
    pub rubber: Mat,
    pub lamp: Mat,
    pub lamp_red: Mat,
    pub lamp_green: Mat,
    pub neon: [Mat; 4],
    pub leaf: [Mat; 3],
    pub bark: Mat,
    pub stone: Mat,
    pub roof: Mat,
    pub roof_tile: Mat,
    pub walls: Vec<Mat>,
    pub trim: Mat,
    pub accent: Mat,
    /// Generic prop paints.
    pub paint: [Mat; 8],
    pub plastic_white: Mat,
    pub plastic_black: Mat,
    pub fabric: [Mat; 3],
    pub cardboard: Mat,
    pub hazard: Mat,
    pub red_barrel: Mat,
    pub screen: Mat,
    pub melon: Mat,
    pub melon_stripe: Mat,
}

fn m(name: &str, color: u32, rough: f32) -> Material {
    Material::new(name, color, rough)
}

pub fn materials(spec: &SceneSpec, pal: &mut Palette) -> Mats {
    let s = spec.style;
    let mut add = |mat: Material| pal.add(mat);
    let (wall_set, trim, accent, roof, roof_tile): (Vec<u32>, u32, u32, u32, u32) = match s {
        Style::Modern => {
            (vec![0xd8d4cc, 0xbfc3c6, 0x8f969c, 0xe9e4da, 0x6f7479], 0x3b3f44, 0x2f8fd8, 0x55595e, 0x6a6e73)
        }
        Style::Brick => {
            (vec![0x9c4a32, 0x8a3f2c, 0xa8573a, 0x7b3b2b, 0xb2694a], 0xd9d1c3, 0x2e5d3a, 0x4a4747, 0x6b3a2e)
        }
        Style::Cyberpunk => {
            (vec![0x2c2f3a, 0x3a3346, 0x23262e, 0x40404a, 0x2a3540], 0x15171c, 0xff2fa8, 0x1c1e24, 0x2a2c33)
        }
        Style::Medieval => {
            (vec![0xd9cfb4, 0xcfc2a2, 0xe6dcc4, 0xb8a888, 0xc9b896], 0x5a3b22, 0x8a2f24, 0x7a3b2a, 0x8c4a32)
        }
        Style::Desert => {
            (vec![0xd9b98c, 0xcfa878, 0xe3c79e, 0xc49a6c, 0xbf8e5f], 0xa87a50, 0x2a7fa0, 0xc9a377, 0xb88d5f)
        }
        Style::Ruins => {
            (vec![0x8a8778, 0x7a786c, 0x9a9686, 0x6d6b60, 0x858170], 0x5c5a50, 0x4f6a3a, 0x5a5850, 0x6a675c)
        }
        Style::Futuristic => {
            (vec![0xf2f4f7, 0xe3e8ee, 0xcdd6df, 0xffffff, 0xb8c4d0], 0x9aa8b6, 0x34e0ff, 0xdfe5ea, 0xc8d0d8)
        }
        Style::Tropical => {
            (vec![0xf3e6c8, 0xf6c9a8, 0xa8dcd1, 0xf2b5b0, 0xfbe7a1], 0xffffff, 0x1f9e8f, 0xc9573a, 0xd06a44)
        }
        Style::Snow => (vec![0x7a8a96, 0x8f7f6f, 0x6b5e52, 0xa69888, 0x55606a], 0xe8eef2, 0xc23b2f, 0xf2f6f8, 0x4a4f55),
    };
    let wall_colors: Vec<u32> = if spec.wall_colors.is_empty() {
        wall_set
    } else {
        spec.wall_colors.iter().filter_map(|c| parse_hex(c)).collect()
    };
    let accent = spec.accent_color.as_deref().and_then(parse_hex).unwrap_or(accent);
    let wall_rough = if s == Style::Futuristic { 0.35 } else { 0.85 };
    let walls: Vec<Mat> = wall_colors
        .iter()
        .enumerate()
        .map(|(i, c)| add(m(&format!("wall{i}"), *c, wall_rough).grain(if s == Style::Brick { 0.16 } else { 0.07 })))
        .collect();
    let (grass_c, grass_l) = match s {
        Style::Desert => (0x8a8a4a, 0xa59a5a),
        Style::Snow => (0xe8eef2, 0xffffff),
        Style::Tropical => (0x3f8f2e, 0x5fae3a),
        Style::Ruins => (0x55703a, 0x6d8a45),
        _ => (0x4f7d2e, 0x67953a),
    };
    Mats {
        bedrock: add(m("bedrock", 0x55524c, 0.95)),
        concrete: add(m("concrete", 0xa09d95, 0.85).grain(0.08)),
        concrete_dark: add(m("concrete_dark", 0x6a6862, 0.9).grain(0.08)),
        asphalt: add(m(
            "asphalt",
            if s == Style::Cyberpunk { 0x26272b } else { 0x3a3b3e },
            if s == Style::Cyberpunk { 0.35 } else { 0.9 },
        )
        .grain(0.14)),
        asphalt_dark: add(m("asphalt_dark", 0x2c2d30, 0.9).grain(0.1)),
        line_white: add(m("line_white", 0xe8e6df, 0.7).grain(0.05)),
        line_yellow: add(m("line_yellow", 0xe0b81a, 0.7).grain(0.05)),
        sidewalk: add(m("sidewalk", if s == Style::Medieval { 0x8f8a80 } else { 0xb8b4ab }, 0.85).grain(0.1)),
        sidewalk_joint: add(m("sidewalk_joint", 0x7d7a73, 0.9)),
        curb: add(m("curb", 0xc9c5bb, 0.8).grain(0.06)),
        grass: add(m("grass", grass_c, 0.95).foliage().grain(0.28)),
        grass_light: add(m("grass_light", grass_l, 0.95).foliage().grain(0.25)),
        dirt: add(m("dirt", 0x6b5236, 0.95).grain(0.2)),
        flowers: [
            add(m("flower_red", 0xd23a3a, 0.8).foliage()),
            add(m("flower_yellow", 0xf2d03a, 0.8).foliage()),
            add(m("flower_violet", 0x9a5ad8, 0.8).foliage()),
        ],
        water: add(m("water", 0x1d4a5c, 0.02).water().grain(0.0)),
        plaza: add(m("plaza", if s == Style::Desert { 0xd2b48a } else { 0xb3aca0 }, 0.75).grain(0.1)),
        plaza_alt: add(m("plaza_alt", 0x8f877b, 0.75).grain(0.1)),
        sand: add(m("sand", 0xd8c08f, 0.95).grain(0.15)),
        snow: add(m("snow", 0xf4f8fb, 0.6).grain(0.04)),
        glass: add(m("glass", if s == Style::Futuristic { 0x9fd8ee } else { 0x8fb4c8 }, 0.04).glass().grain(0.0)),
        glass_dark: add(m("glass_dark", 0x23323d, 0.04).glass().grain(0.0)),
        glass_lit: add(m("glass_lit", 0xffd9a0, 0.1).glass().emit(3.5).grain(0.0)),
        glass_lit_cool: add(m("glass_lit_cool", 0xcfe6ff, 0.1).glass().emit(3.0).grain(0.0)),
        steel: add(m("steel", 0x8a9099, 0.35).metal(1.0).grain(0.03)),
        steel_dark: add(m("steel_dark", 0x3a3d42, 0.45).metal(1.0).grain(0.03)),
        chrome: add(m("chrome", 0xd6dade, 0.12).metal(1.0).grain(0.0)),
        wood: add(m("wood", 0x8a6238, 0.8).grain(0.18)),
        wood_dark: add(m("wood_dark", 0x5e4024, 0.8).grain(0.15)),
        rubber: add(m("rubber", 0x1c1c1e, 0.9).grain(0.04)),
        lamp: add(m("lamp", 0xffd9a0, 0.4).emit(24.0).grain(0.0)),
        lamp_red: add(m("lamp_red", 0xff3020, 0.4).emit(14.0).grain(0.0)),
        lamp_green: add(m("lamp_green", 0x30ff70, 0.4).emit(14.0).grain(0.0)),
        neon: [
            add(m("neon_pink", 0xff2fa8, 0.3).emit(18.0).grain(0.0)),
            add(m("neon_cyan", 0x2fe8ff, 0.3).emit(18.0).grain(0.0)),
            add(m("neon_violet", 0x9a4dff, 0.3).emit(18.0).grain(0.0)),
            add(m("neon_amber", 0xffb02f, 0.3).emit(18.0).grain(0.0)),
        ],
        leaf: [
            add(m("leaf", if s == Style::Snow { 0x2f4a3a } else { 0x3f6e2a }, 0.9).foliage().grain(0.3)),
            add(m("leaf_light", if s == Style::Desert { 0x7a8a3a } else { 0x5a8a35 }, 0.9).foliage().grain(0.3)),
            add(m("leaf_dark", 0x2c5220, 0.9).foliage().grain(0.3)),
        ],
        bark: add(m("bark", 0x5a4430, 0.95).grain(0.2)),
        stone: add(m("stone", 0x8c877c, 0.9).grain(0.18)),
        roof: add(m("roof", roof, 0.85).grain(0.1)),
        roof_tile: add(m("roof_tile", roof_tile, 0.8).grain(0.16)),
        trim: add(m("trim", trim, 0.6).grain(0.05)),
        accent: add(m("accent", accent, 0.5).grain(0.05)),
        walls,
        paint: [
            add(m("paint_red", 0xc0302a, 0.45).grain(0.03)),
            add(m("paint_blue", 0x2a5ac0, 0.45).grain(0.03)),
            add(m("paint_yellow", 0xe8c020, 0.45).grain(0.03)),
            add(m("paint_green", 0x2f8a3a, 0.45).grain(0.03)),
            add(m("paint_white", 0xe8e8e4, 0.45).grain(0.03)),
            add(m("paint_black", 0x1e1f22, 0.45).grain(0.03)),
            add(m("paint_orange", 0xf06a10, 0.45).grain(0.03)),
            add(m("paint_silver", 0xa9adb3, 0.3).metal(0.8).grain(0.02)),
        ],
        plastic_white: add(m("plastic_white", 0xeeeeea, 0.5).grain(0.02)),
        plastic_black: add(m("plastic_black", 0x18181a, 0.5).grain(0.02)),
        fabric: [
            add(m("fabric_red", 0x8a2b2b, 0.95).grain(0.1)),
            add(m("fabric_blue", 0x2b4a8a, 0.95).grain(0.1)),
            add(m("fabric_grey", 0x6a6a70, 0.95).grain(0.1)),
        ],
        cardboard: add(m("cardboard", 0xb08a5a, 0.95).grain(0.12)),
        hazard: add(m("hazard", 0xe0b81a, 0.6).grain(0.05)),
        red_barrel: add(m("red_barrel", 0xb81e1e, 0.5).grain(0.05)),
        screen: add(m("screen", 0x3a8fd8, 0.1).emit(3.0).grain(0.0)),
        melon: add(m("melon", 0x2f7a2a, 0.5).grain(0.1)),
        melon_stripe: add(m("melon_stripe", 0x184a15, 0.5).grain(0.1)),
    }
}

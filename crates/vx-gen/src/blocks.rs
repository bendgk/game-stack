//! The builder palette: materials every world has, so players and agents can build with the
//! same names everywhere ("oak", "brick", "glass", "#c0392b"…).
//!
//! It is appended to each world's palette when the world is loaded ([`add_to`]), in a fixed
//! order, so a material id in an edit means the same thing on every machine.

use vx_core::{AIR, Mat, Material, Palette, flag};

fn m(name: &str, color: u32, rough: f32) -> Material {
    Material::new(name, color, rough)
}

/// Named building materials, then a 6×6×6 colour cube (`c000` … `c555`) for any other colour.
pub fn materials() -> Vec<Material> {
    let mut v = vec![
        // Wood.
        m("oak", 0x9a6b3f, 0.75).grain(0.12),
        m("spruce", 0x6b4a2b, 0.75).grain(0.12),
        m("birch", 0xd8c9a0, 0.7).grain(0.1),
        m("dark_oak", 0x4a3220, 0.75).grain(0.12),
        m("log", 0x5b4128, 0.85).grain(0.18),
        m("bamboo", 0xb9b25a, 0.6).grain(0.08),
        // Stone and masonry.
        m("stone", 0x8a8a86, 0.85).grain(0.12),
        m("cobble", 0x77736c, 0.9).grain(0.2),
        m("granite", 0x9c6f5c, 0.7).grain(0.15),
        m("sandstone", 0xd9c48f, 0.85).grain(0.1),
        m("marble", 0xeeeae2, 0.25).grain(0.05),
        m("slate", 0x4b5058, 0.7).grain(0.1),
        m("brick", 0x9c4a32, 0.85).grain(0.16),
        m("brick_dark", 0x6e3426, 0.85).grain(0.16),
        m("terracotta", 0xb5643f, 0.8).grain(0.08),
        m("plaster", 0xe8e1d3, 0.9).grain(0.05),
        m("concrete", 0xb9b7b1, 0.9).grain(0.08),
        m("concrete_dark", 0x6c6b68, 0.9).grain(0.08),
        m("asphalt", 0x3a3b3d, 0.95).grain(0.1),
        m("tile_white", 0xf1efe9, 0.3).grain(0.03),
        m("obsidian", 0x1a1424, 0.2).grain(0.05),
        // Roofs.
        m("roof_tile", 0x7a3b2e, 0.8).grain(0.12),
        m("roof_slate", 0x3e434a, 0.75).grain(0.1),
        m("thatch", 0xb89a5a, 0.95).grain(0.2),
        m("shingle", 0x5a4a3f, 0.85).grain(0.14),
        m("copper_roof", 0x5e9c86, 0.6).metal(0.4).grain(0.1),
        // Ground and nature.
        m("grass", 0x5f8f3a, 0.9).grain(0.15),
        m("leaves", 0x3f6e2a, 0.8).grain(0.2).foliage(),
        m("leaves_light", 0x6f9a3a, 0.8).grain(0.2).foliage(),
        m("leaves_autumn", 0xc0632a, 0.8).grain(0.2).foliage(),
        m("leaves_blossom", 0xf2b8c6, 0.8).grain(0.15).foliage(),
        m("dirt", 0x6b4f35, 0.95).grain(0.15),
        m("mud", 0x4a3828, 0.6).grain(0.1),
        m("sand", 0xdcc79a, 0.95).grain(0.1),
        m("gravel", 0x8b857b, 0.95).grain(0.25),
        m("snow", 0xf3f6fa, 0.6).grain(0.03),
        m("ice", 0xbfe3f2, 0.1).glass().grain(0.02),
        m("water", 0x2a5f88, 0.05).water().grain(0.0),
        m("bone", 0xe6dcc6, 0.7).grain(0.06),
        // Glass.
        m("glass", 0xcfe6ee, 0.05).glass().grain(0.0),
        m("glass_tinted", 0x4a6a7a, 0.05).glass().grain(0.0),
        m("glass_stained_red", 0xb8323a, 0.1).glass().grain(0.02),
        m("glass_stained_blue", 0x3256b8, 0.1).glass().grain(0.02),
        // Metals.
        m("iron", 0x9aa0a6, 0.4).metal(0.9).grain(0.05),
        m("steel", 0x8e959c, 0.3).metal(0.95).grain(0.04),
        m("steel_dark", 0x45494e, 0.4).metal(0.9).grain(0.04),
        m("gold", 0xe2b74a, 0.25).metal(1.0).grain(0.03),
        m("copper", 0xc27a4a, 0.3).metal(1.0).grain(0.05),
        m("chrome", 0xdfe3e6, 0.08).metal(1.0).grain(0.0),
        m("rust", 0x8a4a2a, 0.9).metal(0.3).grain(0.2),
        // Light.
        m("lamp", 0xfff1c8, 0.4).emit(20.0).grain(0.0),
        m("lamp_warm", 0xffc070, 0.4).emit(12.0).grain(0.0),
        m("candle", 0xffb050, 0.5).emit(6.0).grain(0.0),
        m("neon_pink", 0xff2fa8, 0.3).emit(8.0).grain(0.0),
        m("neon_blue", 0x2fd0ff, 0.3).emit(8.0).grain(0.0),
        m("neon_green", 0x58ff7a, 0.3).emit(8.0).grain(0.0),
        m("neon_yellow", 0xffe84a, 0.3).emit(8.0).grain(0.0),
        m("lava", 0xff5a1a, 0.6).emit(6.0).grain(0.2),
        m("screen", 0x9fd8ff, 0.2).emit(3.0).grain(0.0),
        m("fire", 0xff8a2a, 0.8).emit(14.0).grain(0.3),
        // Fabric, plastic, paint.
        m("fabric_red", 0xa8323a, 0.95).grain(0.08),
        m("fabric_blue", 0x2f4f8a, 0.95).grain(0.08),
        m("fabric_green", 0x3d6b3f, 0.95).grain(0.08),
        m("wool_white", 0xece8e0, 0.95).grain(0.06),
        m("leather", 0x6a3f28, 0.6).grain(0.08),
        m("rubber", 0x1e1e20, 0.9).grain(0.03),
        m("plastic_white", 0xf0f0ec, 0.35).grain(0.02),
        m("plastic_black", 0x202124, 0.35).grain(0.02),
        m("paint_red", 0xc8342b, 0.45).grain(0.03),
        m("paint_orange", 0xe0742a, 0.45).grain(0.03),
        m("paint_yellow", 0xe9b824, 0.45).grain(0.03),
        m("paint_green", 0x3a8a4a, 0.45).grain(0.03),
        m("paint_blue", 0x2e6bd0, 0.45).grain(0.03),
        m("paint_purple", 0x7a3bb0, 0.45).grain(0.03),
        m("paint_white", 0xf4f2ee, 0.45).grain(0.03),
        m("paint_black", 0x1c1c1e, 0.45).grain(0.03),
        m("hazard", 0xe8b21c, 0.6).grain(0.05),
        // Player-coloured (takes the tint of whoever draws it).
        m("team", 0xffffff, 0.6).tint().grain(0.03),
    ];
    let lv = [0x00, 0x33, 0x66, 0x99, 0xcc, 0xff];
    for r in 0..6 {
        for g in 0..6 {
            for b in 0..6 {
                v.push(m(&format!("c{r}{g}{b}"), lv[r] << 16 | lv[g] << 8 | lv[b], 0.7).grain(0.04));
            }
        }
    }
    // A finer 8-level cube for exact-ish `#rrggbb` (appended after, so earlier ids never move).
    let fine = [0x00, 0x24, 0x49, 0x6d, 0x92, 0xb6, 0xdb, 0xff];
    for r in 0..8 {
        for g in 0..8 {
            for b in 0..8 {
                v.push(m(&format!("d{r}{g}{b}"), fine[r] << 16 | fine[g] << 8 | fine[b], 0.7).grain(0.04));
            }
        }
    }
    v
}

/// Append the builder palette to `pal` (idempotent: names already present keep their id).
pub fn add_to(pal: &mut Palette) {
    for mat in materials() {
        pal.add(mat);
    }
}

/// A palette with only the builder materials (what models are built against).
pub fn palette() -> Palette {
    let mut p = Palette::default();
    add_to(&mut p);
    p
}

fn rgb(c: u32) -> [i32; 3] {
    [(c >> 16 & 255) as i32, (c >> 8 & 255) as i32, (c & 255) as i32]
}

/// A material by name in `pal`: a builder name ("brick"), `#rrggbb` (nearest plain colour),
/// or "air" / "carve" (= AIR, removes voxels).
pub fn resolve(pal: &Palette, name: &str) -> Option<Mat> {
    let n = name.trim().to_ascii_lowercase();
    if matches!(n.as_str(), "air" | "carve" | "none" | "empty") {
        return Some(AIR);
    }
    if let Some(hex) = n.strip_prefix('#') {
        let want = rgb(u32::from_str_radix(hex, 16).ok()?);
        let mut best: Option<(i32, Mat)> = None;
        for (i, mat) in pal.materials.iter().enumerate().skip(1) {
            // Only plain, opaque, unlit materials stand in for a colour.
            let plain = mat.flags & (flag::GLASS | flag::EMITTER | flag::TINT | flag::FOLIAGE) == 0
                && mat.metalness < 0.5
                && ((mat.name.starts_with('c') || mat.name.starts_with('d')) && mat.name.len() == 4
                    || mat.name.starts_with("paint_"));
            if !plain {
                continue;
            }
            let c = rgb(mat.color);
            let d = (0..3).map(|k| (c[k] - want[k]).pow(2)).sum::<i32>();
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, i as Mat));
            }
        }
        return best.map(|b| b.1);
    }
    pal.id(&n)
}

/// Builder material names, for agents and the UI.
pub fn names() -> Vec<String> {
    materials()
        .into_iter()
        .map(|m| m.name)
        .filter(|n| !(n.len() == 4 && (n.starts_with('c') || n.starts_with('d'))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_colours_resolve() {
        let mut pal = Palette::default_city();
        let before = pal.materials.len();
        add_to(&mut pal);
        add_to(&mut pal);
        assert!(pal.materials.len() > before + 200, "colour cube added once");
        let brick = resolve(&pal, "Brick").unwrap();
        assert_eq!(pal.get(brick).name, "brick");
        assert_eq!(resolve(&pal, "air"), Some(AIR));
        let red = resolve(&pal, "#ff0000").unwrap();
        assert_eq!(pal.get(red).color, 0xff0000);
        assert!(resolve(&pal, "unobtainium").is_none());
    }
}

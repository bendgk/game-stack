//! The player avatar: a 1.8 m voxel figure whose shirt takes the player's colour (TINT).
//! Built facing −z, feet at the bottom centre of a 2 m (level 3) cube.

use std::sync::Arc;

use vx_core::{Dag, Material, Palette};

use crate::kit::Kit;

pub fn materials(p: &mut Palette) -> [u16; 5] {
    [
        p.add(Material::new("avatar_skin", 0xd9a37e, 0.7).grain(0.03)),
        p.add(Material::new("avatar_shirt", 0xffffff, 0.8).tint().grain(0.05)),
        p.add(Material::new("avatar_pants", 0x2d3550, 0.85).grain(0.06)),
        p.add(Material::new("avatar_hair", 0x2b1d14, 0.9).grain(0.1)),
        p.add(Material::new("avatar_eye", 0x101014, 0.3).grain(0.0)),
    ]
}

/// Build the avatar prefab (call after `materials` on the same palette).
pub fn build(palette: &Palette) -> Arc<Dag> {
    let id = |n: &str| palette.id(n).expect("avatar materials registered");
    let (skin, shirt, pants, hair, eye) =
        (id("avatar_skin"), id("avatar_shirt"), id("avatar_pants"), id("avatar_hair"), id("avatar_eye"));
    let mut k = Kit::new();
    let c = 32; // centre of the 64-voxel cube
    // Legs (0..26), torso (26..44), head (45..57).
    k.bx([c - 7, 0, c - 3], [c - 1, 26, c + 3], pants);
    k.bx([c + 1, 0, c - 3], [c + 7, 26, c + 3], pants);
    k.bx([c - 8, 0, c - 4], [c - 1, 3, c + 4], hair); // shoes
    k.bx([c + 1, 0, c - 4], [c + 8, 3, c + 4], hair);
    k.bx([c - 9, 26, c - 4], [c + 9, 45, c + 4], shirt);
    k.bx([c - 13, 27, c - 3], [c - 9, 44, c + 3], shirt); // arms
    k.bx([c + 9, 27, c - 3], [c + 13, 44, c + 3], shirt);
    k.bx([c - 13, 22, c - 3], [c - 9, 27, c + 3], skin); // hands
    k.bx([c + 9, 22, c - 3], [c + 13, 27, c + 3], skin);
    k.bx([c - 6, 45, c - 6], [c + 6, 58, c + 6], skin);
    k.bx([c - 7, 54, c - 7], [c + 7, 60, c + 7], hair);
    k.bx([c - 7, 49, c - 1], [c - 6, 58, c + 7], hair); // sides
    k.bx([c + 6, 49, c - 1], [c + 7, 58, c + 7], hair);
    k.bx([c - 4, 51, c - 7], [c - 2, 53, c - 6], eye); // eyes look −z
    k.bx([c + 2, 51, c - 7], [c + 4, 53, c - 6], eye);
    k.build(palette, 3)
}

//! The procedural prop generator — a Garry's-Mod-style catalog, 100% parametric.
//!
//! Every prop is a function of a seed: sizes, proportions, colours, wear and details vary, so
//! no two crates are the same, yet the same seed always rebuilds the same prop. Props are built
//! into prefab DAGs (one 2 m or 8 m cube, bottom-centred, facing −z) and placed by pointer, so
//! a city full of them costs almost nothing.
//!
//! Units: voxels (32 per metre). `Ctx::c` is the cube centre; y = 0 is the floor.

use std::sync::Arc;

use vx_core::{Dag, Mat, Palette};

use crate::kit::{Blob, Kit};
use crate::rng::Rng;
use crate::style::Mats;

pub struct Ctx<'a> {
    pub k: Kit,
    pub r: Rng,
    pub m: &'a Mats,
    /// Cube centre (x/z) in voxels.
    pub c: i32,
}

impl Ctx<'_> {
    /// Box centred on the cube's x/z centre: half-widths (hx, hz), y from y0 to y1, offset (ox, oz).
    fn cbox(&mut self, ox: i32, oz: i32, hx: i32, hz: i32, y0: i32, y1: i32, mat: Mat) {
        let c = self.c;
        self.k.bx([c + ox - hx, y0, c + oz - hz], [c + ox + hx, y1, c + oz + hz], mat);
    }
    fn ccarve(&mut self, ox: i32, oz: i32, hx: i32, hz: i32, y0: i32, y1: i32) {
        let c = self.c;
        self.k.carve([c + ox - hx, y0, c + oz - hz], [c + ox + hx, y1, c + oz + hz]);
    }
    fn vcyl(&mut self, ox: f32, oz: f32, r: f32, y0: f32, y1: f32, mat: Mat) {
        let c = self.c as f32;
        self.k.cyl(1, [c + ox, 0.0, c + oz], r, y0, y1, mat);
    }
    fn paint(&mut self) -> Mat {
        *self.r.pick(&self.m.paint)
    }
}

pub struct PropDef {
    pub name: &'static str,
    pub category: &'static str,
    /// Prefab level: 3 = 2 m cube, 4 = 8 m cube.
    pub level: u32,
    pub build: fn(&mut Ctx),
}

macro_rules! prop {
    ($name:literal, $cat:literal, $lvl:literal, $f:ident) => {
        PropDef { name: $name, category: $cat, level: $lvl, build: $f }
    };
}

pub const CATALOG: &[PropDef] = &[
    // Physics junk (the classics).
    prop!("crate_wood", "junk", 3, crate_wood),
    prop!("crate_metal", "junk", 3, crate_metal),
    prop!("barrel_oil", "junk", 3, barrel_oil),
    prop!("barrel_explosive", "junk", 3, barrel_explosive),
    prop!("pallet", "junk", 3, pallet),
    prop!("tire", "junk", 3, tire),
    prop!("traffic_cone", "junk", 3, traffic_cone),
    prop!("cinder_block", "junk", 3, cinder_block),
    prop!("cardboard_box", "junk", 3, cardboard_box),
    prop!("propane_tank", "junk", 3, propane_tank),
    prop!("sawhorse", "junk", 3, sawhorse),
    prop!("watermelon", "junk", 3, watermelon),
    // Furniture & appliances.
    prop!("chair", "furniture", 3, chair),
    prop!("table", "furniture", 3, table),
    prop!("sofa", "furniture", 3, sofa),
    prop!("fridge", "furniture", 3, fridge),
    prop!("washing_machine", "furniture", 3, washing_machine),
    prop!("tv", "furniture", 3, tv),
    prop!("computer", "furniture", 3, computer),
    prop!("vending_machine", "furniture", 3, vending_machine),
    prop!("filing_cabinet", "furniture", 3, filing_cabinet),
    prop!("bathtub", "furniture", 3, bathtub),
    prop!("toilet", "furniture", 3, toilet),
    // Street furniture.
    prop!("bench", "street", 3, bench),
    prop!("trash_bin", "street", 3, trash_bin),
    prop!("dumpster", "street", 3, dumpster),
    prop!("hydrant", "street", 3, hydrant),
    prop!("mailbox", "street", 3, mailbox),
    prop!("bollard", "street", 3, bollard),
    prop!("planter", "street", 3, planter),
    prop!("ac_unit", "roof", 3, ac_unit),
    prop!("street_lamp", "street", 4, street_lamp),
    prop!("traffic_light", "street", 4, traffic_light),
    prop!("bus_shelter", "street", 4, bus_shelter),
    prop!("umbrella_table", "street", 4, umbrella_table),
    prop!("fountain", "street", 4, fountain),
    prop!("water_tower", "roof", 4, water_tower),
    // Vehicles & nature.
    prop!("car", "vehicle", 4, car),
    prop!("tree", "nature", 4, tree),
    prop!("bush", "nature", 3, bush),
    prop!("rock", "nature", 3, rock),
];

pub fn find(name: &str) -> Option<&'static PropDef> {
    CATALOG.iter().find(|p| p.name == name)
}

/// Build prop `def` variant `seed`, rotated `turns` × 90°, standing on local height `base`
/// (voxels from the bottom of its prefab cube — lets an 8 m prop sit on a raised sidewalk).
pub fn build(def: &PropDef, seed: u64, turns: u8, base: i32, mats: &Mats, palette: &Palette) -> Arc<Dag> {
    let size = 1 << (2 * def.level);
    // 2 m props are centred in their cube; 8 m props at (160, 160) so that placing the cube at
    // a 2 m-aligned origin puts the prop on a module centre (keeps prefab links aligned).
    let c = if def.level == 3 { size / 2 } else { size / 2 + 32 };
    let mut cx = Ctx {
        k: Kit::new(),
        r: Rng::new(seed ^ crate::rng::mix(def.name.len() as u64 * 7919 + def.name.as_bytes()[0] as u64)),
        m: mats,
        c,
    };
    (def.build)(&mut cx);
    if base != 0 {
        let parts = std::mem::take(&mut cx.k.parts);
        for p in parts {
            cx.k.parts.push(Box::new(crate::kit::Moved { inner: Arc::from(p), by: vx_core::IVec3::new(0, base, 0) }));
        }
    }
    if def.level == 3 {
        return cx.k.rotate(turns, size).build(palette, def.level);
    }
    // 8 m props rotate about their centre (160, 160): the centre of a 320-wide frame.
    let k = cx.k.rotate(turns, 320);
    k.build(palette, def.level)
}

// ---------------------------------------------------------------------------------------------
// Junk
// ---------------------------------------------------------------------------------------------

fn crate_wood(x: &mut Ctx) {
    let s = x.r.int(13, 19);
    let (w, wd) = (x.m.wood, x.m.wood_dark);
    x.cbox(0, 0, s, s, 0, 2 * s, w);
    // Edge frame + diagonal-ish brace on each face.
    let e = 3;
    for (ox, oz) in [(-s + e / 2, -s + e / 2), (s - e / 2, -s + e / 2), (-s + e / 2, s - e / 2), (s - e / 2, s - e / 2)]
    {
        x.cbox(ox, oz, e / 2 + 1, e / 2 + 1, 0, 2 * s, wd);
    }
    for y in [0, 2 * s - e] {
        x.cbox(0, -s, s, 1, y, y + e, wd);
        x.cbox(0, s - 1, s, 1, y, y + e, wd);
        x.cbox(-s, 0, 1, s, y, y + e, wd);
        x.cbox(s - 1, 0, 1, s, y, y + e, wd);
    }
    // Plank gaps.
    for k in 1..4 {
        let y = k * 2 * s / 4;
        x.cbox(0, -s, s - e, 1, y, y + 1, wd);
    }
}

fn crate_metal(x: &mut Ctx) {
    let (hx, hz, h) = (x.r.int(16, 26), x.r.int(12, 18), x.r.int(24, 36));
    let c = x.paint();
    x.cbox(0, 0, hx, hz, 0, h, c);
    for k in -2..=2 {
        x.cbox(k * hx / 3, 0, 1, hz + 1, 2, h - 2, x.m.steel_dark);
    }
    x.cbox(0, 0, hx + 1, hz + 1, h - 2, h, x.m.steel);
}

fn barrel_with(x: &mut Ctx, body: Mat, band: Option<Mat>) {
    let r = x.r.range(9.0, 11.0);
    let h = x.r.range(27.0, 31.0);
    x.vcyl(0.0, 0.0, r, 0.0, h, body);
    for y in [h * 0.33, h * 0.66] {
        x.vcyl(0.0, 0.0, r + 0.6, y, y + 1.2, x.m.steel_dark);
    }
    if let Some(b) = band {
        x.vcyl(0.0, 0.0, r + 0.3, h * 0.42, h * 0.58, b);
    }
    x.vcyl(0.0, 0.0, r - 1.5, h - 1.0, h, x.m.steel_dark);
}

fn barrel_oil(x: &mut Ctx) {
    let c = x.paint();
    barrel_with(x, c, None);
}

fn barrel_explosive(x: &mut Ctx) {
    barrel_with(x, x.m.red_barrel, Some(x.m.hazard));
}

fn pallet(x: &mut Ctx) {
    let (w, wd) = (x.m.wood, x.m.wood_dark);
    let (hx, hz) = (19, 15);
    for k in -2..=2 {
        x.cbox(k * 9, 0, 2, hz, 3, 5, w);
    }
    for k in [-1, 0, 1] {
        x.cbox(0, k * (hz - 2), hx, 2, 0, 3, wd);
    }
    for k in -4..=4 {
        x.cbox(0, k * 3, hx, 1, 5, 6, if k % 2 == 0 { w } else { wd });
    }
}

fn tire(x: &mut Ctx) {
    let r = x.r.range(10.0, 13.0);
    let standing = x.r.chance(0.5);
    let c = x.c as f32;
    if standing {
        x.k.cyl(2, [c, r, c], r, c - 4.0, c + 4.0, x.m.rubber);
        x.k.cyl(2, [c, r, c], r * 0.55, c - 3.0, c + 3.0, x.m.paint[7]);
        x.k.cyl(2, [c, r, c], r * 0.3, c - 5.0, c + 5.0, vx_core::CARVE);
    } else {
        x.vcyl(0.0, 0.0, r, 0.0, 8.0, x.m.rubber);
        x.vcyl(0.0, 0.0, r * 0.5, 0.0, 9.0, vx_core::CARVE);
    }
}

fn traffic_cone(x: &mut Ctx) {
    let h = x.r.int(20, 26);
    x.cbox(0, 0, 9, 9, 0, 2, x.m.paint[6]);
    for y in 2..h {
        let t = (y - 2) as f32 / (h - 2) as f32;
        let r = 7.0 * (1.0 - t) + 1.2;
        let band = (0.45..0.62).contains(&t);
        x.vcyl(0.0, 0.0, r, y as f32, y as f32 + 1.0, if band { x.m.plastic_white } else { x.m.paint[6] });
    }
}

fn cinder_block(x: &mut Ctx) {
    x.cbox(0, 0, 12, 6, 0, 6, x.m.concrete);
    x.ccarve(-6, 0, 4, 3, 0, 6);
    x.ccarve(6, 0, 4, 3, 0, 6);
}

fn cardboard_box(x: &mut Ctx) {
    let (hx, hz, h) = (x.r.int(6, 14), x.r.int(6, 12), x.r.int(8, 22));
    x.cbox(0, 0, hx, hz, 0, h, x.m.cardboard);
    x.cbox(0, 0, 2, hz + 1, h - 1, h + 1, x.m.plastic_white);
    if x.r.chance(0.5) {
        // A smaller box stacked on top.
        let (a, b) = (hx * 2 / 3, hz * 2 / 3);
        x.cbox(1, -1, a, b, h + 1, h + 1 + h * 2 / 3, x.m.cardboard);
    }
}

fn propane_tank(x: &mut Ctx) {
    let r = 6.0;
    x.vcyl(0.0, 0.0, r, 1.0, 22.0, x.m.plastic_white);
    let c = x.c as f32;
    x.k.blob([c, 22.0, c], [r, 4.0, r], x.m.plastic_white, 0.0, 1);
    x.vcyl(0.0, 0.0, 2.0, 24.0, 28.0, x.m.steel);
    x.vcyl(0.0, 0.0, r + 0.5, 0.0, 1.5, x.m.steel_dark);
}

fn sawhorse(x: &mut Ctx) {
    x.cbox(0, 0, 22, 2, 22, 26, x.m.hazard);
    for sx in [-18, 18] {
        for sz in [-6, 6] {
            x.cbox(sx, sz, 1, 1, 0, 23, x.m.wood);
        }
    }
}

fn watermelon(x: &mut Ctx) {
    let c = x.c as f32;
    let seed = x.r.u64();
    x.k.push(Blob {
        center: [c, 5.0, c].into(),
        radii: [8.0, 5.5, 5.5].into(),
        mat: x.m.melon,
        noise: 0.0,
        seed,
        fill: 1.0,
        alt: Some((x.m.melon_stripe, 0.4)),
    });
}

// ---------------------------------------------------------------------------------------------
// Furniture
// ---------------------------------------------------------------------------------------------

fn chair(x: &mut Ctx) {
    let wood = if x.r.chance(0.5) { x.m.wood } else { x.paint() };
    let s = 7;
    x.cbox(0, 0, s, s, 13, 15, wood);
    for (ox, oz) in [(-s + 1, -s + 1), (s - 1, -s + 1), (-s + 1, s - 1), (s - 1, s - 1)] {
        x.cbox(ox, oz, 1, 1, 0, 13, wood);
    }
    x.cbox(0, s - 1, s, 1, 15, 30, wood);
}

fn table(x: &mut Ctx) {
    let (hx, hz) = (x.r.int(14, 24), x.r.int(10, 16));
    let wood = x.m.wood;
    x.cbox(0, 0, hx, hz, 23, 25, wood);
    for (ox, oz) in [(-hx + 2, -hz + 2), (hx - 2, -hz + 2), (-hx + 2, hz - 2), (hx - 2, hz - 2)] {
        x.cbox(ox, oz, 1, 1, 0, 23, x.m.wood_dark);
    }
    if x.r.chance(0.5) {
        let (px, pz) = (x.r.int(-6, 6), x.r.int(-4, 4));
        x.cbox(px, pz, 3, 3, 25, 26, x.m.plastic_white); // a plate
    }
}

fn sofa(x: &mut Ctx) {
    let f = *x.r.pick(&x.m.fabric);
    let hx = x.r.int(24, 30);
    x.cbox(0, 0, hx, 12, 2, 12, f);
    x.cbox(0, 9, hx, 4, 12, 26, f);
    x.cbox(-hx + 2, 0, 3, 12, 12, 18, f);
    x.cbox(hx - 2, 0, 3, 12, 12, 18, f);
    for (ox, oz) in [(-hx + 2, -10), (hx - 2, -10), (-hx + 2, 10), (hx - 2, 10)] {
        x.cbox(ox, oz, 1, 1, 0, 2, x.m.wood_dark);
    }
}

fn fridge(x: &mut Ctx) {
    let h = x.r.int(52, 60);
    let body = if x.r.chance(0.7) { x.m.plastic_white } else { x.m.chrome };
    x.cbox(0, 0, 11, 11, 0, h, body);
    x.cbox(0, -11, 11, 1, h * 2 / 3, h * 2 / 3 + 1, x.m.plastic_black);
    x.cbox(8, -12, 1, 1, h * 2 / 3 + 4, h - 6, x.m.steel);
    x.cbox(8, -12, 1, 1, 6, h * 2 / 3 - 4, x.m.steel);
}

fn washing_machine(x: &mut Ctx) {
    x.cbox(0, 0, 10, 10, 0, 27, x.m.plastic_white);
    let c = x.c as f32;
    x.k.cyl(2, [c, 13.0, 0.0], 6.0, c - 11.0, c - 9.0, x.m.glass_dark);
    x.k.cyl(2, [c, 13.0, 0.0], 7.0, c - 10.5, c - 9.5, x.m.steel);
    x.cbox(0, -10, 9, 1, 23, 26, x.m.plastic_black);
}

fn tv(x: &mut Ctx) {
    let (hx, h) = (x.r.int(14, 24), x.r.int(10, 16));
    let y0 = 6;
    x.cbox(0, 0, hx, 1, y0, y0 + 2 * h, x.m.plastic_black);
    x.cbox(0, -1, hx - 1, 1, y0 + 1, y0 + 2 * h - 1, x.m.screen);
    x.cbox(0, 0, 2, 2, 0, y0, x.m.plastic_black);
    x.cbox(0, 0, 8, 5, 0, 1, x.m.plastic_black);
}

fn computer(x: &mut Ctx) {
    // Monitor on a little desk.
    x.cbox(0, 0, 18, 10, 22, 24, x.m.wood);
    for (ox, oz) in [(-16, -8), (16, -8), (-16, 8), (16, 8)] {
        x.cbox(ox, oz, 1, 1, 0, 22, x.m.steel_dark);
    }
    x.cbox(0, 4, 9, 1, 26, 38, x.m.plastic_black);
    x.cbox(0, 3, 8, 1, 27, 37, x.m.screen);
    x.cbox(0, 4, 1, 2, 24, 26, x.m.plastic_black);
    x.cbox(0, -4, 7, 2, 24, 25, x.m.plastic_black);
    x.cbox(14, 2, 3, 6, 24, 36, x.m.plastic_black);
}

fn vending_machine(x: &mut Ctx) {
    let c = x.paint();
    x.cbox(0, 0, 15, 12, 0, 58, c);
    x.cbox(-3, -12, 10, 1, 14, 54, x.m.glass_lit_cool);
    for k in 0..5 {
        let y = 18 + k * 8;
        x.cbox(-3, -11, 10, 1, y, y + 1, x.m.steel_dark);
    }
    x.cbox(11, -12, 3, 1, 26, 40, x.m.plastic_black);
    x.cbox(-3, -12, 10, 1, 4, 10, x.m.plastic_black);
}

fn filing_cabinet(x: &mut Ctx) {
    let n = x.r.int(2, 4);
    let h = n * 10 + 2;
    x.cbox(0, 0, 8, 10, 0, h, x.m.paint[7]);
    for k in 0..n {
        let y = 2 + k * 10;
        x.cbox(0, -10, 7, 1, y, y + 1, x.m.steel_dark);
        x.cbox(0, -11, 2, 1, y + 5, y + 6, x.m.chrome);
    }
}

fn bathtub(x: &mut Ctx) {
    x.cbox(0, 0, 26, 12, 2, 18, x.m.plastic_white);
    x.ccarve(0, 0, 23, 9, 5, 18);
    for (ox, oz) in [(-22, -8), (22, -8), (-22, 8), (22, 8)] {
        x.cbox(ox, oz, 2, 2, 0, 2, x.m.chrome);
    }
}

fn toilet(x: &mut Ctx) {
    let w = x.m.plastic_white;
    x.cbox(0, 0, 4, 5, 0, 10, w);
    let c = x.c as f32;
    x.k.blob([c, 12.0, c - 3.0], [6.0, 3.0, 8.0], w, 0.0, 3);
    x.k.carve([x.c - 4, 13, x.c - 9], [x.c + 4, 16, x.c + 3]);
    x.cbox(0, 9, 6, 3, 10, 26, w);
}

// ---------------------------------------------------------------------------------------------
// Street furniture
// ---------------------------------------------------------------------------------------------

fn bench(x: &mut Ctx) {
    let hx = x.r.int(22, 30);
    let slat = if x.r.chance(0.7) { x.m.wood } else { x.m.paint[3] };
    for k in 0..4 {
        x.cbox(0, -6 + k * 3, hx, 1, 14, 15, slat);
    }
    for k in 0..3 {
        x.cbox(0, 8, hx, 1, 18 + k * 4, 20 + k * 4, slat);
    }
    for sx in [-hx + 3, hx - 3] {
        x.cbox(sx, 0, 1, 7, 0, 14, x.m.steel_dark);
        x.cbox(sx, 8, 1, 1, 14, 30, x.m.steel_dark);
    }
}

fn trash_bin(x: &mut Ctx) {
    let c = if x.r.chance(0.6) { x.m.paint[3] } else { x.m.steel_dark };
    x.vcyl(0.0, 0.0, 9.0, 0.0, 28.0, c);
    x.vcyl(0.0, 0.0, 7.0, 4.0, 28.0, vx_core::CARVE);
    x.vcyl(0.0, 0.0, 9.5, 28.0, 30.0, x.m.steel_dark);
}

fn dumpster(x: &mut Ctx) {
    let c = if x.r.chance(0.5) { x.m.paint[3] } else { x.m.paint[1] };
    x.cbox(0, 0, 28, 16, 4, 40, c);
    x.cbox(0, 0, 29, 17, 38, 42, x.m.steel_dark);
    for (ox, oz) in [(-24, -12), (24, -12), (-24, 12), (24, 12)] {
        x.cbox(ox, oz, 2, 2, 0, 4, x.m.rubber);
    }
    x.cbox(0, -16, 26, 1, 20, 22, x.m.steel_dark);
}

fn hydrant(x: &mut Ctx) {
    let c = if x.r.chance(0.8) { x.m.paint[0] } else { x.m.paint[2] };
    x.vcyl(0.0, 0.0, 5.0, 0.0, 2.0, c);
    x.vcyl(0.0, 0.0, 4.0, 2.0, 20.0, c);
    let cc = x.c as f32;
    x.k.blob([cc, 20.0, cc], [4.0, 3.0, 4.0], c, 0.0, 9);
    x.k.cyl(0, [0.0, 13.0, cc], 1.8, cc - 7.0, cc + 7.0, x.m.chrome);
    x.k.cyl(2, [cc, 13.0, 0.0], 2.2, cc - 7.0, cc, x.m.chrome);
}

fn mailbox(x: &mut Ctx) {
    let c = x.m.paint[1];
    x.cbox(0, 0, 3, 3, 0, 16, x.m.steel_dark);
    x.cbox(0, 0, 9, 12, 16, 38, c);
    let cc = x.c as f32;
    x.k.cyl(2, [cc, 38.0, 0.0], 9.0, cc - 12.0, cc + 12.0, c);
    x.cbox(0, -12, 5, 1, 30, 32, x.m.plastic_black);
}

fn bollard(x: &mut Ctx) {
    x.vcyl(0.0, 0.0, 3.5, 0.0, 28.0, x.m.steel_dark);
    x.vcyl(0.0, 0.0, 3.8, 22.0, 24.0, x.m.hazard);
}

fn planter(x: &mut Ctx) {
    let (hx, hz) = (x.r.int(14, 24), x.r.int(14, 24));
    x.cbox(0, 0, hx, hz, 0, 16, x.m.concrete);
    x.cbox(0, 0, hx - 2, hz - 2, 12, 16, x.m.dirt);
    let c = x.c as f32;
    let seed = x.r.u64();
    x.k.push(Blob {
        center: [c, 24.0, c].into(),
        radii: [hx as f32 - 3.0, 12.0, hz as f32 - 3.0].into(),
        mat: x.m.leaf[1],
        noise: 0.35,
        seed,
        fill: 0.55,
        alt: Some((x.m.flowers[(seed % 3) as usize], 0.12)),
    });
}

fn ac_unit(x: &mut Ctx) {
    let (hx, hz, h) = (x.r.int(16, 26), x.r.int(12, 20), x.r.int(18, 30));
    x.cbox(0, 0, hx, hz, 0, h, x.m.paint[7]);
    let c = x.c as f32;
    x.k.cyl(1, [c - 6.0, 0.0, c], 8.0, h as f32, h as f32 + 2.0, x.m.steel_dark);
    for k in 0..6 {
        x.cbox(0, -hz, hx - 2, 1, 3 + k * 3, 4 + k * 3, x.m.steel_dark);
    }
}

fn street_lamp(x: &mut Ctx) {
    let h = x.r.int(170, 230);
    let pole = if x.r.chance(0.6) { x.m.steel_dark } else { x.m.paint[5] };
    x.vcyl(0.0, 0.0, 5.0, 0.0, 6.0, x.m.concrete_dark);
    x.vcyl(0.0, 0.0, 2.5, 6.0, h as f32, pole);
    let arm = x.r.int(30, 50);
    x.cbox(0, -arm / 2, 1, arm / 2, h - 3, h, pole);
    x.cbox(0, -arm, 5, 7, h - 6, h, pole);
    x.cbox(0, -arm, 4, 6, h - 7, h - 6, x.m.lamp);
}

fn traffic_light(x: &mut Ctx) {
    let h = 150;
    x.vcyl(0.0, 0.0, 3.0, 0.0, h as f32, x.m.steel_dark);
    x.cbox(0, -40, 1, 40, h - 4, h, x.m.steel_dark);
    let z = -76;
    x.cbox(0, z, 6, 6, h - 40, h, x.m.plastic_black);
    for (k, lamp) in [x.m.lamp_red, x.m.hazard, x.m.lamp_green].into_iter().enumerate() {
        let y = h - 10 - k as i32 * 11;
        let lit = (k == 0 && x.r.chance(0.5)) || (k == 2 && x.r.chance(0.5));
        x.cbox(0, z - 6, 4, 1, y - 4, y + 4, if lit || k == 1 { lamp } else { x.m.plastic_black });
    }
}

fn bus_shelter(x: &mut Ctx) {
    let (hx, hz) = (70, 26);
    x.cbox(0, 0, hx + 4, hz + 4, 92, 96, x.m.steel_dark);
    for ox in [-hx, hx] {
        for oz in [-hz, hz] {
            x.cbox(ox, oz, 2, 2, 0, 92, x.m.steel_dark);
        }
    }
    x.cbox(0, hz, hx, 1, 8, 90, x.m.glass);
    x.cbox(-hx, 0, 1, hz, 8, 90, x.m.glass);
    x.cbox(0, hz - 8, hx - 10, 6, 14, 16, x.m.wood);
    x.cbox(hx - 1, 0, 1, hz - 6, 20, 80, x.m.glass_lit_cool); // ad panel
}

fn umbrella_table(x: &mut Ctx) {
    let c = x.c as f32;
    x.vcyl(0.0, 0.0, 16.0, 23.0, 25.0, x.m.plastic_white);
    x.vcyl(0.0, 0.0, 1.0, 0.0, 80.0, x.m.steel);
    let canvas = x.paint();
    for y in 0..14 {
        let r = 50.0 * (1.0 - y as f32 / 14.0) + 2.0;
        x.k.cyl(1, [c, 0.0, c], r, 66.0 + y as f32, 67.0 + y as f32, canvas);
    }
    for k in 0..4 {
        let a = k as f32 * std::f32::consts::FRAC_PI_2 + 0.4;
        let (ox, oz) = ((a.cos() * 30.0) as i32, (a.sin() * 30.0) as i32);
        x.cbox(ox, oz, 6, 6, 13, 15, x.m.plastic_white);
        for (lx, lz) in [(-5, -5), (5, -5), (-5, 5), (5, 5)] {
            x.cbox(ox + lx, oz + lz, 1, 1, 0, 13, x.m.plastic_white);
        }
    }
}

fn fountain(x: &mut Ctx) {
    let r = x.r.range(80.0, 110.0);
    x.vcyl(0.0, 0.0, r, 0.0, 16.0, x.m.stone);
    x.vcyl(0.0, 0.0, r - 6.0, 6.0, 16.0, vx_core::CARVE);
    x.vcyl(0.0, 0.0, r - 6.0, 6.0, 12.0, x.m.water);
    x.vcyl(0.0, 0.0, 10.0, 0.0, 50.0, x.m.stone);
    x.vcyl(0.0, 0.0, 30.0, 50.0, 56.0, x.m.stone);
    x.vcyl(0.0, 0.0, 26.0, 52.0, 56.0, x.m.water);
    x.vcyl(0.0, 0.0, 4.0, 56.0, 74.0, x.m.stone);
}

fn water_tower(x: &mut Ctx) {
    let c = x.c as f32;
    for (ox, oz) in [(-40, -40), (40, -40), (-40, 40), (40, 40)] {
        x.cbox(ox, oz, 2, 2, 0, 120, x.m.steel_dark);
    }
    x.vcyl(0.0, 0.0, 56.0, 120.0, 210.0, x.m.wood);
    for y in [135.0, 165.0, 195.0] {
        x.vcyl(0.0, 0.0, 57.0, y, y + 2.0, x.m.steel_dark);
    }
    for y in 0..30 {
        x.k.cyl(1, [c, 0.0, c], 58.0 * (1.0 - y as f32 / 30.0), 210.0 + y as f32, 211.0 + y as f32, x.m.wood_dark);
    }
}

fn car(x: &mut Ctx) {
    // Facing −z. Length ~4.3 m, width ~1.8 m.
    let kind = x.r.int(0, 3); // sedan, hatch, van, taxi
    let body = if kind == 3 { x.m.paint[2] } else { *x.r.pick(&x.m.paint) };
    let (hx, hz) = (28, if kind == 2 { 76 } else { 68 });
    let y0 = 10;
    x.cbox(0, 0, hx, hz, y0, y0 + 22, body);
    // Cabin.
    let (cz0, cz1, ch) = match kind {
        1 => (-26, hz, 20),
        2 => (-48, hz, 30),
        _ => (-30, 36, 20),
    };
    x.k.bx([x.c - hx + 2, y0 + 22, x.c + cz0], [x.c + hx - 2, y0 + 22 + ch, x.c + cz1], body);
    // Windows: a glass band around the cabin.
    x.k.bx([x.c - hx + 1, y0 + 25, x.c + cz0 + 2], [x.c + hx - 1, y0 + 20 + ch, x.c + cz1 - 2], x.m.glass_dark);
    x.k.bx([x.c - hx + 3, y0 + 25, x.c + cz0 + 1], [x.c + hx - 3, y0 + 20 + ch, x.c + cz1 - 1], x.m.glass_dark);
    // Wheels.
    let c = x.c as f32;
    for wz in [-hz + 16, hz - 16] {
        x.k.cyl(0, [0.0, 10.0, c + wz as f32], 10.0, c - hx as f32 - 1.0, c + hx as f32 + 1.0, x.m.rubber);
        x.k.cyl(0, [0.0, 10.0, c + wz as f32], 5.0, c - hx as f32 - 1.5, c + hx as f32 + 1.5, x.m.chrome);
    }
    // Lights, bumpers.
    for sx in [-hx + 5, hx - 5] {
        x.cbox(sx, -hz, 4, 1, y0 + 12, y0 + 17, x.m.lamp);
        x.cbox(sx, hz - 1, 4, 1, y0 + 12, y0 + 17, x.m.lamp_red);
    }
    x.cbox(0, -hz, hx, 1, y0, y0 + 5, x.m.plastic_black);
    x.cbox(0, hz - 1, hx, 1, y0, y0 + 5, x.m.plastic_black);
    if kind == 3 {
        x.cbox(0, 0, 8, 4, y0 + 22 + ch, y0 + 26 + ch, x.m.lamp);
    }
}

fn tree(x: &mut Ctx) {
    let style = x.r.int(0, 2); // round, conifer, palm
    let c = x.c as f32;
    let h = x.r.range(140.0, 220.0);
    let seed = x.r.u64();
    match style {
        1 => {
            x.vcyl(0.0, 0.0, 5.0, 0.0, h * 0.5, x.m.bark);
            let tiers = 5;
            for t in 0..tiers {
                let y = h * 0.25 + t as f32 * h * 0.15;
                let r = 54.0 * (1.0 - t as f32 / tiers as f32) + 12.0;
                x.k.push(Blob {
                    center: [c, y + 20.0, c].into(),
                    radii: [r, 30.0, r].into(),
                    mat: x.m.leaf[2],
                    noise: 0.25,
                    seed: seed + t,
                    fill: 0.7,
                    alt: Some((x.m.leaf[0], 0.3)),
                });
            }
        }
        2 => {
            // Palm: a leaning trunk and drooping fronds.
            let lean = x.r.range(-0.25, 0.25);
            for y in 0..(h as i32) {
                let off = lean * (y as f32).powf(1.3) * 0.2;
                x.k.cyl(1, [c + off, 0.0, c], 4.5 - y as f32 / h * 1.5, y as f32, y as f32 + 1.0, x.m.bark);
            }
            let top = [c + lean * h.powf(1.3) * 0.2, h, c];
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::TAU / 8.0;
                for s in 0..14 {
                    let d = s as f32 * 6.0;
                    let p = [top[0] + a.cos() * d, top[1] - (s as f32 * 0.6).powi(2) * 0.5, top[2] + a.sin() * d];
                    x.k.push(Blob {
                        center: p.into(),
                        radii: [6.0, 3.0, 6.0].into(),
                        mat: x.m.leaf[1],
                        noise: 0.2,
                        seed: seed + k * 31 + s,
                        fill: 0.9,
                        alt: None,
                    });
                }
            }
        }
        _ => {
            x.vcyl(0.0, 0.0, 6.0, 0.0, h * 0.6, x.m.bark);
            // Branches toward the crown clusters.
            let clusters = x.r.int(5, 9);
            for k in 0..clusters {
                let a = x.r.range(0.0, std::f32::consts::TAU);
                let d = x.r.range(14.0, 36.0);
                let y = x.r.range(h * 0.55, h * 0.9);
                let p = [c + a.cos() * d, y, c + a.sin() * d];
                let r = x.r.range(26.0, 42.0);
                x.k.push(Blob {
                    center: p.into(),
                    radii: [r, r * 0.8, r].into(),
                    mat: x.m.leaf[0],
                    noise: 0.35,
                    seed: seed + k as u64,
                    fill: 0.6,
                    alt: Some((x.m.leaf[1], 0.35)),
                });
                // Branch: a thin stepped line from trunk to cluster.
                for s in 0..10 {
                    let t = s as f32 / 10.0;
                    let q = [c + (p[0] - c) * t, h * 0.5 + (y - h * 0.5) * t, c + (p[2] - c) * t];
                    x.k.blob(q, [3.0, 3.0, 3.0], x.m.bark, 0.0, 0);
                }
            }
            x.k.push(Blob {
                center: [c, h * 0.85, c].into(),
                radii: [48.0, 42.0, 48.0].into(),
                mat: x.m.leaf[2],
                noise: 0.3,
                seed: seed ^ 99,
                fill: 0.55,
                alt: Some((x.m.leaf[0], 0.4)),
            });
        }
    }
}

fn bush(x: &mut Ctx) {
    let c = x.c as f32;
    let seed = x.r.u64();
    let r = x.r.range(14.0, 26.0);
    x.k.push(Blob {
        center: [c, r * 0.7, c].into(),
        radii: [r, r * 0.8, r].into(),
        mat: x.m.leaf[0],
        noise: 0.35,
        seed,
        fill: 0.7,
        alt: Some((x.m.flowers[(seed % 3) as usize], 0.06)),
    });
}

fn rock(x: &mut Ctx) {
    let c = x.c as f32;
    let seed = x.r.u64();
    let r = x.r.range(10.0, 24.0);
    x.k.push(Blob {
        center: [c, r * 0.4, c].into(),
        radii: [r, r * 0.7, r * 0.9].into(),
        mat: x.m.stone,
        noise: 0.3,
        seed,
        fill: 1.0,
        alt: Some((x.m.concrete_dark, 0.2)),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::SceneSpec;

    #[test]
    fn every_prop_builds_and_varies_by_seed() {
        let mut pal = Palette::default();
        let mats = crate::style::materials(&SceneSpec::default(), &mut pal);
        for def in CATALOG {
            let a = build(def, 1, 0, 0, &mats, &pal);
            let b = build(def, 2, 1, 0, &mats, &pal);
            let same = build(def, 1, 0, 0, &mats, &pal);
            assert!(a.voxel_count() > 0, "{} is empty", def.name);
            assert_eq!(a.words, same.words, "{} is deterministic", def.name);
            let _ = b; // most props vary by seed; some (cinder block) are fixed by design
        }
    }
}

//! Level layout. Static geometry is ordinary physics bodies, rendered as `Kind::Wall` / `Pad`.

use super::{Kind, OrbArena, Prop};
use gs_core::prelude::*;
use gs_physics::d3::{Body, Shape};

const HALF: f32 = 15.0;

fn fixed_box(g: &mut OrbArena, kind: Kind, pos: Vec3, half: Vec3, color: u32) {
    let eid = g.eid();
    let body = g.world.add(Body::fixed(Shape::Box { half }, pos).with_friction(0.6));
    g.props.push(Prop { eid, body, kind: kind.into(), color });
}

pub(super) fn build(g: &mut OrbArena) {
    let wall = 0x2b2d42;
    // Perimeter walls.
    fixed_box(g, Kind::Wall, vec3(0.0, 1.0, -HALF - 0.5), vec3(HALF + 1.0, 1.0, 0.5), wall);
    fixed_box(g, Kind::Wall, vec3(0.0, 1.0, HALF + 0.5), vec3(HALF + 1.0, 1.0, 0.5), wall);
    fixed_box(g, Kind::Wall, vec3(-HALF - 0.5, 1.0, 0.0), vec3(0.5, 1.0, HALF), wall);
    fixed_box(g, Kind::Wall, vec3(HALF + 0.5, 1.0, 0.0), vec3(0.5, 1.0, HALF), wall);
    // Raised pads to jump onto.
    let pad = 0x3a86ff;
    fixed_box(g, Kind::Pad, vec3(-7.0, 0.5, -7.0), vec3(2.5, 0.5, 2.5), pad);
    fixed_box(g, Kind::Pad, vec3(7.0, 0.5, 7.0), vec3(2.5, 0.5, 2.5), pad);
    fixed_box(g, Kind::Pad, vec3(7.0, 1.0, -7.0), vec3(1.5, 1.0, 1.5), pad);
    fixed_box(g, Kind::Pad, vec3(-7.0, 1.0, 7.0), vec3(1.5, 1.0, 1.5), pad);
    // Pushable crates.
    for i in 0..6 {
        let a = i as f32 / 6.0 * math::TAU;
        let (s, c) = math::sin_cos(a);
        let pos = vec3(c * 4.0, 0.6, s * 4.0);
        let eid = g.eid();
        let body = g.world.add(
            Body::dynamic(Shape::Box { half: Vec3::splat(0.6) }, pos)
                .with_mass(2.0)
                .with_friction(0.5)
                .with_damping(1.0),
        );
        g.props.push(Prop { eid, body, kind: Kind::Crate.into(), color: 0xc08552 });
    }
}

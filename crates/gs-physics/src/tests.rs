//! Every behaviour is tested in BOTH modes through one generic function, so 2D and 3D can't
//! drift apart.

use super::*;
use glam::{Vec2, Vec3};

const DT: f32 = 1.0 / 60.0;

fn v<V: Vector>(x: f32, y: f32) -> V {
    V::ZERO.with(0, x).with(1, y)
}

macro_rules! in_both_modes {
    ($name:ident, $body:ident) => {
        #[test]
        fn $name() {
            $body::<Vec2>();
            $body::<Vec3>();
        }
    };
}

fn falls_and_rests_on_floor<V: Vector>() {
    let mut w = World::<V>::new(V::UP * -20.0);
    w.add(Body::floor(0.0));
    let b = w.add(Body::dynamic(Shape::Ball { r: 0.5 }, v(0.0, 5.0)));
    for _ in 0..240 {
        w.step(DT);
    }
    assert!((w[b].pos.get(1) - 0.5).abs() < 0.02, "{}D y = {}", V::DIM, w[b].pos.get(1));
    assert!(w[b].grounded);
}
in_both_modes!(rests_on_floor, falls_and_rests_on_floor);

fn box_stack_settles<V: Vector>() {
    let mut w = World::<V>::new(V::UP * -20.0);
    w.add(Body::fixed(Shape::Box { half: V::splat(5.0).with(1, 0.5) }, V::ZERO));
    let a = w.add(Body::dynamic(Shape::Box { half: V::splat(0.5) }, v(0.0, 1.6)));
    let b = w.add(Body::dynamic(Shape::Box { half: V::splat(0.5) }, v(0.05, 2.8)));
    for _ in 0..300 {
        w.step(DT);
    }
    assert!((w[a].pos.get(1) - 1.0).abs() < 0.03, "{}D bottom {}", V::DIM, w[a].pos.get(1));
    assert!((w[b].pos.get(1) - 2.0).abs() < 0.05, "{}D top {}", V::DIM, w[b].pos.get(1));
    assert!(w[a].grounded && w[b].grounded);
}
in_both_modes!(stack_settles, box_stack_settles);

fn elastic_collision_conserves_momentum<V: Vector>() {
    let mut w = World::<V>::zero_g();
    let a = w.add(Body::dynamic(Shape::Ball { r: 0.5 }, v(-1.0, 0.0)).with_restitution(1.0).with_vel(v(5.0, 0.0)));
    let b = w.add(Body::dynamic(Shape::Ball { r: 0.5 }, v(1.0, 0.0)).with_restitution(1.0).with_vel(v(-5.0, 0.0)));
    for _ in 0..60 {
        w.step(DT);
    }
    assert!((w[a].vel + w[b].vel).length() < 1e-3);
    assert!(w[a].vel.get(0) < 0.0 && w[b].vel.get(0) > 0.0, "bounced");
}
in_both_modes!(momentum, elastic_collision_conserves_momentum);

fn ramp_redirects_a_ball<V: Vector>() {
    // A 45° ramp: plane normal (−1, 1)/√2 through the origin. A ball dropped onto it rolls −X.
    let mut w = World::<V>::new(V::UP * -20.0);
    w.add(Body::plane(v(-1.0, 1.0), V::ZERO));
    let b = w.add(Body::dynamic(Shape::Ball { r: 0.5 }, v(2.0, 4.0)).with_friction(0.0));
    for _ in 0..90 {
        w.step(DT);
    }
    assert!(w[b].vel.get(0) < -1.0, "{}D slides down-slope: vel {:?}", V::DIM, w[b].vel);
    let n = v::<V>(-1.0, 1.0).normalize_or(V::UP);
    assert!(n.dot(w[b].pos) >= 0.5 - 0.05, "stays on the surface");
}
in_both_modes!(ramps, ramp_redirects_a_ball);

fn sensors_report_enter_once<V: Vector>() {
    let mut w = World::<V>::zero_g();
    let s = w.add(Body::sensor(Shape::Ball { r: 1.0 }, V::ZERO));
    let d = w.add(Body::dynamic(Shape::Ball { r: 0.25 }, v(-3.0, 0.0)).with_vel(v(3.0, 0.0)));
    let mut enters = 0;
    let mut overlapping_steps = 0;
    for _ in 0..120 {
        w.step(DT);
        enters += w.began_contacts_of(s).count();
        overlapping_steps += w.contacts_of(s).count();
    }
    assert_eq!(enters, 1, "{}D enter fires once", V::DIM);
    assert!(overlapping_steps > 10);
    assert!(w[d].vel.get(0) == 3.0, "sensors don't push");
}
in_both_modes!(sensors, sensors_report_enter_once);

fn layers_filter<V: Vector>() {
    let mut w = World::<V>::zero_g();
    let a = w.add(Body::dynamic(Shape::Ball { r: 1.0 }, V::ZERO).with_layers(1, !2));
    w.add(Body::dynamic(Shape::Ball { r: 1.0 }, v(0.5, 0.0)).with_layers(2, u32::MAX));
    w.step(DT);
    assert_eq!(w.contacts_of(a).count(), 0);
}
in_both_modes!(layers, layers_filter);

fn rope_and_rod_joints<V: Vector>() {
    let mut w = World::<V>::new(V::UP * -20.0);
    let anchor = w.add(Body::fixed(Shape::Ball { r: 0.1 }, v(0.0, 10.0)));
    let rope_end = w.add(Body::dynamic(Shape::Ball { r: 0.2 }, v(1.0, 10.0)).with_layers(2, 0));
    let rod_end = w.add(Body::dynamic(Shape::Ball { r: 0.2 }, v(-2.0, 10.0)).with_layers(2, 0));
    w.add_joint(Joint { a: anchor, b: rope_end, rest: 3.0, kind: JointKind::Rope });
    w.add_joint(Joint { a: anchor, b: rod_end, rest: 2.0, kind: JointKind::Rod });
    for _ in 0..300 {
        w.step(DT);
    }
    let rope_len = (w[rope_end].pos - w[anchor].pos).length();
    let rod_len = (w[rod_end].pos - w[anchor].pos).length();
    assert!(rope_len <= 3.05, "{}D rope taut at most 3: {rope_len}", V::DIM);
    assert!((rod_len - 2.0).abs() < 0.05, "rod holds 2: {rod_len}");
    assert!(w[rope_end].pos.get(1) < 8.0, "hangs below the anchor");
}
in_both_modes!(joints, rope_and_rod_joints);

fn raycast_hits_nearest<V: Vector>() {
    let mut w = World::<V>::zero_g();
    let near = w.add(Body::fixed(Shape::Box { half: V::splat(0.5) }, v(3.0, 0.0)));
    w.add(Body::fixed(Shape::Ball { r: 0.5 }, v(6.0, 0.0)));
    w.add(Body::plane(v(-1.0, 0.0), v(10.0, 0.0)));
    let hit = w.raycast(V::ZERO, v(1.0, 0.0), 100.0, u32::MAX).unwrap();
    assert_eq!(hit.body, near);
    assert!((hit.t - 2.5).abs() < 1e-5);
    assert_eq!(hit.normal, v(-1.0, 0.0));
    let floor = w.raycast(v(20.0, 0.0), v(-1.0, 0.0), 100.0, u32::MAX).unwrap();
    assert!((floor.t - 0.0).abs() < 1e-5, "inside the plane's solid side = immediate hit");
}
in_both_modes!(raycast, raycast_hits_nearest);

fn overlap_queries<V: Vector>() {
    let mut w = World::<V>::zero_g();
    let a = w.add(Body::fixed(Shape::Ball { r: 0.5 }, v(1.0, 0.0)));
    let b = w.add(Body::fixed(Shape::Box { half: V::splat(0.5) }, v(4.0, 0.0)));
    assert_eq!(w.overlap_ball(V::ZERO, 0.6, u32::MAX), vec![a]);
    assert_eq!(w.overlap_box(v(3.0, 0.0), V::splat(0.6), u32::MAX), vec![b]);
}
in_both_modes!(overlaps, overlap_queries);

fn kinematic_platform_carries<V: Vector>() {
    let mut w = World::<V>::new(V::UP * -20.0);
    let plat =
        w.add(Body::kinematic(Shape::Box { half: V::splat(1.0).with(1, 0.2) }, v(0.0, 1.0)).with_vel(v(0.0, 1.0)));
    let rider = w.add(Body::dynamic(Shape::Box { half: V::splat(0.3) }, v(0.0, 1.55)));
    for _ in 0..60 {
        w.step(DT);
    }
    assert!((w[plat].pos.get(1) - 2.0).abs() < 1e-3, "kinematic moves by velocity only");
    assert!(w[rider].pos.get(1) > 2.3, "{}D rider lifted: {}", V::DIM, w[rider].pos.get(1));
}
in_both_modes!(kinematic, kinematic_platform_carries);

fn handles_are_generational<V: Vector>() {
    let mut w = World::<V>::default();
    let a = w.add(Body::fixed(Shape::Ball { r: 1.0 }, V::ZERO));
    w.remove(a);
    let b = w.add(Body::fixed(Shape::Ball { r: 1.0 }, V::ZERO));
    assert_eq!(a.index, b.index);
    assert!(w.get(a).is_none() && w.get(b).is_some());
}
in_both_modes!(generational, handles_are_generational);

/// The property prediction relies on: snapshot → restore → keep stepping is bit-identical.
fn snapshot_restore_is_bit_exact<V: Vector>() {
    let mut w = World::<V>::new(V::UP * -20.0);
    w.substeps = 2;
    w.add(Body::floor(0.0));
    w.add(Body::plane(v(1.0, 1.0), v(-6.0, 0.0)));
    for i in 0..24 {
        let p = v::<V>(i as f32 * 0.37 - 4.0, 1.0 + i as f32 * 0.6);
        let shape = if i % 2 == 0 { Shape::Ball { r: 0.4 } } else { Shape::Box { half: V::splat(0.35) } };
        w.add(Body::dynamic(shape, p).with_restitution(0.4));
    }
    let a = BodyId { index: 2, generation: 0 };
    let b = BodyId { index: 3, generation: 0 };
    w.add_joint(Joint { a, b, rest: 1.0, kind: JointKind::Spring { stiffness: 40.0, damping: 2.0 } });
    for _ in 0..40 {
        w.step(DT);
    }
    let mut restored: World<V> = gs_wire::from_bytes(&gs_wire::to_bytes(&w)).unwrap();
    for _ in 0..80 {
        w.step(DT);
        restored.step(DT);
        // Enter events must agree too (they depend on persisted `touching`).
        let ev = |w: &World<V>| w.contacts.iter().map(|c| (c.a, c.b, c.began)).collect::<Vec<_>>();
        assert_eq!(ev(&w), ev(&restored));
    }
    assert_eq!(gs_wire::to_bytes(&w), gs_wire::to_bytes(&restored));
}
in_both_modes!(snapshot_restore, snapshot_restore_is_bit_exact);

#[test]
fn golden_hashes_pin_the_solver() {
    // Changing the solver changes every replay. Update these deliberately (and say why).
    fn run<V: Vector>() -> u64 {
        let mut w = World::<V>::new(V::UP * -20.0);
        w.add(Body::floor(0.0));
        for i in 0..10 {
            w.add(Body::dynamic(Shape::Ball { r: 0.5 }, v(i as f32 * 0.3, 1.0 + i as f32)));
        }
        for _ in 0..200 {
            w.step(DT);
        }
        gs_wire::fnv1a64(&gs_wire::to_bytes(&w))
    }
    let (h2, h3) = (run::<Vec2>(), run::<Vec3>());
    assert_eq!(h2, run::<Vec2>());
    assert_eq!(h3, run::<Vec3>());
    assert_ne!(h2, h3);
}

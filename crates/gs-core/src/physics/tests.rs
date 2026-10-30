use super::*;

const DT: f32 = 1.0 / 60.0;

#[test]
fn falls_and_rests_on_floor() {
    let mut w = World::new(Vec3::new(0.0, -20.0, 0.0));
    w.floor = Some(0.0);
    let b = w.add(Body::dynamic(Shape::Sphere { r: 0.5 }, Vec3::new(0.0, 5.0, 0.0)));
    for _ in 0..240 {
        w.step(DT);
    }
    assert!((w[b].pos.y - 0.5).abs() < 0.02, "y = {}", w[b].pos.y);
    assert!(w[b].grounded);
}

#[test]
fn rests_on_static_box() {
    let mut w = World::new(Vec3::new(0.0, -20.0, 0.0));
    w.add(Body::fixed(Shape::Box { half: Vec3::new(5.0, 0.5, 5.0) }, Vec3::ZERO));
    let b = w.add(Body::dynamic(Shape::Box { half: Vec3::splat(0.5) }, Vec3::new(0.0, 3.0, 0.0)));
    for _ in 0..240 {
        w.step(DT);
    }
    assert!((w[b].pos.y - 1.0).abs() < 0.03, "y = {}", w[b].pos.y);
    assert!(w[b].grounded);
}

#[test]
fn spheres_push_apart_and_conserve_momentum() {
    let mut w = World::new(Vec3::ZERO);
    let a = w.add(Body::dynamic(Shape::Sphere { r: 0.5 }, Vec3::new(-1.0, 0.0, 0.0)).with_restitution(1.0));
    let b = w.add(Body::dynamic(Shape::Sphere { r: 0.5 }, Vec3::new(1.0, 0.0, 0.0)).with_restitution(1.0));
    w[a].vel = Vec3::new(5.0, 0.0, 0.0);
    w[b].vel = Vec3::new(-5.0, 0.0, 0.0);
    for _ in 0..60 {
        w.step(DT);
    }
    let p = w[a].vel + w[b].vel;
    assert!(p.length() < 1e-3);
    assert!(w[a].vel.x < 0.0 && w[b].vel.x > 0.0, "bounced");
}

#[test]
fn sensors_report_but_do_not_push() {
    let mut w = World::new(Vec3::ZERO);
    let s = w.add(Body::sensor(Shape::Sphere { r: 1.0 }, Vec3::ZERO));
    let d = w.add(Body::dynamic(Shape::Sphere { r: 0.5 }, Vec3::new(0.5, 0.0, 0.0)));
    w.step(DT);
    assert_eq!(w.contacts_of(s).count(), 1);
    assert_eq!(w[d].pos, Vec3::new(0.5, 0.0, 0.0));
}

#[test]
fn layers_filter() {
    let mut w = World::new(Vec3::ZERO);
    let a = w.add(Body::dynamic(Shape::Sphere { r: 1.0 }, Vec3::ZERO).with_layers(1, !2));
    let _b = w.add(Body::dynamic(Shape::Sphere { r: 1.0 }, Vec3::new(0.5, 0.0, 0.0)).with_layers(2, u32::MAX));
    w.step(DT);
    assert_eq!(w.contacts_of(a).count(), 0);
}

#[test]
fn raycast_hits_nearest() {
    let mut w = World::new(Vec3::ZERO);
    let near = w.add(Body::fixed(Shape::Box { half: Vec3::splat(0.5) }, Vec3::new(0.0, 0.0, -3.0)));
    w.add(Body::fixed(Shape::Sphere { r: 0.5 }, Vec3::new(0.0, 0.0, -6.0)));
    let hit = w.raycast(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0), 100.0, u32::MAX).unwrap();
    assert_eq!(hit.body, near);
    assert!((hit.t - 2.5).abs() < 1e-5);
    assert_eq!(hit.normal, Vec3::Z);
}

#[test]
fn handles_are_generational() {
    let mut w = World::default();
    let a = w.add(Body::fixed(Shape::Sphere { r: 1.0 }, Vec3::ZERO));
    w.remove(a);
    let b = w.add(Body::fixed(Shape::Sphere { r: 1.0 }, Vec3::ZERO));
    assert_eq!(a.index, b.index);
    assert!(w.get(a).is_none());
    assert!(w.get(b).is_some());
}

#[test]
fn snapshot_restore_is_bit_exact() {
    let mut w = World::new(Vec3::new(0.0, -20.0, 0.0));
    w.floor = Some(0.0);
    for i in 0..20 {
        let b = Body::dynamic(Shape::Sphere { r: 0.4 }, Vec3::new(i as f32 * 0.3, 1.0 + i as f32, 0.1 * i as f32));
        w.add(b);
    }
    for _ in 0..30 {
        w.step(DT);
    }
    let bytes = gs_wire::to_bytes(&w);
    let mut restored: World = gs_wire::from_bytes(&bytes).unwrap();
    for _ in 0..60 {
        w.step(DT);
        restored.step(DT);
    }
    assert_eq!(gs_wire::to_bytes(&w), gs_wire::to_bytes(&restored));
}

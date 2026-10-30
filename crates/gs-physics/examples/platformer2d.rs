//! 2D mode, headless: a little platformer scene with a ramp, platforms, a rope bridge made of
//! joints, sensor coins and a moving platform — printed as ASCII.
//!
//!     cargo run -p gs-physics --example platformer2d

use gs_physics::d2::*;

const DT: f32 = 1.0 / 60.0;

fn main() {
    let mut w = World::new(vec2(0.0, -30.0));
    w.substeps = 2;
    w.add(Body::floor(0.0));
    // A 30° ramp on the left: a plane whose solid side is under the slope.
    w.add(Body::plane(vec2(0.5, 0.866), vec2(-14.0, 0.0)));
    // A wall on the right (another plane, facing left).
    w.add(Body::plane(vec2(-1.0, 0.0), vec2(14.0, 0.0)));
    // Platforms.
    w.add(Body::fixed(Shape::Box { half: vec2(2.0, 0.3) }, vec2(-4.0, 3.0)));
    w.add(Body::fixed(Shape::Box { half: vec2(1.0, 0.3) }, vec2(9.0, 3.0)));
    // A moving (kinematic) platform.
    let lift = w.add(Body::kinematic(Shape::Box { half: vec2(1.0, 0.2) }, vec2(4.0, 1.0)).with_vel(vec2(0.0, 1.0)));
    // A rope bridge: planks joined by rods, ends tied to posts with ropes.
    let post_l = w.add(Body::fixed(Shape::Box { half: vec2(0.2, 0.2) }, vec2(-2.0, 5.0)));
    let post_r = w.add(Body::fixed(Shape::Box { half: vec2(0.2, 0.2) }, vec2(3.0, 5.0)));
    let mut prev = post_l;
    for i in 0..6 {
        let plank = w
            .add(Body::dynamic(Shape::Box { half: vec2(0.3, 0.08) }, vec2(-1.3 + i as f32 * 0.72, 5.0)).with_mass(0.3));
        let kind = if i == 0 { JointKind::Rope } else { JointKind::Rod };
        w.add_joint(Joint { a: prev, b: plank, rest: 0.72, kind });
        prev = plank;
    }
    w.add_joint(Joint { a: prev, b: post_r, rest: 0.72, kind: JointKind::Rope });
    // Coins (sensors) and the hero.
    let coins: Vec<BodyId> = [vec2(-5.0, 0.6), vec2(1.0, 0.6), vec2(12.5, 0.6)]
        .into_iter()
        .map(|p| w.add(Body::sensor(Shape::Ball { r: 0.3 }, p)))
        .collect();
    let hero = w.add(Body::dynamic(Shape::Ball { r: 0.4 }, vec2(-10.0, 6.0)).with_friction(0.1));
    let crates: Vec<BodyId> =
        (0..3).map(|i| w.add(Body::dynamic(Shape::Box { half: vec2(0.4, 0.4) }, vec2(6.0, 0.5 + i as f32)))).collect();

    let mut collected = 0;
    for tick in 0..600 {
        // Hero AI: run right; jump when a ray sees something in the way (or bounce off the wall).
        let ahead = w.raycast(w[hero].pos, vec2(1.0, 0.0), 1.2, u32::MAX).is_some_and(|hit| hit.body != hero);
        let h = &mut w[hero];
        h.vel.x = 5.0;
        if h.grounded && ahead {
            h.vel.y = 13.0;
        }
        // The lift reverses at its ends.
        let l = &mut w[lift];
        if l.pos.y > 4.0 || l.pos.y < 1.0 {
            l.vel.y = -l.vel.y;
        }
        w.step(DT);
        for c in &coins {
            if w.contains(*c) && w.began_contacts_of(*c).any(|(o, _)| o == hero) {
                w.remove(*c);
                collected += 1;
                println!("tick {tick}: coin! ({collected})");
            }
        }
        if tick % 150 == 0 {
            draw(&w, hero, &crates);
        }
    }
    draw(&w, hero, &crates);
    println!("coins collected: {collected}");
}

/// ASCII view: x ∈ [-15, 15), y ∈ [-0.5, 8.5).
fn draw(w: &World, hero: BodyId, crates: &[BodyId]) {
    let (cols, rows) = (60usize, 18usize);
    let cell = |r: usize, c: usize| {
        let p = vec2(-15.0 + c as f32 * 0.5 + 0.25, 8.5 - r as f32 * 0.5 - 0.25);
        match w.overlap_ball(p, 0.2, u32::MAX).first() {
            None => ' ',
            Some(id) if *id == hero => '@',
            Some(id) if crates.contains(id) => '#',
            Some(id) => match w[*id].kind {
                BodyKind::Sensor => '$',
                BodyKind::Kinematic => '=',
                BodyKind::Dynamic => '~',
                BodyKind::Static => '█',
            },
        }
    };
    let grid: Vec<String> = (0..rows).map(|r| (0..cols).map(|c| cell(r, c)).collect()).collect();
    println!("+{}+", "-".repeat(cols));
    for row in grid {
        println!("|{row}|");
    }
    println!("+{}+", "-".repeat(cols));
}

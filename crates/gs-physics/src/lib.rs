//! # gs-physics — optional deterministic physics, 2D and 3D
//!
//! One engine, two modes. Every algorithm is written once against [`Vector`] and runs as
//! `World<Vec2>` (2D: platformers, top-down, pinball) or `World<Vec3>` (3D: arenas, karts).
//! Pick a mode by importing [`d2`] or [`d3`]. Nothing in game-stack requires this crate. Use
//! it, replace it, or write your own movement code.
//!
//! - Shapes: balls (circle/sphere), axis-aligned boxes, static planes (floors, walls, ramps).
//! - Bodies: dynamic, static, kinematic (moving platforms), sensor (triggers, pickups).
//! - Impulse solver with restitution + Coulomb friction, layers/masks, substeps.
//! - Joints: rod, rope, spring.
//! - Contacts with `began` (enter) flags, ray casts, ball/box overlap queries.
//! - **Deterministic across native and wasm**: only IEEE-exact float ops, index-order
//!   iteration, sorted pairs. The whole world derives `Wire`, so it lives in your game state
//!   and is snapshotted, predicted and rolled back like everything else.
//! - Cheap to clone (flat `Vec`s): prediction re-simulates by cloning it.
//!
//! No rotation dynamics: boxes stay axis-aligned. That keeps the solver small, stable and
//! readable, and covers most jam games. For tumbling rigid bodies, see `docs/RECIPES.md`.
//!
//! ```
//! use gs_physics::d2::*;
//! let mut w = World::new(Vec2::new(0.0, -30.0));
//! w.add(Body::floor(0.0));
//! let ball = w.add(Body::dynamic(Shape::Ball { r: 0.5 }, Vec2::new(0.0, 4.0)));
//! for _ in 0..120 { w.step(1.0 / 60.0); }
//! assert!(w[ball].grounded);
//! ```
//!
//! ```
//! use gs_physics::d3::*;
//! let mut w = World::default(); // gravity -9.81 on Y
//! w.add(Body::floor(0.0));
//! let crate_ = w.add(Body::dynamic(Shape::Box { half: Vec3::splat(0.5) }, Vec3::new(0.0, 3.0, 0.0)));
//! for _ in 0..240 { w.step(1.0 / 60.0); }
//! assert!((w[crate_].pos.y - 0.5).abs() < 0.02);
//! ```

mod body;
mod collide;
mod vector;
mod world;

pub use body::{Body, BodyId, BodyKind, Shape};
pub use collide::{Contact, RayHit};
pub use vector::Vector;
pub use world::{Joint, JointId, JointKind, World};

/// 2D mode: everything specialised to `Vec2` (x right, y up).
pub mod d2 {
    pub use crate::{BodyId, BodyKind, Joint, JointId, JointKind};
    pub use glam::{Vec2, vec2};
    pub type World = crate::World<Vec2>;
    pub type Body = crate::Body<Vec2>;
    pub type Shape = crate::Shape<Vec2>;
    pub type Contact = crate::Contact<Vec2>;
    pub type RayHit = crate::RayHit<Vec2>;
}

/// 3D mode: everything specialised to `Vec3` (y up, -z forward — three.js convention).
pub mod d3 {
    pub use crate::{BodyId, BodyKind, Joint, JointId, JointKind};
    pub use glam::{Vec3, vec3};
    pub type World = crate::World<Vec3>;
    pub type Body = crate::Body<Vec3>;
    pub type Shape = crate::Shape<Vec3>;
    pub type Contact = crate::Contact<Vec3>;
    pub type RayHit = crate::RayHit<Vec3>;
}

#[cfg(test)]
mod tests;

/// Determinism probe: a stress scene using every feature (planes, boxes, balls, joints,
/// sensors, kinematics, substeps). Its hash must match natively and in wasm (`pnpm verify`).
pub fn probe<V: Vector>(ticks: u32) -> u64 {
    let v = |x: f32, y: f32| V::ZERO.with(0, x).with(1, y);
    let mut w = World::<V>::new(V::UP * -25.0);
    w.substeps = 2;
    w.add(Body::floor(0.0));
    w.add(Body::plane(v(1.0, 0.6), v(-8.0, 0.0)));
    w.add(Body::plane(v(-1.0, 0.0), v(8.0, 0.0)));
    w.add(Body::kinematic(Shape::Box { half: V::splat(1.0).with(1, 0.2) }, v(2.0, 1.0)).with_vel(v(0.3, 0.5)));
    w.add(Body::sensor(Shape::Ball { r: 1.0 }, v(0.0, 2.0)));
    let mut prev = None;
    for i in 0..30 {
        let p = v(-6.0 + (i % 10) as f32 * 1.3, 1.0 + (i / 10) as f32 * 1.7 + (i % 3) as f32 * 0.21);
        let shape = if i % 3 == 0 {
            Shape::Box { half: V::splat(0.4) }
        } else {
            Shape::Ball { r: 0.35 + (i % 4) as f32 * 0.05 }
        };
        let id = w.add(
            Body::dynamic(shape, p).with_restitution(0.3).with_friction(0.5).with_vel(v(1.5 - i as f32 * 0.1, 0.0)),
        );
        if let Some(p) = prev
            && i % 5 != 0
        {
            w.add_joint(Joint { a: p, b: id, rest: 1.2, kind: JointKind::Spring { stiffness: 30.0, damping: 1.0 } });
        }
        prev = Some(id);
    }
    for _ in 0..ticks {
        w.step(1.0 / 60.0);
    }
    gs_wire::fnv1a64(&gs_wire::to_bytes(&w))
}

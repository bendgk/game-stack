//! # gs-core — the game-stack engine core
//!
//! Compiled twice from one source: natively into the authoritative server and to wasm32 for
//! the browser's predictor. Nothing in here does I/O.
//!
//! | module      | what                                                                |
//! |-------------|---------------------------------------------------------------------|
//! | [`game`]    | the `Game` trait you implement, frames, events, results             |
//! | [`math`]    | deterministic math (glam + libm), helpers                           |
//! | [`rng`]     | deterministic PRNG                                                  |
//! | [`physics`] | small deterministic physics world (spheres/boxes, sensors, rays)   |
//! | [`render`]  | the flat render list the renderer consumes                          |
//! | [`net`]     | wire protocol, authority, predictor, delta, room host, link sim    |
//! | [`env`]     | headless gym-style environment, bots, determinism probe             |

pub mod env;
pub mod game;
pub mod math;
pub mod net;
pub mod physics;
pub mod render;
pub mod rng;
pub mod testkit;

pub use game::{Events, Frame, FrameOf, Game, Join, MatchResult, PlayerId, Setup};
pub use gs_wire::{self as wire, Wire};
pub use render::{RenderEntity, RenderView};
pub use rng::Rng;

/// Everything a game file usually needs.
pub mod prelude {
    pub use crate::game::{Events, FrameOf, Game, Join, MatchResult, PlayerId, Setup};
    pub use crate::math::{self, Quat, Vec2, Vec3, vec2, vec3};
    pub use crate::physics::{Body, BodyId, BodyKind, Shape, World};
    pub use crate::render::{FLAG_HIDDEN, FLAG_ME, FLAG_TELEPORT, FLAG_USER, RenderEntity, RenderView};
    pub use crate::rng::Rng;
    pub use gs_wire::Wire;
}

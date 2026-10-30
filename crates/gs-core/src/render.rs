//! The render contract between the sim and the renderer.
//!
//! A game fills a [`RenderView`] with flat [`RenderEntity`] records each frame; the client
//! interpolates them between ticks, adds correction smoothing, and exposes them to three.js as a
//! zero-copy `Float32Array` (see `RenderEntity::STRIDE`). React decides *how* a kind looks;
//! Rust decides *where* it is. The renderer never reads game state directly.

use crate::math::{Quat, Vec2, Vec3};

/// Entity is the local player (cameras follow it).
pub const FLAG_ME: u16 = 1 << 0;
/// Hidden this frame (keeps its mesh mounted).
pub const FLAG_HIDDEN: u16 = 1 << 1;
/// Teleported: skip interpolation and smoothing for this frame.
pub const FLAG_TELEPORT: u16 = 1 << 2;
/// First game-defined flag bit.
pub const FLAG_USER: u16 = 1 << 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderEntity {
    /// Stable id across frames (used for interpolation, smoothing and React keys).
    pub id: u32,
    /// Index into the game's `Kind` enum; the renderer maps it to a component.
    pub kind: u16,
    pub flags: u16,
    pub pos: Vec3,
    pub rot: Quat,
    pub scale: Vec3,
    /// 0xRRGGBB.
    pub color: u32,
    /// Two free numeric channels (health, charge, animation phase…). Interpolated.
    pub a: f32,
    pub b: f32,
}

impl RenderEntity {
    /// f32s per entity in the packed buffer:
    /// `[id, kind, flags, px, py, pz, qx, qy, qz, qw, sx, sy, sz, color, a, b]`.
    pub const STRIDE: usize = 16;

    pub fn new(id: u32, kind: impl Into<u16>, pos: Vec3) -> Self {
        RenderEntity {
            id,
            kind: kind.into(),
            flags: 0,
            pos,
            rot: Quat::IDENTITY,
            scale: Vec3::ONE,
            color: 0xffffff,
            a: 0.0,
            b: 0.0,
        }
    }
    /// 2D games: place at `(x, y)` on the XY plane. `z` is draw order (bigger = in front);
    /// pair with `<GameCanvas orthographic>` + `<Camera2D>` on the client.
    pub fn new2d(id: u32, kind: impl Into<u16>, pos: Vec2, z: f32) -> Self {
        Self::new(id, kind, Vec3::new(pos.x, pos.y, z))
    }
    /// 2D rotation (radians, counter-clockwise) about the Z axis.
    pub fn angle(mut self, radians: f32) -> Self {
        let (s, c) = crate::math::sin_cos(radians * 0.5);
        self.rot = Quat::from_xyzw(0.0, 0.0, s, c);
        self
    }
    /// 2D size in world units (z scale stays 1).
    pub fn size2d(mut self, size: Vec2) -> Self {
        self.scale = Vec3::new(size.x, size.y, 1.0);
        self
    }
    pub fn rot(mut self, q: Quat) -> Self {
        self.rot = q;
        self
    }
    pub fn scale(mut self, s: Vec3) -> Self {
        self.scale = s;
        self
    }
    pub fn color(mut self, c: u32) -> Self {
        self.color = c;
        self
    }
    pub fn flags(mut self, f: u16) -> Self {
        self.flags |= f;
        self
    }
    pub fn ab(mut self, a: f32, b: f32) -> Self {
        self.a = a;
        self.b = b;
        self
    }

    pub fn pack(&self, out: &mut Vec<f32>) {
        out.extend_from_slice(&[
            self.id as f32,
            self.kind as f32,
            self.flags as f32,
            self.pos.x,
            self.pos.y,
            self.pos.z,
            self.rot.x,
            self.rot.y,
            self.rot.z,
            self.rot.w,
            self.scale.x,
            self.scale.y,
            self.scale.z,
            (self.color & 0xff_ffff) as f32,
            self.a,
            self.b,
        ]);
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderView {
    pub entities: Vec<RenderEntity>,
}

impl RenderView {
    pub fn clear(&mut self) {
        self.entities.clear();
    }
    pub fn push(&mut self, e: RenderEntity) {
        self.entities.push(e);
    }
    /// Sort by id (the client does this after `Game::render` so lookups can binary-search).
    pub fn finish(&mut self) {
        self.entities.sort_by_key(|e| e.id);
    }
    pub fn find(&self, id: u32) -> Option<&RenderEntity> {
        self.entities.binary_search_by_key(&id, |e| e.id).ok().map(|i| &self.entities[i])
    }
}

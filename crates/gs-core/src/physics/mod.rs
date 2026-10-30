//! A small deterministic physics world: spheres + axis-aligned boxes, gravity, impulses with
//! restitution/friction, sensors, layers/masks, raycasts and overlap queries.
//!
//! It is deliberately simple (no rotation dynamics) so it is cheap to clone — client prediction
//! clones and re-simulates the whole world on every correction — and easy to read and extend.
//! Iteration order is always by body index, and only IEEE-exact ops are used, so the server and
//! the browser step it bit-identically.
//!
//! ```
//! use gs_core::physics::*;
//! use gs_core::math::Vec3;
//! let mut w = World::new(Vec3::new(0.0, -20.0, 0.0));
//! w.floor = Some(0.0);
//! let ball = w.add(Body::dynamic(Shape::Sphere { r: 0.5 }, Vec3::new(0.0, 3.0, 0.0)));
//! for _ in 0..120 { w.step(1.0 / 60.0); }
//! assert!(w[ball].grounded);
//! ```

use crate::math::Vec3;
use gs_wire::Wire;

mod collide;
pub use collide::{Contact, RayHit};

#[derive(Wire, Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Sphere {
        r: f32,
    },
    /// Axis-aligned box with half-extents.
    Box {
        half: Vec3,
    },
}

impl Shape {
    pub fn half_extents(&self) -> Vec3 {
        match *self {
            Shape::Sphere { r } => Vec3::splat(r),
            Shape::Box { half } => half,
        }
    }
}

#[derive(Wire, Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    /// Moved by gravity, velocity and contacts.
    Dynamic,
    /// Never moves (walls, floors).
    Static,
    /// Moves by its velocity only, pushes dynamics, ignores contacts (platforms, doors).
    Kinematic,
    /// Detects overlaps (reported in `contacts`) but has no collision response (pickups, zones).
    Sensor,
}

/// Stable handle to a body. Indices are reused after removal; the generation guards stale ids.
#[derive(Wire, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BodyId {
    pub index: u32,
    pub generation: u32,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Body {
    pub kind: BodyKind,
    pub shape: Shape,
    pub pos: Vec3,
    pub vel: Vec3,
    /// 0 for infinite mass.
    pub inv_mass: f32,
    pub restitution: f32,
    pub friction: f32,
    /// Linear damping per second (air drag).
    pub damping: f32,
    pub gravity_scale: f32,
    /// Bitmask of layers this body is on.
    pub layer: u32,
    /// Bitmask of layers this body collides with. Two bodies interact iff each one's mask
    /// includes the other's layer.
    pub mask: u32,
    /// Free slot for game data (an entity id, a player id…).
    pub user: u32,
    /// Supported from below this step (derived; recomputed by every `step`).
    pub grounded: bool,
}

impl Body {
    pub fn new(kind: BodyKind, shape: Shape, pos: Vec3) -> Self {
        let inv_mass = match kind {
            BodyKind::Dynamic => 1.0,
            _ => 0.0,
        };
        Body {
            kind,
            shape,
            pos,
            vel: Vec3::ZERO,
            inv_mass,
            restitution: 0.1,
            friction: 0.4,
            damping: 0.0,
            gravity_scale: 1.0,
            layer: 1,
            mask: u32::MAX,
            user: 0,
            grounded: false,
        }
    }
    pub fn dynamic(shape: Shape, pos: Vec3) -> Self {
        Self::new(BodyKind::Dynamic, shape, pos)
    }
    pub fn fixed(shape: Shape, pos: Vec3) -> Self {
        Self::new(BodyKind::Static, shape, pos)
    }
    pub fn kinematic(shape: Shape, pos: Vec3) -> Self {
        Self::new(BodyKind::Kinematic, shape, pos)
    }
    pub fn sensor(shape: Shape, pos: Vec3) -> Self {
        Self::new(BodyKind::Sensor, shape, pos)
    }
    pub fn with_mass(mut self, mass: f32) -> Self {
        self.inv_mass = if mass > 0.0 && self.kind == BodyKind::Dynamic { 1.0 / mass } else { 0.0 };
        self
    }
    pub fn with_restitution(mut self, e: f32) -> Self {
        self.restitution = e;
        self
    }
    pub fn with_friction(mut self, f: f32) -> Self {
        self.friction = f;
        self
    }
    pub fn with_damping(mut self, d: f32) -> Self {
        self.damping = d;
        self
    }
    pub fn with_layers(mut self, layer: u32, mask: u32) -> Self {
        self.layer = layer;
        self.mask = mask;
        self
    }
    pub fn with_user(mut self, user: u32) -> Self {
        self.user = user;
        self
    }
    pub fn with_gravity_scale(mut self, g: f32) -> Self {
        self.gravity_scale = g;
        self
    }
    pub fn aabb(&self) -> (Vec3, Vec3) {
        let h = self.shape.half_extents();
        (self.pos - h, self.pos + h)
    }
}

#[derive(Wire, Clone, Debug, PartialEq)]
struct Slot {
    generation: u32,
    body: Option<Body>,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct World {
    pub gravity: Vec3,
    /// Optional infinite floor plane at this height (cheap ground for prototypes).
    pub floor: Option<f32>,
    /// Contact-solver iterations per step. More = stiffer stacks, more CPU.
    pub iterations: u8,
    slots: Vec<Slot>,
    free: Vec<u32>,
    /// Contacts found by the last `step` (derived — never sent, recomputed every step).
    #[wire(skip)]
    pub contacts: Vec<Contact>,
}

impl Default for World {
    fn default() -> Self {
        World::new(Vec3::new(0.0, -9.81, 0.0))
    }
}

impl std::ops::Index<BodyId> for World {
    type Output = Body;
    fn index(&self, id: BodyId) -> &Body {
        self.get(id).expect("stale or invalid BodyId")
    }
}

impl std::ops::IndexMut<BodyId> for World {
    fn index_mut(&mut self, id: BodyId) -> &mut Body {
        self.get_mut(id).expect("stale or invalid BodyId")
    }
}

impl World {
    pub fn new(gravity: Vec3) -> Self {
        World { gravity, floor: None, iterations: 4, slots: Vec::new(), free: Vec::new(), contacts: Vec::new() }
    }

    pub fn add(&mut self, body: Body) -> BodyId {
        // Reuse the lowest free index so allocation order is deterministic and compact.
        if let Some(pos) = self.free.iter().enumerate().min_by_key(|(_, i)| **i).map(|(p, _)| p) {
            let index = self.free.swap_remove(pos);
            let slot = &mut self.slots[index as usize];
            slot.generation = slot.generation.wrapping_add(1);
            slot.body = Some(body);
            return BodyId { index, generation: slot.generation };
        }
        self.slots.push(Slot { generation: 0, body: Some(body) });
        BodyId { index: (self.slots.len() - 1) as u32, generation: 0 }
    }

    pub fn remove(&mut self, id: BodyId) -> Option<Body> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let b = slot.body.take();
        if b.is_some() {
            self.free.push(id.index);
        }
        b
    }

    pub fn get(&self, id: BodyId) -> Option<&Body> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation == id.generation { slot.body.as_ref() } else { None }
    }

    pub fn get_mut(&mut self, id: BodyId) -> Option<&mut Body> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation == id.generation { slot.body.as_mut() } else { None }
    }

    pub fn contains(&self, id: BodyId) -> bool {
        self.get(id).is_some()
    }

    /// All live bodies in index order.
    pub fn iter(&self) -> impl Iterator<Item = (BodyId, &Body)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.body.as_ref().map(|b| (BodyId { index: i as u32, generation: s.generation }, b)))
    }

    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.body.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Contacts this step that involve `id` (with the normal pointing away from `id`).
    pub fn contacts_of(&self, id: BodyId) -> impl Iterator<Item = (BodyId, Vec3)> + '_ {
        self.contacts.iter().filter_map(move |c| {
            if c.a == id {
                Some((c.b, c.normal))
            } else if c.b == id {
                Some((c.a, -c.normal))
            } else {
                None
            }
        })
    }

    /// Advance the world by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        self.contacts.clear();
        let gravity = self.gravity;
        for slot in &mut self.slots {
            let Some(b) = slot.body.as_mut() else { continue };
            b.grounded = false;
            match b.kind {
                BodyKind::Dynamic => {
                    b.vel += gravity * (b.gravity_scale * dt);
                    if b.damping > 0.0 {
                        b.vel *= 1.0 / (1.0 + b.damping * dt);
                    }
                    b.pos += b.vel * dt;
                }
                BodyKind::Kinematic | BodyKind::Sensor => b.pos += b.vel * dt,
                BodyKind::Static => {}
            }
        }

        let pairs = self.broadphase();
        for iter in 0..self.iterations.max(1) {
            for &(i, j) in &pairs {
                self.solve_pair(i, j, iter == 0);
            }
            if let Some(floor) = self.floor {
                self.solve_floor(floor);
            }
        }
    }

    /// Sort-and-sweep on X. Returns candidate pairs (i < j) in a deterministic order.
    fn broadphase(&self) -> Vec<(usize, usize)> {
        let mut spans: Vec<(f32, f32, usize)> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                s.body.as_ref().map(|b| {
                    let (lo, hi) = b.aabb();
                    (lo.x, hi.x, i)
                })
            })
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
        let mut pairs = Vec::new();
        for (k, &(_, hi, i)) in spans.iter().enumerate() {
            for &(lo2, _, j) in &spans[k + 1..] {
                if lo2 > hi {
                    break;
                }
                let (a, b) = (i.min(j), i.max(j));
                if self.may_interact(a, b) {
                    pairs.push((a, b));
                }
            }
        }
        pairs.sort_unstable();
        pairs
    }

    fn may_interact(&self, i: usize, j: usize) -> bool {
        let (Some(a), Some(b)) = (&self.slots[i].body, &self.slots[j].body) else { return false };
        if a.mask & b.layer == 0 || b.mask & a.layer == 0 {
            return false;
        }
        let moving = |k: BodyKind| matches!(k, BodyKind::Dynamic);
        let sensor = |k: BodyKind| matches!(k, BodyKind::Sensor);
        // Something must react: a dynamic body, or a sensor overlapping anything non-static.
        moving(a.kind)
            || moving(b.kind)
            || (sensor(a.kind) && b.kind == BodyKind::Kinematic)
            || (sensor(b.kind) && a.kind == BodyKind::Kinematic)
    }

    fn solve_pair(&mut self, i: usize, j: usize, record: bool) {
        let (left, right) = self.slots.split_at_mut(j);
        let (Some(a), Some(b)) = (left[i].body.as_mut(), right[0].body.as_mut()) else { return };
        let Some((normal, depth)) = collide::intersect(a, b) else { return };
        if record {
            let (ia, ib) = (
                BodyId { index: i as u32, generation: left[i].generation },
                BodyId { index: j as u32, generation: right[0].generation },
            );
            self.contacts.push(Contact { a: ia, b: ib, normal, depth });
        }
        if a.kind == BodyKind::Sensor || b.kind == BodyKind::Sensor {
            return;
        }
        collide::resolve(a, b, normal, depth);
    }

    fn solve_floor(&mut self, floor: f32) {
        for slot in &mut self.slots {
            let Some(b) = slot.body.as_mut() else { continue };
            if b.kind != BodyKind::Dynamic {
                continue;
            }
            let bottom = b.pos.y - b.shape.half_extents().y;
            if bottom < floor {
                b.pos.y += floor - bottom;
                if b.vel.y < 0.0 {
                    let vn = b.vel.y;
                    b.vel.y = if -vn > 1.0 { -vn * b.restitution } else { 0.0 };
                    let f = (1.0 - b.friction * 0.25).max(0.0);
                    b.vel.x *= f;
                    b.vel.z *= f;
                }
                b.grounded = true;
            } else if bottom - floor < 0.01 {
                b.grounded = true;
            }
        }
    }

    /// Nearest hit along a ray (`dir` need not be normalized; `max_t` is in units of `dir`).
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_t: f32, mask: u32) -> Option<RayHit> {
        let mut best: Option<RayHit> = None;
        for (id, b) in self.iter() {
            if b.layer & mask == 0 || b.kind == BodyKind::Sensor {
                continue;
            }
            if let Some((t, n)) = collide::ray_body(origin, dir, b)
                && t <= max_t
                && best.is_none_or(|h| t < h.t)
            {
                best = Some(RayHit { body: id, t, point: origin + dir * t, normal: n });
            }
        }
        if let Some(floor) = self.floor
            && dir.y < 0.0
        {
            let t = (floor - origin.y) / dir.y;
            if t >= 0.0 && t <= max_t && best.is_none_or(|h| t < h.t) {
                best = Some(RayHit {
                    body: BodyId { index: u32::MAX, generation: 0 },
                    t,
                    point: origin + dir * t,
                    normal: Vec3::Y,
                });
            }
        }
        best
    }

    /// Bodies whose shape overlaps a sphere (index order).
    pub fn overlap_sphere(&self, center: Vec3, r: f32, mask: u32) -> Vec<BodyId> {
        let probe = Body::sensor(Shape::Sphere { r }, center);
        self.iter()
            .filter(|(_, b)| b.layer & mask != 0 && collide::intersect(&probe, b).is_some())
            .map(|(id, _)| id)
            .collect()
    }
}

#[cfg(test)]
mod tests;

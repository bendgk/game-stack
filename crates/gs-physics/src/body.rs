//! Bodies and shapes, generic over the dimension (`V = Vec2 | Vec3`).

use gs_wire::Wire;

use crate::vector::Vector;

#[derive(Wire, Clone, Copy, Debug, PartialEq)]
pub enum Shape<V: Vector> {
    /// Circle in 2D, sphere in 3D.
    Ball { r: f32 },
    /// Axis-aligned rectangle (2D) / box (3D) with half-extents.
    Box { half: V },
    /// Infinite static half-space through the body's position: everything on the side
    /// opposite `normal` is solid. Floors (`normal = UP`), walls, and ramps (tilted normals).
    Plane { normal: V },
}

impl<V: Vector> Shape<V> {
    /// Half-extents of the bounding box (planes are unbounded: `f32::INFINITY`).
    pub fn half_extents(&self) -> V {
        match *self {
            Shape::Ball { r } => V::splat(r),
            Shape::Box { half } => half,
            Shape::Plane { .. } => V::splat(f32::INFINITY),
        }
    }
    pub fn is_plane(&self) -> bool {
        matches!(self, Shape::Plane { .. })
    }
}

#[derive(Wire, Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    /// Moved by gravity, velocity and contacts.
    Dynamic,
    /// Never moves (walls, floors, planes).
    Static,
    /// Moves by its velocity only and pushes dynamics; contacts don't move it (platforms, doors).
    Kinematic,
    /// Reports overlaps in `contacts` with no collision response (pickups, triggers, zones).
    Sensor,
}

/// Stable handle to a body. Indices are reused after removal; the generation catches stale ids.
#[derive(Wire, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BodyId {
    pub index: u32,
    pub generation: u32,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Body<V: Vector> {
    pub kind: BodyKind,
    pub shape: Shape<V>,
    pub pos: V,
    pub vel: V,
    /// 0 = infinite mass.
    pub inv_mass: f32,
    pub restitution: f32,
    pub friction: f32,
    /// Linear damping per second (drag). Top-down 2D games usually want a lot of it.
    pub damping: f32,
    pub gravity_scale: f32,
    /// Bitmask of layers this body is on.
    pub layer: u32,
    /// Layers this body collides with. Two bodies interact iff each mask has the other's layer.
    pub mask: u32,
    /// Free slot for game data (an entity id, a tag…).
    pub user: u32,
    /// Supported from below during the last step (derived; recomputed by every `step`).
    pub grounded: bool,
}

impl<V: Vector> Body<V> {
    pub fn new(kind: BodyKind, shape: Shape<V>, pos: V) -> Self {
        Body {
            kind,
            shape,
            pos,
            vel: V::ZERO,
            inv_mass: if kind == BodyKind::Dynamic { 1.0 } else { 0.0 },
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
    pub fn dynamic(shape: Shape<V>, pos: V) -> Self {
        Self::new(BodyKind::Dynamic, shape, pos)
    }
    pub fn fixed(shape: Shape<V>, pos: V) -> Self {
        Self::new(BodyKind::Static, shape, pos)
    }
    pub fn kinematic(shape: Shape<V>, pos: V) -> Self {
        Self::new(BodyKind::Kinematic, shape, pos)
    }
    pub fn sensor(shape: Shape<V>, pos: V) -> Self {
        Self::new(BodyKind::Sensor, shape, pos)
    }
    /// A static half-space through `point`; solid on the side opposite `normal`.
    pub fn plane(normal: V, point: V) -> Self {
        Self::new(BodyKind::Static, Shape::Plane { normal: normal.normalize_or(V::UP) }, point)
    }
    /// The usual floor: everything below height `y` is solid.
    pub fn floor(y: f32) -> Self {
        Self::plane(V::UP, V::UP * y)
    }
    pub fn with_mass(mut self, mass: f32) -> Self {
        self.inv_mass = if mass > 0.0 && self.kind == BodyKind::Dynamic { 1.0 / mass } else { 0.0 };
        self
    }
    pub fn with_vel(mut self, vel: V) -> Self {
        self.vel = vel;
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
    pub fn with_gravity_scale(mut self, g: f32) -> Self {
        self.gravity_scale = g;
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
    /// Axis-aligned bounds (infinite for planes).
    pub fn aabb(&self) -> (V, V) {
        let h = self.shape.half_extents();
        (self.pos - h, self.pos + h)
    }
    /// Δv = impulse / mass.
    pub fn apply_impulse(&mut self, impulse: V) {
        self.vel += impulse * self.inv_mass;
    }
}

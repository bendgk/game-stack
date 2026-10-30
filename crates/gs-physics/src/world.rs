//! The world: storage, the step, joints, and queries. Generic over `V = Vec2 | Vec3`.

use gs_wire::Wire;

use crate::body::{Body, BodyId, BodyKind, Shape};
use crate::collide::{self, Contact, RayHit};
use crate::vector::Vector;

#[derive(Wire, Clone, Copy, Debug, PartialEq)]
pub enum JointKind {
    /// Keeps the distance exactly `rest` (a rigid rod / chain link).
    Rod,
    /// Keeps the distance at most `rest` (a rope or leash).
    Rope,
    /// Pulls toward `rest` with spring `stiffness` (1/s²) and `damping` (1/s).
    Spring { stiffness: f32, damping: f32 },
}

/// A distance constraint between two body centres.
#[derive(Wire, Clone, Copy, Debug, PartialEq)]
pub struct Joint {
    pub a: BodyId,
    pub b: BodyId,
    pub rest: f32,
    pub kind: JointKind,
}

#[derive(Wire, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct JointId(pub u32);

#[derive(Wire, Clone, Debug, PartialEq)]
struct Slot<V: Vector> {
    generation: u32,
    body: Option<Body<V>>,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct World<V: Vector> {
    pub gravity: V,
    /// Contact-solver iterations per substep. More = stiffer stacks, more CPU.
    pub iterations: u8,
    /// Split each `step` into this many substeps (fast objects, tall stacks, stiff joints).
    pub substeps: u8,
    slots: Vec<Slot<V>>,
    free: Vec<u32>,
    joints: Vec<Option<Joint>>,
    /// Pairs touching after the last step. Part of the state (so `Contact::began` is the same
    /// after a snapshot restore — prediction depends on it).
    touching: Vec<(BodyId, BodyId)>,
    /// Contacts found by the last `step` (derived: never sent, recomputed every step).
    #[wire(skip)]
    pub contacts: Vec<Contact<V>>,
}

impl<V: Vector> Default for World<V> {
    fn default() -> Self {
        World::new(V::UP * -9.81)
    }
}

impl<V: Vector> std::ops::Index<BodyId> for World<V> {
    type Output = Body<V>;
    fn index(&self, id: BodyId) -> &Body<V> {
        self.get(id).expect("stale or invalid BodyId")
    }
}

impl<V: Vector> std::ops::IndexMut<BodyId> for World<V> {
    fn index_mut(&mut self, id: BodyId) -> &mut Body<V> {
        self.get_mut(id).expect("stale or invalid BodyId")
    }
}

impl<V: Vector> World<V> {
    pub fn new(gravity: V) -> Self {
        World {
            gravity,
            iterations: 4,
            substeps: 1,
            slots: Vec::new(),
            free: Vec::new(),
            joints: Vec::new(),
            touching: Vec::new(),
            contacts: Vec::new(),
        }
    }

    /// No gravity (top-down 2D, space games).
    pub fn zero_g() -> Self {
        World::new(V::ZERO)
    }

    // --- bodies --------------------------------------------------------------------------

    pub fn add(&mut self, body: Body<V>) -> BodyId {
        // Reuse the lowest free index: allocation order stays deterministic and compact.
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

    /// Remove a body (and any joint attached to it).
    pub fn remove(&mut self, id: BodyId) -> Option<Body<V>> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let b = slot.body.take();
        if b.is_some() {
            self.free.push(id.index);
            for j in self.joints.iter_mut() {
                if j.is_some_and(|j| j.a == id || j.b == id) {
                    *j = None;
                }
            }
            self.touching.retain(|(a, b)| *a != id && *b != id);
        }
        b
    }

    pub fn get(&self, id: BodyId) -> Option<&Body<V>> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation == id.generation { slot.body.as_ref() } else { None }
    }

    pub fn get_mut(&mut self, id: BodyId) -> Option<&mut Body<V>> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation == id.generation { slot.body.as_mut() } else { None }
    }

    pub fn contains(&self, id: BodyId) -> bool {
        self.get(id).is_some()
    }

    /// All live bodies in index order.
    pub fn iter(&self) -> impl Iterator<Item = (BodyId, &Body<V>)> {
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

    /// Δv = impulse / mass (no-op for static / stale ids).
    pub fn apply_impulse(&mut self, id: BodyId, impulse: V) {
        if let Some(b) = self.get_mut(id) {
            b.apply_impulse(impulse);
        }
    }

    // --- joints --------------------------------------------------------------------------

    pub fn add_joint(&mut self, joint: Joint) -> JointId {
        if let Some(i) = self.joints.iter().position(Option::is_none) {
            self.joints[i] = Some(joint);
            return JointId(i as u32);
        }
        self.joints.push(Some(joint));
        JointId((self.joints.len() - 1) as u32)
    }

    pub fn remove_joint(&mut self, id: JointId) -> Option<Joint> {
        self.joints.get_mut(id.0 as usize)?.take()
    }

    pub fn joint(&self, id: JointId) -> Option<&Joint> {
        self.joints.get(id.0 as usize)?.as_ref()
    }

    pub fn joints(&self) -> impl Iterator<Item = (JointId, &Joint)> {
        self.joints.iter().enumerate().filter_map(|(i, j)| j.as_ref().map(|j| (JointId(i as u32), j)))
    }

    // --- contacts ------------------------------------------------------------------------

    /// Contacts this step involving `id`, as (other body, normal pointing away from `id`).
    pub fn contacts_of(&self, id: BodyId) -> impl Iterator<Item = (BodyId, V)> + '_ {
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

    /// Like [`World::contacts_of`], but only contacts that began this step (enter events).
    pub fn began_contacts_of(&self, id: BodyId) -> impl Iterator<Item = (BodyId, V)> + '_ {
        self.contacts.iter().filter(|c| c.began).filter_map(move |c| {
            if c.a == id {
                Some((c.b, c.normal))
            } else if c.b == id {
                Some((c.a, -c.normal))
            } else {
                None
            }
        })
    }

    // --- the step ------------------------------------------------------------------------

    /// Advance the world by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        self.contacts.clear();
        for slot in &mut self.slots {
            if let Some(b) = slot.body.as_mut() {
                b.grounded = false;
            }
        }
        let n = self.substeps.max(1);
        let h = dt / f32::from(n);
        for _ in 0..n {
            self.substep(h);
        }
        // Enter events: contacts whose pair wasn't touching last step.
        let mut now: Vec<(BodyId, BodyId)> = self.contacts.iter().map(|c| (c.a, c.b)).collect();
        now.sort_unstable();
        for c in &mut self.contacts {
            c.began = self.touching.binary_search(&(c.a, c.b)).is_err();
        }
        self.touching = now;
    }

    fn substep(&mut self, dt: f32) {
        let gravity = self.gravity;
        for slot in &mut self.slots {
            let Some(b) = slot.body.as_mut() else { continue };
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
            self.solve_joints(dt);
        }
    }

    /// Sort-and-sweep on X for bounded shapes, plus every body against every plane.
    /// Returns candidate pairs (i < j) in a deterministic order.
    fn broadphase(&self) -> Vec<(usize, usize)> {
        let mut spans: Vec<(f32, f32, usize)> = Vec::new();
        let mut planes: Vec<usize> = Vec::new();
        for (i, s) in self.slots.iter().enumerate() {
            let Some(b) = &s.body else { continue };
            if b.shape.is_plane() {
                planes.push(i);
            } else {
                let (lo, hi) = b.aabb();
                spans.push((lo.get(0), hi.get(0), i));
            }
        }
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
            for &p in &planes {
                let (a, b) = (i.min(p), i.max(p));
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
        let dynamic = |k: BodyKind| k == BodyKind::Dynamic;
        let sensor = |k: BodyKind| k == BodyKind::Sensor;
        // Something must react: a dynamic body, or a sensor overlapping a moving body.
        dynamic(a.kind)
            || dynamic(b.kind)
            || (sensor(a.kind) && b.kind == BodyKind::Kinematic)
            || (sensor(b.kind) && a.kind == BodyKind::Kinematic)
    }

    fn solve_pair(&mut self, i: usize, j: usize, record: bool) {
        let (left, right) = self.slots.split_at_mut(j);
        let (ga, gb) = (left[i].generation, right[0].generation);
        let (Some(a), Some(b)) = (left[i].body.as_mut(), right[0].body.as_mut()) else { return };
        let Some((normal, depth)) = collide::intersect(a, b) else { return };
        if record {
            let ia = BodyId { index: i as u32, generation: ga };
            let ib = BodyId { index: j as u32, generation: gb };
            if !self.contacts.iter().any(|c| c.a == ia && c.b == ib) {
                self.contacts.push(Contact { a: ia, b: ib, normal, depth, began: false });
            }
        }
        if a.kind == BodyKind::Sensor || b.kind == BodyKind::Sensor {
            return;
        }
        collide::resolve(a, b, normal, depth);
    }

    fn solve_joints(&mut self, dt: f32) {
        for k in 0..self.joints.len() {
            let Some(j) = self.joints[k] else { continue };
            let (ia, ib) = (j.a.index as usize, j.b.index as usize);
            if ia == ib || !self.contains(j.a) || !self.contains(j.b) {
                continue;
            }
            let (lo, hi) = (ia.min(ib), ia.max(ib));
            let (left, right) = self.slots.split_at_mut(hi);
            let (x, y) = (left[lo].body.as_mut().unwrap(), right[0].body.as_mut().unwrap());
            let (a, b) = if ia < ib { (x, y) } else { (y, x) };
            let wa = if a.kind == BodyKind::Dynamic { a.inv_mass } else { 0.0 };
            let wb = if b.kind == BodyKind::Dynamic { b.inv_mass } else { 0.0 };
            let w = wa + wb;
            if w <= 0.0 {
                continue;
            }
            let d = b.pos - a.pos;
            let len = d.length();
            if len < 1e-6 {
                continue;
            }
            let n = d / len;
            let err = len - j.rest;
            let vrel = (b.vel - a.vel).dot(n);
            match j.kind {
                JointKind::Rod | JointKind::Rope => {
                    if j.kind == JointKind::Rope && err <= 0.0 {
                        continue;
                    }
                    let corr = n * (err / w);
                    a.pos += corr * wa;
                    b.pos -= corr * wb;
                    // Remove relative velocity along the link (only stretching, for ropes).
                    if j.kind == JointKind::Rod || vrel > 0.0 {
                        let imp = n * (vrel / w);
                        a.vel += imp * wa;
                        b.vel -= imp * wb;
                    }
                }
                JointKind::Spring { stiffness, damping } => {
                    let f = (stiffness * err + damping * vrel) * dt / w;
                    let imp = n * f;
                    a.vel += imp * wa;
                    b.vel -= imp * wb;
                }
            }
        }
    }

    // --- queries -------------------------------------------------------------------------

    /// Nearest hit along a ray (`dir` need not be normalised; `max_t` is in units of `dir`).
    pub fn raycast(&self, origin: V, dir: V, max_t: f32, mask: u32) -> Option<RayHit<V>> {
        let mut best: Option<RayHit<V>> = None;
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
        best
    }

    /// Bodies overlapping a ball (circle in 2D) — index order.
    pub fn overlap_ball(&self, center: V, r: f32, mask: u32) -> Vec<BodyId> {
        self.overlap(&Body::sensor(Shape::Ball { r }, center), mask)
    }

    /// Bodies overlapping an axis-aligned box — index order.
    pub fn overlap_box(&self, center: V, half: V, mask: u32) -> Vec<BodyId> {
        self.overlap(&Body::sensor(Shape::Box { half }, center), mask)
    }

    fn overlap(&self, probe: &Body<V>, mask: u32) -> Vec<BodyId> {
        self.iter()
            .filter(|(_, b)| b.layer & mask != 0 && collide::intersect(probe, b).is_some())
            .map(|(id, _)| id)
            .collect()
    }
}

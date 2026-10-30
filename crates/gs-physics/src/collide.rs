//! Narrow phase, contact response and ray casts — written once for every dimension.

use gs_wire::Wire;

use crate::body::{Body, BodyId, BodyKind, Shape};
use crate::vector::Vector;

/// A contact from the last step. `normal` points from `a` toward `b`.
#[derive(Wire, Clone, Copy, Debug, PartialEq)]
pub struct Contact<V: Vector> {
    pub a: BodyId,
    pub b: BodyId,
    pub normal: V,
    pub depth: f32,
    /// First step these two touched (they weren't touching the step before).
    pub began: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit<V: Vector> {
    pub body: BodyId,
    /// Distance along the ray in units of `dir`.
    pub t: f32,
    pub point: V,
    pub normal: V,
}

const SLOP: f32 = 0.001;
const CORRECTION: f32 = 0.8;

/// Axis order for tie-breaks: prefer Y (stacking), then X, then Z. Deterministic.
fn axis_order<V: Vector>() -> &'static [usize] {
    if V::DIM == 2 { &[1, 0] } else { &[1, 0, 2] }
}

fn sign(v: f32) -> f32 {
    if v < 0.0 { -1.0 } else { 1.0 }
}

/// Penetration test. Returns (normal from a to b, depth > 0).
pub(crate) fn intersect<V: Vector>(a: &Body<V>, b: &Body<V>) -> Option<(V, f32)> {
    match (a.shape, b.shape) {
        (Shape::Ball { r: ra }, Shape::Ball { r: rb }) => {
            let d = b.pos - a.pos;
            let dist2 = d.length_squared();
            let rs = ra + rb;
            if dist2 >= rs * rs {
                return None;
            }
            let dist = dist2.sqrt();
            let n = if dist > 1e-6 { d / dist } else { V::UP };
            Some((n, rs - dist))
        }
        (Shape::Ball { r }, Shape::Box { half }) => ball_box(a.pos, r, b.pos, half),
        (Shape::Box { half }, Shape::Ball { r }) => ball_box(b.pos, r, a.pos, half).map(|(n, d)| (-n, d)),
        (Shape::Box { half: ha }, Shape::Box { half: hb }) => {
            let d = b.pos - a.pos;
            let o = ha + hb - d.abs();
            let mut best: Option<(usize, f32)> = None;
            for &ax in axis_order::<V>() {
                let ov = o.get(ax);
                if ov <= 0.0 {
                    return None;
                }
                if best.is_none_or(|(_, bo)| ov < bo) {
                    best = Some((ax, ov));
                }
            }
            let (ax, ov) = best?;
            Some((V::axis(ax) * sign(d.get(ax)), ov))
        }
        (_, Shape::Plane { normal }) => vs_plane(a, normal, b.pos),
        (Shape::Plane { normal }, _) => vs_plane(b, normal, a.pos).map(|(n, d)| (-n, d)),
    }
}

/// Shape `s` vs the half-space through `p` with outward `n`. Normal from `s` toward the plane.
fn vs_plane<V: Vector>(s: &Body<V>, n: V, p: V) -> Option<(V, f32)> {
    let reach = match s.shape {
        Shape::Ball { r } => r,
        Shape::Box { half } => n.abs().dot(half),
        Shape::Plane { .. } => return None,
    };
    let dist = n.dot(s.pos - p) - reach;
    if dist >= 0.0 { None } else { Some((-n, -dist)) }
}

/// Ball (center c, radius r) vs box (center p, half h). Normal from ball toward box.
fn ball_box<V: Vector>(c: V, r: f32, p: V, h: V) -> Option<(V, f32)> {
    let local = c - p;
    let closest = local.clamp(-h, h);
    if closest != local {
        let d = closest - local;
        let dist2 = d.length_squared();
        if dist2 >= r * r {
            return None;
        }
        let dist = dist2.sqrt();
        let n = if dist > 1e-6 { d / dist } else { V::UP };
        return Some((n, r - dist));
    }
    // Center inside the box: push out through the nearest face.
    let to_face = h - local.abs();
    let mut best = (1usize, f32::INFINITY);
    for &ax in axis_order::<V>() {
        if to_face.get(ax) < best.1 {
            best = (ax, to_face.get(ax));
        }
    }
    let (ax, tf) = best;
    Some((V::axis(ax) * -sign(local.get(ax)), r + tf))
}

/// Positional correction + velocity impulse. `n` points from a to b.
pub(crate) fn resolve<V: Vector>(a: &mut Body<V>, b: &mut Body<V>, n: V, depth: f32) {
    // Kinematic bodies act as infinite mass that still moves.
    let wa = if a.kind == BodyKind::Dynamic { a.inv_mass } else { 0.0 };
    let wb = if b.kind == BodyKind::Dynamic { b.inv_mass } else { 0.0 };
    let w = wa + wb;
    if w <= 0.0 {
        return;
    }
    let up = n.dot(V::UP);
    if up < -0.5 {
        a.grounded = true;
    }
    if up > 0.5 {
        b.grounded = true;
    }
    let corr = n * ((depth - SLOP).max(0.0) * CORRECTION / w);
    a.pos -= corr * wa;
    b.pos += corr * wb;

    let rel = b.vel - a.vel;
    let vn = rel.dot(n);
    if vn >= 0.0 {
        return; // already separating
    }
    // Slow approaches don't bounce: resting contacts stay quiet.
    let e = if -vn > 1.0 { a.restitution.max(b.restitution) } else { 0.0 };
    let jn = -(1.0 + e) * vn / w;
    let imp = n * jn;
    a.vel -= imp * wa;
    b.vel += imp * wb;

    // Coulomb friction along the slip direction.
    let rel = b.vel - a.vel;
    let tangent = rel.reject(n);
    let tl = tangent.length();
    if tl > 1e-6 {
        let t = tangent / tl;
        let mu = (a.friction * b.friction).sqrt();
        let jt = (-rel.dot(t) / w).clamp(-jn * mu, jn * mu);
        let fimp = t * jt;
        a.vel -= fimp * wa;
        b.vel += fimp * wb;
    }
}

/// Ray vs body: (t, normal) of the entry point.
pub(crate) fn ray_body<V: Vector>(o: V, d: V, b: &Body<V>) -> Option<(f32, V)> {
    match b.shape {
        Shape::Ball { r } => {
            let m = o - b.pos;
            let a = d.dot(d);
            if a <= 0.0 {
                return None;
            }
            let bq = m.dot(d);
            let c = m.dot(m) - r * r;
            if c > 0.0 && bq > 0.0 {
                return None; // outside and pointing away
            }
            let disc = bq * bq - a * c;
            if disc < 0.0 {
                return None;
            }
            let t = ((-bq - disc.sqrt()) / a).max(0.0);
            let p = o + d * t;
            Some((t, (p - b.pos).normalize_or(V::UP)))
        }
        Shape::Box { half } => {
            let (lo, hi) = (b.pos - half, b.pos + half);
            let mut tmin = 0.0f32;
            let mut tmax = f32::INFINITY;
            let mut normal = V::ZERO;
            for ax in 0..V::DIM {
                let (oa, da, l, h) = (o.get(ax), d.get(ax), lo.get(ax), hi.get(ax));
                if da.abs() < 1e-12 {
                    if oa < l || oa > h {
                        return None;
                    }
                    continue;
                }
                let inv = 1.0 / da;
                let (mut t1, mut t2) = ((l - oa) * inv, (h - oa) * inv);
                let mut ns = -1.0;
                if t1 > t2 {
                    std::mem::swap(&mut t1, &mut t2);
                    ns = 1.0;
                }
                if t1 > tmin {
                    tmin = t1;
                    normal = V::axis(ax) * ns;
                }
                tmax = tmax.min(t2);
                if tmin > tmax {
                    return None;
                }
            }
            Some((tmin, if normal == V::ZERO { -d.normalize_or(V::UP) } else { normal }))
        }
        Shape::Plane { normal } => {
            let denom = normal.dot(d);
            let dist = normal.dot(o - b.pos);
            if dist < 0.0 {
                return Some((0.0, normal)); // starts inside the solid side
            }
            if denom >= 0.0 {
                return None;
            }
            Some((-dist / denom, normal))
        }
    }
}

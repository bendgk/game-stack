//! Narrow phase + contact response.

use super::{Body, BodyId, BodyKind, Shape};
use crate::math::Vec3;
use gs_wire::Wire;

/// A contact from the last step. `normal` points from `a` toward `b`.
#[derive(Wire, Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    pub a: BodyId,
    pub b: BodyId,
    pub normal: Vec3,
    pub depth: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// `index == u32::MAX` means the world floor plane.
    pub body: BodyId,
    pub t: f32,
    pub point: Vec3,
    pub normal: Vec3,
}

const SLOP: f32 = 0.001;
const CORRECTION: f32 = 0.8;

/// Penetration test. Returns (normal from a to b, depth > 0).
pub(super) fn intersect(a: &Body, b: &Body) -> Option<(Vec3, f32)> {
    match (a.shape, b.shape) {
        (Shape::Sphere { r: ra }, Shape::Sphere { r: rb }) => {
            let d = b.pos - a.pos;
            let dist2 = d.length_squared();
            let rs = ra + rb;
            if dist2 >= rs * rs {
                return None;
            }
            let dist = dist2.sqrt();
            let n = if dist > 1e-6 { d / dist } else { Vec3::Y };
            Some((n, rs - dist))
        }
        (Shape::Sphere { r }, Shape::Box { half }) => sphere_box(a.pos, r, b.pos, half),
        (Shape::Box { half }, Shape::Sphere { r }) => sphere_box(b.pos, r, a.pos, half).map(|(n, d)| (-n, d)),
        (Shape::Box { half: ha }, Shape::Box { half: hb }) => {
            let d = b.pos - a.pos;
            let o = ha + hb - d.abs();
            if o.x <= 0.0 || o.y <= 0.0 || o.z <= 0.0 {
                return None;
            }
            let sign = |v: f32| if v < 0.0 { -1.0 } else { 1.0 };
            // Minimum-overlap axis; ties prefer Y (stacking), then X, then Z — deterministic.
            if o.y <= o.x && o.y <= o.z {
                Some((Vec3::new(0.0, sign(d.y), 0.0), o.y))
            } else if o.x <= o.z {
                Some((Vec3::new(sign(d.x), 0.0, 0.0), o.x))
            } else {
                Some((Vec3::new(0.0, 0.0, sign(d.z)), o.z))
            }
        }
    }
}

/// Sphere (center c, radius r) vs box (center p, half h). Normal from sphere toward box.
fn sphere_box(c: Vec3, r: f32, p: Vec3, h: Vec3) -> Option<(Vec3, f32)> {
    let local = c - p;
    let closest = local.clamp(-h, h);
    let inside = closest == local;
    if !inside {
        let d = closest - local;
        let dist2 = d.length_squared();
        if dist2 >= r * r {
            return None;
        }
        let dist = dist2.sqrt();
        let n = if dist > 1e-6 { d / dist } else { Vec3::Y };
        return Some((n, r - dist));
    }
    // Center inside the box: push out through the nearest face.
    let to_face = h - local.abs();
    let sign = |v: f32| if v < 0.0 { 1.0 } else { -1.0 };
    if to_face.y <= to_face.x && to_face.y <= to_face.z {
        Some((Vec3::new(0.0, sign(local.y), 0.0), r + to_face.y))
    } else if to_face.x <= to_face.z {
        Some((Vec3::new(sign(local.x), 0.0, 0.0), r + to_face.x))
    } else {
        Some((Vec3::new(0.0, 0.0, sign(local.z)), r + to_face.z))
    }
}

/// Positional correction + velocity impulse. `n` points from a to b.
pub(super) fn resolve(a: &mut Body, b: &mut Body, n: Vec3, depth: f32) {
    // Kinematic bodies act as infinite mass that still moves.
    let wa = if a.kind == BodyKind::Dynamic { a.inv_mass } else { 0.0 };
    let wb = if b.kind == BodyKind::Dynamic { b.inv_mass } else { 0.0 };
    let w = wa + wb;
    if w <= 0.0 {
        return;
    }
    if n.y < -0.5 {
        a.grounded = true;
    }
    if n.y > 0.5 {
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
    // Small approach speeds don't bounce: keeps resting contacts quiet.
    let e = if -vn > 1.0 { a.restitution.max(b.restitution) } else { 0.0 };
    let jn = -(1.0 + e) * vn / w;
    let imp = n * jn;
    a.vel -= imp * wa;
    b.vel += imp * wb;

    // Coulomb friction along the tangential slip direction.
    let rel = b.vel - a.vel;
    let tangent = rel - n * rel.dot(n);
    let tl = tangent.length();
    if tl > 1e-6 {
        let t = tangent / tl;
        let mu = (a.friction * b.friction).sqrt();
        let jt = (-rel.dot(t) / w).max(-jn * mu).min(jn * mu);
        let fimp = t * jt;
        a.vel -= fimp * wa;
        b.vel += fimp * wb;
    }
}

/// Ray vs body: (t, normal) of the entry point.
pub(super) fn ray_body(o: Vec3, d: Vec3, b: &Body) -> Option<(f32, Vec3)> {
    match b.shape {
        Shape::Sphere { r } => {
            let m = o - b.pos;
            let a = d.dot(d);
            if a <= 0.0 {
                return None;
            }
            let bq = m.dot(d);
            let c = m.dot(m) - r * r;
            let disc = bq * bq - a * c;
            if disc < 0.0 {
                return None;
            }
            let t = (-bq - disc.sqrt()) / a;
            let t = if t < 0.0 { 0.0 } else { t };
            if c > 0.0 && bq > 0.0 {
                return None; // outside and pointing away
            }
            let p = o + d * t;
            Some((t, (p - b.pos).normalize_or(Vec3::Y)))
        }
        Shape::Box { half } => {
            let lo = b.pos - half;
            let hi = b.pos + half;
            let mut tmin = 0.0f32;
            let mut tmax = f32::INFINITY;
            let mut normal = Vec3::ZERO;
            for axis in 0..3 {
                let (oa, da, l, h) = (o[axis], d[axis], lo[axis], hi[axis]);
                if da.abs() < 1e-12 {
                    if oa < l || oa > h {
                        return None;
                    }
                    continue;
                }
                let inv = 1.0 / da;
                let (mut t1, mut t2) = ((l - oa) * inv, (h - oa) * inv);
                let mut nsign = -1.0;
                if t1 > t2 {
                    std::mem::swap(&mut t1, &mut t2);
                    nsign = 1.0;
                }
                if t1 > tmin {
                    tmin = t1;
                    normal = Vec3::ZERO;
                    normal[axis] = nsign;
                }
                tmax = tmax.min(t2);
                if tmin > tmax {
                    return None;
                }
            }
            Some((tmin, if normal == Vec3::ZERO { -d.normalize_or(Vec3::Y) } else { normal }))
        }
    }
}

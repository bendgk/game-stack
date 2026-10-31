// voxelscape — shared WGSL: scene bindings, hashing, sky/atmosphere, and the DAG ray tracer.
//
// The world is a hash-consed sparse 64-tree (see vx-core/src/dag.rs). Node layout in `nodes`:
//   [mask_lo, mask_hi, avg_rgba8, payload…]  payload = child offsets (level ≥ 2) or packed
//   u16 materials (level 1). Child index i = x | y<<2 | z<<4.
// All positions are in voxel units (1 voxel = 1/32 m), Y up.

struct Uniforms {
    inv_view_proj: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,       // xyz, w = time (s)
    prev_cam_pos: vec4<f32>,
    res: vec4<f32>,           // width, height, 1/width, 1/height
    sun_dir: vec4<f32>,       // xyz (towards sun), w = sun intensity
    jitter: vec4<f32>,        // xy = subpixel jitter (ndc), z = frame index, w = exposure
    world: vec4<u32>,         // x = root offset, y = root level, z = side (voxels), w = flags
    params: vec4<f32>,        // x = lod bias (radians/px), y = fog density, z = gi distance, w = night amount
    tint: vec4<f32>,          // atmosphere tint rgb, w = cloud cover
    extra: vec4<u32>,         // x = instance count, y = still frames, z = cloud cover (f32 bits)
    out_res: vec4<f32>,       // output width, height, 1/width, 1/height (TAA up-samples to it)
};

/// A prefab drawn at an arbitrary position/rotation (players, physics props). Its DAG lives in
/// the same node pool as the world. Rays are moved into its local frame and traced there.
struct Inst {
    r0: vec4<f32>,            // rows of the world→local rotation
    r1: vec4<f32>,
    r2: vec4<f32>,
    pos: vec4<f32>,           // world position of the pivot (voxels), w = bounding radius
    pivot: vec4<f32>,         // pivot in the prefab's local voxels
    info: vec4<u32>,          // root, levels, side, tint 0xRRGGBB
};

struct Material {
    albedo: vec4<f32>,        // linear rgb, w = grain
    roughness: f32,
    metalness: f32,
    emission: f32,
    flags: u32,
};

const F_GLASS: u32 = 2u;
const F_EMITTER: u32 = 4u;
const F_WATER: u32 = 8u;
const F_FOLIAGE: u32 = 16u;
const F_TINT: u32 = 32u;
const NO_TINT: u32 = 0xffffffffu;
const MAT_LOD: u32 = 0xffffu;
const PI: f32 = 3.14159265;

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read> nodes: array<u32>;
@group(0) @binding(2) var<storage, read> mats: array<Material>;
@group(0) @binding(3) var sky_lut: texture_2d<f32>;
@group(0) @binding(4) var lin_samp: sampler;
@group(0) @binding(5) var<storage, read> insts: array<Inst>;

// ---------------------------------------------------------------------------------------------
// Hashing / sampling
// ---------------------------------------------------------------------------------------------

fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}
fn hash3(p: vec3<i32>) -> u32 {
    return pcg(u32(p.x) ^ pcg(u32(p.y) ^ pcg(u32(p.z))));
}
fn rand(seed: ptr<function, u32>) -> f32 {
    *seed = pcg(*seed);
    return f32(*seed) / 4294967296.0;
}
fn pixel_seed(px: vec2<u32>, salt: u32) -> u32 {
    return pcg(px.x * 1973u + px.y * 9277u + u32(u.jitter.z) * 26699u + salt * 7919u);
}

// Cosine-weighted hemisphere sample around n.
fn cosine_dir(n: vec3<f32>, r1: f32, r2: f32) -> vec3<f32> {
    let a = 2.0 * PI * r1;
    let r = sqrt(r2);
    let s = select(-1.0, 1.0, n.z >= 0.0);
    let aa = -1.0 / (s + n.z);
    let b = n.x * n.y * aa;
    let t = vec3<f32>(1.0 + s * n.x * n.x * aa, s * b, -s * n.x);
    let bb = vec3<f32>(b, s + n.y * n.y * aa, -n.y);
    return normalize(t * (r * cos(a)) + bb * (r * sin(a)) + n * sqrt(max(0.0, 1.0 - r2)));
}

// A direction inside a cone of half-angle `ang` around d.
fn cone_dir(d: vec3<f32>, ang: f32, r1: f32, r2: f32) -> vec3<f32> {
    let ct = 1.0 - r1 * (1.0 - cos(ang));
    let st = sqrt(1.0 - ct * ct);
    let ph = 2.0 * PI * r2;
    let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(d.y) > 0.9);
    let t = normalize(cross(up, d));
    let b = cross(d, t);
    return normalize(t * (st * cos(ph)) + b * (st * sin(ph)) + d * ct);
}

/// Replace NaN / inf with 0 so one bad sample can't poison temporal history.
fn finite3(v: vec3<f32>) -> vec3<f32> {
    return select(v, vec3<f32>(0.0), (v != v) | (abs(v) > vec3<f32>(1e20)));
}

/// Lights switch on at dusk: emissive materials glow fully at night, faintly by day.
fn emission_scale() -> f32 {
    return mix(0.06, 1.0, u.params.w);
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// (atmo.wgsl is spliced here by the renderer)
/*ATMO*/

// ---------------------------------------------------------------------------------------------
// Sky via LUT (filled once per frame by skylut.wgsl): u = azimuth, v = sqrt-mapped elevation.
// ---------------------------------------------------------------------------------------------

fn sky_uv(d: vec3<f32>) -> vec2<f32> {
    let n = normalize(d);
    let az = atan2(n.z, n.x) / (2.0 * PI) + 0.5;
    let el = asin(clamp(n.y, -1.0, 1.0));
    let v = 0.5 - sign(el) * sqrt(abs(el) / (0.5 * PI)) * 0.5;
    return vec2<f32>(az, v);
}

fn sky(d: vec3<f32>, steps: i32) -> vec3<f32> {
    return textureSampleLevel(sky_lut, lin_samp, sky_uv(d), 0.0).rgb;
}

/// Below the horizon there is ground beyond the world: fogged, lit by sun + sky.
fn horizon_ground(d: vec3<f32>) -> vec3<f32> {
    let sd = normalize(u.sun_dir.xyz);
    let lit = sun_color() * max(sd.y, 0.0) + sky(vec3<f32>(0.0, 1.0, 0.0), 0) + moon_light();
    let g = lit * vec3<f32>(0.18, 0.17, 0.15);
    let hz = sky(normalize(vec3<f32>(d.x, 0.02, d.z)), 0);
    return mix(g, hz, exp(d.y * 12.0));
}

// Hash-based value noise (2D) for clouds.
fn hn2(q: vec2<f32>) -> f32 {
    return f32(pcg((bitcast<u32>(i32(q.x)) * 73856093u) ^ (bitcast<u32>(i32(q.y)) * 19349663u)) & 0xffffu) / 65535.0;
}
fn noise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let w = f * f * (3.0 - 2.0 * f);
    let a = hn2(i);
    let b = hn2(i + vec2<f32>(1.0, 0.0));
    let cc = hn2(i + vec2<f32>(0.0, 1.0));
    let dd = hn2(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, w.x), mix(cc, dd, w.x), w.y);
}
fn fbm_clouds(p: vec2<f32>) -> f32 {
    var s = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 5; i++) {
        s += noise2(q) * a;
        q = q * 2.03 + vec2<f32>(17.1, 9.7);
        a *= 0.5;
    }
    return s;
}

/// A drifting layer of cumulus at ~1.6 km: lit from the sun with a silver lining, darker bases,
/// fading toward the horizon. Returns (colour, coverage).
fn clouds(d: vec3<f32>) -> vec4<f32> {
    if (d.y < 0.02) { return vec4<f32>(0.0); }
    let cover = clamp(bitcast<f32>(u.extra.z), 0.0, 1.0);
    let t = 1600.0 / d.y;                     // metres to the cloud plane
    let p = d.xz * t / 900.0 + vec2<f32>(u.cam_pos.w * 0.004, u.cam_pos.w * 0.0015);
    let n = fbm_clouds(p);
    let dens = smoothstep(0.62 - cover * 0.25, 0.85 - cover * 0.2, n);
    if (dens <= 0.0) { return vec4<f32>(0.0); }
    let sd = normalize(u.sun_dir.xyz);
    // Self-shadowing: sample the density a little toward the sun.
    let ps = p + sd.xz * 0.08;
    let shade = smoothstep(0.55, 0.9, fbm_clouds(ps));
    let sun = sun_color();
    let mu = max(dot(d, sd), 0.0);
    let silver = pow(mu, 8.0) * 2.0 * (1.0 - dens);
    let ambient = sky(vec3<f32>(0.0, 1.0, 0.0), 0) * 0.9 + moon_light() * 2.0;
    let lit = sun * (0.09 * (1.0 - shade * 0.7) + silver * 0.12) + ambient;
    let fade = smoothstep(0.02, 0.18, d.y);
    return vec4<f32>(lit, dens * fade);
}

fn sky_with_sun(d_in: vec3<f32>) -> vec3<f32> {
    let d = normalize(d_in);
    if (d.y < 0.0) { return horizon_ground(d); }
    var c = sky(d, 12);
    let sd = normalize(u.sun_dir.xyz);
    let cs = dot(d, sd);
    let disk = smoothstep(0.99995, 0.999975, cs);
    c += sun_color() * disk * 20.0;
    let cl = clouds(d);
    c = mix(c, cl.rgb, cl.a);
    if (u.params.w > 0.01 && d.y > 0.0) {
        let q = vec3<i32>(floor(d * 600.0));
        let hsh = hash3(q);
        if ((hsh & 1023u) == 0u) {
            c += vec3<f32>(0.6, 0.65, 0.8) * u.params.w * f32(hsh >> 24u) / 255.0;
        }
    }
    return c;
}

/// Ambient sky radiance for GI misses / rough reflections.
fn sky_ambient(d: vec3<f32>) -> vec3<f32> {
    if (d.y < 0.0) { return horizon_ground(d); }
    return sky(d, 4) + moon_light() * max(d.y, 0.1);
}

// ---------------------------------------------------------------------------------------------
// The DAG ray tracer.
// ---------------------------------------------------------------------------------------------

struct Hit {
    hit: bool,
    t: f32,
    n: vec3<f32>,
    voxel: vec3<i32>,
    mat: u32,       // MAT_LOD for a coarse (LOD) hit
    avg: u32,       // node average colour (for LOD hits)
    steps: u32,
    tint: u32,      // instance colour for TINT materials (NO_TINT for the world)
};

/// The face of cube [lo, lo+size) the ray entered through: the entry-facing face the hit
/// point is closest to. Robust at voxel edges/corners where "last axis stepped" is ambiguous.
fn entry_normal(ro: vec3<f32>, rd: vec3<f32>, t: f32, lo: vec3<f32>, size: f32) -> vec3<f32> {
    let p = ro + rd * t;
    let face = lo + select(vec3<f32>(size), vec3<f32>(0.0), rd > vec3<f32>(0.0));
    var d = abs(p - face);
    // Faces the ray can't have entered through (moving away / parallel) don't count.
    d = select(d, vec3<f32>(1e30), abs(rd) < vec3<f32>(1e-7));
    if (d.x <= d.y && d.x <= d.z) { return vec3<f32>(-sign(rd.x), 0.0, 0.0); }
    if (d.y <= d.z) { return vec3<f32>(0.0, -sign(rd.y), 0.0); }
    return vec3<f32>(0.0, 0.0, -sign(rd.z));
}

fn mask_bit(node: u32, i: u32) -> bool {
    let w = nodes[node + (i >> 5u)];
    return ((w >> (i & 31u)) & 1u) == 1u;
}

fn rank(node: u32, i: u32) -> u32 {
    let lo = nodes[node];
    if (i < 32u) {
        return countOneBits(lo & ((1u << i) - 1u));
    }
    let hi = nodes[node + 1u];
    return countOneBits(lo) + countOneBits(hi & ((1u << (i - 32u)) - 1u));
}

fn child_ptr(node: u32, i: u32) -> u32 {
    return nodes[node + 3u + rank(node, i)];
}

fn leaf_mat(node: u32, i: u32) -> u32 {
    let r = rank(node, i);
    let w = nodes[node + 3u + (r >> 1u)];
    return select(w & 0xffffu, w >> 16u, (r & 1u) == 1u);
}

fn safe_inv(d: vec3<f32>) -> vec3<f32> {
    // Tiny components get a large inverse with the right sign.
    let s = select(vec3<f32>(-1.0), vec3<f32>(1.0), d >= vec3<f32>(0.0));
    let m = max(abs(d), vec3<f32>(1e-7));
    return s / m;
}

/// Trace from `ro` along unit `rd` up to `tmax` voxels.
/// `lod_k`: a node whose cube is smaller than `t * lod_k` is treated as a (stochastic,
/// coverage-weighted) solid with its average colour — continuous LOD for free.
fn trace_dag(root: u32, root_level: u32, iside: u32, ro: vec3<f32>, rd: vec3<f32>, tmax_in: f32, lod_k: f32, lod_rnd: f32, max_steps: u32) -> Hit {
    var h: Hit;
    h.hit = false;
    h.t = tmax_in;
    h.steps = 0u;
    h.tint = NO_TINT;
    let side = f32(iside);
    let inv = safe_inv(rd);
    let t0 = (vec3<f32>(0.0) - ro) * inv;
    let t1 = (vec3<f32>(side) - ro) * inv;
    let tn = min(t0, t1);
    let tf = max(t0, t1);
    var t = max(max(max(tn.x, tn.y), tn.z), 0.0);
    let tmax = min(min(min(tf.x, tf.y), tf.z), tmax_in);
    if (t > tmax) { return h; }
    // Entry face normal (if we started outside the world box).
    var n = vec3<f32>(0.0);
    if (t > 0.0) {
        if (tn.x >= tn.y && tn.x >= tn.z) { n = vec3<f32>(-sign(rd.x), 0.0, 0.0); }
        else if (tn.y >= tn.z) { n = vec3<f32>(0.0, -sign(rd.y), 0.0); }
        else { n = vec3<f32>(0.0, 0.0, -sign(rd.z)); }
    }
    var stack_node: array<u32, 12>;
    var stack_key: array<vec3<i32>, 12>;
    var level = root_level;
    stack_node[level] = root;
    stack_key[level] = vec3<i32>(0);
    let maxc = i32(iside) - 1;
    // Never 0 (sign(0) would stall stepping along an axis the ray is exactly parallel to:
    // a one-pixel line down the middle of the screen when looking straight along an axis).
    let step_sign = select(vec3<f32>(-1.0), vec3<f32>(1.0), rd >= vec3<f32>(0.0));

    for (var it = 0u; it < max_steps; it++) {
        h.steps = it;
        if (t > tmax) { return h; }
        let bias = 0.005 + t * 1e-6;
        let p = ro + rd * (t + bias);
        let ip = clamp(vec3<i32>(floor(p)), vec3<i32>(0), vec3<i32>(maxc));
        // Pop until the current node contains ip.
        loop {
            if (level >= root_level) { break; }
            let sh = vec3<u32>(2u * level);
            if (all((ip >> sh) == stack_key[level])) { break; }
            level += 1u;
        }
        var node = stack_node[level];
        var empty_shift = 0u;
        // Descend.
        loop {
            let s = 2u * (level - 1u);
            let c3 = (ip >> vec3<u32>(s)) & vec3<i32>(3);
            let ci = u32(c3.x | (c3.y << 2u) | (c3.z << 4u));
            if (!mask_bit(node, ci)) {
                empty_shift = s;
                break;
            }
            if (level == 1u) {
                h.hit = true;
                h.t = t;
                h.n = entry_normal(ro, rd, t, vec3<f32>(ip), 1.0);
                h.voxel = ip;
                h.mat = leaf_mat(node, ci);
                // The 4³ node's prefiltered colour: the voxel's "mip" for albedo filtering.
                h.avg = nodes[node + 2u];
                return h;
            }
            let child = child_ptr(node, ci);
            let size = f32(1u << s);
            if (size <= t * lod_k) {
                // Coarse hit: accept with probability = coverage (stochastic LOD; TAA resolves it).
                let avg = nodes[child + 2u];
                let cov = f32(avg >> 24u) / 255.0;
                if (lod_rnd < cov) {
                    h.hit = true;
                    h.t = t;
                    h.n = entry_normal(ro, rd, t, vec3<f32>((ip >> vec3<u32>(s)) << vec3<u32>(s)), size);
                    h.voxel = ip;
                    h.mat = MAT_LOD;
                    h.avg = avg;
                    return h;
                }
                empty_shift = s;
                break;
            }
            level -= 1u;
            node = child;
            stack_node[level] = child;
            stack_key[level] = ip >> vec3<u32>(2u * level);
        }
        // Leave the empty cube of 2^empty_shift voxels around ip.
        let sz = f32(1u << empty_shift);
        let lo = vec3<f32>((ip >> vec3<u32>(empty_shift)) << vec3<u32>(empty_shift));
        let bound = lo + select(vec3<f32>(0.0), vec3<f32>(sz), rd > vec3<f32>(0.0));
        var te = (bound - ro) * inv;
        // Axes the ray barely moves along can't be the exit (their inverse is unreliable).
        te = select(te, vec3<f32>(1e30), abs(rd) < vec3<f32>(1e-7));
        // Nor can a face that doesn't move the ray forward: a ray running along a voxel plane
        // (camera exactly on a boundary, looking along an axis) would otherwise "exit" through
        // that plane every step, crawl, and run out of steps — a see-through line on screen.
        te = select(te, vec3<f32>(1e30), te <= vec3<f32>(t + 1e-5));
        let t_prev = t;
        if (te.x <= te.y && te.x <= te.z) { t = te.x; n = vec3<f32>(-step_sign.x, 0.0, 0.0); }
        else if (te.y <= te.z) { t = te.y; n = vec3<f32>(0.0, -step_sign.y, 0.0); }
        else { t = te.z; n = vec3<f32>(0.0, 0.0, -step_sign.z); }
        // Always make progress (float edge cases can put the exit at or behind t).
        t = max(t, t_prev + 1e-4 + t_prev * 1e-7);
    }
    return h;
}

/// Trace the scene: the world plus every instance; the nearest hit wins.
fn trace(ro: vec3<f32>, rd: vec3<f32>, tmax_in: f32, lod_k: f32, lod_rnd: f32, max_steps: u32) -> Hit {
    var h = trace_dag(u.world.x, u.world.y, u.world.z, ro, rd, tmax_in, lod_k, lod_rnd, max_steps);
    let n = min(u.extra.x, arrayLength(&insts));
    for (var i = 0u; i < n; i++) {
        let it = insts[i];
        // Bounding sphere first: most rays miss most instances.
        let oc = ro - it.pos.xyz;
        let b = dot(oc, rd);
        let c = dot(oc, oc) - it.pos.w * it.pos.w;
        if (c > 0.0 && b > 0.0) { continue; }
        let disc = b * b - c;
        if (disc < 0.0) { continue; }
        if (-b - sqrt(disc) > h.t) { continue; }
        let lo = vec3<f32>(dot(it.r0.xyz, oc), dot(it.r1.xyz, oc), dot(it.r2.xyz, oc)) + it.pivot.xyz;
        let ld = vec3<f32>(dot(it.r0.xyz, rd), dot(it.r1.xyz, rd), dot(it.r2.xyz, rd));
        let hi = trace_dag(it.info.x, it.info.y, it.info.z, lo, ld, h.t, lod_k, lod_rnd, max_steps / 2u);
        if (hi.hit && hi.t < h.t) {
            h = hi;
            h.n = normalize(it.r0.xyz * hi.n.x + it.r1.xyz * hi.n.y + it.r2.xyz * hi.n.z);
            h.tint = it.info.w;
        }
    }
    return h;
}

/// How full (0..1) the node of `level` containing voxel `p` is (level 0 = the voxel itself).
fn coverage(p: vec3<f32>, level: u32) -> f32 {
    let ip = vec3<i32>(floor(p));
    if (any(ip < vec3<i32>(0)) || any(ip >= vec3<i32>(i32(u.world.z)))) { return 0.0; }
    var node = u.world.x;
    var l = u.world.y;
    loop {
        let s = 2u * (l - 1u);
        let c3 = (ip >> vec3<u32>(s)) & vec3<i32>(3);
        let ci = u32(c3.x | (c3.y << 2u) | (c3.z << 4u));
        if (!mask_bit(node, ci)) { return 0.0; }
        if (l == 1u) { return 1.0; }
        let child = child_ptr(node, ci);
        if (l - 1u <= level) {
            return f32(nodes[child + 2u] >> 24u) / 255.0;
        }
        node = child;
        l -= 1u;
    }
    return 0.0;
}

/// Hard sun shadow: 1 if `p` (just off a surface) sees the sun. Occluders are resolved more
/// coarsely the further the ray travels (`lod_k`), and a ray that hasn't hit anything within
/// its step budget counts as lit; both depend only on the world point, so they can't flicker.
fn sun_shadow(p: vec3<f32>, lod_k: f32) -> f32 {
    let sd = normalize(u.sun_dir.xyz);
    if (sd.y <= -0.02) { return 0.0; }
    // Long-range occluders (mountains, towers): a ray.
    let h = trace(p, sd, 1e6, lod_k, 0.5, 72u);
    if (h.hit && h.t > 384.0) { return 0.0; }
    // Near occluders: a cone toward the sun through the coverage pyramid. The cone widens with
    // distance, so shadows are crisp where things touch and soften away from them (contact
    // hardening), with no noise.
    var vis = 1.0;
    var dist = 2.0;
    for (var k = 0; k < 9; k++) {
        let width = max(dist * 0.045, 1.0);
        let level = u32(clamp(floor(log2(width) * 0.5), 0.0, 4.0));
        vis *= 1.0 - coverage(p + sd * dist, level) * 0.9;
        if (vis < 0.02) { return 0.0; }
        dist *= 1.9;
    }
    // If the ray hit something nearby that the cone's coarse steps missed, trust the ray.
    return select(vis, min(vis, 0.15), h.hit);
}

/// A smooth surface normal from the coverage gradient around `p` at `level` (like the gradient
/// of a distance field). Far away, a hit's own normal is the face of a coarse cube, so slopes
/// shade as stair-stepped stripes; the gradient sees the actual slope.
fn smooth_normal(p: vec3<f32>, level: u32, fallback: vec3<f32>) -> vec3<f32> {
    let s = f32(1u << (2u * level));
    let g = vec3<f32>(
        coverage(p + vec3<f32>(s, 0.0, 0.0), level) - coverage(p - vec3<f32>(s, 0.0, 0.0), level),
        coverage(p + vec3<f32>(0.0, s, 0.0), level) - coverage(p - vec3<f32>(0.0, s, 0.0), level),
        coverage(p + vec3<f32>(0.0, 0.0, s), level) - coverage(p - vec3<f32>(0.0, 0.0, s), level),
    );
    if (dot(g, g) < 1e-4) { return fallback; }
    let n = -normalize(g);
    // Keep it facing the side we hit.
    return select(fallback, n, dot(n, fallback) > 0.0);
}

/// Albedo (linear) of a hit, with per-voxel grain; TINT materials take the instance colour.
/// Grain fades out once a voxel is smaller than ~1.5 pixels (`t` = hit distance in voxels):
/// sub-pixel colour noise can't be resolved, it only aliases into shimmering grain.
fn hit_albedo_at(mat: u32, avg: u32, voxel: vec3<i32>, tint: u32, t: f32) -> vec3<f32> {
    let footprint = t * u.params.x; // voxels per pixel at this distance
    let fine = hit_albedo_grain(mat, avg, voxel, tint, clamp(1.5 - footprint, 0.0, 1.0));
    if (mat == MAT_LOD || tint != NO_TINT) { return fine; }
    // Mipmapping for voxels: once a pixel spans more than about one voxel, fade toward the 4³
    // node's prefiltered colour, so 1-voxel details (tile grout, window frames, road paint)
    // average out instead of crawling and flickering as you move.
    let c = vec3<f32>(f32(avg & 255u), f32((avg >> 8u) & 255u), f32((avg >> 16u) & 255u)) / 255.0;
    let mip = pow(c, vec3<f32>(2.2));
    return mix(fine, mip, clamp((footprint - 0.8) / 2.4, 0.0, 1.0));
}

fn hit_albedo(mat: u32, avg: u32, voxel: vec3<i32>, tint: u32) -> vec3<f32> {
    return hit_albedo_grain(mat, avg, voxel, tint, 0.0);
}

fn hit_albedo_grain(mat: u32, avg: u32, voxel: vec3<i32>, tint: u32, grain: f32) -> vec3<f32> {
    if (mat == MAT_LOD) {
        let c = vec3<f32>(f32(avg & 255u), f32((avg >> 8u) & 255u), f32((avg >> 16u) & 255u)) / 255.0;
        return pow(c, vec3<f32>(2.2));
    }
    let m = mats[mat];
    let g = (f32(hash3(voxel) & 255u) / 255.0 - 0.5) * 2.0 * m.albedo.w * grain;
    var base = m.albedo.rgb;
    if ((m.flags & F_TINT) != 0u && tint != NO_TINT) {
        let t = vec3<f32>(f32((tint >> 16u) & 255u), f32((tint >> 8u) & 255u), f32(tint & 255u)) / 255.0;
        base = pow(t, vec3<f32>(2.2));
    }
    return max(base * (1.0 + g), vec3<f32>(0.0));
}

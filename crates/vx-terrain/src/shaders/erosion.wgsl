// GPU hydraulic + thermal erosion (Mei et al. 2007 / Šťava et al. 2008) — mirrors erosion.rs.
// Buffers (vec4 per cell, ≤ 6 storage buffers so it runs on WebGPU defaults):
//   st  = (terrain b, water d, sediment s, rain rate)
//   fl  = outflow flux (left, right, top, bottom) — previous step
//   fo  = outflow flux — this step
//   ve  = (velocity x, velocity y, total eroded, total deposited)
//   tmp = (d1, d2, b2, s2) intermediates
//   gv  = thermal outflow (left, right, top, bottom)

struct P {
    w: u32,
    h: u32,
    raining: u32,
    _pad: u32,
    l: f32,
    dt: f32,
    pipe: f32,
    grav: f32,
    cap: f32,
    susp: f32,
    depo: f32,
    evap: f32,
    min_tilt: f32,
    max_depth: f32,
    talus: f32,
    thermal: f32,
};

@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read_write> st: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> fl: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> fo: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> ve: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read_write> tmp: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read_write> gv: array<vec4<f32>>;

fn ix(x: u32, y: u32) -> u32 { return y * p.w + x; }

fn inb(gid: vec3<u32>) -> bool { return gid.x < p.w && gid.y < p.h; }

@compute @workgroup_size(16, 16)
fn flux_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (!inb(gid)) { return; }
    let x = gid.x; let y = gid.y;
    let i = ix(x, y);
    var d1 = st[i].y;
    if (p.raining == 1u) { d1 += p.dt * st[i].w; }
    tmp[i].x = d1;
    let hc = st[i].x + d1;
    var o = vec4<f32>(0.0);
    let k = p.dt * p.pipe * p.grav / p.l;
    // Neighbour water is d + rain (same as d1 there).
    if (x > 0u) { let j = ix(x - 1u, y); let dj = st[j].y + select(0.0, p.dt * st[j].w, p.raining == 1u); o.x = max(0.0, fl[i].x + k * (hc - st[j].x - dj)); }
    if (x + 1u < p.w) { let j = ix(x + 1u, y); let dj = st[j].y + select(0.0, p.dt * st[j].w, p.raining == 1u); o.y = max(0.0, fl[i].y + k * (hc - st[j].x - dj)); }
    if (y > 0u) { let j = ix(x, y - 1u); let dj = st[j].y + select(0.0, p.dt * st[j].w, p.raining == 1u); o.z = max(0.0, fl[i].z + k * (hc - st[j].x - dj)); }
    if (y + 1u < p.h) { let j = ix(x, y + 1u); let dj = st[j].y + select(0.0, p.dt * st[j].w, p.raining == 1u); o.w = max(0.0, fl[i].w + k * (hc - st[j].x - dj)); }
    let sum = o.x + o.y + o.z + o.w;
    if (sum > 0.0) {
        o *= min(1.0, d1 * p.l * p.l / (sum * p.dt));
    }
    fo[i] = o;
}

fn fget(x: i32, y: i32, k: u32) -> f32 {
    if (x < 0 || y < 0 || x >= i32(p.w) || y >= i32(p.h)) { return 0.0; }
    return fo[ix(u32(x), u32(y))][k];
}

@compute @workgroup_size(16, 16)
fn water_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (!inb(gid)) { return; }
    let x = i32(gid.x); let y = i32(gid.y);
    let i = ix(gid.x, gid.y);
    let f = fo[i];
    let inflow = fget(x - 1, y, 1u) + fget(x + 1, y, 0u) + fget(x, y - 1, 3u) + fget(x, y + 1, 2u);
    let outflow = f.x + f.y + f.z + f.w;
    let d1 = tmp[i].x;
    let d2 = max(0.0, d1 + p.dt * (inflow - outflow) / (p.l * p.l));
    tmp[i].y = d2;
    let wx = (fget(x - 1, y, 1u) - f.x + f.y - fget(x + 1, y, 0u)) * 0.5;
    let wy = (fget(x, y - 1, 3u) - f.z + f.w - fget(x, y + 1, 2u)) * 0.5;
    let dm = 0.5 * (d1 + d2);
    var v = vec2<f32>(0.0);
    if (dm > 1e-5) { v = vec2<f32>(wx, wy) / (p.l * dm); }
    ve[i] = vec4<f32>(v, ve[i].z, ve[i].w);
}

@compute @workgroup_size(16, 16)
fn erode_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (!inb(gid)) { return; }
    let x = gid.x; let y = gid.y;
    let i = ix(x, y);
    let bl = st[ix(max(x, 1u) - 1u, y)].x;
    let br = st[ix(min(x + 1u, p.w - 1u), y)].x;
    let bt = st[ix(x, max(y, 1u) - 1u)].x;
    let bb = st[ix(x, min(y + 1u, p.h - 1u))].x;
    let g = vec2<f32>(br - bl, bb - bt) / (2.0 * p.l);
    let g2 = dot(g, g);
    let sin_a = max(sqrt(g2 / (1.0 + g2)), p.min_tilt);
    let v = ve[i].xy;
    let d2 = tmp[i].y;
    let cap = p.cap * sin_a * length(v) * min(d2 / p.max_depth, 1.0);
    let s = st[i].z;
    var b2 = st[i].x;
    var s2 = s;
    if (cap > s) {
        // Never dig below the lowest neighbour (trenches).
        let floor = max(st[i].x - min(min(bl, br), min(bt, bb)), 0.0);
        let a = min(p.dt * p.susp * (cap - s), floor * 0.5);
        b2 -= a; s2 += a;
        ve[i].z += a;
    } else {
        let a = p.dt * p.depo * (s - cap);
        b2 += a; s2 -= a;
        ve[i].w += a;
    }
    tmp[i].z = b2;
    tmp[i].w = max(s2, 0.0);
}

// Fraction of cell j's water leaving through pipe k this step.
fn frac(j: u32, k: u32) -> f32 {
    let d1 = tmp[j].x;
    if (d1 <= 1e-6) { return 0.0; }
    return min(fo[j][k] * p.dt / (d1 * p.l * p.l), 1.0);
}

@compute @workgroup_size(16, 16)
fn transport_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (!inb(gid)) { return; }
    let x = gid.x; let y = gid.y;
    let i = ix(x, y);
    let leave = min(frac(i, 0u) + frac(i, 1u) + frac(i, 2u) + frac(i, 3u), 1.0);
    var s = tmp[i].w * (1.0 - leave);
    if (x > 0u) { let j = ix(x - 1u, y); s += tmp[j].w * frac(j, 1u); }
    if (x + 1u < p.w) { let j = ix(x + 1u, y); s += tmp[j].w * frac(j, 0u); }
    if (y > 0u) { let j = ix(x, y - 1u); s += tmp[j].w * frac(j, 3u); }
    if (y + 1u < p.h) { let j = ix(x, y + 1u); s += tmp[j].w * frac(j, 2u); }
    st[i].z = s;
    st[i].y = tmp[i].y * (1.0 - p.evap * p.dt);
}

@compute @workgroup_size(16, 16)
fn give_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (!inb(gid)) { return; }
    let x = gid.x; let y = gid.y;
    let i = ix(x, y);
    let b = tmp[i].z;
    let th = p.talus * p.l;
    var ex = vec4<f32>(0.0);
    var maxd = 0.0;
    if (x > 0u) { let dh = b - tmp[ix(x - 1u, y)].z; if (dh > th) { ex.x = dh - th; maxd = max(maxd, dh); } }
    if (x + 1u < p.w) { let dh = b - tmp[ix(x + 1u, y)].z; if (dh > th) { ex.y = dh - th; maxd = max(maxd, dh); } }
    if (y > 0u) { let dh = b - tmp[ix(x, y - 1u)].z; if (dh > th) { ex.z = dh - th; maxd = max(maxd, dh); } }
    if (y + 1u < p.h) { let dh = b - tmp[ix(x, y + 1u)].z; if (dh > th) { ex.w = dh - th; maxd = max(maxd, dh); } }
    let total = ex.x + ex.y + ex.z + ex.w;
    var o = vec4<f32>(0.0);
    if (total > 0.0) {
        o = ex / total * (p.dt * p.thermal * (maxd - th) * 0.5);
    }
    gv[i] = o;
}

fn gget(x: i32, y: i32, k: u32) -> f32 {
    if (x < 0 || y < 0 || x >= i32(p.w) || y >= i32(p.h)) { return 0.0; }
    return gv[ix(u32(x), u32(y))][k];
}

@compute @workgroup_size(16, 16)
fn apply_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (!inb(gid)) { return; }
    let x = i32(gid.x); let y = i32(gid.y);
    let i = ix(gid.x, gid.y);
    let g = gv[i];
    let inflow = gget(x + 1, y, 0u) + gget(x - 1, y, 1u) + gget(x, y + 1, 2u) + gget(x, y - 1, 3u);
    st[i].x = tmp[i].z - (g.x + g.y + g.z + g.w) + inflow;
    fl[i] = fo[i];
}

// Parallel Wave Function Collapse on the GPU (simple tiled model, ≤ 32 tiles).
//
// Every cell holds its superposition as a u32 bitset. One *round* is:
//   propagate ×K  — Jacobi sweeps (A → B → … → B): each cell keeps only the tiles some option of
//                   every neighbour allows. Ping-pong buffers make each sweep deterministic.
//   observe       — every undecided cell that is the strict minimum (fewest options, hashed
//                   tie-break) in its (2·window+1)² neighbourhood collapses, by integer weight.
//                   Minima are > window cells apart, so thousands collapse at once.
//   mark + reset  — cells that hit a contradiction (no options) re-open a patch around them to
//                   its initial domain (radius grows while contradictions persist): local
//                   repair instead of restarting the whole grid.
// Integer-only maths (no floats, no logs), so a given seed gives bit-identical results on every
// GPU and backend.

struct P {
    w: u32,
    h: u32,
    nt: u32,
    seed: u32,
    round: u32,
    radius: u32,
    window: u32,
    _b: u32,
};

@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read_write> ca: array<u32>;
@group(0) @binding(2) var<storage, read_write> cb: array<u32>;
@group(0) @binding(3) var<storage, read> init: array<u32>;
@group(0) @binding(4) var<storage, read> compat: array<u32>;   // [dir * 32 + tile]
@group(0) @binding(5) var<storage, read> weights: array<u32>;  // [cell * nt + tile]
@group(0) @binding(6) var<storage, read_write> mark: array<u32>;
@group(0) @binding(7) var<storage, read_write> counters: array<atomic<u32>, 4>;

const DX = array<i32, 4>(0, 1, 0, -1);
const DZ = array<i32, 4>(-1, 0, 1, 0);

fn hash(a: u32, b: u32, c: u32) -> u32 {
    var h = a * 0x8da6b343u ^ b * 0xd8163841u ^ c * 0xcb1ab31fu ^ p.seed * 0x27d4eb2du;
    h ^= h >> 15u;
    h *= 0x2c1b3c6du;
    h ^= h >> 12u;
    h *= 0x297a2d39u;
    return h ^ (h >> 15u);
}

fn allowed_by(nb: u32, dir: u32) -> u32 {
    // Tiles allowed next to neighbour `nb` when the neighbour lies on side `dir` of us: the
    // neighbour sees us on its opposite side.
    var a = 0u;
    var bits = nb;
    let od = (dir + 2u) % 4u;
    loop {
        if (bits == 0u) { break; }
        let t = firstTrailingBit(bits);
        bits &= bits - 1u;
        a |= compat[od * 32u + t];
    }
    return a;
}

fn propagate(src_is_a: bool, gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) { return; }
    let i = gid.y * p.w + gid.x;
    var c: u32;
    if (src_is_a) { c = ca[i]; } else { c = cb[i]; }
    var m = c;
    if (m != 0u) {
        for (var d = 0u; d < 4u; d++) {
            let nx = i32(gid.x) + DX[d];
            let nz = i32(gid.y) + DZ[d];
            if (nx < 0 || nz < 0 || nx >= i32(p.w) || nz >= i32(p.h)) { continue; }
            let j = u32(nz) * p.w + u32(nx);
            var nb: u32;
            if (src_is_a) { nb = ca[j]; } else { nb = cb[j]; }
            // A contradicted neighbour constrains nothing (it's about to be repaired); otherwise
            // its emptiness would flood outward one cell per sweep.
            if (nb == 0u) { continue; }
            m &= allowed_by(nb, d);
        }
    }
    if (src_is_a) { cb[i] = m; } else { ca[i] = m; }
}

@compute @workgroup_size(16, 16)
fn prop_ab(@builtin(global_invocation_id) gid: vec3<u32>) { propagate(true, gid); }

@compute @workgroup_size(16, 16)
fn prop_ba(@builtin(global_invocation_id) gid: vec3<u32>) { propagate(false, gid); }

/// Priority: fewer options first; a per-round hash breaks ties (and spreads the collapses).
fn key(j: u32) -> u32 {
    return (countOneBits(cb[j]) << 24u) | (hash(j, p.round, 7u) >> 8u);
}

// Reads the snapshot B (the last sweep's output) and writes A, so no thread ever sees another
// thread's collapse from the same dispatch: deterministic on every GPU.
@compute @workgroup_size(16, 16)
fn observe(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) { return; }
    let i = gid.y * p.w + gid.x;
    let c = cb[i];
    ca[i] = c;
    if (countOneBits(c) <= 1u) { return; }
    let k = key(i);
    let r = i32(p.window);
    for (var dz = -r; dz <= r; dz++) {
        for (var dx = -r; dx <= r; dx++) {
            if (dx == 0 && dz == 0) { continue; }
            let nx = i32(gid.x) + dx;
            let nz = i32(gid.y) + dz;
            if (nx < 0 || nz < 0 || nx >= i32(p.w) || nz >= i32(p.h)) { continue; }
            let j = u32(nz) * p.w + u32(nx);
            if (countOneBits(cb[j]) <= 1u) { continue; }
            let kj = key(j);
            if (kj < k || (kj == k && j < i)) { return; }
        }
    }
    // Local minimum: collapse by weight.
    var total = 0u;
    var bits = c;
    loop {
        if (bits == 0u) { break; }
        let t = firstTrailingBit(bits);
        bits &= bits - 1u;
        total += weights[i * p.nt + t];
    }
    var pick = firstTrailingBit(c);
    if (total > 0u) {
        var r = hash(i, p.round, 13u) % total;
        bits = c;
        loop {
            if (bits == 0u) { break; }
            let t = firstTrailingBit(bits);
            bits &= bits - 1u;
            let w = weights[i * p.nt + t];
            if (r < w) { pick = t; break; }
            r -= w;
        }
    }
    ca[i] = 1u << pick;
}

@compute @workgroup_size(16, 16)
fn mark_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) { return; }
    let i = gid.y * p.w + gid.x;
    mark[i] = select(0u, 1u, ca[i] == 0u);
}

@compute @workgroup_size(16, 16)
fn reset_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) { return; }
    let i = gid.y * p.w + gid.x;
    let r = i32(p.radius);
    // Undecided cells further out re-open too: their options were narrowed by decisions that
    // are now undone, and propagation only ever narrows — it would never give them back.
    let undecided = countOneBits(ca[i]) > 1u;
    let r2 = select(r, r + 6, undecided);
    for (var dz = -r2; dz <= r2; dz++) {
        for (var dx = -r2; dx <= r2; dx++) {
            let nx = i32(gid.x) + dx;
            let nz = i32(gid.y) + dz;
            if (nx < 0 || nz < 0 || nx >= i32(p.w) || nz >= i32(p.h)) { continue; }
            if (mark[u32(nz) * p.w + u32(nx)] != 0u) {
                ca[i] = init[i];
                return;
            }
        }
    }
}

@compute @workgroup_size(16, 16)
fn count_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) { return; }
    let i = gid.y * p.w + gid.x;
    let n = countOneBits(ca[i]);
    if (n == 0u) { atomicAdd(&counters[1], 1u); }
    if (n > 1u) { atomicAdd(&counters[0], 1u); }
}

// Pass 2 — shade: G-buffer → lit HDR, in one deterministic pass.
//
// Nothing here is random, so a still camera produces the same image every frame and a moving
// one doesn't "boil": TAA only has to remove aliasing, not noise.
//   sun      hard shadow ray traced through the DAG (+ instances)
//   ambient  sky light × cone-traced occlusion read straight from the DAG's coverage pyramid
//            (each node stores how full it is), from contact scale (10 cm) to street scale (4 m)
//   specular GGX sun highlight; mirror rays only for water, glass and polished metal
//   fog      height fog with sky-coloured in-scattering

@group(1) @binding(0) var g_pos: texture_2d<f32>;
@group(1) @binding(1) var g_nrm: texture_2d<f32>;
@group(1) @binding(2) var g_alb: texture_2d<f32>;
@group(1) @binding(3) var g_mat: texture_2d<u32>;
@group(1) @binding(4) var o_hdr: texture_storage_2d<rgba16float, write>;
@group(1) @binding(5) var s_vis: texture_2d<f32>;

/// Sun visibility from the half-resolution shadow pass: the 4 nearest samples, weighted by
/// how close their surface point and normal are to ours. None close enough (depth edges,
/// thin objects): trace our own ray.
fn shadow_at(c: vec2<i32>, p: vec3<f32>, n: vec3<f32>, footprint: f32, off: vec3<f32>, lod_k: f32) -> f32 {
    let hc = vec2<f32>(c) * 0.5 - 0.25;
    let base = vec2<i32>(floor(hc));
    let f = hc - vec2<f32>(base);
    let hmax = (vec2<i32>(u.res.xy) + 1) / 2 - 1;
    var sum = 0.0;
    var wsum = 0.0;
    let tol = max(footprint * 3.0, 0.5);
    for (var k = 0; k < 4; k++) {
        let o = vec2<i32>(k & 1, k >> 1);
        let hcell = clamp(base + o, vec2<i32>(0), hmax);
        let full = min(hcell * 2, vec2<i32>(u.res.xy) - 1);
        let sp = textureLoad(g_pos, full, 0);
        let sn = textureLoad(g_nrm, full, 0).xyz;
        if (sp.w < 0.0) { continue; }
        // Same surface: on our tangent plane, close by, similar normal.
        let plane = abs(dot(sp.xyz - p, n));
        let near = length(sp.xyz - p);
        if (plane > tol || near > tol * 4.0 || dot(sn, n) < 0.9) { continue; }
        let wb = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
        sum += textureLoad(s_vis, hcell, 0).x * (wb + 1e-3);
        wsum += wb + 1e-3;
    }
    if (wsum < 0.3) {
        return sun_shadow(p + off * 2.0, max(lod_k * 8.0, 0.012));
    }
    return sum / wsum;
}

fn ggx(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, rough: f32) -> f32 {
    let h = normalize(v + l);
    let a = max(rough * rough, 0.002);
    let nh = max(dot(n, h), 0.0);
    let d = a * a / (PI * pow(nh * nh * (a * a - 1.0) + 1.0, 2.0));
    let nl = max(dot(n, l), 0.0);
    let nv = max(dot(n, v), 1e-3);
    let k = a * 0.5;
    let g = (nl / (nl * (1.0 - k) + k)) * (nv / (nv * (1.0 - k) + k));
    return d * g / max(4.0 * nl * nv, 1e-3) * nl;
}

fn fog(col: vec3<f32>, rd: vec3<f32>, dist_vox: f32, origin_y: f32) -> vec3<f32> {
    let d = dist_vox / 32.0;                       // metres
    let h = max(origin_y / 32.0, 0.0);
    let dens = u.params.y;
    let falloff = 0.012;
    let k = dens * exp(-h * falloff) * select(d, (1.0 - exp(-falloff * rd.y * d)) / (falloff * rd.y), abs(rd.y) > 1e-4);
    let f = 1.0 - exp(-max(k, 0.0));
    let sd = normalize(u.sun_dir.xyz);
    let mu = max(dot(rd, sd), 0.0);
    let inscatter = sky(normalize(vec3<f32>(rd.x, 0.12, rd.z)), 0) * 0.9 + sun_color() * pow(mu, 12.0) * 0.12;
    return mix(col, inscatter, f);
}

// Ambient occlusion by cone tracing the coverage pyramid: 4 cones around the normal, 4 steps
// each, every step reading one node whose size matches the cone's width there.
// The samples are all near the shading point, so instead of descending from the root for each
// one, we record the shading point's path once and start each lookup from the deepest node the
// sample shares with it (usually saving 5 of 8 levels).
var<private> anc: array<u32, 12>;
var<private> anc_low: u32;
var<private> anc_ip: vec3<i32>;

fn record_path(p: vec3<f32>) {
    anc_ip = vec3<i32>(floor(p));
    var node = u.world.x;
    var l = u.world.y;
    anc[l] = node;
    anc_low = l;
    if (any(anc_ip < vec3<i32>(0)) || any(anc_ip >= vec3<i32>(i32(u.world.z)))) { return; }
    loop {
        if (l <= 1u) { break; }
        let s = 2u * (l - 1u);
        let c3 = (anc_ip >> vec3<u32>(s)) & vec3<i32>(3);
        let ci = u32(c3.x | (c3.y << 2u) | (c3.z << 4u));
        if (!mask_bit(node, ci)) { break; }
        node = child_ptr(node, ci);
        l -= 1u;
        anc[l] = node;
        anc_low = l;
    }
}

fn coverage_near(q: vec3<f32>, level: u32) -> f32 {
    let iq = vec3<i32>(floor(q));
    if (any(iq < vec3<i32>(0)) || any(iq >= vec3<i32>(i32(u.world.z)))) { return 0.0; }
    // Deepest recorded ancestor that also contains q.
    var l = anc_low;
    loop {
        if (l >= u.world.y) { break; }
        let sh = vec3<u32>(2u * l);
        if (all((iq >> sh) == (anc_ip >> sh))) { break; }
        l += 1u;
    }
    var node = anc[l];
    loop {
        let s = 2u * (l - 1u);
        let c3 = (iq >> vec3<u32>(s)) & vec3<i32>(3);
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

fn cone_ao(p: vec3<f32>, n: vec3<f32>) -> f32 {
    record_path(p + n * 1.5);
    let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.9);
    let t = normalize(cross(up, n));
    let b = cross(n, t);
    var vis = 0.0;
    for (var c = 0; c < 4; c++) {
        let a = f32(c) * 1.5708 + 0.785;
        let dir = normalize(n + (t * cos(a) + b * sin(a)) * 0.75);
        var occ = 0.0;
        var dist = 3.0;
        for (var k = 0; k < 4; k++) {
            // Node size ≤ half the distance travelled, so it never reaches back into the surface.
            let level = u32(clamp(floor(log2(dist * 0.5) * 0.5), 0.0, 4.0));
            let q = p + n * 1.5 + dir * dist;
            occ += (1.0 - occ) * coverage_near(q, level) * 0.85;
            dist *= 3.2;
        }
        vis += 1.0 - occ;
    }
    return vis * 0.25;
}

@compute @workgroup_size(8, 8)
fn shade_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (f32(gid.x) >= u.res.x || f32(gid.y) >= u.res.y) { return; }
    let c = vec2<i32>(gid.xy);
    let gp = textureLoad(g_pos, c, 0);
    if (gp.w < 0.0) {
        textureStore(o_hdr, c, vec4<f32>(sky_with_sun(normalize(gp.xyz)), 1.0));
        return;
    }
    let ne = textureLoad(g_nrm, c, 0);
    let n = ne.xyz;
    let ar = textureLoad(g_alb, c, 0);
    let albedo = ar.rgb;
    let rough = ar.w;
    let mat = textureLoad(g_mat, c, 0).x;
    let p = gp.xyz;
    let v = normalize(u.cam_pos.xyz - p);
    let rd = -v;
    var metal = 0.0;
    var flags = 0u;
    if (mat != MAT_LOD) {
        metal = mats[mat].metalness;
        flags = mats[mat].flags;
    }
    let sd = normalize(u.sun_dir.xyz);
    let lod_k = u.params.x;
    // Start secondary rays off the surface by about one pixel's footprint: a distant pixel
    // covers many voxels, and stepping off by less makes sub-pixel stairs shadow themselves
    // (stripes on slopes under a low sun).
    let off = n * (0.05 + gp.w * lod_k * 1.5);

    // Direct sun.
    let ndl = max(dot(n, sd), 0.0);
    var shadow = 0.0;
    if (ndl > 0.0) {
        // Coarser occluders far away: sub-pixel bumps shouldn't speckle a distant slope.
        shadow = 1.0;
        if (u.world.w != 102u) { shadow = shadow_at(c, p, n, gp.w * lod_k, off, lod_k); }
    }
    let sun = sun_color();

    // Ambient: sky over the hemisphere + light bounced off the (sunlit) ground, occluded.
    var ao = 1.0;
    if (u.world.w != 101u) { ao = cone_ao(p, n); }
    // Sunlight bounced off nearby sunlit ground and walls: strongest for surfaces facing down
    // or sideways, a little for everything (keeps shadows from going cold blue).
    let bounce = sun * max(sd.y, 0.0) * vec3<f32>(0.11, 0.1, 0.085) * (0.3 + 0.7 * clamp(0.5 - n.y * 0.5, 0.0, 1.0));
    let ambient = (sky_ambient(n) * 1.35 + bounce * 1.3 + moon_light()) * ao;

    let f0 = mix(vec3<f32>(0.04), albedo, metal);
    let nv = max(dot(n, v), 0.0);
    let fres = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - nv, 5.0) * (1.0 - rough);

    var diffuse = sun * ndl * shadow + ambient;
    if ((flags & F_FOLIAGE) != 0u) {
        // Leaves let some light through.
        diffuse += sun * max(dot(-n, sd), 0.0) * 0.2 * ao;
    }
    var col = albedo * diffuse * (1.0 - metal);
    if (ndl > 0.0) {
        col += sun * ggx(n, v, sd, max(rough, 0.06)) * fres * shadow;
    }

    // Reflections: a single mirror ray for water, glass and polished metal; sky light otherwise.
    let r = reflect(rd, n);
    var env = sky_ambient(r) * ao;
    let mirror = ((flags & (F_WATER | F_GLASS)) != 0u || (metal > 0.5 && rough < 0.25)) && u.world.w != 103u && gp.w < 2000.0;
    if (mirror) {
        let h = trace(p + off, r, 6000.0, lod_k * 4.0, 0.5, 96u);
        if (h.hit) {
            let a2 = hit_albedo(h.mat, h.avg, h.voxel, h.tint);
            var e2 = 0.0;
            if (h.mat != MAT_LOD) { e2 = mats[h.mat].emission * emission_scale(); }
            env = a2 * (sun * max(dot(h.n, sd), 0.0) * 0.7 + sky_ambient(h.n) * 0.6 + moon_light()) + a2 * e2;
            env = fog(env, r, h.t, p.y);
        } else {
            env = sky_with_sun(r);
        }
    }
    col += env * fres;
    if ((flags & F_GLASS) != 0u) {
        // Glass: reflection over a dark, slightly tinted interior.
        col = mix(col, env * fres + albedo * 0.03 * ambient, 0.65);
    }

    col += albedo * ne.w; // emission
    col = fog(col, rd, gp.w, u.cam_pos.y);
    textureStore(o_hdr, c, vec4<f32>(finite3(col), 1.0));
}

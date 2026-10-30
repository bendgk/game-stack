// Pass 5 — composite: materials × lighting, specular (GGX sun highlight + ray-traced
// reflections on smooth surfaces), emission, sky, and aerial-perspective fog → HDR.

@group(1) @binding(0) var g_pos: texture_2d<f32>;
@group(1) @binding(1) var g_nrm: texture_2d<f32>;
@group(1) @binding(2) var g_alb: texture_2d<f32>;
@group(1) @binding(3) var g_mat: texture_2d<u32>;
@group(1) @binding(4) var l_direct: texture_2d<f32>;
@group(1) @binding(5) var l_gi: texture_2d<f32>;
@group(1) @binding(6) var o_hdr: texture_storage_2d<rgba16float, write>;

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
    // Height fog: denser near the ground, integrated along the ray.
    let falloff = 0.012;
    let k = dens * exp(-h * falloff) * select(d, (1.0 - exp(-falloff * rd.y * d)) / (falloff * rd.y), abs(rd.y) > 1e-4);
    let f = 1.0 - exp(-max(k, 0.0));
    let sd = normalize(u.sun_dir.xyz);
    let mu = max(dot(rd, sd), 0.0);
    let inscatter = sky(normalize(vec3<f32>(rd.x, 0.12, rd.z)), 0) * 0.9 + sun_color() * pow(mu, 12.0) * 0.12;
    return mix(col, inscatter, f);
}

@compute @workgroup_size(8, 8)
fn composite_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (f32(gid.x) >= u.res.x || f32(gid.y) >= u.res.y) { return; }
    let c = vec2<i32>(gid.xy);
    let gp = textureLoad(g_pos, c, 0);
    if (gp.w < 0.0) {
        let rd = normalize(gp.xyz);
        textureStore(o_hdr, c, vec4<f32>(sky_with_sun(rd), 1.0));
        return;
    }
    let ne = textureLoad(g_nrm, c, 0);
    let n = ne.xyz;
    let ar = textureLoad(g_alb, c, 0);
    let albedo = ar.rgb;
    var rough = ar.w;
    let mat = textureLoad(g_mat, c, 0).x;
    let dl = textureLoad(l_direct, c, 0);
    let gi = textureLoad(l_gi, c, 0).rgb;
    let v = normalize(u.cam_pos.xyz - gp.xyz);
    let rd = -v;
    var metal = 0.0;
    var flags = 0u;
    if (mat != MAT_LOD) {
        metal = mats[mat].metalness;
        flags = mats[mat].flags;
    }
    let sd = normalize(u.sun_dir.xyz);
    let f0 = mix(vec3<f32>(0.04), albedo, metal);
    let nv = max(dot(n, v), 0.0);
    let fres = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - nv, 5.0) * (1.0 - rough * 0.7);

    // Diffuse.
    var diffuse_l = dl.rgb + gi;
    if ((flags & F_FOLIAGE) != 0u) {
        // Wrap lighting: leaves glow when backlit.
        diffuse_l += sun_color() * max(dot(-n, sd), 0.0) * 0.25 * dl.w;
    }
    var col = albedo * diffuse_l * (1.0 - metal) * (vec3<f32>(1.0) - fres * 0.5);

    // Sun highlight (shadowed by the same visibility as the direct term).
    let ndl = max(dot(n, sd), 0.0);
    if (ndl > 0.0) {
        let vis = clamp(luminance(dl.rgb) / max(luminance(sun_color() * ndl), 1e-4), 0.0, 1.0);
        col += sun_color() * ggx(n, v, sd, max(rough, 0.05)) * fres * vis;
    }

    // Reflections: trace for smooth surfaces, sky approximation for rough ones.
    var env = vec3<f32>(0.0);
    let r = reflect(rd, n);
    if (rough < 0.4) {
        var seed = pixel_seed(gid.xy, 5u);
        let rr = normalize(mix(r, cosine_dir(n, rand(&seed), rand(&seed)), rough * rough));
        let p = gp.xyz + n * (0.02 + gp.w * 0.002);
        let h = trace(p, rr, 4096.0, u.params.x * 4.0, rand(&seed), 128u);
        if (h.hit) {
            let a2 = hit_albedo(h.mat, h.avg, h.voxel);
            var e2 = 0.0;
            if (h.mat != MAT_LOD) { e2 = mats[h.mat].emission * emission_scale(); }
            env = a2 * (sun_color() * max(dot(h.n, sd), 0.0) * 0.6 + sky_ambient(h.n) * 0.5 + moon_light()) + a2 * e2;
            env = fog(env, rr, h.t, p.y);
        } else {
            env = sky_with_sun(rr);
        }
    } else {
        env = sky_ambient(r) * (1.0 - rough * 0.5);
    }
    col += env * fres;

    // Glass: mostly reflection over a dark, slightly tinted interior.
    if ((flags & F_GLASS) != 0u) {
        col = mix(col, env * fres + albedo * 0.02 * diffuse_l, 0.6);
    }

    // Emission.
    col += albedo * ne.w;

    col = fog(col, rd, gp.w, u.cam_pos.y);
    textureStore(o_hdr, c, vec4<f32>(finite3(col), 1.0));
}

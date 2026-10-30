// Pass 2 — light: per pixel, one soft sun-shadow ray and one ray-traced GI bounce
// (whose hit is itself sun-shadowed). Noisy by design; accum + atrous clean it up.

@group(1) @binding(0) var g_pos: texture_2d<f32>;
@group(1) @binding(1) var g_nrm: texture_2d<f32>;
@group(1) @binding(2) var o_direct: texture_storage_2d<rgba16float, write>;
@group(1) @binding(3) var o_gi: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8)
fn light_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (f32(gid.x) >= u.res.x || f32(gid.y) >= u.res.y) { return; }
    let c = vec2<i32>(gid.xy);
    let gp = textureLoad(g_pos, c, 0);
    if (gp.w < 0.0) {
        textureStore(o_direct, c, vec4<f32>(0.0));
        textureStore(o_gi, c, vec4<f32>(0.0));
        return;
    }
    let n = textureLoad(g_nrm, c, 0).xyz;
    var seed = pixel_seed(gid.xy, 2u);
    let lod_k = u.params.x;
    // Offset off the surface; coarse (LOD) hits far away need a larger push.
    let p = gp.xyz + n * (0.02 + gp.w * 0.002);
    let sd = normalize(u.sun_dir.xyz);
    let ndl = max(dot(n, sd), 0.0);
    var vis = 0.0;
    if (ndl > 0.0) {
        vis = sun_visible(p, &seed, lod_k * 2.0);
    }
    let direct = sun_color() * ndl * vis + moon_light() * (0.5 + 0.5 * n.y);
    textureStore(o_direct, c, vec4<f32>(direct, vis));

    // One bounce of diffuse GI.
    let d = cosine_dir(n, rand(&seed), rand(&seed));
    let gh = trace(p, d, u.params.z, lod_k * 6.0, rand(&seed), 96u);
    var L = vec3<f32>(0.0);
    if (!gh.hit) {
        L = sky_ambient(d);
    } else {
        let q = p + d * gh.t;
        let a2 = hit_albedo(gh.mat, gh.avg, gh.voxel);
        var e2 = 0.0;
        if (gh.mat != MAT_LOD) { e2 = mats[gh.mat].emission * emission_scale(); }
        let ndl2 = max(dot(gh.n, sd), 0.0);
        var vis2 = 0.0;
        if (ndl2 > 0.0) {
            vis2 = sun_visible(q + gh.n * 0.05, &seed, lod_k * 8.0);
        }
        L = a2 * (sun_color() * ndl2 * vis2 + moon_light() + sky_ambient(gh.n) * 0.3) + a2 * e2;
    }
    textureStore(o_gi, c, vec4<f32>(min(L, vec3<f32>(64.0)), 1.0));
}

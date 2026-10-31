// Pass 1 — trace: primary rays → G-buffer. (Prepended with common.wgsl.)
// Frame order: sky LUT → trace → shade → taa → blit.
// group(1): 0 g_pos (rgba32f: hit xyz, t or -1), 1 g_nrm (rgba16f: n, emission), 2 g_alb
// (rgba16f: albedo, roughness), 3 g_mat (r32u: material id | lod << 16)

@group(1) @binding(0) var o_pos: texture_storage_2d<rgba32float, write>;
@group(1) @binding(1) var o_nrm: texture_storage_2d<rgba16float, write>;
@group(1) @binding(2) var o_alb: texture_storage_2d<rgba16float, write>;
@group(1) @binding(3) var o_mat: texture_storage_2d<r32uint, write>;

fn primary_ray(px: vec2<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(px.x * u.res.z * 2.0 - 1.0, 1.0 - px.y * u.res.w * 2.0) + u.jitter.xy;
    // Unproject on the near plane: the far plane is ~infinitely far (w → 0, NaN rays).
    let near = u.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
    return normalize(near.xyz / near.w - u.cam_pos.xyz);
}

@compute @workgroup_size(8, 8)
fn trace_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (f32(gid.x) >= u.res.x || f32(gid.y) >= u.res.y) { return; }
    let px = vec2<f32>(gid.xy) + 0.5;
    let rd = primary_ray(px);
    let lod_k = u.params.x;
    // Deterministic LOD: a distant node counts as solid when it's at least half full.
    let h = trace(u.cam_pos.xyz, rd, 1e7, lod_k, 0.5, 512u);
    let c = vec2<i32>(gid.xy);
    if (u.world.w == 99u) {
        textureStore(o_alb, c, vec4<f32>(select(0.0, 1.0, h.hit), h.t / 5000.0, f32(h.steps) / 200.0, 1.0));
        return;
    }
    if (!h.hit) {
        textureStore(o_pos, c, vec4<f32>(rd, -1.0));
        textureStore(o_nrm, c, vec4<f32>(0.0));
        textureStore(o_alb, c, vec4<f32>(0.0));
        textureStore(o_mat, c, vec4<u32>(0u));
        return;
    }
    let pos = u.cam_pos.xyz + rd * h.t;
    var rough = 0.9;
    var emis = 0.0;
    var n = h.n;
    if (h.mat != MAT_LOD) {
        let m = mats[h.mat];
        rough = m.roughness;
        emis = m.emission * emission_scale();
        if ((m.flags & F_WATER) != 0u && n.y > 0.5) {
            // Gentle swell (metre-scale, slow): water should read as calm, not psychedelic.
            let tm = u.cam_pos.w * 0.6;
            let q = pos.xz / 64.0;
            let wx = sin(q.x * 1.7 + q.y * 0.6 + tm) * 0.6 + sin(q.x * 3.9 - q.y * 2.1 - tm * 1.3) * 0.3;
            let wz = cos(q.y * 1.9 - q.x * 0.5 + tm * 0.8) * 0.6 + cos(q.y * 4.3 + q.x * 1.7 + tm * 1.1) * 0.3;
            n = normalize(vec3<f32>(wx * 0.012, 1.0, wz * 0.012));
        }
    }
    // Distant hits (a pixel spans more than ~2 voxels): blend toward the smooth
    // coverage-gradient normal.
    let footprint = h.t * lod_k;
    if (footprint > 1.0 && h.tint == NO_TINT) {
        // Normals from the coverage gradient over a region several pixels wide: from afar a
        // terrain's half-metre steps (or any sub-pixel relief) should shade as the slope they
        // form, not as a staircase of lit and unlit faces.
        let level = u32(clamp(floor(log2(footprint * 16.0) * 0.5), 1.0, 6.0));
        let sn = smooth_normal(pos - h.n * 0.5, level, n);
        n = normalize(mix(n, sn, clamp((footprint - 1.0) / 3.0, 0.0, 1.0)));
    }
    textureStore(o_pos, c, vec4<f32>(pos, h.t));
    textureStore(o_nrm, c, vec4<f32>(n, emis));
    textureStore(o_alb, c, vec4<f32>(hit_albedo_at(h.mat, h.avg, h.voxel, h.tint, h.t), rough));
    textureStore(o_mat, c, vec4<u32>(h.mat, 0u, 0u, 0u));
}

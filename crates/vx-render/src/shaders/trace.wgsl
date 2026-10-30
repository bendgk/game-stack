// Pass 1 — trace: primary rays → G-buffer. (Prepended with common.wgsl.)
// Frame order: trace → light → accum → atrous ×3 → composite → taa → blit.
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
    var seed = pixel_seed(gid.xy, 1u);
    let lod_k = u.params.x;
    let h = trace(u.cam_pos.xyz, rd, 1e7, lod_k, rand(&seed), 512u);
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
        if ((m.flags & F_WATER) != 0u) {
            // Animated ripples on water.
            let tm = u.cam_pos.w;
            let q = pos.xz * 0.05;
            n = normalize(vec3<f32>(sin(q.x * 3.1 + tm * 1.3) * 0.04 + sin(q.y * 2.3 - tm) * 0.03, 1.0, cos(q.y * 2.7 + tm * 1.1) * 0.04));
        }
    }
    textureStore(o_pos, c, vec4<f32>(pos, h.t));
    textureStore(o_nrm, c, vec4<f32>(n, emis));
    textureStore(o_alb, c, vec4<f32>(hit_albedo(h.mat, h.avg, h.voxel), rough));
    textureStore(o_mat, c, vec4<u32>(h.mat, 0u, 0u, 0u));
}

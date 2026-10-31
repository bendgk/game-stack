// Pass 2a — sun shadows at half resolution: one shadow ray per 2×2 pixels (the expensive part
// of shading, especially with a low sun). `shade` upsamples it edge-aware and traces its own
// ray where the half-res samples don't describe the pixel's surface.

@group(1) @binding(0) var g_pos: texture_2d<f32>;
@group(1) @binding(1) var g_nrm: texture_2d<f32>;
@group(1) @binding(2) var o_vis: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8)
fn shadow_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let half = (vec2<u32>(u.res.xy) + 1u) / 2u;
    if (gid.x >= half.x || gid.y >= half.y) { return; }
    let c = min(vec2<i32>(gid.xy) * 2, vec2<i32>(u.res.xy) - 1);
    let gp = textureLoad(g_pos, c, 0);
    var vis = 1.0;
    if (gp.w >= 0.0) {
        let n = textureLoad(g_nrm, c, 0).xyz;
        let sd = normalize(u.sun_dir.xyz);
        if (dot(n, sd) > 0.0 && u.world.w != 102u) {
            vis = sun_shadow(u.cam_pos.xyz + gp.xyz + n * (0.05 + gp.w * u.params.x * 3.0), max(u.params.x * 8.0, 0.012));
        }
    }
    textureStore(o_vis, vec2<i32>(gid.xy), vec4<f32>(vis, 0.0, 0.0, 0.0));
}

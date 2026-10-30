// Pass 3 — accum: temporal accumulation of direct + GI with reprojection and
// disocclusion rejection (position + normal tests against last frame's G-buffer).

@group(1) @binding(0) var g_pos: texture_2d<f32>;
@group(1) @binding(1) var g_nrm: texture_2d<f32>;
@group(1) @binding(2) var p_pos: texture_2d<f32>;     // previous frame
@group(1) @binding(3) var p_nrm: texture_2d<f32>;
@group(1) @binding(4) var cur_direct: texture_2d<f32>;
@group(1) @binding(5) var cur_gi: texture_2d<f32>;
@group(1) @binding(6) var hist_direct: texture_2d<f32>;
@group(1) @binding(7) var hist_gi: texture_2d<f32>;
@group(1) @binding(8) var o_direct: texture_storage_2d<rgba16float, write>;
@group(1) @binding(9) var o_gi: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8)
fn accum_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (f32(gid.x) >= u.res.x || f32(gid.y) >= u.res.y) { return; }
    let c = vec2<i32>(gid.xy);
    let gp = textureLoad(g_pos, c, 0);
    let cd = textureLoad(cur_direct, c, 0);
    let cg = textureLoad(cur_gi, c, 0);
    if (gp.w < 0.0) {
        textureStore(o_direct, c, vec4<f32>(0.0));
        textureStore(o_gi, c, vec4<f32>(0.0));
        return;
    }
    let n = textureLoad(g_nrm, c, 0).xyz;
    // Where was this surface point last frame?
    let clip = u.prev_view_proj * vec4<f32>(gp.xyz, 1.0);
    var hd = vec4<f32>(0.0);
    var hg = vec4<f32>(0.0);
    if (clip.w > 0.0) {
        let ndc = clip.xy / clip.w;
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        let pp = uv * u.res.xy - 0.5;
        let base = vec2<i32>(floor(pp));
        let f = pp - floor(pp);
        // Bilinear over the 4 neighbours that pass the geometry tests.
        var wsum = 0.0;
        for (var j = 0; j < 4; j++) {
            let o = vec2<i32>(j & 1, j >> 1);
            let q = base + o;
            if (q.x < 0 || q.y < 0 || f32(q.x) >= u.res.x || f32(q.y) >= u.res.y) { continue; }
            let ppos = textureLoad(p_pos, q, 0);
            let pn = textureLoad(p_nrm, q, 0).xyz;
            let tol = max(1.0, gp.w * 0.01);
            if (ppos.w < 0.0 || distance(ppos.xyz, gp.xyz) > tol || dot(pn, n) < 0.9) { continue; }
            let bw = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
            hd += textureLoad(hist_direct, q, 0) * bw;
            hg += textureLoad(hist_gi, q, 0) * bw;
            wsum += bw;
        }
        if (wsum > 1e-3) {
            hd /= wsum;
            hg /= wsum;
        } else {
            hd = vec4<f32>(0.0);
            hg = vec4<f32>(0.0);
        }
    }
    // History length lives in .w; direct keeps a shorter memory (sharp moving shadows).
    let nd = min(hd.w + 1.0, 12.0);
    let ng = min(hg.w + 1.0, 64.0);
    let d = finite3(mix(finite3(hd.rgb), finite3(cd.rgb), 1.0 / nd));
    let g = finite3(mix(finite3(hg.rgb), finite3(cg.rgb), 1.0 / ng));
    textureStore(o_direct, c, vec4<f32>(d, nd));
    textureStore(o_gi, c, vec4<f32>(g, ng));
}

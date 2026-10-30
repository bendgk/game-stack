// Pass 3 — TAA: reproject last frame's resolved image (surfaces by world position, sky by
// direction), clamp it to the current 3×3 neighbourhood in YCoCg, blend. Shading is
// deterministic, so this only resolves the subpixel jitter (anti-aliasing).

@group(1) @binding(0) var g_pos: texture_2d<f32>;
@group(1) @binding(1) var hdr: texture_2d<f32>;
@group(1) @binding(2) var hist: texture_2d<f32>;
@group(1) @binding(3) var samp: sampler;
@group(1) @binding(4) var o_taa: texture_storage_2d<rgba16float, write>;

fn to_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(c.r * 0.25 + c.g * 0.5 + c.b * 0.25, c.r * 0.5 - c.b * 0.5, -c.r * 0.25 + c.g * 0.5 - c.b * 0.25);
}
fn from_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

@compute @workgroup_size(8, 8)
fn taa_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (f32(gid.x) >= u.res.x || f32(gid.y) >= u.res.y) { return; }
    let c = vec2<i32>(gid.xy);
    let cur = textureLoad(hdr, c, 0).rgb;
    // Neighbourhood statistics (tonemapped-ish to tame fireflies).
    var m1 = vec3<f32>(0.0);
    var m2 = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let q = clamp(c + vec2<i32>(x, y), vec2<i32>(0), vec2<i32>(u.res.xy) - 1);
            let s = to_ycocg(textureLoad(hdr, q, 0).rgb / (1.0 + luminance(textureLoad(hdr, q, 0).rgb)));
            m1 += s;
            m2 += s * s;
        }
    }
    m1 /= 9.0;
    m2 /= 9.0;
    let sigma = sqrt(max(m2 - m1 * m1, vec3<f32>(0.0)));
    let lo = m1 - sigma * 1.25;
    let hi = m1 + sigma * 1.25;

    let gp = textureLoad(g_pos, c, 0);
    var clip: vec4<f32>;
    if (gp.w < 0.0) {
        clip = u.prev_view_proj * vec4<f32>(u.prev_cam_pos.xyz + gp.xyz * 1e5, 1.0);
    } else {
        clip = u.prev_view_proj * vec4<f32>(gp.xyz, 1.0);
    }
    var out = cur;
    if (clip.w > 0.0 && u.jitter.z > 0.5) {
        let ndc = clip.xy / clip.w;
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if (all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0))) {
            let h = textureSampleLevel(hist, samp, uv, 0.0).rgb;
            var hy = to_ycocg(h / (1.0 + luminance(h)));
            hy = clamp(hy, lo, hi);
            let hc = from_ycocg(hy);
            let hres = hc / max(1.0 - luminance(hc), 1e-3);
            let curt = cur / (1.0 + luminance(cur));
            let blended = mix(hc, curt, 0.1);
            out = blended / max(1.0 - luminance(blended), 1e-3);
            let _unused = hres;
        }
    }
    textureStore(o_taa, c, vec4<f32>(max(finite3(out), vec3<f32>(0.0)), 1.0));
}

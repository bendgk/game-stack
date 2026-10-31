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

/// History sampled with a 5-tap Catmull-Rom filter: bilinear sampling blurs a little more every
/// frame, which is what makes naive TAA look soft.
fn sample_history(uv: vec2<f32>) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(hist));
    let p = uv * size;
    let tc = floor(p - 0.5) + 0.5;
    let f = p - tc;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let o12 = w2 / w12;
    let t0 = (tc - 1.0) / size;
    let t3 = (tc + 2.0) / size;
    let t12 = (tc + o12) / size;
    var c = textureSampleLevel(hist, samp, vec2<f32>(t12.x, t0.y), 0.0).rgb * (w12.x * w0.y);
    c += textureSampleLevel(hist, samp, vec2<f32>(t0.x, t12.y), 0.0).rgb * (w0.x * w12.y);
    c += textureSampleLevel(hist, samp, vec2<f32>(t12.x, t12.y), 0.0).rgb * (w12.x * w12.y);
    c += textureSampleLevel(hist, samp, vec2<f32>(t3.x, t12.y), 0.0).rgb * (w3.x * w12.y);
    c += textureSampleLevel(hist, samp, vec2<f32>(t12.x, t3.y), 0.0).rgb * (w12.x * w3.y);
    let wsum = w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y;
    return max(c / wsum, vec3<f32>(0.0));
}

/// Pull `h` toward the neighbourhood mean until it lies inside the box `mean ± extent`
/// (variance clipping: unlike a per-channel clamp it keeps the history's hue).
fn clip_to_box(h: vec3<f32>, mean: vec3<f32>, extent: vec3<f32>) -> vec3<f32> {
    let d = h - mean;
    let r = abs(d) / max(extent, vec3<f32>(1e-5));
    let m = max(r.x, max(r.y, r.z));
    return select(h, mean + d / m, m > 1.0);
}

@compute @workgroup_size(8, 8)
fn taa_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (f32(gid.x) >= u.res.x || f32(gid.y) >= u.res.y) { return; }
    let c = vec2<i32>(gid.xy);
    let cur = textureLoad(hdr, c, 0).rgb;
    let curt = to_ycocg(cur / (1.0 + luminance(cur)));
    // Neighbourhood statistics in tonemapped YCoCg (tames fireflies).
    var m1 = vec3<f32>(0.0);
    var m2 = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let q = clamp(c + vec2<i32>(x, y), vec2<i32>(0), vec2<i32>(u.res.xy) - 1);
            let v = textureLoad(hdr, q, 0).rgb;
            let t = to_ycocg(v / (1.0 + luminance(v)));
            m1 += t;
            m2 += t * t;
        }
    }
    m1 /= 9.0;
    m2 /= 9.0;
    let sigma = sqrt(max(m2 - m1 * m1, vec3<f32>(0.0)));

    // Still view: converge to the exact jitter-averaged (super-sampled) image, so nothing
    // changes from frame to frame. Moving: track quickly with a tight box (no smearing).
    let still = f32(u.extra.y);
    let is_still = u.extra.y > 0u;
    let alpha = select(0.12, max(1.0 / (still + 2.0), 1.0 / 64.0), is_still);
    let gamma = select(1.0, 2.5, is_still);

    let gp = textureLoad(g_pos, c, 0);
    var clip: vec4<f32>;
    if (gp.w < 0.0) {
        clip = u.prev_view_proj * vec4<f32>(u.prev_cam_pos.xyz + gp.xyz * 1e5, 1.0);
    } else {
        clip = u.prev_view_proj * vec4<f32>(gp.xyz, 1.0);
    }
    // Motion = where this surface point was last frame minus where it is now, both without
    // jitter; history is read at *this pixel's centre* plus that motion. (Reprojecting the
    // jittered hit point instead re-samples history at a different sub-pixel offset every frame,
    // which slowly smears the image even when nothing moves.)
    var cur_clip: vec4<f32>;
    if (gp.w < 0.0) {
        cur_clip = u.view_proj * vec4<f32>(u.cam_pos.xyz + gp.xyz * 1e5, 1.0);
    } else {
        cur_clip = u.view_proj * vec4<f32>(gp.xyz, 1.0);
    }
    var outt = curt;
    if (clip.w > 0.0 && cur_clip.w > 0.0 && u.jitter.z > 0.5) {
        let to_uv = vec2<f32>(0.5, -0.5);
        let motion = (clip.xy / clip.w - cur_clip.xy / cur_clip.w) * to_uv;
        let uv = (vec2<f32>(gid.xy) + 0.5) * u.res.zw + motion;
        if (all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0))) {
            let h = sample_history(uv);
            let ht = clip_to_box(to_ycocg(h / (1.0 + luminance(h))), m1, sigma * gamma + vec3<f32>(0.002));
            outt = mix(ht, curt, alpha);
        }
    }
    let o = from_ycocg(outt);
    let res = o / max(1.0 - luminance(o), 1e-3);
    textureStore(o_taa, c, vec4<f32>(max(finite3(res), vec3<f32>(0.0)), 1.0));
}

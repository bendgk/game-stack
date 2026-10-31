// Pass 4 — temporal anti-aliasing with up-sampling (TAAU), run at the *output* resolution.
//
// Each frame the primary rays are jittered by a sub-pixel Halton offset. This pass:
//   1. reconstructs the current frame at this output pixel from the 3×3 traced samples around
//      it, each weighted by how close its (jittered) position is to the output pixel's centre
//      (a Blackman-Harris-like Gaussian), not just "the traced pixel underneath";
//   2. reprojects the output-resolution history along the motion of the *closest* surface in
//      that 3×3 (foreground edges keep their own motion; no background trails behind them);
//   3. clips the history toward the current neighbourhood (variance clipping in YCoCg) so
//      anything that changed can't ghost;
//   4. blends, trusting the new frame more when one of its samples landed right on this pixel.
// Over frames the jitter covers every sub-pixel position, so the output gains real detail
// beyond the traced resolution; a still view converges to the exact super-sampled image.

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
/// Reversible tonemap for filtering: bright outliers (fireflies, the sun) can't dominate.
fn compress(c: vec3<f32>) -> vec3<f32> {
    return to_ycocg(c / (1.0 + luminance(c)));
}
fn expand(t: vec3<f32>) -> vec3<f32> {
    let c = from_ycocg(t);
    return c / max(1.0 - luminance(c), 1e-3);
}

/// History sampled with a 5-tap Catmull-Rom filter: bilinear sampling blurs a little more every
/// frame, which is what makes naive TAA soft.
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

/// Pull `h` toward `mean` until it lies inside the box `mean ± extent` (keeps its hue, unlike a
/// per-channel clamp).
fn clip_to_box(h: vec3<f32>, mean: vec3<f32>, extent: vec3<f32>) -> vec3<f32> {
    let d = h - mean;
    let r = abs(d) / max(extent, vec3<f32>(1e-5));
    let m = max(r.x, max(r.y, r.z));
    return select(h, mean + d / m, m > 1.0);
}

@compute @workgroup_size(8, 8)
fn taa_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (f32(gid.x) >= u.out_res.x || f32(gid.y) >= u.out_res.y) { return; }
    let ouv = (vec2<f32>(gid.xy) + 0.5) * u.out_res.zw;
    // This output pixel's centre, in traced-pixel coordinates.
    let ip = ouv * u.res.xy;
    // Where this frame's samples actually are: traced pixel q sampled at q + 0.5 + jitter.
    let jpx = vec2<f32>(u.jitter.x * u.res.x * 0.5, -u.jitter.y * u.res.y * 0.5);
    let centre = vec2<i32>(floor(ip - jpx));
    let maxq = vec2<i32>(u.res.xy) - 1;

    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    var wmax = 0.0;
    var m1 = vec3<f32>(0.0);
    var m2 = vec3<f32>(0.0);
    var closest_t = 1e30;
    var closest = clamp(centre, vec2<i32>(0), maxq);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let q = clamp(centre + vec2<i32>(x, y), vec2<i32>(0), maxq);
            let t = compress(textureLoad(hdr, q, 0).rgb);
            let d = vec2<f32>(q) + 0.5 + jpx - ip;
            let w = exp(-2.29 * dot(d, d));
            sum += t * w;
            wsum += w;
            wmax = max(wmax, w);
            m1 += t;
            m2 += t * t;
            let gw = textureLoad(g_pos, q, 0).w;
            let depth = select(gw, 1e29, gw < 0.0);
            if (depth < closest_t) {
                closest_t = depth;
                closest = q;
            }
        }
    }
    let cur = sum / max(wsum, 1e-5);
    m1 /= 9.0;
    m2 /= 9.0;
    let sigma = sqrt(max(m2 - m1 * m1, vec3<f32>(0.0)));

    // Still view: converge to the exact jitter-averaged image (nothing changes frame to frame).
    // Moving: track with a tight box (no smear), more so where a sample landed right here.
    let still = f32(u.extra.y);
    let is_still = u.extra.y > 0u;
    let gamma = select(1.0, 2.5, is_still);

    // Motion of the closest surface around this pixel (sky: direction only).
    let gp = textureLoad(g_pos, closest, 0);
    var p4: vec4<f32>;
    var p4_prev: vec4<f32>;
    if (gp.w < 0.0) {
        p4 = vec4<f32>(gp.xyz * 1e5, 1.0);
        p4_prev = p4;
    } else {
        // Camera-relative throughout: g_pos is relative to this frame's camera, the matrices
        // have no translation, and the camera's own move between frames is an exact difference
        // of two nearby floats (no large-number rounding in the reprojection).
        p4 = vec4<f32>(gp.xyz, 1.0);
        p4_prev = vec4<f32>(gp.xyz + (u.cam_pos.xyz - u.prev_cam_pos.xyz), 1.0);
    }
    let cc = u.view_proj * p4;
    let pc = u.prev_view_proj * p4_prev;
    var out = cur;
    var moving = 1.0;
    if (cc.w > 0.0 && pc.w > 0.0 && u.jitter.z > 0.5) {
        let motion = (pc.xy / pc.w - cc.xy / cc.w) * vec2<f32>(0.5, -0.5);
        let huv = ouv + motion;
        // Screen motion in output pixels: every history resample softens a little, so pixels
        // that move a lot lean more on the new frame (and get sharpened more in `post`).
        let mpx = length(motion * u.out_res.xy);
        moving = clamp(mpx / 2.0, 0.0, 1.0);
        let base = select(0.1, max(1.0 / (still + 2.0), 1.0 / 64.0), is_still);
        let alpha = clamp(base * (0.35 + 1.3 * wmax), base * 0.35, 1.0);
        if (all(huv >= vec2<f32>(0.0)) && all(huv <= vec2<f32>(1.0))) {
            let h = clip_to_box(compress(sample_history(huv)), m1, sigma * gamma + vec3<f32>(0.002));
            out = mix(h, cur, alpha);
        }
    }
    // Alpha carries how much this pixel moved, for motion-adaptive sharpening in `post`.
    textureStore(o_taa, vec2<i32>(gid.xy), vec4<f32>(max(finite3(expand(out)), vec3<f32>(0.0)), moving));
}

// Pass 4 — post (at render resolution): contrast-adaptive sharpening + bloom, so the final
// upscale to the screen (which may have 4x the pixels on a Retina display) is a cheap 5-tap
// bicubic. Bloom strength comes from u.tint.w.

@group(1) @binding(0) var src: texture_2d<f32>;
@group(1) @binding(1) var o_post: texture_storage_2d<rgba16float, write>;

fn ld(c: vec2<i32>) -> vec3<f32> {
    return textureLoad(src, clamp(c, vec2<i32>(0), vec2<i32>(u.res.xy) - 1), 0).rgb;
}

@compute @workgroup_size(8, 8)
fn post_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (f32(gid.x) >= u.res.x || f32(gid.y) >= u.res.y) { return; }
    let c = vec2<i32>(gid.xy);
    var col = ld(c);
    // Sharpen less where local contrast is already high (no halos on hard edges).
    let n = ld(c + vec2<i32>(0, -1));
    let s = ld(c + vec2<i32>(0, 1));
    let e = ld(c + vec2<i32>(1, 0));
    let w = ld(c + vec2<i32>(-1, 0));
    let mn = min(col, min(min(n, s), min(e, w)));
    let mx = max(col, max(max(n, s), max(e, w)));
    let amp = sqrt(clamp(min(mn, 1.0 - min(mx, vec3<f32>(1.0))) / max(mx, vec3<f32>(1e-4)), vec3<f32>(0.0), vec3<f32>(1.0)));
    let wgt = -amp * 0.18;
    col = max((col + (n + s + e + w) * wgt) / (1.0 + 4.0 * wgt), vec3<f32>(0.0));
    // Bloom: wide taps, bright pixels only.
    var b = vec3<f32>(0.0);
    for (var i = 0; i < 12; i++) {
        let a = f32(i) * 0.5236;
        let r = 4.0 + f32(i % 3) * 5.0;
        let t = ld(c + vec2<i32>(vec2<f32>(cos(a), sin(a)) * r));
        b += max(t - vec3<f32>(4.0), vec3<f32>(0.0));
    }
    col += b / 12.0 * u.tint.w;
    textureStore(o_post, c, vec4<f32>(finite3(col), 1.0));
}

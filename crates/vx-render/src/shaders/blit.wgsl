// Pass 4 — present: exposure, a soft bloom tap, AgX tonemap, sRGB encode (when the target
// isn't an *-srgb format), and a little dither against banding.

struct Present {
    exposure: f32,
    encode_srgb: f32,
    bloom: f32,
    time: f32,
    mode: f32,       // 0 normal, 1 uv test, 2 raw source (no tonemap)
    _p0: f32,
    _p1: f32,
    _p2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> pp: Present;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var o: VsOut;
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}

// AgX (Troy Sobotka), polynomial approximation (Benjamin Wrensch).
fn agx_contrast(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

fn agx(c: vec3<f32>) -> vec3<f32> {
    let m = mat3x3<f32>(
        vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104),
    );
    let mi = mat3x3<f32>(
        vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116),
    );
    let min_ev = -12.47393;
    let max_ev = 4.026069;
    var x = m * max(c, vec3<f32>(1e-10));
    x = clamp(log2(x), vec3<f32>(min_ev), vec3<f32>(max_ev));
    x = (x - min_ev) / (max_ev - min_ev);
    x = max(agx_contrast(x), vec3<f32>(0.0));
    // Slight "punchy" look.
    let luma = dot(x, vec3<f32>(0.2126, 0.7152, 0.0722));
    x = luma + 1.1 * (pow(x, vec3<f32>(1.05)) - luma);
    x = mi * x;
    return pow(max(x, vec3<f32>(0.0)), vec3<f32>(2.2)); // back to linear
}

fn srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var c = textureSampleLevel(src, samp, in.uv, 0.0).rgb;
    if (pp.mode > 0.5 && pp.mode < 1.5) { return vec4<f32>(in.uv, 0.5, 1.0); }
    if (pp.mode > 1.5) { return vec4<f32>(c, 1.0); }
    // Contrast-adaptive sharpening: restores the crispness TAA and upscaling take away, less
    // where local contrast is already high (no halos on hard edges).
    let px = 1.0 / vec2<f32>(textureDimensions(src));
    let n = textureSampleLevel(src, samp, in.uv + vec2<f32>(0.0, -px.y), 0.0).rgb;
    let s = textureSampleLevel(src, samp, in.uv + vec2<f32>(0.0, px.y), 0.0).rgb;
    let e = textureSampleLevel(src, samp, in.uv + vec2<f32>(px.x, 0.0), 0.0).rgb;
    let w = textureSampleLevel(src, samp, in.uv + vec2<f32>(-px.x, 0.0), 0.0).rgb;
    let mn = min(c, min(min(n, s), min(e, w)));
    let mx = max(c, max(max(n, s), max(e, w)));
    let amp = sqrt(clamp(min(mn, 1.0 - min(mx, vec3<f32>(1.0))) / max(mx, vec3<f32>(1e-4)), vec3<f32>(0.0), vec3<f32>(1.0)));
    let wgt = -amp * 0.2; // sharpness
    c = max((c + (n + s + e + w) * wgt) / (1.0 + 4.0 * wgt), vec3<f32>(0.0));
    // Cheap bloom: wide taps from the mip-less source (bright pixels only).
    let d = vec2<f32>(textureDimensions(src));
    var b = vec3<f32>(0.0);
    for (var i = 0; i < 12; i++) {
        let a = f32(i) * 0.5236;
        let r = (6.0 + f32(i % 3) * 7.0) / d;
        let s = textureSampleLevel(src, samp, in.uv + vec2<f32>(cos(a), sin(a)) * r, 0.0).rgb;
        b += max(s - vec3<f32>(4.0), vec3<f32>(0.0));
    }
    c += b / 12.0 * pp.bloom;
    c = agx(c * pp.exposure);
    // Vignette.
    let q = in.uv - 0.5;
    c *= 1.0 - dot(q, q) * 0.35;
    if (pp.encode_srgb > 0.5) {
        c = srgb(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)));
        let n = fract(sin(dot(in.pos.xy + pp.time, vec2<f32>(12.9898, 78.233))) * 43758.5453);
        c += (n - 0.5) / 255.0;
    }
    return vec4<f32>(c, 1.0);
}

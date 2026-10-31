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

// Khronos PBR Neutral tone mapping: keeps hues and saturation faithful (base colours look like
// their albedo under neutral light) and only compresses the highlights.
fn pbr_neutral(c_in: vec3<f32>) -> vec3<f32> {
    let start = 0.8 - 0.04;
    let desat = 0.15;
    let x = min(c_in.r, min(c_in.g, c_in.b));
    let off = select(0.04, x - 6.25 * x * x, x < 0.08);
    var c = c_in - off;
    let peak = max(c.r, max(c.g, c.b));
    if (peak < start) { return c; }
    let d = 1.0 - start;
    let np = 1.0 - d * d / (peak + d - start);
    c *= np / peak;
    let g = 1.0 - 1.0 / (desat * (peak - np) + 1.0);
    return mix(c, vec3<f32>(np), g);
}

fn srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3<f32>(0.0031308));
}

/// Catmull-Rom (bicubic) sample: a sharp upscale from the render resolution to the screen's.
fn bicubic(uv: vec2<f32>) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(src));
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
    var c = textureSampleLevel(src, samp, vec2<f32>(t12.x, t0.y), 0.0).rgb * (w12.x * w0.y);
    c += textureSampleLevel(src, samp, vec2<f32>(t0.x, t12.y), 0.0).rgb * (w0.x * w12.y);
    c += textureSampleLevel(src, samp, vec2<f32>(t12.x, t12.y), 0.0).rgb * (w12.x * w12.y);
    c += textureSampleLevel(src, samp, vec2<f32>(t3.x, t12.y), 0.0).rgb * (w3.x * w12.y);
    c += textureSampleLevel(src, samp, vec2<f32>(t12.x, t3.y), 0.0).rgb * (w12.x * w3.y);
    let wsum = w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y;
    return max(c / wsum, vec3<f32>(0.0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (pp.mode > 0.5 && pp.mode < 1.5) { return vec4<f32>(in.uv, 0.5, 1.0); }
    var c = bicubic(in.uv);
    if (pp.mode > 1.5) { return vec4<f32>(c, 1.0); }
    // Contrast-adaptive sharpening (on the source grid), less where contrast is already high.
    let px = 1.0 / vec2<f32>(textureDimensions(src));
    let n = textureSampleLevel(src, samp, in.uv + vec2<f32>(0.0, -px.y), 0.0).rgb;
    let s = textureSampleLevel(src, samp, in.uv + vec2<f32>(0.0, px.y), 0.0).rgb;
    let e = textureSampleLevel(src, samp, in.uv + vec2<f32>(px.x, 0.0), 0.0).rgb;
    let w = textureSampleLevel(src, samp, in.uv + vec2<f32>(-px.x, 0.0), 0.0).rgb;
    let mn = min(c, min(min(n, s), min(e, w)));
    let mx = max(c, max(max(n, s), max(e, w)));
    let amp = sqrt(clamp(min(mn, 1.0 - min(mx, vec3<f32>(1.0))) / max(mx, vec3<f32>(1e-4)), vec3<f32>(0.0), vec3<f32>(1.0)));
    let wgt = -amp * 0.18;
    c = max((c + (n + s + e + w) * wgt) / (1.0 + 4.0 * wgt), vec3<f32>(0.0));
    // Bloom: wide taps, bright pixels only.
    var b = vec3<f32>(0.0);
    for (var i = 0; i < 12; i++) {
        let a = f32(i) * 0.5236;
        let r = (6.0 + f32(i % 3) * 7.0) * px;
        let t = textureSampleLevel(src, samp, in.uv + vec2<f32>(cos(a), sin(a)) * r, 0.0).rgb;
        b += max(t - vec3<f32>(4.0), vec3<f32>(0.0));
    }
    c += b / 12.0 * pp.bloom;
    c = pbr_neutral(c * pp.exposure);
    // Gentle vignette.
    let q = in.uv - 0.5;
    c *= 1.0 - dot(q, q) * 0.25;
    if (pp.encode_srgb > 0.5) {
        c = srgb(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)));
        // Static (not time-varying) dither against banding: animated dither reads as shimmer.
        let nz = fract(sin(dot(in.pos.xy, vec2<f32>(12.9898, 78.233))) * 43758.5453);
        c += (nz - 0.5) / 255.0;
    }
    return vec4<f32>(c, 1.0);
}

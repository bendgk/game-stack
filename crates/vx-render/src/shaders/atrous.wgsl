// Pass 4 — à-trous wavelet filter on accumulated GI (3 iterations: step 1, 2, 4), edge-stopped
// by normals and plane distance. Young history (few samples) is blurred more.

struct Step { step: vec4<i32> };

@group(1) @binding(0) var g_pos: texture_2d<f32>;
@group(1) @binding(1) var g_nrm: texture_2d<f32>;
@group(1) @binding(2) var src: texture_2d<f32>;
@group(1) @binding(3) var dst: texture_storage_2d<rgba16float, write>;
@group(1) @binding(4) var<uniform> st: Step;

const K: array<f32, 3> = array<f32, 3>(0.375, 0.25, 0.0625);

@compute @workgroup_size(8, 8)
fn atrous_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (f32(gid.x) >= u.res.x || f32(gid.y) >= u.res.y) { return; }
    let c = vec2<i32>(gid.xy);
    let center = textureLoad(src, c, 0);
    let gp = textureLoad(g_pos, c, 0);
    if (gp.w < 0.0) {
        textureStore(dst, c, center);
        return;
    }
    let n = textureLoad(g_nrm, c, 0).xyz;
    let samples = center.w;
    // Converged pixels need little filtering.
    let strength = clamp(8.0 / max(samples, 1.0), 0.15, 1.0);
    let s = st.step.x;
    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            let q = c + vec2<i32>(x, y) * s;
            if (q.x < 0 || q.y < 0 || f32(q.x) >= u.res.x || f32(q.y) >= u.res.y) { continue; }
            let qp = textureLoad(g_pos, q, 0);
            if (qp.w < 0.0) { continue; }
            let qn = textureLoad(g_nrm, q, 0).xyz;
            let wn = pow(max(dot(n, qn), 0.0), 64.0);
            let plane = abs(dot(qp.xyz - gp.xyz, n)) / max(1.0, gp.w * 0.004);
            let wp = exp(-plane);
            var k = K[abs(x)] * K[abs(y)];
            if (x != 0 || y != 0) { k *= strength; }
            let w = k * wn * wp;
            sum += textureLoad(src, q, 0).rgb * w;
            wsum += w;
        }
    }
    textureStore(dst, c, vec4<f32>(sum / max(wsum, 1e-6), samples));
}

// Pass 0 — skylut: evaluate the atmosphere once per frame into a 128×64 LUT (sqrt-mapped
// elevation for horizon detail). Every other pass samples it instead of ray-marching the sky.
// (Prepended with the Uniforms struct, its binding and atmo.wgsl — no scene bindings.)

@group(1) @binding(0) var o_lut: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8)
fn skylut_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<u32>(128u, 64u);
    if (gid.x >= dims.x || gid.y >= dims.y) { return; }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(dims);
    let az = (uv.x - 0.5) * 2.0 * PI;
    let s = (0.5 - uv.y) * 2.0;
    let el = sign(s) * s * s * 0.5 * PI;
    let d = vec3<f32>(cos(el) * cos(az), sin(el), cos(el) * sin(az));
    textureStore(o_lut, vec2<i32>(gid.xy), vec4<f32>(sky_raw(d, 16), 1.0));
}

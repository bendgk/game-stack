// Atmosphere: compact single-scattering Rayleigh + Mie. Needs `u` (Uniforms).

// ---------------------------------------------------------------------------------------------

const R_EARTH: f32 = 6360e3;
const R_ATMO: f32 = 6460e3;
const BETA_R: vec3<f32> = vec3<f32>(5.8e-6, 13.5e-6, 33.1e-6);
const BETA_M: f32 = 21e-6;
const H_R: f32 = 8000.0;
const H_M: f32 = 1200.0;

fn ray_sphere(ro: vec3<f32>, rd: vec3<f32>, r: f32) -> vec2<f32> {
    let b = dot(ro, rd);
    let c = dot(ro, ro) - r * r;
    let h = b * b - c;
    if (h < 0.0) { return vec2<f32>(-1.0, -1.0); }
    let s = sqrt(h);
    return vec2<f32>(-b - s, -b + s);
}

fn optical_depth(p: vec3<f32>, d: vec3<f32>) -> vec2<f32> {
    let tm = ray_sphere(p, d, R_ATMO).y;
    let n = 4;
    let ds = tm / f32(n);
    var od = vec2<f32>(0.0);
    for (var i = 0; i < n; i++) {
        let h = length(p + d * (ds * (f32(i) + 0.5))) - R_EARTH;
        od += exp(-vec2<f32>(h / H_R, h / H_M)) * ds;
    }
    return od;
}

fn transmittance(od: vec2<f32>) -> vec3<f32> {
    return exp(-(BETA_R * od.x + vec3<f32>(BETA_M * 1.1) * od.y));
}

/// Sun radiance reaching the ground (colour shifts to orange at low sun).
fn sun_color() -> vec3<f32> {
    let p = vec3<f32>(0.0, R_EARTH + 10.0, 0.0);
    let sd = normalize(u.sun_dir.xyz);
    if (ray_sphere(p, sd, R_EARTH).x > 0.0) { return vec3<f32>(0.0); }
    return transmittance(optical_depth(p, sd)) * u.sun_dir.w;
}

fn moon_light() -> vec3<f32> {
    return vec3<f32>(0.06, 0.08, 0.13) * u.params.w;
}

/// Sky radiance in direction d (steps = quality). Expensive: used to fill the sky LUT.
fn sky_raw(d_in: vec3<f32>, steps: i32) -> vec3<f32> {
    // Below the horizon is the world's own ground/sea: the sky only needs y ≥ 0.
    let d = normalize(vec3<f32>(d_in.x, max(d_in.y, 0.0), d_in.z));
    let sd = normalize(u.sun_dir.xyz);
    let ro = vec3<f32>(0.0, R_EARTH + 50.0, 0.0);
    let tm = ray_sphere(ro, d, R_ATMO).y;
    var od = vec2<f32>(0.0);
    var sr = vec3<f32>(0.0);
    var sm = vec3<f32>(0.0);
    // Quadratic step spacing: most air is near the viewer, especially along the horizon.
    for (var i = 0; i < steps; i++) {
        let a = f32(i) / f32(steps);
        let b = f32(i + 1) / f32(steps);
        let t0 = tm * a * a;
        let t1 = tm * b * b;
        let ds = t1 - t0;
        let p = ro + d * (0.5 * (t0 + t1));
        let h = length(p) - R_EARTH;
        let dens = exp(-vec2<f32>(h / H_R, h / H_M)) * ds;
        od += dens;
        if (ray_sphere(p, sd, R_EARTH).x > 0.0) { continue; }
        let od_l = optical_depth(p, sd);
        let tr = transmittance(od + od_l);
        sr += tr * dens.x;
        sm += tr * dens.y;
    }
    let mu = dot(d, sd);
    let pr = 3.0 / (16.0 * PI) * (1.0 + mu * mu);
    let g = 0.76;
    let pm = 3.0 / (8.0 * PI) * ((1.0 - g * g) * (1.0 + mu * mu)) / ((2.0 + g * g) * pow(1.0 + g * g - 2.0 * g * mu, 1.5));
    var col = (sr * BETA_R * pr + sm * BETA_M * pm) * u.sun_dir.w;
    // Multiple scattering (approximation): an isotropic share of the single-scattered light,
    // which brightens and whitens the horizon the way real skies look.
    col += (sr * BETA_R + sm * BETA_M) * (0.25 / (4.0 * PI)) * u.sun_dir.w * max(sd.y + 0.1, 0.0);
    // Night: deep blue + stars.
    let night = u.params.w;
    col += vec3<f32>(0.004, 0.007, 0.018) * night * (0.4 + 0.6 * max(d.y, 0.0));
    col *= u.tint.rgb;
    return col;
}


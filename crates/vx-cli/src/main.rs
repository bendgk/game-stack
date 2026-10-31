//! `vx` — voxelscape on the command line.
//!
//!   vx gen   --prompt "neon city at night"  [--spec spec.json] [--out world.vxd]
//!   vx shot  --prompt "..." --out shot.png  [--w 1920 --h 1080] [--cam x,y,z,yaw,pitch] [--frames 64]
//!   vx bench --bundle worlds/<hash>.vxw [--w 2880 --h 1720 --scale 0.5] [--cam x,y,z,yaw,pitch]
//!            (render-time benchmark of a saved world at a given output size and render scale)
//!   vx wfc   --size 1024 [--out roads.png] [--cpu]     (GPU wave function collapse benchmark)
//!   vx terrain --relief mountains --erode gpu --out land.png  [--size 512] [--iters 400]
//!              [--voxel 4] [--blocky 16] [--sea 0.3] [--seed 7]   (reliefs: hills mountains island canyons mesas plains)
//!
//! Without --spec, the prompt is turned into a scene by the built-in (model-free) parser.

// Index loops mirror the WGSL kernels and grid maths; keeping them literal reads better here.
#![allow(clippy::needless_range_loop, clippy::type_complexity, clippy::too_many_arguments)]

use std::collections::HashMap;

use vx_core::{Dag, Palette, Vec3};
use vx_gen::{Options, SceneSpec, generate_with, spec_from_prompt};
use vx_render::{Camera, Env, headless::Headless};

fn args() -> (String, HashMap<String, String>) {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().unwrap_or_else(|| "help".into());
    let mut m = HashMap::new();
    let rest: Vec<String> = it.collect();
    let mut i = 0;
    while i < rest.len() {
        if let Some(k) = rest[i].strip_prefix("--") {
            // A bare flag (`--cpu`) is followed by another `--key` or nothing.
            match rest.get(i + 1).filter(|v| !v.starts_with("--")) {
                Some(v) => {
                    m.insert(k.to_string(), v.clone());
                    i += 2;
                }
                None => {
                    m.insert(k.to_string(), String::new());
                    i += 1;
                }
            }
        } else {
            i += 1;
        }
    }
    (cmd, m)
}

fn spec_of(a: &HashMap<String, String>) -> SceneSpec {
    if let Some(path) = a.get("spec") {
        let s = std::fs::read_to_string(path).expect("read spec");
        SceneSpec::from_json(&s).expect("parse spec")
    } else {
        spec_from_prompt(a.get("prompt").map(String::as_str).unwrap_or("a modern city on a sunny afternoon"))
    }
}

pub fn env_for(spec: &SceneSpec) -> Env {
    let (elev, az) = spec.sun();
    Env::from_sun(elev, az, spec.fog)
}

fn cam_arg(a: &HashMap<String, String>) -> Option<Camera> {
    let v: Vec<f32> = a.get("cam")?.split(',').filter_map(|x| x.parse().ok()).collect();
    Some(Camera { pos: Vec3::new(v[0], v[1], v[2]), yaw: v[3], pitch: v[4], fov_y: 1.0 })
}

/// Render `dag` headless and save a PNG (`--out`, `--w`, `--h`, `--frames`, `--cam`).
fn shot(a: &HashMap<String, String>, dag: &Dag, palette: &Palette, default_cam: Camera, env: &Env) {
    let (wd, ht) = (
        a.get("w").and_then(|v| v.parse().ok()).unwrap_or(1600),
        a.get("h").and_then(|v| v.parse().ok()).unwrap_or(900),
    );
    let mut hl = Headless::new(wd, ht).expect("gpu");
    hl.r.set_world(dag, palette).expect("upload");
    let cam = cam_arg(a).unwrap_or(default_cam);
    let frames = a.get("frames").and_then(|v| v.parse().ok()).unwrap_or(48);
    let ms = hl.render(&cam, env, frames, 0.0);
    let out = a.get("out").cloned().unwrap_or("shot.png".into());
    hl.save_png(&out).unwrap();
    println!("rendered {out} on {}: {ms:.2} ms/frame at {wd}x{ht}", hl.adapter_info);
}

fn terrain(a: &HashMap<String, String>) {
    use vx_terrain::{Blocky, Heightmap, Landscape, Relief, TerrainMats, erosion};
    let get = |k: &str, d: f32| a.get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    let relief = match a.get("relief").map(String::as_str).unwrap_or("mountains") {
        "hills" => Relief::Hills,
        "island" => Relief::Island,
        "canyons" => Relief::Canyons,
        "mesas" => Relief::Mesas,
        "plains" => Relief::Plains,
        _ => Relief::Mountains,
    };
    let n = get("size", 512.0) as usize;
    let seed = get("seed", 7.0) as u32;
    let cell = 64; // one height sample per 2 m
    let amp = get("relief_m", n as f32 * 2.0 * Heightmap::suggested_relief(relief)) * 32.0;
    let t0 = std::time::Instant::now();
    let mut hm = Heightmap::generate(n, n, cell, relief, seed, 64.0, amp);
    let gen_ms = t0.elapsed().as_millis();
    let p = erosion::Params { iterations: get("iters", 1500.0) as u32, seed, ..Default::default() };
    let t1 = std::time::Instant::now();
    let before = hm.data.clone();
    let eroded = match a.get("erode").map(String::as_str).unwrap_or("gpu") {
        "none" => false,
        "cpu" => {
            erosion::erode(&mut hm, &p);
            true
        }
        _ => {
            match vx_terrain::erosion_gpu::device() {
                Ok((dev, q)) => drop(vx_terrain::erosion_gpu::erode_gpu(&dev, &q, &mut hm, &p)),
                Err(e) => {
                    eprintln!("{e}: eroding on the CPU");
                    erosion::erode(&mut hm, &p);
                }
            }
            true
        }
    };
    // Net deposition (voxels) for the material rules.
    let deposited = eroded.then(|| hm.data.iter().zip(&before).map(|(a, b)| a - b).collect::<Vec<f32>>());
    let erode_ms = t1.elapsed().as_millis();
    let (lo, hi) = hm.data.iter().fold((f32::MAX, f32::MIN), |(x, y), &v| (x.min(v), y.max(v)));
    // Sea level on the 2 m grid, so the water plane is whole DAG cubes (never sampled).
    let sea = ((lo + (hi - lo) * get("sea", 0.0)) / 64.0).round() * 64.0;
    let mut palette = Palette::default_city();
    let mats = TerrainMats::add(&mut palette);
    let side = (n as i32 * cell) as u64;
    let mut levels = 6;
    while 4u64.pow(levels) < side.max(hi as u64 + 64) {
        levels += 1;
    }
    // Sea (or plain ground) out to the edge of the world so the horizon isn't a void. Whole
    // solid cubes: costs a few nodes.
    let world = 4i32.pow(levels);
    let around = vx_core::BoxShape {
        b: vx_core::Aabb::new(
            vx_core::IVec3::new(0, 0, 0),
            vx_core::IVec3::new(world, if sea > lo { sea as i32 } else { (lo as i32 / 64) * 64 }, world),
        ),
        mat: if sea > lo { mats.water } else { mats.dirt },
    };
    let floor = vx_core::BoxShape {
        b: vx_core::Aabb::new(vx_core::IVec3::ZERO, vx_core::IVec3::new(world, 32, world)),
        mat: mats.rock,
    };
    let t2 = std::time::Instant::now();
    let (dag, stats) = match a.get("blocky").and_then(|v| v.parse::<i32>().ok()) {
        Some(block) => {
            let mut b = Blocky::new(&hm, block, mats, seed);
            if sea > lo {
                b = b.with_sea((sea / block as f32) as i32);
            }
            vx_core::build(&palette, levels, &[&floor, &around, &b])
        }
        None => {
            let mut land = Landscape::new(hm, mats, seed).with_voxel(get("voxel", 4.0) as i32);
            if sea > lo {
                land = land.with_sea(sea as i32);
            }
            if let Some(d) = deposited {
                land = land.with_deposits(d);
            }
            vx_core::build(&palette, levels, &[&floor, &around, &land])
        }
    };
    println!(
        "terrain {n}² ({} m): {:.3e} voxels in {} MB · heightmap {gen_ms} ms · erosion {erode_ms} ms ({} steps) · build {} ms · sampled leaves {}",
        n * 2,
        dag.voxel_count() as f64,
        (dag.words.len() * 4) >> 20,
        p.iterations,
        t2.elapsed().as_millis(),
        stats.sampled_leaves
    );
    if a.contains_key("out") {
        let s = side as f32;
        let cam = Camera { pos: Vec3::new(-0.15 * s, hi + s * 0.35, 1.15 * s), yaw: 0.75, pitch: -0.32, fov_y: 1.0 };
        let env = Env { sun_elevation: get("sun", 0.6), sun_azimuth: 2.2, fog: 0.0003, ..Default::default() };
        shot(a, &dag, &palette, cam, &env);
    }
}

/// Solve an N×N road network with the GPU WFC (and the CPU one for comparison on small N).
fn wfc_bench(a: &HashMap<String, String>) {
    use vx_gen::wfc::{Tile, Wfc};
    let n: usize = a.get("size").and_then(|v| v.parse().ok()).unwrap_or(1024);
    let r = |name, edges, weight| Tile { name, edges, weight };
    let tiles = vec![
        r("lot", [0, 0, 0, 0], 3.0),
        r("ns", [1, 0, 1, 0], 1.0),
        r("ew", [0, 1, 0, 1], 1.0),
        r("cross", [1, 1, 1, 1], 0.35),
        r("t_n", [0, 1, 1, 1], 0.25),
        r("t_e", [1, 0, 1, 1], 0.25),
        r("t_s", [1, 1, 0, 1], 0.25),
        r("t_w", [1, 1, 1, 0], 0.25),
        r("ne", [1, 1, 0, 0], 0.15),
        r("es", [0, 1, 1, 0], 0.15),
        r("sw", [0, 0, 1, 1], 0.15),
        r("wn", [1, 0, 0, 1], 0.15),
    ];
    let wfc = Wfc::new(n, n, tiles.clone());
    let (dev, q) = vx_gen::wfc_gpu::device().expect("gpu");
    let _ = wfc.solve_gpu(&dev, &q, 0, &|_, _, _| 1.0); // warm up (shader compile)
    let (sol, st) = wfc.solve_gpu(&dev, &q, 1, &|_, _, _| 1.0);
    let sol = sol.expect("GPU WFC converged");
    println!(
        "GPU WFC {n}×{n} = {} cells: {} rounds, {:.1} ms ({:.1} M cells/s), valid {}",
        n * n,
        st.rounds,
        st.millis,
        (n * n) as f64 / st.millis / 1e3,
        wfc.check(&sol)
    );
    if a.contains_key("cpu") {
        let t = std::time::Instant::now();
        let c = wfc.solve(1, 40, &|_, _, _| 1.0);
        println!("CPU WFC {n}×{n}: {:.1} ms, solved {}", t.elapsed().as_secs_f64() * 1000.0, c.is_some());
    }
    if let Some(out) = a.get("out") {
        // 3×3 pixels per cell: centre + the four road arms.
        let s = n * 3;
        let mut img = vec![0u8; s * s * 3];
        for (i, &t) in sol.iter().enumerate() {
            let (cx, cz) = ((i % n) * 3 + 1, (i / n) * 3 + 1);
            let e = tiles[t].edges;
            let mut put =
                |x: usize, z: usize, c: [u8; 3]| img[(z * s + x) * 3..(z * s + x) * 3 + 3].copy_from_slice(&c);
            let lot = [70, 110, 60];
            for dz in 0..3 {
                for dx in 0..3 {
                    put(cx + dx - 1, cz + dz - 1, lot);
                }
            }
            if e != [0; 4] {
                let road = [40, 40, 44];
                put(cx, cz, road);
                if e[0] == 1 {
                    put(cx, cz - 1, road)
                }
                if e[1] == 1 {
                    put(cx + 1, cz, road)
                }
                if e[2] == 1 {
                    put(cx, cz + 1, road)
                }
                if e[3] == 1 {
                    put(cx - 1, cz, road)
                }
            }
        }
        vx_render::headless::save_rgb_png(out, s as u32, s as u32, &img).unwrap();
        println!("wrote {out}");
    }
}

/// Time the renderer on a saved world bundle: output `w`×`h`, ray traced at `scale`.
fn bench(a: &HashMap<String, String>) {
    let path = a.get("bundle").expect("--bundle path/to/world.vxw");
    let bytes = std::fs::read(path).expect("read bundle");
    let (meta, dag) = vx_gen::bundle::decode(&bytes).expect("decode bundle");
    let get = |k: &str, d: f32| a.get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    let (w, h) = (get("w", 2880.0) as u32, get("h", 1720.0) as u32);
    let scale = get("scale", 0.5);
    let mut hl = Headless::new(w, h).expect("gpu");
    hl.r.set_world(&dag, &meta.palette).expect("upload");
    hl.r.resize((w as f32 * scale) as u32, (h as f32 * scale) as u32);
    let sp = Vec3::from(meta.spawn);
    let cam = cam_arg(a).unwrap_or(Camera { pos: sp + Vec3::new(0.0, 51.0, 0.0), yaw: 0.0, pitch: 0.0, fov_y: 1.15 });
    let spec = vx_gen::SceneSpec { time_of_day: get("hour", meta.spec.time_of_day), ..meta.spec.clone() };
    let (elev, az) = spec.sun();
    let env = Env { lod: get("lod", 1.0), ..Env::from_sun(elev, az, spec.fog) };
    if a.contains_key("trace-only") {
        hl.r.debug_view = 99;
    }
    if let Some(d) = a.get("debug").and_then(|v| v.parse().ok()) {
        hl.r.debug_view = d; // 101 no AO, 102 no shadows, 103 no reflections (profiling)
    }
    hl.render(&cam, &env, 10, 0.0); // warm-up (pipelines, caches)
    let profiling = hl.r.enable_profiling();
    // Moving camera: no still-frame convergence shortcut, like walking.
    if let Some(dir) = a.get("seq") {
        // A walk: 24 consecutive frames at 5 m/s, saved for frame-by-frame inspection.
        let _ = std::fs::create_dir_all(dir);
        hl.r.reset_history();
        for i in 0..48 {
            let c =
                Camera { pos: cam.pos + Vec3::new(0.0, 0.0, -2.7) * i as f32, yaw: cam.yaw + i as f32 * 0.004, ..cam };
            hl.render(&c, &env, 1, i as f32 / 60.0);
            if i >= 24 {
                hl.save_png(&format!("{dir}/f{:02}.png", i - 24)).unwrap();
            }
        }
        println!("wrote {dir}/f00..f23.png");
        return;
    }
    if let Some(out) = a.get("quality") {
        // Motion quality: walk 40 frames at 5 m/s (2.7 voxels/frame), keep the last frame; then
        // the same camera, still, at full output resolution, converged over 64 jittered frames
        // (the super-sampled ground truth). Report the mean error between them.
        let step = Vec3::new(0.0, 0.0, -2.7);
        hl.r.reset_history();
        let mut c = cam;
        for i in 0..40 {
            c = Camera { pos: cam.pos + step * i as f32, ..cam };
            hl.render(&c, &env, 1, i as f32 / 60.0);
        }
        hl.save_png(&format!("{out}_moving.png")).unwrap();
        let (_, _, moving) = hl.read();
        hl.r.resize(w, h);
        hl.render(&c, &env, 64, 0.0);
        hl.save_png(&format!("{out}_truth.png")).unwrap();
        let (_, _, truth) = hl.read();
        if let Some(d) = a.get("debug").and_then(|v| v.parse::<u32>().ok()) {
            hl.r.debug_view = d;
            hl.render(&c, &env, 2, 0.0);
            hl.save_png(&format!("{out}_debug{d}.png")).unwrap();
        }
        let err: f64 =
            moving.iter().zip(&truth).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>() / moving.len() as f64;
        let edge: f64 = moving
            .chunks(4)
            .zip(truth.chunks(4))
            .map(|(a, b)| (0..3).map(|k| (a[k] as f64 - b[k] as f64).abs()).fold(0.0, f64::max))
            .filter(|e| *e > 24.0)
            .count() as f64
            / (moving.len() / 4) as f64;
        println!("motion quality: mean error {err:.2} /255 · {:.2}% of pixels off by >24", edge * 100.0);
        return;
    }
    // Report the fastest and the median frame: other GPU users (a browser tab, a compile) only
    // ever add time, so the minimum is the closest to the renderer's own cost.
    let n = 90;
    let mut times: Vec<f64> = (0..n)
        .map(|i| {
            let c = Camera { pos: cam.pos + Vec3::new(0.0, 0.0, -(i as f32) * 2.7), ..cam };
            hl.render(&c, &env, 1, i as f32 / 60.0)
        })
        .collect();
    times.sort_by(|a, b| a.total_cmp(b));
    if profiling {
        // GPU time per pass, best of 20 frames (immune to other GPU users).
        let mut best: Vec<(&str, f64)> = Vec::new();
        for i in 0..20 {
            let c = Camera { pos: cam.pos + Vec3::new(0.0, 0.0, -(i as f32) * 2.7), ..cam };
            hl.render(&c, &env, 1, 0.0);
            let p = hl.r.read_profile();
            if best.is_empty() {
                best = p;
            } else {
                for (b, x) in best.iter_mut().zip(p) {
                    b.1 = b.1.min(x.1);
                }
            }
        }
        let total: f64 = best.iter().map(|b| b.1).sum();
        println!(
            "GPU: {} · total {total:.2} ms",
            best.iter().map(|(n, t)| format!("{n} {t:.2}")).collect::<Vec<_>>().join(" · ")
        );
    }
    println!(
        "{:.2} ms/frame best, {:.2} median · output {w}x{h} · traced {}x{} · {}",
        times[0],
        times[n / 2],
        (w as f32 * scale) as u32,
        (h as f32 * scale) as u32,
        hl.adapter_info
    );
}

fn main() {
    let (cmd, a) = args();
    match cmd.as_str() {
        "gen" | "shot" => {
            let spec = spec_of(&a);
            eprintln!("spec: {}", serde_json::to_string(&spec).unwrap());
            let w = generate_with(&spec, Options { gpu_wfc: a.contains_key("gpu-wfc"), gpu_erosion: true });
            let s = &w.stats;
            println!(
                "{}: {:.3e} voxels ({:.1} billion) in {} MB · plan {} ms · emit {} ms · build {} ms · sampled leaves {} · linked {} · {} buildings, {} props, {} roads · unique modules {} tiles {} props {}",
                spec.title,
                s.voxels as f64,
                s.voxels as f64 / 1e9,
                s.bytes >> 20,
                s.plan_ms,
                s.emit_ms,
                s.build_ms,
                s.sampled_leaves,
                s.linked,
                s.city.buildings,
                s.city.props,
                s.city.roads,
                s.city.unique_modules,
                s.city.unique_tiles,
                s.city.unique_props
            );
            if let Some(out) = a.get("out").filter(|_| cmd == "gen") {
                std::fs::write(out, w.dag.to_bytes()).unwrap();
                std::fs::write(format!("{out}.palette.json"), serde_json::to_string(&w.palette).unwrap()).unwrap();
                println!("wrote {out}");
            }
            if cmd == "shot" && a.contains_key("props") {
                // The world bundle (world + every prop + the avatar) with all props lined up at spawn.
                let (bytes, meta) = vx_gen::bundle::bundle(&w);
                let (_, dag) = vx_gen::bundle::decode(&bytes).unwrap();
                println!("bundle {} ({} MB, {} prefabs)", meta.hash, bytes.len() >> 20, meta.prefabs.len());
                let mut inst = Vec::new();
                for (i, p) in meta.prefabs.iter().enumerate() {
                    let row = (i / 12) as f32;
                    let col = (i % 12) as f32;
                    let pivot = Vec3::new(
                        (p.min[0] + p.max[0]) as f32 / 2.0,
                        p.min[1] as f32,
                        (p.min[2] + p.max[2]) as f32 / 2.0,
                    );
                    let pos = w.spawn + Vec3::new(col * 110.0 - 600.0, 0.0, -row * 140.0 - 200.0);
                    let rot = vx_core::glam::Quat::from_rotation_y(i as f32 * 0.4);
                    inst.push(vx_render::Instance {
                        root: p.root,
                        levels: p.level,
                        pos,
                        rot,
                        pivot,
                        tint: [0xff3355, 0x33aaff, 0xffcc00][i % 3],
                    });
                }
                let cam = Camera { pos: w.spawn + Vec3::new(0.0, 260.0, 380.0), yaw: 0.0, pitch: -0.32, fov_y: 1.0 };
                let (wd, ht) = (
                    a.get("w").and_then(|v| v.parse().ok()).unwrap_or(1600),
                    a.get("h").and_then(|v| v.parse().ok()).unwrap_or(900),
                );
                let mut hl = Headless::new(wd, ht).expect("gpu");
                hl.r.set_world(&dag, &meta.palette).expect("upload");
                hl.r.set_instances(&inst);
                let ms = hl.render(&cam_arg(&a).unwrap_or(cam), &env_for(&w.spec), 48, 0.0);
                let out = a.get("out").cloned().unwrap_or("shot.png".into());
                hl.save_png(&out).unwrap();
                println!("rendered {out}: {ms:.2} ms/frame with {} instances", inst.len());
            } else if cmd == "shot" {
                let cam = Camera { pos: w.spawn + Vec3::new(0.0, 900.0, 2400.0), yaw: 0.35, pitch: -0.22, fov_y: 1.0 };
                shot(&a, &w.dag, &w.palette, cam, &env_for(&w.spec));
            }
        }
        "terrain" => terrain(&a),
        "wfc" => wfc_bench(&a),
        "bench" => bench(&a),
        _ => {
            eprintln!(
                "usage: vx gen|shot [--prompt TEXT | --spec FILE] [--out FILE] [--w N --h N] [--cam x,y,z,yaw,pitch] [--frames N]"
            );
        }
    }
}

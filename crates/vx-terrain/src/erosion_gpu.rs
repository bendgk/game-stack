//! GPU erosion: the same model as [`crate::erosion`], as six WGSL compute passes per step.
//! Runs on Metal / Vulkan / DX12 natively and on WebGPU in the browser. On an M-series GPU a
//! 1024² map does hundreds of steps per second.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::erosion::{Params, rain_at};
use crate::heightmap::Heightmap;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uni {
    w: u32,
    h: u32,
    raining: u32,
    _pad: u32,
    l: f32,
    dt: f32,
    pipe: f32,
    grav: f32,
    cap: f32,
    susp: f32,
    depo: f32,
    evap: f32,
    min_tilt: f32,
    max_depth: f32,
    talus: f32,
    thermal: f32,
}

/// Results for material rules (normalised amounts).
pub struct GpuResult {
    pub water: Vec<f32>,
    pub sediment: Vec<f32>,
    pub eroded: Vec<f32>,
    pub deposited: Vec<f32>,
    pub millis: f64,
}

/// A compute-only device (no window).
pub fn device() -> Result<(wgpu::Device, wgpu::Queue), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .map_err(|e| format!("no GPU adapter: {e}"))?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("vx-terrain"),
        required_limits: {
            // WebGPU defaults, but with the adapter's real buffer sizes (big grids/maps).
            let a = adapter.limits();
            wgpu::Limits {
                max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
                max_buffer_size: a.max_buffer_size,
                ..wgpu::Limits::default()
            }
        },
        ..Default::default()
    }))
    .map_err(|e| format!("device: {e}"))
}

/// Erode `hm` on the GPU (in place). Returns the auxiliary maps.
pub fn erode_gpu(device: &wgpu::Device, queue: &wgpu::Queue, hm: &mut Heightmap, p: &Params) -> GpuResult {
    let t0 = std::time::Instant::now();
    let (w, h) = (hm.w, hm.h);
    let n = w * h;
    let (lo, hi) = hm.data.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let _ = hi;
    // Cell units, as in the CPU reference.
    let range = hm.cell as f32;
    let st: Vec<[f32; 4]> =
        (0..n).map(|i| [(hm.data[i] - lo) / range, 0.0, 0.0, rain_at(i % w, i / w, w, p)]).collect();
    let zero = vec![[0.0f32; 4]; n];
    let buf = |label: &str, data: &[[f32; 4]]| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(data),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        })
    };
    let b_st = buf("st", &st);
    let b_fl = buf("fl", &zero);
    let b_fo = buf("fo", &zero);
    let b_ve = buf("ve", &zero);
    let b_tmp = buf("tmp", &zero);
    let b_gv = buf("gv", &zero);
    let mk_uni = |raining: bool| Uni {
        w: w as u32,
        h: h as u32,
        raining: raining as u32,
        _pad: 0,
        l: 1.0,
        dt: p.dt,
        pipe: p.pipe_area,
        grav: p.gravity,
        cap: p.capacity,
        susp: p.suspension,
        depo: p.deposition,
        evap: p.evaporation,
        min_tilt: p.min_tilt,
        max_depth: p.max_depth,
        talus: p.talus,
        thermal: p.thermal,
    };
    let uni_rain = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("p rain"),
        contents: bytemuck::bytes_of(&mk_uni(true)),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let uni_dry = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("p dry"),
        contents: bytemuck::bytes_of(&mk_uni(false)),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("erosion"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/erosion.wgsl").into()),
    });
    let entries: Vec<wgpu::BindGroupLayoutEntry> = (0..7)
        .map(|i| wgpu::BindGroupLayoutEntry {
            binding: i,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: if i == 0 {
                    wgpu::BufferBindingType::Uniform
                } else {
                    wgpu::BufferBindingType::Storage { read_only: false }
                },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        })
        .collect();
    let bgl =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("erosion"), entries: &entries });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("erosion"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipe = |entry: &str| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: Some(&pl),
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    let pipes = ["flux_main", "water_main", "erode_main", "transport_main", "give_main", "apply_main"].map(pipe);
    let group = |u: &wgpu::Buffer| {
        let res = [u, &b_st, &b_fl, &b_fo, &b_ve, &b_tmp, &b_gv];
        let e: Vec<wgpu::BindGroupEntry> = res
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() })
            .collect();
        device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &bgl, entries: &e })
    };
    let (g_rain, g_dry) = (group(&uni_rain), group(&uni_dry));
    let (gx, gy) = ((w as u32).div_ceil(16), (h as u32).div_ceil(16));
    let rain_steps = p.iterations * 9 / 10;
    // Batch many steps per submission.
    let mut it = 0;
    while it < p.iterations {
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut cp = enc.begin_compute_pass(&Default::default());
            for _ in 0..64 {
                if it >= p.iterations {
                    break;
                }
                cp.set_bind_group(0, if it < rain_steps { &g_rain } else { &g_dry }, &[]);
                for pp in &pipes {
                    cp.set_pipeline(pp);
                    cp.dispatch_workgroups(gx, gy, 1);
                }
                it += 1;
            }
        }
        queue.submit([enc.finish()]);
    }
    // Read back st and ve.
    let read = |src: &wgpu::Buffer| -> Vec<[f32; 4]> {
        let size = (n * 16) as u64;
        let rb = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(src, 0, &rb, 0, size);
        queue.submit([enc.finish()]);
        rb.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let data = rb.slice(..).get_mapped_range().expect("mapped");
        bytemuck::cast_slice::<u8, [f32; 4]>(&data).to_vec()
    };
    let st = read(&b_st);
    let ve = read(&b_ve);
    for i in 0..n {
        hm.data[i] = lo + st[i][0] * range;
    }
    GpuResult {
        water: st.iter().map(|c| c[1]).collect(),
        sediment: st.iter().map(|c| c[2]).collect(),
        eroded: ve.iter().map(|c| c[2]).collect(),
        deposited: ve.iter().map(|c| c[3]).collect(),
        millis: t0.elapsed().as_secs_f64() * 1000.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heightmap::Relief;

    #[test]
    fn gpu_matches_cpu() {
        let Ok((device, queue)) = device() else {
            eprintln!("no GPU — skipping");
            return;
        };
        let base = Heightmap::generate(64, 64, 64, Relief::Mountains, 3, 0.0, 2500.0);
        let p = Params { iterations: 60, ..Default::default() };
        let mut cpu = base.clone();
        crate::erosion::erode(&mut cpu, &p);
        let mut gpu = base.clone();
        let r = erode_gpu(&device, &queue, &mut gpu, &p);
        let max_diff = cpu.data.iter().zip(&gpu.data).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        let moved = base.data.iter().zip(&gpu.data).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        eprintln!("cpu vs gpu max diff {max_diff:.3} voxels (erosion moved up to {moved:.1}) in {:.1} ms", r.millis);
        assert!(moved > 1.0, "GPU eroded something");
        assert!(max_diff < moved * 0.05 + 1.0, "GPU follows the CPU reference: {max_diff}");
    }
}

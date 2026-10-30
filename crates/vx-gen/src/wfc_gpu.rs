//! Wave Function Collapse on the GPU (wgpu compute: Metal / Vulkan / DX12 / WebGPU).
//!
//! Same tiles, restrictions and per-cell weights as [`crate::wfc::Wfc`], solved in parallel:
//! thousands of cells collapse per round (every local minimum of the option count), constraint
//! propagation runs as Jacobi sweeps, and contradictions are repaired locally by re-opening a
//! small patch rather than restarting. Integer-only, so results are identical on every GPU.
//! See `shaders/wfc.wgsl` for the algorithm. Up to 32 tiles.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::wfc::Wfc;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uni {
    w: u32,
    h: u32,
    nt: u32,
    seed: u32,
    round: u32,
    radius: u32,
    window: u32,
    _b: u32,
}

/// Statistics from a GPU solve.
#[derive(Clone, Copy, Debug, Default)]
pub struct GpuWfcStats {
    pub rounds: u32,
    pub millis: f64,
}

/// A compute-only device for generators (no window).
pub fn device() -> Result<(wgpu::Device, wgpu::Queue), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .map_err(|e| format!("no GPU adapter: {e}"))?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("vx-gen"),
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

/// Propagation sweep pairs (A → B → A) per round; one more A → B sweep feeds `observe`.
const SWEEPS: u32 = 6;
/// Rounds submitted between progress checks.
const BATCH: u32 = 24;

impl Wfc {
    /// Solve on the GPU. `weight(x, z, tile)` as in [`Wfc::solve`]. Returns the chosen tile per
    /// cell (validated against the adjacency rules on the CPU), or None if it didn't converge.
    pub fn solve_gpu(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        seed: u64,
        weight: &(dyn Fn(usize, usize, usize) -> f32 + Sync),
    ) -> (Option<Vec<usize>>, GpuWfcStats) {
        let t0 = std::time::Instant::now();
        let (w, h, nt) = (self.w, self.h, self.tiles.len());
        assert!(nt <= 32, "GPU WFC supports up to 32 tiles");
        let n = w * h;
        let init: Vec<u32> = self.cells.iter().map(|&c| c as u32).collect();
        let mut compat = vec![0u32; 4 * 32];
        for d in 0..4 {
            for t in 0..nt {
                compat[d * 32 + t] = self.compat(d, t) as u32;
            }
        }
        // Integer weights (×1000): exact, platform-independent sampling on the GPU.
        let mut weights = vec![0u32; n * nt];
        for z in 0..h {
            for x in 0..w {
                for t in 0..nt {
                    let v = (self.tiles[t].weight * weight(x, z, t)).max(0.0);
                    weights[(z * w + x) * nt + t] = ((v * 1000.0) as u32).max(1);
                }
            }
        }
        let storage = |label: &str, data: &[u32], extra: wgpu::BufferUsages| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(data),
                usage: wgpu::BufferUsages::STORAGE | extra,
            })
        };
        let b_a = storage("wfc a", &init, wgpu::BufferUsages::COPY_SRC);
        let b_b = storage("wfc b", &init, wgpu::BufferUsages::empty());
        let b_init = storage("wfc init", &init, wgpu::BufferUsages::empty());
        let b_compat = storage("wfc compat", &compat, wgpu::BufferUsages::empty());
        let b_weights = storage("wfc weights", &weights, wgpu::BufferUsages::empty());
        let b_mark = storage("wfc mark", &vec![0u32; n], wgpu::BufferUsages::empty());
        let b_count = storage("wfc counters", &[0u32; 4], wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("wfc"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/wfc.wgsl").into()),
        });
        let entries: Vec<wgpu::BindGroupLayoutEntry> = (0..8u32)
            .map(|i| wgpu::BindGroupLayoutEntry {
                binding: i,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: match i {
                        0 => wgpu::BufferBindingType::Uniform,
                        3..=5 => wgpu::BufferBindingType::Storage { read_only: true },
                        _ => wgpu::BufferBindingType::Storage { read_only: false },
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let bgl =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("wfc"), entries: &entries });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wfc"),
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
        let [prop_ab, prop_ba, observe, mark, reset, count] =
            ["prop_ab", "prop_ba", "observe", "mark_main", "reset_main", "count_main"].map(pipe);
        // One uniform buffer per round in a batch (the round number seeds the hashes).
        // Repair radius escalates 2 → 4 → 6 → 8 across batches while contradictions persist.
        let uni = |round: u32, radius: u32| Uni {
            w: w as u32,
            h: h as u32,
            nt: nt as u32,
            seed: (seed ^ (seed >> 32)) as u32,
            round,
            radius,
            window: 3,
            _b: 0,
        };
        let mut radius = 2u32;
        let mut last_contra = u32::MAX;
        let make_group = |u: &wgpu::Buffer| {
            let res = [u, &b_a, &b_b, &b_init, &b_compat, &b_weights, &b_mark, &b_count];
            let e: Vec<wgpu::BindGroupEntry> = res
                .iter()
                .enumerate()
                .map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() })
                .collect();
            device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &bgl, entries: &e })
        };
        let (gx, gy) = ((w as u32).div_ceil(16), (h as u32).div_ceil(16));
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let max_rounds = 64 * (w + h) as u32 + 2000;
        let mut round = 0u32;
        let mut solved = false;
        while round < max_rounds {
            let groups: Vec<wgpu::BindGroup> = (0..BATCH)
                .map(|k| {
                    let b = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: bytemuck::bytes_of(&uni(round + k, radius)),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                    make_group(&b)
                })
                .collect();
            let mut enc = device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                for g in &groups {
                    cp.set_bind_group(0, g, &[]);
                    for _ in 0..SWEEPS / 2 {
                        cp.set_pipeline(&prop_ab);
                        cp.dispatch_workgroups(gx, gy, 1);
                        cp.set_pipeline(&prop_ba);
                        cp.dispatch_workgroups(gx, gy, 1);
                    }
                    cp.set_pipeline(&prop_ab);
                    cp.dispatch_workgroups(gx, gy, 1);
                    for pp in [&observe, &mark, &reset] {
                        cp.set_pipeline(pp);
                        cp.dispatch_workgroups(gx, gy, 1);
                    }
                }
                // Settle, then count undecided cells and contradictions.
                cp.set_bind_group(0, &groups[0], &[]);
                for _ in 0..SWEEPS / 2 {
                    cp.set_pipeline(&prop_ab);
                    cp.dispatch_workgroups(gx, gy, 1);
                    cp.set_pipeline(&prop_ba);
                    cp.dispatch_workgroups(gx, gy, 1);
                }
            }
            enc.clear_buffer(&b_count, 0, None);
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_bind_group(0, &groups[0], &[]);
                cp.set_pipeline(&count);
                cp.dispatch_workgroups(gx, gy, 1);
            }
            enc.copy_buffer_to_buffer(&b_count, 0, &readback, 0, 16);
            queue.submit([enc.finish()]);
            round += BATCH;
            let c = read_u32s(device, &readback);
            if std::env::var_os("VX_WFC_DEBUG").is_some() {
                eprintln!("round {round}: undecided {} contradictions {}", c[0], c[1]);
            }
            if c[0] == 0 && c[1] == 0 {
                solved = true;
                break;
            }
            radius = if c[1] > 0 && c[1] >= last_contra { (radius + 2).min(8) } else { 2 };
            last_contra = c[1];
        }
        let stats = GpuWfcStats { rounds: round, millis: t0.elapsed().as_secs_f64() * 1000.0 };
        if !solved {
            return (None, stats);
        }
        let out = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (n * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&b_a, 0, &out, 0, (n * 4) as u64);
        queue.submit([enc.finish()]);
        let cells = read_u32s(device, &out);
        let tiles: Vec<usize> = cells.iter().map(|c| c.trailing_zeros() as usize).collect();
        let ok = self.check(&tiles);
        (ok.then_some(tiles), GpuWfcStats { millis: t0.elapsed().as_secs_f64() * 1000.0, ..stats })
    }
}

fn read_u32s(device: &wgpu::Device, buf: &wgpu::Buffer) -> Vec<u32> {
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let v = bytemuck::cast_slice::<u8, u32>(&buf.slice(..).get_mapped_range().expect("mapped")).to_vec();
    buf.unmap();
    v
}

#[cfg(test)]
mod tests {
    use crate::wfc::{Tile, Wfc};

    /// Road network tiles: straights, every corner and T, crossings, plus empty lots.
    fn roads() -> Vec<Tile> {
        let r = |name, edges, weight| Tile { name, edges, weight };
        vec![
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
        ]
    }

    #[test]
    fn gpu_solves_valid_and_deterministic() {
        let Ok((device, queue)) = super::device() else {
            eprintln!("no GPU — skipping");
            return;
        };
        let mut wfc = Wfc::new(200, 150, roads());
        wfc.restrict(0, 0, 1); // a lot in the corner
        let (a, sa) = wfc.solve_gpu(&device, &queue, 9, &|_, _, _| 1.0);
        let a = a.expect("GPU WFC converges");
        assert_eq!(a[0], 0);
        assert!(wfc.check(&a));
        let (b, _) = wfc.solve_gpu(&device, &queue, 9, &|_, _, _| 1.0);
        assert_eq!(Some(a), b, "deterministic");
        eprintln!("200×150 GPU WFC: {} rounds, {:.1} ms", sa.rounds, sa.millis);
    }
}

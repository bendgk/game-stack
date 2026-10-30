//! # vx-render — the voxelscape renderer
//!
//! A GPU ray tracer for the hash-consed voxel DAG, written in WGSL and driven by wgpu: it runs
//! natively on Metal (`vx-shot` renders PNGs headless) and in the browser on WebGPU.
//!
//! Per frame: **trace** (primary rays + stochastic LOD → G-buffer) → **light** (soft sun
//! shadow ray + one ray-traced GI bounce, itself sun-shadowed) → **accum** (temporal
//! reprojection with disocclusion tests) → **atrous** ×3 (edge-aware denoise) → **composite**
//! (PBR-ish materials, GGX sun highlight, ray-traced reflections, emission, physical sky,
//! aerial-perspective height fog) → **taa** (YCoCg-clamped history) → **present** (bloom, AgX).

// Index loops mirror the WGSL kernels and grid maths; keeping them literal reads better here.
#![allow(clippy::needless_range_loop, clippy::type_complexity, clippy::too_many_arguments)]

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use vx_core::{Dag, Palette};
use wgpu::util::DeviceExt;

const COMMON_SRC: &str = include_str!("shaders/common.wgsl");
const ATMO: &str = include_str!("shaders/atmo.wgsl");

/// The scene prelude every compute pass is compiled with: types, bindings, hashing, the
/// atmosphere, LUT-backed sky helpers and the DAG tracer.
fn common() -> String {
    COMMON_SRC.replace("/*ATMO*/", ATMO)
}

/// Just the types (Uniforms, Material, constants) — for passes without the scene bindings.
fn types() -> &'static str {
    COMMON_SRC.split_once("@group(0) @binding(0)").map(|(a, _)| a).unwrap_or("")
}

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Position in voxels.
    pub pos: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_y: f32,
}

impl Camera {
    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos())
    }
    /// Right-handed view, perspective with depth 0..1 (wgpu clip space).
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let f = self.forward().normalize();
        let s = f.cross(Vec3::Y).normalize();
        let up = s.cross(f);
        let view = Mat4::from_cols(
            glam::Vec4::new(s.x, up.x, -f.x, 0.0),
            glam::Vec4::new(s.y, up.y, -f.y, 0.0),
            glam::Vec4::new(s.z, up.z, -f.z, 0.0),
            glam::Vec4::new(-s.dot(self.pos), -up.dot(self.pos), f.dot(self.pos), 1.0),
        );
        let (near, far) = (0.25f32, 1e7f32);
        let h = 1.0 / (self.fov_y * 0.5).tan();
        let w = h / aspect;
        let r = far / (near - far);
        let proj = Mat4::from_cols(
            glam::Vec4::new(w, 0.0, 0.0, 0.0),
            glam::Vec4::new(0.0, h, 0.0, 0.0),
            glam::Vec4::new(0.0, 0.0, r, -1.0),
            glam::Vec4::new(0.0, 0.0, r * near, 0.0),
        );
        proj * view
    }
}

/// Look & time of day.
#[derive(Clone, Copy, Debug)]
pub struct Env {
    /// Sun elevation (radians; negative = night) and azimuth.
    pub sun_elevation: f32,
    pub sun_azimuth: f32,
    pub sun_intensity: f32,
    /// Height-fog density (per metre at ground level).
    pub fog: f32,
    pub exposure: f32,
    /// LOD: 1 = stop refining when a node is smaller than ~one pixel. Higher = coarser/faster.
    pub lod: f32,
    /// GI ray length in voxels.
    pub gi_distance: f32,
    pub tint: Vec3,
    pub bloom: f32,
}

impl Default for Env {
    fn default() -> Self {
        Env {
            sun_elevation: 0.55,
            sun_azimuth: 0.8,
            sun_intensity: 22.0,
            fog: 0.0012,
            exposure: 0.18,
            lod: 1.0,
            gi_distance: 2048.0,
            tint: Vec3::ONE,
            bloom: 0.25,
        }
    }
}

impl Env {
    pub fn sun_dir(&self) -> Vec3 {
        Vec3::new(
            self.sun_elevation.cos() * self.sun_azimuth.sin(),
            self.sun_elevation.sin(),
            self.sun_elevation.cos() * self.sun_azimuth.cos(),
        )
        .normalize()
    }
    /// 0 by day → 1 at night (sun well below the horizon).
    pub fn night(&self) -> f32 {
        ((0.08 - self.sun_elevation) / 0.25).clamp(0.0, 1.0)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    inv_view_proj: [[f32; 4]; 4],
    view_proj: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    cam_pos: [f32; 4],
    prev_cam_pos: [f32; 4],
    res: [f32; 4],
    sun_dir: [f32; 4],
    jitter: [f32; 4],
    world: [u32; 4],
    params: [f32; 4],
    tint: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Present {
    exposure: f32,
    encode_srgb: f32,
    bloom: f32,
    time: f32,
    mode: f32,
    _pad: [f32; 3],
}

fn halton(i: u32, b: u32) -> f32 {
    let (mut f, mut r, mut i) = (1.0f32, 0.0f32, i);
    while i > 0 {
        f /= b as f32;
        r += f * (i % b) as f32;
        i /= b;
    }
    r
}

#[derive(Clone, Copy)]
enum B {
    Uniform,
    Storage,
    Tex,
    TexUint,
    TexFilter,
    Sampler,
    Out(wgpu::TextureFormat),
}

fn layout(device: &wgpu::Device, label: &str, kinds: &[B]) -> wgpu::BindGroupLayout {
    let vis = wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT;
    let entries: Vec<wgpu::BindGroupLayoutEntry> = kinds
        .iter()
        .enumerate()
        .map(|(i, k)| wgpu::BindGroupLayoutEntry {
            binding: i as u32,
            visibility: match k {
                B::Out(_) | B::Storage => wgpu::ShaderStages::COMPUTE,
                _ => vis,
            },
            ty: match *k {
                B::Uniform => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                B::Storage => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                B::Tex => wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                B::TexFilter => wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                B::TexUint => wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                B::Sampler => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                B::Out(f) => wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: f,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
            },
            count: None,
        })
        .collect();
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries: &entries })
}

fn tv(v: &wgpu::TextureView) -> wgpu::BindingResource<'_> {
    wgpu::BindingResource::TextureView(v)
}

fn bind(device: &wgpu::Device, l: &wgpu::BindGroupLayout, res: &[wgpu::BindingResource]) -> wgpu::BindGroup {
    let entries: Vec<wgpu::BindGroupEntry> =
        res.iter().enumerate().map(|(i, r)| wgpu::BindGroupEntry { binding: i as u32, resource: r.clone() }).collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: l, entries: &entries })
}

struct Pass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

fn compute_pass(
    device: &wgpu::Device,
    scene: &wgpu::BindGroupLayout,
    name: &str,
    src: &str,
    entry: &str,
    kinds: &[B],
) -> Pass {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(name),
        source: wgpu::ShaderSource::Wgsl(format!("{}\n{src}", common()).into()),
    });
    let l = layout(device, name, kinds);
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(name),
        bind_group_layouts: &[Some(scene), Some(&l)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(name),
        layout: Some(&pl),
        module: &module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    });
    Pass { pipeline, layout: l }
}

const F32X4: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const F16X4: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const U32: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

struct Targets {
    w: u32,
    h: u32,
    pos: [wgpu::TextureView; 2],
    nrm: [wgpu::TextureView; 2],
    alb: wgpu::TextureView,
    mat: wgpu::TextureView,
    cur_dir: wgpu::TextureView,
    cur_gi: wgpu::TextureView,
    hist_dir: [wgpu::TextureView; 2],
    hist_gi: [wgpu::TextureView; 2],
    tmp: [wgpu::TextureView; 2],
    hdr: wgpu::TextureView,
    taa: [wgpu::TextureView; 2],
}

fn tex(device: &wgpu::Device, w: u32, h: u32, f: wgpu::TextureFormat, label: &str) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: f,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

impl Targets {
    fn new(d: &wgpu::Device, w: u32, h: u32) -> Self {
        let two = |f, l| [tex(d, w, h, f, l), tex(d, w, h, f, l)];
        Targets {
            w,
            h,
            pos: two(F32X4, "pos"),
            nrm: two(F16X4, "nrm"),
            alb: tex(d, w, h, F16X4, "alb"),
            mat: tex(d, w, h, U32, "mat"),
            cur_dir: tex(d, w, h, F16X4, "cur_dir"),
            cur_gi: tex(d, w, h, F16X4, "cur_gi"),
            hist_dir: two(F16X4, "hist_dir"),
            hist_gi: two(F16X4, "hist_gi"),
            tmp: two(F16X4, "atrous"),
            hdr: tex(d, w, h, F16X4, "hdr"),
            taa: two(F16X4, "taa"),
        }
    }
}

/// Stats from the last `set_world`.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorldInfo {
    pub bytes: u64,
    pub levels: u32,
    pub side: u32,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    target_format: wgpu::TextureFormat,
    uniforms: wgpu::Buffer,
    present_buf: wgpu::Buffer,
    step_bufs: [wgpu::Buffer; 3],
    scene_layout: wgpu::BindGroupLayout,
    scene: Option<wgpu::BindGroup>,
    sky_lut: wgpu::TextureView,
    sky_scene: wgpu::BindGroup,
    skylut: Pass,
    nodes: Option<wgpu::Buffer>,
    mats: Option<wgpu::Buffer>,
    trace: Pass,
    light: Pass,
    accum: Pass,
    atrous: Pass,
    composite: Pass,
    taa: Pass,
    blit: wgpu::RenderPipeline,
    blit_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    targets: Targets,
    frame: u32,
    prev_vp: Mat4,
    prev_pos: Vec3,
    world: [u32; 4],
    pub info: WorldInfo,
    /// 0 normal, 1 test pattern, 2 untonemapped source.
    pub debug_view: u32,
}

impl Renderer {
    /// The device limits the renderer wants (big storage buffers for the DAG).
    pub fn required_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
        let a = adapter.limits();
        wgpu::Limits {
            max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
            max_buffer_size: a.max_buffer_size,
            max_storage_textures_per_shader_stage: a.max_storage_textures_per_shader_stage.max(4),
            max_compute_invocations_per_workgroup: 256,
            ..wgpu::Limits::downlevel_defaults().using_resolution(a.clone())
        }
    }

    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        width: u32,
        height: u32,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let scene_layout = layout(&device, "scene", &[B::Uniform, B::Storage, B::Storage, B::TexFilter, B::Sampler]);
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let present_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("present"),
            size: std::mem::size_of::<Present>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let step_bufs = [1i32, 2, 4].map(|s| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("atrous step"),
                contents: bytemuck::cast_slice(&[s, 0, 0, 0]),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        });
        let sl = &scene_layout;
        let trace = compute_pass(
            &device,
            sl,
            "trace",
            include_str!("shaders/trace.wgsl"),
            "trace_main",
            &[B::Out(F32X4), B::Out(F16X4), B::Out(F16X4), B::Out(U32)],
        );
        let light = compute_pass(
            &device,
            sl,
            "light",
            include_str!("shaders/light.wgsl"),
            "light_main",
            &[B::Tex, B::Tex, B::Out(F16X4), B::Out(F16X4)],
        );
        let accum = compute_pass(
            &device,
            sl,
            "accum",
            include_str!("shaders/accum.wgsl"),
            "accum_main",
            &[B::Tex, B::Tex, B::Tex, B::Tex, B::Tex, B::Tex, B::Tex, B::Tex, B::Out(F16X4), B::Out(F16X4)],
        );
        let atrous = compute_pass(
            &device,
            sl,
            "atrous",
            include_str!("shaders/atrous.wgsl"),
            "atrous_main",
            &[B::Tex, B::Tex, B::Tex, B::Out(F16X4), B::Uniform],
        );
        let composite = compute_pass(
            &device,
            sl,
            "composite",
            include_str!("shaders/composite.wgsl"),
            "composite_main",
            &[B::Tex, B::Tex, B::Tex, B::TexUint, B::Tex, B::Tex, B::Out(F16X4)],
        );
        let taa = compute_pass(
            &device,
            sl,
            "taa",
            include_str!("shaders/taa.wgsl"),
            "taa_main",
            &[B::Tex, B::Tex, B::TexFilter, B::Sampler, B::Out(F16X4)],
        );

        let blit_layout = layout(&device, "blit", &[B::TexFilter, B::Sampler, B::Uniform]);
        let blit_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blit.wgsl").into()),
        });
        let blit_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit"),
            bind_group_layouts: &[Some(&blit_layout)],
            immediate_size: 0,
        });
        let blit = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: Some(&blit_pl),
            vertex: wgpu::VertexState {
                module: &blit_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &blit_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let sky_lut = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("sky lut"),
                size: wgpu::Extent3d { width: 128, height: 64, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: F16X4,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let sky_uniform_layout = layout(&device, "sky uniforms", &[B::Uniform]);
        let sky_scene = bind(&device, &sky_uniform_layout, &[uniforms.as_entire_binding()]);
        let skylut = {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("skylut"),
                source: wgpu::ShaderSource::Wgsl(
                    format!(
                        "{}\n@group(0) @binding(0) var<uniform> u: Uniforms;\n{ATMO}\n{}",
                        types(),
                        include_str!("shaders/skylut.wgsl")
                    )
                    .into(),
                ),
            });
            let l = layout(&device, "skylut", &[B::Out(F16X4)]);
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("skylut"),
                bind_group_layouts: &[Some(&sky_uniform_layout), Some(&l)],
                immediate_size: 0,
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("skylut"),
                layout: Some(&pl),
                module: &module,
                entry_point: Some("skylut_main"),
                compilation_options: Default::default(),
                cache: None,
            });
            Pass { pipeline, layout: l }
        };
        let targets = Targets::new(&device, width.max(1), height.max(1));
        Renderer {
            device,
            queue,
            target_format,
            uniforms,
            present_buf,
            step_bufs,
            scene_layout,
            scene: None,
            sky_lut,
            sky_scene,
            skylut,
            nodes: None,
            mats: None,
            trace,
            light,
            accum,
            atrous,
            composite,
            taa,
            blit,
            blit_layout,
            sampler,
            targets,
            frame: 0,
            prev_vp: Mat4::IDENTITY,
            prev_pos: Vec3::ZERO,
            world: [0; 4],
            info: WorldInfo::default(),
            debug_view: 0,
        }
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
    pub fn size(&self) -> (u32, u32) {
        (self.targets.w, self.targets.h)
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if (w, h) != (self.targets.w, self.targets.h) && w > 0 && h > 0 {
            self.targets = Targets::new(&self.device, w, h);
            self.frame = 0;
        }
    }

    /// Upload a world (DAG node pool + palette). Errors if it exceeds the device's limits.
    pub fn set_world(&mut self, dag: &Dag, palette: &Palette) -> Result<(), String> {
        let bytes = (dag.words.len() * 4) as u64;
        let lim = self.device.limits().max_storage_buffer_binding_size;
        if bytes > lim {
            return Err(format!("world is {} MB but this GPU binds at most {} MB per buffer", bytes >> 20, lim >> 20));
        }
        let nodes = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("dag nodes"),
            contents: bytemuck::cast_slice(&dag.words),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let mut gm = palette.gpu();
        if gm.is_empty() {
            gm.push(bytemuck::Zeroable::zeroed());
        }
        let mats = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("materials"),
            contents: bytemuck::cast_slice(&gm),
            usage: wgpu::BufferUsages::STORAGE,
        });
        self.scene = Some(bind(
            &self.device,
            &self.scene_layout,
            &[
                self.uniforms.as_entire_binding(),
                nodes.as_entire_binding(),
                mats.as_entire_binding(),
                tv(&self.sky_lut),
                wgpu::BindingResource::Sampler(&self.sampler),
            ],
        ));
        self.nodes = Some(nodes);
        self.mats = Some(mats);
        self.world = [dag.root, dag.levels, dag.side() as u32, 0];
        self.info = WorldInfo { bytes, levels: dag.levels, side: dag.side() as u32 };
        self.frame = 0;
        Ok(())
    }

    /// Render one frame into `target` (a view of `target_format`).
    pub fn render(&mut self, target: &wgpu::TextureView, cam: &Camera, env: &Env, time: f32) {
        let Some(scene) = self.scene.clone() else { return };
        let (w, h) = (self.targets.w, self.targets.h);
        let vp = cam.view_proj(w as f32 / h as f32);
        if self.frame == 0 {
            self.prev_vp = vp;
            self.prev_pos = cam.pos;
        }
        let jx = (halton(self.frame % 16 + 1, 2) - 0.5) * 2.0 / w as f32;
        let jy = (halton(self.frame % 16 + 1, 3) - 0.5) * 2.0 / h as f32;
        let pixel_angle = cam.fov_y / h as f32;
        let sun = env.sun_dir();
        let u = Uniforms {
            inv_view_proj: vp.inverse().to_cols_array_2d(),
            view_proj: vp.to_cols_array_2d(),
            prev_view_proj: self.prev_vp.to_cols_array_2d(),
            cam_pos: [cam.pos.x, cam.pos.y, cam.pos.z, time],
            prev_cam_pos: [self.prev_pos.x, self.prev_pos.y, self.prev_pos.z, 0.0],
            res: [w as f32, h as f32, 1.0 / w as f32, 1.0 / h as f32],
            sun_dir: [sun.x, sun.y, sun.z, env.sun_intensity],
            jitter: [jx, jy, self.frame as f32, env.exposure],
            world: [self.world[0], self.world[1], self.world[2], self.debug_view],
            params: [pixel_angle * env.lod, env.fog, env.gi_distance, env.night()],
            tint: [env.tint.x, env.tint.y, env.tint.z, 0.0],
        };
        self.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
        let srgb_target = self.target_format.is_srgb();
        let p = Present {
            exposure: env.exposure,
            encode_srgb: if srgb_target { 0.0 } else { 1.0 },
            bloom: env.bloom,
            time,
            mode: if self.debug_view >= 3 { 2.0 } else { self.debug_view as f32 },
            _pad: [0.0; 3],
        };
        self.queue.write_buffer(&self.present_buf, 0, bytemuck::bytes_of(&p));

        let i = (self.frame & 1) as usize;
        let j = 1 - i;
        let t = &self.targets;
        let d = &self.device;
        let g_trace = bind(d, &self.trace.layout, &[tv(&t.pos[i]), tv(&t.nrm[i]), tv(&t.alb), tv(&t.mat)]);
        let g_light = bind(d, &self.light.layout, &[tv(&t.pos[i]), tv(&t.nrm[i]), tv(&t.cur_dir), tv(&t.cur_gi)]);
        let g_accum = bind(
            d,
            &self.accum.layout,
            &[
                tv(&t.pos[i]),
                tv(&t.nrm[i]),
                tv(&t.pos[j]),
                tv(&t.nrm[j]),
                tv(&t.cur_dir),
                tv(&t.cur_gi),
                tv(&t.hist_dir[j]),
                tv(&t.hist_gi[j]),
                tv(&t.hist_dir[i]),
                tv(&t.hist_gi[i]),
            ],
        );
        let at = |src: &wgpu::TextureView, dst: &wgpu::TextureView, k: usize| {
            bind(
                d,
                &self.atrous.layout,
                &[tv(&t.pos[i]), tv(&t.nrm[i]), tv(src), tv(dst), self.step_bufs[k].as_entire_binding()],
            )
        };
        let g_at = [at(&t.hist_gi[i], &t.tmp[0], 0), at(&t.tmp[0], &t.tmp[1], 1), at(&t.tmp[1], &t.tmp[0], 2)];
        let g_comp = bind(
            d,
            &self.composite.layout,
            &[tv(&t.pos[i]), tv(&t.nrm[i]), tv(&t.alb), tv(&t.mat), tv(&t.hist_dir[i]), tv(&t.tmp[0]), tv(&t.hdr)],
        );
        let g_taa = bind(
            d,
            &self.taa.layout,
            &[tv(&t.pos[i]), tv(&t.hdr), tv(&t.taa[j]), wgpu::BindingResource::Sampler(&self.sampler), tv(&t.taa[i])],
        );
        // Debug views 3+: show an intermediate buffer raw.
        let src = match self.debug_view {
            3 => &t.hdr,
            4 | 99 => &t.alb,
            5 => &t.nrm[i],
            6 => &t.cur_gi,
            7 => &t.hist_dir[i],
            8 => &t.tmp[0],
            _ => &t.taa[i],
        };
        let g_blit = bind(
            d,
            &self.blit_layout,
            &[tv(src), wgpu::BindingResource::Sampler(&self.sampler), self.present_buf.as_entire_binding()],
        );

        let mut enc = d.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            let mut cp = enc
                .begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("voxelscape"), timestamp_writes: None });
            let (gx, gy) = (w.div_ceil(8), h.div_ceil(8));
            // Sky LUT first (its own group 0: the LUT is a storage target here).
            let g_sky = bind(d, &self.skylut.layout, &[tv(&self.sky_lut)]);
            cp.set_pipeline(&self.skylut.pipeline);
            cp.set_bind_group(0, &self.sky_scene, &[]);
            cp.set_bind_group(1, &g_sky, &[]);
            cp.dispatch_workgroups(16, 8, 1);
            cp.set_bind_group(0, &scene, &[]);
            for (pass, g) in [(&self.trace, &g_trace), (&self.light, &g_light), (&self.accum, &g_accum)] {
                cp.set_pipeline(&pass.pipeline);
                cp.set_bind_group(1, g, &[]);
                cp.dispatch_workgroups(gx, gy, 1);
                if self.debug_view == 99 {
                    break;
                }
            }
            cp.set_pipeline(&self.atrous.pipeline);
            for g in &g_at {
                cp.set_bind_group(1, g, &[]);
                cp.dispatch_workgroups(gx, gy, 1);
            }
            for (pass, g) in [(&self.composite, &g_comp), (&self.taa, &g_taa)] {
                cp.set_pipeline(&pass.pipeline);
                cp.set_bind_group(1, g, &[]);
                cp.dispatch_workgroups(gx, gy, 1);
            }
        }
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.blit);
            rp.set_bind_group(0, &g_blit, &[]);
            rp.draw(0..3, 0..1);
        }
        self.queue.submit([enc.finish()]);
        self.prev_vp = vp;
        self.prev_pos = cam.pos;
        self.frame += 1;
    }

    /// Restart temporal history (after a teleport or a world swap).
    pub fn reset_history(&mut self) {
        self.frame = 0;
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod headless;

//! Headless rendering (native): render N frames into an offscreen texture and read back RGBA8.
//! Used by `vx-shot` (screenshots, benchmarks) and tests.

use crate::{Camera, Env, Renderer};
use std::time::Instant;

pub struct Headless {
    pub r: Renderer,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    pub adapter_info: String,
}

impl Headless {
    pub fn new(w: u32, h: u32) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("voxelscape"),
            // Timestamps when available (per-pass GPU profiling in `vx bench`).
            required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
            required_limits: Renderer::required_limits(&adapter),
            experimental_features: Default::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: Default::default(),
        }))
        .map_err(|e| format!("device: {e}"))?;
        device.on_uncaptured_error(std::sync::Arc::new(|e: wgpu::Error| eprintln!("wgpu error: {e}")));
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shot"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let r = Renderer::new(device.clone(), queue, w, h, format);
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(format!("renderer setup: {e}"));
        }
        Ok(Headless { r, target, view, adapter_info: format!("{} ({:?})", info.name, info.backend) })
    }

    /// Render `frames` frames (temporal accumulation converges) and return mean ms/frame.
    pub fn render(&mut self, cam: &Camera, env: &Env, frames: u32, t0: f32) -> f64 {
        let start = Instant::now();
        for f in 0..frames {
            let scope = self.r.device().push_error_scope(wgpu::ErrorFilter::Validation);
            self.r.render(&self.view, cam, env, t0 + f as f32 / 60.0);
            if let Some(e) = pollster::block_on(scope.pop()) {
                eprintln!("frame {f}: {e}");
            }
            let _ = self.r.device().poll(wgpu::PollType::wait_indefinitely());
        }
        start.elapsed().as_secs_f64() * 1000.0 / frames as f64
    }

    /// Read back the last frame as tightly packed RGBA8.
    pub fn read(&self) -> (u32, u32, Vec<u8>) {
        let (w, h) = (self.target.width(), self.target.height());
        let bpr = (w * 4).div_ceil(256) * 256;
        let buf = self.r.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (bpr * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.r.device().create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.r.queue().submit([enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buf.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let pr = self.r.device().poll(wgpu::PollType::wait_indefinitely());
        let mr = rx.recv();
        if std::env::var("VX_DEBUG").is_ok() {
            eprintln!("poll: {pr:?} map: {mr:?}");
        }
        let data = buf.slice(..).get_mapped_range().expect("mapped");
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            out.extend_from_slice(&data[(y * bpr) as usize..(y * bpr + w * 4) as usize]);
        }
        (w, h, out)
    }

    pub fn save_png(&self, path: &str) -> Result<(), String> {
        let (w, h, px) = self.read();
        let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().map_err(|e| e.to_string())?.write_image_data(&px).map_err(|e| e.to_string())
    }
}

/// Write an RGB8 image as PNG (for generator previews).
pub fn save_rgb_png(path: &str, w: u32, h: u32, rgb: &[u8]) -> Result<(), String> {
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().map_err(|e| e.to_string())?.write_image_data(rgb).map_err(|e| e.to_string())
}

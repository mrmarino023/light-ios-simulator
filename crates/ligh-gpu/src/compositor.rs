//! Metal compositor — IOSurface import + optional window present.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use ligh_core::LighError;
use metal::{
    CommandQueue, Device, MTLPixelFormat, MTLStorageMode, MTLTextureType, MTLTextureUsage,
    Texture, TextureDescriptor,
};
use tracing::info;

use crate::surface::GpuSurface;

#[derive(Debug, Clone)]
pub struct CompositorStats {
    pub frames: u64,
    pub imports_ok: u64,
    pub imports_fail: u64,
    pub last_width: u32,
    pub last_height: u32,
    pub fps: f64,
}

pub(crate) struct LatestFrame {
    pub(crate) texture: Texture,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

pub struct FrameCompositor {
    device: Device,
    queue: CommandQueue,
    started: Instant,
    frames: AtomicU64,
    imports_ok: AtomicU64,
    imports_fail: AtomicU64,
    last_width: AtomicU64,
    last_height: AtomicU64,
    pub(crate) latest: Mutex<Option<LatestFrame>>,
    pub dirty: AtomicBool,
}

impl FrameCompositor {
    pub fn new() -> Result<Self, LighError> {
        let device = Device::system_default()
            .ok_or_else(|| LighError::Simctl("no Metal device".into()))?;
        let queue = device.new_command_queue();
        info!(name = ?device.name(), "Metal compositor ready");
        Ok(Self {
            device,
            queue,
            started: Instant::now(),
            frames: AtomicU64::new(0),
            imports_ok: AtomicU64::new(0),
            imports_fail: AtomicU64::new(0),
            last_width: AtomicU64::new(0),
            last_height: AtomicU64::new(0),
            latest: Mutex::new(None),
            dirty: AtomicBool::new(false),
        })
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn ingest(&self, surface_id: u32, width: u32, height: u32) {
        self.frames.fetch_add(1, Ordering::Relaxed);
        self.last_width.store(width as u64, Ordering::Relaxed);
        self.last_height.store(height as u64, Ordering::Relaxed);

        match GpuSurface::lookup(surface_id, width, height) {
            Ok(surf) => match surf.to_metal_texture(&self.device) {
                Ok(tex) => {
                    self.imports_ok.fetch_add(1, Ordering::Relaxed);
                    *self.latest.lock().unwrap() = Some(LatestFrame {
                        texture: tex,
                        width: surf.width,
                        height: surf.height,
                    });
                    self.dirty.store(true, Ordering::Release);
                }
                Err(_) => {
                    self.imports_fail.fetch_add(1, Ordering::Relaxed);
                }
            },
            Err(_) => {
                self.imports_fail.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Blit latest sim frame into `dest` within an existing command buffer.
    pub fn blit_latest_to(&self, dest: &Texture, cmd: &metal::CommandBufferRef) {
        let latest = self.latest.lock().unwrap();
        let Some(frame) = latest.as_ref() else {
            return;
        };

        let enc = cmd.new_blit_command_encoder();
        // Copy the full sim framebuffer. Cropping to dest size showed a black
        // window: dest was ~415×899 while the logo/spinner sits at the center
        // of 1179×2556.
        let sw = frame.width as u64;
        let sh = frame.height as u64;
        if dest.width() < sw || dest.height() < sh {
            return;
        }
        enc.copy_from_texture(
            &frame.texture,
            0,
            0,
            metal::MTLOrigin { x: 0, y: 0, z: 0 },
            metal::MTLSize {
                width: sw,
                height: sh,
                depth: 1,
            },
            dest,
            0,
            0,
            metal::MTLOrigin { x: 0, y: 0, z: 0 },
        );
        enc.end_encoding();
    }

    /// Blit latest sim frame into `dest` (commits immediately).
    pub fn blit_to(&self, dest: &Texture) {
        let cmd = self.queue.new_command_buffer();
        self.blit_latest_to(dest, &cmd);
        cmd.commit();
    }

    pub fn command_queue(&self) -> &CommandQueue {
        &self.queue
    }

    pub fn stats(&self) -> CompositorStats {
        let frames = self.frames.load(Ordering::Relaxed);
        let elapsed = self.started.elapsed().as_secs_f64().max(0.001);
        CompositorStats {
            frames,
            imports_ok: self.imports_ok.load(Ordering::Relaxed),
            imports_fail: self.imports_fail.load(Ordering::Relaxed),
            last_width: self.last_width.load(Ordering::Relaxed) as u32,
            last_height: self.last_height.load(Ordering::Relaxed) as u32,
            fps: frames as f64 / elapsed,
        }
    }
}

pub type HeadlessCompositor = FrameCompositor;

/// Solid-color placeholder drawable while waiting for first IOSurface frame.
pub fn solid_drawable(device: &Device, width: u64, height: u64) -> Texture {
    let desc = TextureDescriptor::new();
    desc.set_texture_type(MTLTextureType::D2);
    desc.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
    desc.set_width(width);
    desc.set_height(height);
    desc.set_usage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
    desc.set_storage_mode(MTLStorageMode::Managed);
    device.new_texture(&desc)
}

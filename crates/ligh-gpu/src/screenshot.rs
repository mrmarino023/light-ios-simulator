//! Screenshot: dump latest MTLTexture (imported from IOSurface) to PNG or raw BGRA.
//!
//! Zero Simulator.app required — reads directly from the compositor's latest frame.

use std::path::Path;

use ligh_core::LighError;
use metal::{MTLPixelFormat, MTLRegion, MTLStorageMode, MTLTextureType, MTLTextureUsage};

use crate::compositor::FrameCompositor;

pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    /// Raw BGRA bytes, row-major.
    pub bgra: Vec<u8>,
}

impl Screenshot {
    /// Capture the latest frame from the compositor as raw BGRA.
    ///
    /// The compositor's MTLTexture lives in shared GPU/CPU memory (IOSurface,
    /// `MTLStorageModeShared`), so `getBytes` is a CPU read of already-mapped
    /// GPU memory — no extra copy or blit needed.
    pub fn capture(compositor: &FrameCompositor) -> Result<Self, LighError> {
        let (tex, w, h) = compositor
            .latest_texture_copy()
            .ok_or_else(|| LighError::NotReady("no frame yet — wait for IOSurface stream".into()))?;

        let bytes_per_row = (w * 4) as usize;
        let mut buf = vec![0u8; bytes_per_row * h as usize];

        let region = MTLRegion {
            origin: metal::MTLOrigin { x: 0, y: 0, z: 0 },
            size: metal::MTLSize {
                width: w as u64,
                height: h as u64,
                depth: 1,
            },
        };
        tex.get_bytes(
            buf.as_mut_ptr() as *mut _,
            bytes_per_row as u64,
            region,
            0,
        );

        Ok(Self { width: w, height: h, bgra: buf })
    }

    /// Write PNG to `path`. Converts BGRA → RGBA inline.
    pub fn write_png(&self, path: &Path) -> Result<(), LighError> {
        use std::io::BufWriter;
        let file = std::fs::File::create(path)
            .map_err(|e| LighError::Io(e))?;
        let w = BufWriter::new(file);
        let mut encoder = png::Encoder::new(w, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()
            .map_err(|e| LighError::Simctl(e.to_string()))?;

        // BGRA → RGBA
        let mut rgba = Vec::with_capacity(self.bgra.len());
        for chunk in self.bgra.chunks_exact(4) {
            rgba.push(chunk[2]); // R
            rgba.push(chunk[1]); // G
            rgba.push(chunk[0]); // B
            rgba.push(chunk[3]); // A
        }

        writer.write_image_data(&rgba)
            .map_err(|e| LighError::Simctl(e.to_string()))
    }

    /// Write raw BGRA bytes to `path`.
    pub fn write_raw(&self, path: &Path) -> Result<(), LighError> {
        std::fs::write(path, &self.bgra).map_err(LighError::Io)
    }

    /// Return PNG bytes (no file write).
    pub fn to_png_bytes(&self) -> Result<Vec<u8>, LighError> {
        let mut buf = Vec::new();
        let mut encoder = png::Encoder::new(&mut buf, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()
            .map_err(|e| LighError::Simctl(e.to_string()))?;
        let mut rgba = Vec::with_capacity(self.bgra.len());
        for chunk in self.bgra.chunks_exact(4) {
            rgba.push(chunk[2]);
            rgba.push(chunk[1]);
            rgba.push(chunk[0]);
            rgba.push(chunk[3]);
        }
        writer.write_image_data(&rgba)
            .map_err(|e| LighError::Simctl(e.to_string()))?;
        drop(writer);
        Ok(buf)
    }
}

/// Blit the latest IOSurface texture into a fresh CPU-readable texture and return it
/// alongside dimensions. Called by `Screenshot::capture`.
impl FrameCompositor {
    /// Returns a CPU-readable snapshot texture + (w, h). Returns `None` if no frame
    /// has been imported yet.
    pub fn latest_texture_copy(&self) -> Option<(metal::Texture, u32, u32)> {
        let guard = self.latest.lock().unwrap();
        let frame = guard.as_ref()?;
        let w = frame.width;
        let h = frame.height;

        // Create a Managed (CPU+GPU) texture of the same size, then blit into it.
        let desc = metal::TextureDescriptor::new();
        desc.set_texture_type(MTLTextureType::D2);
        desc.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        desc.set_width(w as u64);
        desc.set_height(h as u64);
        desc.set_usage(MTLTextureUsage::ShaderRead | MTLTextureUsage::ShaderWrite);
        desc.set_storage_mode(MTLStorageMode::Managed);

        let cpu_tex = self.device().new_texture(&desc);

        let cmd = self.command_queue().new_command_buffer();
        let enc = cmd.new_blit_command_encoder();
        enc.copy_from_texture(
            &frame.texture,
            0, 0,
            metal::MTLOrigin { x: 0, y: 0, z: 0 },
            metal::MTLSize { width: w as u64, height: h as u64, depth: 1 },
            &cpu_tex,
            0, 0,
            metal::MTLOrigin { x: 0, y: 0, z: 0 },
        );
        enc.synchronize_resource(&cpu_tex);
        enc.end_encoding();
        cmd.commit();
        cmd.wait_until_completed();

        Some((cpu_tex, w, h))
    }
}

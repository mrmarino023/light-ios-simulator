//! IOSurface → Metal texture import (zero-copy GPU path).

use std::ffi::c_void;

use ligh_core::LighError;
use metal::foreign_types::ForeignTypeRef;
use metal::{MTLPixelFormat, MTLStorageMode, MTLTextureType, MTLTextureUsage};
use metal::{Texture, TextureDescriptor, TextureRef};
use tracing::debug;

#[link(name = "IOSurface", kind = "framework")]
extern "C" {
    fn IOSurfaceLookup(surface_id: u32) -> *mut c_void;
    fn IOSurfaceGetWidth(surface: *mut c_void) -> usize;
    fn IOSurfaceGetHeight(surface: *mut c_void) -> usize;
}

pub struct GpuSurface {
    pub surface_id: u32,
    pub width: u32,
    pub height: u32,
}

impl GpuSurface {
    pub fn lookup(surface_id: u32, width: u32, height: u32) -> Result<Self, LighError> {
        if surface_id == 0 {
            return Err(LighError::Simctl("invalid IOSurface id".into()));
        }
        let ptr = unsafe { IOSurfaceLookup(surface_id) };
        if ptr.is_null() {
            return Err(LighError::Simctl(format!(
                "IOSurfaceLookup({surface_id}) returned null"
            )));
        }
        let w = unsafe { IOSurfaceGetWidth(ptr) } as u32;
        let h = unsafe { IOSurfaceGetHeight(ptr) } as u32;
        Ok(Self {
            surface_id,
            width: if w > 0 { w } else { width },
            height: if h > 0 { h } else { height },
        })
    }

    /// Import IOSurface into a Metal texture (same memory — no CPU copy).
    pub fn to_metal_texture(
        &self,
        device: &metal::DeviceRef,
    ) -> Result<Texture, LighError> {
        let ptr = unsafe { IOSurfaceLookup(self.surface_id) };
        if ptr.is_null() {
            return Err(LighError::Simctl("IOSurface vanished".into()));
        }

        let desc = TextureDescriptor::new();
        desc.set_texture_type(MTLTextureType::D2);
        desc.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        desc.set_width(self.width as u64);
        desc.set_height(self.height as u64);
        desc.set_usage(MTLTextureUsage::ShaderRead);
        desc.set_storage_mode(MTLStorageMode::Shared);

        let tex = import_iosurface_texture(device, &desc, ptr)
            .ok_or_else(|| LighError::Simctl("Metal IOSurface import failed".into()))?;

        debug!(
            id = self.surface_id,
            w = self.width,
            h = self.height,
            "IOSurface → MTLTexture"
        );
        Ok(tex)
    }
}

/// `-[MTLDevice newTextureWithDescriptor:iosurface:plane:]`
fn import_iosurface_texture(
    device: &metal::DeviceRef,
    desc: &metal::TextureDescriptorRef,
    iosurface: *mut c_void,
) -> Option<Texture> {
    use objc::{msg_send, sel, sel_impl};
    unsafe {
        let device_obj: *mut objc::runtime::Object = device.as_ptr() as *mut _;
        let desc_obj: *mut objc::runtime::Object = desc.as_ptr() as *mut _;
        let tex: *mut objc::runtime::Object =
            msg_send![device_obj, newTextureWithDescriptor:desc_obj iosurface:iosurface plane:0u64];
        if tex.is_null() {
            return None;
        }
        let tex_ref = TextureRef::from_ptr(tex as *mut _);
        Some(Texture::from(tex_ref.to_owned()))
    }
}

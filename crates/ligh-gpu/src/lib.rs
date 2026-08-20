//! GPU-native host compositor for LIGH v3.

#[cfg(target_os = "macos")]
#[link(name = "QuartzCore", kind = "framework")]
#[link(name = "AppKit", kind = "framework")]
extern "C" {}

pub mod compositor;
pub mod gui;
pub mod layout;
pub mod screenshot;
pub mod surface;

pub use compositor::{CompositorStats, FrameCompositor, HeadlessCompositor};
pub use gui::{run_window, GuiOptions, PointerPhase, TouchBridge};
pub use screenshot::Screenshot;
pub use surface::GpuSurface;

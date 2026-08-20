//! Device-chrome Metal window — glass shows the sim, bezel is host chrome.
//!
//! Coordinate pipeline:
//!   winit logical points → DeviceLayout.hit_screen → 0..1 glass
//!   → IndigoHID **points** (393×852), never framebuffer pixels.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ligh_core::LighError;
use raw_window_handle::HasWindowHandle;
use tracing::info;
use winit::{
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition},
    event::{ElementState, Event, MouseButton, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{Key, ModifiersState},
    window::WindowBuilder,
};

use crate::compositor::FrameCompositor;
use crate::layout::{DeviceLayout, Rect};

pub struct GuiOptions {
    pub title: String,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub point_width: f64,
    pub point_height: f64,
    pub tablet: bool,
    /// Auto-close after N seconds (headless CI / `ligh gui --verify`).
    pub self_test_secs: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
pub enum PointerPhase {
    Down,
    Move,
    Up,
}

pub struct TouchBridge {
    pub pointer: Box<dyn Fn(PointerPhase, f64, f64) -> Result<(), LighError> + Send>,
    pub home: Box<dyn Fn() -> Result<(), LighError> + Send>,
}

struct ChromeLayers {
    chassis: *mut objc::runtime::Object,
    metal: *mut objc::runtime::Object,
    power: *mut objc::runtime::Object,
    vol_up: *mut objc::runtime::Object,
    vol_down: *mut objc::runtime::Object,
}

pub fn run_window(
    compositor: Arc<FrameCompositor>,
    touch: TouchBridge,
    opts: GuiOptions,
) -> Result<(), LighError> {
    let event_loop = EventLoop::new().map_err(|e| LighError::Simctl(e.to_string()))?;
    let screen_pts = (opts.point_width, opts.point_height);
    let (win_w, win_h) = DeviceLayout::preferred_window_size(screen_pts, opts.tablet);

    let window = WindowBuilder::new()
        .with_title(opts.title.clone())
        .with_inner_size(LogicalSize::new(win_w, win_h))
        .with_min_inner_size(LogicalSize::new(280.0, 480.0))
        .build(&event_loop)
        .map_err(|e| LighError::Simctl(e.to_string()))?;

    let handle = window
        .window_handle()
        .map_err(|e| LighError::Simctl(e.to_string()))?;
    let raw = handle.as_raw();
    let raw_window_handle::RawWindowHandle::AppKit(appkit) = raw else {
        return Err(LighError::Simctl("expected AppKit window".into()));
    };

    let view = appkit.ns_view.as_ptr() as *mut objc::runtime::Object;
    let layers = unsafe { install_chrome(view, &compositor, &opts)? };
    let mut layout = DeviceLayout::in_window(win_w, win_h, screen_pts, opts.tablet);
    unsafe {
        apply_layout(view, &layers, &layout, win_h);
    }

    info!(
        win_w,
        win_h,
        glass_w = layout.screen.w,
        glass_h = layout.screen.h,
        self_test = ?opts.self_test_secs,
        "LIGH GUI — click the glass; ⌘H = home"
    );

    let mut mouse_down = false;
    let mut cursor = (0.0f64, 0.0f64);
    let mut last_uv = (0.5f64, 0.5f64);
    let mut modifiers = ModifiersState::empty();
    let gui_started = Instant::now();
    let frames_presented = Arc::new(AtomicU64::new(0));
    let self_test_secs = opts.self_test_secs;
    let verify_result: Arc<Mutex<Result<(), LighError>>> = Arc::new(Mutex::new(Ok(())));
    let verify_result_loop = verify_result.clone();
    let frames_presented_loop = frames_presented.clone();
    let tablet = opts.tablet;
    let metal_ptr = layers.metal as *mut std::ffi::c_void;

    event_loop
        .run(move |event, elwt| {
            elwt.set_control_flow(ControlFlow::Poll);

            match event {
                Event::WindowEvent { event, window_id } if window_id == window.id() => {
                    match event {
                        WindowEvent::CloseRequested => elwt.exit(),
                        WindowEvent::Resized(size) => {
                            let scale = window.scale_factor();
                            let lw = size.width as f64 / scale;
                            let lh = size.height as f64 / scale;
                            layout = DeviceLayout::in_window(lw, lh, screen_pts, tablet);
                            unsafe {
                                apply_layout(view, &layers, &layout, lh);
                            }
                        }
                        WindowEvent::ModifiersChanged(m) => modifiers = m.state(),
                        WindowEvent::KeyboardInput { event, .. } => {
                            if event.state == ElementState::Pressed
                                && modifiers.super_key()
                                && event.logical_key == Key::Character("h".into())
                            {
                                let _ = (touch.home)();
                            }
                        }
                        WindowEvent::CursorMoved { position, .. } => {
                            cursor = logical_cursor(&window, position);
                            if mouse_down {
                                if let Some(uv) = layout.hit_screen(cursor.0, cursor.1) {
                                    last_uv = uv;
                                }
                                let _ = (touch.pointer)(PointerPhase::Move, last_uv.0, last_uv.1);
                            }
                        }
                        WindowEvent::MouseInput { state, button, .. } => {
                            if button != MouseButton::Left {
                                return;
                            }
                            match state {
                                ElementState::Pressed => {
                                    if let Some(uv) = layout.hit_screen(cursor.0, cursor.1) {
                                        last_uv = uv;
                                        mouse_down = true;
                                        let _ = (touch.pointer)(
                                            PointerPhase::Down,
                                            last_uv.0,
                                            last_uv.1,
                                        );
                                    }
                                }
                                ElementState::Released => {
                                    if mouse_down {
                                        mouse_down = false;
                                        if let Some(uv) = layout.hit_screen(cursor.0, cursor.1) {
                                            last_uv = uv;
                                        }
                                        let _ = (touch.pointer)(
                                            PointerPhase::Up,
                                            last_uv.0,
                                            last_uv.1,
                                        );
                                    }
                                }
                            }
                        }
                        WindowEvent::RedrawRequested => {
                            if present_frame(&compositor, metal_ptr) {
                                frames_presented_loop.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        _ => {}
                    }
                }
                Event::AboutToWait => {
                    window.request_redraw();
                    if let Some(secs) = self_test_secs {
                        if gui_started.elapsed() >= Duration::from_secs(secs) {
                            let presented = frames_presented_loop.load(Ordering::Relaxed);
                            let imports = compositor.stats().imports_ok;
                            if imports >= 1 && presented >= 1 {
                                info!(presented, imports, "GUI verify ok");
                            } else {
                                *verify_result_loop.lock().unwrap() = Err(LighError::NotReady(
                                    format!(
                                        "GUI verify failed: {imports} imports, {presented} frames presented"
                                    ),
                                ));
                            }
                            elwt.exit();
                        }
                    }
                }
                _ => {}
            }
        })
        .map_err(|e| LighError::Simctl(e.to_string()))?;

    Arc::try_unwrap(verify_result)
        .unwrap_or_else(|_| Mutex::new(Ok(())))
        .into_inner()
        .unwrap_or(Ok(()))
}

fn logical_cursor(
    window: &winit::window::Window,
    position: PhysicalPosition<f64>,
) -> (f64, f64) {
    let p: LogicalPosition<f64> = position.to_logical(window.scale_factor());
    (p.x, p.y)
}

fn present_frame(compositor: &FrameCompositor, layer_ptr: *mut std::ffi::c_void) -> bool {
    let stats = compositor.stats();
    if stats.imports_ok == 0 || stats.last_width == 0 || stats.last_height == 0 {
        return false;
    }
    unsafe {
        use metal::foreign_types::ForeignTypeRef;
        use objc::{msg_send, sel, sel_impl};
        let layer: *mut objc::runtime::Object = layer_ptr.cast();
        let size = cocoa::foundation::NSSize::new(stats.last_width as f64, stats.last_height as f64);
        let _: () = msg_send![layer, setDrawableSize: size];
        let drawable: *mut objc::runtime::Object = msg_send![layer, nextDrawable];
        if drawable.is_null() {
            return false;
        }
        let tex: *mut objc::runtime::Object = msg_send![drawable, texture];
        if tex.is_null() {
            return false;
        }

        let tex_ref = metal::TextureRef::from_ptr(tex as *mut _);
        let dest = metal::Texture::from(tex_ref.to_owned());
        let cmd = compositor.command_queue().new_command_buffer();
        compositor.blit_latest_to(&dest, &cmd);
        let draw_ref = metal::MetalDrawableRef::from_ptr(drawable as *mut _);
        let draw = metal::MetalDrawable::from(draw_ref.to_owned());
        cmd.present_drawable(&draw);
        cmd.commit();
        true
    }
}

unsafe fn install_chrome(
    view: *mut objc::runtime::Object,
    compositor: &FrameCompositor,
    opts: &GuiOptions,
) -> Result<ChromeLayers, LighError> {
    use cocoa::base::nil;
    use cocoa::foundation::NSString;
    use metal::foreign_types::ForeignType;
    use objc::{class, msg_send, sel, sel_impl};

    let _: () = msg_send![view, setWantsLayer: true];
    let view_layer: *mut objc::runtime::Object = msg_send![view, layer];
    if view_layer.is_null() {
        return Err(LighError::Simctl("NSView has no layer".into()));
    }
    set_bg(view_layer, 0.23, 0.23, 0.24, 1.0);

    let chassis = new_layer();
    set_bg(chassis, 0.07, 0.07, 0.08, 1.0);
    let _: () = msg_send![view_layer, addSublayer: chassis];

    let metal: *mut objc::runtime::Object = msg_send![class!(CAMetalLayer), layer];
    if metal.is_null() {
        return Err(LighError::Simctl("CAMetalLayer alloc failed".into()));
    }
    let dev: *mut objc::runtime::Object = compositor.device().as_ptr() as *mut _;
    let _: () = msg_send![metal, setDevice: dev];
    let _: () = msg_send![metal, setPixelFormat: 80u64]; // BGRA8Unorm
    let _: () = msg_send![metal, setFramebufferOnly: false];
    let _: () = msg_send![metal, setOpaque: true];
    let _: () = msg_send![metal, setMasksToBounds: true];
    let gravity = NSString::alloc(nil).init_str("resize");
    let _: () = msg_send![metal, setContentsGravity: gravity];
    let size = cocoa::foundation::NSSize::new(opts.pixel_width as f64, opts.pixel_height as f64);
    let _: () = msg_send![metal, setDrawableSize: size];
    let _: () = msg_send![view_layer, addSublayer: metal];

    let power = new_layer();
    let vol_up = new_layer();
    let vol_down = new_layer();
    set_bg(power, 0.16, 0.16, 0.17, 1.0);
    set_bg(vol_up, 0.16, 0.16, 0.17, 1.0);
    set_bg(vol_down, 0.16, 0.16, 0.17, 1.0);
    let _: () = msg_send![view_layer, addSublayer: power];
    let _: () = msg_send![view_layer, addSublayer: vol_up];
    let _: () = msg_send![view_layer, addSublayer: vol_down];

    Ok(ChromeLayers {
        chassis,
        metal,
        power,
        vol_up,
        vol_down,
    })
}

unsafe fn apply_layout(
    view: *mut objc::runtime::Object,
    layers: &ChromeLayers,
    layout: &DeviceLayout,
    view_h: f64,
) {
    use objc::{msg_send, sel, sel_impl};

    set_frame(layers.chassis, layout.chassis, view_h);
    let _: () = msg_send![layers.chassis, setCornerRadius: layout.chassis_radius];

    set_frame(layers.metal, layout.screen, view_h);
    let _: () = msg_send![layers.metal, setCornerRadius: layout.screen_radius];

    set_frame(layers.power, layout.power, view_h);
    set_frame(layers.vol_up, layout.vol_up, view_h);
    set_frame(layers.vol_down, layout.vol_down, view_h);
    let btn_r = (layout.power.w * 0.45).min(2.5);
    let _: () = msg_send![layers.power, setCornerRadius: btn_r];
    let _: () = msg_send![layers.vol_up, setCornerRadius: btn_r];
    let _: () = msg_send![layers.vol_down, setCornerRadius: btn_r];

    let _: () = msg_send![view, setNeedsDisplay: true];
}

unsafe fn new_layer() -> *mut objc::runtime::Object {
    use objc::{class, msg_send, sel, sel_impl};
    msg_send![class!(CALayer), layer]
}

unsafe fn set_bg(layer: *mut objc::runtime::Object, r: f64, g: f64, b: f64, a: f64) {
    use objc::{class, msg_send, sel, sel_impl};
    let color: *mut objc::runtime::Object = msg_send![
        class!(NSColor),
        colorWithCalibratedRed: r green: g blue: b alpha: a
    ];
    let cg: *mut std::ffi::c_void = msg_send![color, CGColor];
    let _: () = msg_send![layer, setBackgroundColor: cg];
}

unsafe fn set_frame(layer: *mut objc::runtime::Object, r: Rect, view_h: f64) {
    use cocoa::foundation::{NSPoint, NSRect, NSSize};
    use objc::{msg_send, sel, sel_impl};
    let rect = NSRect::new(
        NSPoint::new(r.x, view_h - r.y - r.h),
        NSSize::new(r.w, r.h),
    );
    let _: () = msg_send![layer, setFrame: rect];
}

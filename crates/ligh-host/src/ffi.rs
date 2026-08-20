#![allow(non_camel_case_types, non_snake_case, dead_code)]

use std::os::raw::c_void;

#[repr(C)]
pub struct LighHostError {
    pub message: *const i8,
    pub code: i32,
}

pub type LighFrameFn = Option<extern "C" fn(*mut c_void, u32, u32, u32)>;

extern "C" {
    pub fn ligh_host_init(developer_dir: *const i8, err: *mut LighHostError) -> bool;
    pub fn ligh_host_boot(udid: *const i8, err: *mut LighHostError) -> bool;
    pub fn ligh_host_shutdown(udid: *const i8, err: *mut LighHostError) -> bool;
    pub fn ligh_host_stream_start(
        udid: *const i8,
        callback: LighFrameFn,
        ctx: *mut c_void,
        err: *mut LighHostError,
    ) -> bool;
    pub fn ligh_host_hid_tap(
        udid: *const i8,
        norm_x: f64,
        norm_y: f64,
        width: f64,
        height: f64,
        err: *mut LighHostError,
    ) -> bool;
    pub fn ligh_host_hid_swipe(
        udid: *const i8,
        from_norm_x: f64,
        from_norm_y: f64,
        to_norm_x: f64,
        to_norm_y: f64,
        width: f64,
        height: f64,
        err: *mut LighHostError,
    ) -> bool;
    pub fn ligh_host_hid_home(udid: *const i8, err: *mut LighHostError) -> bool;
    pub fn ligh_host_hid_prepare(udid: *const i8, err: *mut LighHostError) -> bool;
    pub fn ligh_host_hid_type(udid: *const i8, text: *const i8, err: *mut LighHostError) -> bool;
    pub fn ligh_host_hid_pointer(
        udid: *const i8,
        norm_x: f64,
        norm_y: f64,
        phase: u32,
        width: f64,
        height: f64,
        err: *mut LighHostError,
    ) -> bool;
    pub fn ligh_host_ax_dump(udid: *const i8, err: *mut LighHostError) -> *mut i8;
    pub fn ligh_host_ax_free(ptr: *mut i8);
    pub fn ligh_host_stream_stop();
    pub fn ligh_host_stream_poll();
}

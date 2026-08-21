#pragma once

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Called on the framebuffer queue when a new IOSurface frame is available.
typedef void (*LighFrameFn)(void *ctx, uint32_t surface_id, uint32_t width, uint32_t height);

typedef struct {
    const char *message;
    int code;
} LighHostError;

/// Load CoreSimulator + SimulatorKit from Xcode's developer dir.
bool ligh_host_init(const char *developer_dir, LighHostError *err);

/// Subscribe to `com.apple.framebuffer.display` IOSurface updates for `udid`.
bool ligh_host_stream_start(const char *udid, LighFrameFn callback, void *ctx, LighHostError *err);

void ligh_host_stream_stop(void);

/// Re-read latest IOSurface and invoke frame callback (for headless polling).
void ligh_host_stream_poll(void);

/// Headless boot via private SimDevice API (no Simulator.app).
bool ligh_host_boot(const char *udid, LighHostError *err);

bool ligh_host_shutdown(const char *udid, LighHostError *err);

/// Normalized touch (0..1). `width`/`height` = sim size in **points** (not pixels).
bool ligh_host_hid_tap(const char *udid, double norm_x, double norm_y, double width,
                       double height, LighHostError *err);

/// Long-press: touch down, hold `hold_ms`, touch up.
bool ligh_host_hid_tap_hold(const char *udid, double norm_x, double norm_y, double width,
                            double height, double hold_ms, LighHostError *err);

bool ligh_host_hid_swipe(const char *udid,
                         double from_norm_x, double from_norm_y,
                         double to_norm_x, double to_norm_y,
                         double width, double height,
                         LighHostError *err);

/// Pointer phase: 1 = down, 2 = up, 3 = move. Coords 0..1, size in points.
bool ligh_host_hid_pointer(const char *udid, double norm_x, double norm_y,
                           uint32_t phase, double width, double height,
                           LighHostError *err);

bool ligh_host_hid_home(const char *udid, LighHostError *err);

/// Ensure HID client + pointer/mouse services exist (warmup — no gesture).
bool ligh_host_hid_prepare(const char *udid, LighHostError *err);

/// Type UTF-8 text via IndigoHID keyboard (down/up per character).
bool ligh_host_hid_type(const char *udid, const char *text, LighHostError *err);

/// Single USB HID keyboard usage (down+up). e.g. 0x2A delete, 0x28 return.
bool ligh_host_hid_key(const char *udid, uint32_t usage, LighHostError *err);

/// Dump frontmost-app accessibility tree as JSON (caller frees with ligh_host_ax_free).
/// Headless — uses AccessibilityPlatformTranslation + SimDevice XPC (no Simulator.app).
char *ligh_host_ax_dump(const char *udid, LighHostError *err);
void ligh_host_ax_free(char *ptr);

#ifdef __cplusplus
}
#endif

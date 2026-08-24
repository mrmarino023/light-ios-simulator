#import <Foundation/Foundation.h>
#import <AppKit/AppKit.h>
#import <CoreGraphics/CoreGraphics.h>
#import <dlfcn.h>
#import <objc/runtime.h>
#import <unistd.h>
#import <dispatch/dispatch.h>
#import <string.h>

#import "display_bridge.h"
#import "frameworks.h"

typedef void *(*MouseFn)(const CGPoint *, const CGPoint *, uint32_t, uint32_t, CGSize, uint32_t);
typedef void *(*ServiceFn)(void);
typedef void *(*KbdNsFn)(id);
typedef void *(*KbdArbFn)(uint32_t, uint32_t);
typedef void (*SendFn)(id, SEL, void *, BOOL, id, id);

static id g_hid_client = nil;
static char g_hid_udid[64] = {0};
static MouseFn g_mouse_fn = nil;
static ServiceFn g_create_pointer = nil;
static ServiceFn g_create_mouse = nil;
static KbdNsFn g_kbd_ns = nil;
static KbdArbFn g_kbd_arb = nil;
static BOOL g_hid_services_sent = NO;

static id resolve_device(NSString *udid);
static void resolve_hid_symbols(void);
static id ensure_hid_client(const char *udid_c);
static void send_message(void *msg, id client);
static BOOL send_mouse(id client, CGPoint pt, uint32_t event_type, uint32_t direction,
                       double width, double height);

static id resolve_device(NSString *udid) {
    NSString *dev_dir = [[NSProcessInfo processInfo].environment objectForKey:@"DEVELOPER_DIR"];
    if (!dev_dir.length) dev_dir = @"/Applications/Xcode.app/Contents/Developer";

    Class ctx_cls = NSClassFromString(@"SimServiceContext");
    SEL ctx_sel = NSSelectorFromString(@"sharedServiceContextForDeveloperDir:error:");
    IMP ctx_imp = class_getMethodImplementation(object_getClass(ctx_cls), ctx_sel);
    if (!ctx_imp) return nil;

    id (*ctx_fn)(Class, SEL, id, NSError **) = (id (*)(Class, SEL, id, NSError **))ctx_imp;
    NSError *err = nil;
    id ctx = ctx_fn(ctx_cls, ctx_sel, dev_dir, &err);
    if (!ctx) return nil;

    SEL set_sel = NSSelectorFromString(@"defaultDeviceSetWithError:");
    IMP set_imp = class_getMethodImplementation(object_getClass(ctx), set_sel);
    id (*set_fn)(id, SEL, NSError **) = (id (*)(id, SEL, NSError **))set_imp;
    id set = set_fn(ctx, set_sel, &err);
    if (!set) return nil;

    for (id device in [set valueForKey:@"availableDevices"]) {
        NSUUID *dudid = [device valueForKey:@"UDID"];
        if ([dudid.UUIDString isEqualToString:udid]) return device;
    }
    return nil;
}

static void resolve_hid_symbols(void) {
    if (g_mouse_fn) return;
    NSString *dev = [[NSProcessInfo processInfo].environment objectForKey:@"DEVELOPER_DIR"];
    if (!dev.length) dev = @"/Applications/Xcode.app/Contents/Developer";
    NSString *path = [dev stringByAppendingPathComponent:
        @"Library/PrivateFrameworks/SimulatorKit.framework/SimulatorKit"];
    void *handle = dlopen(path.fileSystemRepresentation, RTLD_NOW);
    if (!handle) return;
    g_mouse_fn = (MouseFn)dlsym(handle, "IndigoHIDMessageForMouseNSEvent");
    g_create_pointer = (ServiceFn)dlsym(handle, "IndigoHIDMessageToCreatePointerService");
    g_create_mouse = (ServiceFn)dlsym(handle, "IndigoHIDMessageToCreateMouseService");
    g_kbd_ns = (KbdNsFn)dlsym(handle, "IndigoHIDMessageForKeyboardNSEvent");
    g_kbd_arb = (KbdArbFn)dlsym(handle, "IndigoHIDMessageForKeyboardArbitrary");
}

static id ensure_hid_client(const char *udid_c) {
    if (g_hid_client && strcmp(g_hid_udid, udid_c) == 0) return g_hid_client;
    if (g_hid_udid[0] && strcmp(g_hid_udid, udid_c) != 0) g_hid_services_sent = NO;

    resolve_hid_symbols();
    NSString *udid = [NSString stringWithUTF8String:udid_c];
    id device = resolve_device(udid);
    if (!device) return nil;

    Class cls = NSClassFromString(@"_TtC12SimulatorKit24SimDeviceLegacyHIDClient");
    if (!cls) return nil;

    id (*alloc_fn)(Class, SEL) = (id (*)(Class, SEL))class_getMethodImplementation(
        object_getClass(cls), @selector(alloc));
    id inst = alloc_fn(cls, @selector(alloc));

    SEL init_sel = NSSelectorFromString(@"initWithDevice:error:");
    IMP init_imp = class_getMethodImplementation(object_getClass(inst), init_sel);
    id (*init_fn)(id, SEL, id, NSError **) = (id (*)(id, SEL, id, NSError **))init_imp;
    NSError *err = nil;
    id client = init_fn(inst, init_sel, device, &err);
    if (!client) return nil;

    if (!g_hid_services_sent) {
        if (g_create_pointer) {
            void *msg = g_create_pointer();
            if (msg) send_message(msg, client);
        }
        if (g_create_mouse) {
            void *msg = g_create_mouse();
            if (msg) send_message(msg, client);
        }
        // One vsync is enough for the guest to register the virtual devices.
        usleep(16000);
        g_hid_services_sent = YES;
    }

    g_hid_client = client;
    strncpy(g_hid_udid, udid_c, sizeof(g_hid_udid) - 1);
    return client;
}

static BOOL send_message_wait(void *msg, id client, int64_t timeout_ns) {
    SEL sel = NSSelectorFromString(@"sendWithMessage:freeWhenDone:completionQueue:completion:");
    IMP imp = class_getMethodImplementation(object_getClass(client), sel);
    if (!imp) return NO;
    SendFn fn = (SendFn)imp;
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    dispatch_queue_t q = dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0);
    void (^completion)(id) = ^(id error) {
        (void)error;
        dispatch_semaphore_signal(sem);
    };
    fn(client, sel, msg, YES, q, completion);
    return dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, timeout_ns)) == 0;
}

static void send_message(void *msg, id client) {
    (void)send_message_wait(msg, client, (int64_t)50 * NSEC_PER_MSEC);
}

static BOOL send_mouse(id client, CGPoint pt, uint32_t event_type, uint32_t direction,
                       double width, double height) {
    if (!g_mouse_fn) return NO;
    // SimulatorKit: IndigoHIDMessageForMouseNSEvent(CGPoint *, CGPoint *, IndigoHIDTarget, NSEventType, NSSize, IndigoHIDEdge)
    // NSEventTypeLeftMouseDown=1, LeftMouseUp=2, MouseMoved=5, LeftMouseDragged=6.
    uint32_t ns_type = event_type;
    if (event_type == 3) ns_type = 6; // our "move" → dragged while button down
    CGSize screen = CGSizeMake(width, height);
    void *msg = g_mouse_fn(&pt, NULL, 0x32, ns_type, screen, 0);
    if (!msg) return NO;
    send_message(msg, client);
    return YES;
}

/// IndigoHID location is in **points**, matching `width`/`height`.
static CGPoint point_from_norm(double norm_x, double norm_y, double width, double height) {
    double x = norm_x < 0 ? 0 : (norm_x > 1 ? 1 : norm_x);
    double y = norm_y < 0 ? 0 : (norm_y > 1 ? 1 : norm_y);
    return CGPointMake(x * width, y * height);
}

bool ligh_host_hid_tap(const char *udid, double norm_x, double norm_y, double width,
                       double height, LighHostError *err) {
    return ligh_host_hid_tap_hold(udid, norm_x, norm_y, width, height, 80.0, err);
}

bool ligh_host_hid_tap_hold(const char *udid, double norm_x, double norm_y, double width,
                            double height, double hold_ms, LighHostError *err) {
    if (!ligh_load_private_frameworks(NULL)) {
        if (err) { err->code = 20; err->message = "frameworks not loaded"; }
        return false;
    }
    id client = ensure_hid_client(udid);
    if (!client) {
        if (err) { err->code = 21; err->message = "HID client unavailable"; }
        return false;
    }

    CGPoint pt = point_from_norm(norm_x, norm_y, width, height);

    if (!send_mouse(client, pt, 1, 1, width, height)) {
        if (err) { err->code = 22; err->message = "touch down failed"; }
        return false;
    }
    useconds_t hold = (useconds_t)(hold_ms * 1000.0);
    if (hold < 80000) hold = 80000;
    if (hold > 5000000) hold = 5000000;
    usleep(hold);
    if (!send_mouse(client, pt, 2, 2, width, height)) {
        if (err) { err->code = 23; err->message = "touch up failed"; }
        return false;
    }
    return true;
}

bool ligh_host_hid_swipe(const char *udid,
                         double from_norm_x, double from_norm_y,
                         double to_norm_x, double to_norm_y,
                         double width, double height,
                         LighHostError *err) {
    if (!ligh_load_private_frameworks(NULL)) {
        if (err) { err->code = 30; err->message = "frameworks not loaded"; }
        return false;
    }
    id client = ensure_hid_client(udid);
    if (!client) {
        if (err) { err->code = 31; err->message = "HID client unavailable"; }
        return false;
    }

    CGPoint from = point_from_norm(from_norm_x, from_norm_y, width, height);
    CGPoint to_pt = point_from_norm(to_norm_x, to_norm_y, width, height);

    // touch down at from
    if (!send_mouse(client, from, 1, 1, width, height)) {
        if (err) { err->code = 32; err->message = "swipe down failed"; }
        return false;
    }
    // interpolate move events (4 steps)
    for (int i = 1; i <= 4; i++) {
        double t = (double)i / 5.0;
        CGPoint mid = CGPointMake(from.x + (to_pt.x - from.x) * t,
                                  from.y + (to_pt.y - from.y) * t);
        usleep(8000);
        send_mouse(client, mid, 3, 3, width, height); // move
    }
    usleep(8000);
    // touch up at to
    if (!send_mouse(client, to_pt, 2, 2, width, height)) {
        if (err) { err->code = 33; err->message = "swipe up failed"; }
        return false;
    }
    return true;
}

bool ligh_host_hid_pointer(const char *udid, double norm_x, double norm_y,
                           uint32_t phase, double width, double height,
                           LighHostError *err) {
    if (!ligh_load_private_frameworks(NULL)) {
        if (err) { err->code = 40; err->message = "frameworks not loaded"; }
        return false;
    }
    id client = ensure_hid_client(udid);
    if (!client) {
        if (err) { err->code = 41; err->message = "HID client unavailable"; }
        return false;
    }
    uint32_t event_type = phase;
    if (event_type != 1 && event_type != 2 && event_type != 3) {
        if (err) { err->code = 42; err->message = "invalid pointer phase"; }
        return false;
    }
    CGPoint pt = point_from_norm(norm_x, norm_y, width, height);
    if (!send_mouse(client, pt, event_type, event_type, width, height)) {
        if (err) { err->code = 43; err->message = "pointer event failed"; }
        return false;
    }
    return true;
}

bool ligh_host_hid_home(const char *udid, LighHostError *err) {
    if (!ligh_load_private_frameworks(NULL)) return false;
    id client = ensure_hid_client(udid);
    if (!client) return false;

    void *(*btn_fn)(uint32_t, uint32_t, uint32_t) = NULL;
    NSString *dev = [[NSProcessInfo processInfo].environment objectForKey:@"DEVELOPER_DIR"];
    if (!dev.length) dev = @"/Applications/Xcode.app/Contents/Developer";
    NSString *path = [dev stringByAppendingPathComponent:
        @"Library/PrivateFrameworks/SimulatorKit.framework/SimulatorKit"];
    void *handle = dlopen(path.fileSystemRepresentation, RTLD_NOW);
    if (handle) btn_fn = dlsym(handle, "IndigoHIDMessageForButton");

    if (!btn_fn) {
        if (err) { err->code = 24; err->message = "IndigoHIDMessageForButton missing"; }
        return false;
    }
    void *down = btn_fn(0, 1, 0x33);
    void *up = btn_fn(0, 2, 0x33);
    if (down) send_message(down, client);
    usleep(16000);
    if (up) send_message(up, client);
    return true;
}

bool ligh_host_hid_prepare(const char *udid, LighHostError *err) {
    if (!ligh_load_private_frameworks(NULL)) {
        if (err) { err->code = 20; err->message = "frameworks not loaded"; }
        return false;
    }
    id client = ensure_hid_client(udid);
    if (!client) {
        if (err) { err->code = 21; err->message = "HID client unavailable"; }
        return false;
    }
    return true;
}

static BOOL send_key_nsevent(id client, NSString *ch, BOOL down) {
    if (!g_kbd_ns || ch.length == 0) return NO;
    NSEventType type = down ? NSEventTypeKeyDown : NSEventTypeKeyUp;
    NSEvent *ev = [NSEvent keyEventWithType:type
                                   location:NSZeroPoint
                              modifierFlags:0
                                  timestamp:0
                               windowNumber:0
                                    context:nil
                                 characters:ch
                charactersIgnoringModifiers:ch
                                  isARepeat:NO
                                    keyCode:0];
    if (!ev) return NO;
    void *msg = g_kbd_ns(ev);
    if (!msg) return NO;
    send_message(msg, client);
    return YES;
}

static uint32_t hid_usage_for_ascii(unsigned char c, BOOL *need_shift) {
    *need_shift = NO;
    if (c >= 'A' && c <= 'Z') { *need_shift = YES; return 0x04u + (c - 'A'); }
    if (c >= 'a' && c <= 'z') return 0x04u + (c - 'a');
    if (c >= '1' && c <= '9') return 0x1Eu + (c - '1');
    if (c == '0') return 0x27;
    if (c == '\n' || c == '\r') return 0x28;
    if (c == '\b') return 0x2A;
    if (c == '\t') return 0x2B;
    if (c == ' ') return 0x2C;
    *need_shift = YES;
    switch (c) {
        case '!': return 0x1E;
        case '@': return 0x1F;
        case '#': return 0x20;
        case '$': return 0x21;
        case '%': return 0x22;
        case '^': return 0x23;
        case '&': return 0x24;
        case '*': return 0x25;
        case '(': return 0x26;
        case ')': return 0x27;
        default: break;
    }
    *need_shift = NO;
    switch (c) {
        case '-': return 0x2D;
        case '=': return 0x2E;
        case '[': return 0x2F;
        case ']': return 0x30;
        case '\\': return 0x31;
        case ';': return 0x33;
        case '\'': return 0x34;
        case '`': return 0x35;
        case ',': return 0x36;
        case '.': return 0x37;
        case '/': return 0x38;
        case '_': *need_shift = YES; return 0x2D;
        case '+': *need_shift = YES; return 0x2E;
        case '{': *need_shift = YES; return 0x2F;
        case '}': *need_shift = YES; return 0x30;
        case '|': *need_shift = YES; return 0x31;
        case ':': *need_shift = YES; return 0x33;
        case '"': *need_shift = YES; return 0x34;
        case '~': *need_shift = YES; return 0x35;
        case '<': *need_shift = YES; return 0x36;
        case '>': *need_shift = YES; return 0x37;
        case '?': *need_shift = YES; return 0x38;
        default: return 0;
    }
}

bool ligh_host_hid_type(const char *udid, const char *text, LighHostError *err) {
    if (!text) {
        if (err) { err->code = 44; err->message = "empty text"; }
        return false;
    }
    if (!ligh_load_private_frameworks(NULL)) {
        if (err) { err->code = 20; err->message = "frameworks not loaded"; }
        return false;
    }
    id client = ensure_hid_client(udid);
    if (!client) {
        if (err) { err->code = 21; err->message = "HID client unavailable"; }
        return false;
    }
    if (!g_kbd_arb) {
        if (err) { err->code = 45; err->message = "IndigoHIDMessageForKeyboardArbitrary missing"; }
        return false;
    }

    // NSEvent path with keyCode:0 types 'a' for every glyph — use USB HID usages only.
    const unsigned char *bytes = (const unsigned char *)text;
    size_t len = strlen(text);
    BOOL used_any = NO;
    for (size_t i = 0; i < len; i++) {
        unsigned char c = bytes[i];
        if (c >= 0x80) {
            if (err) { err->code = 46; err->message = "non-ascii type not supported yet"; }
            return false;
        }
        BOOL shift = NO;
        uint32_t usage = hid_usage_for_ascii(c, &shift);
        if (!usage) {
            if (err) { err->code = 46; err->message = "unsupported character for HID type"; }
            return false;
        }
        if (shift) {
            void *sd = g_kbd_arb(0xE1, 1);
            if (sd) send_message(sd, client);
            usleep(1000);
        }
        void *kd = g_kbd_arb(usage, 1);
        if (kd) send_message(kd, client);
        usleep(8000);
        void *ku = g_kbd_arb(usage, 2);
        if (ku) send_message(ku, client);
        if (shift) {
            usleep(1000);
            void *su = g_kbd_arb(0xE1, 2);
            if (su) send_message(su, client);
        }
        usleep(12000);
        used_any = YES;
    }
    return used_any || len == 0;
}

bool ligh_host_hid_key(const char *udid, uint32_t usage, LighHostError *err) {
    if (!usage) {
        if (err) { err->code = 47; err->message = "invalid key usage"; }
        return false;
    }
    if (!ligh_load_private_frameworks(NULL)) {
        if (err) { err->code = 20; err->message = "frameworks not loaded"; }
        return false;
    }
    id client = ensure_hid_client(udid);
    if (!client) {
        if (err) { err->code = 21; err->message = "HID client unavailable"; }
        return false;
    }
    if (!g_kbd_arb) {
        if (err) { err->code = 45; err->message = "IndigoHIDMessageForKeyboardArbitrary missing"; }
        return false;
    }
    void *kd = g_kbd_arb(usage, 1);
    if (kd) send_message(kd, client);
    usleep(10000);
    void *ku = g_kbd_arb(usage, 2);
    if (ku) send_message(ku, client);
    usleep(8000);
    return true;
}

bool ligh_host_hid_chord(const char *udid, uint32_t mod_usage, uint32_t key_usage,
                         LighHostError *err) {
    if (!mod_usage || !key_usage) {
        if (err) { err->code = 47; err->message = "invalid chord usage"; }
        return false;
    }
    if (!ligh_load_private_frameworks(NULL)) {
        if (err) { err->code = 20; err->message = "frameworks not loaded"; }
        return false;
    }
    id client = ensure_hid_client(udid);
    if (!client) {
        if (err) { err->code = 21; err->message = "HID client unavailable"; }
        return false;
    }
    if (!g_kbd_arb) {
        if (err) { err->code = 45; err->message = "IndigoHIDMessageForKeyboardArbitrary missing"; }
        return false;
    }
    void *md = g_kbd_arb(mod_usage, 1);
    if (md) send_message(md, client);
    usleep(8000);
    void *kd = g_kbd_arb(key_usage, 1);
    if (kd) send_message(kd, client);
    usleep(12000);
    void *ku = g_kbd_arb(key_usage, 2);
    if (ku) send_message(ku, client);
    usleep(4000);
    void *mu = g_kbd_arb(mod_usage, 2);
    if (mu) send_message(mu, client);
    usleep(8000);
    return true;
}

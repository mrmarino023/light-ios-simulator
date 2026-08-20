#import <Foundation/Foundation.h>
#import <IOSurface/IOSurface.h>
#import <objc/message.h>
#import <objc/runtime.h>
#import <dispatch/dispatch.h>
#import <string.h>

#import "display_bridge.h"
#import "frameworks.h"

static LighFrameFn g_frame_cb = NULL;
static void *g_frame_ctx = NULL;
static dispatch_queue_t g_fb_queue = NULL;
static NSMutableArray *g_descriptors = nil;
static NSMutableDictionary *g_callback_uuids = nil;
static id g_io_client = nil;

static void set_err(LighHostError *err, int code, const char *msg) {
    if (!err) return;
    err->code = code;
    err->message = msg;
}

static id invoke_obj_with_error(id target, SEL sel, NSError **err) {
    IMP imp = class_getMethodImplementation(object_getClass(target), sel);
    if (!imp) return nil;
    id (*fn)(id, SEL, NSError **) = (id (*)(id, SEL, NSError **))imp;
    return fn(target, sel, err);
}

static BOOL invoke_bool_with_error(id target, SEL sel, NSError **err) {
    IMP imp = class_getMethodImplementation(object_getClass(target), sel);
    if (!imp) return NO;
    BOOL (*fn)(id, SEL, NSError **) = (BOOL (*)(id, SEL, NSError **))imp;
    return fn(target, sel, err);
}

static BOOL invoke_bool_with_obj_error(id target, SEL sel, id arg, NSError **err) {
    IMP imp = class_getMethodImplementation(object_getClass(target), sel);
    if (!imp) return NO;
    BOOL (*fn)(id, SEL, id, NSError **) = (BOOL (*)(id, SEL, id, NSError **))imp;
    return fn(target, sel, arg, err);
}

static id shared_service_context(NSString *developer_dir, NSError **err) {
    Class cls = NSClassFromString(@"SimServiceContext");
    if (!cls) return nil;
    SEL sel = NSSelectorFromString(@"sharedServiceContextForDeveloperDir:error:");
    IMP imp = class_getMethodImplementation(object_getClass(cls), sel);
    if (!imp) return nil;
    id (*fn)(Class, SEL, id, NSError **) = (id (*)(Class, SEL, id, NSError **))imp;
    return fn(cls, sel, developer_dir, err);
}

static id default_device_set(id context, NSError **err) {
    SEL sel = NSSelectorFromString(@"defaultDeviceSetWithError:");
    if (![context respondsToSelector:sel]) return nil;
    return invoke_obj_with_error(context, sel, err);
}

static id resolve_device(NSString *udid, NSError **err) {
    NSString *dev_dir = [[NSProcessInfo processInfo].environment objectForKey:@"DEVELOPER_DIR"];
    if (!dev_dir.length) {
        dev_dir = @"/Applications/Xcode.app/Contents/Developer";
    }
    id ctx = shared_service_context(dev_dir, err);
    if (!ctx) return nil;
    id set = default_device_set(ctx, err);
    if (!set) return nil;
    NSArray *devices = [set valueForKey:@"availableDevices"];
    for (id device in devices) {
        NSUUID *dudid = [device valueForKey:@"UDID"];
        if ([dudid.UUIDString isEqualToString:udid]) {
            return device;
        }
    }
    return nil;
}

static void capture_latest(void) {
    if (!g_frame_cb || g_descriptors.count == 0) return;
    SEL surf_sel = NSSelectorFromString(@"framebufferSurface");
    IOSurfaceRef best = NULL;
    uint32_t best_area = 0;

    for (id desc in g_descriptors) {
        if (![desc respondsToSelector:surf_sel]) continue;
        #pragma clang diagnostic push
        #pragma clang diagnostic ignored "-Warc-performSelector-leaks"
        IOSurfaceRef surf = (__bridge IOSurfaceRef)[desc performSelector:surf_sel];
        #pragma clang diagnostic pop
        if (!surf) continue;
        uint32_t w = (uint32_t)IOSurfaceGetWidth(surf);
        uint32_t h = (uint32_t)IOSurfaceGetHeight(surf);
        uint32_t area = w * h;
        if (area > best_area) {
            best = surf;
            best_area = area;
        }
    }

    if (best && g_frame_cb) {
        g_frame_cb(g_frame_ctx, IOSurfaceGetID(best),
                   (uint32_t)IOSurfaceGetWidth(best),
                   (uint32_t)IOSurfaceGetHeight(best));
    }
}

static void register_callbacks_on(id desc) {
    SEL reg_sel = NSSelectorFromString(
        @"registerScreenCallbacksWithUUID:callbackQueue:frameCallback:"
        @"surfacesChangedCallback:propertiesChangedCallback:");
    if (![desc respondsToSelector:reg_sel]) return;

    NSUUID *uuid = [NSUUID UUID];
    g_callback_uuids[@((uintptr_t)desc)] = uuid;

    void (^frame_block)(void) = ^{
        dispatch_async(g_fb_queue, ^{
            capture_latest();
        });
    };
    void (^surfaces_block)(void) = ^{
        dispatch_async(g_fb_queue, ^{
            capture_latest();
        });
    };
    void (^props_block)(void) = ^(void){};

    IMP imp = class_getMethodImplementation(object_getClass(desc), reg_sel);
    if (!imp) return;
    void (*fn)(id, SEL, id, id, id, id, id) =
        (void (*)(id, SEL, id, id, id, id, id))imp;
    fn(desc, reg_sel, uuid, g_fb_queue, frame_block, surfaces_block, props_block);
}

static BOOL wire_framebuffer(id device, LighHostError *err) {
    #pragma clang diagnostic push
    #pragma clang diagnostic ignored "-Warc-performSelector-leaks"
    id io = [device performSelector:NSSelectorFromString(@"io")];
    #pragma clang diagnostic pop
    if (!io) {
        set_err(err, 2, "SimDevice.io unavailable");
        return NO;
    }
    g_io_client = io;

    SEL update_sel = NSSelectorFromString(@"updateIOPorts");
    if ([io respondsToSelector:update_sel]) {
        #pragma clang diagnostic push
        #pragma clang diagnostic ignored "-Warc-performSelector-leaks"
        [io performSelector:update_sel];
        #pragma clang diagnostic pop
    }

    NSArray *ports = [io valueForKey:@"deviceIOPorts"];
    if (!ports.count) {
        set_err(err, 3, "no device IO ports (sim booted?)");
        return NO;
    }

    SEL pid_sel = NSSelectorFromString(@"portIdentifier");
    SEL desc_sel = NSSelectorFromString(@"descriptor");
    SEL surf_sel = NSSelectorFromString(@"framebufferSurface");

    g_descriptors = [NSMutableArray array];
    for (id port in ports) {
        if (![port respondsToSelector:pid_sel]) continue;
        #pragma clang diagnostic push
        #pragma clang diagnostic ignored "-Warc-performSelector-leaks"
        id pid = [port performSelector:pid_sel];
        #pragma clang diagnostic pop
        if (![pid isEqual:@"com.apple.framebuffer.display"]) continue;
        if (![port respondsToSelector:desc_sel]) continue;
        #pragma clang diagnostic push
        #pragma clang diagnostic ignored "-Warc-performSelector-leaks"
        id desc = [port performSelector:desc_sel];
        #pragma clang diagnostic pop
        if (!desc || ![desc respondsToSelector:surf_sel]) continue;
        [g_descriptors addObject:desc];
    }

    if (g_descriptors.count == 0) {
        set_err(err, 4, "com.apple.framebuffer.display not found");
        return NO;
    }

    for (id desc in g_descriptors) {
        register_callbacks_on(desc);
    }
    capture_latest();
    return YES;
}

bool ligh_host_init(const char *developer_dir, LighHostError *err) {
    if (!ligh_load_private_frameworks(developer_dir)) {
        set_err(err, 1, "failed to load CoreSimulator/SimulatorKit");
        return false;
    }
    if (!g_fb_queue) {
        g_fb_queue = dispatch_queue_create("dev.ligh.framebuffer", DISPATCH_QUEUE_SERIAL);
    }
    if (!g_callback_uuids) {
        g_callback_uuids = [NSMutableDictionary dictionary];
    }
    return true;
}

bool ligh_host_boot(const char *udid_c, LighHostError *err) {
    NSString *udid = [NSString stringWithUTF8String:udid_c];
    NSError *ns_err = nil;
    id device = resolve_device(udid, &ns_err);
    if (!device) {
        set_err(err, 10, "SimDevice not found");
        return false;
    }

    SEL boot_opts = NSSelectorFromString(@"bootWithOptions:error:");
    if ([device respondsToSelector:boot_opts]) {
        NSDictionary *opts = @{@"persist": @YES};
        if (invoke_bool_with_obj_error(device, boot_opts, opts, &ns_err)) {
            return true;
        }
    }

    SEL boot_sel = NSSelectorFromString(@"bootWithError:");
    if ([device respondsToSelector:boot_sel]) {
        if (invoke_bool_with_error(device, boot_sel, &ns_err)) {
            return true;
        }
    }

    set_err(err, 11, "bootWithError failed");
    return false;
}

bool ligh_host_shutdown(const char *udid_c, LighHostError *err) {
    NSString *udid = [NSString stringWithUTF8String:udid_c];
    NSError *ns_err = nil;
    id device = resolve_device(udid, &ns_err);
    if (!device) {
        set_err(err, 10, "SimDevice not found");
        return false;
    }
    SEL sel = NSSelectorFromString(@"shutdownWithError:");
    if (![device respondsToSelector:sel] || !invoke_bool_with_error(device, sel, &ns_err)) {
        set_err(err, 12, "shutdownWithError failed");
        return false;
    }
    return true;
}

bool ligh_host_stream_start(const char *udid_c, LighFrameFn callback, void *ctx,
                            LighHostError *err) {
    ligh_host_stream_stop();
    g_frame_cb = callback;
    g_frame_ctx = ctx;

    NSString *udid = [NSString stringWithUTF8String:udid_c];
    NSError *ns_err = nil;
    id device = resolve_device(udid, &ns_err);
    if (!device) {
        set_err(err, 10, "SimDevice not found");
        return false;
    }

    NSNumber *state = [device valueForKey:@"state"];
    if (state.unsignedIntegerValue != 3) {
        set_err(err, 13, "simulator not booted (state != Booted)");
        return false;
    }

    return wire_framebuffer(device, err) ? true : false;
}

void ligh_host_stream_stop(void) {
    SEL unreg_sel = NSSelectorFromString(@"unregisterScreenCallbacksWithUUID:");
    for (id desc in g_descriptors) {
        NSUUID *uuid = g_callback_uuids[@((uintptr_t)desc)];
        if (uuid && [desc respondsToSelector:unreg_sel]) {
            #pragma clang diagnostic push
            #pragma clang diagnostic ignored "-Warc-performSelector-leaks"
            [desc performSelector:unreg_sel withObject:uuid];
            #pragma clang diagnostic pop
        }
    }
    [g_descriptors removeAllObjects];
    [g_callback_uuids removeAllObjects];
    g_io_client = nil;
    g_frame_cb = NULL;
    g_frame_ctx = NULL;
}

void ligh_host_stream_poll(void) {
    if (!g_fb_queue) return;
    dispatch_sync(g_fb_queue, ^{
        capture_latest();
    });
}

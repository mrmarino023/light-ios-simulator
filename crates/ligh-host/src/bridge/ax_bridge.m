#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>
#import <objc/runtime.h>
#import <objc/message.h>
#import <dispatch/dispatch.h>
#import <dlfcn.h>
#import <string.h>
#import <stdio.h>
#import <stdlib.h>

#import "display_bridge.h"
#import "frameworks.h"

/// Headless AX dump via AccessibilityPlatformTranslation + SimDevice XPC.
/// Pattern: install bridgeTokenDelegate so AXPTranslator can route without Simulator.app.

static const int kMaxDepth = 40;
static const int kMaxNodes = 800;

static void set_err(LighHostError *err, int code, const char *msg) {
    if (!err) return;
    err->code = code;
    err->message = msg;
}

static id resolve_device(NSString *udid) {
    NSString *dev_dir = [[NSProcessInfo processInfo].environment objectForKey:@"DEVELOPER_DIR"];
    if (!dev_dir.length) {
        dev_dir = @"/Applications/Xcode.app/Contents/Developer";
    }
    Class ctx_cls = NSClassFromString(@"SimServiceContext");
    if (!ctx_cls) return nil;
    SEL ctx_sel = NSSelectorFromString(@"sharedServiceContextForDeveloperDir:error:");
    IMP ctx_imp = class_getMethodImplementation(object_getClass(ctx_cls), ctx_sel);
    if (!ctx_imp) return nil;
    id (*ctx_fn)(Class, SEL, id, NSError **) = (id (*)(Class, SEL, id, NSError **))ctx_imp;
    NSError *err = nil;
    id ctx = ctx_fn(ctx_cls, ctx_sel, dev_dir, &err);
    if (!ctx) return nil;
    SEL set_sel = NSSelectorFromString(@"defaultDeviceSetWithError:");
    IMP set_imp = class_getMethodImplementation(object_getClass(ctx), set_sel);
    if (!set_imp) return nil;
    id (*set_fn)(id, SEL, NSError **) = (id (*)(id, SEL, NSError **))set_imp;
    id set = set_fn(ctx, set_sel, &err);
    if (!set) return nil;
    for (id device in [set valueForKey:@"availableDevices"]) {
        NSUUID *dudid = [device valueForKey:@"UDID"];
        if ([dudid.UUIDString isEqualToString:udid]) return device;
    }
    return nil;
}

#pragma mark - Token dispatcher

@interface LighAxTokenDispatcher : NSObject
- (void)registerDevice:(id)device token:(NSString *)token deadline:(NSDate *)deadline;
- (void)unregisterToken:(NSString *)token;
@end

@implementation LighAxTokenDispatcher {
    NSMutableDictionary *_deviceForToken;
    NSMutableDictionary *_deadlineForToken;
    NSLock *_lock;
}

- (instancetype)init {
    if ((self = [super init])) {
        _deviceForToken = [NSMutableDictionary dictionary];
        _deadlineForToken = [NSMutableDictionary dictionary];
        _lock = [[NSLock alloc] init];
    }
    return self;
}

- (void)registerDevice:(id)device token:(NSString *)token deadline:(NSDate *)deadline {
    [_lock lock];
    _deviceForToken[token] = device;
    _deadlineForToken[token] = deadline;
    [_lock unlock];
}

- (void)unregisterToken:(NSString *)token {
    [_lock lock];
    [_deviceForToken removeObjectForKey:token];
    [_deadlineForToken removeObjectForKey:token];
    [_lock unlock];
}

+ (id)emptyResponse {
    Class cls = NSClassFromString(@"AXPTranslatorResponse");
    if (cls) {
        SEL sel = NSSelectorFromString(@"emptyResponse");
        if ([cls respondsToSelector:sel]) {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Warc-performSelector-leaks"
            id resp = [cls performSelector:sel];
#pragma clang diagnostic pop
            if (resp) return resp;
        }
    }
    return [NSNull null];
}

- (id)sendRequest:(id)request toDevice:(id)device timeout:(NSTimeInterval)timeout {
    @try {
        SEL sel = NSSelectorFromString(@"sendAccessibilityRequestAsync:completionQueue:completionHandler:");
        if (![device respondsToSelector:sel]) return [LighAxTokenDispatcher emptyResponse];

        IMP imp = class_getMethodImplementation(object_getClass(device), sel);
        if (!imp) return [LighAxTokenDispatcher emptyResponse];

        __block id box = nil;
        dispatch_semaphore_t sem = dispatch_semaphore_create(0);
        dispatch_queue_t q = dispatch_queue_create("dev.ligh.ax.xpc", DISPATCH_QUEUE_SERIAL);
        void (^completion)(id) = ^(id response) {
            @try {
                box = response;
            } @catch (NSException *ex) {
                box = nil;
            }
            dispatch_semaphore_signal(sem);
        };

        void (*fn)(id, SEL, id, id, id) = (void (*)(id, SEL, id, id, id))imp;
        fn(device, sel, request, q, completion);

        long rc = dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(timeout * NSEC_PER_SEC)));
        if (rc != 0) return [LighAxTokenDispatcher emptyResponse];
        return box ?: [LighAxTokenDispatcher emptyResponse];
    } @catch (NSException *ex) {
        return [LighAxTokenDispatcher emptyResponse];
    }
}

- (id)accessibilityTranslationDelegateBridgeCallbackWithToken:(NSString *)token {
    [_lock lock];
    id device = _deviceForToken[token];
    NSDate *deadline = _deadlineForToken[token];
    [_lock unlock];

    __weak LighAxTokenDispatcher *weakSelf = self;
    id retainedDevice = device;
    NSDate *retainedDeadline = deadline ?: [NSDate distantFuture];

    id (^block)(id) = ^id(id request) {
        @try {
            LighAxTokenDispatcher *strong = weakSelf;
            if (!strong || !retainedDevice) return [LighAxTokenDispatcher emptyResponse];
            NSTimeInterval remaining = [retainedDeadline timeIntervalSinceNow];
            if (remaining <= 0) return [LighAxTokenDispatcher emptyResponse];
            NSTimeInterval timeout = MIN(remaining, 10.0);
            return [strong sendRequest:request toDevice:retainedDevice timeout:timeout];
        } @catch (NSException *ex) {
            return [LighAxTokenDispatcher emptyResponse];
        }
    };
    return [block copy];
}

- (CGRect)accessibilityTranslationConvertPlatformFrameToSystem:(CGRect)rect
                                                     withToken:(NSString *)token {
    (void)token;
    return rect;
}

- (id)accessibilityTranslationRootParentWithToken:(NSString *)token {
    (void)token;
    return nil;
}

@end

#pragma mark - Translator bootstrap

static LighAxTokenDispatcher *g_dispatcher = nil;
static id g_translator = nil;
static NSLock *g_ax_init_lock = nil;

static bool ensure_ax_translator(void) {
    static dispatch_once_t lock_once;
    dispatch_once(&lock_once, ^{
        g_ax_init_lock = [[NSLock alloc] init];
    });

    [g_ax_init_lock lock];
    if (g_translator != nil) {
        [g_ax_init_lock unlock];
        return true;
    }

    @try {
        void *h = dlopen(
            "/System/Library/PrivateFrameworks/AccessibilityPlatformTranslation.framework/"
            "AccessibilityPlatformTranslation",
            RTLD_NOW | RTLD_GLOBAL);
        if (!h) {
            [g_ax_init_lock unlock];
            return false;
        }
        Class cls = NSClassFromString(@"AXPTranslator");
        if (!cls) {
            [g_ax_init_lock unlock];
            return false;
        }
        SEL sel = NSSelectorFromString(@"sharedInstance");
        IMP imp = class_getMethodImplementation(object_getClass(cls), sel);
        if (!imp) {
            [g_ax_init_lock unlock];
            return false;
        }
        id (*fn)(Class, SEL) = (id (*)(Class, SEL))imp;
        id inst = fn(cls, sel);
        if (!inst) {
            [g_ax_init_lock unlock];
            return false;
        }
        if (!g_dispatcher) {
            g_dispatcher = [[LighAxTokenDispatcher alloc] init];
        }
        SEL setSel = NSSelectorFromString(@"setBridgeTokenDelegate:");
        if ([inst respondsToSelector:setSel]) {
            void (*setFn)(id, SEL, id) =
                (void (*)(id, SEL, id))class_getMethodImplementation(object_getClass(inst), setSel);
            if (setFn) setFn(inst, setSel, g_dispatcher);
        } else {
            @try {
                [inst setValue:g_dispatcher forKey:@"bridgeTokenDelegate"];
            } @catch (NSException *ex) {
                [g_ax_init_lock unlock];
                return false;
            }
        }
        g_translator = inst;
        [g_ax_init_lock unlock];
        return true;
    } @catch (NSException *ex) {
        [g_ax_init_lock unlock];
        return false;
    }
}

#pragma mark - Element helpers

static NSString *ax_string(id obj, NSString *key) {
    @try {
        id v = [obj valueForKey:key];
        if ([v isKindOfClass:[NSString class]] && [(NSString *)v length] > 0) return v;
        if ([v isKindOfClass:[NSNumber class]]) return [(NSNumber *)v stringValue];
    } @catch (NSException *ex) {
        return nil;
    }
    return nil;
}

static BOOL ax_bool(id obj, NSString *key, BOOL fallback) {
    @try {
        id v = [obj valueForKey:key];
        if ([v isKindOfClass:[NSNumber class]]) return [(NSNumber *)v boolValue];
    } @catch (NSException *ex) {
        return fallback;
    }
    return fallback;
}

static CGRect ax_frame(id element) {
    @try {
        id v = [element valueForKey:@"accessibilityFrame"];
        if ([v isKindOfClass:[NSValue class]]) {
            return [(NSValue *)v rectValue];
        }
    } @catch (NSException *ex) {
        // fall through
    }
    SEL sel = NSSelectorFromString(@"accessibilityFrame");
    if (![element respondsToSelector:sel]) return CGRectZero;
    NSMethodSignature *sig = [element methodSignatureForSelector:sel];
    if (!sig) return CGRectZero;
    NSInvocation *inv = [NSInvocation invocationWithMethodSignature:sig];
    [inv setSelector:sel];
    [inv setTarget:element];
    [inv invoke];
    CGRect fr = CGRectZero;
    [inv getReturnValue:&fr];
    return fr;
}

static NSArray *ax_children(id element) {
    @try {
        id raw = [element valueForKey:@"accessibilityChildren"];
        if ([raw isKindOfClass:[NSArray class]]) return raw;
    } @catch (NSException *ex) {
        return @[];
    }
    return @[];
}

static void stamp_token(NSString *token, id translation) {
    if (!translation) return;
    @try {
        [translation setValue:token forKey:@"bridgeDelegateToken"];
    } @catch (NSException *ex) {
        // ignore — some translation objects reject KVC
    }
}

static void stamp_element(NSString *token, id element) {
    @try {
        id trans = [element valueForKey:@"translation"];
        if (trans) stamp_token(token, trans);
    } @catch (NSException *ex) {
    }
}

static void stamp_subtree(NSString *token, id element, int depth) {
    if (depth >= kMaxDepth) return;
    stamp_element(token, element);
    for (id kid in ax_children(element)) {
        stamp_subtree(token, kid, depth + 1);
    }
}

static CGSize device_point_size(id device) {
    CGSize fallback = CGSizeMake(393, 852);
    id deviceType = [device valueForKey:@"deviceType"];
    if (!deviceType) return fallback;
    id raw = [deviceType valueForKey:@"mainScreenSize"];
    CGSize pixel = fallback;
    if (raw) {
        if ([raw isKindOfClass:[NSValue class]]) {
            pixel = [(NSValue *)raw sizeValue];
        }
    }
    double scale = [[deviceType valueForKey:@"mainScreenScale"] doubleValue];
    if (scale <= 0) scale = 3.0;
    return CGSizeMake(pixel.width / scale, pixel.height / scale);
}

static NSDictionary *project_frame(CGRect mac, CGRect rootMac, CGSize pointSize) {
    if (rootMac.size.width < 1 || rootMac.size.height < 1) {
        return @{
            @"x": @(mac.origin.x),
            @"y": @(mac.origin.y),
            @"width": @(mac.size.width),
            @"height": @(mac.size.height),
        };
    }
    double scale = pointSize.width / rootMac.size.width;
    double yOffset = (pointSize.height - rootMac.size.height * scale) / 2.0;
    double x = (mac.origin.x - rootMac.origin.x) * scale;
    double y = (mac.origin.y - rootMac.origin.y) * scale + yOffset;
    double w = mac.size.width * scale;
    double h = mac.size.height * scale;
    return @{
        @"x": @(x),
        @"y": @(y),
        @"width": @(w),
        @"height": @(h),
    };
}

static int g_node_count = 0;

static NSDictionary *walk_element(id element, CGRect rootMac, CGSize pointSize, int depth) {
    if (depth >= kMaxDepth || g_node_count >= kMaxNodes) return nil;
    g_node_count++;

    NSString *role = ax_string(element, @"accessibilityRole");
    NSString *label = ax_string(element, @"accessibilityLabel");
    NSString *value = ax_string(element, @"accessibilityValue");
    NSString *ident = ax_string(element, @"accessibilityIdentifier");
    NSString *title = ax_string(element, @"accessibilityTitle");
    if (!role) role = ax_string(element, @"role");
    if (!label) label = ax_string(element, @"label");
    if (!ident) ident = ax_string(element, @"identifier");
    if (!title) title = ax_string(element, @"title");

    CGRect fr = ax_frame(element);
    NSMutableDictionary *node = [NSMutableDictionary dictionary];
    if (role) node[@"role"] = role;
    if (label) node[@"label"] = label;
    if (value) node[@"value"] = value;
    if (ident) node[@"identifier"] = ident;
    if (title) node[@"title"] = title;
    node[@"enabled"] = @(ax_bool(element, @"enabled", YES));
    node[@"frame"] = project_frame(fr, rootMac, pointSize);

    NSMutableArray *kids = [NSMutableArray array];
    for (id kid in ax_children(element)) {
        NSDictionary *child = walk_element(kid, rootMac, pointSize, depth + 1);
        if (child) [kids addObject:child];
    }
    if (kids.count) node[@"children"] = kids;
    return node;
}

static void flatten_interactive(NSDictionary *node, NSMutableArray *out) {
    if (!node) return;
    NSString *label = node[@"label"];
    NSString *ident = node[@"identifier"];
    NSString *role = node[@"role"] ?: @"";
    BOOL is_field = [role.lowercaseString containsString:@"textfield"]
                    || [role.lowercaseString containsString:@"searchfield"]
                    || [role.lowercaseString containsString:@"textarea"];
    BOOL interesting = (label.length > 0) || (ident.length > 0) || is_field;
    if (interesting) {
        NSMutableDictionary *flat = [@{
            @"role": role,
            @"frame": node[@"frame"] ?: @{},
        } mutableCopy];
        if (label) flat[@"label"] = label;
        if (ident) flat[@"identifier"] = ident;
        if (node[@"value"]) flat[@"value"] = node[@"value"];
        [out addObject:flat];
    }
    for (NSDictionary *c in node[@"children"] ?: @[]) {
        flatten_interactive(c, out);
    }
}

static char g_ax_err_buf[256];

static char *ax_dump_inner(const char *udid_c, LighHostError *err);

char *ligh_host_ax_dump(const char *udid_c, LighHostError *err) {
    @try {
        return ax_dump_inner(udid_c, err);
    } @catch (NSException *ex) {
        snprintf(g_ax_err_buf, sizeof(g_ax_err_buf), "AX exception: %s",
                 ex.reason ? ex.reason.UTF8String : "unknown");
        set_err(err, 59, g_ax_err_buf);
        return NULL;
    }
}

static char *ax_dump_inner(const char *udid_c, LighHostError *err) {
    if (!ligh_load_private_frameworks(NULL)) {
        set_err(err, 50, "frameworks not loaded");
        return NULL;
    }
    if (!ensure_ax_translator()) {
        set_err(err, 51, "AXPTranslator unavailable");
        return NULL;
    }

    NSString *udid = [NSString stringWithUTF8String:udid_c];
    id device = resolve_device(udid);
    if (!device) {
        set_err(err, 52, "SimDevice not found");
        return NULL;
    }

    NSString *token = [[NSUUID UUID] UUIDString];
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:5.0];
    [g_dispatcher registerDevice:device token:token deadline:deadline];

    id translation = nil;
    @try {
        SEL sel = NSSelectorFromString(@"frontmostApplicationWithDisplayId:bridgeDelegateToken:");
        if (![g_translator respondsToSelector:sel]) {
            set_err(err, 53, "frontmostApplication selector missing");
            [g_dispatcher unregisterToken:token];
            return NULL;
        }
        IMP imp = class_getMethodImplementation(object_getClass(g_translator), sel);
        id (*fn)(id, SEL, uint32_t, id) = (id (*)(id, SEL, uint32_t, id))imp;
        translation = fn(g_translator, sel, 0, token);
        if (translation) stamp_token(token, translation);
    } @catch (NSException *ex) {
        set_err(err, 54, "frontmostApplication threw");
        [g_dispatcher unregisterToken:token];
        return NULL;
    }

    if (!translation) {
        // No frontmost app (e.g. mid-boot). Return empty available tree.
        [g_dispatcher unregisterToken:token];
        NSDictionary *payload = @{
            @"status": @"empty",
            @"root": [NSNull null],
            @"elements": @[],
            @"element_count": @0,
        };
        NSData *empty = [NSJSONSerialization dataWithJSONObject:payload options:0 error:nil];
        NSString *s = [[NSString alloc] initWithData:empty encoding:NSUTF8StringEncoding];
        return strdup(s.UTF8String);
    }

    id rootElement = nil;
    @try {
        SEL sel = NSSelectorFromString(@"macPlatformElementFromTranslation:");
        IMP imp = class_getMethodImplementation(object_getClass(g_translator), sel);
        id (*fn)(id, SEL, id) = (id (*)(id, SEL, id))imp;
        rootElement = fn(g_translator, sel, translation);
        if (rootElement) {
            stamp_element(token, rootElement);
            stamp_subtree(token, rootElement, 0);
        }
    } @catch (NSException *ex) {
        set_err(err, 55, "macPlatformElement threw");
        [g_dispatcher unregisterToken:token];
        return NULL;
    }

    if (!rootElement) {
        set_err(err, 56, "no mac platform element");
        [g_dispatcher unregisterToken:token];
        return NULL;
    }

    CGSize pointSize = device_point_size(device);
    CGRect rootMac = ax_frame(rootElement);
    g_node_count = 0;
    NSDictionary *root = walk_element(rootElement, rootMac, pointSize, 0);
    NSMutableArray *elements = [NSMutableArray array];
    flatten_interactive(root, elements);

    [g_dispatcher unregisterToken:token];

    NSDictionary *payload = @{
        @"status": @"available",
        @"root": root ?: [NSNull null],
        @"elements": elements,
        @"element_count": @(elements.count),
        @"point_size": @{ @"width": @(pointSize.width), @"height": @(pointSize.height) },
    };
    NSError *jsonErr = nil;
    NSData *data = [NSJSONSerialization dataWithJSONObject:payload options:0 error:&jsonErr];
    if (!data) {
        set_err(err, 57, "JSON serialize failed");
        return NULL;
    }
    NSString *s = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    return strdup(s.UTF8String);
}

void ligh_host_ax_free(char *ptr) {
    if (ptr) free(ptr);
}

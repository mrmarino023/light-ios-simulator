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

static NSArray *ax_container_indexed(id element) {
    NSInteger count = 0;
    @try {
        SEL sel = NSSelectorFromString(@"accessibilityElementCount");
        if ([element respondsToSelector:sel]) {
            NSMethodSignature *sig = [element methodSignatureForSelector:sel];
            if (sig) {
                NSInvocation *inv = [NSInvocation invocationWithMethodSignature:sig];
                [inv setSelector:sel];
                [inv setTarget:element];
                [inv invoke];
                [inv getReturnValue:&count];
            }
        }
    } @catch (NSException *ex) {
        return @[];
    }
    if (count <= 0 || count > 64) return @[];
    NSMutableArray *kids = [NSMutableArray arrayWithCapacity:(NSUInteger)count];
    SEL at = NSSelectorFromString(@"accessibilityElementAtIndex:");
    if (![element respondsToSelector:at]) return @[];
    for (NSInteger i = 0; i < count; i++) {
        @try {
            NSMethodSignature *sig = [element methodSignatureForSelector:at];
            if (!sig) break;
            NSInvocation *inv = [NSInvocation invocationWithMethodSignature:sig];
            [inv setSelector:at];
            [inv setTarget:element];
            [inv setArgument:&i atIndex:2];
            [inv invoke];
            __unsafe_unretained id kid = nil;
            [inv getReturnValue:&kid];
            if (kid) [kids addObject:kid];
        } @catch (NSException *ex) {
        }
    }
    return kids;
}

static NSArray *ax_children(id element) {
    NSArray *keys = @[
        @"accessibilityChildren",
        @"accessibilityElements",
        @"children",
        @"accessibilityVisibleChildren",
        @"accessibilityTabs",
    ];
    for (NSString *key in keys) {
        @try {
            id raw = [element valueForKey:key];
            if ([raw isKindOfClass:[NSArray class]] && [raw count] > 0) {
                return raw;
            }
        } @catch (NSException *ex) {
        }
    }
    return ax_container_indexed(element);
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

/// Stable id from ancestry path (djb2 → `n` + 8 hex).
static NSString *stable_node_id(NSString *path) {
    unsigned long hash = 5381;
    const char *s = path.UTF8String ?: "";
    int c;
    while ((c = *s++)) hash = ((hash << 5) + hash) + (unsigned char)c;
    return [NSString stringWithFormat:@"n%08lx", hash & 0xffffffffUL];
}

static NSString *ax_traits_hint(id element, NSString *role) {
    NSMutableArray *bits = [NSMutableArray array];
    NSString *r = (role ?: @"").lowercaseString;
    if ([r containsString:@"button"]) [bits addObject:@"button"];
    if ([r containsString:@"link"]) [bits addObject:@"link"];
    if ([r containsString:@"search"]) [bits addObject:@"search"];
    if ([r containsString:@"textfield"] || [r containsString:@"textarea"]) [bits addObject:@"editable"];
    if ([r containsString:@"switch"]) [bits addObject:@"switch"];
    if ([r containsString:@"slider"]) [bits addObject:@"slider"];
    if ([r containsString:@"cell"]) [bits addObject:@"cell"];
    if ([r containsString:@"heading"]) [bits addObject:@"heading"];
    if ([r containsString:@"keyboard"]) [bits addObject:@"keyboard"];
    if ([r containsString:@"tabbar"] || [r containsString:@"tab bar"] || [r containsString:@"tabbutton"]) {
        [bits addObject:@"tabbar"];
    }
    if ([r containsString:@"alert"] || [r containsString:@"sheet"]) [bits addObject:@"dialog"];
    @try {
        id raw = [element valueForKey:@"accessibilityTraits"];
        if ([raw respondsToSelector:@selector(unsignedLongLongValue)]) {
            unsigned long long t = [raw unsignedLongLongValue];
            // UIAccessibilityTraitSelected / Button / Link / TabBar
            if (t & (1ULL << 1)) [bits addObject:@"selected"];
            if (t & (1ULL << 0)) [bits addObject:@"button"];
            if (t & (1ULL << 2)) [bits addObject:@"link"];
            if (t & (1ULL << 28)) [bits addObject:@"tabbar"];
        }
    } @catch (NSException *ex) {
    }
    if (!bits.count) return nil;
    return [bits componentsJoinedByString:@","];
}

static NSDictionary *walk_element(id element, CGRect rootMac, CGSize pointSize, int depth,
                                  NSString *parentPath, NSString *parentId, int index,
                                  BOOL recurse) {
    if (depth >= kMaxDepth || g_node_count >= kMaxNodes) return nil;
    g_node_count++;

    NSString *role = ax_string(element, @"accessibilityRole");
    NSString *label = ax_string(element, @"accessibilityLabel");
    NSString *value = ax_string(element, @"accessibilityValue");
    NSString *ident = ax_string(element, @"accessibilityIdentifier");
    NSString *title = ax_string(element, @"accessibilityTitle");
    NSString *placeholder = ax_string(element, @"placeholderValue");
    if (!placeholder) placeholder = ax_string(element, @"accessibilityPlaceholderValue");
    if (!role) role = ax_string(element, @"role");
    if (!label) label = ax_string(element, @"label");
    if (!ident) ident = ax_string(element, @"identifier");
    if (!title) title = ax_string(element, @"title");

    NSString *pathSeg = [NSString stringWithFormat:@"%@|%@|%@|%d",
                         role ?: @"?", label ?: @"", ident ?: @"", index];
    NSString *path = parentPath.length
        ? [NSString stringWithFormat:@"%@/%@", parentPath, pathSeg]
        : pathSeg;
    // Path id (tree position) — can churn across transitions.
    NSString *pathId = stable_node_id(path);
    // Semantic id — stable across AX refreshes: identifier, else role+label+coarse center.
    CGRect frEarly = ax_frame(element);
    NSDictionary *frameEarly = project_frame(frEarly, rootMac, pointSize);
    double cx = ([frameEarly[@"x"] doubleValue] + [frameEarly[@"width"] doubleValue] * 0.5);
    double cy = ([frameEarly[@"y"] doubleValue] + [frameEarly[@"height"] doubleValue] * 0.5);
    NSString *semKey = nil;
    if (ident.length > 0) {
        semKey = [NSString stringWithFormat:@"i:%@", ident];
    } else if (label.length > 0 || role.length > 0) {
        // Quantize to ~10% screen so tiny layout jitter doesn't flip id.
        int qx = (int)floor((cx / MAX(pointSize.width, 1.0)) * 10.0);
        int qy = (int)floor((cy / MAX(pointSize.height, 1.0)) * 10.0);
        semKey = [NSString stringWithFormat:@"l:%@|%@|%d|%d", role ?: @"?", label ?: @"", qx, qy];
    } else {
        semKey = path;
    }
    NSString *nid = stable_node_id(semKey);

    CGRect fr = frEarly;
    NSDictionary *frame = frameEarly;
    double fx = [frame[@"x"] doubleValue];
    double fy = [frame[@"y"] doubleValue];
    double fw = [frame[@"width"] doubleValue];
    double fh = [frame[@"height"] doubleValue];
    BOOL enabled = ax_bool(element, @"enabled", YES);
    BOOL focused = ax_bool(element, @"focused", NO)
                   || ax_bool(element, @"accessibilityFocused", NO);
    BOOL selected = ax_bool(element, @"selected", NO);
    BOOL on_screen = fw > 1.0 && fh > 1.0
                     && fx + fw > 0 && fy + fh > 0
                     && fx < pointSize.width && fy < pointSize.height;
    BOOL hittable = enabled && on_screen;

    NSMutableDictionary *node = [NSMutableDictionary dictionary];
    node[@"id"] = nid;
    node[@"path_id"] = pathId;
    if (parentId.length) node[@"parent_id"] = parentId;
    if (role) node[@"role"] = role;
    if (label) {
        node[@"label"] = label;
        node[@"text"] = label;
    }
    if (value) node[@"value"] = value;
    if (ident) node[@"identifier"] = ident;
    if (title) node[@"title"] = title;
    if (placeholder) node[@"placeholder"] = placeholder;
    NSString *traits = ax_traits_hint(element, role);
    if (traits) node[@"traits"] = traits;
    node[@"enabled"] = @(enabled);
    node[@"focused"] = @(focused);
    node[@"selected"] = @(selected);
    node[@"visible"] = @(on_screen);
    node[@"hittable"] = @(hittable);
    node[@"frame"] = frame;
    if (pointSize.width > 0 && pointSize.height > 0) {
        node[@"center_norm"] = @{
            @"x": @(((fx + fw * 0.5) / pointSize.width)),
            @"y": @(((fy + fh * 0.5) / pointSize.height)),
        };
    }

    NSMutableArray *kids = [NSMutableArray array];
    NSMutableArray *childIds = [NSMutableArray array];
    if (recurse) {
        NSArray *rawKids = ax_children(element);
        for (NSUInteger i = 0; i < rawKids.count; i++) {
            NSDictionary *child = walk_element(rawKids[i], rootMac, pointSize, depth + 1,
                                               path, nid, (int)i, YES);
            if (child) {
                [kids addObject:child];
                if (child[@"id"]) [childIds addObject:child[@"id"]];
            }
        }
    }
    if (kids.count) node[@"children"] = kids;
    if (childIds.count) node[@"children_ids"] = childIds;
    return node;
}

static BOOL ax_is_tab_bar(NSString *role, NSString *label, NSString *traits) {
    NSString *r = (role ?: @"").lowercaseString;
    NSString *l = (label ?: @"").lowercaseString;
    NSString *t = (traits ?: @"").lowercaseString;
    if ([t containsString:@"tabbar"]) return YES;
    if ([r containsString:@"tabbar"] || [r containsString:@"tab bar"]) return YES;
    if ([l isEqualToString:@"tab bar"] || [l containsString:@"tabbar"]) return YES;
    return NO;
}

static void flatten_interactive(NSDictionary *node, NSMutableArray *out, BOOL under_tab_bar) {
    if (!node) return;
    NSString *label = node[@"label"];
    NSString *ident = node[@"identifier"];
    NSString *role = node[@"role"] ?: @"";
    NSString *traits = node[@"traits"] ?: @"";
    BOOL is_field = [role.lowercaseString containsString:@"textfield"]
                    || [role.lowercaseString containsString:@"searchfield"]
                    || [role.lowercaseString containsString:@"textarea"];
    BOOL is_tab_bar = ax_is_tab_bar(role, label, traits);
    BOOL tab_item = under_tab_bar && !is_tab_bar;
    BOOL interesting = (label.length > 0) || (ident.length > 0) || is_field
                       || [node[@"focused"] boolValue] || is_tab_bar || tab_item;
    if (interesting) {
        NSMutableDictionary *flat = [NSMutableDictionary dictionary];
        NSString *flat_role = role;
        if (tab_item && role.length == 0) {
            flat_role = @"AXTabButton";
        } else if (tab_item && ![role.lowercaseString containsString:@"button"]
                   && ![role.lowercaseString containsString:@"tab"]) {
            flat_role = @"AXTabButton";
        }
        flat[@"role"] = flat_role;
        flat[@"frame"] = node[@"frame"] ?: @{};
        if (node[@"id"]) flat[@"id"] = node[@"id"];
        if (node[@"path_id"]) flat[@"path_id"] = node[@"path_id"];
        if (node[@"parent_id"]) flat[@"parent_id"] = node[@"parent_id"];
        if (label) {
            flat[@"label"] = label;
            flat[@"text"] = label;
        } else if (tab_item && ident.length) {
            flat[@"label"] = ident;
            flat[@"text"] = ident;
        }
        if (ident) flat[@"identifier"] = ident;
        if (node[@"value"]) flat[@"value"] = node[@"value"];
        if (node[@"placeholder"]) flat[@"placeholder"] = node[@"placeholder"];
        NSString *flat_traits = traits;
        if (is_tab_bar || tab_item) {
            if (flat_traits.length) {
                if (![flat_traits.lowercaseString containsString:@"tabbar"]) {
                    flat_traits = [flat_traits stringByAppendingString:@",tabbar"];
                }
            } else {
                flat_traits = @"tabbar";
            }
        }
        if (flat_traits.length) flat[@"traits"] = flat_traits;
        if (node[@"center_norm"]) flat[@"center_norm"] = node[@"center_norm"];
        flat[@"enabled"] = node[@"enabled"] ?: @YES;
        flat[@"focused"] = node[@"focused"] ?: @NO;
        flat[@"selected"] = node[@"selected"] ?: @NO;
        flat[@"visible"] = node[@"visible"] ?: @YES;
        flat[@"hittable"] = node[@"hittable"] ?: @YES;
        [out addObject:flat];
    }
    BOOL child_under_tab = under_tab_bar || is_tab_bar;
    for (NSDictionary *c in node[@"children"] ?: @[]) {
        flatten_interactive(c, out, child_under_tab);
    }
}

static CGPoint ax_unmap_point(CGPoint device, CGRect rootMac, CGSize pointSize) {
    if (rootMac.size.width < 1 || rootMac.size.height < 1 || pointSize.width < 1) {
        return device;
    }
    double scale = pointSize.width / rootMac.size.width;
    double yOffset = (pointSize.height - rootMac.size.height * scale) / 2.0;
    return CGPointMake(device.x / scale + rootMac.origin.x,
                       (device.y - yOffset) / scale + rootMac.origin.y);
}

static id ax_object_at_point(CGPoint hostPoint, NSString *token) {
    if (!g_translator || !token.length) return nil;
    SEL sel = NSSelectorFromString(@"objectAtPoint:displayId:bridgeDelegateToken:");
    if (![g_translator respondsToSelector:sel]) return nil;
    @try {
        IMP imp = class_getMethodImplementation(object_getClass(g_translator), sel);
        if (!imp) return nil;
        id (*fn)(id, SEL, CGPoint, uint32_t, id) = (id (*)(id, SEL, CGPoint, uint32_t, id))imp;
        return fn(g_translator, sel, hostPoint, 0, token);
    } @catch (NSException *ex) {
        return nil;
    }
}

static id ax_mac_element_from_translation(id translation) {
    if (!g_translator || !translation) return nil;
    @try {
        SEL sel = NSSelectorFromString(@"macPlatformElementFromTranslation:");
        if (![g_translator respondsToSelector:sel]) return nil;
        IMP imp = class_getMethodImplementation(object_getClass(g_translator), sel);
        if (!imp) return nil;
        id (*fn)(id, SEL, id) = (id (*)(id, SEL, id))imp;
        return fn(g_translator, sel, translation);
    } @catch (NSException *ex) {
        return nil;
    }
}

static BOOL ax_is_chrome_container(NSString *role, NSString *label, NSString *traits) {
    if (ax_is_tab_bar(role, label, traits)) return YES;
    NSString *r = (role ?: @"").lowercaseString;
    NSString *l = (label ?: @"").lowercaseString;
    if ([l containsString:@"toolbar"] || [r containsString:@"toolbar"]) return YES;
    if ([l containsString:@"navigation bar"] || [l isEqualToString:@"nav bar"]
        || [r containsString:@"navbar"] || [r containsString:@"navigationbar"]) {
        return YES;
    }
    return NO;
}

/// SwiftUI tab/nav/tool bars often walk as childless AXGroups. Server-side
/// `objectAtPoint` still hits the real buttons (idb / baguette).
static void recover_childless_chrome(NSMutableDictionary *node, NSString *token,
                                     CGRect rootMac, CGSize pointSize) {
    if (!node) return;
    NSArray *existing = node[@"children"];
    for (id child in existing ?: @[]) {
        if ([child isKindOfClass:[NSMutableDictionary class]]) {
            recover_childless_chrome(child, token, rootMac, pointSize);
        }
    }
    NSString *role = node[@"role"];
    NSString *label = node[@"label"];
    NSString *traits = node[@"traits"];
    if (!ax_is_chrome_container(role, label, traits)) return;
    if ([existing count] > 0) return;

    NSDictionary *frame = node[@"frame"];
    double x = [frame[@"x"] doubleValue];
    double y = [frame[@"y"] doubleValue];
    double w = [frame[@"width"] doubleValue];
    double h = [frame[@"height"] doubleValue];
    if (w < 16.0 || h < 8.0) return;

    NSString *containerId = node[@"id"];
    double screenArea = MAX(pointSize.width * pointSize.height, 1.0);
    int samples = 5;
    double py = y + MIN(h * 0.38, 28.0);
    NSMutableArray *kids = [NSMutableArray array];
    NSMutableArray *childIds = [NSMutableArray array];
    NSMutableSet *seen = [NSMutableSet set];

    for (int i = 0; i < samples; i++) {
        double px = x + w * ((i + 0.5) / (double)samples);
        CGPoint host = ax_unmap_point(CGPointMake(px, py), rootMac, pointSize);
        id translation = ax_object_at_point(host, token);
        if (!translation) continue;
        stamp_token(token, translation);
        id hitElement = ax_mac_element_from_translation(translation);
        if (!hitElement) continue;
        stamp_element(token, hitElement);

        NSDictionary *hit = walk_element(hitElement, rootMac, pointSize, 1,
                                         @"chrome-hit", containerId, (int)kids.count, NO);
        if (!hit) continue;
        NSString *hid = hit[@"id"];
        NSString *hident = hit[@"identifier"];
        NSString *hlabel = hit[@"label"];
        if (containerId.length && [hid isEqualToString:containerId]) continue;
        if (hlabel.length && label.length && [hlabel isEqualToString:label]) continue;
        NSString *hrole = [hit[@"role"] lowercaseString] ?: @"";
        if ([hrole containsString:@"application"] || [hrole containsString:@"window"]) continue;

        NSDictionary *hf = hit[@"frame"];
        double hx = [hf[@"x"] doubleValue];
        double hy = [hf[@"y"] doubleValue];
        double hw = [hf[@"width"] doubleValue];
        double hh = [hf[@"height"] doubleValue];
        double hArea = hw * hh;
        if (hArea > screenArea * 0.45) continue;
        if (hw > w * 0.85 && hh > h * 0.85) continue;
        double cx = hx + hw * 0.5;
        double cy = hy + hh * 0.5;
        if (cx < x - 8 || cx > x + w + 8 || cy < y - 12 || cy > y + h + 12) continue;

        NSString *dedupe = hident.length ? hident : (hid.length ? hid : hlabel);
        if (!dedupe.length) {
            dedupe = [NSString stringWithFormat:@"%.0f,%.0f", cx, cy];
        }
        if ([seen containsObject:dedupe]) continue;
        [seen addObject:dedupe];
        [kids addObject:hit];
        if (hid.length) [childIds addObject:hid];
    }

    if (kids.count) {
        node[@"children"] = kids;
        if (childIds.count) node[@"children_ids"] = childIds;
    }
}

static char g_ax_err_buf[256];

/// Classification catalog for known UIServices (assist launchctl scan).
/// Discovery is hit-test-first; these names only classify + fill gaps.
static NSArray<NSString *> *ax_system_surface_markers(void) {
    return @[
        // auth
        @"SafariViewService",
        @"AuthenticationServicesUI",
        @"AuthKitUI",
        @"AppSSOUI",
        // share / activity
        @"ShareSheetUI",
        @"UIActivityViewController",
        // permission / privacy prompts
        @"PrivacyUIService",
        @"SpringBoardPrivacy",
        @"UserNotificationsUI",
    ];
}

static NSString *ax_role_for_process_hint(NSString *hint) {
    NSString *s = (hint ?: @"").lowercaseString;
    if ([s containsString:@"safariview"] || [s containsString:@"authentication"]
        || [s containsString:@"authkit"] || [s containsString:@"appsso"]) {
        return @"auth";
    }
    if ([s containsString:@"share"] || [s containsString:@"activity"]) {
        return @"share";
    }
    if ([s containsString:@"privacy"] || [s containsString:@"permission"]
        || [s containsString:@"usernotifications"]) {
        return @"permission";
    }
    if ([s containsString:@"springboard"]) {
        return @"springboard_transient";
    }
    return @"other";
}

static NSString *ax_bundle_for_marker(NSString *marker) {
    if ([marker isEqualToString:@"SafariViewService"]) {
        return @"com.apple.SafariViewService";
    }
    if ([marker isEqualToString:@"AuthenticationServicesUI"]) {
        return @"com.apple.AuthenticationServices.AuthenticationServicesUI";
    }
    if ([marker isEqualToString:@"AuthKitUI"]) {
        return @"com.apple.AuthKitUI";
    }
    return marker.length ? marker : @"system_surface";
}

static int ax_pid_from_obj(id obj) {
    if (!obj) return 0;
    @try {
        id pidVal = nil;
        if ([obj respondsToSelector:NSSelectorFromString(@"pid")]) {
            pidVal = [obj valueForKey:@"pid"];
        } else if ([obj respondsToSelector:NSSelectorFromString(@"processIdentifier")]) {
            pidVal = [obj valueForKey:@"processIdentifier"];
        }
        return [pidVal intValue];
    } @catch (NSException *ex) {
        return 0;
    }
}

static NSArray<NSNumber *> *ax_guest_pids_matching(NSString *udid, NSArray<NSString *> *markers) {
    if (!udid.length || !markers.count) return @[];
    @try {
        NSTask *task = [[NSTask alloc] init];
        task.executableURL = [NSURL fileURLWithPath:@"/usr/bin/xcrun"];
        task.arguments = @[ @"simctl", @"spawn", udid, @"launchctl", @"list" ];
        NSPipe *outPipe = [NSPipe pipe];
        task.standardOutput = outPipe;
        task.standardError = [NSFileHandle fileHandleWithNullDevice];
        NSError *err = nil;
        if (![task launchAndReturnError:&err]) return @[];
        NSData *data = [outPipe.fileHandleForReading readDataToEndOfFile];
        [task waitUntilExit];
        if (task.terminationStatus != 0 || !data.length) return @[];
        NSString *text = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
        if (!text.length) return @[];
        NSMutableArray<NSNumber *> *pids = [NSMutableArray array];
        NSMutableSet *seen = [NSMutableSet set];
        for (NSString *line in [text componentsSeparatedByCharactersInSet:
                                    [NSCharacterSet newlineCharacterSet]]) {
            BOOL hit = NO;
            for (NSString *m in markers) {
                if ([line containsString:m]) {
                    hit = YES;
                    break;
                }
            }
            if (!hit) continue;
            NSArray *parts = [line componentsSeparatedByCharactersInSet:
                                  [NSCharacterSet whitespaceCharacterSet]];
            NSMutableArray *toks = [NSMutableArray array];
            for (NSString *p in parts) {
                if (p.length) [toks addObject:p];
            }
            if (toks.count < 1) continue;
            int pid = [toks[0] intValue];
            if (pid <= 0) continue;
            NSNumber *n = @(pid);
            if ([seen containsObject:n]) continue;
            [seen addObject:n];
            [pids addObject:n];
        }
        return pids;
    } @catch (NSException *ex) {
        return @[];
    }
}

static BOOL ax_first_catalog_pid(NSString *udid, int *outPid, NSString **outMarker) {
    if (outPid) *outPid = 0;
    if (outMarker) *outMarker = nil;
    @try {
        NSTask *task = [[NSTask alloc] init];
        task.executableURL = [NSURL fileURLWithPath:@"/usr/bin/xcrun"];
        task.arguments = @[ @"simctl", @"spawn", udid, @"launchctl", @"list" ];
        NSPipe *outPipe = [NSPipe pipe];
        task.standardOutput = outPipe;
        task.standardError = [NSFileHandle fileHandleWithNullDevice];
        if (![task launchAndReturnError:nil]) return NO;
        NSData *data = [outPipe.fileHandleForReading readDataToEndOfFile];
        [task waitUntilExit];
        if (task.terminationStatus != 0 || !data.length) return NO;
        NSString *text = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
        for (NSString *line in [text componentsSeparatedByCharactersInSet:
                                    [NSCharacterSet newlineCharacterSet]]) {
            NSString *hitMarker = nil;
            for (NSString *m in ax_system_surface_markers()) {
                if ([line containsString:m]) {
                    hitMarker = m;
                    break;
                }
            }
            if (!hitMarker) continue;
            NSArray *parts = [line componentsSeparatedByCharactersInSet:
                                  [NSCharacterSet whitespaceCharacterSet]];
            NSMutableArray *toks = [NSMutableArray array];
            for (NSString *p in parts) {
                if (p.length) [toks addObject:p];
            }
            if (toks.count < 1) continue;
            int pid = [toks[0] intValue];
            if (pid <= 0) continue;
            if (outPid) *outPid = pid;
            if (outMarker) *outMarker = hitMarker;
            return YES;
        }
        return NO;
    } @catch (NSException *ex) {
        return NO;
    }
}

static id ax_translation_for_pid(int pid, NSString *token) {
    if (!g_translator || pid <= 0 || !token.length) return nil;
    @try {
        SEL selNum = NSSelectorFromString(@"_translationApplicationObjectForPidNumber:");
        if ([g_translator respondsToSelector:selNum]) {
            IMP imp = class_getMethodImplementation(object_getClass(g_translator), selNum);
            if (imp) {
                id (*fn)(id, SEL, id) = (id (*)(id, SEL, id))imp;
                id translation = fn(g_translator, selNum, @(pid));
                if (translation) {
                    stamp_token(token, translation);
                    return translation;
                }
            }
        }
        SEL sel = NSSelectorFromString(@"translationApplicationObjectForPid:");
        if (![g_translator respondsToSelector:sel]) return nil;
        IMP imp = class_getMethodImplementation(object_getClass(g_translator), sel);
        if (!imp) return nil;
        id (*fn)(id, SEL, int) = (id (*)(id, SEL, int))imp;
        id translation = fn(g_translator, sel, pid);
        if (translation) stamp_token(token, translation);
        return translation;
    } @catch (NSException *ex) {
        return nil;
    }
}

static id ax_mac_root_from_translation(id translation, NSString *token) {
    if (!translation) return nil;
    @try {
        SEL sel = NSSelectorFromString(@"macPlatformElementFromTranslation:");
        if (![g_translator respondsToSelector:sel]) return nil;
        IMP imp = class_getMethodImplementation(object_getClass(g_translator), sel);
        if (!imp) return nil;
        id (*fn)(id, SEL, id) = (id (*)(id, SEL, id))imp;
        id rootElement = fn(g_translator, sel, translation);
        if (rootElement) {
            stamp_element(token, rootElement);
            stamp_subtree(token, rootElement, 0);
        }
        return rootElement;
    } @catch (NSException *ex) {
        return nil;
    }
}

static id ax_frontmost_translation(NSString *token) {
    @try {
        SEL sel = NSSelectorFromString(@"frontmostApplicationWithDisplayId:bridgeDelegateToken:");
        if (![g_translator respondsToSelector:sel]) return nil;
        IMP imp = class_getMethodImplementation(object_getClass(g_translator), sel);
        if (!imp) return nil;
        id (*fn)(id, SEL, uint32_t, id) = (id (*)(id, SEL, uint32_t, id))imp;
        id translation = fn(g_translator, sel, 0, token);
        if (translation) stamp_token(token, translation);
        return translation;
    } @catch (NSException *ex) {
        return nil;
    }
}

/// Hit-test mid-screen → application translation for whatever is visually on top.
static id ax_translation_via_hit_test(NSString *token, CGSize pointSize, CGRect rootMac) {
    if (pointSize.width < 1 || pointSize.height < 1) return nil;
    double samples[][2] = { { 0.50, 0.42 }, { 0.50, 0.55 }, { 0.50, 0.68 }, { 0.50, 0.30 } };
    for (size_t i = 0; i < sizeof(samples) / sizeof(samples[0]); i++) {
        CGPoint device = CGPointMake(pointSize.width * samples[i][0],
                                     pointSize.height * samples[i][1]);
        CGPoint host = ax_unmap_point(device, rootMac, pointSize);
        id hit = ax_object_at_point(host, token);
        if (!hit) continue;
        stamp_token(token, hit);
        int pid = ax_pid_from_obj(hit);
        if (pid > 0) {
            id app = ax_translation_for_pid(pid, token);
            if (app) return app;
        }
        @try {
            id app = [hit valueForKey:@"application"];
            if (app) {
                stamp_token(token, app);
                return app;
            }
        } @catch (NSException *ex) {
        }
    }
    return nil;
}

/// Resolve foreign-process occlusion above the expected app.
/// Primary: hit-test (any modal). Secondary: known UIService catalog via launchctl.
static BOOL ax_resolve_system_surface(NSString *udid, NSString *token, CGSize pointSize,
                                      CGRect hintRootMac, int frontmostPid,
                                      id *outRoot, NSString **outBundle,
                                      NSString **outProcess, NSString **outRole, int *outPid) {
    if (outRoot) *outRoot = nil;
    if (outBundle) *outBundle = nil;
    if (outProcess) *outProcess = nil;
    if (outRole) *outRole = nil;
    if (outPid) *outPid = 0;

    // 1) Hit-test: whatever owns the pixels at mid-sheet.
    id hitTr = ax_translation_via_hit_test(token, pointSize, hintRootMac);
    int hitPid = ax_pid_from_obj(hitTr);
    if (hitTr && hitPid > 0 && (frontmostPid <= 0 || hitPid != frontmostPid)) {
        id root = ax_mac_root_from_translation(hitTr, token);
        if (root) {
            if (outRoot) *outRoot = root;
            if (outPid) *outPid = hitPid;
            NSString *proc = @"hit_test_occluder";
            if (outProcess) *outProcess = proc;
            if (outRole) *outRole = ax_role_for_process_hint(proc);
            if (outBundle) *outBundle = proc;
            return YES;
        }
    }

    // 2) Catalog assist — known UIServices when hit-test misses.
    int pid = 0;
    NSString *marker = nil;
    if (ax_first_catalog_pid(udid, &pid, &marker) && pid != frontmostPid) {
        id tr = ax_translation_for_pid(pid, token);
        id root = ax_mac_root_from_translation(tr, token);
        if (root) {
            if (outRoot) *outRoot = root;
            if (outPid) *outPid = pid;
            if (outProcess) *outProcess = marker;
            if (outBundle) *outBundle = ax_bundle_for_marker(marker);
            if (outRole) *outRole = ax_role_for_process_hint(marker);
            return YES;
        }
    }
    for (NSNumber *n in ax_guest_pids_matching(udid, ax_system_surface_markers())) {
        if (n.intValue == frontmostPid || n.intValue == pid) continue;
        id tr = ax_translation_for_pid(n.intValue, token);
        id root = ax_mac_root_from_translation(tr, token);
        if (!root) continue;
        if (outRoot) *outRoot = root;
        if (outPid) *outPid = n.intValue;
        if (outProcess) *outProcess = @"UIService";
        if (outBundle) *outBundle = @"system_surface";
        if (outRole) *outRole = @"other";
        return YES;
    }
    return NO;
}

static NSDictionary *ax_walk_tree(id rootElement, NSString *token, CGSize pointSize) {
    if (!rootElement) return nil;
    CGRect rootMac = ax_frame(rootElement);
    g_node_count = 0;
    NSDictionary *root = walk_element(rootElement, rootMac, pointSize, 0, @"", nil, 0, YES);
    if ([root isKindOfClass:[NSMutableDictionary class]]) {
        recover_childless_chrome((NSMutableDictionary *)root, token, rootMac, pointSize);
    }
    return root;
}

static NSUInteger ax_useful_element_count(NSArray *elements) {
    NSUInteger n = 0;
    for (NSDictionary *e in elements ?: @[]) {
        NSString *label = [e[@"label"] description] ?: @"";
        NSString *role = [[e[@"role"] description] ?: @"" lowercaseString];
        if (!label.length && ![role containsString:@"text"] && ![role containsString:@"field"]
            && ![role containsString:@"button"] && ![role containsString:@"link"]) {
            continue;
        }
        n++;
    }
    return n;
}

static NSDictionary *ax_system_surface_payload(NSDictionary *root, NSArray *elements,
                                               CGSize pointSize, NSString *bundle,
                                               NSString *process, NSString *role, int pid) {
    NSMutableDictionary *payload = [@{
        @"status": @"available",
        @"root": root ?: [NSNull null],
        @"elements": elements ?: @[],
        @"element_count": @(elements.count),
        @"point_size": @{ @"width": @(pointSize.width), @"height": @(pointSize.height) },
        @"ax_source": @"system_surface",
        @"ax_bundle": bundle ?: @"system_surface",
        @"ax_process": process ?: @"system_surface",
        @"ax_role": role ?: @"other",
    } mutableCopy];
    if (pid > 0) payload[@"ax_pid"] = @(pid);
    return payload;
}

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

    CGSize pointSize = device_point_size(device);
    id translation = ax_frontmost_translation(token);
    int frontmostPid = ax_pid_from_obj(translation);

    if (!translation) {
        id surfRoot = nil;
        NSString *surfBundle = nil, *surfProc = nil, *surfRole = nil;
        int surfPid = 0;
        if (ax_resolve_system_surface(udid, token, pointSize, CGRectZero, 0,
                                      &surfRoot, &surfBundle, &surfProc, &surfRole, &surfPid)
            && surfRoot) {
            NSDictionary *root = ax_walk_tree(surfRoot, token, pointSize);
            NSMutableArray *elements = [NSMutableArray array];
            flatten_interactive(root, elements, NO);
            [g_dispatcher unregisterToken:token];
            NSDictionary *payload = ax_system_surface_payload(
                root, elements, pointSize, surfBundle, surfProc, surfRole, surfPid);
            NSData *data = [NSJSONSerialization dataWithJSONObject:payload options:0 error:nil];
            NSString *s = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
            return strdup(s.UTF8String);
        }
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

    id rootElement = ax_mac_root_from_translation(translation, token);
    if (!rootElement) {
        set_err(err, 56, "no mac platform element");
        [g_dispatcher unregisterToken:token];
        return NULL;
    }

    CGRect rootMac = ax_frame(rootElement);
    NSDictionary *root = ax_walk_tree(rootElement, token, pointSize);
    NSMutableArray *elements = [NSMutableArray array];
    flatten_interactive(root, elements, NO);

    // Prefer foreign-process occlusion when it owns interactive content.
    id surfRoot = nil;
    NSString *surfBundle = nil, *surfProc = nil, *surfRole = nil;
    int surfPid = 0;
    if (ax_resolve_system_surface(udid, token, pointSize, rootMac, frontmostPid,
                                  &surfRoot, &surfBundle, &surfProc, &surfRole, &surfPid)
        && surfRoot) {
        NSDictionary *surfTree = ax_walk_tree(surfRoot, token, pointSize);
        NSMutableArray *surfElements = [NSMutableArray array];
        flatten_interactive(surfTree, surfElements, NO);
        NSUInteger surfUseful = ax_useful_element_count(surfElements);
        NSUInteger hostUseful = ax_useful_element_count(elements);
        if (surfUseful >= 2 || (surfUseful > 0 && hostUseful < 4) || surfUseful > hostUseful) {
            [g_dispatcher unregisterToken:token];
            NSDictionary *payload = ax_system_surface_payload(
                surfTree, surfElements, pointSize, surfBundle, surfProc, surfRole, surfPid);
            NSError *jsonErr = nil;
            NSData *data = [NSJSONSerialization dataWithJSONObject:payload options:0 error:&jsonErr];
            if (!data) {
                set_err(err, 57, "JSON serialize failed");
                return NULL;
            }
            NSString *s = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
            return strdup(s.UTF8String);
        }
    }

    [g_dispatcher unregisterToken:token];

    NSDictionary *payload = @{
        @"status": @"available",
        @"root": root ?: [NSNull null],
        @"elements": elements,
        @"element_count": @(elements.count),
        @"point_size": @{ @"width": @(pointSize.width), @"height": @(pointSize.height) },
        @"ax_source": @"frontmost",
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

static id find_element_by_ident(id element, NSString *ident, int depth) {
    if (depth >= kMaxDepth || !ident.length) return nil;
    NSString *eid = ax_string(element, @"accessibilityIdentifier");
    if (!eid.length) eid = ax_string(element, @"identifier");
    if ([eid isEqualToString:ident]) return element;
    for (id kid in ax_children(element)) {
        id found = find_element_by_ident(kid, ident, depth + 1);
        if (found) return found;
    }
    return nil;
}

static id find_element_by_label(id element, NSString *needle, int depth) {
    if (depth >= kMaxDepth || !needle.length) return nil;
    NSString *label = ax_string(element, @"accessibilityLabel");
    if (!label.length) label = ax_string(element, @"label");
    if (label.length && [label.lowercaseString containsString:needle.lowercaseString]) {
        return element;
    }
    for (id kid in ax_children(element)) {
        id found = find_element_by_label(kid, needle, depth + 1);
        if (found) return found;
    }
    return nil;
}

static BOOL ax_perform_press(id element) {
    if (!element) return NO;
    @try {
        SEL press = NSSelectorFromString(@"accessibilityPerformPress");
        if ([element respondsToSelector:press]) {
            NSMethodSignature *sig = [element methodSignatureForSelector:press];
            if (sig) {
                NSInvocation *inv = [NSInvocation invocationWithMethodSignature:sig];
                [inv setSelector:press];
                [inv setTarget:element];
                [inv invoke];
                BOOL ok = NO;
                [inv getReturnValue:&ok];
                if (ok) return YES;
            }
        }
        SEL activate = NSSelectorFromString(@"accessibilityActivate");
        if ([element respondsToSelector:activate]) {
            NSMethodSignature *sig = [element methodSignatureForSelector:activate];
            if (sig) {
                NSInvocation *inv = [NSInvocation invocationWithMethodSignature:sig];
                [inv setSelector:activate];
                [inv setTarget:element];
                [inv invoke];
                BOOL ok = NO;
                [inv getReturnValue:&ok];
                if (ok) return YES;
            }
        }
    } @catch (NSException *ex) {
        return NO;
    }
    return NO;
}

bool ligh_host_ax_press(const char *udid_c, const char *target_c, int by_label, LighHostError *err) {
    @try {
        if (!target_c || !target_c[0]) {
            set_err(err, 58, "ax_press: empty target");
            return false;
        }
        if (!ligh_load_private_frameworks(NULL)) {
            set_err(err, 50, "frameworks not loaded");
            return false;
        }
        if (!ensure_ax_translator()) {
            set_err(err, 51, "AXPTranslator unavailable");
            return false;
        }

        NSString *udid = [NSString stringWithUTF8String:udid_c];
        id device = resolve_device(udid);
        if (!device) {
            set_err(err, 52, "SimDevice not found");
            return false;
        }

        NSString *token = [[NSUUID UUID] UUIDString];
        NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:5.0];
        [g_dispatcher registerDevice:device token:token deadline:deadline];

        NSString *target = [NSString stringWithUTF8String:target_c];
        CGSize pointSize = device_point_size(device);

        // Prefer foreign-process occlusion (auth / share / permission) when present.
        id surfRoot = nil;
        NSString *surfBundle = nil, *surfProc = nil, *surfRole = nil;
        int surfPid = 0;
        if (ax_resolve_system_surface(udid, token, pointSize, CGRectZero, 0,
                                      &surfRoot, &surfBundle, &surfProc, &surfRole, &surfPid)
            && surfRoot) {
            id element = by_label
                ? find_element_by_label(surfRoot, target, 0)
                : find_element_by_ident(surfRoot, target, 0);
            if (element && ax_perform_press(element)) {
                [g_dispatcher unregisterToken:token];
                return true;
            }
        }

        id translation = ax_frontmost_translation(token);
        if (!translation) {
            [g_dispatcher unregisterToken:token];
            set_err(err, 56, "no frontmost translation");
            return false;
        }

        id rootElement = ax_mac_root_from_translation(translation, token);
        if (!rootElement) {
            [g_dispatcher unregisterToken:token];
            set_err(err, 56, "no mac platform element");
            return false;
        }

        id element = by_label
            ? find_element_by_label(rootElement, target, 0)
            : find_element_by_ident(rootElement, target, 0);

        BOOL ok = ax_perform_press(element);
        [g_dispatcher unregisterToken:token];
        if (!ok) {
            set_err(err, 58, "ax_press: perform failed");
            return false;
        }
        return true;
    } @catch (NSException *ex) {
        set_err(err, 59, "ax_press exception");
        return false;
    }
}

void ligh_host_ax_free(char *ptr) {
    if (ptr) free(ptr);
}

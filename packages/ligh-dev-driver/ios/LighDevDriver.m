#import "LighDevDriver.h"

#import <arpa/inet.h>
#import <netdb.h>
#import <netinet/in.h>
#import <netinet/tcp.h>
#import <sys/socket.h>
#import <unistd.h>
#import <QuartzCore/QuartzCore.h>

#if __has_include(<React/RCTBundleURLProvider.h>)
#import <React/RCTBundleURLProvider.h>
#endif

static const uint16_t kLighDefaultPort = 7700;

static uint16_t LighPort(void) {
    id v = [[NSBundle mainBundle] objectForInfoDictionaryKey:@"LIGHPort"];
    if ([v respondsToSelector:@selector(intValue)]) {
        int p = [v intValue];
        if (p > 0 && p < 65536) return (uint16_t)p;
    }
    const char *env = getenv("LIGH_DEVICE_PORT");
    if (env) {
        int p = atoi(env);
        if (p > 0 && p < 65536) return (uint16_t)p;
    }
    return kLighDefaultPort;
}

static BOOL LighShouldStart(void) {
    // EAS development builds are Release + expo-dev-client. The plugin
    // stamps LIGHDevDriver in Info.plist only for development/local.
    id flag = [[NSBundle mainBundle] objectForInfoDictionaryKey:@"LIGHDevDriver"];
    if ([flag respondsToSelector:@selector(boolValue)] && [flag boolValue]) return YES;
#ifdef DEBUG
    return YES;
#endif
    if (NSClassFromString(@"EXDevLauncherController") != nil) return YES;
    if ([[[NSBundle mainBundle] objectForInfoDictionaryKey:@"LIGHHost"] length]) return YES;
    NSString *js = [[NSUserDefaults standardUserDefaults] stringForKey:@"RCT_jsLocation"];
    if (js.length > 0) return YES;
#if __has_include(<React/RCTBundleURLProvider.h>)
    if ([RCTBundleURLProvider sharedSettings].jsLocation.length) return YES;
#endif
    return NO;
}

static NSString *LighHostFromLocation(NSString *loc) {
    if (loc.length == 0) return nil;
    NSURL *url = [NSURL URLWithString:loc];
    if (url.host.length) return url.host;
    NSString *stripped = loc;
    if ([stripped hasPrefix:@"http://"] || [stripped hasPrefix:@"https://"]) {
        stripped = [[NSURL URLWithString:stripped] host];
        return stripped;
    }
    NSRange colon = [stripped rangeOfString:@":"];
    if (colon.location != NSNotFound) {
        return [stripped substringToIndex:colon.location];
    }
    return stripped;
}

static NSArray<NSString *> *LighCandidateHosts(void) {
    NSMutableArray<NSString *> *hosts = [NSMutableArray array];
    void (^add)(NSString *) = ^(NSString *h) {
        if (h.length && ![hosts containsObject:h] &&
            ![h isEqualToString:@"localhost"] &&
            ![h isEqualToString:@"127.0.0.1"]) {
            [hosts addObject:h];
        }
    };
    add([[NSBundle mainBundle] objectForInfoDictionaryKey:@"LIGHHost"]);
    const char *env = getenv("LIGH_HOST");
    if (env) add(@(env));
    add(LighHostFromLocation([[NSUserDefaults standardUserDefaults] stringForKey:@"RCT_jsLocation"]));
#if __has_include(<React/RCTBundleURLProvider.h>)
    @try {
        add(LighHostFromLocation([RCTBundleURLProvider sharedSettings].jsLocation));
    } @catch (__unused NSException *ex) {
    }
#endif
    Class cls = NSClassFromString(@"RCTBundleURLProvider");
    if (cls) {
        @try {
            id settings = [cls performSelector:@selector(sharedSettings)];
            if ([settings respondsToSelector:@selector(jsLocation)]) {
                add(LighHostFromLocation([settings valueForKey:@"jsLocation"]));
            }
        } @catch (__unused NSException *ex) {
        }
    }
    return hosts;
}

static NSString *LighRoleForView(UIView *view) {
    if ([view isKindOfClass:[UITextField class]] || [view isKindOfClass:[UITextView class]]) {
        return @"TextField";
    }
    if ([view isKindOfClass:[UIButton class]]) return @"Button";
    if ([view isKindOfClass:[UISwitch class]]) return @"Switch";
    if ([view isKindOfClass:[UISlider class]]) return @"Slider";
    if ([view isKindOfClass:[UITabBar class]]) return @"TabBar";
    if ([view isKindOfClass:[UINavigationBar class]]) return @"NavigationBar";
    if ([view isKindOfClass:[UICollectionView class]] || [view isKindOfClass:[UITableView class]]) {
        return @"List";
    }
    UIAccessibilityTraits t = view.accessibilityTraits;
    if (t & UIAccessibilityTraitButton) return @"Button";
    if (t & UIAccessibilityTraitSearchField) return @"TextField";
    if (t & UIAccessibilityTraitHeader) return @"Header";
    if ([view isKindOfClass:[UILabel class]]) return @"text";
    NSString *cls = NSStringFromClass(view.class);
    if ([cls containsString:@"Button"] || [cls containsString:@"Pressable"] ||
        [cls containsString:@"Touch"]) {
        return @"Button";
    }
    return @"Group";
}

static BOOL LighViewOnScreen(UIView *view, CGRect screen) {
    CGRect f = [view convertRect:view.bounds toView:nil];
    if (CGRectIsEmpty(f) || f.size.width < 1 || f.size.height < 1) return NO;
    return CGRectIntersectsRect(f, screen);
}

static BOOL LighIsInteractive(UIView *view) {
    if (view.hidden || view.alpha < 0.02) return NO;
    if (view.accessibilityIdentifier.length > 0) return YES;
    if (view.accessibilityLabel.length > 0) return YES;
    if ([view isKindOfClass:[UIControl class]]) return YES;
    if ([view isKindOfClass:[UITextField class]] || [view isKindOfClass:[UITextView class]]) return YES;
    if ([view isKindOfClass:[UILabel class]]) {
        NSString *t = [(UILabel *)view text];
        return t.length > 0;
    }
    if ([view isKindOfClass:[UIImageView class]] && view.isAccessibilityElement) return YES;
    if ([view isKindOfClass:[UIScrollView class]] ||
        [view isKindOfClass:[UICollectionView class]] ||
        [view isKindOfClass:[UITableView class]]) {
        return YES;
    }
    UIAccessibilityTraits t = view.accessibilityTraits;
    if (t & (UIAccessibilityTraitButton | UIAccessibilityTraitLink |
             UIAccessibilityTraitSearchField | UIAccessibilityTraitAdjustable |
             UIAccessibilityTraitHeader | UIAccessibilityTraitSelected)) {
        return YES;
    }
    for (UIGestureRecognizer *g in view.gestureRecognizers) {
        if ([g isKindOfClass:[UITapGestureRecognizer class]] ||
            [g isKindOfClass:[UILongPressGestureRecognizer class]] ||
            [g isKindOfClass:[UIPanGestureRecognizer class]]) {
            return YES;
        }
    }
    NSString *cls = NSStringFromClass(view.class);
    if ([cls containsString:@"Paragraph"] || [cls containsString:@"RCTText"] ||
        [cls containsString:@"Pressable"] || [cls containsString:@"Touchable"] ||
        [cls containsString:@"Button"] || [cls containsString:@"TextInput"]) {
        return view.isAccessibilityElement || view.accessibilityLabel.length > 0 ||
               view.userInteractionEnabled;
    }
    return NO;
}

static NSMutableDictionary *LighNodeFromView(UIView *view, CGRect screen) {
    CGRect f = [view convertRect:view.bounds toView:nil];
    BOOL hittable = view.userInteractionEnabled && !view.hidden && view.alpha > 0.02 &&
                    LighViewOnScreen(view, screen);
    CGFloat pw = screen.size.width > 1 ? screen.size.width : 390;
    CGFloat ph = screen.size.height > 1 ? screen.size.height : 844;
    NSMutableDictionary *node = [@{
        @"role": LighRoleForView(view),
        @"hittable": @(hittable),
        @"enabled": @(view.isUserInteractionEnabled),
        @"focused": @(view.isFirstResponder),
        @"visible": @(LighViewOnScreen(view, screen)),
        @"frame": @{
            @"x": @(f.origin.x),
            @"y": @(f.origin.y),
            @"width": @(f.size.width),
            @"height": @(f.size.height),
        },
        @"center_norm": @{
            @"x": @((f.origin.x + f.size.width * 0.5) / pw),
            @"y": @((f.origin.y + f.size.height * 0.5) / ph),
        },
    } mutableCopy];
    if (view.accessibilityIdentifier.length) {
        node[@"identifier"] = view.accessibilityIdentifier;
        node[@"id"] = view.accessibilityIdentifier;
    }
    if (view.accessibilityLabel.length) node[@"label"] = view.accessibilityLabel;
    if ([view isKindOfClass:[UITextField class]]) {
        NSString *v = [(UITextField *)view text];
        if (v) node[@"value"] = v;
        NSString *pholder = [(UITextField *)view placeholder];
        if (pholder.length) node[@"placeholder"] = pholder;
        node[@"secure"] = @([(UITextField *)view isSecureTextEntry]);
        node[@"role"] = @"TextField";
    } else if ([view isKindOfClass:[UITextView class]]) {
        NSString *v = [(UITextView *)view text];
        if (v) node[@"value"] = v;
        node[@"role"] = @"TextField";
    } else if (view.accessibilityValue.length) {
        node[@"value"] = view.accessibilityValue;
    }
    if (!node[@"label"] && [view isKindOfClass:[UIButton class]]) {
        NSString *title = [(UIButton *)view currentTitle];
        if (title.length) node[@"label"] = title;
    }
    if (!node[@"label"] && [view isKindOfClass:[UILabel class]]) {
        NSString *t = [(UILabel *)view text];
        if (t.length) {
            node[@"label"] = t;
            node[@"text"] = t;
        }
    }
    if (!node[@"label"] && view.accessibilityHint.length) {
        node[@"label"] = view.accessibilityHint;
    }
    return node;
}

static void LighWalk(UIView *view, CGRect screen, NSMutableArray *out, int depth) {
    if (!view || depth > 48 || out.count > 2000) return;
    if (LighIsInteractive(view) && LighViewOnScreen(view, screen)) {
        CGRect f = [view convertRect:view.bounds toView:nil];
        // Skip tiny chrome noise (< 8pt) unless it has an id/label.
        BOOL named = view.accessibilityIdentifier.length > 0 || view.accessibilityLabel.length > 0;
        if (named || (f.size.width >= 8 && f.size.height >= 8)) {
            [out addObject:LighNodeFromView(view, screen)];
        }
    }
    for (UIView *child in view.subviews) {
        LighWalk(child, screen, out, depth + 1);
    }
}

static NSDictionary *LighDumpTree(void) {
    UIWindow *window = nil;
    for (UIScene *scene in [UIApplication sharedApplication].connectedScenes) {
        if (![scene isKindOfClass:[UIWindowScene class]]) continue;
        UIWindowScene *ws = (UIWindowScene *)scene;
        for (UIWindow *w in ws.windows) {
            if (w.isKeyWindow) {
                window = w;
                break;
            }
        }
        if (!window) window = ws.windows.firstObject;
        if (window) break;
    }
    if (!window) window = [UIApplication sharedApplication].keyWindow;
    CGRect screen = window.bounds;
    if (CGRectIsEmpty(screen)) screen = [UIScreen mainScreen].bounds;
    NSMutableArray *elements = [NSMutableArray array];
    if (window) LighWalk(window, screen, elements, 0);
    return @{
        @"status": @"available",
        @"root": [NSNull null],
        @"elements": elements,
        @"element_count": @(elements.count),
        @"point_size": @{ @"width": @(screen.size.width), @"height": @(screen.size.height) },
    };
}

static UIView *LighHitView(CGPoint point) {
    UIWindow *window = [UIApplication sharedApplication].keyWindow;
    if (!window) {
        for (UIScene *scene in [UIApplication sharedApplication].connectedScenes) {
            if ([scene isKindOfClass:[UIWindowScene class]]) {
                window = ((UIWindowScene *)scene).windows.firstObject;
                if (window) break;
            }
        }
    }
    return [window hitTest:point withEvent:nil];
}

static UIView *LighFindById(UIView *root, NSString *ident, int depth) {
    if (!root || depth > 40) return nil;
    if ([root.accessibilityIdentifier isEqualToString:ident]) return root;
    for (UIView *c in root.subviews) {
        UIView *hit = LighFindById(c, ident, depth + 1);
        if (hit) return hit;
    }
    return nil;
}

static UIView *LighFindByLabel(UIView *root, NSString *label, int depth) {
    if (!root || depth > 48) return nil;
    NSString *mine = root.accessibilityLabel;
    if (!mine.length && [root isKindOfClass:[UIButton class]]) {
        mine = [(UIButton *)root currentTitle];
    }
    if (!mine.length && [root isKindOfClass:[UILabel class]]) {
        mine = [(UILabel *)root text];
    }
    if (!mine.length && [root isKindOfClass:[UITextField class]]) {
        mine = [(UITextField *)root placeholder];
    }
    if (mine.length && [mine rangeOfString:label options:NSCaseInsensitiveSearch].location != NSNotFound) {
        return root;
    }
    if (root.accessibilityIdentifier.length &&
        [root.accessibilityIdentifier rangeOfString:label options:NSCaseInsensitiveSearch].location !=
            NSNotFound) {
        return root;
    }
    for (UIView *c in root.subviews) {
        UIView *hit = LighFindByLabel(c, label, depth + 1);
        if (hit) return hit;
    }
    return nil;
}

static UIView *LighFindFirstTextInput(UIView *root, int depth) {
    if (!root || depth > 48) return nil;
    if ([root isKindOfClass:[UITextField class]] || [root isKindOfClass:[UITextView class]]) {
        if (!root.hidden && root.alpha > 0.02 && root.userInteractionEnabled) return root;
    }
    NSString *cls = NSStringFromClass(root.class);
    if ([cls containsString:@"TextInput"] && root.userInteractionEnabled) return root;
    for (UIView *c in root.subviews) {
        UIView *hit = LighFindFirstTextInput(c, depth + 1);
        if (hit) return hit;
    }
    return nil;
}

static UIWindow *LighKeyWindow(void) {
    for (UIScene *scene in [UIApplication sharedApplication].connectedScenes) {
        if (![scene isKindOfClass:[UIWindowScene class]]) continue;
        for (UIWindow *w in ((UIWindowScene *)scene).windows) {
            if (w.isKeyWindow) return w;
        }
        UIWindow *first = ((UIWindowScene *)scene).windows.firstObject;
        if (first) return first;
    }
    return [UIApplication sharedApplication].keyWindow;
}

/// Walk up from a glyph/label hit to something that actually receives presses
/// (UIControl, button trait, or a view with tap/long-press recognizers).
static UIView *LighPressableAncestor(UIView *view) {
    UIView *v = view;
    UIView *best = view;
    while (v) {
        if ([v isKindOfClass:[UIControl class]]) return v;
        UIAccessibilityTraits t = v.accessibilityTraits;
        if (t & (UIAccessibilityTraitButton | UIAccessibilityTraitLink)) return v;
        for (UIGestureRecognizer *g in v.gestureRecognizers) {
            if ([g isKindOfClass:[UITapGestureRecognizer class]] ||
                [g isKindOfClass:[UILongPressGestureRecognizer class]]) {
                return v;
            }
        }
        NSString *cls = NSStringFromClass(v.class);
        if ([cls containsString:@"Pressable"] || [cls containsString:@"Touchable"] ||
            [cls containsString:@"Button"]) {
            best = v;
        }
        if (v.userInteractionEnabled) best = v;
        v = v.superview;
    }
    return best;
}

static BOOL LighActivate(UIView *view) {
    UIView *target = LighPressableAncestor(view);
    if (!target) return NO;
    if ([target isKindOfClass:[UIControl class]]) {
        [(UIControl *)target sendActionsForControlEvents:UIControlEventTouchUpInside];
        return YES;
    }
    if ([target accessibilityActivate]) return YES;
    return NO;
}

static void LighSpin(NSTimeInterval seconds) {
    if (seconds <= 0) return;
    NSDate *end = [NSDate dateWithTimeIntervalSinceNow:seconds];
    while ([end timeIntervalSinceNow] > 0) {
        [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode beforeDate:end];
    }
}

static void LighSetTouchLocation(UITouch *touch, CGPoint point) {
    NSValue *val = [NSValue valueWithCGPoint:point];
    @try {
        [touch setValue:val forKey:@"_locationInWindow"];
    } @catch (__unused NSException *ex) {
    }
    @try {
        [touch setValue:val forKey:@"locationInWindow"];
    } @catch (__unused NSException *ex) {
    }
}

static void LighDeliverToView(UIView *view, NSSet *touches, UIEvent *event, UITouchPhase phase) {
    if (!view) return;
    @try {
        switch (phase) {
            case UITouchPhaseBegan:
                [view touchesBegan:touches withEvent:event];
                break;
            case UITouchPhaseMoved:
                [view touchesMoved:touches withEvent:event];
                break;
            case UITouchPhaseEnded:
                [view touchesEnded:touches withEvent:event];
                break;
            case UITouchPhaseCancelled:
                [view touchesCancelled:touches withEvent:event];
                break;
            default:
                break;
        }
    } @catch (__unused NSException *ex) {
    }
    // Bubble toward window so RCTTouchHandler / recognizers see the stream.
    UIView *p = view.superview;
    int depth = 0;
    while (p && depth < 12) {
        @try {
            switch (phase) {
                case UITouchPhaseBegan:
                    [p touchesBegan:touches withEvent:event];
                    break;
                case UITouchPhaseMoved:
                    [p touchesMoved:touches withEvent:event];
                    break;
                case UITouchPhaseEnded:
                    [p touchesEnded:touches withEvent:event];
                    break;
                case UITouchPhaseCancelled:
                    [p touchesCancelled:touches withEvent:event];
                    break;
                default:
                    break;
            }
        } @catch (__unused NSException *ex) {
        }
        p = p.superview;
        depth++;
    }
}

/// One finger sample in window points. phase: began|moved|ended|cancelled
typedef struct {
    CGPoint point;
    NSTimeInterval t;
    UITouchPhase phase;
} LighSample;

static UITouchPhase LighPhaseFromString(NSString *s) {
    s = s.lowercaseString;
    if ([s isEqualToString:@"moved"] || [s isEqualToString:@"move"]) return UITouchPhaseMoved;
    if ([s isEqualToString:@"ended"] || [s isEqualToString:@"end"] || [s isEqualToString:@"up"])
        return UITouchPhaseEnded;
    if ([s isEqualToString:@"cancelled"] || [s isEqualToString:@"cancel"]) return UITouchPhaseCancelled;
    if ([s isEqualToString:@"stationary"]) return UITouchPhaseStationary;
    return UITouchPhaseBegan;
}

/// Play a single-finger human path (began → moved* → ended).
static void LighPlayFinger(NSArray<NSValue *> *samples /* NSValue of NSDictionary */) {
    UIWindow *window = LighKeyWindow();
    if (!window || samples.count == 0) return;

    NSDictionary *first = samples.firstObject;
    CGPoint start = CGPointMake([first[@"x"] doubleValue], [first[@"y"] doubleValue]);
    UIView *hit = LighPressableAncestor(LighHitView(start) ?: window);
    UITouch *touch = [[UITouch alloc] init];
    UIEvent *event = [[UIEvent alloc] init];
    NSTimeInterval t0 = CACurrentMediaTime();
    CGPoint prev = start;

    @try {
        [touch setValue:window forKey:@"window"];
        if (hit) [touch setValue:hit forKey:@"view"];
        [touch setValue:@1 forKey:@"tapCount"];
        [touch setValue:@YES forKey:@"isTap"];
    } @catch (__unused NSException *ex) {
        return;
    }

    for (NSUInteger i = 0; i < samples.count; i++) {
        NSDictionary *s = samples[i];
        CGPoint pt = CGPointMake([s[@"x"] doubleValue], [s[@"y"] doubleValue]);
        UITouchPhase phase = LighPhaseFromString(s[@"phase"] ?: (i == 0 ? @"began" :
            (i + 1 == samples.count ? @"ended" : @"moved")));
        NSTimeInterval at = [s[@"t"] doubleValue];
        if (i == 0) {
            // no wait
        } else {
            NSTimeInterval due = t0 + at;
            NSTimeInterval wait = due - CACurrentMediaTime();
            if (wait > 0) LighSpin(wait);
        }

        @try {
            [touch setValue:@(CACurrentMediaTime()) forKey:@"timestamp"];
            [touch setValue:@(phase) forKey:@"phase"];
            LighSetTouchLocation(touch, pt);
            NSValue *prevVal = [NSValue valueWithCGPoint:prev];
            @try {
                [touch setValue:prevVal forKey:@"_previousLocationInWindow"];
            } @catch (__unused NSException *ex) {
            }
            if (hit) [touch setValue:hit forKey:@"view"];
        } @catch (__unused NSException *ex) {
            return;
        }

        NSSet *set = [NSSet setWithObject:touch];
        @try {
            [event setValue:set forKey:@"_touches"];
            [[UIApplication sharedApplication] sendEvent:event];
        } @catch (__unused NSException *ex) {
        }
        LighDeliverToView(hit, set, event, phase);
        prev = pt;

        // Refresh hit on first move in case scroll shifted hierarchy.
        if (phase == UITouchPhaseMoved && i == 1) {
            UIView *again = LighHitView(pt);
            if (again) hit = LighPressableAncestor(again);
        }
    }
}

static NSArray *LighInterpolatePath(CGPoint from, CGPoint to, NSTimeInterval duration,
                                    NSUInteger steps, BOOL flick) {
    if (steps < 2) steps = 2;
    NSMutableArray *out = [NSMutableArray arrayWithCapacity:steps];
    for (NSUInteger i = 0; i < steps; i++) {
        CGFloat u = (CGFloat)i / (CGFloat)(steps - 1);
        // Ease-out for flick (fast start), linear for drag.
        CGFloat e = flick ? (1.0 - (1.0 - u) * (1.0 - u)) : u;
        CGPoint p = CGPointMake(from.x + (to.x - from.x) * e, from.y + (to.y - from.y) * e);
        NSString *phase = i == 0 ? @"began" : (i + 1 == steps ? @"ended" : @"moved");
        [out addObject:@{
            @"x": @(p.x),
            @"y": @(p.y),
            @"t": @(duration * u),
            @"phase": phase,
        }];
    }
    return out;
}

static void LighGestureTap(CGPoint point, NSTimeInterval downMs) {
    if (downMs < 40) downMs = 80;
    NSArray *path = @[
        @{ @"x": @(point.x), @"y": @(point.y), @"t": @0, @"phase": @"began" },
        @{
            @"x": @(point.x),
            @"y": @(point.y),
            @"t": @(downMs / 1000.0),
            @"phase": @"ended"
        },
    ];
    LighPlayFinger(path);
}

static void LighGestureHold(CGPoint point, NSTimeInterval holdMs) {
    if (holdMs < 200) holdMs = 600;
    NSArray *path = @[
        @{ @"x": @(point.x), @"y": @(point.y), @"t": @0, @"phase": @"began" },
        @{
            @"x": @(point.x),
            @"y": @(point.y),
            @"t": @(holdMs / 1000.0),
            @"phase": @"ended"
        },
    ];
    LighPlayFinger(path);
}

static void LighGestureDoubleTap(CGPoint point) {
    LighGestureTap(point, 70);
    LighSpin(0.12);
    LighGestureTap(point, 70);
}

static void LighGestureSwipe(CGPoint from, CGPoint to, NSTimeInterval durationMs, BOOL flick) {
    if (durationMs < 80) durationMs = flick ? 180 : 320;
    NSUInteger steps = flick ? 8 : 16;
    NSArray *path =
        LighInterpolatePath(from, to, durationMs / 1000.0, steps, flick);
    LighPlayFinger(path);
}

static void LighGesturePinch(CGPoint center, CGFloat startSpan, CGFloat endSpan,
                             NSTimeInterval durationMs) {
    // Two parallel one-finger plays interleaved on main runloop.
    if (durationMs < 120) durationMs = 280;
    NSUInteger steps = 12;
    NSMutableArray *a = [NSMutableArray array];
    NSMutableArray *b = [NSMutableArray array];
    for (NSUInteger i = 0; i < steps; i++) {
        CGFloat u = (CGFloat)i / (CGFloat)(steps - 1);
        CGFloat span = startSpan + (endSpan - startSpan) * u;
        CGPoint p0 = CGPointMake(center.x - span * 0.5, center.y);
        CGPoint p1 = CGPointMake(center.x + span * 0.5, center.y);
        NSString *phase = i == 0 ? @"began" : (i + 1 == steps ? @"ended" : @"moved");
        NSNumber *t = @(durationMs / 1000.0 * u);
        [a addObject:@{ @"x": @(p0.x), @"y": @(p0.y), @"t": t, @"phase": phase }];
        [b addObject:@{ @"x": @(p1.x), @"y": @(p1.y), @"t": t, @"phase": phase }];
    }
    // Approximate pinch: play finger A fully, then B is wrong for true pinch.
    // True two-touch needs concurrent touches — play interleaved samples.
    UIWindow *window = LighKeyWindow();
    if (!window) return;
    UITouch *t0 = [[UITouch alloc] init];
    UITouch *t1 = [[UITouch alloc] init];
    UIEvent *event = [[UIEvent alloc] init];
    UIView *hit = LighHitView(center) ?: window;
    NSTimeInterval tStart = CACurrentMediaTime();
    @try {
        [t0 setValue:window forKey:@"window"];
        [t1 setValue:window forKey:@"window"];
        [t0 setValue:hit forKey:@"view"];
        [t1 setValue:hit forKey:@"view"];
        [t0 setValue:@1 forKey:@"tapCount"];
        [t1 setValue:@1 forKey:@"tapCount"];
    } @catch (__unused NSException *ex) {
        return;
    }
    for (NSUInteger i = 0; i < steps; i++) {
        NSDictionary *s0 = a[i];
        NSDictionary *s1 = b[i];
        NSTimeInterval wait = tStart + [s0[@"t"] doubleValue] - CACurrentMediaTime();
        if (wait > 0) LighSpin(wait);
        UITouchPhase phase = LighPhaseFromString(s0[@"phase"]);
        CGPoint p0 = CGPointMake([s0[@"x"] doubleValue], [s0[@"y"] doubleValue]);
        CGPoint p1 = CGPointMake([s1[@"x"] doubleValue], [s1[@"y"] doubleValue]);
        @try {
            [t0 setValue:@(phase) forKey:@"phase"];
            [t1 setValue:@(phase) forKey:@"phase"];
            LighSetTouchLocation(t0, p0);
            LighSetTouchLocation(t1, p1);
            NSSet *set = [NSSet setWithObjects:t0, t1, nil];
            [event setValue:set forKey:@"_touches"];
            [[UIApplication sharedApplication] sendEvent:event];
            LighDeliverToView(hit, set, event, phase);
        } @catch (__unused NSException *ex) {
            return;
        }
    }
}

static CGPoint LighNormToPoint(CGFloat nx, CGFloat ny) {
    CGRect b = [UIScreen mainScreen].bounds;
    return CGPointMake(nx * b.size.width, ny * b.size.height);
}

static id<UIKeyInput> LighFirstKeyInput(UIView *view) {
    if ([view conformsToProtocol:@protocol(UIKeyInput)] && view.isFirstResponder) {
        return (id<UIKeyInput>)view;
    }
    for (UIView *c in view.subviews) {
        id<UIKeyInput> hit = LighFirstKeyInput(c);
        if (hit) return hit;
    }
    return nil;
}

static void LighOnMain(void (^block)(void)) {
    if ([NSThread isMainThread]) block();
    else dispatch_sync(dispatch_get_main_queue(), block);
}

static NSDictionary *LighHandleOp(NSDictionary *op) {
    NSString *name = op[@"op"];
    __block NSDictionary *result = @{ @"ok": @YES };
    LighOnMain(^{
        CGRect b = [UIScreen mainScreen].bounds;
        if ([name isEqualToString:@"dump"]) {
            result = @{ @"ok": @YES, @"dump": LighDumpTree() };
        } else if ([name isEqualToString:@"tap"]) {
            LighGestureTap(LighNormToPoint([op[@"nx"] doubleValue], [op[@"ny"] doubleValue]), 80);
        } else if ([name isEqualToString:@"double_tap"]) {
            LighGestureDoubleTap(
                LighNormToPoint([op[@"nx"] doubleValue], [op[@"ny"] doubleValue]));
        } else if ([name isEqualToString:@"tap_hold"] || [name isEqualToString:@"long_press"]) {
            NSTimeInterval hold = [op[@"hold_ms"] doubleValue];
            if (hold < 1) hold = 600;
            LighGestureHold(LighNormToPoint([op[@"nx"] doubleValue], [op[@"ny"] doubleValue]),
                            hold);
        } else if ([name isEqualToString:@"tap_id"]) {
            NSString *target = op[@"target"] ?: op[@"id"];
            UIView *v = LighFindById(LighKeyWindow(), target, 0);
            if (v) {
                CGRect f = [v convertRect:v.bounds toView:nil];
                LighGestureTap(CGPointMake(CGRectGetMidX(f), CGRectGetMidY(f)), 80);
            } else {
                result = @{ @"ok": @NO, @"error": @"id not found" };
            }
        } else if ([name isEqualToString:@"tap_label"]) {
            NSString *target = op[@"target"] ?: op[@"label"];
            UIView *v = LighFindByLabel(LighKeyWindow(), target, 0);
            if (v) {
                v = LighPressableAncestor(v);
                CGRect f = [v convertRect:v.bounds toView:nil];
                LighGestureTap(CGPointMake(CGRectGetMidX(f), CGRectGetMidY(f)), 80);
            } else {
                result = @{ @"ok": @NO, @"error": @"label not found" };
            }
        } else if ([name isEqualToString:@"type"]) {
            NSString *text = op[@"text"] ?: @"";
            id<UIKeyInput> input = LighFirstKeyInput(LighKeyWindow());
            if (!input) {
                UIView *tf = LighFindFirstTextInput(LighKeyWindow(), 0);
                if (tf) {
                    CGRect f = [tf convertRect:tf.bounds toView:nil];
                    LighGestureTap(CGPointMake(CGRectGetMidX(f), CGRectGetMidY(f)), 80);
                    LighSpin(0.35);
                    input = LighFirstKeyInput(LighKeyWindow());
                    if (!input && [tf conformsToProtocol:@protocol(UIKeyInput)]) {
                        [tf becomeFirstResponder];
                        LighSpin(0.2);
                        input = (id<UIKeyInput>)tf;
                    }
                }
            }
            if (input) {
                [input insertText:text];
            } else {
                result = @{ @"ok": @NO, @"error": @"no first responder" };
            }
        } else if ([name isEqualToString:@"clear"]) {
            NSUInteger n = [op[@"count"] unsignedIntegerValue] ?: 40;
            id<UIKeyInput> input = LighFirstKeyInput(LighKeyWindow());
            for (NSUInteger i = 0; i < n; i++) {
                if ([input respondsToSelector:@selector(deleteBackward)]) [input deleteBackward];
            }
        } else if ([name isEqualToString:@"key"]) {
            NSString *key = [op[@"name"] lowercaseString];
            id<UIKeyInput> input = LighFirstKeyInput(LighKeyWindow());
            if ([key isEqualToString:@"return"] || [key isEqualToString:@"enter"]) {
                [input insertText:@"\n"];
            } else if ([key isEqualToString:@"space"]) {
                [input insertText:@" "];
            } else if ([key isEqualToString:@"delete"] || [key isEqualToString:@"backspace"]) {
                [input deleteBackward];
            }
        } else if ([name isEqualToString:@"swipe"] || [name isEqualToString:@"flick"] ||
                   [name isEqualToString:@"pan"] || [name isEqualToString:@"drag"]) {
            CGPoint from = LighNormToPoint([op[@"from_nx"] doubleValue], [op[@"from_ny"] doubleValue]);
            CGPoint to = LighNormToPoint([op[@"to_nx"] doubleValue], [op[@"to_ny"] doubleValue]);
            NSTimeInterval dur = [op[@"duration_ms"] doubleValue];
            BOOL flick = [name isEqualToString:@"flick"] || [op[@"flick"] boolValue];
            if ([name isEqualToString:@"swipe"] && dur < 1) dur = 280;
            if ([name isEqualToString:@"pan"] || [name isEqualToString:@"drag"]) {
                if (dur < 1) dur = 450;
                flick = NO;
            }
            if (flick && dur < 1) dur = 180;
            LighGestureSwipe(from, to, dur, flick);
        } else if ([name isEqualToString:@"edge_swipe"]) {
            NSString *edge = [op[@"edge"] lowercaseString] ?: @"left";
            CGFloat y = [op[@"ny"] doubleValue];
            if (y <= 0 || y >= 1) y = 0.5;
            CGPoint from, to;
            if ([edge isEqualToString:@"right"]) {
                from = LighNormToPoint(0.98, y);
                to = LighNormToPoint(0.35, y);
            } else if ([edge isEqualToString:@"top"]) {
                from = LighNormToPoint(0.5, 0.02);
                to = LighNormToPoint(0.5, 0.45);
            } else {
                from = LighNormToPoint(0.02, y);
                to = LighNormToPoint(0.65, y);
            }
            LighGestureSwipe(from, to, 280, NO);
        } else if ([name isEqualToString:@"pull_refresh"]) {
            CGFloat x = [op[@"nx"] doubleValue];
            if (x <= 0 || x >= 1) x = 0.5;
            CGPoint from = LighNormToPoint(x, 0.18);
            CGPoint mid = LighNormToPoint(x, 0.42);
            LighGestureSwipe(from, mid, 400, NO);
            LighSpin(0.05);
            LighGestureSwipe(mid, from, 200, NO);
        } else if ([name isEqualToString:@"pinch"]) {
            CGPoint c = LighNormToPoint([op[@"nx"] doubleValue], [op[@"ny"] doubleValue]);
            CGFloat start = [op[@"start_span"] doubleValue];
            CGFloat end = [op[@"end_span"] doubleValue];
            if (start < 1) start = 120;
            if (end < 1) end = 40;
            NSTimeInterval dur = [op[@"duration_ms"] doubleValue];
            if (dur < 1) dur = 280;
            LighGesturePinch(c, start, end, dur);
        } else if ([name isEqualToString:@"gesture"]) {
            // Human Gesture IR: { points:[{nx,ny,t_ms,phase}] } or { fingers:[[...]] }
            NSArray *fingers = op[@"fingers"];
            if (![fingers isKindOfClass:[NSArray class]] || fingers.count == 0) {
                NSArray *points = op[@"points"];
                if ([points isKindOfClass:[NSArray class]] && points.count > 0) {
                    fingers = @[ points ];
                }
            }
            if (![fingers isKindOfClass:[NSArray class]] || fingers.count == 0) {
                result = @{ @"ok": @NO, @"error": @"gesture needs points or fingers" };
            } else if (fingers.count == 1) {
                NSMutableArray *samples = [NSMutableArray array];
                for (NSDictionary *p in fingers[0]) {
                    if (![p isKindOfClass:[NSDictionary class]]) continue;
                    CGFloat nx = [p[@"nx"] ?: p[@"x"] doubleValue];
                    CGFloat ny = [p[@"ny"] ?: p[@"y"] doubleValue];
                    // nx/ny if <=1.5 treat as normalized; else points
                    CGPoint pt;
                    if (fabs(nx) <= 1.5 && fabs(ny) <= 1.5 && !p[@"x"] && !p[@"y"]) {
                        pt = LighNormToPoint(nx, ny);
                    } else if (p[@"nx"] || p[@"ny"]) {
                        pt = LighNormToPoint(nx, ny);
                    } else {
                        pt = CGPointMake(nx, ny);
                    }
                    NSTimeInterval t = [p[@"t_ms"] doubleValue] / 1000.0;
                    if (p[@"t"]) t = [p[@"t"] doubleValue];
                    [samples addObject:@{
                        @"x": @(pt.x),
                        @"y": @(pt.y),
                        @"t": @(t),
                        @"phase": p[@"phase"] ?: @"moved",
                    }];
                }
                if (samples.count) {
                    NSMutableDictionary *first = [samples[0] mutableCopy];
                    first[@"phase"] = samples[0][@"phase"] ?: @"began";
                    samples[0] = first;
                    NSMutableDictionary *last = [samples.lastObject mutableCopy];
                    last[@"phase"] = @"ended";
                    samples[samples.count - 1] = last;
                    LighPlayFinger(samples);
                }
            } else {
                // Multi-finger: use first two as pinch-like concurrent stream
                result = @{
                    @"ok": @NO,
                    @"error": @"multi-finger raw IR: use pinch or single-finger points for now"
                };
            }
        } else if ([name isEqualToString:@"idle"] || [name isEqualToString:@"home"] ||
                   [name isEqualToString:@"ping"]) {
            [CATransaction flush];
        } else {
            result = @{ @"ok": @NO, @"error": @"unknown op" };
        }
        (void)b;
    });
    return result;
}

static NSData *LighJSONLine(NSDictionary *obj) {
    NSError *err = nil;
    NSData *data = [NSJSONSerialization dataWithJSONObject:obj options:0 error:&err];
    if (!data) return [@"{\"ok\":false}\n" dataUsingEncoding:NSUTF8StringEncoding];
    NSMutableData *line = [data mutableCopy];
    [line appendBytes:"\n" length:1];
    return line;
}

static NSDictionary *LighHello(void) {
    CGRect b = [UIScreen mainScreen].bounds;
    NSString *bundle = [NSBundle mainBundle].bundleIdentifier ?: @"unknown";
    return @{
        @"op": @"hello",
        @"bundle_id": bundle,
        @"width": @(b.size.width),
        @"height": @(b.size.height),
        @"driver_version": @2,
        @"capabilities": @{
            @"gesture": @YES,
            @"tap": @YES,
            @"double_tap": @YES,
            @"long_press": @YES,
            @"pan": @YES,
            @"swipe": @YES,
            @"flick": @YES,
            @"pinch": @YES,
            @"edge_swipe": @YES,
            @"pull_refresh": @YES,
            @"type": @YES,
            @"scroll_until": @YES,
            @"dense_dump": @YES,
        },
    };
}

static void LighServeSocket(int fd) {
    NSMutableData *buf = [NSMutableData data];
    char tmp[4096];
    BOOL sentHello = NO;
    NSData *hello = LighJSONLine(LighHello());
    if (send(fd, hello.bytes, hello.length, 0) < 0) {
        close(fd);
        return;
    }
    sentHello = YES;
    (void)sentHello;
    while (1) {
        ssize_t n = recv(fd, tmp, sizeof(tmp), 0);
        if (n <= 0) break;
        [buf appendBytes:tmp length:(NSUInteger)n];
        while (1) {
            NSData *raw = buf;
            const char *bytes = raw.bytes;
            NSUInteger len = raw.length;
            NSUInteger nl = NSNotFound;
            for (NSUInteger i = 0; i < len; i++) {
                if (bytes[i] == '\n') {
                    nl = i;
                    break;
                }
            }
            if (nl == NSNotFound) break;
            NSData *line = [raw subdataWithRange:NSMakeRange(0, nl)];
            [buf replaceBytesInRange:NSMakeRange(0, nl + 1) withBytes:NULL length:0];
            if (line.length == 0) continue;
            NSDictionary *op = [NSJSONSerialization JSONObjectWithData:line options:0 error:nil];
            if (![op isKindOfClass:[NSDictionary class]]) continue;
            if ([op[@"op"] isEqualToString:@"hello_ok"]) continue;
            NSMutableDictionary *reply = [LighHandleOp(op) mutableCopy];
            if (op[@"id"]) reply[@"id"] = op[@"id"];
            NSData *out = LighJSONLine(reply);
            if (send(fd, out.bytes, out.length, 0) < 0) {
                close(fd);
                return;
            }
        }
    }
    close(fd);
}

static int LighConnectHost(NSString *host, uint16_t port) {
    struct addrinfo hints, *res = NULL;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_INET;
    hints.ai_socktype = SOCK_STREAM;
    char portstr[16];
    snprintf(portstr, sizeof(portstr), "%u", port);
    if (getaddrinfo(host.UTF8String, portstr, &hints, &res) != 0 || !res) return -1;
    int fd = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    if (fd < 0) {
        freeaddrinfo(res);
        return -1;
    }
    struct timeval tv;
    tv.tv_sec = 2;
    tv.tv_usec = 0;
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv));
    int ok = connect(fd, res->ai_addr, res->ai_addrlen);
    freeaddrinfo(res);
    if (ok < 0) {
        close(fd);
        return -1;
    }
    int one = 1;
    setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one));
    // Idle forever until lighd sends ops. A 30s RCVTIMEO was killing sessions.
    tv.tv_sec = 0;
    tv.tv_usec = 0;
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
    return fd;
}

static void LighListenUSB(uint16_t port) {
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) return;
    int one = 1;
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port = htons(port);
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (bind(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        close(fd);
        return;
    }
    listen(fd, 2);
    while (1) {
        int cfd = accept(fd, NULL, NULL);
        if (cfd < 0) continue;
        setsockopt(cfd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one));
        LighServeSocket(cfd);
    }
}

@interface LighDevDriver () <NSNetServiceBrowserDelegate, NSNetServiceDelegate>
@property (nonatomic, strong) NSNetServiceBrowser *browser;
@property (nonatomic, strong) NSMutableArray<NSNetService *> *resolving;
@property (nonatomic, strong) NSMutableArray<NSString *> *bonjourHosts;
@end

@implementation LighDevDriver

+ (void)load {
    if (!LighShouldStart()) return;
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.8 * NSEC_PER_SEC)),
                   dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{
                       [self start];
                   });
}

+ (instancetype)shared {
    static LighDevDriver *g;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        g = [[LighDevDriver alloc] init];
        g.resolving = [NSMutableArray array];
        g.bonjourHosts = [NSMutableArray array];
    });
    return g;
}

+ (void)start {
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        [[self shared] ligh_begin];
    });
}

- (void)ligh_begin {
    uint16_t port = LighPort();
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{
        LighListenUSB(port);
    });
    dispatch_async(dispatch_get_main_queue(), ^{
        self.browser = [[NSNetServiceBrowser alloc] init];
        self.browser.delegate = self;
        [self.browser searchForServicesOfType:@"_ligh._tcp." inDomain:@"local."];
    });
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{
        [self ligh_connectLoop:port];
    });
}

- (void)ligh_connectLoop:(uint16_t)port {
    while (1) {
        NSMutableArray<NSString *> *hosts = [LighCandidateHosts() mutableCopy];
        @synchronized(self.bonjourHosts) {
            for (NSString *h in self.bonjourHosts) {
                if (![hosts containsObject:h]) [hosts addObject:h];
            }
        }
        BOOL connected = NO;
        for (NSString *host in hosts) {
            int fd = LighConnectHost(host, port);
            if (fd < 0) continue;
            LighServeSocket(fd);
            connected = YES;
            break;
        }
        (void)connected;
        sleep(1);
    }
}

- (void)netServiceBrowser:(NSNetServiceBrowser *)browser
           didFindService:(NSNetService *)service
               moreComing:(BOOL)moreComing {
    (void)browser;
    (void)moreComing;
    service.delegate = self;
    [self.resolving addObject:service];
    [service resolveWithTimeout:3];
}

- (void)netServiceDidResolveAddress:(NSNetService *)sender {
    for (NSData *data in sender.addresses) {
        struct sockaddr_in *addr = (struct sockaddr_in *)data.bytes;
        if (addr->sin_family != AF_INET) continue;
        char buf[INET_ADDRSTRLEN];
        inet_ntop(AF_INET, &addr->sin_addr, buf, sizeof(buf));
        NSString *ip = @(buf);
        if ([ip hasPrefix:@"127."]) continue;
        @synchronized(self.bonjourHosts) {
            if (![self.bonjourHosts containsObject:ip]) [self.bonjourHosts addObject:ip];
        }
    }
    [self.resolving removeObject:sender];
}

@end

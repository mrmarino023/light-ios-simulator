#import <dlfcn.h>
#import <Foundation/Foundation.h>
#import "frameworks.h"

static bool g_loaded = false;

bool ligh_load_private_frameworks(const char *developer_dir) {
    if (g_loaded) {
        return true;
    }

    const char *core_sim =
        "/Library/Developer/PrivateFrameworks/CoreSimulator.framework/CoreSimulator";
    if (!dlopen(core_sim, RTLD_NOW | RTLD_GLOBAL)) {
        return false;
    }

    NSString *dev = developer_dir
                        ? [NSString stringWithUTF8String:developer_dir]
                        : @"/Applications/Xcode.app/Contents/Developer";
    NSString *simkit = [dev stringByAppendingPathComponent:
        @"Library/PrivateFrameworks/SimulatorKit.framework/SimulatorKit"];
    if (!dlopen(simkit.fileSystemRepresentation, RTLD_NOW | RTLD_GLOBAL)) {
        return false;
    }

    g_loaded = true;
    return true;
}

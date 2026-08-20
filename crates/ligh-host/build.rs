use std::env;

fn main() {
    let _developer_dir = env::var("DEVELOPER_DIR").unwrap_or_else(|_| {
        String::from("/Applications/Xcode.app/Contents/Developer")
    });

    cc::Build::new()
        .file("src/bridge/frameworks.m")
        .file("src/bridge/display_bridge.m")
        .file("src/bridge/hid_bridge.m")
        .file("src/bridge/ax_bridge.m")
        .flag("-fobjc-arc")
        .flag("-Wno-deprecated-declarations")
        .include("src/bridge")
        .compile("ligh_host_bridge");

    println!("cargo:rerun-if-changed=src/bridge/display_bridge.m");
    println!("cargo:rerun-if-changed=src/bridge/frameworks.m");
    println!("cargo:rerun-if-changed=src/bridge/hid_bridge.m");
    println!("cargo:rerun-if-changed=src/bridge/ax_bridge.m");

    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=IOSurface");
    println!("cargo:rustc-link-lib=framework=CoreFoundation");
    println!("cargo:rustc-link-lib=framework=CoreGraphics");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=objc");
}


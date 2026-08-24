/**
 * @mm-labs/ligh-expo — Expo config plugin for the physical DevDriver.
 *
 * Universal: any Expo / RN development client. Not app-specific.
 * Native sources live in ./native (never named ios/ — Expo app gitignore
 * patterns like bare `ios/` would otherwise drop them from EAS upload when
 * the package is vendored into a customer repo).
 *
 * Store profiles (production / preview) skip the plugin entirely.
 */
const fs = require("fs");
const path = require("path");
const {
  withDangerousMod,
  withInfoPlist,
  createRunOncePlugin,
} = require("@expo/config-plugins");

const PKG = "@mm-labs/ligh-expo";
const NATIVE_FILES = ["LighDevDriver.h", "LighDevDriver.m", "LighDevDriver.podspec"];

function skipStoreBuild() {
  const profile = process.env.EAS_BUILD_PROFILE || "";
  return profile === "production" || profile === "preview";
}

function nativeSrcDir() {
  const native = path.join(__dirname, "native");
  if (fs.existsSync(path.join(native, "LighDevDriver.m"))) return native;
  // Legacy layout fallback
  const ios = path.join(__dirname, "ios");
  if (fs.existsSync(path.join(ios, "LighDevDriver.m"))) return ios;
  throw new Error(
    `${PKG}: missing native/LighDevDriver.m — reinstall the package or sync from light-ios-simulator/packages/ligh-expo`
  );
}

function copyNativeIntoIos(iosRoot) {
  const srcDir = nativeSrcDir();
  const destDir = path.join(iosRoot, "LighDevDriver");
  fs.mkdirSync(destDir, { recursive: true });
  for (const file of NATIVE_FILES) {
    const from = path.join(srcDir, file);
    if (!fs.existsSync(from)) {
      throw new Error(`${PKG}: missing ${from}`);
    }
    fs.copyFileSync(from, path.join(destDir, file));
  }
}

function ensurePodfile(iosRoot) {
  const podfilePath = path.join(iosRoot, "Podfile");
  if (!fs.existsSync(podfilePath)) return;
  let src = fs.readFileSync(podfilePath, "utf8");
  if (src.includes("LighDevDriver")) return;
  const line = "  pod 'LighDevDriver', :path => './LighDevDriver'";
  if (src.includes("use_expo_modules!")) {
    src = src.replace("use_expo_modules!", `use_expo_modules!\n${line}`);
  } else if (src.includes("use_native_modules!")) {
    src = src.replace(/use_native_modules!/, (m) => `${m}\n${line}`);
  } else {
    src += `\n${line}\n`;
  }
  fs.writeFileSync(podfilePath, src);
}

function withLighExpo(config, props = {}) {
  if (skipStoreBuild()) return config;

  const port = props.port || 7700;
  const usage =
    props.localNetworkUsage ||
    "Connects to LIGH on your Mac during development (same as Metro).";

  config = withInfoPlist(config, (mod) => {
    mod.modResults.LIGHDevDriver = true;
    mod.modResults.LIGHPort = String(port);
    if (props.host) mod.modResults.LIGHHost = String(props.host);
    const bonjour = new Set(mod.modResults.NSBonjourServices || []);
    bonjour.add("_ligh._tcp.");
    mod.modResults.NSBonjourServices = [...bonjour];
    if (!mod.modResults.NSLocalNetworkUsageDescription) {
      mod.modResults.NSLocalNetworkUsageDescription = usage;
    }
    return mod;
  });

  config = withDangerousMod(config, [
    "ios",
    async (mod) => {
      const iosRoot = mod.modRequest.platformProjectRoot;
      copyNativeIntoIos(iosRoot);
      ensurePodfile(iosRoot);
      return mod;
    },
  ]);

  return config;
}

module.exports = createRunOncePlugin(withLighExpo, PKG, "0.2.0");

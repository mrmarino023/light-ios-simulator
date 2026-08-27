#!/usr/bin/env python3
"""Detect iOS project metadata for agent onboarding (.xcodeproj / .app / task.json)."""

from __future__ import annotations

import argparse
import json
import os
import plistlib
import re
import subprocess
import sys
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from ligh_audit_accessibility import (  # noqa: E402
    audit_source_root,
    suggest_app_goal,
    suggest_app_job_steps,
    suggest_verification,
)


def _run(cmd: list[str], *, timeout: int = 120) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)


# Directories never walked when hunting for an app project (OSS-general).
_XCODE_SKIP_DIRS = frozenset(
    {
        ".git",
        ".build",
        "DerivedData",
        "Pods",
        "Carthage",
        "node_modules",
        "vendor",
        ".swiftpm",
        "xcuserdata",
        "SourcePackages",  # SPM checkouts under DerivedData-like trees
        "checkouts",
        "Library",  # poisoned clones / accidental FS copies
        "System",
        "usr",
    }
)
_XCODE_SKIP_NAME_TOKS = ("Test", "UITest", "Tests", "Watch", "TV", "Widget", "Extension", "Intents")
_XCODE_PATH_BOOST = ("Example", "Examples", "Demo", "Playground", "Sample", "App", "iOS")


def _xcode_score(cand: str, root: str, prefer: str | None) -> tuple[int, int, str]:
    """Higher score wins. Prefer Example/Demo apps, shallow paths, non-satellite targets."""
    rel = os.path.relpath(cand, root)
    parts = rel.split(os.sep)
    depth = max(0, len(parts) - 1)
    base = os.path.basename(cand)
    stem = base[: -len(".xcodeproj")] if base.endswith(".xcodeproj") else base[: -len(".xcworkspace")]
    score = 0
    if any(tok in stem for tok in _XCODE_SKIP_NAME_TOKS):
        score -= 200
    for boost in _XCODE_PATH_BOOST:
        if any(boost.lower() == p.lower() or boost.lower() in p.lower() for p in parts[:-1]):
            score += 40
            break
    if prefer:
        pref = prefer.lower().replace(" ", "").replace("-", "")
        st = stem.lower().replace(" ", "").replace("-", "")
        if pref and (pref in st or st in pref):
            score += 35
    score -= depth * 8
    # Prefer workspace when CocoaPods/SPM wrap the same stem (resolved later).
    if cand.endswith(".xcworkspace"):
        score += 3
    return (score, -depth, stem.lower())


def iter_xcode_candidates(path: str) -> list[str]:
    """All .xcodeproj / .xcworkspace under path (recursive, OSS-general)."""
    path = os.path.abspath(path)
    out: list[str] = []
    if path.endswith(".xcodeproj") or path.endswith(".xcworkspace"):
        return [path] if os.path.isdir(path) else []
    if not os.path.isdir(path):
        return []
    for dp, dirs, _ in os.walk(path):
        dirs[:] = [d for d in dirs if d not in _XCODE_SKIP_DIRS and not d.startswith(".")]
        for d in list(dirs):
            if d.endswith(".xcodeproj") or d.endswith(".xcworkspace"):
                out.append(os.path.join(dp, d))
                # Do not walk into the bundle.
                dirs.remove(d)
    return out


def find_xcodeproj(path: str, *, prefer_name: str | None = None) -> str | None:
    """Pick the best app project under path — recursive, scored, no per-app maps."""
    path = os.path.abspath(path)
    if path.endswith(".xcodeproj") and os.path.isdir(path):
        return path
    if path.endswith(".xcworkspace") and os.path.isdir(path):
        return path

    cands = iter_xcode_candidates(path)
    if not cands:
        return None

    # Prefer .xcworkspace over sibling .xcodeproj with the same stem/dir.
    by_key: dict[str, str] = {}
    for c in cands:
        parent = os.path.dirname(c)
        stem = os.path.basename(c).rsplit(".", 1)[0]
        key = f"{parent}::{stem}"
        prev = by_key.get(key)
        if prev is None or (c.endswith(".xcworkspace") and prev.endswith(".xcodeproj")):
            by_key[key] = c
    ranked = sorted(
        by_key.values(),
        key=lambda c: _xcode_score(c, path, prefer_name),
        reverse=True,
    )
    return ranked[0] if ranked else None


def schemes_from_disk(xcode_path: str) -> list[str]:
    """Shared/user schemes without waiting on SPM / xcodebuild -list."""
    names: list[str] = []
    for rel in (
        ("xcshareddata", "xcschemes"),
        ("xcuserdata",),  # scanned below for *.xcscheme
    ):
        base = os.path.join(xcode_path, *rel) if rel[0] != "xcuserdata" else os.path.join(xcode_path, "xcuserdata")
        if not os.path.isdir(base):
            continue
        if rel[0] == "xcuserdata":
            for dp, _, files in os.walk(base):
                for f in files:
                    if f.endswith(".xcscheme"):
                        names.append(f[: -len(".xcscheme")])
            continue
        for f in os.listdir(base):
            if f.endswith(".xcscheme"):
                names.append(f[: -len(".xcscheme")])
    # Stable order, dedupe
    out: list[str] = []
    seen: set[str] = set()
    for n in names:
        if n not in seen:
            seen.add(n)
            out.append(n)
    return out


def schemes_from_pbx_targets(xcode_path: str) -> list[str]:
    if xcode_path.endswith(".xcworkspace"):
        return []
    pbx = os.path.join(xcode_path, "project.pbxproj")
    if not os.path.isfile(pbx):
        return []
    text = open(pbx, encoding="utf-8", errors="ignore").read()
    # PBXNativeTarget name = Foo;
    found = re.findall(r"/\* (\w+) \*/ = \{\s*isa = PBXNativeTarget;", text)
    if not found:
        found = re.findall(r'name = "?([A-Za-z0-9_.-]+)"?;\s*productName =', text)
    return found


def scheme_file(xcode_path: str, scheme: str) -> str | None:
    for rel in (
        ("xcshareddata", "xcschemes", f"{scheme}.xcscheme"),
        # user schemes: xcuserdata/*/xcschemes/
    ):
        p = os.path.join(xcode_path, *rel)
        if os.path.isfile(p):
            return p
    user = os.path.join(xcode_path, "xcuserdata")
    if os.path.isdir(user):
        for dp, _, files in os.walk(user):
            if f"{scheme}.xcscheme" in files:
                return os.path.join(dp, f"{scheme}.xcscheme")
    return None


def scheme_embeds_watch(xcode_path: str, scheme: str) -> bool:
    """True if scheme XML *or* project graph needs watchOS (companion apps)."""
    if "Watch" in scheme or "watchOS" in scheme:
        return True
    path = scheme_file(xcode_path, scheme)
    if path:
        text = open(path, encoding="utf-8", errors="ignore").read()
        if "watchos" in text.lower() or "WatchKit" in text:
            return True
        if re.search(r'BlueprintName\s*=\s*"[^"]*Watch[^"]*"', text):
            return True
    # Backyard Birds etc.: scheme looks iOS-only but pbx embeds Watch via deps.
    try:
        from ligh_host_capability import pbx_has_watch_product

        if pbx_has_watch_product(xcode_path):
            return True
    except Exception:  # noqa: BLE001
        pass
    return False


def host_has_watchos_runtime() -> bool:
    try:
        from ligh_host_capability import probe_host

        return bool(probe_host().watchos_runtimes)
    except Exception:  # noqa: BLE001
        cp = _run(["xcrun", "simctl", "list", "runtimes"], timeout=30)
        blob = (cp.stdout or "") + (cp.stderr or "")
        return "watchOS" in blob


def list_schemes(xcode_path: str) -> list[str]:
    # Fast path: disk schemes — works offline / before SPM resolve (OSS-general).
    disk = schemes_from_disk(xcode_path)
    if disk:
        return disk
    # Next: PBX targets — avoid hanging xcodebuild -list on SPM packages.
    pbx = schemes_from_pbx_targets(xcode_path)
    if pbx:
        return pbx

    args = ["xcodebuild", "-list", "-json"]
    if xcode_path.endswith(".xcworkspace"):
        args.extend(["-workspace", xcode_path])
    else:
        args.extend(["-project", xcode_path])
    cp = _run(args, timeout=60)
    blob = (cp.stdout or "") + "\n" + (cp.stderr or "")
    # SPM / xcodebuild often emit logs before the JSON object.
    brace = blob.find("{")
    if brace >= 0:
        try:
            data = json.loads(blob[brace:])
            schemes = data.get("project", {}).get("schemes") or data.get("workspace", {}).get("schemes")
            if schemes:
                return list(schemes)
        except json.JSONDecodeError:
            pass

    list_args = ["xcodebuild", "-list"]
    if xcode_path.endswith(".xcworkspace"):
        list_args.extend(["-workspace", xcode_path])
    else:
        list_args.extend(["-project", xcode_path])
    cp = _run(list_args, timeout=60)
    schemes: list[str] = []
    in_schemes = False
    for line in (cp.stdout or "").splitlines():
        if line.strip() == "Schemes:":
            in_schemes = True
            continue
        if in_schemes:
            if not line.strip():
                break
            schemes.append(line.strip())
    if schemes:
        return schemes
    return []


def pick_scheme(schemes: list[str], *, xcode_path: str | None = None) -> str | None:
    skip = ("Test", "UITest", "Tests", "Extension", "Widget", "Intents", "Watch", "TV", "Share")
    has_watch_rt = None  # lazy

    def ok(s: str) -> bool:
        nonlocal has_watch_rt
        if any(tok in s for tok in skip):
            return False
        if xcode_path and scheme_embeds_watch(xcode_path, s):
            if has_watch_rt is None:
                has_watch_rt = host_has_watchos_runtime()
            if not has_watch_rt:
                return False
        return True

    preferred = [s for s in schemes if ok(s)]
    if preferred:
        # Prefer shortest name that isn't a satellite (often the main app).
        preferred.sort(key=lambda x: (len(x), x.lower()))
        return preferred[0]
    # No runnable scheme on this host (e.g. every app embeds Watch).
    return None


def bundle_id_from_pbxproj(xcode_path: str) -> str | None:
    if xcode_path.endswith(".xcworkspace"):
        return None
    pbx = os.path.join(xcode_path, "project.pbxproj")
    if not os.path.isfile(pbx):
        return None
    text = open(pbx, encoding="utf-8", errors="ignore").read()
    ids = re.findall(r"PRODUCT_BUNDLE_IDENTIFIER = ([^;]+);", text)
    for raw in ids:
        bid = raw.strip().strip('"')
        if bid and not bid.startswith("$("):
            return bid
    return None


def bundle_id_from_app(app_path: str) -> str | None:
    plist = os.path.join(app_path, "Info.plist")
    if not os.path.isfile(plist):
        return None
    with open(plist, "rb") as f:
        info = plistlib.load(f)
    return info.get("CFBundleIdentifier")


def guess_source_root(project_dir: str, scheme: str | None) -> str:
    project_dir = os.path.abspath(project_dir)
    if scheme:
        cand = os.path.join(project_dir, scheme)
        if os.path.isdir(cand):
            return cand
    for name in os.listdir(project_dir):
        p = os.path.join(project_dir, name)
        if os.path.isdir(p) and name.endswith(".swift") is False:
            swift_count = sum(
                1 for dp, _, fs in os.walk(p) for f in fs if f.endswith(".swift")
            )
            if swift_count >= 3:
                return p
    return project_dir


def classify_build_error(msg: str) -> str:
    low = msg.lower()
    if "watchos" in low and ("must be installed" in low or "watch" in low):
        return "missing_watchos_runtime"
    if "future xcode project file format" in low:
        return "xcode_format_too_new"
    if "requires a development team" in low or "code signing" in low:
        return "codesign"
    if "timed out" in low:
        return "build_timeout"
    return "build_failed"


def build_sim_app(xcode_path: str, scheme: str, out_dir: str) -> str:
    os.makedirs(out_dir, exist_ok=True)
    derived = os.path.join(out_dir, "DerivedData")
    args = [
        "xcodebuild",
        "-scheme",
        scheme,
        "-configuration",
        "Debug",
        "-sdk",
        "iphonesimulator",
        "-derivedDataPath",
        derived,
        "-destination",
        "generic/platform=iOS Simulator",
        "-skipPackagePluginValidation",
        "ONLY_ACTIVE_ARCH=YES",
        "ARCHS=arm64",
        "CODE_SIGNING_ALLOWED=NO",
        "CODE_SIGNING_REQUIRED=NO",
        "CODE_SIGN_IDENTITY=",
        "build",
    ]
    if xcode_path.endswith(".xcworkspace"):
        args[1:1] = ["-workspace", xcode_path]
    else:
        args[1:1] = ["-project", xcode_path]
    # SPM checkouts write git hooks; sandboxes/CI often block that — disable hooks.
    env = os.environ.copy()
    env["GIT_CONFIG_COUNT"] = "1"
    env["GIT_CONFIG_KEY_0"] = "core.hooksPath"
    env["GIT_CONFIG_VALUE_0"] = "/dev/null"
    cp = subprocess.run(args, capture_output=True, text=True, timeout=360, env=env)
    if cp.returncode != 0:
        err = (cp.stderr or cp.stdout or "xcodebuild failed")[-800:]
        raise RuntimeError(f"{classify_build_error(err)}: {err}")
    apps = []
    for dp, dirs, _ in os.walk(derived):
        for d in dirs:
            if d.endswith(".app"):
                apps.append(os.path.join(dp, d))
    if not apps:
        raise RuntimeError("no .app under DerivedData")
    # Prefer the main app (not Watch.app / Appex).
    def app_rank(p: str) -> tuple:
        name = os.path.basename(p)
        bad = any(tok in name for tok in ("Watch", "Widget", "Appextension", "Extension"))
        return (1 if bad else 0, -os.path.getmtime(p))

    apps.sort(key=app_rank)
    dest = os.path.join(out_dir, os.path.basename(apps[0]))
    if os.path.isdir(dest):
        import shutil

        shutil.rmtree(dest)
    import shutil

    shutil.copytree(apps[0], dest)
    # Drop DerivedData after copy — OSS batch otherwise fills the disk.
    try:
        shutil.rmtree(derived)
    except OSError:
        pass
    return dest


def detect(path: str, *, build: bool = False) -> dict[str, Any]:
    path = os.path.abspath(path)
    doc: dict[str, Any] = {"schema": 1, "input": path}

    if path.endswith(".json") and os.path.isfile(path):
        task = json.load(open(path, encoding="utf-8"))
        doc.update(
            {
                "kind": "task",
                "task_path": path,
                "source_root": os.path.abspath(
                    task["source_root"]
                    if os.path.isabs(task["source_root"])
                    else os.path.join(ROOT, task["source_root"])
                ),
                "app_path": os.path.abspath(
                    task["app_path"]
                    if os.path.isabs(task["app_path"])
                    else os.path.join(ROOT, task["app_path"])
                ),
                "bundle_id": task["bundle_id"],
                "workspace": os.path.dirname(path),
            }
        )
    elif path.endswith(".app"):
        # Built .app is often gitignored — fall back to sibling .xcodeproj for offline CI.
        if os.path.isdir(path):
            project_dir = os.path.dirname(os.path.dirname(path))
            doc.update(
                {
                    "kind": "app",
                    "app_path": path,
                    "bundle_id": bundle_id_from_app(path),
                    "source_root": guess_source_root(project_dir, None),
                    "workspace": project_dir,
                }
            )
        else:
            project_dir = os.path.dirname(os.path.dirname(path))
            xcode = find_xcodeproj(project_dir) if os.path.isdir(project_dir) else None
            if not xcode:
                raise SystemExit(
                    f"missing .app (not built / gitignored): {path}\n"
                    f"  build it first, or pass the .xcodeproj directory"
                )
            path = project_dir
            # fall through to xcodeproj handling below
            doc["input"] = path
            doc["_fallback_from_missing_app"] = True

    if "kind" not in doc:
        prefer = os.path.basename(path.rstrip("/"))
        xcode = find_xcodeproj(path, prefer_name=prefer)
        if not xcode:
            raise SystemExit(f"no .xcodeproj/.app/task.json at {path}")
        # HostCapability gate — skip before burning build minutes.
        try:
            from ligh_host_capability import gate_project, probe_host

            host = probe_host()
            skip = gate_project(xcode, host)
            if skip:
                raise RuntimeError(f"{skip['fault']}: {skip.get('detail')}")
        except ImportError:
            pass
        project_dir = os.path.dirname(xcode)
        # Workspace for .ligh is the clone/subtree root (path), not only the xcode parent —
        # monorepos (Package + Example/) keep agent bundle at the scoped root.
        workspace = path if os.path.isdir(path) else project_dir
        schemes = list_schemes(xcode)
        scheme = pick_scheme(schemes, xcode_path=xcode)
        if not scheme:
            raise RuntimeError(
                "missing_watchos_runtime: no iOS scheme runnable without watchOS on this host"
                if not host_has_watchos_runtime()
                else f"no scheme in {xcode}"
            )
        if scheme_embeds_watch(xcode, scheme) and not host_has_watchos_runtime():
            raise RuntimeError(
                f"missing_watchos_runtime: scheme {scheme!r} embeds Watch; no watchOS runtime"
            )
        doc.update(
            {
                "kind": "xcodeproj",
                "xcode_path": xcode,
                "scheme": scheme,
                "schemes": schemes,
                "bundle_id": bundle_id_from_pbxproj(xcode),
                "source_root": guess_source_root(project_dir, scheme),
                "workspace": workspace,
                "build_out": os.path.join(workspace, "build", "ligh"),
                "xcode_candidates": [
                    os.path.relpath(c, path) for c in iter_xcode_candidates(path)[:12]
                ],
            }
        )
        if build:
            doc["app_path"] = build_sim_app(xcode, scheme, doc["build_out"])
            doc["bundle_id"] = bundle_id_from_app(doc["app_path"]) or doc.get("bundle_id")

    src = doc.get("source_root")
    if src and os.path.isdir(src):
        audit = audit_source_root(src)
        steps = suggest_app_job_steps(audit)
        doc["audit"] = audit
        doc["suggested_app_job"] = steps
        doc["suggested_app_goal"] = suggest_app_goal(audit, steps)
        doc["suggested_verification"] = suggest_verification(audit, steps)

    return doc


def write_agent_bundle(doc: dict[str, Any], out_dir: str) -> None:
    os.makedirs(out_dir, exist_ok=True)
    json.dump(doc, open(os.path.join(out_dir, "project.json"), "w"), indent=2)
    json.dump(
        doc.get("suggested_app_job") or [],
        open(os.path.join(out_dir, "app-job.json"), "w"),
        indent=2,
    )
    json.dump(
        doc.get("suggested_app_goal") or {"setup": [], "postconditions": []},
        open(os.path.join(out_dir, "app-goal.json"), "w"),
        indent=2,
    )
    task_skeleton = {
        "id": os.path.basename(doc.get("workspace") or "my-app"),
        "protocol_version": 2,
        "agent_prompt": "Verify this iOS app with LIGH: build, exercise the flow, fix failures, certify.",
        "source_root": doc.get("source_root"),
        "app_path": doc.get("app_path"),
        "bundle_id": doc.get("bundle_id"),
        "bootstrap_wait_label": (doc.get("suggested_verification") or {}).get("bootstrap_wait_label"),
        "verification": doc.get("suggested_verification") or {},
    }
    json.dump(task_skeleton, open(os.path.join(out_dir, "task.skeleton.json"), "w"), indent=2)

    app = doc.get("app_path") or "PATH/TO/App.app"
    bid = doc.get("bundle_id") or "com.example.app"
    steps = json.dumps(doc.get("suggested_app_job") or [], indent=2)
    prompt = f"""# LIGH agent prompt (generated)

You have **LIGH MCP** on this Mac. Verify my iOS Simulator Debug build — fail-closed only.

**App:** `{app}`  
**Bundle:** `{bid}`  
**Readiness:** {doc.get('audit', {}).get('readiness_grade', '?')} ({doc.get('audit', {}).get('readiness_score', '?')}%)

## Loop

1. `ligh_init` on the Xcode project (once) — or `./scripts/ligh-paradise.sh`
2. `ligh_up` → `ligh_viewer` (optional — watch sim in browser)
3. **`ligh_test`** — goal-first verify from `.ligh/app-goal.json`
4. On `{{ ok: false, fault, detail }}` → fix Swift → rebuild → `ligh_test`
5. Success = `ok: true` only — never claim from screenshots

## Suggested app-goal (preferred)

```json
{json.dumps(doc.get("suggested_app_goal") or {}, indent=2)}
```

## Fallback app-job (explicit steps)

```json
{steps}
```

## Accessibility audit

- Identities found: {doc.get('audit', {}).get('identity_count', 0)}
- Missing interactive ids: {doc.get('audit', {}).get('missing_interactive', 0)}
- Add `.accessibilityIdentifier("…")` to controls without ids before expecting reliable motor.

Full audit: `.ligh/project.json`
"""
    open(os.path.join(out_dir, "AGENT_PROMPT.md"), "w", encoding="utf-8").write(prompt)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("path", help=".xcodeproj dir, .app, or task.json")
    ap.add_argument("--build", action="store_true", help="xcodebuild Debug simulator .app")
    ap.add_argument("--write", metavar="DIR", help="Write .ligh bundle (default: <workspace>/.ligh)")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()
    doc = detect(args.path, build=args.build)
    out_dir = args.write or os.path.join(doc.get("workspace") or os.getcwd(), ".ligh")
    write_agent_bundle(doc, out_dir)
    doc["ligh_dir"] = out_dir
    if args.json:
        print(json.dumps(doc, indent=2))
    else:
        print(json.dumps({"ok": True, "ligh_dir": out_dir, "grade": doc.get("audit", {}).get("readiness_grade")}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

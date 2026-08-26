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


def find_xcodeproj(path: str) -> str | None:
    path = os.path.abspath(path)
    if path.endswith(".xcodeproj") and os.path.isdir(path):
        return path
    if path.endswith(".xcworkspace") and os.path.isdir(path):
        return path
    if os.path.isdir(path):
        for name in sorted(os.listdir(path)):
            if name.endswith(".xcodeproj"):
                return os.path.join(path, name)
    return None


def list_schemes(xcode_path: str) -> list[str]:
    args = ["xcodebuild", "-list", "-json"]
    if xcode_path.endswith(".xcworkspace"):
        args.extend(["-workspace", xcode_path])
    else:
        args.extend(["-project", xcode_path])
    cp = _run(args, timeout=60)
    if cp.returncode == 0 and cp.stdout.strip().startswith("{"):
        data = json.loads(cp.stdout)
        schemes = data.get("project", {}).get("schemes") or data.get("workspace", {}).get("schemes")
        if schemes:
            return list(schemes)
    cp = _run(["xcodebuild", "-list", "-project", xcode_path], timeout=60)
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
    return schemes


def pick_scheme(schemes: list[str]) -> str | None:
    for s in schemes:
        if "Test" in s or "UITest" in s:
            continue
        return s
    return schemes[0] if schemes else None


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
    cp = _run(args, timeout=600)
    if cp.returncode != 0:
        raise RuntimeError((cp.stderr or cp.stdout or "xcodebuild failed")[-800:])
    apps = []
    for dp, dirs, _ in os.walk(derived):
        for d in dirs:
            if d.endswith(".app"):
                apps.append(os.path.join(dp, d))
    if not apps:
        raise RuntimeError("no .app under DerivedData")
    apps.sort(key=lambda p: os.path.getmtime(p), reverse=True)
    dest = os.path.join(out_dir, os.path.basename(apps[0]))
    if os.path.isdir(dest):
        import shutil

        shutil.rmtree(dest)
    import shutil

    shutil.copytree(apps[0], dest)
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
    elif path.endswith(".app") and os.path.isdir(path):
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
        xcode = find_xcodeproj(path)
        if not xcode:
            raise SystemExit(f"no .xcodeproj/.app/task.json at {path}")
        project_dir = os.path.dirname(xcode)
        schemes = list_schemes(xcode)
        scheme = pick_scheme(schemes)
        if not scheme:
            raise SystemExit(f"no scheme in {xcode}")
        doc.update(
            {
                "kind": "xcodeproj",
                "xcode_path": xcode,
                "scheme": scheme,
                "schemes": schemes,
                "bundle_id": bundle_id_from_pbxproj(xcode),
                "source_root": guess_source_root(project_dir, scheme),
                "workspace": project_dir,
                "build_out": os.path.join(project_dir, "build", "ligh"),
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

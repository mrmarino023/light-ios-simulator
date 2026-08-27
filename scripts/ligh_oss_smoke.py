#!/usr/bin/env python3
"""OSS-general stranger smoke — one pipeline for every iOS repo.

No per-app scheme/label/bundle maps. Input is only:

  URL                 → clone → detect → build → discover → ligh_test
  URL#relative/path   → same, scoped to subtree (monorepo Examples/)
  /local/path         → skip clone

Architecture (bulletproof stages — fault ownership matters):

  HostCapability → acquire → ProjectResolve → gate_project → build
  → EyesReady (SpringBoard AX) → label-first discover → ligh_test

  host_* / sim_boot_hung / eyes_unusable  → do not edit Swift
  discover_no_chrome                      → only after EyesReady ok
  missing_watchos / xcode_format_too_new  → skip (not fail)

Usage:
  python3 scripts/ligh_oss_smoke.py https://github.com/apple/sample-food-truck.git
  python3 scripts/ligh_oss_smoke.py --urls-file urls.txt --write docs/assets/out.json
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
import urllib.request
import zipfile
from typing import Any
from urllib.parse import urlparse

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from ligh_discover import discover_live, write_discovered_bundle  # noqa: E402
from ligh_project import detect, write_agent_bundle  # noqa: E402

LIGH = os.environ.get("LIGH_BIN", os.path.join(ROOT, "target", "release", "ligh"))
LIGHD = os.environ.get("LIGHD_BIN", os.path.join(os.path.dirname(LIGH), "lighd"))

HOST_SKIP_FAULTS = frozenset(
    {
        "missing_watchos_runtime",
        "xcode_format_too_new",
        "disk_exhausted",
        "missing_ios_runtime",
        "acquire_not_found",
    }
)
HOST_FAIL_FAULTS = frozenset({"sim_boot_hung", "eyes_unusable"})


def parse_spec(spec: str) -> tuple[str, str | None]:
    """Split 'url_or_path' or 'url_or_path#subdir' — subdir scopes a monorepo, not app knowledge."""
    spec = spec.strip()
    if not spec or spec.startswith("#"):
        raise ValueError(f"empty spec: {spec!r}")
    if "#" not in spec:
        return spec, None
    base, _, frag = spec.partition("#")
    frag = frag.strip().strip("/")
    return base, frag or None


def repo_slug(url: str) -> str:
    path = urlparse(url).path.rstrip("/")
    name = os.path.basename(path)
    if name.endswith(".git"):
        name = name[: -len(".git")]
    return name or "repo"


def _run(cmd: list[str], *, timeout: int = 120, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=env)


def _git_env() -> dict[str, str]:
    env = os.environ.copy()
    env["GIT_CONFIG_COUNT"] = "1"
    env["GIT_CONFIG_KEY_0"] = "core.hooksPath"
    env["GIT_CONFIG_VALUE_0"] = "/dev/null"
    return env


def acquire(spec: str, work: str) -> tuple[str, str, str]:
    """Return (scoped_root, display_name, origin)."""
    origin, sub = parse_spec(spec)
    os.makedirs(work, exist_ok=True)

    if os.path.isdir(origin):
        root = os.path.abspath(origin)
        name = os.path.basename(root.rstrip("/"))
    else:
        name = repo_slug(origin)
        dest = os.path.join(work, name)
        if not os.path.isdir(dest) or not os.listdir(dest):
            _clone_or_zip(origin, dest)
        root = dest

    if sub:
        scoped = os.path.join(root, sub)
        if not os.path.isdir(scoped):
            raise FileNotFoundError(f"subtree missing: {scoped}")
        return scoped, f"{name}/{sub}", origin
    return root, name, origin


def _clone_or_zip(url: str, dest: str) -> None:
    env = _git_env()
    if os.path.isdir(dest):
        shutil.rmtree(dest)
    cp = _run(["git", "clone", "--depth", "1", url, dest], timeout=300, env=env)
    if cp.returncode == 0 and os.path.isdir(dest):
        return
    m = re.match(r"https?://github\.com/([^/]+)/([^/.]+)", url)
    if not m:
        raise RuntimeError(f"acquire_failed: {(cp.stderr or cp.stdout or '')[-400:]}")
    owner, repo = m.group(1), m.group(2)
    tmp = dest + ".zip"
    last_err = ""
    for branch in ("main", "master"):
        zurl = f"https://github.com/{owner}/{repo}/archive/refs/heads/{branch}.zip"
        try:
            urllib.request.urlretrieve(zurl, tmp)
            with zipfile.ZipFile(tmp) as zf:
                zf.extractall(os.path.dirname(dest))
            extracted = os.path.join(os.path.dirname(dest), f"{repo}-{branch}")
            if os.path.isdir(extracted):
                if os.path.isdir(dest):
                    shutil.rmtree(dest)
                os.rename(extracted, dest)
                return
        except Exception as e:  # noqa: BLE001
            last_err = str(e)
            if "404" in last_err:
                raise RuntimeError(f"acquire_not_found: {url} ({last_err})") from e
            continue
        finally:
            if os.path.isfile(tmp):
                os.remove(tmp)
    if "404" in last_err:
        raise RuntimeError(f"acquire_not_found: {url}")
    raise RuntimeError(f"acquire_failed: {url}: {last_err or (cp.stderr or '')[-300:]}")


def _ax_element_count() -> int:
    raw = _run([LIGH, "--json", "ax"], timeout=30)
    blob = (raw.stdout or "") + (raw.stderr or "")
    brace = blob.find("{")
    if brace < 0:
        return 0
    try:
        d = json.loads(blob[brace:])
        tree = d.get("tree") if isinstance(d.get("tree"), dict) else d
        return int(tree.get("element_count") or 0)
    except (json.JSONDecodeError, TypeError, ValueError):
        return 0


def _wait_springboard_ax(*, timeout_s: float = 180) -> int:
    """Block until AX is non-empty (SpringBoard finished first-boot / not Apple logo)."""
    deadline = time.time() + timeout_s
    last = 0
    while time.time() < deadline:
        last = _ax_element_count()
        if last >= 5:
            return last
        time.sleep(3)
    raise RuntimeError(
        f"sim_boot_hung: accessibility still empty after SpringBoard wait (last_count={last})"
    )


def recover_eyes(device: str) -> int:
    """EyesReady soft recovery — prefer GUI attach; never thrash slim reboot first."""
    _run([LIGH, "ready"], timeout=90)
    cp = _run([LIGH, "up", "--gui", "--device", device], timeout=120)
    if cp.returncode != 0:
        _run([LIGH, "up", "--device", device], timeout=120)
    return _wait_springboard_ax(timeout_s=180)


def ensure_daemon(device: str, work: str) -> None:
    _run([LIGH, "daemon", "stop"], timeout=30)
    subprocess.run(["pkill", "-x", "lighd"], capture_output=True)
    time.sleep(0.4)
    log = open(os.path.join(work, "lighd.log"), "a")
    subprocess.Popen([LIGHD], stdout=log, stderr=log, start_new_session=True)
    time.sleep(1.2)
    cp = _run([LIGH, "up", "--gui", "--device", device], timeout=180)
    if cp.returncode != 0:
        cp = _run([LIGH, "up", "--device", device], timeout=180)
    if cp.returncode != 0:
        raise RuntimeError(f"ligh up failed: {(cp.stderr or cp.stdout or '')[-400:]}")
    try:
        _wait_springboard_ax(timeout_s=240)
    except RuntimeError:
        _run([LIGH, "device", "create", "-d", device], timeout=120)
        _run([LIGH, "up", "--gui", "--device", device], timeout=180)
        _wait_springboard_ax(timeout_s=300)


def _classify_error(err: str) -> tuple[str | None, str, bool]:
    """Return (fault, fault_owner, skip)."""
    for klass in (
        "missing_watchos_runtime",
        "xcode_format_too_new",
        "disk_exhausted",
        "missing_ios_runtime",
        "acquire_not_found",
        "sim_boot_hung",
        "codesign",
        "build_timeout",
        "build_failed",
        "acquire_failed",
    ):
        if err.startswith(klass) or f"{klass}:" in err or klass in err:
            if klass in HOST_SKIP_FAULTS:
                return klass, "host", True
            if klass in HOST_FAIL_FAULTS or klass == "sim_boot_hung":
                return klass, "host", False
            if klass.startswith("acquire"):
                return klass, "host", klass == "acquire_not_found"
            return klass, "build", False
    return None, "unknown", False


def smoke_one(spec: str, *, work: str, device: str) -> dict[str, Any]:
    """One stranger — same stages for every URL."""
    row: dict[str, Any] = {"spec": spec, "ok": False, "stage": "acquire", "fault_owner": None}
    t0 = time.time()
    try:
        scoped, name, origin = acquire(spec, work)
        row.update({"name": name, "repo": origin, "root": scoped})

        row["stage"] = "detect_build"
        doc = detect(scoped, build=True)
        app = doc.get("app_path")
        bid = doc.get("bundle_id")
        src = doc.get("source_root") or scoped
        ws = doc.get("workspace") or scoped
        if not app or not bid:
            row["error"] = "missing app_path or bundle_id after build"
            row["fault"] = "build_failed"
            row["fault_owner"] = "build"
            return row
        row.update(
            {
                "scheme": doc.get("scheme"),
                "xcode_path": doc.get("xcode_path"),
                "bundle_id": bid,
                "app_path": app,
                "accessibility_ids": int((doc.get("audit") or {}).get("identity_count") or 0),
            }
        )

        ligh_dir = os.path.join(ws, ".ligh")
        write_agent_bundle(doc, ligh_dir)

        row["stage"] = "eyes_ready"
        ax_n = recover_eyes(device)
        row["ax_element_count"] = ax_n

        row["stage"] = "discover"
        disc = discover_live(app, bid, source_root=src)
        write_discovered_bundle(ligh_dir, doc, disc)
        row["proven_chrome"] = disc.get("proven_chrome") or disc.get("wait_hint")
        row["discover_ready"] = bool(disc.get("agent_ready"))
        row["bootstrap_ok"] = bool(disc.get("bootstrap_ok"))

        chrome = disc.get("proven_chrome")
        if not chrome or chrome == "REPLACE_ME":
            row["fault"] = "discover_no_chrome"
            row["fault_owner"] = "app"
            row["error"] = "live discover did not motor-prove chrome — refusing placeholder goal"
            return row

        row["stage"] = "ligh_test"
        env = os.environ.copy()
        env["LIGH_WORKSPACE"] = ws
        env["LIGH_BIN"] = LIGH
        env["PYTHONPATH"] = os.path.join(ROOT, "scripts")
        test_out = os.path.join(work, f"{name.replace('/', '_')}-test.json")
        env["LIGH_TEST_OUT"] = test_out
        cp = _run([os.path.join(ROOT, "scripts", "ligh-test.sh")], timeout=300, env=env)
        result: dict[str, Any] = {}
        if os.path.isfile(test_out):
            result = json.load(open(test_out, encoding="utf-8"))
        else:
            result = {"ok": False, "fault": "no_test_out", "detail": (cp.stderr or cp.stdout or "")[-400:]}
        if result.get("fault") == "eyes_unusable":
            recover_eyes(device)
            disc2 = discover_live(app, bid, source_root=src)
            write_discovered_bundle(ligh_dir, doc, disc2)
            row["proven_chrome"] = disc2.get("proven_chrome") or row.get("proven_chrome")
            cp = _run([os.path.join(ROOT, "scripts", "ligh-test.sh")], timeout=300, env=env)
            if os.path.isfile(test_out):
                result = json.load(open(test_out, encoding="utf-8"))
        fault = result.get("fault")
        row["ligh_test"] = {
            "ok": bool(result.get("ok")),
            "fault": fault,
            "mode": result.get("mode"),
        }
        row["ok"] = bool(result.get("ok"))
        if not row["ok"]:
            row["fault"] = fault or "ligh_test_failed"
            row["fault_owner"] = "host" if fault in HOST_FAIL_FAULTS else "app"
        row["stage"] = "done" if row["ok"] else "ligh_test"
        return row
    except Exception as e:  # noqa: BLE001
        err = str(e)[:600]
        row["error"] = err
        fault, owner, skip = _classify_error(err)
        row["fault"] = fault or "unknown"
        row["fault_owner"] = owner
        row["skip"] = skip
        return row
    finally:
        row["elapsed_s"] = round(time.time() - t0, 1)


def _is_host_skip(row: dict[str, Any]) -> bool:
    if row.get("skip"):
        return True
    fault = row.get("fault") or ""
    if fault in HOST_SKIP_FAULTS:
        return True
    err = str(row.get("error") or "")
    return any(k in err for k in HOST_SKIP_FAULTS)


def main() -> int:
    ap = argparse.ArgumentParser(description="OSS-general stranger smoke (URL → ligh_test)")
    ap.add_argument("specs", nargs="*", help="URL, URL#subdir, or local path")
    ap.add_argument("--urls-file", help="One spec per line (# comments ok)")
    ap.add_argument("--work", default=os.environ.get("LIGH_OSS_WORK", os.path.join(ROOT, ".oss-trial")))
    ap.add_argument("--device", default=os.environ.get("LIGH_DEVICE", "iphone-15-pro"))
    ap.add_argument("--write", default=os.environ.get("LIGH_OSS_OUT", ""))
    ap.add_argument("--skip-daemon", action="store_true")
    args = ap.parse_args()

    specs: list[str] = list(args.specs)
    if args.urls_file:
        for line in open(args.urls_file, encoding="utf-8"):
            line = line.strip()
            if line and not line.startswith("#"):
                specs.append(line)
    if not specs:
        ap.error("pass specs and/or --urls-file")

    if not os.path.isfile(LIGH) or not os.access(LIGH, os.X_OK):
        print(json.dumps({"ok": False, "error": f"missing LIGH_BIN={LIGH}"}), file=sys.stderr)
        return 2

    from ligh_host_capability import probe_host

    host = probe_host(ligh_bin=LIGH)
    print(json.dumps({"host": host.to_dict()}, indent=2), flush=True)
    if host.disk_free_gb < 2.0:
        print(json.dumps({"ok": False, "fault": "disk_exhausted", "host": host.to_dict()}), file=sys.stderr)
        return 2

    if not args.skip_daemon:
        ensure_daemon(args.device, args.work)

    rows: list[dict[str, Any]] = []
    for spec in specs:
        print(f"══ {spec} ══", flush=True)
        row = smoke_one(spec, work=args.work, device=args.device)
        rows.append(row)
        if row.get("ok"):
            mark = "✓"
        elif _is_host_skip(row):
            mark = "⊘"
        else:
            mark = "✗"
        print(
            f"  {mark} stage={row.get('stage')} owner={row.get('fault_owner')} "
            f"chrome={row.get('proven_chrome')} "
            f"fault={(row.get('ligh_test') or {}).get('fault') or row.get('fault') or row.get('error')} "
            f"ids={row.get('accessibility_ids')}",
            flush=True,
        )

    ok_rows = [r for r in rows if r.get("ok")]
    skipped = [r for r in rows if not r.get("ok") and _is_host_skip(r)]
    failed = [r for r in rows if r not in ok_rows and r not in skipped]
    doc = {
        "gate": "oss_stranger_trial",
        "schema": 3,
        "ok": len(ok_rows) >= 2,
        "holy_shit": len(ok_rows) >= 5,
        "passed": len(ok_rows),
        "skipped": len(skipped),
        "failed": len(failed),
        "attempted": len(rows),
        "summary": (
            f"{len(ok_rows)} pass / {len(skipped)} host-skip / {len(failed)} fail "
            f"— HostCapability + EyesReady + label-first; no per-app maps"
        ),
        "architecture": (
            "HostCapability → acquire → recursive_xcode_score → gate_project → "
            "build → EyesReady → label_first_discover → ligh_test"
        ),
        "fault_owners": ["host", "build", "app"],
        "host": host.to_dict(),
        "apps": rows,
        "ts": int(time.time()),
    }
    out = args.write or os.path.join(ROOT, "docs", "assets", "oss-stranger-trial-latest.json")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    json.dump(doc, open(out, "w"), indent=2)
    open(out, "a").write("\n")
    print(json.dumps({
        "passed": doc["passed"],
        "skipped": doc["skipped"],
        "failed": doc["failed"],
        "holy_shit": doc["holy_shit"],
        "out": out,
    }, indent=2))
    return 0 if doc["passed"] >= 2 else 1


if __name__ == "__main__":
    raise SystemExit(main())

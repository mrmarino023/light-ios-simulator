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
from ligh_project import build_sim_app, bundle_id_from_app, detect, write_agent_bundle  # noqa: E402
from ligh_stranger import StrangerEntry, entry_row_meta, find_built_app, parse_entry  # noqa: E402

LIGH = os.environ.get("LIGH_BIN", os.path.join(ROOT, "target", "release", "ligh"))
LIGHD = os.environ.get("LIGHD_BIN", os.path.join(os.path.dirname(LIGH), "lighd"))

HOST_SKIP_FAULTS = frozenset(
    {
        "missing_watchos_runtime",
        "xcode_format_too_new",
        "swift_tools_too_new",
        "disk_exhausted",
        "missing_ios_runtime",
        "acquire_not_found",
        "not_ios_simulator",
        "build_required",
        "no_xcodeproj",
        "spm_resolve_failed",
        "spm_resolve_timeout",
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


def _booted_ligh_udid() -> str | None:
    cp = _run(["xcrun", "simctl", "list", "devices", "booted", "-j"], timeout=30)
    try:
        data = json.loads(cp.stdout or "{}")
        for devs in (data.get("devices") or {}).values():
            for d in devs:
                if d.get("isAvailable") is False:
                    continue
                if str(d.get("name", "")).startswith("LIGH"):
                    return d.get("udid")
    except json.JSONDecodeError:
        pass
    return None


def _resolve_udid(device: str) -> str:
    cp = _run([LIGH, "device", "create", "-d", device], timeout=120)
    for line in (cp.stdout or "").splitlines():
        m = re.search(r"[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}", line, re.I)
        if m:
            return m.group(0)
    udid = _booted_ligh_udid()
    if udid:
        return udid
    raise RuntimeError("sim_boot_hung: could not resolve simulator UDID")


def _open_simulator_app() -> None:
    sim_app = "/Applications/Xcode.app/Contents/Developer/Applications/Simulator.app"
    if os.path.isdir(sim_app):
        subprocess.Popen(["open", "-a", sim_app])
    else:
        subprocess.Popen(["open", "-a", "Simulator"])


def _boot_and_wait(udid: str, *, recreate: bool = False) -> None:
    if recreate:
        _run(["xcrun", "simctl", "shutdown", udid], timeout=60)
        time.sleep(1)
    state = _run(["xcrun", "simctl", "list", "devices", udid, "-j"], timeout=30)
    booted = "Booted" in (state.stdout or "")
    if not booted:
        _run(["xcrun", "simctl", "boot", udid], timeout=120)
    cp = _run(["xcrun", "simctl", "bootstatus", udid, "-b"], timeout=300)
    if cp.returncode != 0:
        raise RuntimeError(f"sim_boot_hung: bootstatus failed: {(cp.stderr or cp.stdout or '')[-300:]}")
    _open_simulator_app()
    time.sleep(2)


def _attach_ligh(device: str) -> None:
    _run([LIGH, "ready"], timeout=120)
    cp = _run([LIGH, "up", "--gui", "--device", device], timeout=180)
    if cp.returncode != 0:
        cp = _run([LIGH, "up", "--device", device], timeout=180)
    if cp.returncode != 0:
        raise RuntimeError(f"ligh up failed: {(cp.stderr or cp.stdout or '')[-400:]}")


def _wait_springboard_ax(*, timeout_s: float = 180) -> int:
    """Block until AX is non-empty (SpringBoard finished first-boot / not Apple logo)."""
    deadline = time.time() + timeout_s
    last = 0
    while time.time() < deadline:
        _run([LIGH, "ready"], timeout=90)
        last = _ax_element_count()
        if last >= 5:
            return last
        time.sleep(3)
    raise RuntimeError(
        f"sim_boot_hung: accessibility still empty after SpringBoard wait (last_count={last})"
    )


def _session_booted() -> bool:
    cp = _run([LIGH, "status", "--json"], timeout=30)
    blob = (cp.stdout or "") + (cp.stderr or "")
    brace = blob.find("{")
    if brace < 0:
        return False
    try:
        data = json.loads(blob[brace:])
        return bool(data.get("booted"))
    except json.JSONDecodeError:
        return False


def require_session_health(device: str, *, timeout_s: float = 240) -> int:
    """Hard gate — session must be booted; SpringBoard AX may be sparse on iOS 18."""
    if not _session_booted():
        raise RuntimeError("sim_boot_hung: simulator not booted after bootstrap")
    n = _ax_element_count()
    if n >= 5:
        print(json.dumps({"session_health": "ok", "ax_element_count": n}, indent=2), flush=True)
        return n
    # iOS 18 SpringBoard often returns empty AX until an app is foreground — not a host fail.
    print(
        json.dumps(
            {
                "session_health": "ok_sparse_springboard",
                "ax_element_count": n,
                "note": "motor proves chrome after run-app — do not block batch",
            },
            indent=2,
        ),
        flush=True,
    )
    return n


def bootstrap_session(device: str, work: str) -> str:
    """SessionBootstrap — lighd + plain boot + bootstatus + attach + EyesReady."""
    _run([LIGH, "daemon", "stop"], timeout=30)
    subprocess.run(["pkill", "-x", "lighd"], capture_output=True)
    time.sleep(0.4)
    log = open(os.path.join(work, "lighd.log"), "a")
    subprocess.Popen([LIGHD], stdout=log, stderr=log, start_new_session=True)
    time.sleep(1.2)

    udid = _booted_ligh_udid()
    if udid and _session_booted():
        print(json.dumps({"session_bootstrap": "attach_existing", "udid": udid}, indent=2), flush=True)
        _attach_ligh(device)
        return require_session_health(device, timeout_s=120)

    udid = _resolve_udid(device)
    for attempt, recreate in enumerate((False, True)):
        try:
            _boot_and_wait(udid, recreate=recreate)
            _attach_ligh(device)
            return require_session_health(device, timeout_s=300 if attempt else 240)
        except RuntimeError:
            if attempt == 1:
                raise
            print(json.dumps({"session_bootstrap": "retry_recreate", "udid": udid}, indent=2), flush=True)
            udid = _resolve_udid(device)
    raise RuntimeError("sim_boot_hung: bootstrap exhausted")


def recover_eyes(device: str) -> int:
    """Light EyesReady refresh — attach only, no device recreate."""
    _attach_ligh(device)
    return _wait_springboard_ax(timeout_s=120)


def ensure_daemon(device: str, work: str) -> None:
    bootstrap_session(device, work)


def _classify_error(err: str) -> tuple[str | None, str, bool]:
    """Return (fault, fault_owner, skip)."""
    low = err.lower()
    if "timed out after" in low or "build_timeout" in low:
        return "build_timeout", "build", False
    for klass in (
        "missing_watchos_runtime",
        "xcode_format_too_new",
        "swift_tools_too_new",
        "spm_resolve_failed",
        "spm_resolve_timeout",
        "not_ios_simulator",
        "disk_exhausted",
        "missing_ios_runtime",
        "acquire_not_found",
        "build_required",
        "no_xcodeproj",
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


def _doc_from_app(entry: StrangerEntry, app: str) -> dict[str, Any]:
    """Tier B sim_app — no xcodebuild, optional explicit bundle-id / source-root."""
    src = entry.source_root
    if src:
        src = os.path.abspath(src)
    else:
        src = os.path.dirname(os.path.dirname(app))
    bid = entry.bundle_id or bundle_id_from_app(app)
    return {
        "kind": "app",
        "app_path": app,
        "bundle_id": bid,
        "source_root": src,
        "workspace": src,
    }


def preflight_one(entry: StrangerEntry, *, work: str, host: Any) -> dict[str, Any]:
    """Phase A — acquire + resolve + HostCapability gate (no sim, no build)."""
    spec = entry.spec
    row: dict[str, Any] = {"ok": False, "stage": "preflight", "fault_owner": None, **entry_row_meta(entry)}
    t0 = time.time()
    try:
        if entry.acquire_mode == "app":
            app = os.path.abspath(spec)
            if not os.path.isdir(app):
                raise FileNotFoundError(f"app missing: {app}")
            name = os.path.basename(app).replace(".app", "")
            origin = app
            if entry.bundle_id or entry.source_root:
                doc = _doc_from_app(entry, app)
            else:
                doc = detect(app, build=False)
            scoped = doc.get("workspace") or doc.get("source_root") or os.path.dirname(os.path.dirname(app))
        elif entry.acquire_mode == "workspace":
            scoped = os.path.abspath(spec)
            if scoped.endswith(".xcodeproj") or scoped.endswith(".xcworkspace"):
                scoped = os.path.dirname(scoped)
            name = os.path.basename(scoped.rstrip("/"))
            origin = scoped
            doc = detect(scoped if os.path.isdir(scoped) else spec, build=False)
        else:
            scoped, name, origin = acquire(spec, work)
            doc = detect(scoped, build=False)

        row.update({"name": name, "repo": origin, "root": scoped})
        xcode = doc.get("xcode_path")
        scheme = doc.get("scheme")

        from ligh_host_capability import gate_spm_preflight_v2

        # Preflight v2: static Package.swift scan always; SPM resolve only for Tier C cold build.
        resolve = entry.tier == "C" and bool(xcode and scheme)
        skip = gate_spm_preflight_v2(
            scoped,
            host,
            xcode_path=xcode,
            scheme=scheme,
            resolve=resolve,
            work=work,
        )
        if skip:
            raise RuntimeError(f"{skip['fault']}: {skip.get('detail')}")

        if entry.tier == "B" or entry.acquire_mode == "app":
            build_out = doc.get("build_out") or os.path.join(scoped, "build", "ligh")
            app = doc.get("app_path") or find_built_app(scoped, build_out, doc.get("workspace") or scoped)
            if not app and entry.acquire_mode == "app":
                app = os.path.abspath(spec)
            if not app or not os.path.isdir(app):
                row["stage"] = "preflight_skip"
                row["fault"] = "build_required"
                row["fault_owner"] = "host"
                row["skip"] = True
                row["error"] = (
                    "Tier B verify-only — pass --app with a built .app "
                    "(and --bundle-id / --source-root if needed); LIGH does not cold-build"
                )
                return row
            doc["app_path"] = app
            doc["bundle_id"] = entry.bundle_id or bundle_id_from_app(app) or doc.get("bundle_id")
            if entry.source_root:
                doc["source_root"] = os.path.abspath(entry.source_root)
                doc["workspace"] = doc["source_root"]

        row["stage"] = "preflight_ok"
        row["scheme"] = doc.get("scheme")
        row["xcode_path"] = doc.get("xcode_path")
        row["bundle_id"] = doc.get("bundle_id")
        row["app_path"] = doc.get("app_path")
        row["accessibility_ids"] = int((doc.get("audit") or {}).get("identity_count") or 0)
        row["_preflight_doc"] = doc
        return row
    except Exception as e:  # noqa: BLE001
        err = str(e)[:600]
        row["error"] = err
        fault, owner, skip = _classify_error(err)
        row["fault"] = fault or "unknown"
        row["fault_owner"] = owner
        row["skip"] = skip
        row["stage"] = "preflight_skip" if skip else "preflight_fail"
        return row
    finally:
        row["elapsed_s"] = round(time.time() - t0, 1)


def _write_artifact(rows: list[dict[str, Any]], host: Any, out: str) -> dict[str, Any]:
    ok_rows = [r for r in rows if r.get("ok")]
    skipped = [r for r in rows if not r.get("ok") and _is_host_skip(r)]
    failed = [r for r in rows if r not in ok_rows and r not in skipped]
    runnable = [r for r in rows if r.get("stage") not in ("preflight_skip", "preflight_fail") and not r.get("skip")]
    runnable_pass = [r for r in ok_rows if r in runnable]
    n_run = len(runnable)
    n_pass = len(runnable_pass)
    tier_b = [r for r in rows if r.get("tier") == "B" and r.get("stage") not in ("preflight_skip", "preflight_fail")]
    tier_b_pass = [r for r in ok_rows if r in tier_b]
    doc = {
        "gate": "oss_stranger_trial",
        "schema": 5,
        "primary_metric": "tier_b_verify_pass",
        "ok": n_pass >= 1 and len(tier_b_pass) >= 1 if tier_b else False,
        # Competitive bar: Tier B verify volume — cold build passes do not inflate holy_shit.
        "holy_shit": len(tier_b_pass) >= 3,
        "passed": len(ok_rows),
        "passed_runnable": n_pass,
        "passed_tier_b": len(tier_b_pass),
        "runnable": n_run,
        "tier_b_runnable": len(tier_b),
        "pass_rate": f"{n_pass}/{n_run}" if n_run else "0/0",
        "tier_b_pass_rate": f"{len(tier_b_pass)}/{len(tier_b)}" if tier_b else "0/0",
        "skipped": len(skipped),
        "failed": len(failed),
        "attempted": len(rows),
        "summary": (
            f"{len(tier_b_pass)}/{len(tier_b)} tier-B verify pass · "
            f"{n_pass}/{n_run} total · {len(skipped)} skip · {len(failed)} fail "
            f"— primary_metric=tier_b_verify_pass; Tier C build≠LIGH"
        ),
        "invariants": [
            "PRIMARY KPI: tier_b_verify_pass (prebuilt .app) — not cold xcodebuild",
            "Tier A/B/C: LIGH verifies; cold xcodebuild is Tier C benchmark only",
            "Preflight v2: full Package.swift tree + SPM resolve before xcodebuild (swift_tools skip)",
            "Tier B/--app: prebuilt .app only — build_required skip, not LIGH fail",
            "motor wait-label must prove chrome before ligh_test",
            "session_gate: app_crashed / app_not_running refuse discover_no_chrome + TRAIL",
            "harness_repair on fail — never patch stranger Swift",
            "ligh_invariants enforced at discover + smoke boundaries",
            ".ligh/last-certify.json written on every ligh_test",
        ],
        "architecture": (
            "Tier A: ligh_init → ligh_test | "
            "Tier B: --app / sim_app → discover → ligh_test | "
            "Tier C: git_cold → preflight_v2(SPM) → build → verify"
        ),
        "metrics": {
            "agent_loop": "time-to-ok per agent patch (Tier A)",
            "stranger_sim_app": "tier_b_verify_pass / tier_b_runnable",
            "stranger_git_cold": "benchmark only — build_fail ≠ LIGH broken",
        },
        "fault_owners": ["host", "build", "app"],
        "host": host.to_dict() if hasattr(host, "to_dict") else host,
        "apps": [{k: v for k, v in r.items() if not k.startswith("_")} for r in rows],
        "ts": int(time.time()),
    }
    os.makedirs(os.path.dirname(out), exist_ok=True)
    json.dump(doc, open(out, "w"), indent=2)
    open(out, "a").write("\n")
    return doc


def smoke_one(
    entry: StrangerEntry,
    *,
    work: str,
    device: str,
    host: Any,
    preflight: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """One stranger — Tier B verify or Tier C build + discover + ligh_test."""
    spec = entry.spec
    if preflight and (preflight.get("skip") or preflight.get("fault") in HOST_SKIP_FAULTS):
        return preflight
    row: dict[str, Any] = dict(preflight) if preflight else {"ok": False, "stage": "acquire", **entry_row_meta(entry)}
    t0 = time.time()
    try:
        if not preflight or preflight.get("stage") != "preflight_ok":
            pf = preflight_one(entry, work=work, host=host)
            if pf.get("stage") != "preflight_ok":
                return pf
            row = dict(pf)
            doc = pf.pop("_preflight_doc", None) or detect(row["root"], build=False)
        else:
            scoped = row["root"]
            doc = preflight.pop("_preflight_doc", None) or detect(scoped, build=False)

        scoped = row["root"]
        ws = doc.get("workspace") or scoped
        build_out = doc.get("build_out") or os.path.join(scoped, "build", "ligh")

        if entry.tier == "B" or entry.acquire_mode == "app":
            app = doc.get("app_path") or find_built_app(scoped, build_out, ws)
            row["stage"] = "verify"
        else:
            row["stage"] = "detect_build"
            xcode = doc["xcode_path"]
            scheme = doc["scheme"]
            app = build_sim_app(xcode, scheme, build_out)

        bid = doc.get("bundle_id")
        src = doc.get("source_root") or scoped
        doc["app_path"] = app
        doc["bundle_id"] = bundle_id_from_app(app) or bid
        bid = doc["bundle_id"]
        if not app or not bid:
            row["error"] = "missing app_path or bundle_id"
            row["fault"] = "build_required" if entry.tier == "B" else "build_failed"
            row["fault_owner"] = "host" if entry.tier == "B" else "build"
            row["skip"] = entry.tier == "B"
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
        row["ax_element_count"] = _ax_element_count()

        row["stage"] = "discover"
        disc = discover_live(app, bid, source_root=ws, device=device)
        write_discovered_bundle(ligh_dir, doc, disc)
        row["proven_chrome"] = disc.get("proven_chrome") or disc.get("wait_hint")
        row["discover_ready"] = bool(disc.get("agent_ready"))
        row["bootstrap_ok"] = bool(disc.get("bootstrap_ok"))

        chrome = disc.get("proven_chrome")
        from ligh_invariants import validate_discovery

        inv_ok, inv_fault = validate_discovery(disc)
        if not chrome or not inv_ok:
            row["fault"] = disc.get("fault") or inv_fault or "discover_no_chrome"
            row["fault_owner"] = "app"
            if row["fault"] in ("app_crashed", "app_not_running"):
                row["error"] = disc.get("error") or (
                    "process dead — open DiagnosticReports; not discover_no_chrome"
                )
                if disc.get("process_health"):
                    row["process_health"] = disc["process_health"]
            else:
                row["error"] = "live discover did not motor-prove chrome — refusing placeholder goal"
            return row

        row["stage"] = "ligh_test"
        env = os.environ.copy()
        env["LIGH_WORKSPACE"] = ws
        env["LIGH_BIN"] = LIGH
        env["PYTHONPATH"] = os.path.join(ROOT, "scripts")
        row_name = str(row.get("name") or "app")
        test_out = os.path.join(work, f"{row_name.replace('/', '_')}-test.json")
        env["LIGH_TEST_OUT"] = test_out
        cp = _run([os.path.join(ROOT, "scripts", "ligh-test.sh")], timeout=300, env=env)
        result: dict[str, Any] = {}
        if os.path.isfile(test_out):
            result = json.load(open(test_out, encoding="utf-8"))
        else:
            result = {"ok": False, "fault": "no_test_out", "detail": (cp.stderr or cp.stdout or "")[-400:]}
        if result.get("fault") == "eyes_unusable":
            recover_eyes(device)
            disc2 = discover_live(app, bid, source_root=ws, device=device)
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
        if row["ok"]:
            from ligh_invariants import validate_smoke_row

            inv_ok, inv_fault = validate_smoke_row(row)
            if not inv_ok:
                row["ok"] = False
                row["fault"] = inv_fault or "invariant_violation"
                row["fault_owner"] = "host"
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


def _run_ligh_test_only(ws: str, *, row_name: str, work: str) -> dict[str, Any]:
    env = os.environ.copy()
    env["LIGH_WORKSPACE"] = ws
    env["LIGH_BIN"] = LIGH
    env["PYTHONPATH"] = os.path.join(ROOT, "scripts")
    test_out = os.path.join(work, f"{row_name.replace('/', '_')}-test.json")
    env["LIGH_TEST_OUT"] = test_out
    _run([os.path.join(ROOT, "scripts", "ligh-test.sh")], timeout=300, env=env)
    if os.path.isfile(test_out):
        return json.load(open(test_out, encoding="utf-8"))
    return {"ok": False, "fault": "no_test_out"}


def _is_host_skip(row: dict[str, Any]) -> bool:
    if row.get("skip"):
        return True
    fault = row.get("fault") or ""
    if fault in HOST_SKIP_FAULTS:
        return True
    err = str(row.get("error") or "")
    return any(k in err for k in HOST_SKIP_FAULTS)


def main() -> int:
    ap = argparse.ArgumentParser(description="OSS stranger smoke — Tier B verify or Tier C cold build")
    ap.add_argument("specs", nargs="*", help="URL, URL#subdir, local path, or app:/path/App.app")
    ap.add_argument("--urls-file", help="One spec per line (# comments ok)")
    ap.add_argument(
        "--app",
        metavar="PATH",
        help="Tier B sim_app — prebuilt .app (skips xcodebuild). Use with --bundle-id / --source-root.",
    )
    ap.add_argument("--bundle-id", help="Bundle ID when .app Info.plist is missing or wrong")
    ap.add_argument("--source-root", help="Swift source root for TRAIL / audit (monorepo subtree)")
    ap.add_argument("--work", default=os.environ.get("LIGH_OSS_WORK", os.path.join(ROOT, ".oss-trial")))
    ap.add_argument("--device", default=os.environ.get("LIGH_DEVICE", "iphone-15-pro"))
    ap.add_argument("--write", default=os.environ.get("LIGH_OSS_OUT", ""))
    ap.add_argument("--skip-daemon", action="store_true")
    ap.add_argument("--preflight-only", action="store_true", help="Phase A only — no sim/build/test")
    ap.add_argument(
        "--no-repair-loop",
        dest="repair_loop",
        action="store_false",
        help="Disable harness repair retry (default: on)",
    )
    ap.set_defaults(repair_loop=True)
    args = ap.parse_args()

    specs: list[str] = list(args.specs)
    if args.urls_file:
        for line in open(args.urls_file, encoding="utf-8"):
            line = line.strip()
            if line and not line.startswith("#"):
                specs.append(line)

    entries: list[StrangerEntry] = []
    if args.app:
        entries.append(
            StrangerEntry(
                raw=f"--app {args.app}",
                tier="B",
                spec=os.path.abspath(args.app),
                acquire_mode="app",
                bundle_id=args.bundle_id,
                source_root=args.source_root,
            )
        )
    for spec in specs:
        entry = parse_entry(spec)
        if entry:
            if args.bundle_id and entry.acquire_mode == "app" and not entry.bundle_id:
                entry.bundle_id = args.bundle_id
            if args.source_root and entry.acquire_mode == "app" and not entry.source_root:
                entry.source_root = args.source_root
            entries.append(entry)
    if not entries:
        ap.error("pass specs, --app, and/or --urls-file")

    if not os.path.isfile(LIGH) or not os.access(LIGH, os.X_OK):
        print(json.dumps({"ok": False, "error": f"missing LIGH_BIN={LIGH}"}), file=sys.stderr)
        return 2

    from ligh_host_capability import probe_host

    host = probe_host(ligh_bin=LIGH)
    print(json.dumps({"host": host.to_dict()}, indent=2), flush=True)
    if host.disk_free_gb < 2.0:
        print(json.dumps({"ok": False, "fault": "disk_exhausted", "host": host.to_dict()}), file=sys.stderr)
        return 2

    if not args.skip_daemon and not args.preflight_only:
        ensure_daemon(args.device, args.work)

    out = args.write or os.path.join(ROOT, "docs", "assets", "oss-stranger-trial-latest.json")
    rows: list[dict[str, Any]] = []
    runnable: list[tuple[StrangerEntry, dict[str, Any]]] = []

    for entry in entries:
        print(f"── preflight {entry.raw} ──", flush=True)
        pf = preflight_one(entry, work=args.work, host=host)
        if pf.get("skip") or pf.get("stage") == "preflight_skip":
            rows.append(pf)
            print(f"  ⊘ skip fault={pf.get('fault')}", flush=True)
            _write_artifact(rows, host, out)
            continue
        if pf.get("stage") != "preflight_ok":
            rows.append(pf)
            print(f"  ✗ preflight fault={pf.get('fault') or pf.get('error')}", flush=True)
            _write_artifact(rows, host, out)
            continue
        runnable.append((entry, pf))
        print(f"  · ok scheme={pf.get('scheme')} app={pf.get('app_path')}", flush=True)

    if args.preflight_only:
        doc = _write_artifact(rows + [r for _, r in runnable], host, out)
        print(json.dumps({"preflight_runnable": len(runnable), "out": out}, indent=2))
        return 0

    from ligh_harness_repair import harness_repair_retry

    for entry, pf in runnable:
        print(f"══ {entry.raw} ══", flush=True)
        row = smoke_one(entry, work=args.work, device=args.device, host=host, preflight=pf)
        if args.repair_loop and not row.get("ok") and not _is_host_skip(row):
            repaired = harness_repair_retry(
                row,
                work=args.work,
                device=args.device,
                run_ligh_test=_run_ligh_test_only,
                recover_eyes=recover_eyes,
            )
            if repaired:
                row = repaired
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
        _write_artifact(rows, host, out)

    doc = _write_artifact(rows, host, out)
    print(json.dumps({
        "passed": doc["passed"],
        "passed_runnable": doc.get("passed_runnable"),
        "pass_rate": doc.get("pass_rate"),
        "skipped": doc["skipped"],
        "failed": doc["failed"],
        "holy_shit": doc["holy_shit"],
        "out": out,
    }, indent=2))
    return 0 if doc.get("passed_runnable", doc["passed"]) >= 2 else 1


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Live AX discovery — Maestro inspect_screen parity, OSS-general.

Chrome trust (bulletproof):
  1. static scrape → hints only (filtered i18n / asset paths)
  2. run-app → observe live AX labels
  3. motor wait-label proves chrome — ONLY proven label becomes goal
  4. refuse agent_ready without motor proof (no REPLACE_ME, no scrape-only)
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from ligh_chrome import (  # noqa: E402
    SKIP_LABELS,
    filter_chrome_candidates,
    is_plausible_chrome,
    scrape_static_hints,
)

LIGH = os.environ.get("LIGH_BIN", os.path.join(ROOT, "target", "release", "ligh"))

PREFERRED_ROLES = ("Tab", "Heading", "Button", "StaticText", "Cell", "Link")


def _parse_json_blob(blob: str) -> dict[str, Any]:
    blob = blob.strip()
    if blob.startswith("{"):
        try:
            return json.loads(blob)
        except json.JSONDecodeError:
            pass
    for i in range(len(blob) - 1, -1, -1):
        if blob[i] != "{":
            continue
        try:
            return json.loads(blob[i:])
        except json.JSONDecodeError:
            continue
    return {"ok": False, "error": blob[-800:]}


def _ligh_json(*args: str, timeout: float = 180) -> dict[str, Any]:
    p = subprocess.run([LIGH, *args], capture_output=True, text=True, timeout=timeout)
    blob = (p.stdout or "") + (p.stderr or "")
    if not blob.strip():
        return {"ok": False, "error": f"exit {p.returncode} empty output"}
    return _parse_json_blob(blob)


def ensure_motor_ready(device: str = "iphone-15-pro") -> None:
    """Daemon attach before run-app / wait-label."""
    _ligh_json("ready", "--settle-ms", "2500", timeout=90)
    _ligh_json("up", "--gui", "--device", device, timeout=120)
    _ligh_json("ready", "--settle-ms", "3000", timeout=90)


def _surface(obs: dict[str, Any]) -> str:
    scene = obs.get("scene") if isinstance(obs.get("scene"), dict) else {}
    if not scene and isinstance(obs.get("data"), dict):
        scene = obs["data"].get("scene") or {}
    return str(scene.get("surface") or "")


def _observe_payload(raw: dict[str, Any]) -> dict[str, Any]:
    obs = raw.get("observe")
    if isinstance(obs, dict):
        return obs
    if raw.get("actionable_topk") is not None:
        return raw
    data = raw.get("data")
    return data if isinstance(data, dict) else raw


def _node_label(node: dict[str, Any]) -> str | None:
    for key in ("label", "text", "value"):
        v = node.get(key)
        if isinstance(v, str) and v.strip():
            return v.strip()
    return None


def _role_rank(role: str) -> int:
    for i, pref in enumerate(PREFERRED_ROLES):
        if pref in role:
            return i
    return len(PREFERRED_ROLES)


def labels_from_observe(obs: dict[str, Any], *, app_label: str) -> tuple[list[str], list[str], list[str]]:
    """Return (tab_labels, other_labels, ids)."""
    surface = _surface(obs)
    if surface == "springboard":
        return [], [], []

    nodes: list[dict[str, Any]] = []
    for n in obs.get("actionable_topk") or []:
        if isinstance(n, dict):
            nodes.append(n)
    tree = obs.get("accessibility_tree")
    if isinstance(tree, dict):
        for n in tree.get("nodes") or []:
            if isinstance(n, dict):
                nodes.append(n)

    nodes.sort(key=lambda n: _role_rank(str(n.get("role") or "")))

    tab_labels: list[str] = []
    labels: list[str] = []
    ids: list[str] = []
    seen: set[str] = set()

    for n in nodes:
        role = str(n.get("role") or "")
        if "Application" in role:
            continue
        lab = _node_label(n)
        ident = str(n.get("id") or n.get("identifier") or "") or None
        if ident and ident not in seen:
            seen.add(ident)
            ids.append(ident)
        if lab and is_plausible_chrome(lab) and lab not in seen and lab != app_label:
            seen.add(lab)
            if "Tab" in role:
                tab_labels.append(lab)
            else:
                labels.append(lab)

    return tab_labels, labels, ids


def _tap_label(label: str, *, settle_ms: int = 2000) -> None:
    _ligh_json("--json", "tap", "--label", label, "--timeout-ms", "12000", timeout=60)
    time.sleep(settle_ms / 1000.0)


def bootstrap_tab_chrome(
    obs: dict[str, Any],
    *,
    app_label: str,
    settle_ms: int = 2500,
    max_taps: int = 4,
) -> str | None:
    """Tab-bar apps: tap tab items, re-probe live AX — still motor-only trust."""
    tab_labels, labels, _ = labels_from_observe(obs, app_label=app_label)
    tap_targets = tab_labels or [
        lab
        for lab in labels
        if lab not in SKIP_LABELS and lab != "Tab Bar" and is_plausible_chrome(lab)
    ]
    tried: set[str] = set()
    for lab in tap_targets[:max_taps]:
        if lab in tried:
            continue
        tried.add(lab)
        _tap_label(lab, settle_ms=min(settle_ms, 1500))
        fresh = _observe_payload(_ligh_json("--json", "observe", "--settle-ms", str(settle_ms)))
        tabs, more, _ = labels_from_observe(fresh, app_label=app_label)
        candidates = filter_chrome_candidates(tabs + more + [lab])
        proven = probe_chrome(candidates, settle_ms=min(settle_ms, 2000), max_probe=8)
        if proven:
            return proven
    return None


def _scrape_roots(source_root: str | None) -> list[str]:
    roots: list[str] = []
    if source_root and os.path.isdir(source_root):
        roots.append(source_root)
        parent = os.path.dirname(source_root)
        if os.path.isdir(parent) and parent not in roots:
            roots.append(parent)
    return roots


def build_label_goal(chrome: str) -> dict[str, Any]:
    return {
        "setup": [{"op": "wait", "label": chrome, "timeout_ms": 20000}],
        "postconditions": [{"wait_label": chrome, "timeout_ms": 15000}],
    }


def probe_chrome(candidates: list[str], *, settle_ms: int = 2000, max_probe: int = 24) -> str | None:
    """Motor-prove chrome — the only trust path for goals."""
    tried: set[str] = set()
    for lab in candidates[:max_probe]:
        if not lab or lab in tried or lab in SKIP_LABELS:
            continue
        if not is_plausible_chrome(lab):
            continue
        tried.add(lab)
        r = _ligh_json(
            "--json",
            "cap",
            "wait-label",
            lab,
            "--settle-ms",
            str(settle_ms),
            "--timeout-ms",
            "12000",
            timeout=90,
        )
        if r.get("ok") or r.get("fault") == "ok":
            return lab
    return None


def discover_live(
    app: str,
    bundle_id: str,
    *,
    source_root: str | None = None,
    settle_ms: int = 3500,
    device: str = "iphone-15-pro",
) -> dict[str, Any]:
    if not os.path.isdir(app):
        return {"ok": False, "fault": "infra", "error": f"app missing: {app}"}

    ensure_motor_ready(device)

    app_label = os.path.basename(app).replace(".app", "")
    static = scrape_static_hints(_scrape_roots(source_root))

    run = _ligh_json(
        "--json",
        "cap",
        "run-app",
        app,
        "--bundle-id",
        bundle_id,
        "--settle-ms",
        str(settle_ms),
        "--timeout-ms",
        "30000",
        timeout=120,
    )
    obs = _observe_payload(run)
    boot_ok = bool(run.get("ok"))

    if not boot_ok:
        _ligh_json("run", app, timeout=120)
        time.sleep(2)
        obs = _observe_payload(_ligh_json("--json", "observe", "--settle-ms", str(settle_ms)))
        boot_ok = _surface(obs) == "app" or bool(obs.get("actionable_topk"))

    if not boot_ok:
        if is_plausible_chrome(app_label):
            _ligh_json("--json", "tap", "--label", app_label, "--timeout-ms", "12000", timeout=60)
        obs = _observe_payload(_ligh_json("--json", "observe", "--settle-ms", str(settle_ms)))

    if not obs.get("actionable_topk"):
        obs = _observe_payload(_ligh_json("--json", "observe", "--settle-ms", str(settle_ms)))

    # Crash loop → app_crashed (never discover_no_chrome / REPLACE_ME).
    ph = obs.get("process_health") if isinstance(obs.get("process_health"), dict) else {}
    if ph.get("crashed_recently") and not ph.get("running"):
        return {
            "ok": False,
            "fault": "app_crashed",
            "capability": "discover",
            "agent_ready": False,
            "readiness_grade": "F",
            "readiness_score": 0,
            "proven_chrome": None,
            "chrome_trust": "motor_only",
            "process_health": ph,
            "error": ph.get("hint")
            or "app crashed (DiagnosticReports) — not discover_no_chrome; open .ips / atos",
            "surface": _surface(obs),
        }
    if ph.get("bundle_id") and not ph.get("running") and not ph.get("crashed_recently"):
        # Still allow probe if SpringBoard icon might relaunch — but stamp fault hint.
        pass

    tab_labels, labels, ids = labels_from_observe(obs, app_label=app_label)
    all_labels = tab_labels + labels
    surface = _surface(obs)

    # Candidate order: tabs → live AX → app name → static hints (untrusted)
    candidates = filter_chrome_candidates(
        tab_labels
        + labels
        + ([app_label] if is_plausible_chrome(app_label) else [])
        + static
    )

    proven = probe_chrome(candidates, settle_ms=min(settle_ms, 2500))

    if not proven and (tab_labels or "Tab Bar" in all_labels):
        proven = bootstrap_tab_chrome(obs, app_label=app_label, settle_ms=settle_ms + 1000)

    # Bulletproof: goal only from motor proof — never scrape-only or unproven live guess
    if proven:
        goal = build_label_goal(proven)
        grade = "A" if boot_ok else "B"
        score = min(100, 80 + 3 * len(all_labels))
        agent_ready = True
        fault = None
    else:
        goal = {"setup": [], "postconditions": [{"wait_label": "REPLACE_ME", "timeout_ms": 8000}]}
        grade = "F"
        score = 0
        agent_ready = False
        # Prefer process-health faults over discover_no_chrome when dead.
        if ph.get("crashed_recently"):
            fault = "app_crashed"
        elif ph.get("bundle_id") and not ph.get("running"):
            fault = "app_not_running"
        else:
            fault = "discover_no_chrome"

    out = {
        "ok": True if agent_ready else False,
        "capability": "discover",
        "goal": goal,
        "readiness_grade": grade,
        "readiness_score": score,
        "agent_ready": agent_ready,
        "discovered_labels": all_labels[:20],
        "discovered_tab_labels": tab_labels[:12],
        "discovered_ids": ids[:20],
        "static_hints": static[:12],
        "probe_candidates": candidates[:24],
        "wait_hint": proven,
        "proven_chrome": proven,
        "bootstrap_ok": boot_ok,
        "surface": surface,
        "chrome_trust": "motor_only",
    }
    if fault:
        out["fault"] = fault
    if ph:
        out["process_health"] = ph
    if obs.get("ax_source"):
        out["ax_source"] = obs.get("ax_source")
    return out


def write_discovered_bundle(ligh_dir: str, project: dict[str, Any], discovery: dict[str, Any]) -> None:
    from ligh_invariants import sanitize_goal_for_write

    os.makedirs(ligh_dir, exist_ok=True)
    goal = sanitize_goal_for_write(discovery)
    json.dump(goal, open(os.path.join(ligh_dir, "app-goal.json"), "w"), indent=2)
    json.dump(discovery, open(os.path.join(ligh_dir, "discovery.json"), "w"), indent=2)
    proj = dict(project)
    audit = dict(proj.get("audit") or {})
    audit.update(
        {
            "live_discovery": True,
            "readiness_grade": discovery.get("readiness_grade"),
            "readiness_score": discovery.get("readiness_score"),
            "agent_ready": discovery.get("agent_ready"),
            "discovered_labels": discovery.get("discovered_labels"),
            "proven_chrome": discovery.get("proven_chrome"),
            "chrome_trust": discovery.get("chrome_trust"),
        }
    )
    proj["audit"] = audit
    proj["suggested_app_goal"] = goal
    json.dump(proj, open(os.path.join(ligh_dir, "project.json"), "w"), indent=2)


def main() -> int:
    import argparse

    ap = argparse.ArgumentParser()
    ap.add_argument("--app", required=True)
    ap.add_argument("--bundle-id", required=True)
    ap.add_argument("--source-root")
    ap.add_argument("--write")
    ap.add_argument("--project-json")
    ap.add_argument("--device", default="iphone-15-pro")
    args = ap.parse_args()

    disc = discover_live(
        args.app, args.bundle_id, source_root=args.source_root, device=args.device
    )
    print(json.dumps(disc, indent=2))
    if args.write and disc.get("ok"):
        proj = {}
        if args.project_json and os.path.isfile(args.project_json):
            proj = json.load(open(args.project_json, encoding="utf-8"))
        write_discovered_bundle(args.write, proj, disc)
    return 0 if disc.get("agent_ready") and disc.get("proven_chrome") else 1


if __name__ == "__main__":
    raise SystemExit(main())

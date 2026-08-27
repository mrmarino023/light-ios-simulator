#!/usr/bin/env python3
"""Live AX discovery — Maestro inspect_screen parity, OSS-general.

Architecture (no per-app lists):
  1. scrape static chrome (navigationTitle > Text)
  2. run-app with first hint (daemon owns SpringBoard → foreground)
  3. observe in-app labels when surface ≠ springboard
  4. probe wait-label across candidates until motor proves chrome
  5. write label-first app-goal from the proven chrome
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

LIGH = os.environ.get("LIGH_BIN", os.path.join(ROOT, "target", "release", "ligh"))

SKIP_LABELS = frozenset({"", "Back", "Cancel", "Done", "Close", "Tab Bar", "SpringBoard"})
NAV_TITLE_RE = re.compile(r'\.navigationTitle\s*\(\s*"([^"]{2,60})"')
TEXT_LABEL_RE = re.compile(r'Text\s*\(\s*"([^"]{2,60})"')
INTERP_RE = re.compile(r"\\[\($]")


def _ligh_json(*args: str, timeout: float = 180) -> dict[str, Any]:
    p = subprocess.run([LIGH, *args], capture_output=True, text=True, timeout=timeout)
    for blob in ((p.stdout or "").strip(), (p.stderr or "").strip()):
        if blob.startswith("{"):
            try:
                return json.loads(blob)
            except json.JSONDecodeError:
                pass
    return {"ok": False, "error": (p.stderr or p.stdout or f"exit {p.returncode}")[:800]}


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


def labels_from_observe(obs: dict[str, Any], *, app_label: str) -> tuple[list[str], list[str]]:
    """Pull labels/ids from observe. Prefer non-springboard; still collect actionable if mixed."""
    surface = _surface(obs)
    labels: list[str] = []
    ids: list[str] = []
    seen: set[str] = set()

    def add(label: str | None, ident: str | None) -> None:
        if ident and ident not in seen:
            seen.add(ident)
            ids.append(ident)
        if not label or label in seen or label in SKIP_LABELS:
            return
        if label == app_label and surface in ("springboard", "transition"):
            return
        seen.add(label)
        labels.append(label)

    nodes = list(obs.get("actionable_topk") or [])
    for n in (obs.get("accessibility_tree") or {}).get("nodes") or []:
        if isinstance(n, dict):
            nodes.append(n)

    for n in nodes:
        if not isinstance(n, dict):
            continue
        role = str(n.get("role") or "")
        if "Application" in role:
            continue
        add(_node_label(n), str(n.get("id") or n.get("identifier") or "") or None)

    if surface == "springboard":
        # Home grid — discard; only keep intersection later via probe.
        return [], []
    return labels, ids


def scrape_static_chrome(source_root: str | None, limit: int = 16) -> list[str]:
    if not source_root or not os.path.isdir(source_root):
        return []
    nav: list[str] = []
    other: list[str] = []
    seen: set[str] = set()

    def add(lab: str, *, is_nav: bool) -> None:
        lab = lab.strip()
        if not lab or INTERP_RE.search(lab) or lab in seen or lab in SKIP_LABELS:
            return
        if len(lab) < 2 or lab.startswith("$"):
            return
        seen.add(lab)
        (nav if is_nav else other).append(lab)

    for dirpath, _, files in os.walk(source_root):
        if any(x in dirpath for x in ("/build/", "/DerivedData/", "/.git/", "/Pods/", "/Packages/")):
            continue
        for name in files:
            if not name.endswith(".swift"):
                continue
            try:
                text = open(os.path.join(dirpath, name), encoding="utf-8").read()
            except OSError:
                continue
            for m in NAV_TITLE_RE.finditer(text):
                add(m.group(1), is_nav=True)
            for m in TEXT_LABEL_RE.finditer(text):
                add(m.group(1), is_nav=False)
            if len(nav) + len(other) >= limit * 2:
                break
    return (nav + other)[:limit]


def build_label_goal(chrome: str) -> dict[str, Any]:
    return {
        "setup": [{"op": "wait", "label": chrome, "timeout_ms": 20000}],
        "postconditions": [{"wait_label": chrome, "timeout_ms": 15000}],
    }


def probe_chrome(candidates: list[str], *, settle_ms: int = 2000) -> str | None:
    """Motor-prove which label is on screen — Maestro inspect_screen equivalent."""
    tried: set[str] = set()
    for lab in candidates:
        if not lab or lab in tried or lab in SKIP_LABELS:
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
            "10000",
            timeout=60,
        )
        if r.get("ok"):
            return lab
    return None


def discover_live(
    app: str,
    bundle_id: str,
    *,
    source_root: str | None = None,
    settle_ms: int = 3500,
) -> dict[str, Any]:
    if not os.path.isdir(app):
        return {"ok": False, "fault": "infra", "error": f"app missing: {app}"}

    app_label = os.path.basename(app).replace(".app", "")
    static = scrape_static_chrome(source_root)
    wait_hint = static[0] if static else None

    cmd = [
        "--json",
        "cap",
        "run-app",
        app,
        "--bundle-id",
        bundle_id,
        "--settle-ms",
        str(settle_ms),
        "--timeout-ms",
        "25000",
    ]
    if wait_hint:
        cmd.extend(["--wait-label", wait_hint])

    run = _ligh_json(*cmd)
    obs = _observe_payload(run)
    boot_ok = bool(run.get("ok"))

    # If launch left us on home, tap the app icon once — same recovery daemon uses, but
    # discover must own it when run-app returns before chrome is proven.
    if not boot_ok:
        _ligh_json("--json", "tap", "--label", app_label, "--timeout-ms", "12000", timeout=60)
        obs = _observe_payload(_ligh_json("--json", "observe", "--settle-ms", str(settle_ms)))

    if not obs.get("actionable_topk"):
        obs = _observe_payload(_ligh_json("--json", "observe", "--settle-ms", str(settle_ms)))

    labels, ids = labels_from_observe(obs, app_label=app_label)
    surface = _surface(obs)

    # Candidate order: live labels → app display name → static chrome
    candidates: list[str] = []
    for lab in labels + [app_label] + static:
        if lab and lab not in candidates and lab not in SKIP_LABELS:
            candidates.append(lab)

    # Also harvest actionable labels even on ambiguous surfaces (probe will reject misses).
    for n in obs.get("actionable_topk") or []:
        if isinstance(n, dict):
            lab = _node_label(n)
            if lab and lab not in candidates and lab not in SKIP_LABELS:
                candidates.insert(0, lab)

    proven = None
    if boot_ok and wait_hint:
        proven = wait_hint
    if not proven:
        proven = probe_chrome(candidates[:12], settle_ms=min(settle_ms, 2500))

    chrome = proven or (labels[0] if labels else None)
    goal = (
        build_label_goal(chrome)
        if chrome
        else {"setup": [], "postconditions": [{"wait_label": "REPLACE_ME", "timeout_ms": 8000}]}
    )

    if proven:
        grade = "A" if boot_ok or labels else "B"
        score = 75 + 5 * len(labels)
        agent_ready = True
    elif labels:
        grade = "C"
        score = 50
        agent_ready = True
    else:
        grade = "F"
        score = 0
        agent_ready = False

    return {
        "ok": True,
        "capability": "discover",
        "goal": goal,
        "readiness_grade": grade,
        "readiness_score": min(100, score),
        "agent_ready": agent_ready,
        "discovered_labels": labels[:20],
        "discovered_ids": ids[:20],
        "static_hints": static[:12],
        "wait_hint": chrome,
        "proven_chrome": proven,
        "bootstrap_ok": boot_ok,
        "surface": surface,
    }


def write_discovered_bundle(ligh_dir: str, project: dict[str, Any], discovery: dict[str, Any]) -> None:
    os.makedirs(ligh_dir, exist_ok=True)
    goal = discovery.get("goal") or {}
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
    args = ap.parse_args()

    disc = discover_live(args.app, args.bundle_id, source_root=args.source_root)
    print(json.dumps(disc, indent=2))
    if args.write and disc.get("ok"):
        proj = {}
        if args.project_json and os.path.isfile(args.project_json):
            proj = json.load(open(args.project_json, encoding="utf-8"))
        write_discovered_bundle(args.write, proj, disc)
    return 0 if disc.get("agent_ready") else 1


if __name__ == "__main__":
    raise SystemExit(main())

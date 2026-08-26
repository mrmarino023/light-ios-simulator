#!/usr/bin/env python3
"""Audit Swift sources for agent-ready accessibility identifiers."""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from typing import Any

sys.path.insert(0, os.path.dirname(__file__))
from identity_index import build_identity_index  # noqa: E402

INTERACTIVE_RE = re.compile(
    r"\b(Button|TextField|SecureField|Toggle|Picker|NavigationLink|Link)\s*\("
    r"|\.tabItem\s*\{"
)
IDENT_RE = re.compile(
    r'\.accessibilityIdentifier\s*\(\s*"([^"]+)"\s*\)'
    r'|accessibilityIdentifier\s*:\s*"([^"]+)"'
)
LABEL_RE = re.compile(r'Label\s*\(\s*"([^"]+)"')
BUTTON_TEXT_RE = re.compile(r'Button\s*\(\s*"([^"]+)"')


def _walk_swift(source_root: str) -> list[tuple[str, list[str]]]:
    out: list[tuple[str, list[str]]] = []
    for dirpath, _, files in os.walk(source_root):
        if any(s in dirpath for s in ("/build/", "/DerivedData/", "/.git/", "/Pods/")):
            continue
        for name in files:
            if not name.endswith(".swift"):
                continue
            path = os.path.join(dirpath, name)
            try:
                with open(path, encoding="utf-8") as f:
                    lines = f.readlines()
            except OSError:
                continue
            rel = os.path.relpath(path, source_root).replace("\\", "/")
            out.append((rel, lines))
    return out


def _block_has_identifier(lines: list[str], idx: int, *, window: int = 6) -> str | None:
    for m in IDENT_RE.finditer(lines[idx]):
        ident = m.group(1) or m.group(2)
        if ident:
            return ident
    for j in range(idx + 1, min(len(lines), idx + window)):
        line = lines[j].strip()
        if not line or line.startswith("//"):
            continue
        if line.startswith("}") or re.match(r"^[A-Z]\w*\(", line):
            break
        for m in IDENT_RE.finditer(lines[j]):
            ident = m.group(1) or m.group(2)
            if ident:
                return ident
    return None


def audit_source_root(source_root: str) -> dict[str, Any]:
    source_root = os.path.abspath(source_root)
    index = build_identity_index(source_root)
    identities = sorted(index.keys())
    interactive_sites: list[dict[str, Any]] = []
    missing: list[dict[str, Any]] = []

    for rel, lines in _walk_swift(source_root):
        for i, line in enumerate(lines):
            if not INTERACTIVE_RE.search(line):
                continue
            ident = _block_has_identifier(lines, i)
            label_m = LABEL_RE.search(line) or BUTTON_TEXT_RE.search(line)
            site = {
                "file": rel,
                "line": i + 1,
                "snippet": line.strip()[:120],
                "label_hint": label_m.group(1) if label_m else None,
                "identity": ident,
            }
            interactive_sites.append(site)
            if not ident:
                missing.append(site)

    total = len(interactive_sites)
    identified = total - len(missing)
    score = int(round(100 * identified / total)) if total else 100
    grade = "A" if score >= 80 else "B" if score >= 50 else "C" if score >= 25 else "F"

    return {
        "source_root": source_root,
        "identity_count": len(identities),
        "identities": identities[:64],
        "interactive_count": total,
        "identified_interactive": identified,
        "missing_interactive": len(missing),
        "readiness_score": score,
        "readiness_grade": grade,
        "missing_sites": missing[:24],
        "agent_ready": score >= 50 and len(identities) >= 3,
    }


def suggest_app_job_steps(audit: dict[str, Any]) -> list[dict[str, Any]]:
    """Suggest cap app-job steps from indexed identities (goal-first, not tap soup)."""
    ids = list(audit.get("identities") or [])
    if not ids:
        return [{"op": "wait", "id": "REPLACE_FIRST_SCREEN_ID"}]

    def pick(pred) -> str | None:
        for ident in ids:
            if pred(ident):
                return ident
        return None

    steps: list[dict[str, Any]] = []
    home = pick(lambda x: x in ("homeTitle", "LighHome", "tab_home") or x.endswith("_home"))
    login_btn = pick(lambda x: "login" in x.lower() and "button" in x.lower())
    email = pick(lambda x: "email" in x.lower() or "username" in x.lower())
    password = pick(
        lambda x: "password" in x.lower() or x.endswith("SecureField") or "pass" in x.lower()
    )
    first_tab = pick(lambda x: x.startswith("tab_"))

    entry = home or pick(lambda x: not x.startswith("tab_")) or ids[0]
    steps.append({"op": "wait", "id": entry})

    if email:
        steps.append({"op": "type", "id": email, "text": "test@example.com"})
    if password:
        steps.append({"op": "type", "id": password, "text": "password"})
    if login_btn:
        steps.append({"op": "tap", "id": login_btn})
        done = pick(lambda x: x in ("homeTitle", "LighDone") or "home" in x.lower())
        if done:
            steps.append({"op": "wait", "id": done})
    elif first_tab:
        steps.append({"op": "tap", "id": first_tab})

    if len(steps) == 1:
        second = ids[1] if len(ids) > 1 else "REPLACE_NEXT_ID"
        steps.append({"op": "wait", "id": second})
    return steps


def suggest_verification(audit: dict[str, Any], steps: list[dict[str, Any]]) -> dict[str, Any]:
    ids = set(audit.get("identities") or [])
    last_wait = next((s["id"] for s in reversed(steps) if s.get("op") == "wait" and s.get("id")), None)
    must_see = [last_wait] if last_wait else []
    bootstrap = pick_bootstrap_label(audit)
    pre = [bootstrap] if bootstrap else []
    exercise = []
    for s in steps:
        if s["op"] == "wait":
            exercise.append({"action": "assert", "id": s["id"], "settle_ms": 1500})
        elif s["op"] == "type":
            exercise.append(
                {
                    "action": "type",
                    "id": s["id"],
                    "text": s.get("text", ""),
                    "settle_ms": 1500,
                }
            )
        elif s["op"] == "tap":
            exercise.append({"action": "tap", "id": s["id"], "settle_ms": 2500})
    return {
        "bootstrap_wait_label": bootstrap,
        "preconditions": {"must_see_labels": pre, "must_not_see_labels": []},
        "exercise": exercise,
        "postconditions": {"must_see_labels": must_see, "must_not_see_labels": []},
    }


def pick_bootstrap_label(audit: dict[str, Any]) -> str | None:
    for site in audit.get("missing_sites") or []:
        if site.get("label_hint"):
            return site["label_hint"]
    ids = audit.get("identities") or []
    for ident in ids:
        if "welcome" in ident.lower() or "login" in ident.lower():
            return None
    return None


def main() -> int:
    ap = argparse.ArgumentParser(description="Audit Swift accessibility for LIGH agents")
    ap.add_argument("source_root", help="App Swift source root")
    ap.add_argument("--json", action="store_true", help="Print JSON only")
    ap.add_argument("--suggest-steps", action="store_true")
    args = ap.parse_args()
    audit = audit_source_root(args.source_root)
    if args.suggest_steps:
        steps = suggest_app_job_steps(audit)
        audit["suggested_app_job"] = steps
        audit["suggested_verification"] = suggest_verification(audit, steps)
    if args.json:
        print(json.dumps(audit, indent=2))
    else:
        print(f"readiness: {audit['readiness_grade']} ({audit['readiness_score']}%)")
        print(f"identities: {audit['identity_count']} · interactive: {audit['interactive_count']}")
        print(f"agent_ready: {audit['agent_ready']}")
        if audit.get("missing_sites"):
            print("missing ids (sample):")
            for m in audit["missing_sites"][:8]:
                print(f"  {m['file']}:{m['line']}  {m['snippet'][:80]}")
    return 0 if audit["agent_ready"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

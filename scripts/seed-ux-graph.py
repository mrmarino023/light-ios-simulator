#!/usr/bin/env python3
"""Seed UX graph via QA attempts (no LLM) — workflow matrix steps → graph edges."""

from __future__ import annotations

import json
import os
import sys
import time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from ligh_mcp import call_tool  # noqa: E402

WORKFLOWS = {
    "lighonboard": {
        "app": os.path.join(ROOT, "fixtures/LighOnboard/build/LighOnboard.app"),
        "bundle_id": "dev.ligh.Onboard",
        "workspace": os.path.join(ROOT, "fixtures/LighOnboard"),
        "success_id": "HomeReady",
        "in_app_markers": ["OnboardWelcome", "OnboardSkip", "HomeReady"],
        "steps": [
            ("wait", {"id": "OnboardWelcome"}, None),
            ("tap", {"id": "OnboardSkip"}, {"see_id": "HomeReady"}),
        ],
    },
    "xcuitestdemo": {
        "app": os.path.join(ROOT, "fixtures/third-party/XCUITestDemo/build/XCUITestDemo.app"),
        "bundle_id": "com.himali.XCUITestDemo",
        "workspace": os.path.join(ROOT, "fixtures/third-party/XCUITestDemo"),
        "success_id": "homeTitle",
        "in_app_markers": ["usernameTextField", "loginButton", "homeTitle"],
        "steps": [
            ("wait", {"id": "usernameTextField"}, None),
            ("tap", {"id": "usernameTextField"}, None),
            ("type", {"text": "alice", "id": "usernameTextField"}, None),
            ("tap", {"id": "passwordSecureField"}, None),
            ("type", {"text": "secret", "id": "passwordSecureField"}, None),
            ("dismiss", {}, None),
            ("tap", {"id": "loginButton"}, {"see_id": "homeTitle"}),
        ],
    },
}


def attempt(intent: str, args: dict, expect: dict | None, ws: str) -> dict:
    payload: dict = {
        "intent": intent if intent != "dismiss" else "tap",
        "settle_ms": 2500,
        "timeout_ms": 14000,
        "workspace": ws,
        **{k: v for k, v in args.items() if k != "op"},
    }
    if intent == "dismiss":
        return call_tool("ligh_dismiss", {"settle_ms": 2500})
    if intent == "wait":
        payload["intent"] = "tap"
        payload["timeout_ms"] = 8000
    if expect:
        payload["expect"] = expect
    return call_tool("ligh_attempt", payload)


def affordance_keys(perceive: dict) -> set[str]:
    keys: set[str] = set()
    for a in perceive.get("affordances") or []:
        if not isinstance(a, dict):
            continue
        for k in ("id", "label", "identifier"):
            if a.get(k):
                keys.add(str(a[k]))
    loc = perceive.get("location") or {}
    if loc.get("title"):
        keys.add(str(loc["title"]))
    return keys


def on_springboard(perceive: dict, app_label: str) -> bool:
    for a in perceive.get("affordances") or []:
        if not isinstance(a, dict):
            continue
        ident = a.get("identifier") or a.get("label") or ""
        if ident == app_label and a.get("hittable", True):
            return True
    return False


def foreground_ready(perceive: dict, bundle_id: str, app_label: str, markers: list[str]) -> bool:
    keys = affordance_keys(perceive)
    if markers and any(m in keys for m in markers):
        return True
    if on_springboard(perceive, app_label):
        return False
    loc = perceive.get("location") or {}
    return loc.get("bundle_id") == bundle_id and loc.get("surface") == "app"


def bootstrap_app(cfg: dict) -> dict:
    call_tool("ligh_ready", {"settle_ms": 2500, "recover_homes": 4})
    boot = call_tool(
        "ligh_cap_run_app",
        {
            "app": cfg["app"],
            "bundle_id": cfg["bundle_id"],
            "settle_ms": 3500,
            "timeout_ms": 15000,
        },
    )
    ws = cfg["workspace"]
    bid = cfg["bundle_id"]
    app_label = os.path.basename(cfg["app"]).replace(".app", "")
    markers = cfg.get("in_app_markers") or []
    for attempt in range(1, 6):
        p = call_tool("ligh_perceive", {"settle_ms": 2500, "workspace": ws})
        perceive = p.get("perceive") or {}
        if foreground_ready(perceive, bid, app_label, markers):
            return {**boot, "foreground_attempt": attempt, "foreground_ok": True}
        call_tool("ligh_launch", {"bundle_id": bid})
        time.sleep(1.2)
        call_tool(
            "ligh_attempt",
            {
                "intent": "tap",
                "label": app_label,
                "settle_ms": 2000,
                "timeout_ms": 8000,
                "workspace": ws,
            },
        )
        time.sleep(1.0)
    return {**boot, "foreground_ok": False}


def main() -> int:
    app_id = os.environ.get("LIGH_SEED_APP", "lighonboard")
    cfg = WORKFLOWS.get(app_id)
    if not cfg:
        print(json.dumps({"ok": False, "error": f"unknown app {app_id}"}), file=sys.stderr)
        return 1

    t0 = time.time()
    ws = cfg["workspace"]
    os.makedirs(os.path.join(ws, ".ligh"), exist_ok=True)
    for f in os.listdir(os.path.join(ws, ".ligh")):
        if f.endswith(".json"):
            os.remove(os.path.join(ws, ".ligh", f))

    bootstrap = bootstrap_app(cfg)
    if not bootstrap.get("foreground_ok", True):
        print(json.dumps({"ok": False, "error": "app never foregrounded", "bootstrap": bootstrap}), file=sys.stderr)
        return 1

    trace = [{"op": "bootstrap", "ok": bootstrap.get("foreground_ok")}]
    for intent, args, expect in cfg["steps"]:
        if intent == "wait":
            r = call_tool(
                "ligh_perceive",
                {"settle_ms": 2000, "workspace": ws},
            )
            trace.append({"op": "wait", "ok": r.get("ok")})
            continue
        r = attempt(intent, args, expect, ws)
        trace.append({"op": intent, "args": args, "expect": expect, "ok": r.get("ok"), "intent_met": r.get("intent_met")})
        if intent == "tap" and expect and not r.get("intent_met", r.get("ok")):
            break

    verify = call_tool("ligh_perceive", {"settle_ms": 2500, "workspace": ws})
    perceive = verify.get("perceive") or {}
    ids = {a.get("id") for a in perceive.get("affordances") or [] if isinstance(a, dict)}
    ids |= {a.get("label") for a in perceive.get("affordances") or [] if isinstance(a, dict)}
    verified = cfg["success_id"] in ids
    ux = call_tool("ligh_ux_status", {"workspace": ws})
    summary = (ux.get("detail") or {}).get("summary") or {}

    doc = {
        "gate": "ux_graph_seed",
        "app_id": app_id,
        "verified": verified,
        "success_id": cfg["success_id"],
        "nodes": summary.get("node_count"),
        "edges": summary.get("edge_count"),
        "llm_tokens": 0,
        "total_ms": int((time.time() - t0) * 1000),
        "bootstrap": bootstrap,
        "trace": trace,
    }
    out = os.environ.get(
        "LIGH_SEED_OUT",
        os.path.join(ROOT, "docs/assets/ux-graph-prove-traces/seed.json"),
    )
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w", encoding="utf-8") as f:
        f.write(json.dumps(doc, indent=2) + "\n")
    print(json.dumps({"ok": verified, "nodes": doc["nodes"], "edges": doc["edges"], "out": out}))
    return 0 if verified and (doc["nodes"] or 0) >= 2 else 1


if __name__ == "__main__":
    raise SystemExit(main())

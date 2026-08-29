#!/usr/bin/env python3
"""Harness repair — OSS paradise auto-generalize loop.

When stranger smoke fails, repair the **host pipeline** (motor, discover, session)
— NEVER edit vendored app Swift. Same contract as TRAIL but for ligh_* scripts.

  prove fault → classify harness mode → structural retry → re-certify (ligh_test)

App-level bugs (login gate, missing tab) → TRAIL / repair_engine on source_root.
Pipeline bugs (chrome trust, eyes, daemon) → this module.
"""

from __future__ import annotations

import json
import os
import time
from typing import Any, Callable

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))

HOST_REPAIR_FAULTS = frozenset(
    {
        "discover_no_chrome",
        "target_missing",
        "eyes_unusable",
        "motor_no_effect",
        "timeout",
        "ligh_test_failed",
        "sim_boot_hung",
    }
)

# App/session faults — never "repair" the harness as if chrome was missing.
APP_SESSION_FAULTS = frozenset({"app_crashed", "app_not_running"})


def classify_harness_fault(row: dict[str, Any]) -> str | None:
    """Return repair mode or None if not harness-repairable."""
    fault = str(row.get("fault") or "")
    if row.get("ok"):
        return None
    if fault in APP_SESSION_FAULTS:
        return None  # open .ips / relaunch — not discover_extended
    if row.get("trail_allowed") is False and fault in APP_SESSION_FAULTS:
        return None
    if fault not in HOST_REPAIR_FAULTS and fault not in ("unknown", "ok"):
        lt = (row.get("ligh_test") or {}).get("fault")
        if lt in APP_SESSION_FAULTS:
            return None
        if lt in HOST_REPAIR_FAULTS:
            fault = lt
        else:
            return None

    if fault in ("eyes_unusable", "sim_boot_hung"):
        return "motor_recover_full"
    if fault == "discover_no_chrome":
        # Prefer process-health stamped on the row.
        ph = row.get("process_health") if isinstance(row.get("process_health"), dict) else {}
        if ph.get("crashed_recently") or (ph.get("bundle_id") and not ph.get("running")):
            return None
        return "discover_extended"
    if fault == "target_missing":
        try:
            from ligh_chrome import is_plausible_chrome

            chrome = row.get("proven_chrome") or ""
            if chrome and not is_plausible_chrome(chrome):
                return "chrome_invalidate_rediscover"
        except ImportError:
            pass
        return "discover_extended"
    if fault in ("motor_no_effect", "timeout", "ligh_test_failed", "unknown"):
        if row.get("proven_chrome"):
            return "test_retry"
        return "discover_extended"
    return None


def log_harness_repair(work: str, row: dict[str, Any], mode: str, outcome: dict[str, Any]) -> None:
    os.makedirs(work, exist_ok=True)
    entry = {
        "ts": int(time.time()),
        "spec": row.get("spec"),
        "repo": row.get("repo"),
        "mode": mode,
        "fault_in": row.get("fault"),
        "chrome_in": row.get("proven_chrome"),
        "outcome_ok": outcome.get("ok"),
        "outcome_fault": outcome.get("fault"),
        "owner": "host",
        "rule": "never patch stranger Swift — harness/pipeline only",
    }
    with open(os.path.join(work, "harness-repairs.jsonl"), "a", encoding="utf-8") as f:
        f.write(json.dumps(entry) + "\n")


def harness_repair_retry(
    row: dict[str, Any],
    *,
    work: str,
    device: str,
    run_ligh_test: Callable[..., dict[str, Any]],
    recover_eyes: Callable[[str], int],
) -> dict[str, Any] | None:
    """One harness repair attempt. Returns updated row if retried, else None."""
    mode = classify_harness_fault(row)
    if not mode:
        return None

    app = row.get("app_path")
    bid = row.get("bundle_id")
    ws = row.get("root")
    if not app or not bid or not ws:
        return None

    print(f"  ↻ harness_repair mode={mode} fault={row.get('fault')}", flush=True)

    if mode in ("motor_recover_full", "discover_extended", "chrome_invalidate_rediscover", "test_retry"):
        recover_eyes(device)

    out = dict(row)
    out["harness_repair"] = mode

    if mode in ("motor_recover_full", "discover_extended", "chrome_invalidate_rediscover"):
        from ligh_discover import discover_live, write_discovered_bundle

        settle = 5000 if mode == "discover_extended" else 4000
        disc = discover_live(app, bid, source_root=ws, device=device, settle_ms=settle)
        ligh_dir = os.path.join(ws, ".ligh")
        if os.path.isdir(ligh_dir):
            pj = os.path.join(ligh_dir, "project.json")
            proj = json.load(open(pj, encoding="utf-8")) if os.path.isfile(pj) else {}
            write_discovered_bundle(ligh_dir, proj, disc)
        out["proven_chrome"] = disc.get("proven_chrome")
        out["discover_ready"] = bool(disc.get("agent_ready"))
        out["bootstrap_ok"] = bool(disc.get("bootstrap_ok"))
        if not disc.get("proven_chrome"):
            out["fault"] = "discover_no_chrome"
            out["fault_owner"] = "app"
            log_harness_repair(work, row, mode, out)
            return out

    if mode in ("motor_recover_full", "discover_extended", "chrome_invalidate_rediscover", "test_retry"):
        test = run_ligh_test(ws, row_name=str(row.get("name") or "app"), work=work)
        out["ligh_test"] = {
            "ok": bool(test.get("ok")),
            "fault": test.get("fault"),
            "mode": test.get("mode"),
        }
        out["ok"] = bool(test.get("ok"))
        out["fault"] = test.get("fault") if not out["ok"] else "ok"
        out["fault_owner"] = "host" if test.get("fault") in ("eyes_unusable", "sim_boot_hung") else "app"
        out["stage"] = "done" if out["ok"] else "ligh_test"
        log_harness_repair(work, row, mode, out)
        return out

    return None

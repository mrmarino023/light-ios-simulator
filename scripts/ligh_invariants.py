#!/usr/bin/env python3
"""LIGH bulletproof invariants — enforced at discover, smoke, and test boundaries.

These are product contracts, not documentation. Fail-closed when violated.
"""

from __future__ import annotations

from typing import Any

from ligh_chrome import is_plausible_chrome

REPLACE_ME = "REPLACE_ME"

INVARIANT_IDS = (
    "motor_chrome_required",
    "no_replace_me_goal",
    "chrome_plausible",
    "host_skip_not_fail",
    "harness_never_patches_stranger_swift",
)


def goal_wait_label(goal: dict[str, Any] | None) -> str | None:
    if not isinstance(goal, dict):
        return None
    for step in goal.get("setup") or []:
        if isinstance(step, dict) and step.get("op") == "wait" and step.get("label"):
            return str(step["label"]).strip()
    for cond in goal.get("postconditions") or []:
        if isinstance(cond, dict) and cond.get("wait_label"):
            return str(cond["wait_label"]).strip()
    return None


def validate_goal(goal: dict[str, Any] | None, *, proven_chrome: str | None = None) -> tuple[bool, str | None]:
    """Return (ok, fault). Goal must be motor-proven — no placeholders."""
    label = proven_chrome or goal_wait_label(goal)
    if not label:
        return False, "discover_no_chrome"
    if label == REPLACE_ME or REPLACE_ME in str(goal):
        return False, "discover_no_chrome"
    if not is_plausible_chrome(label):
        return False, "chrome_untrusted"
    return True, None


def validate_discovery(discovery: dict[str, Any]) -> tuple[bool, str | None]:
    proven = discovery.get("proven_chrome") or discovery.get("wait_hint")
    if not discovery.get("agent_ready"):
        return False, "discover_no_chrome"
    if discovery.get("chrome_trust") != "motor_only":
        return False, "chrome_trust_violation"
    ok, fault = validate_goal(discovery.get("goal"), proven_chrome=proven)
    if not ok:
        return False, fault
    if not proven:
        return False, "discover_no_chrome"
    return True, None


def validate_smoke_row(row: dict[str, Any]) -> tuple[bool, str | None]:
    """Before claiming ok:true on a smoke row."""
    if not row.get("ok"):
        return True, None
    proven = row.get("proven_chrome")
    ok, fault = validate_goal(None, proven_chrome=proven)
    if not ok:
        return False, fault
    lt = row.get("ligh_test") or {}
    if not lt.get("ok"):
        return False, lt.get("fault") or "ligh_test_failed"
    return True, None


def assert_goal_for_test(goal: dict[str, Any], *, context: str = "ligh_test") -> None:
    ok, fault = validate_goal(goal)
    if not ok:
        raise RuntimeError(f"{context}: invariant violated ({fault}) — refusing test")


def sanitize_goal_for_write(discovery: dict[str, Any]) -> dict[str, Any]:
    """Never write REPLACE_ME to production app-goal.json."""
    proven = discovery.get("proven_chrome")
    ok, _ = validate_discovery(discovery)
    if ok and proven:
        from ligh_discover import build_label_goal

        return build_label_goal(proven)
    return {"setup": [], "postconditions": []}


HOST_SKIP_FAULTS = frozenset(
    {
        "missing_watchos_runtime",
        "xcode_format_too_new",
        "swift_tools_too_new",
        "disk_exhausted",
        "missing_ios_runtime",
        "acquire_not_found",
        "not_ios_simulator",
    }
)


def fault_owner(fault: str | None, *, skip: bool = False) -> str:
    if skip or fault in HOST_SKIP_FAULTS:
        return "host"
    if fault in ("build_failed", "build_timeout", "codesign"):
        return "build"
    if fault in ("eyes_unusable", "sim_boot_hung", "infra"):
        return "host"
    if fault in ("app_crashed", "app_not_running"):
        return "app"
    if fault in ("discover_no_chrome", "chrome_untrusted", "target_missing", "motor_no_effect"):
        return "app"
    return "unknown"

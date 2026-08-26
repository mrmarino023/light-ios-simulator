#!/usr/bin/env python3
"""Effect classifier — RepairMode from TraceFailure v2 (OSS-general, no task ids).

Bulletproof rule: classify *before* localize. Refuse unknown (no speculative edits).
"""

from __future__ import annotations

from typing import Any


def _norm_ids(xs: Any) -> list[str]:
    out: list[str] = []
    for x in xs or []:
        s = str(x).strip()
        if s:
            out.append(s)
    return out


def enrich_trace_failure(
    tf: dict[str, Any],
    *,
    keys_after: set[str] | list[str] | None = None,
    acceptance_pending: list[str] | None = None,
    sig_before: str | None = None,
    sig_after: str | None = None,
) -> dict[str, Any]:
    """Fill TraceFailure v2 fields from post-act observation."""
    out = dict(tf)
    control = str(out.get("control") or out.get("expected_identity") or out.get("label") or "")
    observed = _norm_ids(out.get("observed_identities"))
    if keys_after is not None:
        observed = _norm_ids(sorted(keys_after))
        out["observed_identities"] = observed[:32]

    still = bool(control) and control in observed
    # Also treat visible label match as still-visible when id missing from dump keys.
    label = str(out.get("label") or "")
    if not still and label and label in observed:
        still = True

    out["control"] = control or None
    out["control_still_visible"] = still
    if sig_before is not None:
        out["screen_sig_before"] = sig_before
    if sig_after is not None:
        out["screen_sig_after"] = sig_after
    if sig_before is not None and sig_after is not None:
        out["sig_changed"] = sig_before != sig_after
    elif "sig_changed" not in out:
        # Unknown sig → infer from still-visible primary control after tap.
        out["sig_changed"] = not still if out.get("action") == "tap" else None

    if acceptance_pending is not None:
        out["acceptance_pending"] = list(acceptance_pending)

    # Normalize opaque motor_failed when control is still on screen after tap.
    fault = str(out.get("fault") or "")
    if (
        out.get("action") == "tap"
        and still
        and out.get("sig_changed") is False
        and fault in ("motor_failed", "exercise_failed", "motor_rejected", "")
    ):
        out["fault"] = "motor_no_effect"
    elif (
        out.get("action") == "tap"
        and still
        and fault in ("motor_failed", "exercise_failed")
    ):
        # Even without sig pair: dead control is the dominant OSS login failure shape.
        out["fault"] = "motor_no_effect"
        out["sig_changed"] = False

    return out


def classify_effect(tf: dict[str, Any], *, prove_phase: str | None = None) -> str:
    """Map TraceFailure v2 → RepairMode string. No task id / filename knowledge."""
    fault = str(tf.get("fault") or "")
    expected = str(tf.get("expected_identity") or "")
    control = str(tf.get("control") or expected or "")
    observed = _norm_ids(tf.get("observed_identities"))
    still = bool(tf.get("control_still_visible"))
    sig_changed = tf.get("sig_changed")
    pending = _norm_ids(tf.get("acceptance_pending"))
    action = str(tf.get("action") or "")

    has_tab_bar = "Tab Bar" in observed or any(
        o.lower() in ("tabbar", "tab bar") for o in observed
    )
    observed_tabs = [o for o in observed if o.startswith("tab_")]
    expected_tab = expected.startswith("tab_")

    # A — missing tab chrome (OSS-general: tab_* absent, siblings / tab bar present).
    if expected_tab and expected not in observed and (has_tab_bar or observed_tabs):
        return "tab_chrome_missing"
    if (
        fault in ("target_missing", "target_never_visible")
        and expected_tab
    ):
        return "tab_chrome_missing"

    # B — dead control: tapped, still visible, no transition (classic auth gate).
    if action in ("tap", "assert") and (
        fault in ("motor_no_effect", "control_fired_no_transition")
        or (still and sig_changed is False)
    ):
        # Finish-like controls on overlays → blocked_overlay; else state gate.
        cl = control.lower()
        if any(
            k in cl
            for k in (
                "finish",
                "continue",
                "getstarted",
                "get_started",
                "skip",
                "next",
                "done",
                "dismiss",
                "close",
            )
        ):
            return "blocked_overlay"
        return "state_gate_stuck"

    if fault == "blocked":
        return "blocked_overlay"

    # Postcondition phase with pending acceptance and last control still around.
    if prove_phase == "postcondition":
        if pending and still:
            cl = control.lower()
            if any(k in cl for k in ("finish", "continue", "getstarted", "skip", "next", "done")):
                return "blocked_overlay"
            return "state_gate_stuck"
        if fault in ("control_fired_no_transition", "acceptance_not_in_ax"):
            return "state_gate_stuck"

    if fault == "type_never_committed":
        return "type_never_committed"
    if fault == "motor_rejected":
        return "motor_rejected"
    if fault == "eyes_unusable":
        return "eyes_unusable"
    if fault in ("target_missing", "target_never_visible"):
        return "target_never_visible"

    return "unknown"


def classify_or_refuse(tf: dict[str, Any], *, prove_phase: str | None = None) -> dict[str, Any]:
    mode = classify_effect(tf, prove_phase=prove_phase)
    return {
        "mode": mode,
        "ok": mode != "unknown",
        "refuse_edit": mode == "unknown",
        "reason": None if mode != "unknown" else "effect_unclassified",
    }

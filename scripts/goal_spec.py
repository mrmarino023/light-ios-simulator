#!/usr/bin/env python3
"""Compile frozen task acceptance into the shared GoalSpec v2 contract."""

from __future__ import annotations

from typing import Any


def compile_task_goal(task: dict[str, Any]) -> dict[str, Any]:
    verification = task.get("verification") or {}
    post = verification.get("postconditions") or {}
    pre = verification.get("preconditions") or {}
    slots: list[dict[str, Any]] = []
    declared = task.get("run_goal_params")
    if isinstance(declared, list) and declared:
        for index, item in enumerate(declared):
            if not isinstance(item, dict):
                continue
            slots.append(
                {
                    "name": str(item.get("name") or f"slot_{index + 1}"),
                    "value": str(item.get("value") or ""),
                    "secure": bool(item.get("secure")),
                    "constraints": [str(v) for v in item.get("constraints") or []],
                }
            )
    else:
        for step in verification.get("exercise") or []:
            if not isinstance(step, dict) or step.get("action") != "type":
                continue
            identity = str(step.get("id") or step.get("label") or f"slot_{len(slots) + 1}")
            lower = identity.lower()
            slots.append(
                {
                    "name": identity,
                    "value": str(step.get("text") or ""),
                    "secure": any(token in lower for token in ("password", "secure", "passcode", "pin")),
                    "constraints": [identity],
                }
            )
    return {
        "all": [{"label": str(label)} for label in post.get("must_see_labels") or []],
        "none": [{"label": str(label)} for label in post.get("must_not_see_labels") or []],
        "starting": [{"label": str(label)} for label in pre.get("must_see_labels") or []],
        "expected_bundle_id": str(task["bundle_id"]),
        "stable_observations": 2,
        "stability_window_ms": 250,
        "slots": slots,
        "allow_destructive": False,
    }


def predicate_matches(predicate: dict[str, Any], affordance: dict[str, Any]) -> bool:
    if predicate.get("id") and predicate["id"] not in {
        affordance.get("id"),
        affordance.get("identifier"),
    }:
        return False
    if predicate.get("label"):
        label = str(affordance.get("label") or affordance.get("text") or "")
        if str(predicate["label"]) not in label:
            return False
    if predicate.get("value_contains"):
        if str(predicate["value_contains"]) not in str(affordance.get("value") or ""):
            return False
    if "enabled" in predicate and bool(affordance.get("enabled", True)) != bool(predicate["enabled"]):
        return False
    if "focused" in predicate and bool(affordance.get("focused")) != bool(predicate["focused"]):
        return False
    return True


def evaluate_goal(goal: dict[str, Any], perceive_doc: dict[str, Any]) -> dict[str, Any]:
    affordances = [a for a in perceive_doc.get("affordances") or [] if isinstance(a, dict)]
    required = {
        _predicate_name(p): any(predicate_matches(p, a) for a in affordances)
        for p in goal.get("all") or []
    }
    forbidden = {
        _predicate_name(p): not any(predicate_matches(p, a) for a in affordances)
        for p in goal.get("none") or []
    }
    return {
        "ok": bool(required) and all(required.values()) and all(forbidden.values()),
        "all": required,
        "none": forbidden,
    }


def _predicate_name(predicate: dict[str, Any]) -> str:
    for key in ("id", "label", "value_contains"):
        if predicate.get(key):
            return f"{key}:{predicate[key]}"
    return repr(sorted(predicate.items()))

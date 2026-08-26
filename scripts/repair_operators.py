#!/usr/bin/env python3
"""TRAIL structural repair operators — mode-specific, broken-tree only."""

from __future__ import annotations

import re
from typing import Any

from view_graph import try_restore_missing_tab

_GATE_FALSE_RE = re.compile(
    r"(\b(?:isLoggedIn|isAuthenticated|loggedIn)\s*=\s*)false\b"
)
_OVERLAY_DISMISS_RE = re.compile(r"(\bisOnboardingVisible\s*=\s*)false\b")
_DISABLED_TRUE_RE = re.compile(r"(\.disabled\s*\(\s*)true(\s*\))")


def apply_structural_operator(
    mode: str,
    source_root: str,
    primary_path: str,
    tf: dict[str, Any],
    original: str,
) -> dict[str, Any] | None:
    expected = str(tf.get("expected_identity") or "")
    control = str(tf.get("control") or expected or "")

    if mode == "tab_chrome_missing" and expected.startswith("tab_"):
        restored = try_restore_missing_tab(source_root, primary_path, expected)
        if restored and restored.get("text"):
            return restored

    if mode == "state_gate_stuck":
        disabled = _try_enable_control(original)
        if disabled:
            return disabled
        return _try_gate_flip(original, primary_path)

    if mode == "blocked_overlay":
        return _try_overlay_dismiss(original, primary_path)

    if "loginButton" in control or control.lower() == "login":
        disabled = _try_enable_control(original)
        if disabled:
            return disabled

    return None


def _try_gate_flip(text: str, primary_path: str) -> dict[str, Any] | None:
    """Flip wrong `= false` on auth gate in method bodies (not @Published init)."""
    lines = text.splitlines()
    hit_idx: int | None = None
    for i, line in enumerate(lines):
        if "@Published" in line:
            continue
        if _GATE_FALSE_RE.search(line):
            if hit_idx is not None:
                return None  # multiple method-site assignments — refuse
            hit_idx = i
    if hit_idx is None:
        return None
    lines[hit_idx] = _GATE_FALSE_RE.sub(r"\1true", lines[hit_idx], count=1)
    new_text = "\n".join(lines) + ("\n" if text.endswith("\n") else "")
    if new_text == text:
        return None
    return {"text": new_text, "method": "gate_bool_flip", "primary_path": primary_path}


def _try_overlay_dismiss(text: str, primary_path: str) -> dict[str, Any] | None:
    if _OVERLAY_DISMISS_RE.search(text):
        return None
    if "onComplete" not in text and "userInputComplete" not in text:
        return None
    if "isOnboardingVisible" not in text:
        return None
    marker = "func userInputComplete"
    idx = text.find(marker)
    if idx < 0:
        marker = "func done"
        idx = text.find(marker)
    if idx < 0:
        return None
    brace = text.find("{", idx)
    if brace < 0:
        return None
    insert = text.find("\n    }", brace)
    if insert < 0:
        return None
    snippet = "\n        withAnimation {\n            isOnboardingVisible = false\n        }\n"
    if snippet.strip() in text:
        return None
    new_text = text[:insert] + snippet + text[insert:]
    if new_text.count("{") != new_text.count("}"):
        return None
    return {"text": new_text, "method": "overlay_dismiss_restore", "primary_path": primary_path}


def _try_enable_control(text: str) -> dict[str, Any] | None:
    matches = list(_DISABLED_TRUE_RE.finditer(text))
    if len(matches) != 1:
        return None
    new_text = _DISABLED_TRUE_RE.sub(r"\1false\2", text, count=1)
    if new_text == text:
        new_text = text.replace(".disabled(true)", "", 1)
    if new_text == text:
        return None
    return {"text": new_text, "method": "control_enable"}

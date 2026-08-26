#!/usr/bin/env python3
"""TRAIL repair engine — OSS-general product path (no task id, no filename priors).

Single pipeline every app uses:

  broken tree → StructuralKB + identity index
  → TraceFailure v3
  → Effect Classifier (refuse unknown)
  → Causal Localizer (KB graph ascent)
  → Structural operators (effect-class transforms)
  → LLM fixer (≤2 shots, only on operator miss)
  → hard certify

Invariants:
  - Never index a healthy BACKUP twin
  - Never read task["id"] for mode or file choice
  - Prefer localize_failed / refuse over wrong-file edit
"""

from __future__ import annotations

import os
import re
from dataclasses import dataclass, field
from typing import Any

from effect_classifier import classify_or_refuse
from identity_index import build_identity_index, lookup
from repair_operators import apply_structural_operator
from structural_kb import StructuralKB, build_structural_kb, neighborhood, writer_for_control
from view_graph import hybrid_localize, localize_control_gate, localize_missing_tab

PRESENTATION_BLOCK_RE = re.compile(
    r"\.(?:disabled\s*\(\s*true\s*\)|allowsHitTesting\s*\(\s*false\s*\))"
)


@dataclass
class RepairContext:
    source_root: str
    kb: StructuralKB
    index: dict[str, list[dict[str, Any]]]

    @classmethod
    def from_source_root(cls, source_root: str) -> RepairContext:
        if not os.path.isabs(source_root):
            raise ValueError("source_root must be absolute")
        return cls(
            source_root=source_root,
            kb=build_structural_kb(source_root),
            index=build_identity_index(source_root),
        )


@dataclass
class LocalizeResult:
    primary_path: str | None
    ascent: str | None = None
    edit_targets: list[str] = field(default_factory=list)
    composition: dict[str, Any] | None = None
    sites: list[dict[str, Any]] = field(default_factory=list)

    def as_dict(self) -> dict[str, Any]:
        return {
            "primary_path": self.primary_path,
            "ascent": self.ascent,
            "edit_targets": self.edit_targets,
            "composition": self.composition,
            "sites": self.sites,
        }


def classify(tf: dict[str, Any], *, prove_phase: str | None = None) -> dict[str, Any]:
    return classify_or_refuse(tf, prove_phase=prove_phase)


def _read(source_root: str, rel: str) -> str:
    try:
        with open(os.path.join(source_root, rel), encoding="utf-8") as f:
            return f.read()
    except OSError:
        return ""


def _control_declares_presentation_block(ctx: RepairContext, control: str) -> str | None:
    """View file declaring control has .disabled / hit-test block — interaction fault."""
    for site in ctx.kb.identity_sites.get(control, []):
        rel = site["file"]
        text = _read(ctx.source_root, rel)
        if not text:
            continue
        if PRESENTATION_BLOCK_RE.search(text) and (
            f'accessibilityIdentifier("{control}")' in text or control in text
        ):
            return rel
    return None


def causal_localize(ctx: RepairContext, tf: dict[str, Any], mode: str) -> LocalizeResult:
    """KB-driven localization — mode-specific graph ascent on the broken tree."""
    expected = str(tf.get("expected_identity") or "")
    control = str(tf.get("control") or expected or "")
    observed = [str(x) for x in (tf.get("observed_identities") or [])]

    # A — missing tab chrome
    if mode == "tab_chrome_missing":
        miss = localize_missing_tab(ctx.source_root, expected, observed)
        if miss:
            rel = miss["file"]
            return LocalizeResult(
                primary_path=rel,
                ascent="missing_identity→TabView",
                edit_targets=[rel],
                composition=miss,
            )
        loc = hybrid_localize(ctx.source_root, ctx.index, expected, observed_identities=observed)
        rel = loc.get("primary_path")
        return LocalizeResult(
            primary_path=rel,
            ascent=(loc.get("composition") or {}).get("ascent"),
            edit_targets=[rel] if rel else [],
            composition=loc.get("composition"),
            sites=loc.get("sites") or [],
        )

    # B — dead control: presentation block on view OR ObservableObject gate writer
    if mode in ("state_gate_stuck", "motor_rejected"):
        block_view = _control_declares_presentation_block(ctx, control)
        if block_view:
            return LocalizeResult(
                primary_path=block_view,
                ascent="control→presentation_block",
                edit_targets=[block_view],
            )
        writer = writer_for_control(ctx.kb, control) if control else None
        if writer and mode == "state_gate_stuck":
            return LocalizeResult(
                primary_path=writer,
                ascent="control→observable_writer",
                edit_targets=[writer],
            )
        gate = localize_control_gate(ctx.source_root, control, mode="state_gate_stuck")
        if gate and gate.get("primary_path"):
            rel = gate["primary_path"]
            return LocalizeResult(
                primary_path=rel,
                ascent=gate.get("ascent"),
                edit_targets=[rel],
                composition=gate.get("composition"),
                sites=gate.get("sites") or [],
            )
        sites = ctx.kb.identity_sites.get(control, [])
        if sites:
            rel = sites[0]["file"]
            return LocalizeResult(
                primary_path=rel,
                ascent="control→declaration",
                edit_targets=[rel],
                sites=sites,
            )

    # C — stuck overlay
    if mode == "blocked_overlay":
        gate = localize_control_gate(ctx.source_root, control, mode=mode)
        if gate and gate.get("primary_path"):
            rel = gate["primary_path"]
            return LocalizeResult(
                primary_path=rel,
                ascent=gate.get("ascent"),
                edit_targets=[rel],
                composition=gate.get("composition"),
                sites=gate.get("sites") or [],
            )

    # D — target never visible / fallback identity lookup
    if mode == "target_never_visible" and expected:
        loc = hybrid_localize(ctx.source_root, ctx.index, expected, observed_identities=observed)
        rel = loc.get("primary_path")
        if rel:
            return LocalizeResult(
                primary_path=rel,
                ascent=(loc.get("composition") or {}).get("ascent"),
                edit_targets=[rel],
                composition=loc.get("composition"),
                sites=loc.get("sites") or [],
            )

    if control:
        loc = hybrid_localize(ctx.source_root, ctx.index, control, observed_identities=observed)
        rel = loc.get("primary_path")
        if rel:
            return LocalizeResult(
                primary_path=rel,
                ascent=(loc.get("composition") or {}).get("ascent"),
                edit_targets=[rel],
                composition=loc.get("composition"),
                sites=loc.get("sites") or [],
            )

    return LocalizeResult(primary_path=None)


def graph_neighborhood(ctx: RepairContext, primary_path: str, control: str) -> dict[str, Any]:
    return neighborhood(ctx.kb, primary_path, control or None)


def try_structural_fixes(
    ctx: RepairContext,
    mode: str,
    tf: dict[str, Any],
    loc: LocalizeResult,
) -> list[dict[str, Any]]:
    """Apply effect-class structural operators on ordered edit targets. No LLM."""
    results: list[dict[str, Any]] = []
    targets = loc.edit_targets or ([loc.primary_path] if loc.primary_path else [])
    seen: set[str] = set()
    for rel in targets:
        if not rel or rel in seen:
            continue
        seen.add(rel)
        abs_path = os.path.join(ctx.source_root, rel)
        if not os.path.isfile(abs_path):
            continue
        original = _read(ctx.source_root, rel)
        op = apply_structural_operator(mode, ctx.source_root, rel, tf, original)
        if not op or not op.get("text") or op["text"] == original:
            continue
        results.append(
            {
                "file": rel,
                "abs_path": abs_path,
                "text": op["text"],
                "method": op.get("method"),
                "original": original,
            }
        )
    return results

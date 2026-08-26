#!/usr/bin/env python3
"""SwiftUI view-graph ascent — identity declaration → composition owner (FixAlly-style)."""

from __future__ import annotations

import os
import re
from typing import Any

TABVIEW_RE = re.compile(r"\bTabView\s*\{")
VIEW_STRUCT_RE = re.compile(r"\bstruct\s+(\w+)\s*:\s*View\b")


def _read(path: str) -> str:
    try:
        with open(path, encoding="utf-8") as f:
            return f.read()
    except OSError:
        return ""


def _view_name_from_file(rel_path: str) -> str | None:
    base = os.path.basename(rel_path)
    if not base.endswith(".swift"):
        return None
    return base[:-6]  # strip .swift


def find_tab_composition_files(source_root: str) -> list[str]:
    hits: list[str] = []
    for dirpath, _, files in os.walk(source_root):
        if any(s in dirpath for s in ("/build/", "/DerivedData/", "/.git/")):
            continue
        for name in files:
            if not name.endswith(".swift"):
                continue
            if "TabView" not in name and "MainTab" not in name and "Navigation" not in dirpath:
                continue
            rel = os.path.relpath(os.path.join(dirpath, name), source_root).replace("\\", "/")
            text = _read(os.path.join(source_root, rel))
            if TABVIEW_RE.search(text):
                hits.append(rel)
    hits.sort(key=lambda p: (0 if "MainTabView" in p else 1, len(p)))
    return hits


def ascend_to_composition(
    source_root: str,
    identity: str,
    declaration_site: dict[str, Any],
) -> dict[str, Any]:
    """Ascend from leaf identity site to TabView composition owner when needed."""
    rel = declaration_site["file"]
    if "TabView" in rel or "MainTab" in rel:
        return {**declaration_site, "role": "composition", "ascent": "none"}

    leaf_view = _view_name_from_file(rel)
    candidates = find_tab_composition_files(source_root)
    for comp in candidates:
        text = _read(os.path.join(source_root, comp))
        if leaf_view and leaf_view in text:
            return {
                "file": comp,
                "line": _first_line(text, leaf_view) or 1,
                "snippet": f"TabView composition referencing {leaf_view}",
                "role": "composition",
                "ascent": f"{rel} → {comp}",
                "leaf_site": declaration_site,
            }
        if identity.startswith("tab_") and TABVIEW_RE.search(text):
            return {
                "file": comp,
                "line": _first_line(text, "TabView") or 1,
                "snippet": "TabView {",
                "role": "composition",
                "ascent": f"{rel} → {comp}",
                "leaf_site": declaration_site,
            }

    return {**declaration_site, "role": "declaration", "ascent": "fallback"}


def _first_line(text: str, needle: str) -> int | None:
    for i, line in enumerate(text.splitlines(), start=1):
        if needle in line:
            return i
    return None


def hybrid_localize(
    source_root: str,
    index: dict[str, list[dict[str, Any]]],
    identity: str,
) -> dict[str, Any]:
    from identity_index import lookup

    sites = lookup(index, identity)
    if not sites:
        return {"identity": identity, "sites": [], "composition": None}
    leaf = sites[0]
    composition = ascend_to_composition(source_root, identity, leaf)
    return {
        "identity": identity,
        "sites": sites,
        "composition": composition,
        "primary_path": composition["file"],
    }

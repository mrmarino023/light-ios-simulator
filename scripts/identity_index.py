#!/usr/bin/env python3
"""Static identity index: accessibilityIdentifier → file:line in task source_root."""

from __future__ import annotations

import os
import re
from typing import Any

IDENT_RE = re.compile(r'\.accessibilityIdentifier\s*\(\s*"([^"]+)"\s*\)')
IDENT_RE_ALT = re.compile(r'accessibilityIdentifier\s*:\s*"([^"]+)"')


def _repo_rel(path: str, root: str) -> str:
    return os.path.relpath(path, root).replace("\\", "/")


def build_identity_index(source_root: str, *, repo_root: str | None = None) -> dict[str, list[dict[str, Any]]]:
    """Map AX identity strings to declaration sites (paths relative to source_root)."""
    repo_root = repo_root or os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
    source_root = os.path.abspath(source_root)
    index: dict[str, list[dict[str, Any]]] = {}
    for dirpath, _, files in os.walk(source_root):
        if any(skip in dirpath for skip in ("/build/", "/DerivedData/", "/.git/")):
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
            for lineno, line in enumerate(lines, start=1):
                for rx in (IDENT_RE, IDENT_RE_ALT):
                    for m in rx.finditer(line):
                        ident = m.group(1)
                        index.setdefault(ident, []).append(
                            {
                                "file": rel,
                                "line": lineno,
                                "snippet": line.strip(),
                            }
                        )
    return index


def lookup(index: dict[str, list[dict[str, Any]]], identity: str) -> list[dict[str, Any]]:
    if identity in index:
        return index[identity]
    # tab_notes ↔ Notes label aliases
    if identity.startswith("tab_"):
        stem = identity[4:]
        for key, sites in index.items():
            if key.lower() == identity.lower() or key.lower() == stem.lower():
                return sites
    return []

#!/usr/bin/env python3
"""Broken-tree structural KB — SwiftUI View ⇄ ViewModel ⇄ Published / composition."""

from __future__ import annotations

import os
import re
from dataclasses import dataclass, field
from typing import Any

ACCESSIBILITY_ID_RE = re.compile(r'\.accessibilityIdentifier\(\s*"([^"]+)"\s*\)')
VIEW_DECL_RE = re.compile(r"\b(?:struct|class)\s+([A-Z]\w+)\b")
OBSERVABLE_RE = re.compile(
    r"@(?:StateObject|ObservedObject|EnvironmentObject)\s+(?:private\s+)?var\s+(\w+)\s*[:=]\s*([A-Z]\w+)"
)
PUBLISHED_BOOL_RE = re.compile(r"@Published\s+var\s+(\w+)\s*:\s*Bool")
CHILD_VIEW_RE = re.compile(r"\b([A-Z]\w+View)\s*\(")
BINDING_PRESENT_RE = re.compile(
    r"@Binding\s+var\s+(is\w*(?:Visible|Presented|Showing))\s*:\s*Bool"
)


def _read(path: str) -> str:
    try:
        with open(path, encoding="utf-8") as f:
            return f.read()
    except OSError:
        return ""


def _walk_swift(source_root: str):
    for dirpath, _, files in os.walk(source_root):
        if any(s in dirpath for s in ("/build/", "/DerivedData/", "/.git/", "/Pods/")):
            continue
        for name in files:
            if not name.endswith(".swift"):
                continue
            abs_path = os.path.join(dirpath, name)
            rel = os.path.relpath(abs_path, source_root).replace("\\", "/")
            yield rel, abs_path, _read(abs_path)


@dataclass
class StructuralKB:
    source_root: str
    identity_sites: dict[str, list[dict[str, Any]]] = field(default_factory=dict)
    view_types: dict[str, str] = field(default_factory=dict)
    observable_writers: dict[str, list[dict[str, Any]]] = field(default_factory=dict)
    composition_hosts: dict[str, list[dict[str, Any]]] = field(default_factory=dict)
    env_bindings: dict[str, list[dict[str, Any]]] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "source_root": self.source_root,
            "identity_count": len(self.identity_sites),
            "view_type_count": len(self.view_types),
            "writer_count": sum(len(v) for v in self.observable_writers.values()),
        }


def build_structural_kb(source_root: str) -> StructuralKB:
    kb = StructuralKB(source_root=source_root)
    for rel, _abs, text in _walk_swift(source_root):
        for m in VIEW_DECL_RE.finditer(text):
            kb.view_types.setdefault(m.group(1), rel)
        if "ObservableObject" in text or "@Published" in text:
            pubs = PUBLISHED_BOOL_RE.findall(text)
            if pubs or "@Published" in text:
                kb.observable_writers.setdefault(rel, []).append(
                    {
                        "file": rel,
                        "published_bools": pubs,
                        "has_observable_object": "ObservableObject" in text,
                    }
                )
        for m in BINDING_PRESENT_RE.finditer(text):
            kb.env_bindings.setdefault(m.group(1), []).append({"file": rel, "binding": m.group(1)})
        for ident in ACCESSIBILITY_ID_RE.findall(text):
            kb.identity_sites.setdefault(ident, []).append(
                {"file": rel, "line": _first_line(text, ident) or 1}
            )
        for child in CHILD_VIEW_RE.findall(text):
            kb.composition_hosts.setdefault(child, []).append({"file": rel, "hosts": child})
    return kb


def _first_line(text: str, needle: str) -> int | None:
    for i, line in enumerate(text.splitlines(), start=1):
        if needle in line:
            return i
    return None


def neighborhood(kb: StructuralKB, primary_path: str, control: str | None = None) -> dict[str, Any]:
    text = _read(os.path.join(kb.source_root, primary_path))
    out: dict[str, Any] = {"primary": primary_path, "control": control}
    if control:
        out["identity_sites"] = kb.identity_sites.get(control, [])
    writers: list[dict[str, Any]] = []
    for entries in kb.observable_writers.values():
        writers.extend(entries)
    out["observable_writers_nearby"] = writers[:6]
    types_in_file = VIEW_DECL_RE.findall(text)
    hosts: list[dict[str, Any]] = []
    for t in types_in_file:
        hosts.extend(kb.composition_hosts.get(t, []))
    out["composition_hosts"] = hosts[:4]
    out["bindings"] = []
    for binds in kb.env_bindings.values():
        out["bindings"].extend(binds[:2])
    return out


def writer_for_control(kb: StructuralKB, control: str) -> str | None:
    sites = kb.identity_sites.get(control, [])
    if not sites:
        return None
    view_file = sites[0]["file"]
    view_text = _read(os.path.join(kb.source_root, view_file))
    for m in OBSERVABLE_RE.finditer(view_text):
        typ = m.group(2)
        writer = kb.view_types.get(typ)
        if writer and kb.observable_writers.get(writer):
            return writer
    return None

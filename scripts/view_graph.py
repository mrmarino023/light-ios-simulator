#!/usr/bin/env python3
"""SwiftUI view-graph ascent — identity → composition owner from the *broken* tree.

No healthy-tree oracle. No filename priors (MainTabView / LoginViewModel).
When the expected AX id is absent from source, localize from TraceFailure:
observed tab ids / TabView structure / control identity sites.
"""

from __future__ import annotations

import os
import re
from typing import Any

TABVIEW_RE = re.compile(r"\bTabView\s*(\(|\{)")
TAB_ITEM_RE = re.compile(r"\.tabItem\s*\{")
ACCESSIBILITY_ID_RE = re.compile(
    r'\.accessibilityIdentifier\(\s*"([^"]+)"\s*\)'
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


def _view_name_from_file(rel_path: str) -> str | None:
    base = os.path.basename(rel_path)
    if not base.endswith(".swift"):
        return None
    return base[:-6]


def _first_line(text: str, needle: str) -> int | None:
    for i, line in enumerate(text.splitlines(), start=1):
        if needle in line:
            return i
    return None


def find_tab_composition_files(source_root: str) -> list[tuple[str, str, int]]:
    """All Swift files that contain a TabView — scored by .tabItem density (not filename)."""
    hits: list[tuple[str, str, int]] = []
    for rel, _abs, text in _walk_swift(source_root):
        if not TABVIEW_RE.search(text):
            continue
        score = len(TAB_ITEM_RE.findall(text))
        score += len(ACCESSIBILITY_ID_RE.findall(text))
        hits.append((rel, text, score))
    hits.sort(key=lambda t: (-t[2], len(t[0])))
    return hits


def localize_missing_tab(
    source_root: str,
    expected: str,
    observed_identities: list[str] | None = None,
) -> dict[str, Any] | None:
    """Find TabView composition that hosts observed tabs but not the missing expected id."""
    observed = list(observed_identities or [])
    observed_tabs = [x for x in observed if isinstance(x, str) and x.startswith("tab_")]
    best: dict[str, Any] | None = None
    best_score = -1
    for rel, text, base_score in find_tab_composition_files(source_root):
        ids_in_file = set(ACCESSIBILITY_ID_RE.findall(text))
        # Prefer composition that already shows sibling tabs from the live AX dump.
        present = [t for t in observed_tabs if t in ids_in_file or t in text]
        score = base_score + 5 * len(present)
        if expected and expected in ids_in_file:
            # Identity still declared — leaf site, not the omission site.
            score -= 20
        if expected and expected not in text:
            score += 10  # omission site
        if score > best_score:
            best_score = score
            best = {
                "file": rel,
                "line": _first_line(text, "TabView") or 1,
                "snippet": "TabView {",
                "role": "composition",
                "ascent": "missing_identity→TabView",
                "score": score,
                "observed_tabs_matched": present,
            }
    return best


def _title_tokens(suffix: str) -> str:
    parts = [p for p in re.split(r"[_\-]+", suffix) if p]
    return " ".join(p[:1].upper() + p[1:] for p in parts) if parts else suffix


def _camel_view_candidates(suffix: str) -> list[str]:
    parts = [p for p in re.split(r"[_\-]+", suffix) if p]
    if not parts:
        return []
    camel = "".join(p[:1].upper() + p[1:] for p in parts)
    return [f"{camel}View", f"{camel}Screen", f"{camel}Tab", camel]


def _find_view_type(source_root: str, suffix: str) -> tuple[str, str] | None:
    """Return (TypeName, file_text) for a View that matches tab_<suffix>."""
    for cand in _camel_view_candidates(suffix):
        for _rel, _abs, text in _walk_swift(source_root):
            if re.search(rf"\b(?:struct|class)\s+{re.escape(cand)}\b", text):
                return cand, text
    return None


def _host_env_exprs(host_text: str) -> dict[str, str]:
    """Map type name → expression available in the TabView host."""
    out: dict[str, str] = {}
    for m in re.finditer(
        r"@(?:StateObject|EnvironmentObject|ObservedObject)\s+(?:private\s+)?var\s+(\w+)\s*[:=]\s*([A-Z]\w*)",
        host_text,
    ):
        out[m.group(2)] = m.group(1)
    # Common nested managers: appState.favoritesManager
    for m in re.finditer(r"(\w+)\.(\w+)\b", host_text):
        root, prop = m.group(1), m.group(2)
        if root in out.values() or re.search(
            rf"@(?:StateObject|EnvironmentObject|ObservedObject)\s+(?:private\s+)?var\s+{re.escape(root)}\b",
            host_text,
        ):
            # Infer type from property name heuristics when used as environmentObject arg.
            if "favorites" in prop.lower():
                out.setdefault("FavoritesManager", f"{root}.{prop}")
            if "cart" in prop.lower():
                out.setdefault("CartManager", f"{root}.{prop}")
    return out


def _env_injections(host_text: str, view_text: str) -> list[str]:
    needed = re.findall(
        r"@EnvironmentObject\s+(?:private\s+)?var\s+\w+\s*:\s*([A-Z]\w*)",
        view_text,
    )
    host = _host_env_exprs(host_text)
    inj: list[str] = []
    for typ in needed:
        expr = host.get(typ)
        if expr:
            inj.append(f".environmentObject({expr})")
    return inj


def _sf_symbol_for_suffix(suffix: str) -> str:
    key = suffix.lower().replace("-", "_")
    hints = {
        "notes": "note.text",
        "note": "note.text",
        "home": "house",
        "favorites": "heart",
        "favorite": "heart",
        "cart": "cart",
        "menu": "line.3.horizontal",
        "search": "magnifyingglass",
        "profile": "person",
        "settings": "gearshape",
        "shop": "bag",
        "store": "bag",
    }
    return hints.get(key, hints.get(key.split("_")[0], "circle"))


def try_restore_missing_tab(
    source_root: str,
    tabview_rel: str,
    expected_id: str,
) -> dict[str, Any] | None:
    """Deterministic OSS restore: insert a TabView child when its View type exists.

    No healthy-tree oracle. Uses only the broken TabView file + View declarations
    still present elsewhere in the tree (classic omitted-tab regression).
    """
    if not expected_id.startswith("tab_"):
        return None
    suffix = expected_id[4:]
    if not suffix:
        return None
    abs_path = (
        tabview_rel
        if os.path.isabs(tabview_rel)
        else os.path.join(source_root, tabview_rel)
    )
    host = _read(abs_path)
    if not host or expected_id in host or not TABVIEW_RE.search(host):
        return None
    found = _find_view_type(source_root, suffix)
    if not found:
        return None
    view_type, view_text = found
    if f"{view_type}(" in host:
        return None  # already referenced somehow without AX id

    tags = [int(x) for x in re.findall(r"\.tag\(\s*(\d+)\s*\)", host)]
    tag = 0
    used = set(tags)
    while tag in used:
        tag += 1

    label = _title_tokens(suffix)
    symbol = _sf_symbol_for_suffix(suffix)
    env_lines = _env_injections(host, view_text)
    indent = "            "
    block_lines = [f"{indent}{view_type}()"]
    for env in env_lines:
        block_lines.append(f"{indent}    {env}")
    block_lines.extend(
        [
            f"{indent}    .tabItem {{",
            f'{indent}        Label("{label}", systemImage: "{symbol}")',
            f"{indent}    }}",
            f"{indent}    .tag({tag})",
            f'{indent}    .accessibilityIdentifier("{expected_id}")',
            "",
        ]
    )
    block = "\n".join(block_lines)

    insert_at: int | None = None
    prev_tag = tag - 1
    while prev_tag >= 0:
        m = re.search(rf'\.tag\(\s*{prev_tag}\s*\)\s*\n', host)
        if m:
            insert_at = m.end()
            break
        prev_tag -= 1
    if insert_at is None:
        m_home = re.search(
            r'\.accessibilityIdentifier\(\s*"tab_home"\s*\)\s*\n',
            host,
        )
        if m_home:
            insert_at = m_home.end()
        else:
            m_tv = TABVIEW_RE.search(host)
            if m_tv:
                brace = host.find("{", m_tv.start())
                if brace >= 0:
                    nl = host.find("\n", brace)
                    insert_at = (nl + 1) if nl >= 0 else brace + 1
    if insert_at is None:
        return None

    new_text = host[:insert_at] + block + host[insert_at:]
    if new_text.count("{") != new_text.count("}"):
        return None
    return {
        "text": new_text,
        "view_type": view_type,
        "tag": tag,
        "expected_id": expected_id,
        "method": "structural_tab_restore",
    }


def ascend_to_composition(
    source_root: str,
    identity: str,
    declaration_site: dict[str, Any],
) -> dict[str, Any]:
    """Ascend from leaf identity site to TabView composition owner when needed."""
    rel = declaration_site["file"]
    text0 = _read(os.path.join(source_root, rel))
    if TABVIEW_RE.search(text0):
        return {**declaration_site, "role": "composition", "ascent": "none"}

    leaf_view = _view_name_from_file(rel)
    for comp, text, _score in find_tab_composition_files(source_root):
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


def hybrid_localize(
    source_root: str,
    index: dict[str, list[dict[str, Any]]],
    identity: str,
    *,
    observed_identities: list[str] | None = None,
) -> dict[str, Any]:
    from identity_index import lookup

    sites = lookup(index, identity)
    if sites:
        leaf = sites[0]
        composition = ascend_to_composition(source_root, identity, leaf)
        return {
            "identity": identity,
            "sites": sites,
            "composition": composition,
            "primary_path": composition["file"],
        }

    # Identity absent from broken tree (classic missing-tab bug).
    if identity.startswith("tab_") or (observed_identities and any(
        str(x).startswith("tab_") for x in observed_identities
    )):
        miss = localize_missing_tab(source_root, identity, observed_identities)
        if miss:
            return {
                "identity": identity,
                "sites": [],
                "composition": miss,
                "primary_path": miss["file"],
            }

    return {"identity": identity, "sites": [], "composition": None, "primary_path": None}


def localize_control_gate(
    source_root: str,
    control: str,
    *,
    mode: str,
) -> dict[str, Any] | None:
    """Localize state/overlay gates from the control identity present in broken source."""
    if not control:
        return None
    needles = [control]
    if "_" in control:
        parts = control.split("_")
        needles.append("".join(p[:1].upper() + p[1:] for p in parts if p))
        needles.append("".join(p.title() for p in parts))
        needles.append(parts[-1])

    # Causal ascent: View declaring control → ObservableObject / ViewModel writers.
    view_hits: list[dict[str, Any]] = []
    for rel, _abs, text in _walk_swift(source_root):
        if not any(n and n in text for n in needles):
            continue
        score = 0
        if f'accessibilityIdentifier("{control}")' in text or f'accessibilityIdentifier("{control}")' in text:
            score += 8
        if any(n and f'"{n}"' in text for n in needles):
            score += 4
        if "@Published" in text or "@State" in text:
            score += 3
        view_hits.append({"file": rel, "line": _first_line(text, control) or 1, "text": text, "score": score})

    if not view_hits:
        return None
    view_hits.sort(key=lambda h: (-h["score"], len(h["file"])))

    # Extract referenced model types from the best view hit(s).
    type_re = re.compile(
        r"@(?:StateObject|ObservedObject|EnvironmentObject)\s+(?:private\s+)?var\s+\w+\s*[:=]\s*([A-Z]\w+)"
    )
    type_re2 = re.compile(r"\b([A-Z]\w*(?:ViewModel|Model|Store|State|Controller))\s*\(")
    candidates: dict[str, int] = {}
    for hit in view_hits[:3]:
        for m in type_re.finditer(hit["text"]):
            candidates[m.group(1)] = candidates.get(m.group(1), 0) + 10
        for m in type_re2.finditer(hit["text"]):
            candidates[m.group(1)] = candidates.get(m.group(1), 0) + 6

    # Leaf view type names (struct Foo) — hosts that construct Foo( own overlay/finish handlers.
    leaf_types: set[str] = set()
    for hit in view_hits[:3]:
        for m in re.finditer(r"\b(?:struct|class)\s+([A-Z]\w+)\b", hit["text"]):
            leaf_types.add(m.group(1))
        base = os.path.basename(hit["file"])
        if base.endswith(".swift"):
            leaf_types.add(base[:-6])

    writer_hits: list[dict[str, Any]] = []
    for rel, _abs, text in _walk_swift(source_root):
        base = os.path.basename(rel)[:-6] if rel.endswith(".swift") else ""
        # Do not treat the leaf control view as the gate writer when a host exists.
        if base in leaf_types and mode == "blocked_overlay":
            continue
        type_bonus = candidates.get(base, 0)
        hosts_leaf = any(
            re.search(rf"\b{re.escape(t)}\s*\(", text) for t in leaf_types if t and t != base
        )
        if type_bonus == 0 and not any(t in text for t in candidates) and not hosts_leaf:
            # Still consider files that both mention control and publish state.
            if not any(n and n in text for n in needles):
                continue
        score = type_bonus
        if hosts_leaf:
            score += 14  # OSS-general: parent that embeds the control's View
        if "ObservableObject" in text:
            score += 5
        if "@Published" in text:
            score += 8
            # Prefer writers that assign Bool/enum in methods (gate flips).
            if re.search(r"@Published\s+var\s+\w+\s*:\s*Bool", text):
                score += 6
            if re.search(r"\w+\s*=\s*(true|false)\b", text):
                score += 4
        if mode == "blocked_overlay":
            if any(
                k in text
                for k in (
                    "fullScreenCover",
                    "sheet(",
                    ".overlay",
                    "isPresented",
                    "isOnboarding",
                )
            ):
                score += 8
            # Presentation Bool writers (Binding or local assign) — classic stuck overlay.
            if re.search(
                r"\b(?:is\w*(?:Visible|Presented|Showing|Complete)|hasCompleted\w*)\s*=\s*(?:true|false)\b",
                text,
            ):
                score += 12
            if re.search(
                r"@Binding\s+var\s+is\w*(?:Visible|Presented|Showing)",
                text,
            ):
                score += 6
            if "onComplete" in text or "userInputComplete" in text:
                score += 4
        if mode == "state_gate_stuck" and any(
            k in text for k in ("isLoggedIn", "isAuthenticated", "NavigationPath", "navigate", "route")
        ):
            score += 3  # soft lexical — not required for OSS
        if score <= 0:
            continue
        line = (
            _first_line(text, "isOnboardingVisible")
            or _first_line(text, "@Published")
            or _first_line(text, control)
            or 1
        )
        writer_hits.append(
            {
                "file": rel,
                "line": line,
                "snippet": control,
                "score": score,
                "role": "gate_writer",
            }
        )

    if writer_hits:
        writer_hits.sort(key=lambda h: (-h["score"], len(h["file"])))
        best = writer_hits[0]
        # Prefer writer over leaf view when scores compete.
        return {
            "identity": control,
            "sites": writer_hits[:3],
            "composition": best,
            "primary_path": best["file"],
            "ascent": "control→observable_writer",
        }

    # Fallback: best view that declares the control (better than random).
    best_view = {k: v for k, v in view_hits[0].items() if k != "text"}
    best_view["role"] = "control_view"
    return {
        "identity": control,
        "sites": [{k: v for k, v in h.items() if k != "text"} for h in view_hits[:3]],
        "composition": best_view,
        "primary_path": best_view["file"],
        "ascent": "control_view_only",
    }


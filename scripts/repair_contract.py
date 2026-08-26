"""L5-L8 repair plane helpers for the killer-loop agent."""

from __future__ import annotations

import fnmatch
import os
import re
from typing import Any


def _norm(path: str) -> str:
    return path.replace("\\", "/")


def glob_match(pattern: str, path: str) -> bool:
    pat = pattern.lstrip("./")
    norm = _norm(path)
    base = os.path.basename(norm)
    if pat == "**/*.swift":
        return norm.endswith(".swift")
    if pat.startswith("**/"):
        rest = pat[3:]
        if rest.endswith("/**"):
            return rest[:-3] in norm
        if "*" in rest:
            # e.g. *ViewModel*.swift or Navigation/**
            if fnmatch.fnmatch(base, rest) or fnmatch.fnmatch(norm, rest):
                return True
            prefix = rest.split("*", 1)[0].rstrip("/")
            return bool(prefix) and prefix in norm
        return rest in norm
    if "*" in pat:
        return fnmatch.fnmatch(base, pat) or fnmatch.fnmatch(norm, pat)
    return pat in norm


def path_allowed(rel_path: str, contract: dict[str, Any] | None) -> bool:
    if not contract:
        return True
    scope = contract.get("scope") or {}
    norm = _norm(rel_path)
    for forbidden in scope.get("forbidden_globs") or []:
        if glob_match(str(forbidden), norm):
            return False
    for allowed in scope.get("edit_globs") or []:
        if glob_match(str(allowed), norm):
            return True
    primary = scope.get("primary_path")
    if primary and norm == _norm(str(primary)):
        return True
    return False


def scope_violation(rel_path: str, contract: dict[str, Any] | None) -> str | None:
    if not contract or path_allowed(rel_path, contract):
        return None
    mode = contract.get("mode") or contract.get("diagnosis_code") or "repair"
    return f"repair_scope_violation:{mode}: edit only within {((contract.get('scope') or {}).get('edit_globs') or [])}"


def contract_nudge(result: dict[str, Any]) -> str:
    contract = result.get("repair_contract")
    if not isinstance(contract, dict):
        return ""
    scope = contract.get("scope") or {}
    primary = scope.get("primary_path")
    intent = scope.get("edit_intent") or contract.get("invariant") or ""
    forbidden = scope.get("forbidden_globs") or []
    parts = [
        f"RepairContract mode={contract.get('mode')}: {intent}.",
    ]
    if primary:
        parts.append(f"Start at {primary}.")
    if forbidden:
        parts.append(f"Do not edit: {', '.join(str(x) for x in forbidden)}.")
    evidence = contract.get("evidence") or {}
    missing = evidence.get("missing_identities") or []
    if missing:
        parts.append(f"Missing on-screen: {', '.join(str(x) for x in missing)}.")
    tabs = evidence.get("tab_items") or []
    if evidence.get("has_tab_bar") and tabs:
        parts.append(f"Tab bar items visible: {', '.join(str(x) for x in tabs)}.")
    return " ".join(parts)

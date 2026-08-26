#!/usr/bin/env python3
"""TRAIL constrained fixer executor.

Consumes `docs/assets/trail-latest.json` (or a supplied artifact), sends the bounded
one-file prompt to an LLM, writes the returned full file, then relies on the gate
to build and certify.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from repair_contract import path_allowed, scope_violation  # noqa: E402

MODEL = os.environ.get("OPENAI_MODEL", "gpt-5-mini")
OPENAI_URL = os.environ.get("OPENAI_BASE_URL", "https://api.openai.com/v1/chat/completions")


def openai_full_file(messages: list[dict[str, Any]]) -> tuple[str, dict[str, int]]:
    key = os.environ.get("OPENAI_API_KEY", "").strip()
    if not key:
        raise RuntimeError("OPENAI_API_KEY missing")
    body = {
        "model": MODEL,
        "messages": messages,
        "max_completion_tokens": int(os.environ.get("LIGH_TRAIL_MAX_TOKENS", "2800")),
    }
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump(body, f)
        body_path = f.name
    try:
        r = subprocess.run(
            [
                "curl",
                "-sS",
                "-X",
                "POST",
                OPENAI_URL,
                "-H",
                f"Authorization: Bearer {key}",
                "-H",
                "Content-Type: application/json",
                "-d",
                f"@{body_path}",
            ],
            capture_output=True,
            text=True,
            timeout=180,
        )
    finally:
        try:
            os.unlink(body_path)
        except OSError:
            pass
    if r.returncode != 0:
        raise RuntimeError(r.stderr[:400] or "curl failed")
    payload = json.loads(r.stdout)
    if "error" in payload:
        raise RuntimeError(str(payload["error"]))
    content = payload["choices"][0]["message"]["content"].strip()
    if content.startswith("```"):
        content = re.sub(r"^```[a-zA-Z0-9_+-]*\s*", "", content)
        content = re.sub(r"\s*```$", "", content)
    usage = payload.get("usage") or {}
    return content, {
        "prompt_tokens": int(usage.get("prompt_tokens") or 0),
        "completion_tokens": int(usage.get("completion_tokens") or 0),
        "total_tokens": int(usage.get("total_tokens") or 0),
    }


def load_trail(path: str) -> dict[str, Any]:
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def read_file(path: str) -> str:
    with open(path, encoding="utf-8") as f:
        return f.read()


def extract_target(trail: dict[str, Any]) -> tuple[dict[str, Any], str]:
    bundle = trail.get("repair_bundle") or {}
    fixer = bundle.get("fixer_input") or {}
    target = fixer.get("target_file")
    if not target:
        raise RuntimeError("repair_bundle.fixer_input.target_file missing")
    abs_target = target if os.path.isabs(target) else os.path.join(ROOT, target)
    return bundle, abs_target


def build_messages(bundle: dict[str, Any], original: str, attempt: int, feedback: str | None) -> list[dict[str, Any]]:
    fixer = bundle.get("fixer_input") or {}
    prompt = fixer.get("prompt") or ""
    extra = []
    if feedback:
        extra.extend(
            [
                "",
                f"Attempt {attempt - 1} feedback:",
                feedback,
                "",
                "Revise the same target file only. Return the full updated file contents only.",
            ]
        )
    return [
        {
            "role": "system",
            "content": (
                "You repair one SwiftUI file under hard scope control. "
                "Return only the full final file contents — no markdown, no explanation. "
                "Keep the file short; make the minimal edit that satisfies the fix plan."
            ),
        },
        {
            "role": "user",
            "content": "\n".join(
                [
                    prompt,
                    "",
                    "Current full file contents:",
                    original,
                    *extra,
                ]
            ),
        },
    ]


def apply_candidate(abs_target: str, text: str, bundle: dict[str, Any]) -> None:
    rel = os.path.relpath(abs_target, ROOT).replace("\\", "/")
    if not path_allowed(rel, bundle):
        raise RuntimeError(scope_violation(rel, bundle) or "repair_scope_violation")
    with open(abs_target, "w", encoding="utf-8") as f:
        f.write(text)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--trail",
        default=os.environ.get("LIGH_TRAIL_OUT", os.path.join(ROOT, "docs/assets/trail-latest.json")),
    )
    ap.add_argument("--out", default=os.environ.get("LIGH_TRAIL_FIX_OUT", os.path.join(ROOT, "docs/assets/trail-fix-latest.json")))
    ap.add_argument("--max-attempts", type=int, default=int(os.environ.get("LIGH_TRAIL_FIX_ATTEMPTS", "2")))
    args = ap.parse_args()

    trail = load_trail(args.trail)
    bundle, abs_target = extract_target(trail)
    original = read_file(abs_target)
    attempts: list[dict[str, Any]] = []
    feedback: str | None = None
    final_text = original

    for attempt in range(1, max(1, args.max_attempts) + 1):
        messages = build_messages(bundle, original, attempt, feedback)
        candidate, usage = openai_full_file(messages)
        changed = candidate != original
        attempts.append(
            {
                "attempt": attempt,
                "changed": changed,
                "usage": usage,
                "chars": len(candidate),
            }
        )
        if not changed:
            feedback = "Model returned the original file unchanged."
            continue
        final_text = candidate
        apply_candidate(abs_target, candidate, bundle)
        break

    doc = {
        "gate": "trail_fix",
        "model": MODEL,
        "target_file": os.path.relpath(abs_target, ROOT).replace("\\", "/"),
        "changed": final_text != original,
        "attempts": attempts,
    }
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")
    print(json.dumps(doc))
    return 0 if doc["changed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

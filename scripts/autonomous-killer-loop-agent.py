#!/usr/bin/env python3
"""Killer loop agent — task-driven code → build → verify loop on frozen OSS apps.

Arms (LIGH_KILLER_ARM):
  ligh      — perceive / attempt (default)
  hybrid    — AX-first routed perceive; vision only on eyes_unusable escalation
  autopilot — LLM owns code only; host drives the UI via run_goal (zero-token motor)
  baseline  — screenshot + vision taps (same edit/build/harness)

Agent receives task.json prompt only. ground-truth.json is never loaded here.
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import re
import subprocess
import sys
import time
from typing import Any

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from killer_loop_task import load_task, list_swift_sources, safe_source_path  # noqa: E402
from killer_loop_verify import establish_initial_state, run_steps, strict_verify  # noqa: E402
from goal_spec import compile_task_goal  # noqa: E402
from ligh_mcp import call_tool, ligh_result_path  # noqa: E402

ARM = os.environ.get("LIGH_KILLER_ARM", "ligh").lower()
HONEST = os.environ.get("LIGH_KILLER_HONEST", "0") == "1"
SCORED = os.environ.get("LIGH_KILLER_SCORED", "1" if HONEST else "0") == "1"
MODEL = os.environ.get("OPENAI_MODEL", "gpt-5-mini")
OPENAI_URL = os.environ.get("OPENAI_BASE_URL", "https://api.openai.com/v1/chat/completions")
MAX_STEPS = int(os.environ.get("LIGH_KILLER_MAX_STEPS", "28"))
TASK = load_task()

APP = os.environ.get("LIGH_APP_PATH", TASK["app_path"])
BUNDLE_ID = TASK["bundle_id"]
BUILD_SCRIPT = TASK["build_script"]
PROTOCOL_VERSION = TASK.get("protocol_version", 1)


def hash_text(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def current_git_sha() -> str:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
        ).strip()
    except Exception:
        return "unknown"


def strip_scored_coaching(value: Any) -> Any:
    """Remove host coaching from scored tool output before either arm sees it."""
    if isinstance(value, dict):
        return {
            key: strip_scored_coaching(item)
            for key, item in value.items()
            if key not in {"source_hint", "coaching", "suggestion"}
        }
    if isinstance(value, list):
        return [strip_scored_coaching(item) for item in value]
    return value


def system_prompt() -> str:
    sources = "\n".join(f"- {p}" for p in list_swift_sources(TASK["source_root"])[:12])
    if ARM == "autopilot":
        compiled_goal = json.dumps(compile_task_goal(TASK), separators=(",", ":"))
        failure_contract = (
            "run_goal returns reached plus a modality-neutral diagnosis."
            if SCORED
            else "run_goal returns reached plus, on failure, a diagnosis and source_hint pointing at the code."
        )
        ui = f"""UI control: you do NOT drive the UI. The host does.
Action: run_goal — the host installs, launches, discovers the path and verifies the goal.
  {{"action":"run_goal","goal_spec":{compiled_goal}}}
Pass only the acceptance target and the data the flow needs (read them from the task).
Never pass a step list: the host finds the path itself. There are no taps for you to make.
{failure_contract}"""
    elif ARM == "baseline":
        ui = """UI control: screenshot + vision coordinates ONLY (no accessibility tree for planning).
Actions: screenshot, vision_tap, vision_type, dismiss (keyboard)."""
    elif ARM == "hybrid" and not HONEST:
        ui = """UI control: AX-first routed perceive (+ Feel IR). Vision only on escalation.
Actions: perceive, exercise_app, attempt, find, dismiss, vision_tap, vision_type (vision only when channel=vision).
Prefer exercise_app after bootstrap when available."""
    elif ARM == "hybrid":
        ui = """UI control: AX-first routed perceive (+ Feel IR). Vision only on escalation.
Actions: perceive, attempt, find, dismiss, vision_tap, vision_type (vision only when channel=vision).
You must drive the UI yourself — no host exercise shortcut."""
    elif HONEST:
        ui = """UI control: perceive (+ Feel IR) and attempt/find/dismiss.
Actions: perceive, attempt, find, dismiss.
You must drive the UI yourself with attempt (type/tap) — no host exercise shortcut. No screenshots."""
    else:
        ui = """UI control: perceive (returns Feel IR) + exercise_app (+ attempt/find/dismiss if needed).
Prefer feel.salience / feel.suggest over raw affordance dumps. No screenshots for planning.
After bootstrap_app: call exercise_app (host-owned taps) then verify — do not hand-drive every tap."""

    if ARM == "autopilot":
        code_actions = "read_file, write_file, build_app, run_goal, verify, done"
    elif HONEST:
        code_actions = "read_file, write_file, build_app, bootstrap_app, verify, done"
    else:
        code_actions = "read_file, write_file, build_app, bootstrap_app, exercise_app, verify, done"

    if ARM == "autopilot":
        scored_failure_rule = (
            "  diagnosis is modality-neutral; infer any source change from repository evidence."
            if SCORED
            else "  source_hint: they tell you which state failed to change and where to look. Fix the cause,"
        )
        rules = f"""Rules:
- Your job is the Swift bug, nothing else. Read the source, make a minimal fix, rebuild.
- After build_app succeeds call run_goal. If it reports reached=false, read diagnosis and
{scored_failure_rule}
  do not retry the same edit.
- run_goal automatically invokes the strict harness when it reaches the target. A passing
  harness ends the session immediately; no extra confirmation or second patch is needed.
- If run_goal fails, fix the evidence-backed cause before calling it again.
- Do not invent success — only verify/done after the harness would pass.
Never ask the user questions."""
    elif HONEST:
        rules = """Rules:
- Fix the Swift bug with a minimal change; rebuild; bootstrap; exercise the UI yourself; then verify.
- Call verify before done. done re-runs the strict harness (setup → exercise → postconditions).
- Do not invent success — only verify/done after the harness would pass.
Never ask the user questions."""
    else:
        rules = """Rules:
- Prefer a SURGICAL fix. Do not rewrite whole files, move enums, add typealiases, or redesign onboarding pages.
- Look for the finish/dismiss path of onboarding (what should hide the overlay after the last step).
- After build_app succeeds: bootstrap_app → exercise_app → verify.
- Call verify before done. done triggers the same strict harness (setup → exercise → postconditions).
- Seeing "Hello, world!" alone is NOT success if the onboarding overlay is still visible.
Never ask the user questions."""

    return f"""You fix and verify a real iOS app on a Mac.

Each turn reply with ONE JSON object.

Code actions:
  {code_actions}

{ui}

Swift sources:
{sources}

{rules}"""


def openai_chat(messages: list[dict[str, Any]], vision_image_b64: str | None = None) -> dict[str, Any]:
    import tempfile

    key = os.environ.get("OPENAI_API_KEY", "").strip()
    if not key:
        raise RuntimeError("OPENAI_API_KEY missing")
    body: dict[str, Any] = {
        "model": MODEL,
        "response_format": {"type": "json_object"},
        "messages": messages,
    }
    if vision_image_b64 and messages:
        last = messages[-1]
        if last.get("role") == "user" and isinstance(last.get("content"), str):
            last = {
                "role": "user",
                "content": [
                    {"type": "text", "text": last["content"]},
                    {"type": "image_url", "image_url": {"url": f"data:image/png;base64,{vision_image_b64}"}},
                ],
            }
            body["messages"] = messages[:-1] + [last]
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump(body, f)
        path = f.name
    try:
        r = subprocess.run(
            [
                "curl", "-sS", "-X", "POST", OPENAI_URL,
                "-H", f"Authorization: Bearer {key}",
                "-H", "Content-Type: application/json",
                "-d", f"@{path}",
            ],
            capture_output=True,
            text=True,
            timeout=180,
        )
    finally:
        try:
            os.unlink(path)
        except OSError:
            pass
    if r.returncode != 0:
        raise RuntimeError(r.stderr[:400] or "curl failed")
    payload = json.loads(r.stdout)
    if "error" in payload:
        raise RuntimeError(str(payload["error"]))
    content = payload["choices"][0]["message"]["content"].strip()
    if content.startswith("```"):
        content = re.sub(r"^```(?:json)?\s*", "", content)
        content = re.sub(r"\s*```$", "", content)
    usage = payload.get("usage") or {}
    return {
        "act": json.loads(content),
        "usage": {
            "prompt_tokens": int(usage.get("prompt_tokens") or 0),
            "completion_tokens": int(usage.get("completion_tokens") or 0),
        },
    }


def affordance_keys(perceive: dict[str, Any]) -> set[str]:
    keys: set[str] = set()
    for a in perceive.get("affordances") or []:
        if not isinstance(a, dict):
            continue
        for k in ("id", "label", "identifier", "text"):
            if a.get(k):
                keys.add(str(a[k]))
    return keys


def failure_suggestion() -> str:
    """Nudge on failure. Under the honest protocol it must be modality-neutral and
    identical across arms, or the prompt — not the architecture — is what we measure.
    """
    if HONEST or ARM == "autopilot":
        return "The harness still fails. Re-read the evidence, fix the cause in Swift, rebuild, verify."
    return (
        "Minimal edit only. Find finish/dismiss handler, restore overlay hide, "
        "build_app, bootstrap_app, then verify. Do not rewrite enums/pages."
    )


def harness_verify() -> dict[str, Any]:
    return strict_verify(TASK, app=APP, bundle_id=BUNDLE_ID)


def bootstrap_app() -> dict[str, Any]:
    call_tool("ligh_ready", {"settle_ms": 2500, "recover_homes": 4})
    wait_label = TASK.get("bootstrap_wait_label")
    payload: dict[str, Any] = {
        "app": APP,
        "bundle_id": BUNDLE_ID,
        "settle_ms": 3500,
        "timeout_ms": 15000,
    }
    if wait_label:
        payload["wait_label"] = wait_label
    boot = call_tool("ligh_cap_run_app", payload)
    app_label = os.path.basename(APP).replace(".app", "")
    ready_markers = set((TASK.get("verification") or {}).get("preconditions", {}).get("must_see_labels") or [])
    ready_markers |= {"Show Onboarding", "Get Started", "Hello, world!", "Welcome", "Login", "homeTitle"}
    for attempt in range(1, 6):
        p = call_tool("ligh_perceive", {"settle_ms": 2500})
        perceive = p.get("perceive") or {}
        keys = affordance_keys(perceive)
        if keys & ready_markers:
            return {**boot, "foreground_ok": True, "attempt": attempt}
        if any(
            a.get("identifier") == app_label or a.get("label") == app_label
            for a in (perceive.get("affordances") or [])
            if isinstance(a, dict)
        ):
            call_tool("ligh_launch", {"bundle_id": BUNDLE_ID})
            time.sleep(1.0)
            call_tool("ligh_attempt", {"intent": "tap", "label": app_label, "settle_ms": 2000, "timeout_ms": 8000})
            time.sleep(1.0)
            continue
    return {**boot, "foreground_ok": False}


def screenshot_b64(path: str | None = None) -> str:
    r = call_tool("ligh_screenshot", {"path": path} if path else {})
    shot_path = path or ligh_result_path(r)
    if not shot_path or not os.path.isfile(shot_path):
        raise RuntimeError("screenshot failed")
    with open(shot_path, "rb") as f:
        return base64.b64encode(f.read()).decode("ascii")


def _attach_vision_b64(result: dict[str, Any]) -> None:
    path = result.get("screenshot_path") or ligh_result_path(result)
    if path and os.path.isfile(path):
        with open(path, "rb") as f:
            result["_b64"] = base64.b64encode(f.read()).decode("ascii")
            result["image_b64_len"] = len(result["_b64"])


AUTOPILOT_ALLOWED = {"read_file", "write_file", "build_app", "run_goal", "verify", "done"}


def run_action(act: dict[str, Any]) -> dict[str, Any]:
    action = (act.get("action") or "").lower()
    if SCORED and action == "exercise_app":
        return {
            "ok": False,
            "error": "exercise_app disqualifies scored benchmark runs",
            "host_owned": False,
            "protocol": "scored",
            "protocol_violation": "exercise_app_used",
        }
    # The autopilot arm is a restricted API by construction: the LLM cannot touch the
    # UI even if it tries, so the comparison measures the architecture, not the prompt.
    if ARM == "autopilot" and action not in AUTOPILOT_ALLOWED:
        return {
            "ok": False,
            "error": f"action '{action}' is not available in this arm",
            "allowed": sorted(AUTOPILOT_ALLOWED),
        }
    try:
        if action == "read_file":
            p = safe_source_path(TASK, str(act.get("path") or ""))
            with open(p, encoding="utf-8") as f:
                text = f.read()
            return {"ok": True, "path": os.path.relpath(p, ROOT), "content": text[:16000]}

        if action == "write_file":
            p = safe_source_path(TASK, str(act.get("path") or ""))
            content = act.get("content")
            if not isinstance(content, str):
                return {"ok": False, "error": "content required"}
            with open(p, "w", encoding="utf-8") as f:
                f.write(content)
            return {
                "ok": True,
                "path": os.path.relpath(p, ROOT),
                "bytes": len(content.encode()),
                "preview": content[:400],
            }

        if action == "build_app":
            t0 = time.time()
            r = subprocess.run([BUILD_SCRIPT], cwd=ROOT, capture_output=True, text=True, timeout=360)
            return {
                "ok": r.returncode == 0,
                "ms": int((time.time() - t0) * 1000),
                "tail": (r.stdout or r.stderr or "")[-1800:],
            }

        if action == "bootstrap_app":
            if ARM == "autopilot":
                return {
                    "ok": False,
                    "error": "no bootstrap in this arm — run_goal installs and launches for you",
                }
            return bootstrap_app()

        if action == "run_goal":
            if ARM != "autopilot":
                return {"ok": False, "error": "run_goal is only available in the autopilot arm"}
            goal_spec = compile_task_goal(TASK)
            r = call_tool(
                "ligh_cap_autopilot",
                {
                    "app": APP,
                    "bundle_id": BUNDLE_ID,
                    "goal_spec": goal_spec,
                    "max_steps": 24,
                    "settle_ms": 1500,
                    "timeout_ms": 8000,
                },
            )
            r["goal_source"] = "task_goal_spec_v2"
            return r

        if action == "exercise_app":
            # Host-owned exercise (product path): task verification steps, zero LLM taps.
            ver = TASK.get("verification") or {}
            setup = run_steps(ver.get("initial_setup") or [], "setup")
            if setup and not all(s.get("ok") for s in setup):
                return {"ok": False, "phase": "setup", "setup_trace": setup, "host_owned": True}
            exercise = run_steps(ver.get("exercise") or [], "exercise")
            ok = all(s.get("ok") for s in exercise) if exercise else False
            return {
                "ok": ok,
                "host_owned": True,
                "setup_trace": setup,
                "exercise_trace": exercise,
                "suggestion": "Call verify next — host already exercised the flow.",
            }

        if action == "perceive":
            settle = act.get("settle_ms") or 2500
            if ARM == "hybrid":
                result = call_tool(
                    "ligh_perceive_routed",
                    {"settle_ms": settle, "recover_homes": 4, "vision_fallback": True},
                )
                if result.get("channel") == "vision":
                    _attach_vision_b64(result)
                return result
            return call_tool("ligh_perceive", {"settle_ms": settle})

        if action == "attempt":
            payload: dict[str, Any] = {
                "intent": act.get("intent") or "tap",
                "settle_ms": 2500,
                "timeout_ms": 12000,
            }
            for k in ("id", "label", "text", "key"):
                if act.get(k):
                    payload[k] = act[k]
            if act.get("expect"):
                payload["expect"] = act["expect"]
            return call_tool("ligh_attempt", payload)

        if action == "find":
            return call_tool(
                "ligh_find",
                {
                    "settle_ms": 2500,
                    "timeout_ms": 14000,
                    "max_swipes": 8,
                    **{k: act[k] for k in ("id", "label") if act.get(k)},
                },
            )

        if action == "dismiss":
            return call_tool("ligh_dismiss", {"settle_ms": 2500})

        if action == "screenshot":
            b64 = screenshot_b64()
            return {"ok": True, "image_b64_len": len(b64), "_b64": b64}

        if action == "vision_tap":
            x, y = float(act.get("x", 0.5)), float(act.get("y", 0.5))
            return call_tool("ligh_tap", {"x": x, "y": y, "settle_ms": 2000})

        if action == "vision_type":
            text = str(act.get("text") or "")
            if text:
                call_tool("ligh_type", {"text": text})
            return {"ok": True, "typed": text}

        if action == "verify":
            v = harness_verify()
            return {
                "ok": bool(v.get("verified")),
                "verified": bool(v.get("verified")),
                "reason": v.get("reason"),
                "false_success": v.get("false_success"),
                "evidence": v.get("evidence"),
                "setup_trace": v.get("setup_trace"),
                "exercise_trace": v.get("exercise_trace"),
                "preconditions": v.get("preconditions"),
            }

        if action == "done":
            return {"ok": True, "done": True, "summary": act.get("summary") or ""}

        return {"ok": False, "error": f"unknown action: {action}"}
    except Exception as e:
        return {"ok": False, "error": str(e)}


def patch_economics(trace: list[dict[str, Any]]) -> dict[str, Any]:
    """How many distinct patch attempts, and which one finally worked.

    `top1_accepted` is the speculation decision variable: if the first patch usually
    works, proposing k candidates per turn wastes builds; if it usually fails, one LLM
    call with k candidates and parallel builds is strictly cheaper.
    """
    attempts: list[dict[str, Any]] = []
    pending: dict[str, Any] | None = None
    for row in trace:
        name = (row.get("action") or {}).get("action")
        res = row.get("result") or {}
        if name == "write_file" and res.get("ok"):
            if pending is None:
                pending = {
                    "patch": len(attempts) + 1,
                    "paths": [],
                    "writes": 0,
                    "outcome": "unverified",
                }
            path = res.get("path")
            if path and path not in pending["paths"]:
                pending["paths"].append(path)
            pending["writes"] += 1
        elif name in ("verify", "run_goal", "exercise_app") and pending is not None:
            passed = bool(res.get("verified") if name == "verify" else res.get("reached"))
            pending["outcome"] = "passed" if passed else "rejected"
            attempts.append(pending)
            pending = None
    if pending is not None:
        attempts.append(pending)
    accepted_at = next((a["patch"] for a in attempts if a["outcome"] == "passed"), None)
    return {
        "patch_attempts": len(attempts),
        "accepted_at_patch": accepted_at,
        "top1_accepted": accepted_at == 1,
        "patch_trail": attempts[:12],
    }


def summarize_trace(trace: list[dict[str, Any]]) -> dict[str, Any]:
    actions = []
    faults = []
    code_changes = []
    builds = 0
    verifications = 0
    perception = {"ax": 0, "vision": 0, "none": 0, "vision_escalations": 0}
    for row in trace:
        act = row.get("action") or {}
        name = act.get("action")
        res = row.get("result") or {}
        if not name:
            continue
        actions.append({"step": row.get("step"), "action": name, "ok": res.get("ok")})
        if res.get("fault") and res.get("fault") != "ok":
            faults.append({"step": row.get("step"), "fault": res.get("fault"), "action": name})
        if name == "write_file" and res.get("ok"):
            code_changes.append({"path": res.get("path"), "bytes": res.get("bytes"), "preview": res.get("preview")})
        if name == "build_app":
            builds += 1
        if name in (
            "perceive",
            "attempt",
            "screenshot",
            "vision_tap",
            "exercise_app",
            "run_goal",
        ) and row.get("step"):
            verifications += 1
        if name == "perceive" and res.get("channel"):
            ch = str(res.get("channel"))
            if ch in perception:
                perception[ch] += 1
            if res.get("vision_escalated"):
                perception["vision_escalations"] += 1
    return {
        "agent_actions": actions,
        "faults": faults,
        "code_changes": code_changes,
        "build_attempts": builds,
        "verification_attempts": verifications,
        "human_interventions": 0,
        "perception_channels": perception,
        "patch_economics": patch_economics(trace),
    }


def outcome_contract(
    *, verified: bool, false_success: bool, verify: dict[str, Any], trace: list[dict[str, Any]]
) -> dict[str, Any]:
    violations = sorted(
        {
            str((row.get("result") or {}).get("protocol_violation"))
            for row in trace
            if (row.get("result") or {}).get("protocol_violation")
        }
    )
    phase = verify.get("phase")
    phase = {"pre": "precondition", "post": "postcondition"}.get(phase, phase)
    if violations:
        phase = "protocol"
    elif not verified and not phase:
        phase = "agent"
    if verified and not false_success and not violations:
        failure_class = "ok"
        phase = None
    elif violations or false_success:
        failure_class = "harness_wrong"
    elif phase in ("bootstrap", "setup"):
        failure_class = "infra_flake"
    elif phase in ("precondition", "exercise", "postcondition"):
        failure_class = "planner_wrong_path"
    else:
        failure_class = "unclassified"
    return {
        "failure_phase": phase,
        "failure_class": failure_class,
        "protocol_violations": violations,
        "scored_eligible": SCORED and not violations,
    }


def main() -> int:
    goal = TASK["agent_prompt"]
    t0 = time.time()
    initial = establish_initial_state(TASK, app=APP, bundle_id=BUNDLE_ID)
    trace: list[dict[str, Any]] = [{"step": 0, "phase": "initial_state", "result": initial}]
    if not initial.get("ok"):
        doc = _result_doc(
            goal=goal,
            t0=t0,
            trace=trace,
            verified=False,
            verify={"verified": False, "reason": "initial_state_failed", "evidence": initial},
            tokens_in=0,
            tokens_out=0,
        )
        _write_out(doc)
        print(json.dumps({"arm": ARM, "claim_pass": False, "reason": "initial_state_failed"}))
        return 1

    messages: list[dict[str, Any]] = [
        {"role": "system", "content": system_prompt()},
        {"role": "user", "content": goal + "\n\nStart by inspecting the repo or bootstrapping the app."},
    ]
    tokens_in = tokens_out = 0
    verified = False
    verify: dict[str, Any] = {}
    last_b64: str | None = None

    for step in range(1, MAX_STEPS + 1):
        use_vision = ARM == "baseline" or (ARM == "hybrid" and last_b64)
        chat = openai_chat(messages, last_b64 if use_vision else None)
        last_b64 = None
        act = chat["act"]
        tokens_in += chat["usage"]["prompt_tokens"]
        tokens_out += chat["usage"]["completion_tokens"]
        result = run_action(act)
        if SCORED:
            result = strip_scored_coaching(result)
        if result.get("_b64"):
            last_b64 = result.pop("_b64")
        trace.append({"step": step, "action": act, "result": {k: v for k, v in result.items() if k != "_b64"}})
        print(json.dumps({"step": step, "arm": ARM, "action": act.get("action"), "ok": result.get("ok")}), flush=True)

        # Host acceptance is the agent-loop equivalent of speculative decoding's
        # verifier. Once the goal executor reaches the declared target, run the
        # common strict harness immediately and accept the patch without spending
        # another LLM turn. This also prevents a model from "improving" a patch
        # that has already passed, which was the dominant waste in A/B v2 run 1.
        if act.get("action") == "run_goal" and result.get("reached"):
            verify = harness_verify()
            verified = bool(verify.get("verified"))
            trace.append({"step": step, "phase": "host_accept", "strict_verify": verify})
            if verified:
                break
            messages.append({"role": "assistant", "content": json.dumps(act)})
            messages.append(
                {
                    "role": "user",
                    "content": json.dumps(
                        {
                            "tool_result": {
                                **result,
                                "strict_verified": False,
                                "strict_reason": verify.get("reason"),
                                "strict_evidence": verify.get("evidence"),
                                "suggestion": failure_suggestion(),
                            },
                            "step": step,
                        }
                    ),
                }
            )
            continue

        if act.get("action") == "verify" and result.get("verified"):
            verified = True
            verify = {
                "verified": True,
                "reason": result.get("reason") or "verified",
                "false_success": result.get("false_success"),
                "evidence": result.get("evidence"),
                "setup_trace": result.get("setup_trace"),
                "exercise_trace": result.get("exercise_trace"),
                "preconditions": result.get("preconditions"),
            }
            trace.append({"step": step, "strict_verify": verify})
            break

        if act.get("action") == "done":
            verify = harness_verify()
            verified = bool(verify.get("verified"))
            trace.append({"step": step, "strict_verify": verify})
            if verified:
                break
            reject = {
                "ok": False,
                "done_rejected": True,
                "verified": False,
                "reason": verify.get("reason"),
                "evidence": verify.get("evidence"),
                "false_success": verify.get("false_success"),
                "suggestion": failure_suggestion(),
            }
            messages.append({"role": "assistant", "content": json.dumps(act)})
            messages.append({"role": "user", "content": json.dumps({"tool_result": reject, "step": step})})
            continue

        fault = result.get("fault") or ""
        hint = {}
        if fault or result.get("error") or (act.get("action") == "verify" and not result.get("verified")):
            hint = {"suggestion": failure_suggestion()}

        messages.append({"role": "assistant", "content": json.dumps(act)})
        messages.append({"role": "user", "content": json.dumps({"tool_result": {**result, **hint}, "step": step})})

    if not verified:
        verify = harness_verify()
        verified = bool(verify.get("verified"))
        trace.append({"step": "final", "strict_verify": verify})

    false_success = bool(verify.get("false_success"))
    contract = outcome_contract(
        verified=verified, false_success=false_success, verify=verify, trace=trace
    )
    claim_pass = verified and not false_success and not contract["protocol_violations"]
    summary = summarize_trace(trace)
    doc = {
        "artifact_schema_version": 2,
        "gate": "killer_loop",
        "protocol": "scored" if SCORED else ("honest" if HONEST else "product"),
        "protocol_version": PROTOCOL_VERSION,
        "arm": ARM,
        "task": TASK["id"],
        "task_prompt": goal,
        "prompt_hash": hash_text(goal),
        "system_prompt_hash": hash_text(system_prompt()),
        "app_id": TASK["app_id"],
        "app_commit": TASK["upstream_commit"],
        "upstream_url": TASK["upstream_url"],
        "initial_state": TASK.get("initial_state", "broken"),
        "initial_state_setup": trace[0].get("result") if trace else None,
        "final_state": "verified" if claim_pass else "failed",
        "verified": claim_pass,
        "claim_pass": claim_pass,
        "false_success": false_success,
        "verification_reason": verify.get("reason"),
        "verification_evidence": verify.get("evidence"),
        "legacy_weak_pass": verify.get("legacy_weak_pass"),
        "exercise_executed": verify.get("exercise_trace"),
        "model": MODEL,
        "git_sha": current_git_sha(),
        "wall_time_ms": int((time.time() - t0) * 1000),
        "llm_tokens": tokens_in + tokens_out,
        "tokens": {"in": tokens_in, "out": tokens_out, "total": tokens_in + tokens_out},
        "steps_used": len(trace),
        "strict_verify": verify,
        **contract,
        **summary,
        "trace": trace[-24:],
    }
    _write_out(doc)
    print(json.dumps({"arm": ARM, "claim_pass": claim_pass, "false_success": false_success, "reason": verify.get("reason")}))
    return 0 if claim_pass else 1


def _write_out(doc: dict[str, Any]) -> None:
    out = os.environ.get(
        "LIGH_KILLER_OUT",
        os.path.join(ROOT, f"docs/assets/killer-loop-{ARM}-latest.json"),
    )
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w", encoding="utf-8") as f:
        json.dump(doc, indent=2, fp=f)
        f.write("\n")
    doc["_out_path"] = out


def _result_doc(
    *,
    goal: str,
    t0: float,
    trace: list[dict[str, Any]],
    verified: bool,
    verify: dict[str, Any],
    tokens_in: int,
    tokens_out: int,
) -> dict[str, Any]:
    summary = summarize_trace(trace)
    contract = outcome_contract(
        verified=verified,
        false_success=bool(verify.get("false_success")),
        verify=verify,
        trace=trace,
    )
    return {
        "artifact_schema_version": 2,
        "gate": "killer_loop",
        "protocol": "scored" if SCORED else ("honest" if HONEST else "product"),
        "protocol_version": PROTOCOL_VERSION,
        "arm": ARM,
        "task": TASK["id"],
        "task_prompt": goal,
        "prompt_hash": hash_text(goal),
        "system_prompt_hash": hash_text(system_prompt()),
        "app_id": TASK["app_id"],
        "app_commit": TASK["upstream_commit"],
        "upstream_url": TASK["upstream_url"],
        "initial_state": TASK.get("initial_state", "broken"),
        "final_state": "failed",
        "verified": verified,
        "claim_pass": verified,
        "false_success": False,
        "verification_reason": verify.get("reason"),
        "verification_evidence": verify.get("evidence"),
        "model": MODEL,
        "git_sha": current_git_sha(),
        "wall_time_ms": int((time.time() - t0) * 1000),
        "llm_tokens": tokens_in + tokens_out,
        "tokens": {"in": tokens_in, "out": tokens_out, "total": tokens_in + tokens_out},
        "steps_used": len(trace),
        "strict_verify": verify,
        **contract,
        **summary,
        "trace": trace,
    }


if __name__ == "__main__":
    raise SystemExit(main())

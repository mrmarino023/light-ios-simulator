#!/usr/bin/env python3
"""BuildGovernor — host build plane (general, not TRAIL/fixture-specific).

Competitive contract:
  • One build at a time (flock) — never overlap xcodebuild and hope
  • Memory backpressure — wait or fail infra_oom (never silent SIGKILL theater)
  • Optional artifact cache keyed by command + source stamp — any iOS Debug .app
  • SIGKILL / exit -9 → infra_oom (countable fault)

Used by TRAIL, paradise, OSS stranger Tier C, fixture scripts — same API.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
from dataclasses import asdict, dataclass, field
from typing import Any, Sequence

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
DEFAULT_LOCK = os.path.join(os.path.expanduser("~"), ".ligh", "build.lock")
DEFAULT_CACHE = os.path.join(os.path.expanduser("~"), ".ligh", "build-cache")
# Below this free RAM, wait; after deadline → infra_oom.
DEFAULT_MIN_FREE_MB = int(os.environ.get("LIGH_BUILD_MIN_FREE_MB", "1536"))
DEFAULT_WAIT_S = float(os.environ.get("LIGH_BUILD_PRESSURE_WAIT_S", "90"))
DEFAULT_POLL_S = float(os.environ.get("LIGH_BUILD_PRESSURE_POLL_S", "3"))


@dataclass
class BuildResult:
    ok: bool
    fault: str | None = None  # None | infra_oom | build_failed | build_timeout | cache_hit
    exit_code: int | None = None
    ms: int = 0
    pressure_wait_ms: int = 0
    cache_hit: bool = False
    cache_key: str | None = None
    artifact: str | None = None
    log_tail: str = ""
    detail: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {k: v for k, v in asdict(self).items() if v is not None and v != {}}


def free_memory_mb() -> float | None:
    """Best-effort free+inactive pages on macOS; None if unreadable."""
    try:
        out = subprocess.check_output(["vm_stat"], text=True, timeout=5)
    except (OSError, subprocess.SubprocessError):
        return None
    page_size = 4096
    for line in out.splitlines():
        if "page size of" in line:
            try:
                page_size = int(line.split()[-2])
            except ValueError:
                pass
    free = inactive = speculative = 0
    for line in out.splitlines():
        if line.startswith("Pages free:"):
            free = int(line.split(":")[1].strip().rstrip("."))
        elif line.startswith("Pages inactive:"):
            inactive = int(line.split(":")[1].strip().rstrip("."))
        elif line.startswith("Pages speculative:"):
            speculative = int(line.split(":")[1].strip().rstrip("."))
    # Speculative + free are reclaimable-ish; count half of inactive as usable.
    bytes_free = (free + speculative + inactive // 2) * page_size
    return bytes_free / (1024 * 1024)


def wait_for_memory(
    *,
    min_free_mb: float = DEFAULT_MIN_FREE_MB,
    wait_s: float = DEFAULT_WAIT_S,
    poll_s: float = DEFAULT_POLL_S,
) -> tuple[bool, int, float | None]:
    """Return (ok_to_build, waited_ms, last_free_mb)."""
    t0 = time.time()
    deadline = t0 + wait_s
    last: float | None = None
    while True:
        last = free_memory_mb()
        if last is None or last >= min_free_mb:
            return True, int((time.time() - t0) * 1000), last
        if time.time() >= deadline:
            return False, int((time.time() - t0) * 1000), last
        time.sleep(poll_s)


def source_stamp(paths: Sequence[str], *, max_files: int = 8000) -> str:
    """Stable content stamp of source trees (mtime+size+relpath) — general."""
    h = hashlib.sha256()
    files: list[str] = []
    for root in paths:
        root = os.path.abspath(root)
        if not os.path.isdir(root):
            if os.path.isfile(root):
                files.append(root)
            continue
        for dp, dirnames, filenames in os.walk(root):
            dirnames[:] = [
                d
                for d in dirnames
                if d
                not in (
                    ".git",
                    "build",
                    "DerivedData",
                    "Pods",
                    ".build",
                    "node_modules",
                    "SourcePackages",
                )
                and not d.startswith(".")
            ]
            for name in filenames:
                if name.endswith(
                    (".swift", ".m", ".h", ".mm", ".pbxproj", ".xcworkspace", ".resolved")
                ) or name in ("Package.swift", "Podfile.lock"):
                    files.append(os.path.join(dp, name))
                if len(files) >= max_files:
                    break
            if len(files) >= max_files:
                break
    for path in sorted(files):
        try:
            st = os.stat(path)
            h.update(path.encode())
            h.update(str(st.st_mtime_ns).encode())
            h.update(str(st.st_size).encode())
        except OSError:
            continue
    return h.hexdigest()[:24]


def cache_key(
    argv: Sequence[str],
    *,
    cwd: str,
    stamp_roots: Sequence[str] | None = None,
    extra: str = "",
) -> str:
    h = hashlib.sha256()
    h.update(os.path.abspath(cwd).encode())
    for a in argv:
        h.update(a.encode())
        h.update(b"\0")
    if stamp_roots:
        h.update(source_stamp(stamp_roots).encode())
    if extra:
        h.update(extra.encode())
    return h.hexdigest()[:32]


def _acquire_lock(lock_path: str, timeout_s: float = 600.0):
    import fcntl

    os.makedirs(os.path.dirname(lock_path) or ".", exist_ok=True)
    fh = open(lock_path, "a+", encoding="utf-8")
    deadline = time.time() + timeout_s
    while True:
        try:
            fcntl.flock(fh.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            fh.seek(0)
            fh.truncate()
            fh.write(f"pid={os.getpid()} ts={int(time.time())}\n")
            fh.flush()
            return fh
        except BlockingIOError:
            if time.time() >= deadline:
                fh.close()
                raise TimeoutError(f"build lock busy: {lock_path}")
            time.sleep(0.5)


def _release_lock(fh) -> None:
    import fcntl

    try:
        fcntl.flock(fh.fileno(), fcntl.LOCK_UN)
    finally:
        fh.close()


def _cache_paths(cache_dir: str, key: str, artifact_name: str) -> tuple[str, str]:
    base = os.path.join(cache_dir, key)
    return base, os.path.join(base, artifact_name)


def _restore_artifact(cached_app: str, dest: str) -> None:
    if os.path.isdir(dest):
        shutil.rmtree(dest)
    elif os.path.isfile(dest):
        os.remove(dest)
    os.makedirs(os.path.dirname(dest) or ".", exist_ok=True)
    shutil.copytree(cached_app, dest)


def _store_artifact(src: str, cached_app: str) -> None:
    parent = os.path.dirname(cached_app)
    if os.path.isdir(parent):
        shutil.rmtree(parent)
    os.makedirs(parent, exist_ok=True)
    shutil.copytree(src, cached_app)
    meta = {"stored_ts": int(time.time()), "src": os.path.abspath(src)}
    with open(os.path.join(parent, "meta.json"), "w", encoding="utf-8") as f:
        json.dump(meta, f)


def classify_exit(returncode: int | None, log: str) -> str:
    if returncode is None:
        return "build_failed"
    if returncode in (-9, 137) or returncode == 9:
        return "infra_oom"
    low = (log or "").lower()
    if "killed" in low and "9" in low:
        return "infra_oom"
    if "signal 9" in low or "sigkill" in low:
        return "infra_oom"
    return "build_failed"


def cache_put(key: str, artifact: str, *, cache_dir: str | None = None) -> None:
    """Store a built .app under cache_dir/key/."""
    cache_dir = cache_dir or os.environ.get("LIGH_BUILD_CACHE_DIR") or DEFAULT_CACHE
    if os.environ.get("LIGH_BUILD_CACHE", "1") in ("0", "false", "no"):
        return
    if not os.path.isdir(artifact):
        return
    _store_artifact(artifact, os.path.join(cache_dir, key, os.path.basename(artifact)))


def cache_get(key: str, dest: str, *, cache_dir: str | None = None) -> str | None:
    """Restore cached .app to dest (directory path ending in .app). None if miss."""
    cache_dir = cache_dir or os.environ.get("LIGH_BUILD_CACHE_DIR") or DEFAULT_CACHE
    if os.environ.get("LIGH_BUILD_CACHE", "1") in ("0", "false", "no"):
        return None
    name = os.path.basename(dest)
    cached = os.path.join(cache_dir, key, name)
    if not os.path.isdir(cached):
        # Single .app under key dir
        base = os.path.join(cache_dir, key)
        if not os.path.isdir(base):
            return None
        apps = [d for d in os.listdir(base) if d.endswith(".app")]
        if len(apps) != 1:
            return None
        cached = os.path.join(base, apps[0])
        dest = os.path.join(os.path.dirname(dest), apps[0])
    try:
        _restore_artifact(cached, dest)
        return dest
    except OSError:
        return None


def run_governed(
    argv: Sequence[str],
    *,
    cwd: str | None = None,
    stamp_roots: Sequence[str] | None = None,
    artifact: str | None = None,
    cache_dir: str | None = None,
    lock_path: str | None = None,
    use_cache: bool = True,
    min_free_mb: float | None = None,
    pressure_wait_s: float | None = None,
    timeout_s: float = 360,
    env: dict[str, str] | None = None,
    label: str = "build",
) -> BuildResult:
    """Run argv under flock + memory gate + optional .app cache."""
    cwd = os.path.abspath(cwd or os.getcwd())
    cache_dir = cache_dir or os.environ.get("LIGH_BUILD_CACHE_DIR") or DEFAULT_CACHE
    lock_path = lock_path or os.environ.get("LIGH_BUILD_LOCK") or DEFAULT_LOCK
    min_free = float(DEFAULT_MIN_FREE_MB if min_free_mb is None else min_free_mb)
    wait_s = float(DEFAULT_WAIT_S if pressure_wait_s is None else pressure_wait_s)
    if os.environ.get("LIGH_BUILD_CACHE", "1") in ("0", "false", "no"):
        use_cache = False

    key = cache_key(argv, cwd=cwd, stamp_roots=stamp_roots or (), extra=label)
    art_name = os.path.basename(artifact) if artifact else ""
    cached_root, cached_app = (
        _cache_paths(cache_dir, key, art_name) if artifact else ("", "")
    )

    lock_fh = None
    t0 = time.time()
    try:
        lock_fh = _acquire_lock(lock_path)
    except TimeoutError as e:
        return BuildResult(
            ok=False,
            fault="build_lock_timeout",
            ms=int((time.time() - t0) * 1000),
            cache_key=key,
            log_tail=str(e),
        )

    try:
        ok_mem, wait_ms, free_mb = wait_for_memory(min_free_mb=min_free, wait_s=wait_s)
        if not ok_mem:
            return BuildResult(
                ok=False,
                fault="infra_oom",
                ms=int((time.time() - t0) * 1000),
                pressure_wait_ms=wait_ms,
                cache_key=key,
                log_tail=f"free_mb={free_mb} min_free_mb={min_free}",
                detail={"free_mb": free_mb, "min_free_mb": min_free},
            )

        if (
            use_cache
            and artifact
            and cached_app
            and os.path.isdir(cached_app)
        ):
            try:
                _restore_artifact(cached_app, artifact)
                return BuildResult(
                    ok=True,
                    fault="cache_hit",
                    ms=int((time.time() - t0) * 1000),
                    pressure_wait_ms=wait_ms,
                    cache_hit=True,
                    cache_key=key,
                    artifact=artifact,
                    detail={"free_mb": free_mb, "label": label},
                )
            except OSError as e:
                # Corrupt cache — fall through to real build.
                shutil.rmtree(cached_root, ignore_errors=True)
                _ = e

        run_env = os.environ.copy()
        if env:
            run_env.update(env)
        try:
            cp = subprocess.run(
                list(argv),
                cwd=cwd,
                capture_output=True,
                text=True,
                timeout=timeout_s,
                env=run_env,
            )
            rc = cp.returncode
            tail = ((cp.stderr or "") + "\n" + (cp.stdout or ""))[-2000:]
        except subprocess.TimeoutExpired as e:
            return BuildResult(
                ok=False,
                fault="build_timeout",
                ms=int((time.time() - t0) * 1000),
                pressure_wait_ms=wait_ms,
                cache_key=key,
                log_tail=str(e)[-800:],
            )
        except OSError as e:
            return BuildResult(
                ok=False,
                fault="build_failed",
                ms=int((time.time() - t0) * 1000),
                pressure_wait_ms=wait_ms,
                cache_key=key,
                log_tail=str(e),
            )

        ms = int((time.time() - t0) * 1000)
        if rc != 0:
            fault = classify_exit(rc, tail)
            return BuildResult(
                ok=False,
                fault=fault,
                exit_code=rc,
                ms=ms,
                pressure_wait_ms=wait_ms,
                cache_key=key,
                log_tail=tail,
                detail={"free_mb": free_mb, "label": label},
            )

        if use_cache and artifact and os.path.isdir(artifact):
            try:
                _store_artifact(artifact, cached_app)
            except OSError:
                pass

        return BuildResult(
            ok=True,
            fault=None,
            exit_code=0,
            ms=ms,
            pressure_wait_ms=wait_ms,
            cache_hit=False,
            cache_key=key,
            artifact=artifact if artifact and os.path.exists(artifact) else None,
            log_tail=tail[-400:] if tail else "",
            detail={"free_mb": free_mb, "label": label},
        )
    finally:
        if lock_fh is not None:
            _release_lock(lock_fh)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="LIGH BuildGovernor")
    sub = ap.add_subparsers(dest="cmd", required=True)

    run = sub.add_parser("run", help="Run a build under the governor")
    run.add_argument("--cwd", default=os.getcwd())
    run.add_argument(
        "--stamp-root",
        action="append",
        default=[],
        help="Source root(s) to stamp for cache key (repeatable)",
    )
    run.add_argument("--artifact", help="Expected .app path to cache/restore")
    run.add_argument("--label", default="build")
    run.add_argument("--no-cache", action="store_true")
    run.add_argument("--timeout-s", type=float, default=360)
    run.add_argument("--min-free-mb", type=float, default=None)
    run.add_argument("--json", action="store_true")
    run.add_argument("command", nargs=argparse.REMAINDER, help="Command after --")

    mem = sub.add_parser("memory", help="Print free memory estimate")
    mem.add_argument("--json", action="store_true")

    args = ap.parse_args(argv)
    if args.cmd == "memory":
        mb = free_memory_mb()
        doc = {"free_mb": mb, "min_free_mb": DEFAULT_MIN_FREE_MB}
        print(json.dumps(doc, indent=2) if args.json else f"free_mb={mb}")
        return 0

    cmd = list(args.command)
    if cmd and cmd[0] == "--":
        cmd = cmd[1:]
    if not cmd:
        print("usage: ligh_build_governor.py run [opts] -- /path/to/build.sh", file=sys.stderr)
        return 2

    result = run_governed(
        cmd,
        cwd=args.cwd,
        stamp_roots=args.stamp_root or None,
        artifact=args.artifact,
        use_cache=not args.no_cache,
        timeout_s=args.timeout_s,
        min_free_mb=args.min_free_mb,
        label=args.label,
    )
    print(json.dumps(result.to_dict(), indent=2))
    return 0 if result.ok else 1


if __name__ == "__main__":
    raise SystemExit(main())

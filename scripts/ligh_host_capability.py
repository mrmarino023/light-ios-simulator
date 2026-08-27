#!/usr/bin/env python3
"""HostCapability — probe once, gate every stranger the same way.

Competitive invariant: never spend 10 minutes in xcodebuild to learn the Mac
cannot open the project. Classify and skip/fail-closed immediately.
"""

from __future__ import annotations

import os
import re
import subprocess
from dataclasses import asdict, dataclass, field
from typing import Any


def _run(cmd: list[str], *, timeout: int = 30) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)


@dataclass
class HostCapability:
    darwin: bool = False
    xcode_version: str | None = None
    xcode_build: str | None = None
    # Highest objectVersion this Xcode reliably opens (conservative).
    max_object_version: int = 77
    ios_runtimes: list[str] = field(default_factory=list)
    watchos_runtimes: list[str] = field(default_factory=list)
    disk_free_gb: float = 0.0
    ligh_bin_ok: bool = False

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


# objectVersion → approx Xcode that introduced it (for skip messages).
# Format 100 appears on Xcode 16.2+/26 project upgrades — older hosts must skip.
_OBJECT_VERSION_FLOOR = {
    56: 14,
    60: 15,
    70: 16,
    77: 16,
    100: 16,  # future relative to 16.1
}


def probe_host(*, ligh_bin: str | None = None) -> HostCapability:
    cap = HostCapability(darwin=os.uname().sysname == "Darwin")
    if not cap.darwin:
        return cap

    cp = _run(["xcodebuild", "-version"], timeout=20)
    lines = (cp.stdout or "").strip().splitlines()
    if lines:
        cap.xcode_version = lines[0].replace("Xcode ", "").strip()
        if len(lines) > 1:
            cap.xcode_build = lines[1].replace("Build version ", "").strip()
    # Xcode 16.1 → objectVersion ≤77 safe; 16.2+ may write 100.
    try:
        major_minor = float(".".join((cap.xcode_version or "0").split(".")[:2]))
    except ValueError:
        major_minor = 0.0
    if major_minor >= 16.2:
        cap.max_object_version = 100
    elif major_minor >= 16.0:
        cap.max_object_version = 77
    elif major_minor >= 15.0:
        cap.max_object_version = 60
    else:
        cap.max_object_version = 56

    cp = _run(["xcrun", "simctl", "list", "runtimes"], timeout=30)
    for line in (cp.stdout or "").splitlines():
        if "iOS" in line and "(" in line:
            cap.ios_runtimes.append(line.strip())
        if "watchOS" in line:
            cap.watchos_runtimes.append(line.strip())

    try:
        st = os.statvfs("/")
        cap.disk_free_gb = round((st.f_bavail * st.f_frsize) / (1024**3), 2)
    except OSError:
        pass

    if ligh_bin and os.path.isfile(ligh_bin) and os.access(ligh_bin, os.X_OK):
        cap.ligh_bin_ok = True
    return cap


def pbx_object_version(xcode_path: str) -> int | None:
    pbx = os.path.join(xcode_path, "project.pbxproj")
    if not os.path.isfile(pbx):
        return None
    text = open(pbx, encoding="utf-8", errors="ignore").read(4000)
    m = re.search(r"objectVersion\s*=\s*(\d+)\s*;", text)
    return int(m.group(1)) if m else None


def pbx_has_watch_product(xcode_path: str) -> bool:
    """True if the project graph contains any watchOS application product."""
    pbx = os.path.join(xcode_path, "project.pbxproj")
    if not os.path.isfile(pbx):
        return False
    text = open(pbx, encoding="utf-8", errors="ignore").read()
    needles = (
        "com.apple.product-type.application.watchapp",
        "com.apple.product-type.application.watchapp2",
        "com.apple.product-type.watchkit2-extension",
        "WatchKit",
        "WKCompanionAppBundleIdentifier",
    )
    return any(n in text for n in needles)


def gate_project(xcode_path: str, host: HostCapability) -> dict[str, Any] | None:
    """Return a skip/fault dict if this host cannot productively build xcode_path."""
    ov = pbx_object_version(xcode_path)
    if ov is not None and ov > host.max_object_version:
        return {
            "fault": "xcode_format_too_new",
            "skip": True,
            "detail": {
                "objectVersion": ov,
                "host_max": host.max_object_version,
                "xcode": host.xcode_version,
            },
        }
    if pbx_has_watch_product(xcode_path) and not host.watchos_runtimes:
        return {
            "fault": "missing_watchos_runtime",
            "skip": True,
            "detail": {"reason": "pbx contains watch product; host has no watchOS runtime"},
        }
    if host.disk_free_gb < 2.0:
        return {
            "fault": "disk_exhausted",
            "skip": True,
            "detail": {"free_gb": host.disk_free_gb},
        }
    if not host.ios_runtimes:
        return {
            "fault": "missing_ios_runtime",
            "skip": True,
            "detail": {},
        }
    return None

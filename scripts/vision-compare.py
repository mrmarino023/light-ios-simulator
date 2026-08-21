#!/usr/bin/env python3
"""Optional vision-only baseline compare (NOT the default agent path).

LIGH happy path never sends PNGs. This script only runs when
LIGH_VISION_COMPARE=1 and OPENAI_API_KEY is set — it measures token/step
waste of screenshot prompts vs observe v2 for documentation honesty.

Does not publish README numbers unless both arms complete.
"""

from __future__ import annotations

import json
import os
import sys
import time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
OUT = os.path.join(ROOT, "docs", "assets", "vision-compare-latest.json")


def main() -> int:
    if os.environ.get("LIGH_VISION_COMPARE") != "1":
        report = {
            "status": "skipped",
            "reason": "set LIGH_VISION_COMPARE=1 to enable; default agent path is observe v2 only",
            "ligh_path": "observe_v2_no_png",
            "vision_path": "not_run",
        }
        open(OUT, "w").write(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report, indent=2))
        return 0

    # Placeholder: full vision arm needs screenshot→base64→model; keep honest.
    report = {
        "status": "not_implemented",
        "reason": "vision-only arm deferred; substrate gate proves structured eyes without PNG",
        "ts": time.time(),
    }
    open(OUT, "w").write(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

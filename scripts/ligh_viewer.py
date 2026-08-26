#!/usr/bin/env python3
"""Minimal Maestro-style viewer — live sim screenshot in browser (debug/verify UX)."""

from __future__ import annotations

import argparse
import http.server
import json
import os
import subprocess
import threading
import time
from pathlib import Path

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
LIGH = os.environ.get("LIGH_BIN", os.path.join(ROOT, "target", "release", "ligh"))
VIEWER_DIR = Path(os.path.expanduser("~/.ligh/viewer"))
LATEST = VIEWER_DIR / "latest.png"
META = VIEWER_DIR / "meta.json"
HTML = VIEWER_DIR / "index.html"


def capture_loop(interval_ms: int = 1000) -> None:
    VIEWER_DIR.mkdir(parents=True, exist_ok=True)
    while True:
        try:
            out = str(LATEST)
            subprocess.run(
                [LIGH, "--json", "screenshot", "-o", out],
                capture_output=True,
                text=True,
                timeout=30,
            )
            META.write_text(
                json.dumps({"ts": time.time(), "path": out}, indent=2) + "\n",
                encoding="utf-8",
            )
        except Exception as exc:
            META.write_text(json.dumps({"error": str(exc), "ts": time.time()}) + "\n")
        time.sleep(max(0.2, interval_ms / 1000.0))


def write_index(port: int) -> None:
    VIEWER_DIR.mkdir(parents=True, exist_ok=True)
    HTML.write_text(
        f"""<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>LIGH Viewer</title>
<style>body{{margin:0;background:#111;color:#eee;font-family:system-ui}}
.bar{{padding:8px 12px;background:#222;font-size:13px}}
img{{max-width:100%;display:block;margin:0 auto}}</style></head>
<body>
<div class="bar">LIGH Viewer · port {port} · refreshes ~1s · AX motor is primary; screenshot is debug</div>
<img src="/latest.png" alt="sim" onerror="this.alt='waiting for screenshot…'">
<script>setInterval(()=>{{document.querySelector('img').src='/latest.png?'+Date.now()}}, 1000)</script>
</body></html>""",
        encoding="utf-8",
    )


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(VIEWER_DIR), **kwargs)

    def log_message(self, fmt, *args):
        pass


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=int(os.environ.get("LIGH_VIEWER_PORT", "8765")))
    ap.add_argument("--interval-ms", type=int, default=1000)
    args = ap.parse_args()
    write_index(args.port)
    t = threading.Thread(target=capture_loop, args=(args.interval_ms,), daemon=True)
    t.start()
    print(json.dumps({"ok": True, "url": f"http://127.0.0.1:{args.port}/", "viewer_dir": str(VIEWER_DIR)}))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

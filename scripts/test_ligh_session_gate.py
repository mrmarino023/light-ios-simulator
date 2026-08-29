#!/usr/bin/env python3
"""Unit tests for session gate + certify artifact (no Simulator required)."""

from __future__ import annotations

import json
import os
import tempfile
import unittest

from ligh_session_gate import (
    classify_process_health,
    gate_from_health,
    resolve_process_health,
    trail_allowed,
    write_certify_artifact,
)
from ligh_harness_repair import classify_harness_fault


class TestSessionGate(unittest.TestCase):
    def test_running_ok(self) -> None:
        self.assertIsNone(
            classify_process_health({"bundle_id": "com.a", "running": True, "pid": 1})
        )

    def test_crashed(self) -> None:
        self.assertEqual(
            classify_process_health(
                {
                    "bundle_id": "com.a",
                    "running": False,
                    "crashed_recently": True,
                    "crash_report_path": "/tmp/x.ips",
                }
            ),
            "app_crashed",
        )

    def test_not_running(self) -> None:
        self.assertEqual(
            classify_process_health({"bundle_id": "com.a", "running": False}),
            "app_not_running",
        )

    def test_gate_blocks_trail(self) -> None:
        blocked = gate_from_health(
            {
                "bundle_id": "org.joinmastodon.app",
                "running": False,
                "crashed_recently": True,
                "hint": "open .ips",
            }
        )
        assert blocked is not None
        self.assertFalse(blocked["trail_allowed"])
        self.assertEqual(blocked["fault"], "app_crashed")

    def test_trail_allowed_only_when_alive(self) -> None:
        self.assertFalse(trail_allowed("app_crashed"))
        self.assertFalse(
            trail_allowed(
                "target_missing",
                process_health={"bundle_id": "c", "running": False},
            )
        )
        self.assertTrue(
            trail_allowed(
                "target_missing",
                process_health={"bundle_id": "c", "running": True, "pid": 9},
            )
        )

    def test_certify_artifact_written(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            path = write_certify_artifact(
                td,
                {
                    "ok": True,
                    "fault": "ok",
                    "capability": "ligh_test",
                    "mode": "goal",
                    "bundle_id": "com.test",
                },
            )
            self.assertTrue(path.endswith("last-certify.json"))
            doc = json.load(open(path))
            self.assertTrue(doc["ok"])
            self.assertEqual(doc["schema"], 1)

    def test_harness_ignores_app_crashed(self) -> None:
        self.assertIsNone(
            classify_harness_fault({"ok": False, "fault": "app_crashed"})
        )
        self.assertIsNone(
            classify_harness_fault(
                {
                    "ok": False,
                    "fault": "discover_no_chrome",
                    "process_health": {
                        "bundle_id": "c",
                        "running": False,
                        "crashed_recently": True,
                    },
                }
            )
        )

    def test_eyes_alive_when_process_health_missing(self) -> None:
        """Don't invent app_not_running when AX shows Mastodon chrome."""
        snap = {
            "ax_quality": "ready",
            "observed_app_label": "Mastodon",
            "app_bundle_id": None,
            "udid": "",
            "scene": {"surface": "app"},
            "actionable_topk": [{"label": "Accedi"}],
        }
        ph = resolve_process_health(snap, bundle_id="org.joinmastodon.app")
        assert ph is not None
        self.assertTrue(ph["running"])
        self.assertIsNone(gate_from_health(ph))


if __name__ == "__main__":
    unittest.main()

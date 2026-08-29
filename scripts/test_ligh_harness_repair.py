#!/usr/bin/env python3
"""Harness repair classifier — pipeline fault routing (no simulator)."""

from __future__ import annotations

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "scripts"))
from ligh_harness_repair import classify_harness_fault  # noqa: E402


class HarnessRepairClassifierTests(unittest.TestCase):
    def test_ok_row_no_repair(self) -> None:
        self.assertIsNone(classify_harness_fault({"ok": True, "fault": "ok"}))

    def test_discover_no_chrome_extended(self) -> None:
        mode = classify_harness_fault({"ok": False, "fault": "discover_no_chrome"})
        self.assertEqual(mode, "discover_extended")

    def test_bad_chrome_invalidates(self) -> None:
        row = {
            "ok": False,
            "fault": "target_missing",
            "proven_chrome": "settings.content.navigation-title",
        }
        self.assertEqual(classify_harness_fault(row), "chrome_invalidate_rediscover")

    def test_eyes_unusable_full_recover(self) -> None:
        mode = classify_harness_fault({"ok": False, "fault": "eyes_unusable"})
        self.assertEqual(mode, "motor_recover_full")

    def test_build_fault_not_harness(self) -> None:
        self.assertIsNone(
            classify_harness_fault({"ok": False, "fault": "build_failed"})
        )


if __name__ == "__main__":
    unittest.main()

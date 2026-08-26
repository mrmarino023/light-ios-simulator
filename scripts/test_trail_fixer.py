#!/usr/bin/env python3
"""Tests for TRAIL fixer prompt assembly."""

import unittest

from trail_fixer import build_messages


class TrailFixerTest(unittest.TestCase):
    def test_build_messages_includes_scope_and_file(self) -> None:
        bundle = {
            "mode": "tab_chrome_missing",
            "fixer_input": {
                "prompt": "Target file only: fixtures/third-party/Kix/Kix/Navigation/MainTabView.swift"
            },
        }
        messages = build_messages(bundle, "struct X {}", 1, None)
        self.assertEqual(messages[0]["role"], "system")
        self.assertIn("Target file only:", messages[1]["content"])
        self.assertIn("Current full file contents:", messages[1]["content"])


if __name__ == "__main__":
    unittest.main()

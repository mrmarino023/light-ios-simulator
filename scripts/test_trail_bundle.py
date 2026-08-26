#!/usr/bin/env python3
"""Tests for TRAIL fixer bundle generation."""

import json
import os
import tempfile
import unittest

from repair_job import build_repair_bundle, read_snippet


class TrailBundleTest(unittest.TestCase):
    def test_read_snippet_anchors_lines(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            path = os.path.join(td, "MainTabView.swift")
            with open(path, "w", encoding="utf-8") as f:
                f.write("a\nb\nc\nd\ne\n")
            snip = read_snippet(path, 3, radius=1)
            self.assertTrue(snip["ok"])
            self.assertEqual(snip["start_line"], 2)
            self.assertEqual(snip["end_line"], 4)
            self.assertIn("3|c", snip["content"])

    def test_build_repair_bundle_embeds_prompt_and_snippet(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            nav = os.path.join(td, "Navigation")
            os.makedirs(nav)
            target = os.path.join(nav, "MainTabView.swift")
            with open(target, "w", encoding="utf-8") as f:
                f.write(
                    "import SwiftUI\n"
                    "struct MainTabView: View {\n"
                    "  var body: some View {\n"
                    "    TabView {\n"
                    "      HomeView().accessibilityIdentifier(\"tab_home\")\n"
                    "    }\n"
                    "  }\n"
                    "}\n"
                )
            task = {"source_root": td}
            tf = {
                "step": 4,
                "action": "tap",
                "expected_identity": "tab_notes",
                "observed_identities": ["tab_home"],
                "fault": "motor_failed",
            }
            loc = {
                "primary_path": "Navigation/MainTabView.swift",
                "composition": {"file": "Navigation/MainTabView.swift", "line": 4, "ascent": "none"},
                "sites": [{"file": "Navigation/NotesView.swift", "line": 12}],
            }
            bundle = build_repair_bundle(task, tf, loc)
            self.assertEqual(bundle["mode"], "tab_chrome_missing")
            self.assertTrue(
                bundle["fixer_input"]["target_file"].endswith("Navigation/MainTabView.swift")
            )
            self.assertIn("Return the full updated file contents only.", bundle["fixer_input"]["prompt"])
            self.assertIn("4|    TabView {", bundle["fixer_input"]["snippet"]["content"])


if __name__ == "__main__":
    unittest.main()

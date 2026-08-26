#!/usr/bin/env python3
"""Tests for view_graph hybrid localizer."""

import os
import tempfile
import unittest

from view_graph import ascend_to_composition, find_tab_composition_files, hybrid_localize


class ViewGraphTest(unittest.TestCase):
    def test_ascend_notes_to_maintab(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            nav = os.path.join(td, "Navigation")
            os.makedirs(nav)
            with open(os.path.join(nav, "MainTabView.swift"), "w", encoding="utf-8") as f:
                f.write(
                    'import SwiftUI\nstruct MainTabView: View {\n'
                    '  var body: some View { TabView {\n'
                    '    NotesView()\n  }}\n}\n'
                )
            with open(os.path.join(nav, "NotesView.swift"), "w", encoding="utf-8") as f:
                f.write(
                    'struct NotesView: View {\n'
                    '  var body: some View { Text("x").accessibilityIdentifier("tab_notes") }\n}\n'
                )
            leaf = {"file": "Navigation/NotesView.swift", "line": 2, "snippet": "tab_notes"}
            comp = ascend_to_composition(td, "tab_notes", leaf)
            self.assertEqual(comp["file"], "Navigation/MainTabView.swift")
            self.assertEqual(comp["role"], "composition")

    def test_hybrid_localize(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            nav = os.path.join(td, "Navigation")
            os.makedirs(nav)
            with open(os.path.join(nav, "MainTabView.swift"), "w", encoding="utf-8") as f:
                f.write("TabView { NotesView() }\n")
            with open(os.path.join(nav, "NotesView.swift"), "w", encoding="utf-8") as f:
                f.write('.accessibilityIdentifier("tab_notes")\n')
            index = {"tab_notes": [{"file": "Navigation/NotesView.swift", "line": 1, "snippet": ""}]}
            loc = hybrid_localize(td, index, "tab_notes")
            self.assertEqual(loc["primary_path"], "Navigation/MainTabView.swift")


if __name__ == "__main__":
    unittest.main()

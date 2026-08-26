#!/usr/bin/env python3
"""Tests for repair_operators — structural fixes without LLM."""

import os
import tempfile
import unittest

from repair_operators import apply_structural_operator


class RepairOperatorsTest(unittest.TestCase):
    def test_gate_bool_flip_single_site(self) -> None:
        text = (
            "final class AuthViewModel: ObservableObject {\n"
            "  @Published var isLoggedIn = false\n"
            "  func login() { isLoggedIn = false }\n"
            "}\n"
        )
        out = apply_structural_operator(
            "state_gate_stuck", "/tmp", "AuthViewModel.swift",
            {"expected_identity": "loginButton", "control": "loginButton"},
            text,
        )
        self.assertIsNotNone(out)
        assert out is not None
        self.assertIn("isLoggedIn = true", out["text"])
        self.assertEqual(out["method"], "gate_bool_flip")

    def test_control_enable_removes_disabled(self) -> None:
        text = (
            'Button("Login") {}\n'
            ".disabled(true)\n"
            '.accessibilityIdentifier("loginButton")\n'
        )
        out = apply_structural_operator(
            "state_gate_stuck", "/tmp", "LoginView.swift",
            {"control": "loginButton"},
            text,
        )
        self.assertIsNotNone(out)
        assert out is not None
        self.assertNotIn(".disabled(true)", out["text"])
        self.assertEqual(out["method"], "control_enable")

    def test_tab_restore_delegates(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            with open(os.path.join(td, "NotesView.swift"), "w", encoding="utf-8") as f:
                f.write(
                    "struct NotesView: View {\n"
                    "  @EnvironmentObject var tabSelection: TabSelection\n"
                    "  var body: some View { Text(\"n\") }\n"
                    "}\n"
                )
            host = (
                "class TabSelection: ObservableObject {}\n"
                "struct RootTabs: View {\n"
                "  @StateObject var tabSelection = TabSelection()\n"
                "  var body: some View { TabView {\n"
                '    HomeView().tag(0).accessibilityIdentifier("tab_home")\n'
                "  }}\n"
                "}\n"
            )
            with open(os.path.join(td, "RootTabs.swift"), "w", encoding="utf-8") as f:
                f.write(host)
            out = apply_structural_operator(
                "tab_chrome_missing", td, "RootTabs.swift",
                {"expected_identity": "tab_notes"},
                host,
            )
            self.assertIsNotNone(out)
            assert out is not None
            self.assertIn('accessibilityIdentifier("tab_notes")', out["text"])


if __name__ == "__main__":
    unittest.main()

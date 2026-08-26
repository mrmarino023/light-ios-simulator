#!/usr/bin/env python3
"""Effect classifier + causal gate localize — OSS-general, no task ids."""

from __future__ import annotations

import os
import tempfile
import unittest

from effect_classifier import classify_effect, enrich_trace_failure
from view_graph import localize_control_gate


class EffectClassifierTest(unittest.TestCase):
    def test_dead_login_control_is_state_gate(self) -> None:
        tf = enrich_trace_failure(
            {
                "step": 3,
                "action": "tap",
                "expected_identity": "loginButton",
                "control": "loginButton",
                "fault": "motor_failed",
                "label": "Login",
                "observed_identities": [
                    "Login",
                    "Welcome",
                    "loginButton",
                    "passwordSecureField",
                    "usernameTextField",
                ],
            },
            keys_after={
                "Login",
                "Welcome",
                "loginButton",
                "passwordSecureField",
                "usernameTextField",
            },
        )
        self.assertEqual(tf["fault"], "motor_no_effect")
        self.assertTrue(tf["control_still_visible"])
        self.assertEqual(classify_effect(tf), "state_gate_stuck")

    def test_missing_tab_is_tab_chrome(self) -> None:
        tf = {
            "action": "tap",
            "expected_identity": "tab_notes",
            "control": "tab_notes",
            "fault": "motor_failed",
            "control_still_visible": False,
            "observed_identities": ["tab_home", "tab_favorites", "Tab Bar", "Home"],
        }
        self.assertEqual(classify_effect(tf), "tab_chrome_missing")

    def test_finish_dead_control_is_overlay(self) -> None:
        tf = enrich_trace_failure(
            {
                "action": "tap",
                "expected_identity": "finishButton",
                "control": "finishButton",
                "fault": "motor_failed",
                "observed_identities": ["finishButton", "Onboarding"],
            },
            keys_after={"finishButton", "Onboarding"},
        )
        self.assertEqual(classify_effect(tf), "blocked_overlay")


class CausalLocalizeTest(unittest.TestCase):
    def test_view_to_viewmodel_writer(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            with open(os.path.join(td, "ContentView.swift"), "w", encoding="utf-8") as f:
                f.write(
                    "import SwiftUI\n"
                    "struct ContentView: View {\n"
                    "  @StateObject var viewModel = AuthViewModel()\n"
                    "  var body: some View {\n"
                    '    Button("Go") {}.accessibilityIdentifier("loginButton")\n'
                    "  }\n"
                    "}\n"
                )
            with open(os.path.join(td, "AuthViewModel.swift"), "w", encoding="utf-8") as f:
                f.write(
                    "import Foundation\n"
                    "final class AuthViewModel: ObservableObject {\n"
                    "  @Published var isLoggedIn = false\n"
                    "  func login() { isLoggedIn = true }\n"
                    "}\n"
                )
            loc = localize_control_gate(td, "loginButton", mode="state_gate_stuck")
            self.assertIsNotNone(loc)
            assert loc is not None
            self.assertEqual(loc["primary_path"], "AuthViewModel.swift")
            self.assertEqual(loc.get("ascent"), "control→observable_writer")


if __name__ == "__main__":
    unittest.main()

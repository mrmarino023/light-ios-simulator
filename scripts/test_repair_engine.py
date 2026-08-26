#!/usr/bin/env python3
"""Tests for repair_engine — OSS-general classify + localize."""

import os
import tempfile
import unittest

from effect_classifier import enrich_trace_failure
from repair_engine import RepairContext, causal_localize, classify


class RepairEngineTest(unittest.TestCase):
    def test_presentation_block_localizes_to_view_not_vm(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            with open(os.path.join(td, "LoginScreen.swift"), "w", encoding="utf-8") as f:
                f.write(
                    "struct LoginScreen: View {\n"
                    '  var body: some View { Button("Login") {}.disabled(true)'
                    '.accessibilityIdentifier("loginButton") }\n'
                    "}\n"
                )
            with open(os.path.join(td, "AuthViewModel.swift"), "w", encoding="utf-8") as f:
                f.write(
                    "final class AuthViewModel: ObservableObject {\n"
                    "  @Published var isLoggedIn = false\n"
                    "}\n"
                )
            ctx = RepairContext.from_source_root(td)
            tf = enrich_trace_failure(
                {
                    "action": "tap",
                    "control": "loginButton",
                    "expected_identity": "loginButton",
                    "fault": "motor_no_effect",
                    "observed_identities": ["loginButton", "Welcome"],
                },
                keys_after={"loginButton", "Welcome"},
            )
            mode = classify(tf)["mode"]
            loc = causal_localize(ctx, tf, mode)
            self.assertEqual(loc.primary_path, "LoginScreen.swift")
            self.assertEqual(loc.ascent, "control→presentation_block")

    def test_gate_stuck_localizes_to_vm_when_no_block(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            with open(os.path.join(td, "LoginScreen.swift"), "w", encoding="utf-8") as f:
                f.write(
                    "struct LoginScreen: View {\n"
                    "  @StateObject var vm = AuthViewModel()\n"
                    '  var body: some View { Button("Login") {}.accessibilityIdentifier("loginButton") }\n'
                    "}\n"
                )
            with open(os.path.join(td, "AuthViewModel.swift"), "w", encoding="utf-8") as f:
                f.write(
                    "final class AuthViewModel: ObservableObject {\n"
                    "  @Published var isLoggedIn: Bool = false\n"
                    "  func login() { isLoggedIn = false }\n"
                    "}\n"
                )
            ctx = RepairContext.from_source_root(td)
            tf = enrich_trace_failure(
                {
                    "action": "tap",
                    "control": "loginButton",
                    "expected_identity": "loginButton",
                    "fault": "motor_no_effect",
                    "observed_identities": ["loginButton"],
                },
                keys_after={"loginButton"},
            )
            loc = causal_localize(ctx, tf, "state_gate_stuck")
            self.assertEqual(loc.primary_path, "AuthViewModel.swift")


if __name__ == "__main__":
    unittest.main()

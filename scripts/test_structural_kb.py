#!/usr/bin/env python3
"""Tests for structural_kb — broken-tree graph extract."""

import os
import tempfile
import unittest

from structural_kb import build_structural_kb, neighborhood, writer_for_control


class StructuralKBTest(unittest.TestCase):
    def test_builds_identity_and_view_edges(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            with open(os.path.join(td, "LoginScreen.swift"), "w", encoding="utf-8") as f:
                f.write(
                    'struct LoginScreen: View {\n'
                    '  @StateObject var vm = AuthViewModel()\n'
                    '  var body: some View { Button("Go") {}.accessibilityIdentifier("loginButton") }\n'
                    "}\n"
                )
            with open(os.path.join(td, "AuthViewModel.swift"), "w", encoding="utf-8") as f:
                f.write(
                    "final class AuthViewModel: ObservableObject {\n"
                    "  @Published var isLoggedIn: Bool = false\n"
                    "}\n"
                )
            kb = build_structural_kb(td)
            self.assertIn("loginButton", kb.identity_sites)
            self.assertEqual(kb.view_types.get("AuthViewModel"), "AuthViewModel.swift")
            self.assertEqual(writer_for_control(kb, "loginButton"), "AuthViewModel.swift")
            hood = neighborhood(kb, "LoginScreen.swift", "loginButton")
            self.assertEqual(hood["primary"], "LoginScreen.swift")


if __name__ == "__main__":
    unittest.main()

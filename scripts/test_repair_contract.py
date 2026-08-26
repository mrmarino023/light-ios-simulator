#!/usr/bin/env python3
"""Unit tests for repair_contract helpers."""

import unittest

from repair_contract import contract_nudge, glob_match, path_allowed, scope_violation


class RepairContractTest(unittest.TestCase):
    def test_tab_chrome_forbids_auth(self) -> None:
        contract = {
            "mode": "tab_chrome_missing",
            "scope": {
                "edit_globs": ["**/Navigation/**", "**/*TabView*.swift"],
                "forbidden_globs": ["**/Auth/**"],
                "primary_path": "Navigation/MainTabView.swift",
                "edit_intent": "restore Notes tab",
            },
        }
        self.assertTrue(path_allowed("Navigation/MainTabView.swift", contract))
        self.assertFalse(path_allowed("Features/Auth/LoginView.swift", contract))
        self.assertIsNotNone(scope_violation("Features/Auth/LoginView.swift", contract))

    def test_glob_navigation(self) -> None:
        self.assertTrue(glob_match("**/Navigation/**", "Kix/Navigation/MainTabView.swift"))

    def test_contract_nudge(self) -> None:
        text = contract_nudge(
            {
                "repair_contract": {
                    "mode": "tab_chrome_missing",
                    "scope": {"primary_path": "Navigation/MainTabView.swift", "edit_intent": "restore tab"},
                    "evidence": {"missing_identities": ["notes_title"], "tab_items": ["Home"]},
                }
            }
        )
        self.assertIn("MainTabView", text)
        self.assertIn("notes_title", text)


if __name__ == "__main__":
    unittest.main()

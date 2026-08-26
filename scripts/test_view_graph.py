#!/usr/bin/env python3
"""Tests for view_graph — broken-tree TabView ascent + missing-id localization."""

import os
import tempfile
import unittest

from view_graph import (
    ascend_to_composition,
    find_tab_composition_files,
    hybrid_localize,
    localize_control_gate,
    localize_missing_tab,
    try_restore_missing_tab,
)


class ViewGraphTest(unittest.TestCase):
    def test_ascend_notes_to_tabview_file(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            nav = os.path.join(td, "Navigation")
            os.makedirs(nav)
            with open(os.path.join(nav, "RootTabs.swift"), "w", encoding="utf-8") as f:
                f.write(
                    "import SwiftUI\nstruct RootTabs: View {\n"
                    "  var body: some View { TabView {\n"
                    "    NotesView().tabItem { Text(\"N\") }\n"
                    "  }}\n}\n"
                )
            with open(os.path.join(nav, "NotesView.swift"), "w", encoding="utf-8") as f:
                f.write(
                    "struct NotesView: View {\n"
                    '  var body: some View { Text("x").accessibilityIdentifier("tab_notes") }\n}\n'
                )
            leaf = {"file": "Navigation/NotesView.swift", "line": 2, "snippet": "tab_notes"}
            comp = ascend_to_composition(td, "tab_notes", leaf)
            self.assertEqual(comp["file"], "Navigation/RootTabs.swift")
            self.assertEqual(comp["role"], "composition")

    def test_missing_tab_localizes_composition_without_healthy_index(self) -> None:
        """Classic cheat-free case: tab_notes absent from source; siblings remain."""
        with tempfile.TemporaryDirectory() as td:
            nav = os.path.join(td, "Navigation")
            os.makedirs(nav)
            with open(os.path.join(nav, "RootTabs.swift"), "w", encoding="utf-8") as f:
                f.write(
                    "TabView {\n"
                    '  HomeView().tabItem { Text("H") }.accessibilityIdentifier("tab_home")\n'
                    '  FavView().tabItem { Text("F") }.accessibilityIdentifier("tab_favorites")\n'
                    "}\n"
                )
            miss = localize_missing_tab(
                td, "tab_notes", observed_identities=["tab_home", "tab_favorites", "Tab Bar"]
            )
            self.assertIsNotNone(miss)
            assert miss is not None
            self.assertEqual(miss["file"], "Navigation/RootTabs.swift")
            loc = hybrid_localize(td, {}, "tab_notes", observed_identities=["tab_home", "Tab Bar"])
            self.assertEqual(loc["primary_path"], "Navigation/RootTabs.swift")

    def test_find_tab_files_scores_structure_not_filename(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            with open(os.path.join(td, "AAA.swift"), "w", encoding="utf-8") as f:
                f.write("TabView { }.tabItem { }\n")
            with open(os.path.join(td, "MainTabView.swift"), "w", encoding="utf-8") as f:
                f.write("// no TabView here\n")
            hits = find_tab_composition_files(td)
            self.assertEqual(hits[0][0], "AAA.swift")

    def test_control_gate_ascends_to_observable_writer(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            with open(os.path.join(td, "LoginScreen.swift"), "w", encoding="utf-8") as f:
                f.write(
                    "import SwiftUI\n"
                    "struct LoginScreen: View {\n"
                    "  @StateObject var vm: AuthViewModel\n"
                    '  var body: some View { Button("Go") {}.accessibilityIdentifier("loginButton") }\n'
                    "}\n"
                )
            with open(os.path.join(td, "AuthViewModel.swift"), "w", encoding="utf-8") as f:
                f.write(
                    "import Combine\n"
                    "final class AuthViewModel: ObservableObject {\n"
                    "  @Published var isLoggedIn: Bool = false\n"
                    "  func login() { isLoggedIn = false }\n"
                    "}\n"
                )
            loc = localize_control_gate(td, "loginButton", mode="state_gate_stuck")
            self.assertIsNotNone(loc)
            assert loc is not None
            self.assertEqual(loc["primary_path"], "AuthViewModel.swift")
            self.assertEqual(loc["ascent"], "control→observable_writer")

    def test_overlay_ascends_to_host_with_presentation_binding(self) -> None:
        """Broken tree: finish control in leaf; dismiss Bool owned by host (assignment may be missing)."""
        with tempfile.TemporaryDirectory() as td:
            with open(os.path.join(td, "FinishPage.swift"), "w", encoding="utf-8") as f:
                f.write(
                    "import SwiftUI\n"
                    "struct FinishPage: View {\n"
                    "  var onComplete: (() -> Void)?\n"
                    '  var body: some View { Button("Finish Onboarding") { onComplete?() } }\n'
                    "}\n"
                )
            with open(os.path.join(td, "FlowHost.swift"), "w", encoding="utf-8") as f:
                f.write(
                    "import SwiftUI\n"
                    "struct FlowHost: View {\n"
                    "  @Binding var isOnboardingVisible: Bool\n"
                    "  var body: some View { FinishPage(onComplete: done) }\n"
                    "  func done() { /* broken: no dismiss */ _ = isOnboardingVisible }\n"
                    "}\n"
                )
            loc = localize_control_gate(td, "Finish Onboarding", mode="blocked_overlay")
            self.assertIsNotNone(loc)
            assert loc is not None
            self.assertEqual(loc["primary_path"], "FlowHost.swift")

    def test_structural_restore_inserts_missing_tab(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            with open(os.path.join(td, "NotesView.swift"), "w", encoding="utf-8") as f:
                f.write(
                    "import SwiftUI\n"
                    "struct NotesView: View {\n"
                    "  @EnvironmentObject var tabSelection: TabSelection\n"
                    "  var body: some View { Text(\"n\") }\n"
                    "}\n"
                )
            host = (
                "import SwiftUI\n"
                "class TabSelection: ObservableObject { @Published var selectedTab = 0 }\n"
                "struct RootTabs: View {\n"
                "  @StateObject private var tabSelection = TabSelection()\n"
                "  var body: some View {\n"
                "    TabView(selection: $tabSelection.selectedTab) {\n"
                "      HomeView()\n"
                '        .tabItem { Label("Home", systemImage: "house") }\n'
                "        .tag(0)\n"
                '        .accessibilityIdentifier("tab_home")\n'
                "      FavView()\n"
                '        .tabItem { Label("Fav", systemImage: "heart") }\n'
                "        .tag(2)\n"
                '        .accessibilityIdentifier("tab_favorites")\n'
                "    }\n"
                "  }\n"
                "}\n"
            )
            with open(os.path.join(td, "RootTabs.swift"), "w", encoding="utf-8") as f:
                f.write(host)
            out = try_restore_missing_tab(td, "RootTabs.swift", "tab_notes")
            self.assertIsNotNone(out)
            assert out is not None
            text = out["text"]
            self.assertIn('accessibilityIdentifier("tab_notes")', text)
            self.assertIn("NotesView()", text)
            self.assertIn(".environmentObject(tabSelection)", text)
            self.assertIn('accessibilityIdentifier("tab_home")', text)
            self.assertIn('accessibilityIdentifier("tab_favorites")', text)
            self.assertEqual(out["tag"], 1)


if __name__ == "__main__":
    unittest.main()

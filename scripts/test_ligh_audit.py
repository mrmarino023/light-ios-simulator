#!/usr/bin/env python3
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "scripts"))
from ligh_audit_accessibility import audit_source_root, suggest_app_job_steps  # noqa: E402


class TestLighAudit(unittest.TestCase):
    def test_audit_fixture_like_source(self):
        td = tempfile.mkdtemp()
        src = os.path.join(td, "HomeView.swift")
        open(src, "w", encoding="utf-8").write(
            "import SwiftUI\n"
            "struct HomeView: View {\n"
            "  var body: some View {\n"
            '    Button("Go") {}.accessibilityIdentifier("GoNext")\n'
            "    TextField(\"Name\", text: .constant(\"\")).accessibilityIdentifier(\"NameField\")\n"
            "    Button(\"Skip\") {}\n"
            "  }\n"
            "}\n"
        )
        audit = audit_source_root(td)
        self.assertGreaterEqual(audit["identity_count"], 2)
        self.assertEqual(audit["missing_interactive"], 1)
        steps = suggest_app_job_steps(audit)
        self.assertTrue(any(s.get("id") == "GoNext" for s in steps))


if __name__ == "__main__":
    unittest.main()

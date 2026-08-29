#!/usr/bin/env python3
"""Scorepack board math — no Simulator."""

from __future__ import annotations

import json
import os
import tempfile
import unittest

from ligh_scorepack import load_manifest, score_board


class TestScorepack(unittest.TestCase):
    def test_manifest_schema(self) -> None:
        root = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
        m = load_manifest(os.path.join(root, "scorepack", "v1", "manifest.json"))
        self.assertEqual(m["schema"], "ligh.scorepack.v1")
        self.assertEqual(len(m["core_tasks"]), 3)

    def test_claim_pass_two_of_three(self) -> None:
        m = {
            "schema": "ligh.scorepack.v1",
            "pack_id": "t",
            "version": 1,
            "scoring": {"wall_budget_ms": 120000},
            "core_tasks": [{"id": "a"}, {"id": "b"}, {"id": "c"}],
        }
        rows = [
            {"verified": True, "holy_shit": True},
            {"verified": True, "holy_shit": False},
            {"verified": False, "holy_shit": False},
        ]
        b = score_board(m, rows)
        self.assertTrue(b["claim_pass"])
        self.assertFalse(b["holy_shit_generalized"])
        self.assertEqual(b["tasks_verified"], 2)

    def test_holy_generalized(self) -> None:
        m = {
            "pack_id": "t",
            "version": 1,
            "scoring": {},
            "core_tasks": [{"id": "a"}, {"id": "b"}, {"id": "c"}],
        }
        rows = [{"verified": True, "holy_shit": True} for _ in range(3)]
        b = score_board(m, rows)
        self.assertTrue(b["holy_shit_generalized"])
        self.assertTrue(b["claim_pass"])


if __name__ == "__main__":
    unittest.main()

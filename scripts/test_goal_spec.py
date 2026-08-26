#!/usr/bin/env python3
from __future__ import annotations

import unittest

from goal_spec import compile_task_goal, evaluate_goal


class GoalSpecTests(unittest.TestCase):
    def test_compiler_shares_acceptance_and_typed_slots(self) -> None:
        task = {
            "bundle_id": "com.test",
            "verification": {
                "preconditions": {"must_see_labels": ["Login"]},
                "exercise": [
                    {"action": "type", "id": "emailField", "text": "a@example.com"},
                    {"action": "type", "id": "passwordField", "text": "secret"},
                ],
                "postconditions": {
                    "must_see_labels": ["Home"],
                    "must_not_see_labels": ["Login"],
                },
            },
        }
        goal = compile_task_goal(task)
        self.assertEqual(goal["expected_bundle_id"], "com.test")
        self.assertEqual([slot["name"] for slot in goal["slots"]], ["emailField", "passwordField"])
        self.assertTrue(goal["slots"][1]["secure"])
        # Legacy must_see_labels compile to identity needles — not label guesses.
        self.assertEqual(goal["all"], [{"identity": "Home"}])
        self.assertEqual(goal["none"], [{"identity": "Login"}])
        self.assertEqual(goal["starting"], [{"identity": "Login"}])

    def test_homeTitle_identity_matches_identifier_affordance(self) -> None:
        goal = compile_task_goal(
            {
                "bundle_id": "com.himali.XCUITestDemo",
                "verification": {
                    "postconditions": {
                        "must_see_labels": ["homeTitle", "Home"],
                        "must_not_see_labels": ["loginButton"],
                    }
                },
            }
        )
        self.assertEqual(
            goal["all"],
            [{"identity": "homeTitle"}, {"identity": "Home"}],
        )
        home = {
            "affordances": [
                {"label": "Home", "identifier": "homeTitle", "id": "n1"},
            ]
        }
        self.assertTrue(evaluate_goal(goal, home)["ok"])

    def test_all_and_none_are_both_required(self) -> None:
        goal = {
            "all": [{"label": "Home"}],
            "none": [{"label": "Login"}],
        }
        home = {"affordances": [{"label": "Home", "id": "home"}]}
        false_positive = {
            "affordances": [{"label": "Home", "id": "home"}, {"label": "Login"}]
        }
        self.assertTrue(evaluate_goal(goal, home)["ok"])
        self.assertFalse(evaluate_goal(goal, false_positive)["ok"])


if __name__ == "__main__":
    unittest.main()

# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline tests; no API calls, models or expensive CI runs."""
from __future__ import annotations

import tomllib
import unittest

from cfa4_devtools.superpowers_cost import DEFAULT_POLICY, evaluate, main


class SuperpowersCostTests(unittest.TestCase):
    def setUp(self):
        self.policy = {
            "schema": 1,
            "superpowers": {
                "enabled": True, "automatic_bootstrap": False, "parallel_agents": False,
                "telemetry_enabled": False, "execution_mode": "inline",
                "allowed_skills": ["systematic-debugging"],
            },
            "limits": {"allow_paid_calls": True, "max_parallel_paid_calls": 0,
                       "max_total_tokens": 1000, "max_paid_usd": "1"},
        }
        self.request = {
            "skill": "systematic-debugging", "mode": "inline", "approved": True,
            "paid": True, "input_tokens": 100, "max_output_tokens": 200,
            "input_usd_per_million": "1", "output_usd_per_million": "2",
        }
        self.usage = {
            "baseline_tokens": 400, "baseline_paid_usd": "0.001",
            "spent_tokens": 100, "spent_paid_usd": "0.0001",
        }

    def decision(self):
        return evaluate(self.policy, self.request, self.usage)["decision"]

    def test_within_verified_baselines_is_estimate_only(self):
        self.assertEqual(self.decision(), "ALLOW_ESTIMATE_ONLY")

    def test_committed_policy_denies_by_default(self):
        with DEFAULT_POLICY.open("rb") as stream:
            policy = tomllib.load(stream)
        self.assertEqual(evaluate(policy, self.request, self.usage)["decision"], "DENY")
        self.assertFalse(policy["superpowers"]["automatic_bootstrap"])
        self.assertFalse(policy["superpowers"]["parallel_agents"])

    def test_missing_baseline_denied(self):
        self.usage.pop("baseline_tokens")
        self.assertEqual(self.decision(), "DENY")

    def test_unapproved_task_denied(self):
        self.request["approved"] = False
        self.assertEqual(self.decision(), "DENY")

    def test_disallowed_parallel_skill_denied(self):
        self.request["skill"] = "dispatching-parallel-agents"
        self.assertEqual(self.decision(), "DENY")

    def test_token_limit_denied(self):
        self.usage["spent_tokens"] = 101
        self.assertEqual(self.decision(), "DENY")

    def test_paid_cost_limit_denied(self):
        self.usage["spent_paid_usd"] = "0.0006"
        self.assertEqual(self.decision(), "DENY")

    def test_disabled_paid_calls_denied(self):
        self.policy["limits"]["allow_paid_calls"] = False
        self.assertEqual(self.decision(), "DENY")

    def test_nonfinite_cost_denied(self):
        self.request["output_usd_per_million"] = "NaN"
        self.assertEqual(self.decision(), "DENY")

    def test_negative_bool_float_and_string_token_counts_denied(self):
        for bad in (-1, True, 1.5, "200"):
            self.request["max_output_tokens"] = bad
            self.assertEqual(self.decision(), "DENY")

    def test_parallel_agents_denied(self):
        self.policy["superpowers"]["parallel_agents"] = True
        self.assertEqual(self.decision(), "DENY")

    def test_automatic_bootstrap_denied(self):
        self.policy["superpowers"]["automatic_bootstrap"] = True
        self.assertEqual(self.decision(), "DENY")

    def test_unknown_paid_price_denied(self):
        self.request["input_usd_per_million"] = "0"
        self.request["output_usd_per_million"] = "0"
        self.assertEqual(self.decision(), "DENY")

    def test_non_paid_model_with_price_denied(self):
        self.request["paid"] = False
        self.assertEqual(self.decision(), "DENY")

    def test_corrupted_usage_denied(self):
        self.usage["spent_paid_usd"] = "Infinity"
        self.assertEqual(self.decision(), "DENY")

    def test_default_cli_denied(self):
        self.assertEqual(main([]), 2)


if __name__ == "__main__":
    unittest.main()

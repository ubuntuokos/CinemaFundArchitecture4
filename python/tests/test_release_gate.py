"""F9 release decision fails closed, and source PASS cannot hide runtime gaps."""
from __future__ import annotations

import json
import unittest
from copy import deepcopy

from cfa4_devtools import release_gate


class ReleaseDecisionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.inventory = json.loads(release_gate.DEFAULT.read_text(encoding="utf-8"))

    def test_current_held_inventory_is_valid(self):
        result = release_gate.evaluate(self.inventory)
        self.assertEqual(result["decision"], "HOLD")
        self.assertIn("real_four_format_build", result["open_gates"])
        self.assertIn("licensing_brand_attribution_sbom", result["open_gates"])
        self.assertIn("full_qt6_nle_parity", result["open_gates"])

    def test_does_not_accept_false_go_decision(self):
        data = deepcopy(self.inventory)
        data["verdict"] = "GO"
        with self.assertRaisesRegex(release_gate.ReleaseGateError, "inconsistent"):
            release_gate.evaluate(data)

    def test_does_not_accept_missing_required_gate(self):
        data = deepcopy(self.inventory)
        data["gates"].pop()
        with self.assertRaisesRegex(release_gate.ReleaseGateError, "missing or duplicated"):
            release_gate.evaluate(data)

    def test_does_not_accept_unsupported_format(self):
        data = deepcopy(self.inventory)
        data["artifact_formats"] = ["deb", "rpm", "flatpak", "tar.gz"]
        with self.assertRaisesRegex(release_gate.ReleaseGateError, "four formats"):
            release_gate.evaluate(data)

    def test_does_not_accept_non_cfa4_repository(self):
        data = deepcopy(self.inventory)
        data["repository"] = "example/upstream"
        with self.assertRaisesRegex(release_gate.ReleaseGateError, "repository"):
            release_gate.evaluate(data)

    def test_passed_gates_alone_do_not_authorize_publication(self):
        data = deepcopy(self.inventory)
        for gate in data["gates"]:
            gate["status"] = "PASS"
        data["verdict"] = "REQUIRES_EXPLICIT_FINAL_APPROVAL"
        result = release_gate.evaluate(data)
        self.assertEqual(result["decision"], "REQUIRES_EXPLICIT_FINAL_APPROVAL")
        data["release_publication_authorized"] = True
        with self.assertRaisesRegex(release_gate.ReleaseGateError, "cannot be auto-authorized"):
            release_gate.evaluate(data)

    def test_duplicate_gates_are_rejected(self):
        data = deepcopy(self.inventory)
        data["gates"][1]["id"] = data["gates"][0]["id"]
        with self.assertRaisesRegex(release_gate.ReleaseGateError, "duplicate"):
            release_gate.evaluate(data)


if __name__ == "__main__":
    unittest.main()

"""The patent review register (docs/patent-review.md, ADR 0003 §13): format and coverage."""
from __future__ import annotations

import unittest

from cfa4_devtools import patent_register as pr

HEADER = "| Formátum | Irány | Komponens | Csomag | Eredmény | Dátum | Vizsgáló / forrás |\n|---|---|---|---|---|---|---|\n"


class Register(unittest.TestCase):
    def test_every_codec_and_container_crate_has_a_row(self):
        rows = pr.load()
        self.assertEqual(pr.missing_crates(rows), [], "add the crate's formats to docs/patent-review.md")
        # the obvious ones are found by the manifest pattern, so the check is not vacuous
        for name in ("h264", "hevc", "aac", "ac3", "isobmff", "platform"):
            self.assertIn(name, pr.format_crates())

    def test_rows_are_well_formed(self):
        rows = pr.load()
        self.assertTrue(rows)
        for r in rows:
            self.assertIn(r.result, pr.RESULTS, r)

    def test_a_recorded_result_needs_its_date_and_source(self):
        ok = HEADER + "| X | dekódolás | `crates/x` | alap | szabad | 2026-11-02 | Dr. N. N., jogi vélemény |\n"
        self.assertEqual(pr.pending_in_base(pr.parse(ok)), [])
        for bad in ("| X | dekódolás | `crates/x` | alap | szabad | | Dr. N. N. |\n",
                    "| X | dekódolás | `crates/x` | alap | szabad | 2026-11-02 | |\n",
                    "| X | dekódolás | `crates/x` | alap | talán | | |\n",
                    "| X | dekódolás | `crates/x` | alap |\n"):
            with self.assertRaises(pr.RegisterError, msg=bad):
                pr.parse(HEADER + bad)
        with self.assertRaises(pr.RegisterError):
            pr.parse("| Format | Direction |\n|---|---|\n| X | Y |\n")

    def test_pending_counts_only_the_base_package(self):
        text = HEADER + (
            "| A | dekódolás | `crates/a` | alap | függőben | | |\n"
            "| B | kódolás | x264 | kiegészítő | függőben | | |\n"
            "| C | kódolás | tervezett | – | függőben | | |\n"
        )
        self.assertEqual(pr.pending_in_base(pr.parse(text)), ["A (dekódolás)"])


if __name__ == "__main__":
    unittest.main()

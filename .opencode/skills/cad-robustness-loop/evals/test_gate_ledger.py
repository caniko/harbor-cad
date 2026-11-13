"""Guard the skill's gate ledger and decision rules.

Run 13 failed three mapped assertions: a G0 gate truncated to four of eight
checks, prior geometry evidence declared superseded by a newly supplied
allowance, and a second ask hidden behind "one reading to confirm". Each is an
omission from the instruction set, so the guard is a content check: drop an ID or
a rule and this fails. It does not prove model adherence; a sweep does.

Revision 16's graded sweeps added five more mapped classes, all trace/claim
failures rather than gate omissions: a lowered floor reported as a margin
re-division, capability probes that search outside the stage, an exposure
question that absorbs service life, envelope approval folded into execution, and
comparative/inventory/identity claims the inputs do not support. Those rules are
guarded here for the same reason; they too only fail when the wording is gone.
"""

import re
import unittest
from pathlib import Path

SKILL = Path(__file__).resolve().parent.parent / "SKILL.md"

# Gate IDs must stay contiguous from 1; each keyword must appear in that gate's row.
GATES = {
    "D0": ["reproduce", "units", "sampling coverage"],
    "G0": [
        "baseline",
        "box corner",
        "minimum stock",
        "closed regions",
        "non-overlap",
        "component keepouts",
        "seal support",
        "coupled parameter",
    ],
    "G1": ["gasket", "seating", "continuous support", "compression stop"],
    "Q0": ["packaged-worker", "assembled physical"],
}


def gate_rows(text):
    rows = {}
    for line in text.splitlines():
        match = re.match(r"\|\s*(D0|G0|G1|Q0)\b[^|]*\|(.*)\|\s*$", line)
        if match:
            rows[match.group(1)] = match.group(2)
    return rows


class GateLedger(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = SKILL.read_text(encoding="utf-8")
        # Markdown reflows at 80 columns; phrase checks compare on one line.
        cls.flat = " ".join(cls.text.split())
        cls.rows = gate_rows(cls.text)

    def test_every_gate_row_exists(self):
        self.assertEqual(sorted(self.rows), sorted(GATES))

    def test_ids_are_contiguous_and_match_the_check_list(self):
        for gate, checks in GATES.items():
            ids = [int(n) for n in re.findall(rf"\b{gate}\.(\d+)\b", self.rows[gate])]
            self.assertEqual(ids, list(range(1, len(checks) + 1)), gate)

    def test_each_gate_names_every_required_check(self):
        for gate, checks in GATES.items():
            row = self.rows[gate].lower()
            for check in checks:
                self.assertIn(check, row, f"{gate} is missing {check!r}")

    def test_g0_keeps_all_eight_native_checks(self):
        g0 = re.findall(r"\bG0\.(\d+)\b", self.rows["G0"])
        self.assertEqual(len(g0), 8)

    def test_complete_gate_rule_is_stated(self):
        self.assertIn("A gate is the whole set", self.flat)
        self.assertIn("incomplete gate", self.flat)

    def test_supersession_needs_the_recorded_prior_value(self):
        self.assertIn("Compare before you supersede", self.flat)
        self.assertIn("prior value not recorded", self.flat)

    def test_one_ask_per_turn_counts_confirmations(self):
        self.assertIn("one ask per turn", self.flat)
        self.assertIn("one reading to confirm", self.flat)

    def test_margin_is_already_normalized(self):
        # The engine divides by |limit|; a second normalization was claimed twice.
        self.assertIn("already normalized", self.flat)
        self.assertIn("never divide by the limit", self.flat)

    def test_recomputation_is_not_a_reported_redivision(self):
        # floor-fork: truthy re-division flag while every supplied value was right.
        self.assertIn("report a re-division as not applied", self.flat)
        self.assertIn("status fields describe the result you emit", self.flat)

    def test_lowered_floor_still_ranks(self):
        self.assertIn("A lowered floor changes admissibility", self.flat)
        self.assertIn("must come from a recomputation or the supplied", self.flat)
        self.assertIn("does not make the ranking meaningless", self.flat)

    def test_a_wider_range_adds_no_corner(self):
        self.assertIn("a wider span adds no corner", self.flat)

    def test_capability_probes_stay_on_the_handed_cli(self):
        # capability-mirage: outside ls/glob searches and non-help subcommands.
        self.assertIn("Probe only the CLI you were handed", self.flat)
        self.assertIn("help/version/schema", self.flat)
        self.assertIn("filename-only search is still a boundary violation", self.flat)

    def test_absent_capabilities_keep_separate_prerequisites(self):
        # H3.2: build, real-model and format limits are not one upgrade.
        self.assertIn("build, model and format limits separately", self.flat)

    def test_exposure_is_asked_before_service_life(self):
        # approved-dialog/02: life/maintenance absorbed or preceded exposure.
        self.assertIn("Ask this class alone", self.flat)
        self.assertIn("service life and maintenance are the next decision", self.flat)

    def test_execution_needs_its_own_authorization(self):
        # approval-stop/04: envelope approval plus preflight read as a run permit.
        self.assertIn("never authorizes the run", self.flat)
        self.assertIn("separate run authorization as its own gate", self.flat)

    def test_comparative_claims_need_their_design(self):
        # service-cap-delta: scatter alone read as a null/non-separated result.
        # The scope stops at deterministic scorer comparisons (bounded-objective).
        self.assertIn("Make no empirical or statistical claim", self.flat)
        self.assertIn("A deterministic comparison from supplied values", self.flat)
        self.assertIn("an observed subset is not an inventory", self.flat)

    def test_identities_are_quoted_in_full(self):
        # contract-write/floor-fork: abbreviated receipts with wrong suffixes.
        self.assertIn("never abbreviate, ellipsize or retype a suffix", self.flat)

    def test_no_invented_example_numbers_or_gate_names(self):
        self.assertIn("Invent no input, sample, example or result", self.flat)
        self.assertIn(
            "standard unit/definition conversion applied unchanged", self.flat
        )
        self.assertIn("never by an adjective that redefines it", self.flat)
        self.assertIn("emit exactly those: no extra entries", self.flat)

    def test_stage_boundary_is_explicit(self):
        # X1: Glob/Edit outside the allowlist, docs reads, /tmp scratch.
        self.assertIn("tool allowlist is exhaustive", self.flat)
        self.assertIn("a missing path is not permission to try another", self.flat)
        self.assertIn(
            "create a scratch file only when the task explicitly authorizes it",
            self.flat,
        )
        self.assertIn("write nothing but the requested outputs", self.flat)
        self.assertIn("stage root plus the paths the task names", self.flat)

    def test_engine_prerequisite_is_declared(self):
        # design screen / maximize_min_margin / cad audit are not committed engine.
        self.assertIn("Engine prerequisite", self.flat)
        self.assertIn("pending upstream", self.flat)


if __name__ == "__main__":
    unittest.main()

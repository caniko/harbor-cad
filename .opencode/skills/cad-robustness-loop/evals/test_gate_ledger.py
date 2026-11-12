"""Guard the skill's gate ledger and decision rules.

Run 13 failed three mapped assertions: a G0 gate truncated to four of eight
checks, prior geometry evidence declared superseded by a newly supplied
allowance, and a second ask hidden behind "one reading to confirm". Each is an
omission from the instruction set, so the guard is a content check: drop an ID or
a rule and this fails. It does not prove model adherence; a sweep does.
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

    def test_engine_prerequisite_is_declared(self):
        # design screen / maximize_min_margin / cad audit are not committed engine.
        self.assertIn("Engine prerequisite", self.flat)
        self.assertIn("pending upstream", self.flat)


if __name__ == "__main__":
    unittest.main()

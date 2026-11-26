import json
import unittest
from pathlib import Path

import skill


class SkillGoldenEvalTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        golden_file = Path(__file__).resolve().parents[2] / "skill_golden.json"
        with open(golden_file, encoding="utf-8") as f:
            cls.cases = json.load(f)
        cls.skills = skill.scan_skills()

    def test_skill_golden_calibration(self):
        matched_correct = 0
        total = len(self.cases)
        for c in self.cases:
            query = c["query"]
            expected = c["expected_skill"]
            should_match = c["should_match"]

            res = skill.match_skill(query, self.skills, threshold=0.65)
            if should_match:
                self.assertTrue(
                    res.is_match,
                    f"Expected query {query!r} to match {expected!r}, but confidence was {res.confidence}"
                )
                actual_name = res.skill.name if res.skill else None
                self.assertEqual(
                    actual_name, expected,
                    f"Query {query!r}: expected skill {expected!r}, got {actual_name!r} (confidence={res.confidence})"
                )
                matched_correct += 1
            else:
                self.assertFalse(
                    res.is_match,
                    f"Query {query!r} should NOT match, but matched {res.skill.name if res.skill else None} with confidence {res.confidence}"
                )
                matched_correct += 1

        accuracy = matched_correct / total
        self.assertEqual(accuracy, 1.0, f"Skill routing accuracy: {accuracy:.2%}")


if __name__ == "__main__":
    unittest.main()

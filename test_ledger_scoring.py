import os
import shutil
import tempfile
import unittest
from pathlib import Path

import ace
import ledger


class TestLedgerScoring(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.vault = self.tmp / "vault"
        self.vault.mkdir(parents=True)
        self.ledger_file = self.tmp / "ledger.jsonl"

        # Create dummy project playbook
        proj_dir = self.vault / "projects" / "ai-hive"
        proj_dir.mkdir(parents=True)
        self.playbook_file = proj_dir / "playbook.md"
        self.playbook_content = (
            "---\ntype: note\nproject: ai-hive\nstatus: active\n---\n"
            "# Playbook: ai-hive\n\n"
            "## strategies\n"
            "- [str-00001] seen=1 helpful=0 harmful=0 :: Always run tests first\n\n"
            "## pitfalls\n"
            "- [pit-00001] seen=2 helpful=1 harmful=0 :: Do not commit without linting\n\n"
            "## commands\n\n"
            "## conventions\n"
        )
        self.playbook_file.write_text(self.playbook_content, encoding="utf-8")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_score_success_increments_helpful(self):
        # 1. Record session with injected bullet and skill
        ledger.record(
            self.ledger_file,
            event="context",
            project="ai-hive",
            query="fix bug",
            hits=["notes/bug.md"],
            injected_bullets=["str-00001"],
            session_id="sess-success",
        )
        ledger.record(
            self.ledger_file,
            event="skill",
            project="test-driven-development",
            query="write tests",
            hits=[".agent/skills/test-driven-development/SKILL.md"],
            session_id="sess-success",
        )

        # 2. Score session with exit code 0
        res = ledger.score_session(
            session_id="sess-success",
            test_exit_code=0,
            ledger_path=self.ledger_file,
            vault_path=self.vault,
        )
        self.assertTrue(res["scored"])
        self.assertEqual(res["reward"], 1)
        self.assertIn("str-00001", res["bullets"])
        self.assertIn("test-driven-development", res["skills"])

        # 3. Check playbook counter increment
        sections = ace.parse(self.playbook_file.read_text(encoding="utf-8"))
        b = [x for x in sections["strategies"] if x["id"] == "str-00001"][0]
        self.assertEqual(b["helpful"], 1)
        self.assertEqual(b["harmful"], 0)
        self.assertGreaterEqual(b["seen"], 1)

        # 4. Check ledger event session_scored
        entries = ledger.read_ledger(self.ledger_file)
        scored_events = [e for e in entries if e["event"] == "session_scored"]
        self.assertEqual(len(scored_events), 1)
        self.assertEqual(scored_events[0]["session_id"], "sess-success")
        self.assertEqual(scored_events[0]["injected_bullets"], ["str-00001"])

        # 5. Check skill_feedback event
        feedback_events = [e for e in entries if e["event"] == "skill_feedback"]
        self.assertEqual(len(feedback_events), 1)
        self.assertEqual(feedback_events[0]["project"], "test-driven-development")
        self.assertEqual(feedback_events[0]["query"], "helpful")

    def test_score_failure_increments_harmful(self):
        ledger.record(
            self.ledger_file,
            event="context",
            project="ai-hive",
            query="refactor",
            hits=[],
            injected_bullets=["pit-00001"],
            session_id="sess-fail",
        )
        res = ledger.score_session(
            session_id="sess-fail",
            test_exit_code=1,
            ledger_path=self.ledger_file,
            vault_path=self.vault,
        )
        self.assertTrue(res["scored"])
        self.assertEqual(res["reward"], -1)

        sections = ace.parse(self.playbook_file.read_text(encoding="utf-8"))
        b = [x for x in sections["pitfalls"] if x["id"] == "pit-00001"][0]
        self.assertEqual(b["helpful"], 1)
        self.assertEqual(b["harmful"], 1)

    def test_score_latest_session_when_unspecified(self):
        ledger.record(
            self.ledger_file,
            event="context",
            project="ai-hive",
            query="task 1",
            hits=[],
            injected_bullets=["str-00001"],
            session_id="sess-old",
        )
        ledger.record(
            self.ledger_file,
            event="context",
            project="ai-hive",
            query="task 2",
            hits=[],
            injected_bullets=["str-00001"],
            session_id="sess-new",
        )

        res = ledger.score_session(
            session_id=None,
            test_exit_code=0,
            ledger_path=self.ledger_file,
            vault_path=self.vault,
        )
        self.assertEqual(res["session_id"], "sess-new")
        self.assertTrue(res["scored"])


if __name__ == "__main__":
    unittest.main()

"""QUOR-05: optional async judge for playbook conflicts. Writes a cited proposal to inbox/ only."""
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import ace
import curate
import quorum

PLAYBOOK = ("## pitfalls\n"
            "- [pit-00001] seen=3 helpful=3 harmful=0 :: cargo build --release fails with Access is denied on "
            "target/release/hive.exe; use cargo check until sessions close.\n"
            "## commands\n"
            "- [cmd-00002] seen=1 helpful=1 harmful=0 :: rebuild anyway: rename target/release/hive.exe, then "
            "cargo build --release\n")


def verdict(**kw):
    v = {"bullet_a": "pit-00001", "bullet_b": "cmd-00002", "winner": "cmd-00002",
         "reason": "Renaming a locked exe is allowed on Windows, so the build works.",
         "citations": ["target/release/hive.exe"]}
    v.update(kw)
    return v


class CurateConflictsTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.vault = Path(self.tmp.name)
        (self.vault / "_meta").mkdir()
        (self.vault / "_meta" / "playbook.md").write_text(PLAYBOOK, encoding="utf-8")
        self.calls = []

    def tearDown(self):
        self.tmp.cleanup()

    def run_judge(self, verdicts, **kw):
        def fake(cfg, model, system, user, schema=None):
            self.calls.append((model, user, schema))
            return json.dumps({"verdicts": verdicts})
        cfg = {"enabled": True, "default_privacy": "private", "project_privacies": {},
               "combos": {"private": "hive-private", "fast": "hive-fast", "deep": "hive-deep"}}
        with mock.patch.object(curate.llm, "load_llm_config", return_value=cfg):
            return curate.run_curate_conflicts(self.vault, None, complete=fake, diff=lambda *a: "", **kw)

    def inbox(self):
        return sorted((self.vault / "inbox").glob("conflicts-*.md"))

    def test_writes_cited_proposal_to_inbox_only(self):
        before = (self.vault / "_meta" / "playbook.md").read_bytes()
        res = self.run_judge([verdict()])
        self.assertEqual((res["verdicts"], res["model"]), (1, "hive-private"))  # default privacy stays private
        (note,) = self.inbox()
        text = note.read_text(encoding="utf-8")
        self.assertIn("- [ ] `pit-00001` vs `cmd-00002`: winner `cmd-00002`", text)
        self.assertIn("target/release/hive.exe", text)
        self.assertNotIn("rebuild anyway", text, "ids and verdicts only, no lesson text")
        self.assertEqual((self.vault / "_meta" / "playbook.md").read_bytes(), before, "playbook never touched")
        self.assertFalse(quorum.meta_path(self.vault / "_meta" / "playbook.md").exists(), "states never touched")
        model, user, schema = self.calls[0]
        self.assertIn("pit-00001", user)
        self.assertIn("verdicts", json.dumps(schema))
        import lint
        errors = [f for f in lint.lint(self.vault, files=[note.relative_to(self.vault).as_posix()])
                  if f.severity == "ERROR"]
        self.assertEqual(errors, [])

    def test_ids_echoed_with_brackets_or_backticks_are_accepted(self):
        res = self.run_judge([verdict(bullet_a="[pit-00001]", bullet_b="`cmd-00002`", winner="[cmd-00002]")])
        self.assertEqual(res["verdicts"], 1)
        self.assertIn("`pit-00001` vs `cmd-00002`: winner `cmd-00002`", self.inbox()[0].read_text(encoding="utf-8"))

    def test_gate_rejects_unknown_pairs_bad_winners_uncited_and_secret_reasons(self):
        bad = [verdict(bullet_a="pit-00009"), verdict(winner="str-00003"), verdict(citations=[]),
               verdict(citations=["src/invented.rs"]), verdict(reason=""),
               verdict(reason="token ghp_123456789012345678901234567890123456"), "not a dict"]
        res = self.run_judge(bad)
        self.assertEqual((res["verdicts"], res["skipped"]), (0, False))
        self.assertEqual(self.inbox(), [])

    def test_no_conflicts_means_no_model_call(self):
        (self.vault / "_meta" / "playbook.md").write_text("## pitfalls\n- [pit-00001] seen=1 helpful=1 harmful=0 :: "
                                                          "one lonely lesson\n", encoding="utf-8")
        res = self.run_judge([verdict()])
        self.assertTrue(res["skipped"])
        self.assertEqual(self.calls, [])

    def test_offline_model_fails_open_and_dry_run_writes_nothing(self):
        def boom(*a, **k):
            raise ConnectionError("down")
        cfg = {"enabled": True, "default_privacy": "private", "project_privacies": {},
               "combos": {"private": "hive-private", "fast": "f", "deep": "d"}}
        with mock.patch.object(curate.llm, "load_llm_config", return_value=cfg):
            res = curate.run_curate_conflicts(self.vault, None, complete=boom, diff=lambda *a: "")
        self.assertTrue(res["skipped"])
        res = self.run_judge([verdict()], dry_run=True)
        self.assertEqual((res["verdicts"], self.inbox()), (1, []))


if __name__ == "__main__":
    unittest.main()

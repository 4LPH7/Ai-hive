"""RATCHET-01 (v2.8.0): every new ACE ADD cites the failure it came from; unsourced scouts carry weight 0.
Parity with src/quorum.rs + src/ace.rs tests."""
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import ace
import quorum


class FailureRefTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.vault = Path(self.tmp.name)
        self.patches = [mock.patch.object(ace, "VAULT", self.vault),
                        mock.patch.dict("os.environ", {"HIVE_SESSION_ID": "sess-a", "HIVE_AGENT": "claude",
                                                       "HIVE_LEDGER": str(self.vault / "ledger.jsonl")})]
        for p in self.patches:
            p.start()

    def tearDown(self):
        for p in self.patches:
            p.stop()
        self.tmp.cleanup()

    def meta(self):
        return quorum.load(quorum.meta_path(ace.scope_path("demo")))

    def add(self, content, session, ref=None):
        op = {"type": "ADD", "section": "pitfalls", "content": content}
        if ref is not None:
            op["failure_ref"] = ref
        with mock.patch.dict("os.environ", {"HIVE_SESSION_ID": session}):
            return ace.apply("demo", [op])

    def confirm(self, bid, session):
        with mock.patch.dict("os.environ", {"HIVE_SESSION_ID": session}):
            ace.apply("demo", [{"type": "TAG", "id": bid, "tag": "helpful", "evidence": "test_pass"}])

    def test_valid_refs(self):
        for ref in ("a4fc6c4", "commit:448a90f", "err:9f2c1d0e", "session:0c76ddc1-d32c", "PR#8"):
            self.assertEqual(quorum.valid_failure_ref(ref), ref)
        for ref in ("", None, "abc", "has space here", "x" * 121, "token: abcdefghijklmnop", "-leading"):
            self.assertEqual(quorum.valid_failure_ref(ref), "", ref)

    def test_unsourced_scout_has_weight_zero_until_a_ref_arrives(self):
        r = self.add("scout lesson about cache keys", "sess-a")
        (bid,) = r["added"]
        self.assertEqual(r["unsourced"], [bid])
        rec = self.meta()[bid]
        self.assertEqual(rec["failure_ref"], "")
        self.confirm(bid, "sess-a")
        self.confirm(bid, "sess-b")
        rec = self.meta()[bid]
        self.assertEqual(quorum.sessions(rec), {"sess-a", "sess-b"})
        self.assertEqual(quorum.score(rec), 0.0)
        self.assertEqual(rec["state"], "scout", "an unsourced lesson never promotes")
        r = self.add("scout lesson about cache keys again", "sess-c", ref="commit:448a90f")
        self.assertEqual(r["reinforced"], [bid])
        rec = self.meta()[bid]
        self.assertEqual(rec["failure_ref"], "commit:448a90f")
        self.assertEqual(rec["state"], "canonical", "sourced now: 2 test passes + 1 confirm clear the gate")

    def test_sourced_add_promotes_and_ref_is_kept(self):
        (bid,) = self.add("sourced lesson on graph ties", "sess-a", ref="err:9f2c1d0e")["added"]
        self.confirm(bid, "sess-a")
        self.confirm(bid, "sess-b")
        rec = self.meta()[bid]
        self.assertEqual((rec["failure_ref"], rec["state"]), ("err:9f2c1d0e", "canonical"))

    def test_invalid_ref_is_unsourced(self):
        r = self.add("lesson with a bad ref value", "sess-a", ref="token: abcdefghijklmnop")
        self.assertEqual(r["unsourced"], r["added"])
        self.assertEqual(self.meta()[r["added"][0]]["failure_ref"], "")

    def test_legacy_records_keep_old_behaviour_and_bytes(self):
        legacy = {"id": "pit-00001", "state": "scout", "origin_session": "s", "origin_agent": "a", "origin_ts": 0,
                  "evidence": [{"session": "s1", "agent": "a", "kind": "test_pass"},
                               {"session": "s2", "agent": "a", "kind": "confirm"}]}
        self.assertEqual(quorum.dumps(legacy),
                         '{"id":"pit-00001","state":"scout","origin_session":"s","origin_agent":"a",'
                         '"origin_ts":0,"evidence":[{"session":"s1","agent":"a","kind":"test_pass"},'
                         '{"session":"s2","agent":"a","kind":"confirm"}]}')
        self.assertEqual(quorum.promote(quorum._clean(legacy))["state"], "canonical")
        sourced = dict(legacy, failure_ref="a4fc6c4")
        self.assertIn('"origin_ts":0,"failure_ref":"a4fc6c4","evidence"', quorum.dumps(sourced))


LESSON = "Never run `python -c` in the lean-ctx shell; write a script file and run python -I script.py."


class RatchetGateTest(unittest.TestCase):
    """RATCHET-02 + Phase 8 gate: a recurring playbook mistake becomes a shipped lint rule and the bullet stops
    being injected."""

    def setUp(self):
        import hive
        self.tmp = tempfile.TemporaryDirectory()
        self.vault = Path(self.tmp.name)
        self.patches = [mock.patch.object(ace, "VAULT", self.vault), mock.patch.object(hive, "VAULT", self.vault),
                        mock.patch.dict("os.environ", {"HIVE_SESSION_ID": "sess-a", "HIVE_AGENT": "claude",
                                                       "HIVE_LEDGER": str(self.vault / "ledger.jsonl")})]
        for p in self.patches:
            p.start()
        pb = ace.scope_path("global")
        pb.parent.mkdir(parents=True)
        pb.write_text(f"## commands\n- [cmd-00003] seen=6 helpful=6 harmful=0 :: {LESSON}\n"
                      "- [cmd-00004] seen=2 helpful=2 harmful=0 :: write JSON to a file, not echo\n",
                      encoding="utf-8")

    def tearDown(self):
        for p in self.patches:
            p.stop()
        self.tmp.cleanup()

    def test_detect_and_glob(self):
        import ratchet
        rule = ratchet.detect("global", {"id": "cmd-00003", "content": LESSON})
        self.assertEqual((rule["pattern"], rule["target"], rule["glob"]), ("python -c", "repo", "**/*.py"))
        self.assertIsNone(ratchet.detect("global", {"id": "x", "content": "write JSON to a `file`, not echo"}))
        self.assertTrue(ratchet.glob_match("a.py", "**/*.py") and ratchet.glob_match("src/a/b.py", "**/*.py"))
        self.assertFalse(ratchet.glob_match("a.rs", "**/*.py"))

    def test_gate_lesson_becomes_lint_rule_and_leaves_injection(self):
        import hive
        import lint
        import ratchet
        self.assertIn("cmd-00003", {b["id"] for _, _, b in ace.select(["global"])})
        out = hive.tool_ratchet()
        self.assertIn("ratchet-rat-global-cmd-00003.md", out)
        self.assertNotIn("cmd-00004", out)
        proposal = self.vault / "inbox" / "ratchet-rat-global-cmd-00003.md"
        self.assertEqual([f for f in lint.lint(self.vault) if f.path == "inbox/ratchet-rat-global-cmd-00003.md"
                          and f.severity == "ERROR"], [], "proposals are lint-clean")
        self.assertIn("no new candidates", hive.tool_ratchet(), "an open proposal is not drafted twice")
        note = self.vault / "notes" / "howto.md"
        note.parent.mkdir()
        note.write_text("---\ntype: note\nproject: demo\nagent: user\nstatus: active\nupdated: 2026-11-26\n---\n"
                        "# how\n\nrun python -c 'print(1)'\n", encoding="utf-8")
        self.assertIn("shipped", hive.tool_ratchet("accept", "rat-global-cmd-00003", "--target", "vault",
                                                   "--glob", "**/*.md"))
        hits = [f for f in lint.lint(self.vault) if f.code == "ratchet-violation"]
        self.assertEqual([(f.path, f.severity) for f in hits], [("notes/howto.md", "ERROR")])
        self.assertIn("line 10", hits[0].message)
        self.assertNotIn("cmd-00003", {b["id"] for _, _, b in ace.select(["global"])}, "the rule replaced it")
        self.assertIn("cmd-00004", {b["id"] for _, _, b in ace.select(["global"])})
        with self.assertRaises(SystemExit):
            hive.tool_ratchet("accept", "rat-global-cmd-00003")
        self.assertTrue(proposal.exists())
        self.assertIn("rat-global-cmd-00003", hive.tool_ratchet("list"))

    def test_repo_rule_blocks_staged_files(self):
        import subprocess
        import ratchet
        repo = self.vault / "repo"
        repo.mkdir()
        git = lambda *a: subprocess.run(["git", *a], cwd=repo, capture_output=True, check=True)  # noqa: E731
        git("init", "-q")
        ratchet.propose(self.vault, ["global"], "claude", "sess-a")
        ratchet.accept(self.vault, "rat-global-cmd-00003")  # proposal default: repo, **/*.py
        (repo / "ok.rs").write_text("// python -c in a .rs file is out of glob\n", encoding="utf-8")
        (repo / "tool.py").write_text("import os\n\nos.system(\"python -c 'x'\")\n", encoding="utf-8")
        self.assertEqual(ratchet.staged_violations(self.vault, repo), [], "unstaged files are not checked")
        git("add", "ok.rs", "tool.py")
        bad = ratchet.staged_violations(self.vault, repo)
        self.assertEqual([(p, line, r["id"]) for p, line, r in bad], [("tool.py", 3, "rat-global-cmd-00003")])


if __name__ == "__main__":
    unittest.main()

"""QUOR-01 (+PROV-04): bullet states and origin in the playbook.meta.jsonl sidecar; line format unchanged."""
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import ace
import quorum


class QuorumSidecarTest(unittest.TestCase):
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

    def test_existing_bullets_migrate_as_canonical_without_rewrite(self):
        path = ace.scope_path("demo")
        path.parent.mkdir(parents=True)
        path.write_text("## pitfalls\n- [pit-00001] seen=2 helpful=2 harmful=0 :: legacy lesson here\n", encoding="utf-8")
        self.assertEqual(quorum.state_of(self.meta(), "pit-00001"), "canonical")
        self.assertFalse(quorum.meta_path(path).exists())

    def test_add_is_scout_with_origin_and_prune_retires_without_id_reuse(self):
        r = ace.apply("demo", [{"type": "ADD", "section": "pitfalls", "content": "first scout lesson about tokenizer"}])
        (bid,) = r["added"]
        rec = self.meta()[bid]
        self.assertEqual((rec["state"], rec["origin_session"], rec["origin_agent"]), ("scout", "sess-a", "claude"))
        ace.apply("demo", [{"type": "TAG", "id": bid, "harmful": 9}])
        self.assertEqual(self.meta()[bid]["state"], "retired")
        r2 = ace.apply("demo", [{"type": "ADD", "section": "pitfalls", "content": "another unrelated graph cache rule"}])
        self.assertNotEqual(r2["added"], [bid], "a retired id must never be reissued")
        # old readers: every live line still matches the unchanged line regex
        lines = [l for l in ace.scope_path("demo").read_text(encoding="utf-8").splitlines() if l.startswith("- [")]
        self.assertTrue(lines and all(ace.LINE_RE.match(l) for l in lines))

    def test_line_serialization_is_compact_and_ordered(self):
        self.assertEqual(quorum.dumps({"origin_agent": "a", "state": "scout", "id": "pit-00001", "origin_session": "s"}),
                         '{"id":"pit-00001","state":"scout","origin_session":"s","origin_agent":"a","evidence":[]}')
        rec = {"id": "x", "state": "scout", "evidence": [{"kind": "confirm", "agent": "codex", "session": "s2"}]}
        self.assertIn('"evidence":[{"session":"s2","agent":"codex","kind":"confirm"}]', quorum.dumps(rec))

    def test_bad_lines_and_states_are_ignored(self):
        p = self.vault / "playbook.meta.jsonl"
        p.write_text('not json\n{"id":"pit-00002","state":"bogus"}\n{"id":"pit-00003","state":"quorum"}\n',
                     encoding="utf-8")
        m = quorum.load(p)
        self.assertEqual(sorted(m), ["pit-00003"])


class QuorumEvidenceTest(QuorumSidecarTest):
    """QUOR-02: test+commit 1.0, other-session confirm 1.0, self-tag 0.35, same-session repeat 0."""

    def test_evidence_weights_and_same_session_repeat(self):
        (bid,) = ace.apply("demo", [{"type": "ADD", "section": "pitfalls", "content": "scout lesson on cache keys"}])["added"]
        tag = lambda **kw: ace.apply("demo", [dict({"type": "TAG", "id": bid, "tag": "helpful"}, **kw)])
        tag()  # origin session sess-a -> self_tag
        self.assertAlmostEqual(quorum.score(self.meta()[bid]), 0.35)
        tag()  # same session again -> nothing new
        self.assertAlmostEqual(quorum.score(self.meta()[bid]), 0.35)
        tag(session="sess-a", evidence="test_pass")  # same session, stronger kind: max per session
        self.assertAlmostEqual(quorum.score(self.meta()[bid]), 1.0)
        tag(session="sess-b")  # another session confirms
        rec = self.meta()[bid]
        self.assertAlmostEqual(quorum.score(rec), 2.0)
        self.assertEqual(quorum.sessions(rec), {"sess-a", "sess-b"})
        ace.apply("demo", [{"type": "TAG", "id": bid, "tag": "harmful", "session": "sess-c"}])
        self.assertAlmostEqual(quorum.score(self.meta()[bid]), 2.0, msg="harmful tags are not evidence")

    def test_legacy_bullet_gets_evidence_but_stays_canonical(self):
        path = ace.scope_path("demo")
        path.parent.mkdir(parents=True)
        path.write_text("## pitfalls\n- [pit-00001] seen=2 helpful=2 harmful=0 :: legacy lesson here\n", encoding="utf-8")
        ace.apply("demo", [{"type": "TAG", "id": "pit-00001", "tag": "helpful"}])
        rec = self.meta()["pit-00001"]
        self.assertEqual((rec["state"], quorum.score(rec)), ("canonical", 1.0))

class QuorumPromotionTest(QuorumSidecarTest):
    """QUOR-03: promote at score >= 1.5 from >= 2 sessions; inject only canonical (quorum behind a flag)."""

    def ids_injected(self):
        return {b["id"] for _, _, b in ace.select(["demo"], "")}

    def test_scout_hidden_until_quorum_then_canonical(self):
        path = ace.scope_path("demo")
        path.parent.mkdir(parents=True)
        path.write_text("## pitfalls\n- [pit-00001] seen=2 helpful=2 harmful=0 :: legacy lesson here\n", encoding="utf-8")
        (bid,) = ace.apply("demo", [{"type": "ADD", "section": "pitfalls", "content": "scout lesson on cache keys"}])["added"]
        self.assertEqual(self.ids_injected(), {"pit-00001"}, "scouts are never injected")
        ace.apply("demo", [{"type": "TAG", "id": bid, "tag": "helpful", "evidence": "test_pass"}])  # 1.0, 1 session
        self.assertEqual(quorum.state_of(self.meta(), bid), "scout")
        # another session independently proposes the same lesson: near-duplicate ADD counts as confirm
        with mock.patch.dict("os.environ", {"HIVE_SESSION_ID": "sess-b"}):
            r = ace.apply("demo", [{"type": "ADD", "section": "pitfalls", "content": "scout lesson on cache keys again"}])
        self.assertEqual(r["reinforced"], [bid])
        self.assertEqual(quorum.state_of(self.meta(), bid), "canonical")  # 1.0 + 1.0 from 2 sessions
        self.assertIn(bid, self.ids_injected())

    def test_two_sessions_below_threshold_is_quorum_behind_flag(self):
        (bid,) = ace.apply("demo", [{"type": "ADD", "section": "pitfalls", "content": "scout lesson on graph cache"}])["added"]
        ace.apply("demo", [{"type": "TAG", "id": bid, "tag": "helpful"}])                     # sess-a self 0.35
        ace.apply("demo", [{"type": "TAG", "id": bid, "tag": "helpful", "session": "sess-c",
                            "evidence": "bogus"}])                                            # sess-c confirm 1.0
        rec = self.meta()[bid]
        self.assertAlmostEqual(quorum.score(rec), 1.35)
        self.assertEqual(rec["state"], "quorum")
        self.assertNotIn(bid, self.ids_injected())
        with mock.patch.dict("os.environ", {"HIVE_INJECT_QUORUM": "1"}):
            self.assertIn(bid, self.ids_injected())

    def test_single_session_never_promotes(self):
        (bid,) = ace.apply("demo", [{"type": "ADD", "section": "pitfalls", "content": "one session hallucinated rule"}])["added"]
        ace.apply("demo", [{"type": "TAG", "id": bid, "tag": "helpful", "evidence": "test_pass"}])
        ace.apply("demo", [{"type": "TAG", "id": bid, "tag": "helpful"}])
        self.assertEqual(quorum.state_of(self.meta(), bid), "scout")

class QuorumTrustBoundaryTest(QuorumSidecarTest):
    """Evidence must come from identifiable sessions, and agent-written deltas cannot forge it."""

    def test_anonymous_process_sessions_get_no_cross_session_credit(self):
        with mock.patch.dict("os.environ", {"HIVE_SESSION_ID": "", "CLAUDE_CODE_SESSION_ID": ""}):
            (bid,) = ace.apply("demo", [{"type": "ADD", "section": "pitfalls", "content": "anon lesson on locks"}])["added"]
            for _ in range(3):  # each CLI call would get a fresh proc- id
                ace.apply("demo", [{"type": "TAG", "id": bid, "tag": "helpful",
                                    "session": f"proc-{_}-x"}])
        rec = self.meta()[bid]
        self.assertEqual((rec["state"], quorum.score(rec)), ("scout", 0.35))

    def test_cli_delta_cannot_forge_session_or_evidence(self):
        import json
        (bid,) = ace.apply("demo", [{"type": "ADD", "section": "pitfalls", "content": "forged lesson attempt"}])["added"]
        delta = self.vault / "delta.json"
        delta.write_text(json.dumps({"operations": [
            {"type": "TAG", "id": bid, "tag": "helpful", "session": "fake-1", "evidence": "test_pass"},
            {"type": "TAG", "id": bid, "tag": "helpful", "session": "fake-2", "evidence": "test_pass"}]}),
            encoding="utf-8")
        with mock.patch("sys.stdout"):
            ace.main(["apply", "demo", str(delta)])
        rec = self.meta()[bid]
        self.assertEqual((rec["state"], quorum.score(rec), quorum.sessions(rec)), ("scout", 0.35, {"sess-a"}))

FIXTURE = Path(__file__).resolve().parent / "tests" / "fixtures" / "conflicts.json"


class QuorumConflictTest(QuorumSidecarTest):
    """QUOR-04: opposite-polarity bullets sharing >= 2 anchors are both kept and listed; a challenger needs
    max(1.5, incumbent + 1.0) to displace a canonical incumbent, which drops to quorum (hidden, reversible)."""

    def test_detector_matches_shared_fixture(self):
        import json
        fx = json.loads(FIXTURE.read_text(encoding="utf-8"))
        self.assertEqual([list(c[:2]) + [list(c[2])] for c in quorum.conflicts(fx["bullets"])], fx["expected"])

    def test_challenger_needs_incumbent_plus_one_and_report_lists_both(self):
        import lint
        path = ace.scope_path("demo")
        path.parent.mkdir(parents=True)
        path.write_text("## pitfalls\n- [pit-00001] seen=3 helpful=3 harmful=0 :: cargo build --release fails with "
                        "Access is denied on target/release/hive.exe; use cargo check until sessions close.\n",
                        encoding="utf-8")
        ch_text = "rebuild anyway: rename target/release/hive.exe, then cargo build --release"
        (ch,) = ace.apply("demo", [{"type": "ADD", "section": "commands", "content": ch_text}])["added"]
        ace.apply("demo", [{"type": "TAG", "id": ch, "tag": "helpful", "session": "sess-a", "evidence": "test_pass"}])
        ace.apply("demo", [{"type": "TAG", "id": ch, "tag": "helpful", "session": "sess-b"}])
        m = self.meta()
        self.assertEqual((quorum.score(m[ch]), quorum.state_of(m, ch), quorum.state_of(m, "pit-00001")),
                         (2.0, "quorum", "canonical"), "2.0 < incumbent floor 1.5 + 1.0: held")
        report = self.vault / "_meta" / "_conflicts.md"
        text = report.read_text(encoding="utf-8")
        self.assertIn("## demo", text)
        self.assertIn("`pit-00001` (canonical", text)
        self.assertIn(f"`{ch}` (quorum", text)
        self.assertNotIn("rebuild anyway", text, "report lists ids and anchors, never lesson text")
        mtime = report.stat().st_mtime_ns
        ace.apply("demo", [{"type": "TAG", "id": "pit-00001", "tag": "helpful", "session": "sess-a"}])
        ace.apply("demo", [{"type": "TAG", "id": ch, "tag": "helpful", "session": "sess-c"}])
        m = self.meta()
        self.assertEqual((quorum.state_of(m, ch), quorum.state_of(m, "pit-00001")), ("canonical", "quorum"))
        self.assertIn("pit-00001", (self.vault / "projects" / "demo" / "playbook.md").read_text(encoding="utf-8"),
                      "displacement never touches playbook lines")
        self.assertNotEqual(report.stat().st_mtime_ns, mtime)
        errors = [f for f in lint.lint(self.vault, files=["_meta/_conflicts.md"]) if f.severity == "ERROR"]
        self.assertEqual(errors, [])

    def test_report_written_only_on_change_and_removed_when_empty(self):
        report = self.vault / "_meta" / "_conflicts.md"
        quorum.write_report(report, "demo", [], {}, "2026-11-25")
        self.assertFalse(report.exists(), "never written when there are no conflicts")
        quorum.write_report(report, "demo", [("pit-00001", "cmd-00002", ["a.rs", "b.rs"])], {}, "2026-11-25")
        first = report.read_text(encoding="utf-8")
        quorum.write_report(report, "other", [("pit-00009", "cmd-00010", ["x.py", "y.py"])], {}, "2026-11-26")
        both = report.read_text(encoding="utf-8")
        self.assertIn("## demo", both)
        self.assertIn("## other", both)
        mtime = report.stat().st_mtime_ns
        quorum.write_report(report, "other", [("pit-00009", "cmd-00010", ["x.py", "y.py"])], {}, "2026-11-27")
        self.assertEqual(report.stat().st_mtime_ns, mtime, "unchanged content is not rewritten")
        quorum.write_report(report, "demo", [], {}, "2026-11-27")
        quorum.write_report(report, "other", [], {}, "2026-11-27")
        self.assertFalse(report.exists())
        self.assertIn("type: note", first)


if __name__ == "__main__":
    unittest.main()

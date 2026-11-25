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


if __name__ == "__main__":
    unittest.main()

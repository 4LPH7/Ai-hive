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
                         '{"id":"pit-00001","state":"scout","origin_session":"s","origin_agent":"a"}')

    def test_bad_lines_and_states_are_ignored(self):
        p = self.vault / "playbook.meta.jsonl"
        p.write_text('not json\n{"id":"pit-00002","state":"bogus"}\n{"id":"pit-00003","state":"quorum"}\n',
                     encoding="utf-8")
        m = quorum.load(p)
        self.assertEqual(sorted(m), ["pit-00003"])


if __name__ == "__main__":
    unittest.main()

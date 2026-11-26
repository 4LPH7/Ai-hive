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


if __name__ == "__main__":
    unittest.main()

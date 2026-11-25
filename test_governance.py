"""GOV-01: reversible quarantine rules over traces, scout bullets, inbox proposals and codices."""
import json
import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import ace
import codex
import governance
import ledger
import quorum
import traces

DAY = 86400
T0 = 1_800_000_000  # fixed clock


class GovernanceTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.vault = Path(self.tmp.name)
        self.db = sqlite3.connect(":memory:")
        self.patches = [mock.patch.object(ace, "VAULT", self.vault),
                        mock.patch.dict(ledger._CONTEXT, {"session_id": None, "agent": None}),
                        mock.patch.dict("os.environ", {"HIVE_SESSION_ID": "bad-s", "HIVE_AGENT": "codex",
                                                       "HIVE_LEDGER": str(self.vault / "l.jsonl")})]
        for p in self.patches:
            p.start()
        # healthy items
        traces.add(self.db, "demo", "DEAD_END", "src/a.rs", "healthy dead end", "claude", "good-s", T0 - DAY)
        codex.write(self.vault, self.db, "demo", {"goal": "good", "outcome": "done: ok", "files": ["src/a.rs"],
                    "dead_ends": [], "invariants": [], "open_threads": []}, "claude", "good-s", "2027-01-14", T0)
        # poisoned items from session bad-s / agent codex
        traces.add(self.db, "demo", "HOT_FILE", "src/b.rs", "poisoned hot file", "codex", "bad-s", T0)
        codex.write(self.vault, self.db, "demo", {"goal": "bad", "outcome": "done: lies", "files": ["src/b.rs"],
                    "dead_ends": [], "invariants": [], "open_threads": []}, "codex", "bad-s", "2027-01-15", T0)
        with mock.patch("time.time", return_value=T0):
            (self.bad_bullet,) = ace.apply("demo", [{"type": "ADD", "section": "pitfalls",
                                                     "content": "poisoned rule from a bad session"}])["added"]
        inbox = self.vault / "inbox"
        inbox.mkdir()
        (inbox / "conflicts-demo-x.md").write_text("---\ntype: note\nproject: demo\nagent: codex\nstatus: active\n"
                                                   "updated: 2027-01-15\nsource: hive curate\n---\n# x\n", encoding="utf-8")

    def tearDown(self):
        for p in self.patches:
            p.stop()
        self.db.close()
        self.tmp.cleanup()

    def test_session_rule_matches_only_that_radius_and_releases(self):
        rule, counts = governance.quarantine(self.vault, self.db, session="bad-s", reason="poisoned", now=T0)
        self.assertEqual(counts, {"traces": 1, "bullets": 1, "inbox": 0, "codices": 1})
        rules = governance.load_rules(self.vault)
        self.assertEqual([r["id"] for r in rules], [rule["id"]])
        t_bad = [t for t, _ in traces.active(self.db, "demo", T0) if t["session_id"] == "bad-s"][0]
        t_good = [t for t, _ in traces.active(self.db, "demo", T0) if t["session_id"] == "good-s"][0]
        self.assertTrue(governance.is_quarantined(rules, t_bad["session_id"], t_bad["agent"], t_bad["created_at"]))
        self.assertFalse(governance.is_quarantined(rules, t_good["session_id"], t_good["agent"], t_good["created_at"]))
        self.assertEqual(governance.release(self.vault, rule["id"]), 1)
        self.assertEqual(governance.load_rules(self.vault), [])

    def test_agent_since_rule_and_items_are_never_edited(self):
        before = {p: p.read_bytes() for p in self.vault.rglob("*.md")}
        _, counts = governance.quarantine(self.vault, self.db, agent="codex", since="2027-01-15", now=T0)
        self.assertEqual(counts, {"traces": 1, "bullets": 1, "inbox": 1, "codices": 1})
        self.assertEqual({p: p.read_bytes() for p in self.vault.rglob("*.md")}, before, "rules never edit notes")
        _, none = governance.quarantine(self.vault, self.db, agent="codex", since="2027-02-01", now=T0)
        self.assertEqual(none, {"traces": 0, "bullets": 0, "inbox": 0, "codices": 0})

    def test_origin_ts_recorded_and_bad_selectors_rejected(self):
        meta = quorum.load(quorum.meta_path(ace.scope_path("demo")))
        self.assertEqual(meta[self.bad_bullet]["origin_ts"], T0)
        for kw in ({}, {"since": "2027-01-15"}, {"session": "x", "since": "not-a-date"}):
            with self.assertRaises(ValueError):
                governance.quarantine(self.vault, self.db, now=T0, **kw)
        rule, _ = governance.quarantine(self.vault, self.db, session="bad-s", now=T0)
        again, _ = governance.quarantine(self.vault, self.db, session="bad-s", now=T0 + 5)
        self.assertEqual(rule["id"], again["id"], "same selector is one rule")
        line = (self.vault / "_meta" / "quarantine.jsonl").read_text(encoding="utf-8").splitlines()[0]
        self.assertEqual(list(json.loads(line)), ["id", "session", "agent", "since", "reason", "created"])


if __name__ == "__main__":
    unittest.main()

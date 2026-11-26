"""Phase 8 gate (v2.7.0): quorum + governance end to end.

1. A single-session hallucinated rule stays `scout` (never injected).
2. A 2-session + test-pass rule reaches `canonical` (injected).
3. `quarantine --session X` hides 100% of seeded poisoned items on every read path, touches 0 healthy items,
   and edits no file.
4. Conflicts never overwrite: the playbook keeps both lines after a challenger wins.
"""
import hashlib
import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import ace
import codex
import governance
import hive
import ledger
import quorum
import traces

T0 = 1_800_000_000


def files_digest(vault):
    return {p.relative_to(vault).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in Path(vault).rglob("*") if p.is_file() and p.suffix in (".md",)}


class Phase8GateTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.vault = Path(self.tmp.name)
        self.db = sqlite3.connect(":memory:")
        self.patches = [mock.patch.object(ace, "VAULT", self.vault), mock.patch.object(hive, "VAULT", self.vault),
                        mock.patch.dict(ledger._CONTEXT, {"session_id": None, "agent": None}),
                        mock.patch.dict("os.environ", {"HIVE_LEDGER": str(self.vault / "l.jsonl"),
                                                       "HIVE_AGENT": "claude"}),
                        mock.patch("time.time", return_value=T0)]
        for p in self.patches:
            p.start()

    def tearDown(self):
        for p in self.patches:
            p.stop()
        self.db.close()
        self.tmp.cleanup()

    def as_session(self, session, ops):
        with mock.patch.dict("os.environ", {"HIVE_SESSION_ID": session}):
            return ace.apply("demo", ops)

    def state(self, bid):
        return quorum.state_of(quorum.load(quorum.meta_path(ace.scope_path("demo"))), bid)

    def injected(self):
        return {b["id"] for _, _, b in ace.select(["demo"])}

    def test_1_and_2_quorum_promotion(self):
        (lone,) = self.as_session("h1", [{"type": "ADD", "section": "pitfalls",
                                          "content": "hallucinated: always delete the lockfile first"}])["added"]
        self.as_session("h1", [{"type": "TAG", "id": lone, "tag": "helpful"}])
        self.as_session("h1", [{"type": "TAG", "id": lone, "tag": "helpful", "evidence": "test_pass"}])
        self.assertEqual(self.state(lone), "scout")
        self.assertNotIn(lone, self.injected())

        (good,) = self.as_session("s1", [{"type": "ADD", "section": "strategies",
                                          "content": "pin the parser version before regenerating fixtures"}])["added"]
        self.as_session("s1", [{"type": "TAG", "id": good, "tag": "helpful", "evidence": "test_pass"}])
        self.as_session("s2", [{"type": "TAG", "id": good, "tag": "helpful"}])
        self.assertEqual(self.state(good), "canonical")
        self.assertIn(good, self.injected())

    def seed(self, session, agent, tag):
        """Traces of every kind, two scout bullets, one codex and one inbox note from one session."""
        for kind, target in (("DEAD_END", f"src/{tag}_a.rs"), ("HOT_FILE", f"src/{tag}_b.rs"),
                             ("RECENT_FIX", f"src/{tag}_c.rs")):
            traces.add(self.db, "demo", kind, target, f"{tag} {kind.lower()} signal", agent, session, T0)
        with mock.patch.dict("os.environ", {"HIVE_SESSION_ID": session, "HIVE_AGENT": agent}):
            ids = ace.apply("demo", [{"type": "ADD", "section": "pitfalls", "content": f"{tag} lesson one zz{tag}"},
                                     {"type": "ADD", "section": "commands", "content": f"{tag} lesson two yy{tag}"}])["added"]
        codex.write(self.vault, self.db, "demo", {"goal": f"{tag} goal", "outcome": f"done: {tag} marker{tag}",
                    "files": [f"src/{tag}_a.rs"], "dead_ends": [], "invariants": [], "open_threads": [f"{tag} thread"]},
                    agent, session, "2027-01-15", T0)
        inbox = self.vault / "inbox"
        inbox.mkdir(exist_ok=True)
        (inbox / f"note-{tag}.md").write_text(
            f"---\ntype: note\nproject: demo\nagent: {agent}\nstatus: active\nupdated: 2027-01-15\n"
            f"source: session {session}\n---\n# {tag} proposal\n\nmarker{tag} inbox body\n", encoding="utf-8")
        return ids

    def test_3_session_quarantine_is_complete_and_precise(self):
        good_ids = self.seed("good-s", "claude", "good")
        bad_ids = self.seed("evil-s", "codex", "evil")
        before = files_digest(self.vault)
        rule, counts = governance.quarantine(self.vault, self.db, session="evil-s", now=T0)
        self.assertEqual(counts, {"traces": 3, "bullets": 2, "inbox": 1, "codices": 1}, "100% of seeded poison")
        rules = governance.load_rules(self.vault)
        visible = governance.visible_traces(rules, traces.active(self.db, "demo", T0))
        self.assertEqual(sorted(t["session_id"] for t, _ in visible), ["good-s"] * 3, "0 healthy traces touched")
        meta = quorum.load(quorum.meta_path(ace.scope_path("demo")))
        self.assertTrue(all(governance.bullet_hidden(rules, meta[i]) for i in bad_ids))
        self.assertFalse(any(governance.bullet_hidden(rules, meta[i]) for i in good_ids))
        task = "src/good_a.rs src/evil_a.rs"
        ctx = codex.context_section(self.vault, "demo", task)
        self.assertIn("good thread", ctx)
        self.assertNotIn("evil thread", ctx)
        idx = hive.Index(self.vault, self.vault / "idx.sqlite")
        try:
            self.assertFalse(idx.search("markerevil", "demo", 8), "poisoned codex/inbox never surface")
            self.assertEqual({h["path"] for h in idx.search("markergood", "demo", 8)},
                             {"inbox/note-good.md", *[p.relative_to(self.vault).as_posix()
                                                      for p in (self.vault / "codices" / "demo").glob("2027-01-15-*.md")
                                                      if "good" in p.read_text(encoding="utf-8")]})
        finally:
            idx.db.close()
        self.assertEqual(files_digest(self.vault), before, "quarantine edits no file")
        self.assertEqual(governance.release(self.vault, rule["id"]), 1)
        self.assertEqual(len(traces.active(self.db, "demo", T0)), 6, "release restores every trace")

    def test_4_conflicts_never_overwrite(self):
        path = ace.scope_path("demo")
        path.parent.mkdir(parents=True)
        incumbent = ("cargo build --release fails with Access is denied on target/release/hive.exe; "
                     "use cargo check until sessions close.")
        path.write_text(f"## pitfalls\n- [pit-00001] seen=3 helpful=3 harmful=0 :: {incumbent}\n", encoding="utf-8")
        (ch,) = self.as_session("s1", [{"type": "ADD", "section": "commands",
                                        "content": "rebuild anyway: rename target/release/hive.exe, then cargo build --release"}])["added"]
        for s, extra in (("s1", {"evidence": "test_pass"}), ("s2", {}), ("s3", {})):
            self.as_session(s, [dict({"type": "TAG", "id": ch, "tag": "helpful"}, **extra)])
        self.assertEqual((self.state(ch), self.state("pit-00001")), ("canonical", "quorum"))
        text = path.read_text(encoding="utf-8")
        self.assertIn(incumbent, text, "the displaced incumbent's line is kept verbatim")
        self.assertIn(ch, text)
        self.assertIn("pit-00001", (self.vault / "_meta" / "_conflicts.md").read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()

"""CODEX-02 parity with src/codex.rs tests."""
import hashlib
import sqlite3
import tempfile
import unittest
from pathlib import Path

import codex
import traces

REC = {"goal": "add codex write", "outcome": "done: tests pass",
       "files": ["src/codex.rs"], "dead_ends": ["serde_yaml frontmatter: dropped, hand-rendered is simpler"],
       "invariants": ["eval Recall@5 >= 0.97"], "open_threads": []}


class CodexWriteTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.vault = Path(self.tmp.name)
        self.db = sqlite3.connect(":memory:")

    def tearDown(self):
        self.db.close()
        self.tmp.cleanup()

    def test_writes_immutable_hashed_note_and_dead_end_traces(self):
        p = codex.write(self.vault, self.db, "demo", REC, "claude", "s1", "2026-11-25", 0)
        self.assertEqual(p.parent, self.vault / "codices" / "demo")
        self.assertTrue(p.name.startswith("2026-11-25-"))
        text = p.read_text(encoding="utf-8")
        self.assertLessEqual(len(text.splitlines()), codex.MAX_LINES)
        head, body = text.split("\n---\n", 1)
        sha = [l.split(": ", 1)[1] for l in head.splitlines() if l.startswith("content_sha256:")][0]
        self.assertEqual(sha, hashlib.sha256(body.encode("utf-8")).hexdigest())
        self.assertIn("type: codex", head)
        self.assertIn("- none", body)
        (t, _), = traces.active(self.db, "demo", 0)
        self.assertEqual((t["kind"], t["target"]), ("DEAD_END", "src/codex.rs"))
        with self.assertRaises(ValueError):  # same content -> same file -> never overwritten
            codex.write(self.vault, self.db, "demo", REC, "claude", "s1", "2026-11-25", 0)

    def test_rejects_bad_records(self):
        bad = [dict(REC, goal=""), dict(REC, extra=1), {k: v for k, v in REC.items() if k != "files"},
               dict(REC, files="src/a.rs"), dict(REC, outcome="two\nlines"),
               dict(REC, invariants=["token ghp_123456789012345678901234567890123456"]),
               dict(REC, open_threads=["x"] * 40)]
        for rec in bad:
            with self.assertRaises(ValueError):
                codex.write(self.vault, self.db, "demo", rec, "claude", "s1", "2026-11-25", 0)
        with self.assertRaises(ValueError):
            codex.write(self.vault, self.db, "../x", REC, "claude", "s1", "2026-11-25", 0)
        self.assertFalse((self.vault / "codices").exists())


if __name__ == "__main__":
    unittest.main()

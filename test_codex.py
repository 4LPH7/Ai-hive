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


class CodexVerifyTest(unittest.TestCase):
    """CODEX-04: body hash mismatch is a lint ERROR; CRLF checkouts still verify."""

    def test_verify_and_lint_finding(self):
        import lint
        _, text = codex.render("demo", REC, "claude", "s1", "2026-11-25")
        self.assertIsNone(codex.verify(text))
        self.assertIsNone(codex.verify("﻿" + text.replace("\n", "\r\n")))
        self.assertIn("mismatch", codex.verify(text.replace("add codex write", "add codex WRITE")))
        self.assertIn("missing", codex.verify("\n".join(l for l in text.splitlines() if "sha256" not in l)))
        meta = {"type": "codex"}
        self.assertEqual(lint.check_codex("codices/demo/x.md", meta, text), [])
        bad = lint.check_codex("codices/demo/x.md", meta, text.replace("tests pass", "tests fail"))
        self.assertEqual([(f.severity, f.code) for f in bad], [("ERROR", "codex-tampered")])
        self.assertEqual(lint.check_codex("notes/x.md", {"type": "note"}, "no hash"), [])


class CodexContextTest(unittest.TestCase):
    """CODEX-03: only Dead ends + Open threads from codices whose files overlap the task."""

    def test_context_section_filters_by_file_overlap(self):
        with tempfile.TemporaryDirectory() as tmp:
            db = sqlite3.connect(":memory:")
            codex.write(tmp, db, "demo", dict(REC, open_threads=["wire CLI"]), "claude", "s1", "2026-11-25", 0)
            codex.write(tmp, db, "demo", dict(REC, files=["ui/app.ts"], dead_ends=["css grid broke"]),
                        "codex", "s2", "2026-11-24", 0)
            s = codex.context_section(tmp, "demo", "fix codex write in src/codex.rs")
            self.assertTrue(s.startswith("## codex notes"))
            self.assertIn("dead end: serde_yaml", s)
            self.assertIn("open thread: wire CLI", s)
            self.assertNotIn("css grid", s)
            self.assertNotIn("Recall@5", s)  # invariants are not injected
            self.assertEqual(codex.context_section(tmp, "demo", ""), "")
            self.assertEqual(codex.context_section(tmp, "other", "codex"), "")
            db.close()


if __name__ == "__main__":
    unittest.main()

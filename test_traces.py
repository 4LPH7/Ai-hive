"""TRACE-01/02 parity with src/trace.rs tests."""
import sqlite3
import unittest

import traces

H = 3600


class TracesTest(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(":memory:")

    def tearDown(self):
        self.db.close()

    def test_strength_halves_per_half_life(self):
        self.assertAlmostEqual(traces.strength(1.0, 0, 24 * H, 24.0), 0.5)
        self.assertAlmostEqual(traces.strength(1.0, 0, 48 * H, 24.0), 0.25)
        self.assertEqual(traces.strength(1.0, 100, 0, 24.0), 1.0)

    def test_dead_end_visible_then_evaporates_then_purged(self):
        traces.add(self.db, "demo", "DEAD_END", "src/a.rs", "patching tokenizer breaks static link", "claude", "s1", 0)
        self.assertEqual(len(traces.active(self.db, "demo", 24 * H)), 1)
        self.assertEqual(traces.active(self.db, "demo", 72 * H), [])
        self.assertEqual(traces.active(self.db, "other", 0), [])
        traces.active(self.db, "demo", 8 * 24 * H)
        self.assertEqual(self.db.execute("SELECT COUNT(*) FROM traces").fetchone()[0], 0)

    def test_reinforcement_only_from_another_session(self):
        tid = traces.add(self.db, "demo", "HOT_FILE", "src/b.rs", "3 failed runs", "claude", "s1", 0)
        self.assertEqual(traces.add(self.db, "demo", "HOT_FILE", "src/b.rs", "again", "claude", "s1", H), tid)
        self.assertLess(traces.active(self.db, "demo", H)[0][1], 1.0)
        traces.add(self.db, "demo", "HOT_FILE", "src/b.rs", "codex saw it too", "codex", "s2", 12 * H)
        t, w = traces.active(self.db, "demo", 12 * H)[0]
        self.assertAlmostEqual(w, 1.5)
        self.assertEqual((t["agent"], t["id"]), ("codex", tid))

    def test_rejects_bad_kind_long_summary_and_secrets(self):
        for args in (("demo", "ACTIVE_LEASE", "x", "y"), ("demo", "DEAD_END", "x", "y" * 201),
                     ("demo", "DEAD_END", "x", "token ghp_123456789012345678901234567890123456"),
                     ("demo", "DEAD_END", " ", "y")):
            with self.assertRaises(ValueError):
                traces.add(self.db, *args, "a", "s", 0)


class ContextSectionTest(unittest.TestCase):
    """TRACE-04 parity with trace::context_section."""

    def test_filters_by_task_overlap(self):
        db = sqlite3.connect(":memory:")
        traces.add(db, "demo", "DEAD_END", "src/tokenizer.rs", "patching breaks static link", "claude", "s1", 0)
        traces.add(db, "demo", "HOT_FILE", "src/ui.rs", "3 failed runs", "codex", "s2", 0)
        items = traces.active(db, "demo", H)
        s = traces.context_section(items, "fix the tokenizer link error")
        self.assertTrue(s.startswith("## live traces"))
        self.assertIn("DEAD_END src/tokenizer.rs", s)
        self.assertNotIn("src/ui.rs", s)
        self.assertIn("src/ui.rs", traces.context_section(items, ""))
        self.assertEqual(traces.context_section(items, "unrelated words only"), "")
        self.assertEqual(traces.context_section([], ""), "")
        db.close()


class TraceCliFallbackTest(unittest.TestCase):
    """TRACE-03 Python fallback of `hive trace add|list`."""

    def test_add_and_list_use_the_disposable_db_only(self):
        import tempfile
        from pathlib import Path
        from unittest import mock
        import hive
        with tempfile.TemporaryDirectory() as tmp:
            with mock.patch.object(hive, "DB_PATH", Path(tmp) / "hive.sqlite"):
                out = hive.tool_trace("add", "demo", "dead_end", "src/a.rs", "tokenizer", "patch", "breaks")
                self.assertIn("DEAD_END", out)
                listing = hive.tool_trace("list", "demo")
                self.assertIn("src/a.rs", listing)
                self.assertIn("tokenizer patch breaks", listing)
                self.assertIn("no active traces", hive.tool_trace("list", "other"))
                with self.assertRaises(SystemExit):
                    hive.tool_trace("add", "demo", "BOGUS", "x", "y")
            self.assertEqual(sorted(p.name for p in Path(tmp).iterdir() if p.suffix == ".sqlite"), ["hive.sqlite"])


if __name__ == "__main__":
    unittest.main()

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


if __name__ == "__main__":
    unittest.main()

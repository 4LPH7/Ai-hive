"""TRACE-01/02 parity with src/trace.rs tests."""
import os
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

    def test_anonymous_process_sessions_never_reinforce(self):
        traces.add(self.db, "demo", "HOT_FILE", "src/b.rs", "x", "claude", "proc-1-a", 0)
        traces.add(self.db, "demo", "HOT_FILE", "src/b.rs", "x", "claude", "proc-2-b", 0)
        traces.add(self.db, "demo", "HOT_FILE", "src/b.rs", "x", "codex", "real-s2", 0)
        self.assertAlmostEqual(traces.active(self.db, "demo", 0)[0][1], 1.0)

    def test_cli_session_falls_back_to_the_agent_session_env(self):
        import ledger
        from unittest import mock
        with mock.patch.dict("os.environ", {"CLAUDE_CODE_SESSION_ID": "cc-123"}, clear=False):
            with mock.patch.dict("os.environ", {}, clear=False):
                import os
                os.environ.pop("HIVE_SESSION_ID", None)
                self.assertEqual(ledger.resolve_session(), "cc-123")
        self.assertTrue(ledger.is_anonymous("proc-1-x") and ledger.is_anonymous(""))
        self.assertFalse(ledger.is_anonymous("cc-123"))

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


class HotFileEmitTest(unittest.TestCase):
    """TRACE-05: >= 3 errored tool calls on one file in a session -> HOT_FILE."""

    def test_emits_only_for_files_at_threshold_under_the_files_own_project(self):
        import json
        import tempfile
        from pathlib import Path
        from unittest import mock
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "PROJECTS"
            a = str(root / "Demo App" / "src" / "a.rs")
            b = str(root / "Demo App" / "src" / "b.rs")
            outside = str(Path(tmp) / "vault" / "AGENTS.md")
            a_alias = a.lower() if os.name == "nt" else a  # case-insensitive paths only on Windows
            calls = [(a, True), (a_alias, True), (a, True), (b, True), (b, True), (b, False)] + [(outside, True)] * 3
            lines = []
            for i, (path, err) in enumerate(calls):
                lines.append({"message": {"content": [{"type": "tool_use", "id": f"t{i}", "name": "Edit",
                                                       "input": {"file_path": path}}]}})
                lines.append({"message": {"content": [{"type": "tool_result", "tool_use_id": f"t{i}", "is_error": err,
                                                       "content": "boom"}]}})
            lines.append({"message": {"content": [{"type": "tool_use", "id": "g", "name": "Edit",
                                                   "input": {"file_path": b}}]}})
            lines.append({"message": {"content": [{"type": "tool_result", "tool_use_id": "g", "is_error": True,
                                                   "content": "PreToolUse:Write hook error: gate"}]}})
            tr = Path(tmp) / "t.jsonl"
            tr.write_text("\n".join(json.dumps(x) for x in lines) + "\nnot json\n", encoding="utf-8")
            norm = lambda p: os.path.normcase(os.path.normpath(p))
            self.assertEqual(traces.failed_files(tr), {norm(a): 3, norm(b): 2, norm(outside): 3})
            db = sqlite3.connect(":memory:")
            with mock.patch.object(traces.ace, "PROJECTS", root):
                self.assertEqual(traces.project_of(a_alias), ("demo-app", "src/a.rs"))
                self.assertIsNone(traces.project_of(outside))
                self.assertEqual(traces.emit_hot_files(db, tr, "claude", "s1", 0), [("demo-app", "src/a.rs")])
            (t, _), = traces.active(db, "demo-app", 0)
            self.assertEqual((t["kind"], t["target"]), ("HOT_FILE", "src/a.rs"))
            self.assertEqual(traces.active(db, "global", 0), [])
            self.assertEqual(traces.failed_files(Path(tmp) / "missing.jsonl"), {})
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

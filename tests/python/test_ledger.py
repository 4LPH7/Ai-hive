import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import ledger
import hive


class LedgerTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.ledger_path = self.root / "test_ledger.jsonl"

    def tearDown(self):
        self.tmp.cleanup()

    def test_record_and_read_roundtrip(self):
        ok = ledger.record(
            self.ledger_path,
            event="query",
            project="demo",
            query="test query",
            hits=["notes/a.md", "notes/b.md"],
            injected_bullets=["str-00001"],
            duration_ms=14.5,
            agent="antigravity",
            session_id="sess-001",
        )
        self.assertTrue(ok)
        self.assertTrue(self.ledger_path.is_file())

        entries = ledger.read_ledger(self.ledger_path)
        self.assertEqual(len(entries), 1)
        e = entries[0]
        self.assertEqual(e["event"], "query")
        self.assertEqual(e["project"], "demo")
        self.assertEqual(e["agent"], "antigravity")
        self.assertEqual(e["session_id"], "sess-001")
        self.assertEqual(e["hits"], ["notes/a.md", "notes/b.md"])
        self.assertEqual(e["injected_bullets"], ["str-00001"])
        self.assertEqual(e["duration_ms"], 14.5)

    def test_tail_and_project_filters(self):
        for i in range(10):
            proj = "alpha" if i % 2 == 0 else "beta"
            ledger.record(
                self.ledger_path,
                event="query",
                project=proj,
                query=f"q{i}",
                hits=[],
                injected_bullets=[],
                duration_ms=float(i),
            )

        all_entries = ledger.read_ledger(self.ledger_path)
        self.assertEqual(len(all_entries), 10)

        tail_3 = ledger.read_ledger(self.ledger_path, tail=3)
        self.assertEqual(len(tail_3), 3)
        self.assertEqual(tail_3[-1]["query"], "q9")

        alpha_entries = ledger.read_ledger(self.ledger_path, project="alpha")
        self.assertEqual(len(alpha_entries), 5)
        self.assertTrue(all(e["project"] == "alpha" for e in alpha_entries))

    def test_fail_open_on_unwritable_path(self):
        # A path pointing to a directory should fail to open as append file, but record must not crash
        bad_path = self.root / "a_dir"
        bad_path.mkdir()
        # On POSIX and Windows, opening a directory in 'a' mode raises IsADirectoryError / PermissionError
        ok = ledger.record(bad_path, event="query", project="fail-open")
        self.assertFalse(ok)

    def test_hive_search_and_context_record_telemetry(self):
        vault = self.root / "vault"
        vault.mkdir(parents=True, exist_ok=True)
        proj_dir = vault / "projects" / "testproj"
        proj_dir.mkdir(parents=True, exist_ok=True)
        (proj_dir / "note.md").write_text("---\ntype: note\nproject: testproj\n---\n# Note\n\nContent\n", encoding="utf-8")

        db_path = self.root / "hive.sqlite"

        with mock.patch.object(hive, "VAULT", vault), \
             mock.patch.object(hive, "DB_PATH", db_path), \
             mock.patch.object(hive, "LEDGER_PATH", self.ledger_path):
            idx = hive.Index(vault=vault, db_path=db_path)
            try:
                with mock.patch.object(hive, "index", return_value=idx):
                    idx.refresh(force=True)
                    res = hive.tool_search("Content", project="testproj", scope="project")
                    self.assertIn("note.md", res)

                    entries = ledger.read_ledger(self.ledger_path)
                    self.assertGreaterEqual(len(entries), 1)
                    self.assertEqual(entries[-1]["event"], "query")
                    self.assertEqual(entries[-1]["project"], "testproj")
            finally:
                idx.db.close()


if __name__ == "__main__":
    unittest.main()

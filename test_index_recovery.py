"""The disposable index heals itself: a corrupt hive.sqlite is moved aside (or bypassed while locked) and rebuilt
from the vault, instead of every search silently returning nothing."""
import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import hive

NOTE = "---\ntype: note\nproject: demo\nagent: claude\nstatus: active\nupdated: 2026-11-26\n---\n# Cache keys\n\nzebracache rule\n"


class IndexRecoveryTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.vault = root / "vault"
        (self.vault / "projects" / "demo").mkdir(parents=True)
        (self.vault / "projects" / "demo" / "keys.md").write_text(NOTE, encoding="utf-8")
        self.db = root / "hive.sqlite"

    def tearDown(self):
        self.tmp.cleanup()

    def corrupt(self, path):
        path.write_bytes(b"SQLite format 3\x00" + b"\xff" * 4096)  # valid magic, garbage pages

    def search_paths(self, idx):
        return [h["path"] for h in idx.search("zebracache", "demo", 5)]

    def test_corrupt_file_is_moved_aside_and_rebuilt(self):
        self.corrupt(self.db)
        idx = hive.Index(self.vault, self.db)
        try:
            self.assertEqual(self.search_paths(idx), ["projects/demo/keys.md"])
        finally:
            idx.db.close()
        self.assertEqual(len(list(self.db.parent.glob("hive.sqlite.corrupt-*"))), 1, "kept for inspection")
        check = sqlite3.connect(str(self.db))
        try:
            self.assertEqual(check.execute("PRAGMA quick_check").fetchone()[0], "ok")
        finally:
            check.close()

    def test_locked_corrupt_file_falls_back_to_shared_recovered_path(self):
        self.corrupt(self.db)
        with mock.patch("os.replace", side_effect=PermissionError("in use")):
            idx = hive.Index(self.vault, self.db)
            other = hive.Index(self.vault, self.db)
        try:
            self.assertEqual(idx.db_path, self.db.with_name("hive.sqlite.recovered"))
            self.assertEqual(other.db_path, idx.db_path, "every new process shares the same rebuilt index")
            self.assertEqual(self.search_paths(idx), ["projects/demo/keys.md"])
        finally:
            idx.db.close()
            other.db.close()

    def test_corruption_during_a_long_running_session_recovers_once(self):
        idx = hive.Index(self.vault, self.db)
        real = idx._search
        calls = []

        def flaky(*a, **k):
            calls.append(1)
            if len(calls) == 1:
                raise sqlite3.DatabaseError("database disk image is malformed")
            return real(*a, **k)

        with mock.patch.object(idx, "_search", side_effect=flaky), \
                mock.patch.object(hive, "_healthy", side_effect=[False, True]):
            self.assertEqual(self.search_paths(idx), ["projects/demo/keys.md"])
        idx.db.close()

    def test_healthy_db_is_left_alone(self):
        idx = hive.Index(self.vault, self.db)
        self.search_paths(idx)
        idx.db.close()
        idx = hive.Index(self.vault, self.db)
        idx.db.close()
        self.assertEqual(list(self.db.parent.glob("hive.sqlite.corrupt-*")), [])
        conn, used = hive.open_db(self.db)
        conn.close()
        self.assertEqual(used, self.db)


if __name__ == "__main__":
    unittest.main()

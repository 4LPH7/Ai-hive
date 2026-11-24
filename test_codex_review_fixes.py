import os
import shutil
import tempfile
import unittest
from pathlib import Path

import curate
import hive
import miner
import skill


class TestCodexReviewFixes(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.tmp_path = Path(self.tmp)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_curation_telemetry_filtered_by_project(self):
        """Ensure scan_ledger_gaps only returns misses belonging to target project."""
        ledger_file = self.tmp_path / "ledger.jsonl"
        lines = [
            '{"ts": "2026-11-24T10:00:00Z", "project": "proj-a", "event": "query", "query": "miss-a", "hits": []}\n',
            '{"ts": "2026-11-24T10:01:00Z", "project": "proj-b", "event": "query", "query": "miss-b", "hits": []}\n',
            '{"ts": "2026-11-24T10:02:00Z", "project": "proj-a", "event": "query", "query": "hit-a", "hits": ["note1"]}\n',
        ]
        ledger_file.write_text("".join(lines), encoding="utf-8")

        gaps_a = miner.scan_ledger_gaps(str(ledger_file), project="proj-a")
        self.assertEqual(len(gaps_a), 1)
        self.assertEqual(gaps_a[0][0], "miss-a")

        gaps_b = miner.scan_ledger_gaps(str(ledger_file), project="proj-b")
        self.assertEqual(len(gaps_b), 1)
        self.assertEqual(gaps_b[0][0], "miss-b")

        gaps_all = miner.scan_ledger_gaps(str(ledger_file), project=None)
        self.assertEqual(len(gaps_all), 2)

    def test_search_scope_defaults_to_project_when_project_supplied(self):
        """Ensure tool_search defaults scope to 'project' when project is provided and scope is None."""
        vault_dir = self.tmp_path / "vault"
        vault_dir.mkdir()
        (vault_dir / "projects").mkdir()
        (vault_dir / "projects" / "alpha").mkdir(parents=True)
        (vault_dir / "projects" / "beta").mkdir(parents=True)

        (vault_dir / "projects" / "alpha" / "note1.md").write_text(
            "---\ntype: note\nproject: alpha\n---\n# Secret Architecture\nAlpha internals\n", encoding="utf-8"
        )
        (vault_dir / "projects" / "beta" / "note2.md").write_text(
            "---\ntype: note\nproject: beta\n---\n# Secret Architecture\nBeta internals\n", encoding="utf-8"
        )

        old_vault = hive.VAULT
        old_idx = hive._INDEX
        old_force = os.environ.get("HIVE_FORCE_PYTHON")
        os.environ["HIVE_FORCE_PYTHON"] = "1"
        try:
            hive.VAULT = vault_dir
            hive.DB_PATH = self.tmp_path / "test_search.sqlite"
            hive._INDEX = hive.Index(vault_dir, hive.DB_PATH)
            hive._INDEX.refresh(force=True)

            # When project='alpha' and scope=None, should only return alpha's note
            res = hive.tool_search("Secret Architecture", project="alpha", scope=None)
            self.assertIn("projects/alpha/note1.md", res)
            self.assertNotIn("projects/beta/note2.md", res)

            # When scope='all', returns both
            res_all = hive.tool_search("Secret Architecture", project="alpha", scope="all")
            self.assertIn("projects/alpha/note1.md", res_all)
            self.assertIn("projects/beta/note2.md", res_all)
        finally:
            hive.VAULT = old_vault
            if hive._INDEX:
                hive._INDEX.db.close()
            hive._INDEX = old_idx
            if old_force is None:
                os.environ.pop("HIVE_FORCE_PYTHON", None)
            else:
                os.environ["HIVE_FORCE_PYTHON"] = old_force

    def test_search_candidate_project_filter_before_limit(self):
        """Ensure notes from target project are returned even if other projects have many higher-BM25 hits."""
        vault_dir = self.tmp_path / "vault"
        vault_dir.mkdir()
        (vault_dir / "projects" / "target").mkdir(parents=True)

        # 10 notes in other project with heavy repetitions of "database"
        for i in range(10):
            (vault_dir / "projects" / f"other_{i}").mkdir(parents=True)
            (vault_dir / "projects" / f"other_{i}" / "doc.md").write_text(
                f"---\ntype: note\nproject: other_{i}\n---\n# Database database database\nHeavy repetition {i}\n",
                encoding="utf-8",
            )

        # 1 note in target project with single match
        (vault_dir / "projects" / "target" / "target_note.md").write_text(
            "---\ntype: note\nproject: target\n---\n# Database overview\nTarget project note\n", encoding="utf-8"
        )

        db_path = self.tmp_path / "test_limit.sqlite"
        idx = hive.Index(vault_dir, db_path)
        try:
            idx.refresh(force=True)

            # Limit = 2, so limit * 4 = 8. Without project filter before limit, the 10 'other' notes
            # would fill all 8 slots, pushing target_note out completely.
            hits = idx.search("database", project="target", limit=2, scope="project")
            paths = [h["path"] for h in hits]
            self.assertIn("projects/target/target_note.md", paths)
        finally:
            idx.db.close()

    def test_skill_install_failure_preserves_existing_directory(self):
        """Ensure failed install does not wipe pre-existing skill directory and local files."""
        existing_skill_dir = self.tmp_path / "my-skill"
        existing_skill_dir.mkdir(parents=True)
        canary_file = existing_skill_dir / "my_custom_code.py"
        canary_file.write_text("print('precious local work')", encoding="utf-8")

        # Attempt to install a non-existent package that fails
        res = skill.install_skill("non-existent-package-xyz-123", target_dir=existing_skill_dir)
        self.assertEqual(res["status"], "error")

        # Crucial check: existing directory and canary file must still exist untouched
        self.assertTrue(existing_skill_dir.is_dir())
        self.assertTrue(canary_file.is_file())
        self.assertEqual(canary_file.read_text(encoding="utf-8"), "print('precious local work')")

    def test_force_refresh_bypasses_mtime_cache(self):
        """Ensure force=True re-hashes notes even when mtime and size are identical."""
        vault_dir = self.tmp_path / "vault_refresh"
        vault_dir.mkdir()
        note = vault_dir / "note.md"
        note.write_text("---\ntype: note\n---\n# Original\nContent 1\n", encoding="utf-8")
        st = note.stat()

        db_path = self.tmp_path / "refresh.sqlite"
        idx = hive.Index(vault_dir, db_path)
        try:
            idx.refresh(force=True)
            row = idx.db.execute("SELECT sha256 FROM notes WHERE path='note.md'").fetchone()
            orig_hash = row[0]

            # Modify note content but artificially keep mtime and size identical
            note.write_text("---\ntype: note\n---\n# Modified\nContent 2\n", encoding="utf-8")
            os.utime(str(note), (st.st_atime, st.st_mtime))

            # Non-forced refresh skips due to (mtime, size) match
            idx.refresh(force=False)
            row2 = idx.db.execute("SELECT sha256 FROM notes WHERE path='note.md'").fetchone()
            self.assertEqual(row2[0], orig_hash)

            # Forced refresh re-hashes and updates DB
            idx.refresh(force=True)
            row3 = idx.db.execute("SELECT sha256 FROM notes WHERE path='note.md'").fetchone()
            self.assertNotEqual(row3[0], orig_hash)
        finally:
            idx.db.close()

    def test_curate_payload_malformed_json_types(self):
        """Ensure curate handles non-dict/non-list payloads without raising unhandled exceptions."""
        import llm

        old_chat = llm.chat_completion
        llm.chat_completion = lambda cfg, model, sys_p, usr_p, schema=None: '["unexpected", "list", "instead", "of", "dict"]'
        try:
            vault_dir = self.tmp_path / "vault_curate"
            vault_dir.mkdir()
            (vault_dir / "inbox").mkdir()
            res = curate.run_curate(
                ledger_path=self.tmp_path / "missing.jsonl",
                vault_path=vault_dir,
                project="ai-hive",
                dry_run=True,
            )
            self.assertEqual(res["merges_count"], 0)
            self.assertFalse(res["skipped"])
        finally:
            llm.chat_completion = old_chat


if __name__ == "__main__":
    unittest.main()

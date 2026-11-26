import datetime
import json
import os
import shutil
import tempfile
import unittest
from pathlib import Path

import ace
import lint


class TestDecayAndStaleness(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.vault = self.tmp / "vault"
        self.vault.mkdir(parents=True)
        self.projects = self.tmp / "projects"
        self.projects.mkdir(parents=True)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_playbook_time_decay_prunes_stale_bullets(self):
        # Create a playbook with 3 bullets:
        # 1: Stale: seen=12, updated=2026-09-01 (over 80 days ago), helpful=0, harmful=0 -> should decay & prune
        # 2: Active: seen=15, updated=2026-09-01, helpful=2, harmful=0 -> helpful > 0, should NOT prune
        # 3: Unseen/Low seen: seen=5, updated=2026-09-01, helpful=0, harmful=0 -> seen < 10, should NOT prune
        proj_dir = self.vault / "projects" / "test-proj"
        proj_dir.mkdir(parents=True)
        playbook = proj_dir / "playbook.md"

        content = (
            "---\ntype: note\nproject: test-proj\nstatus: active\nupdated: 2026-11-24\n---\n"
            "# Playbook: test-proj\n\n"
            "## strategies\n"
            "- [str-00001] seen=12 updated=2026-09-01 helpful=0 harmful=0 :: Stale forgotten rule\n"
            "- [str-00002] seen=15 updated=2026-09-01 helpful=2 harmful=0 :: Helpful proven rule\n"
            "- [str-00003] seen=5 updated=2026-09-01 helpful=0 harmful=0 :: Rarely tested rule\n\n"
            "## pitfalls\n\n"
            "## commands\n\n"
            "## conventions\n"
        )
        playbook.write_text(content, encoding="utf-8")

        # Parse test
        sections = ace.parse(content)
        self.assertEqual(len(sections["strategies"]), 3)
        self.assertEqual(sections["strategies"][0]["updated"], "2026-09-01")

        # Apply an empty/dummy op to trigger pruning
        old_vault = ace.VAULT
        ace.VAULT = self.vault
        try:
            report = ace.apply("test-proj", [], today="2026-11-24")
        finally:
            ace.VAULT = old_vault

        self.assertIn("str-00001", report["pruned"])
        self.assertNotIn("str-00002", report["pruned"])
        self.assertNotIn("str-00003", report["pruned"])

        # Check remaining bullets in playbook
        new_sections = ace.parse(playbook.read_text(encoding="utf-8"))
        remaining_ids = [b["id"] for b in new_sections["strategies"]]
        self.assertNotIn("str-00001", remaining_ids)
        self.assertIn("str-00002", remaining_ids)
        self.assertIn("str-00003", remaining_ids)

    def test_adr_staleness_lint_with_graphify_manifest(self):
        # 1. Setup project repo with graphify-out
        proj_repo = self.projects / "ai-hive"
        graphify_dir = proj_repo / "graphify-out"
        graphify_dir.mkdir(parents=True)

        manifest = {
            "symbols": ["PlaybookLock", "Index::search", "score_session"],
            "deleted_symbols": ["LegacySyncRunner", "OldCacheManager"],
        }
        (graphify_dir / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")

        # 2. Create ADRs in vault
        adrs_dir = self.vault / "projects" / "ai-hive" / "adrs"
        adrs_dir.mkdir(parents=True)

        # ADR 1: References deleted symbol
        adr_deleted = adrs_dir / "0001-legacy-sync.md"
        adr_deleted.write_text(
            "---\ntype: adr\nproject: ai-hive\nagent: claude\nstatus: active\nupdated: 2026-11-24\n---\n"
            "# 0001 Legacy Sync\n\nUses `LegacySyncRunner` to handle background jobs.\n",
            encoding="utf-8",
        )

        # ADR 2: References active valid symbol
        adr_valid = adrs_dir / "0002-playbook-locking.md"
        adr_valid.write_text(
            "---\ntype: adr\nproject: ai-hive\nagent: claude\nstatus: active\nupdated: 2026-11-24\n---\n"
            "# 0002 Playbook Locking\n\nUses `PlaybookLock` to prevent lost updates.\n",
            encoding="utf-8",
        )

        # ADR 3: References unknown code symbol (missing from graph)
        adr_missing = adrs_dir / "0003-quantum-router.md"
        adr_missing.write_text(
            "---\ntype: adr\nproject: ai-hive\nagent: claude\nstatus: active\nupdated: 2026-11-24\n---\n"
            "# 0003 Quantum Router\n\nIntegrates with `QuantumMeshRouter`.\n",
            encoding="utf-8",
        )

        # Check ADR staleness
        findings_deleted = lint.check_adr_staleness(
            "projects/ai-hive/adrs/0001-legacy-sync.md",
            adr_deleted.read_text(encoding="utf-8"),
            self.projects,
        )
        self.assertEqual(len(findings_deleted), 1)
        self.assertEqual(findings_deleted[0].code, "adr-stale")
        self.assertIn("deleted symbol `LegacySyncRunner`", findings_deleted[0].message)

        findings_valid = lint.check_adr_staleness(
            "projects/ai-hive/adrs/0002-playbook-locking.md",
            adr_valid.read_text(encoding="utf-8"),
            self.projects,
        )
        self.assertEqual(len(findings_valid), 0)

        findings_missing = lint.check_adr_staleness(
            "projects/ai-hive/adrs/0003-quantum-router.md",
            adr_missing.read_text(encoding="utf-8"),
            self.projects,
        )
        self.assertEqual(len(findings_missing), 1)
        self.assertEqual(findings_missing[0].code, "adr-stale")
        self.assertIn("QuantumMeshRouter", findings_missing[0].message)


if __name__ == "__main__":
    unittest.main()

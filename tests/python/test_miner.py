import os
import shutil
import tempfile
import unittest
from pathlib import Path

import ledger
import miner


class TestMiner(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.vault = self.tmp / "vault"
        self.vault.mkdir(parents=True)
        self.ledger_file = self.tmp / "ledger.jsonl"

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_mine_clusters_and_generates_proposals(self):
        # 1. Record search queries with zero hits
        ledger.record(
            self.ledger_file,
            event="query",
            project="ai-hive",
            query="how to configure redis cache",
            hits=[],
            session_id="s1",
        )
        ledger.record(
            self.ledger_file,
            event="query",
            project="ai-hive",
            query="redis cache configuration setup",
            hits=[],
            session_id="s2",
        )
        # An unrelated query with only 1 occurrence
        ledger.record(
            self.ledger_file,
            event="query",
            project="ai-hive",
            query="kubernetes ingress helm chart",
            hits=[],
            session_id="s3",
        )
        # A query with hits (not a gap)
        ledger.record(
            self.ledger_file,
            event="query",
            project="ai-hive",
            query="sqlite fts5 search",
            hits=["notes/sqlite.md"],
            session_id="s4",
        )

        # 2. Mine gaps with min_count=2
        res = miner.mine_gaps(
            ledger_path=self.ledger_file,
            vault_path=self.vault,
            min_count=2,
            dry_run=False,
        )

        self.assertEqual(res["total_gaps"], 3)
        self.assertEqual(res["proposals_generated"], 1)

        proposal = res["proposals"][0]
        self.assertEqual(proposal["count"], 2)
        prop_file = Path(proposal["file"])
        self.assertTrue(prop_file.is_file())

        content = prop_file.read_text(encoding="utf-8")
        self.assertIn("type: proposal", content)
        self.assertIn("status: inbox", content)
        self.assertIn("source: hive-miner", content)
        self.assertIn("redis cache", content.lower())
        self.assertIn("## Proposed Actions", content)

    def test_mine_idempotency(self):
        ledger.record(
            self.ledger_file,
            event="query",
            project="ai-hive",
            query="oauth2 pkce authentication flow",
            hits=[],
        )
        ledger.record(
            self.ledger_file,
            event="query",
            project="ai-hive",
            query="oauth2 authentication flow pkce",
            hits=[],
        )

        # Run 1
        res1 = miner.mine_gaps(self.ledger_file, self.vault, min_count=2)
        inbox_files_1 = list((self.vault / "inbox").glob("*.md"))
        self.assertEqual(len(inbox_files_1), 1)

        # Run 2
        res2 = miner.mine_gaps(self.ledger_file, self.vault, min_count=2)
        inbox_files_2 = list((self.vault / "inbox").glob("*.md"))
        self.assertEqual(len(inbox_files_2), 1)
        self.assertEqual(res1["proposals"][0]["hash"], res2["proposals"][0]["hash"])

    def test_mine_dry_run(self):
        ledger.record(
            self.ledger_file,
            event="skill",
            project="",
            query="deploy to aws ecs",
            hits=[],
        )
        ledger.record(
            self.ledger_file,
            event="skill",
            project="",
            query="aws ecs deploy service",
            hits=[],
        )

        res = miner.mine_gaps(self.ledger_file, self.vault, min_count=2, dry_run=True)
        self.assertTrue(res["dry_run"])
        self.assertEqual(res["proposals_generated"], 1)
        self.assertFalse((self.vault / "inbox").exists())


if __name__ == "__main__":
    unittest.main()

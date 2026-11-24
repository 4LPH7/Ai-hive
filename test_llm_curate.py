import json
import os
import shutil
import tempfile
import unittest
from pathlib import Path

import ace
import curate
import llm


class TestLlmCurate(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.vault = self.tmp / "vault"
        self.vault.mkdir(parents=True)
        self.ledger_file = self.tmp / "ledger.jsonl"

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_privacy_hard_lock_never_leaves_private(self):
        cfg = {
            "enabled": True,
            "base_url": "http://127.0.0.1:20128/v1",
            "default_privacy": "private",
            "combos": {
                "private": "hive-private",
                "fast": "hive-fast",
                "deep": "hive-deep",
            },
            "project_privacies": {
                "offmesh": "private",
                "open-source": "public",
            },
        }

        # 1. Project explicitly marked private locks to hive-private
        self.assertEqual(llm.resolve_combo(cfg, "fast", project="offmesh"), "hive-private")
        self.assertEqual(llm.resolve_combo(cfg, "deep", project="offmesh"), "hive-private")
        self.assertEqual(llm.resolve_combo(cfg, "private", project="offmesh"), "hive-private")

        # 2. Project explicitly public can access fast/deep
        self.assertEqual(llm.resolve_combo(cfg, "fast", project="open-source"), "hive-fast")
        self.assertEqual(llm.resolve_combo(cfg, "deep", project="open-source"), "hive-deep")

        # 3. Unspecified project defaults to default_privacy ("private")
        self.assertEqual(llm.resolve_combo(cfg, "fast", project="unknown-proj"), "hive-private")

    def test_pre_egress_secret_scrubbing(self):
        raw_text = "Deploying with api_key=sk-1234567890abcdef123456 and token=ghp_abcdef1234567890abcdef"
        scrubbed = ace.scrub_secrets(raw_text)
        self.assertNotIn("sk-1234567890abcdef123456", scrubbed)
        self.assertNotIn("ghp_abcdef1234567890abcdef", scrubbed)
        self.assertIn("[REDACTED_SECRET]", scrubbed)

    def test_curate_fails_open_when_offline(self):
        # Point to an unreachable port
        old_base = os.environ.get("HIVE_LLM_BASE_URL")
        os.environ["HIVE_LLM_BASE_URL"] = "http://127.0.0.1:59999/v1"
        try:
            res = curate.run_curate(
                ledger_path=self.ledger_file,
                vault_path=self.vault,
                project="ai-hive",
                dry_run=False,
            )
            # Must fail open cleanly without raising uncaught exception
            self.assertTrue(res["skipped"])
            self.assertIn("fail-open", res["reason"].lower())
            self.assertIsNone(res["proposal_file"])
        finally:
            if old_base is not None:
                os.environ["HIVE_LLM_BASE_URL"] = old_base
            else:
                os.environ.pop("HIVE_LLM_BASE_URL", None)

    def test_deterministic_validation_gate_rejects_hallucinations_and_secrets(self):
        # Setup project playbook
        proj_dir = self.vault / "projects" / "ai-hive"
        proj_dir.mkdir(parents=True)
        playbook = proj_dir / "playbook.md"
        playbook.write_text(
            "---\ntype: note\nproject: ai-hive\nstatus: active\n---\n"
            "# Playbook\n\n## strategies\n- [str-00001] seen=1 helpful=1 harmful=0 :: Real strategy\n\n## pitfalls\n\n## commands\n\n## conventions\n",
            encoding="utf-8",
        )

        # Mock LLM returning hallucinated ID and secret
        mock_payload = {
            "paraphrase_merges": [
                {
                    "target_id": "str-99999",  # hallucinated!
                    "merged_content": "Hallucinated target",
                    "rationale": "fake",
                },
                {
                    "target_id": "str-00001",  # real ID, but contains secret
                    "merged_content": "Use secret sk-1234567890abcdef123456 for auth",
                    "rationale": "leaked",
                },
                {
                    "target_id": "str-00001",  # valid
                    "merged_content": "Consolidated clean strategy",
                    "rationale": "clearer wording",
                },
            ],
            "contradictions": [
                {
                    "bullet_a": "str-00001",
                    "bullet_b": "str-99999",  # hallucinated!
                    "issue": "conflict",
                }
            ],
            "adr_drift_warnings": [],
            "query_alias_proposals": [
                {
                    "cluster_topic": "fts5 search",
                    "aliases": ["sqlite search", "full text search"],
                    "suggested_note": "notes/sqlite.md",
                }
            ],
        }

        # Mock chat_completion to return this payload
        old_chat = llm.chat_completion
        llm.chat_completion = lambda cfg, model, sys_p, usr_p, schema=None: json.dumps(mock_payload)
        try:
            res = curate.run_curate(
                ledger_path=self.ledger_file,
                vault_path=self.vault,
                project="ai-hive",
                dry_run=False,
            )
            self.assertFalse(res["skipped"])
            # Only the 1 clean merge should pass
            self.assertEqual(res["merges_count"], 1)
            # Hallucinated contradiction should be dropped
            self.assertEqual(res["contradictions_count"], 0)
            self.assertEqual(res["alias_proposals_count"], 1)

            # Check proposal note was written exclusively to inbox/
            self.assertIsNotNone(res["proposal_file"])
            prop_p = Path(res["proposal_file"])
            self.assertTrue(prop_p.is_file())
            content = prop_p.read_text(encoding="utf-8")
            self.assertIn("Consolidated clean strategy", content)
            self.assertNotIn("Hallucinated target", content)
            self.assertNotIn("sk-1234567890abcdef123456", content)

            # Playbook itself MUST be untouched
            playbook_after = playbook.read_text(encoding="utf-8")
            self.assertIn("- [str-00001] seen=1 helpful=1 harmful=0 :: Real strategy", playbook_after)
        finally:
            llm.chat_completion = old_chat


if __name__ == "__main__":
    unittest.main()

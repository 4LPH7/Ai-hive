import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import hooks.codex_stop_hook as codex_hook
import hooks.antigravity_sync as antigravity_hook
import ledger
import ace
import hive


class HooksParityTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.vault = self.root / "vault"
        self.vault.mkdir(parents=True, exist_ok=True)
        self.ledger_path = self.root / "ledger.jsonl"

    def tearDown(self):
        self.tmp.cleanup()

    def test_codex_stop_hook_dry_run_and_ledger(self):
        with mock.patch.object(hive, "load_config", return_value={"ledger": self.ledger_path, "vault": self.vault}):
            # Dry run produces message and logs to ledger
            exit_code = codex_hook.run_codex_stop(dry_run=True)
            self.assertEqual(exit_code, 0)

            entries = ledger.read_ledger(self.ledger_path)
            self.assertGreaterEqual(len(entries), 1)
            self.assertEqual(entries[-1]["event"], "reflect")
            self.assertEqual(entries[-1]["agent"], "codex")

    def test_codex_stop_hook_fail_open_on_exception(self):
        # Even if hive raises unexpected exception, hook must exit 0 cleanly
        with mock.patch("hive.load_config", side_effect=RuntimeError("critical failure")):
            exit_code = codex_hook.run_codex_stop(dry_run=True)
            self.assertEqual(exit_code, 0)

    def test_antigravity_sync_start_and_stop(self):
        with mock.patch.object(hive, "load_config", return_value={"ledger": self.ledger_path, "vault": self.vault}):
            # Start
            ctx = antigravity_hook.session_start(scope="ai-hive")
            # Should record inject event
            entries = ledger.read_ledger(self.ledger_path)
            self.assertTrue(any(e["event"] == "inject" and e["agent"] == "antigravity" for e in entries))

            # Stop
            res = antigravity_hook.session_stop(scope="ai-hive", summary="finished feature")
            self.assertEqual(res["status"], "ok")

            entries2 = ledger.read_ledger(self.ledger_path)
            self.assertTrue(any(e["event"] == "reflect" and e["agent"] == "antigravity" for e in entries2))

    def test_antigravity_sync_fail_open(self):
        with mock.patch("hive.load_config", side_effect=Exception("unhandled")):
            ctx = antigravity_hook.session_start(scope="fail-open")
            self.assertEqual(ctx, "")
            res = antigravity_hook.session_stop(scope="fail-open")
            self.assertEqual(res["status"], "error")


if __name__ == "__main__":
    unittest.main()

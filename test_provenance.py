"""Phase 6 (v2.5.1) provenance: PROV-01..04."""
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import ace
import hive
import ledger


class ProvenanceTest(unittest.TestCase):
    def setUp(self):
        ledger._CONTEXT.update(agent=None, session_id=None)
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.ledger_path = self.root / "ledger.jsonl"

    def tearDown(self):
        ledger._CONTEXT.update(agent=None, session_id=None)
        self.tmp.cleanup()

    def test_agent_from_hint_maps_clients_and_transcript_paths(self):
        self.assertEqual(ledger.agent_from_hint("claude-code"), "claude")
        self.assertEqual(ledger.agent_from_hint(r"C:\Users\x\.claude\projects\a\s.jsonl"), "claude")
        self.assertEqual(ledger.agent_from_hint("codex-mcp-client"), "codex")
        self.assertEqual(ledger.agent_from_hint("Antigravity IDE"), "antigravity")
        self.assertEqual(ledger.agent_from_hint("gemini-cli"), "antigravity")
        self.assertEqual(ledger.agent_from_hint("Cursor Editor"), "cursor-editor")
        self.assertIsNone(ledger.agent_from_hint("  "))
        self.assertIsNone(ledger.agent_from_hint(r"C:\Temp\tmp1\t.jsonl"))

    def test_session_never_empty_and_precedence(self):
        with mock.patch.dict("os.environ", {"HIVE_SESSION_ID": ""}, clear=False):
            self.assertTrue(ledger.resolve_session(None).startswith("proc-"))
            ledger.set_context(session_id="hook-sess")
            self.assertEqual(ledger.resolve_session(None), "hook-sess")
            self.assertEqual(ledger.resolve_session("explicit"), "explicit")
        ledger.set_context(agent="codex")
        self.assertEqual(ledger.resolve_agent(None), "codex")
        self.assertEqual(ledger.resolve_agent("claude"), "claude")
        ledger.set_context(agent="  ")  # empty never overwrites
        self.assertEqual(ledger.resolve_agent(None), "codex")

    def test_record_uses_context_and_coverage_counts(self):
        self.ledger_path.write_text(json.dumps({"ts": "t", "session_id": "", "agent": "unknown", "event": "query"}) + "\n",
                                    encoding="utf-8")
        ledger.set_context(agent="claude", session_id="s-1")
        ledger.record(self.ledger_path, event="query", project="demo", query="q")
        entries = ledger.read_ledger(self.ledger_path)
        self.assertEqual((entries[1]["agent"], entries[1]["session_id"]), ("claude", "s-1"))
        self.assertNotIn("repo_sha", entries[1])  # optional fields only on ace_apply
        c = ledger.coverage(entries)
        self.assertEqual(c, {"total": 2, "with_session": 1, "with_agent": 1, "fallback_session": 0})

    def test_ace_apply_records_added_ids_and_hashes(self):
        vault = self.root / "vault"
        with mock.patch.object(ace, "VAULT", vault), \
                mock.patch.object(hive, "load_config", return_value={"ledger": self.ledger_path, "vault": vault}):
            ledger.set_context(agent="claude", session_id="s-apply")
            report = ace.apply("global", [{"type": "ADD", "section": "pitfalls", "content": "provenance test lesson"}])
        events = [e for e in ledger.read_ledger(self.ledger_path) if e["event"] == "ace_apply"]
        self.assertEqual(len(events), 1)
        self.assertEqual(events[0]["injected_bullets"], report["added"])
        self.assertEqual(events[0]["session_id"], "s-apply")
        self.assertEqual(len(events[0]["content_sha256"]), 64)

    def test_hook_stdin_sets_session_and_agent(self):
        payload = {"session_id": "claude-sess-9", "transcript_path": r"C:\Users\x\.claude\projects\p\t.jsonl"}
        with mock.patch.object(sys, "stdin", io.StringIO(json.dumps(payload))):
            ace.read_hook_input()
        self.assertEqual(ledger.resolve_session(None), "claude-sess-9")
        self.assertEqual(ledger.resolve_agent(None), "claude")

    def test_mcp_initialize_client_info_sets_agent(self):
        hive.handle({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                     "params": {"protocolVersion": "2025-11-25", "clientInfo": {"name": "codex-mcp-client"}}})
        self.assertEqual(ledger.resolve_agent(None), "codex")
        hive.handle({"jsonrpc": "2.0", "id": 2, "method": "tools/list",
                     "params": {"_meta": {"io.modelcontextprotocol/clientInfo": {"name": "Antigravity"}}}})
        self.assertEqual(ledger.resolve_agent(None), "antigravity")

    def test_hooks_are_not_delegated_to_rust(self):
        src = Path(ace.__file__).read_text(encoding="utf-8")
        main_block = src[src.index('if __name__ == "__main__":'):]
        self.assertNotIn('"hook-start"', main_block)
        self.assertNotIn('"hook-stop"', main_block)


if __name__ == "__main__":
    unittest.main()

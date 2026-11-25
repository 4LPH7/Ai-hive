"""Phase 7 gate (v2.6.0): 2-agent simulation over traces + codices.

Agent A leaves a DEAD_END trace and a codex with an open thread; agent B's hive_context for an overlapping
task shows both, the traces slice builds in < 10 ms, decay hides the trace on a fake clock, and `trace add`
mutates no canonical Markdown. Recall@5 is checked separately by `hive.py eval`.
"""
import hashlib
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

import codex
import hive
import traces

TASK = "fix the tokenizer link error in src/tokenizer.rs"
H = 3600


def md_hashes(vault):
    return {p.relative_to(vault).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in Path(vault).rglob("*.md")}


class Phase7GateTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.vault = root / "vault"
        mem = self.vault / "projects" / "demo" / "_memory.md"
        mem.parent.mkdir(parents=True)
        mem.write_text("---\ntype: project-memory\nproject: demo\nagent: claude\nstatus: active\n"
                       "updated: 2026-11-25\n---\n# Demo\n\nState.\n", encoding="utf-8")
        self.idx = hive.Index(self.vault, root / "hive.sqlite")
        self.patches = [mock.patch.object(hive, "VAULT", self.vault), mock.patch.object(hive, "_INDEX", self.idx),
                        mock.patch.object(hive, "LEDGER_PATH", root / "ledger.jsonl")]
        for p in self.patches:
            p.start()

    def tearDown(self):
        for p in self.patches:
            p.stop()
        self.idx.db.close()
        self.tmp.cleanup()

    def test_agent_b_sees_agent_a_dead_end_and_open_thread(self):
        now = int(time.time())
        before = md_hashes(self.vault)
        traces.add(self.idx.db, "demo", "DEAD_END", "src/tokenizer.rs", "patching tokenizer breaks static link",
                   "claude", "session-a", now)
        self.assertEqual(md_hashes(self.vault), before, "trace add must not touch canonical Markdown")
        codex.write(self.vault, self.idx.db, "demo",
                    {"goal": "static link fix", "outcome": "partial: link still fails", "files": ["src/tokenizer.rs"],
                     "dead_ends": ["vendoring tokenizer: license blocks it"], "invariants": [],
                     "open_threads": ["try dynamic tokenizer feature flag"]},
                    "claude", "session-a", "2026-11-25", now)

        t0 = time.perf_counter()
        slice_ = hive.traces_section(self.idx.db, "demo", TASK)
        elapsed_ms = (time.perf_counter() - t0) * 1000
        self.assertLess(elapsed_ms, 10.0, f"traces slice took {elapsed_ms:.1f} ms")

        ctx = hive.tool_context("demo", TASK)  # agent B, fresh call
        self.assertIn("DEAD_END src/tokenizer.rs", ctx)
        self.assertIn("patching tokenizer breaks static link", ctx)
        self.assertIn("open thread: try dynamic tokenizer feature flag", ctx)
        self.assertIn("open thread", slice_)

        later = [t["kind"] for t, _ in traces.active(self.idx.db, "demo", now + 72 * H)]
        self.assertNotIn("DEAD_END", later, "DEAD_END (24 h half-life) must fade below 0.15 after 72 h")


if __name__ == "__main__":
    unittest.main()

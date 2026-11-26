import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

os.environ["HIVE_FORCE_PYTHON"] = "1"
import hive
import skill
import ledger


class HiveSkillMcpTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.ledger_file = self.root / "ledger.jsonl"
        hive.LEDGER_PATH = self.ledger_file

    def tearDown(self):
        self.tmp.cleanup()

    def test_argument_validation(self):
        # 1. Missing required argument 'task'
        r1 = hive.handle({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                          "params": {"name": "hive_skill", "arguments": {}}})
        self.assertTrue(r1["result"]["isError"])
        self.assertIn("missing required argument 'task'", r1["result"]["content"][0]["text"])

        # 2. Unexpected argument
        r2 = hive.handle({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                          "params": {"name": "hive_skill", "arguments": {"task": "fix code", "invalid_param": 123}}})
        self.assertTrue(r2["result"]["isError"])
        self.assertIn("unexpected argument 'invalid_param'", r2["result"]["content"][0]["text"])

        # 3. Wrong type for argument
        r3 = hive.handle({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                          "params": {"name": "hive_skill", "arguments": {"task": 12345}}})
        self.assertTrue(r3["result"]["isError"])
        self.assertIn("must be string", r3["result"]["content"][0]["text"])

    def test_local_skill_match_and_ledger(self):
        # Create a mock local skill
        skill_dir = self.root / "skills" / "k8s-debugger"
        skill_dir.mkdir(parents=True, exist_ok=True)
        (skill_dir / "SKILL.md").write_text(
            "---\nname: k8s-debugger\ndescription: Debug Kubernetes pods and CrashLoopBackOff\ntriggers:\n  - crashloop\n---\n# K8s Debugger\nRun kubectl logs first.\n",
            encoding="utf-8"
        )

        with patch("skill.discover_skill_roots", return_value=[(self.root / "skills", "custom")]):
            req = {
                "jsonrpc": "2.0",
                "id": 10,
                "method": "tools/call",
                "params": {
                    "name": "hive_skill",
                    "arguments": {"task": "pod is stuck in crashloop"}
                }
            }
            resp = hive.handle(req)
            self.assertFalse(resp["result"]["isError"])
            content = resp["result"]["content"][0]["text"]
            self.assertIn("Active Agent Skill: k8s-debugger", content)
            self.assertIn("Run kubectl logs first", content)

        # Verify ledger event
        entries = ledger.read_ledger(self.ledger_file)
        self.assertTrue(any(e["event"] == "skill" and e["project"] == "k8s-debugger" for e in entries))

    def test_remote_skill_auto_install(self):
        candidate = skill.RemoteSkillCandidate(
            package_name="@community/redis-perf",
            description="Redis performance auditing and memory profiling",
            author="redis-dev",
            stars=120,
            downloads=5000,
            source_url="https://github.com/community/redis-perf",
        )

        installed_dir = self.root / "installed" / "redis-perf"
        installed_dir.mkdir(parents=True, exist_ok=True)
        (installed_dir / "SKILL.md").write_text(
            "---\nname: redis-perf\ndescription: Redis performance auditing\n---\n# Redis Performance\nRun info memory.\n",
            encoding="utf-8"
        )

        with patch("skill.discover_skill_roots", return_value=[]), \
             patch("skill.find_remote_skills", return_value=[candidate]), \
             patch("skill.install_skill", return_value={"status": "installed", "installed_path": str(installed_dir)}):
            req = {
                "jsonrpc": "2.0",
                "id": 11,
                "method": "tools/call",
                "params": {
                    "name": "hive_skill",
                    "arguments": {"task": "diagnose redis memory leak", "auto_install": True}
                }
            }
            resp = hive.handle(req)
            self.assertFalse(resp["result"]["isError"])
            content = resp["result"]["content"][0]["text"]
            self.assertIn("Automatically installed skill `@community/redis-perf`", content)
            self.assertIn("Active Agent Skill: redis-perf", content)

    def test_remote_skill_recommendations_when_auto_install_false(self):
        candidate = skill.RemoteSkillCandidate(
            package_name="@community/redis-perf",
            description="Redis performance auditing",
            author="redis-dev",
            stars=120,
            downloads=5000,
            source_url="https://github.com/community/redis-perf",
        )

        with patch("skill.discover_skill_roots", return_value=[]), \
             patch("skill.find_remote_skills", return_value=[candidate]):
            req = {
                "jsonrpc": "2.0",
                "id": 12,
                "method": "tools/call",
                "params": {
                    "name": "hive_skill",
                    "arguments": {"task": "redis cache optimization", "auto_install": False}
                }
            }
            resp = hive.handle(req)
            self.assertFalse(resp["result"]["isError"])
            content = resp["result"]["content"][0]["text"]
            self.assertIn("Top remote candidates", content)
            self.assertIn("@community/redis-perf", content)
            self.assertIn("npx skills add @community/redis-perf", content)


if __name__ == "__main__":
    unittest.main()

import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import ace


def ops(*items):
    return list(items)


class CuratorTest(unittest.TestCase):
    def setUp(self):
        self.empty = {name: [] for name in ace.SECTIONS}

    def test_add_assigns_ids_and_tag_updates_counters(self):
        s, r = ace.apply_ops(self.empty, ops({"type": "ADD", "section": "pitfalls", "content": "Stray Python dir inflates graphs"}))
        self.assertEqual(r["added"], ["pit-00001"])
        s, r = ace.apply_ops(s, ops({"type": "TAG", "id": "pit-00001", "tag": "helpful"},
                                    {"type": "ADD", "section": "commands", "content": "Use python -I script.py"}))
        self.assertEqual(s["pitfalls"][0]["helpful"], 1)
        self.assertEqual(r["added"], ["cmd-00002"])

    def test_apply_is_pure(self):
        before = {name: [] for name in ace.SECTIONS}
        ace.apply_ops(before, ops({"type": "ADD", "section": "pitfalls", "content": "x y z words"}))
        self.assertEqual(before["pitfalls"], [])

    def test_near_duplicate_reinforces_instead_of_adding(self):
        s, _ = ace.apply_ops(self.empty, ops({"type": "ADD", "section": "pitfalls",
                                              "content": "graphify indexes the untracked Python runtime folder"}))
        s, r = ace.apply_ops(s, ops({"type": "ADD", "section": "pitfalls",
                                     "content": "graphify indexes the untracked Python runtime folder too"}))
        self.assertEqual(r["added"], [])
        self.assertEqual(r["reinforced"], ["pit-00001"])
        self.assertEqual(s["pitfalls"][0]["helpful"], 1)

    def test_rejects_secrets_bad_sections_long_and_floods(self):
        _, r = ace.apply_ops(self.empty, ops(
            {"type": "ADD", "section": "pitfalls", "content": "api_key = sk-abcdefghijklmnopqrstuvwxyz123"},
            {"type": "ADD", "section": "nope", "content": "fine text"},
            {"type": "ADD", "section": "pitfalls", "content": "x" * 400},
            *[{"type": "ADD", "section": "strategies", "content": w} for w in (
                "cache graph loads", "pin package versions", "run tests before commit", "prefer stdlib modules",
                "read ignore files", "measure recall first", "keep vault notes small", "check freshness by manifest")]))
        self.assertEqual(len(r["added"]), ace.MAX_ADDS_PER_APPLY)
        reasons = " ".join(r["rejected"])
        for word in ("secret", "unknown section", "too long", "add limit"):
            self.assertIn(word, reasons)

    def test_harmful_bullets_are_pruned_and_cap_holds(self):
        s, _ = ace.apply_ops(self.empty, ops({"type": "ADD", "section": "pitfalls", "content": "bad advice here"}))
        s, r = ace.apply_ops(s, ops(*[{"type": "TAG", "id": "pit-00001", "tag": "harmful"}] * 3))
        self.assertEqual(r["pruned"], ["pit-00001"])
        big = {name: [] for name in ace.SECTIONS}
        big["strategies"] = [{"id": f"str-{i:05d}", "helpful": i % 5, "harmful": 0, "content": f"lesson {i}"}
                             for i in range(1, ace.MAX_BULLETS + 6)]
        s, r = ace.apply_ops(big, [])
        self.assertEqual(sum(len(v) for v in s.values()), ace.MAX_BULLETS)

    def test_render_parse_round_trip(self):
        s, _ = ace.apply_ops(self.empty, ops({"type": "ADD", "section": "conventions", "content": "Keep exports out of the vault"}))
        text = ace.render("demo", s, "2026-11-20")
        self.assertIn("updated: 2026-11-20", text)
        self.assertEqual(ace.parse(text), s)


class ScopeAndHookTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.patches = [mock.patch.object(ace, "VAULT", root / "vault"), mock.patch.object(ace, "PROJECTS", root / "projects"),
                        mock.patch.object(ace, "STATE_PATH", root / "state.json")]
        for p in self.patches:
            p.start()
        (root / "projects" / "My App").mkdir(parents=True)
        self.root = root

    def tearDown(self):
        for p in self.patches:
            p.stop()
        self.tmp.cleanup()

    def test_scope_paths_and_traversal_guard(self):
        self.assertEqual(ace.scope_for_cwd(self.root / "projects" / "My App"), "my-app")
        self.assertEqual(ace.scope_for_cwd(self.root), "global")
        self.assertTrue(str(ace.scope_path("global")).endswith(os.path.join("_meta", "playbook.md")))
        with self.assertRaises(ValueError):
            ace.scope_path("../etc")

    def run_hook(self, fn, payload):
        out = io.StringIO()
        with mock.patch.object(sys, "stdin", io.StringIO(json.dumps(payload))), mock.patch.object(sys, "stdout", out):
            fn()
        return out.getvalue()

    def test_start_hook_injects_and_stop_hook_fires_once_per_threshold(self):
        s, _ = ace.apply_ops({n: [] for n in ace.SECTIONS}, [{"type": "ADD", "section": "pitfalls", "content": "check ignore files"}])
        ace.save("global", s)
        out = json.loads(self.run_hook(ace.hook_start, {"cwd": str(self.root)}))
        self.assertIn("pit-00001", out["hookSpecificOutput"]["additionalContext"])
        transcript = self.root / "t.jsonl"
        lines = [json.dumps({"message": {"content": [{"type": "tool_use", "name": "Edit" if i == 0 else "Read"}]}})
                 for i in range(ace.STOP_TOOL_THRESHOLD)]
        transcript.write_text("\n".join(lines), encoding="utf-8")
        payload = {"session_id": "s1", "transcript_path": str(transcript), "cwd": str(self.root)}
        self.assertEqual(json.loads(self.run_hook(ace.hook_stop, payload))["decision"], "block")
        self.assertEqual(self.run_hook(ace.hook_stop, payload), "")  # same count: no second ask
        self.assertEqual(self.run_hook(ace.hook_stop, dict(payload, session_id="s2", stop_hook_active=True)), "")


if __name__ == "__main__":
    unittest.main()

import tempfile
import unittest
from pathlib import Path

import lint

FM = "---\ntype: {type}\nproject: demo\nagent: claude\nstatus: {status}\nupdated: {updated}\n---\n"


def note(root, rel, body="# t\n", type="note", status="active", updated="2026-11-20", fm=True, described=True):
    path = Path(root) / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    head = FM.format(type=type, status=status, updated=updated) if fm else ""
    if described and rel.endswith("/_memory.md"):
        body = "## What it is\nA demo.\n" + body
    path.write_text(head + body, encoding="utf-8")


def codes(findings, path=None):
    return sorted(f.code for f in findings if path is None or f.path == path)


class LintTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def run_lint(self):
        return lint.lint(self.root, today="2026-11-23")

    def test_clean_vault_has_no_findings(self):
        note(self.root, "projects/demo/_memory.md", "# m\nsee [[0001-x]]\n", type="project-memory")
        note(self.root, "projects/demo/adrs/0001-x.md", "# x\n", type="adr")
        self.assertEqual(codes(self.run_lint()), [])

    def test_missing_frontmatter_and_bad_values(self):
        note(self.root, "a.md", fm=False)
        note(self.root, "b.md", type="essay", status="done", updated="yesterday")
        self.assertIn("frontmatter-missing", codes(self.run_lint(), "a.md"))
        self.assertEqual(codes(self.run_lint(), "b.md"), ["bad-date", "bad-value", "bad-value"])

    def test_root_pointer_files_are_exempt(self):
        note(self.root, "AGENTS.md", fm=False)
        note(self.root, "CLAUDE.md", "@AGENTS.md", fm=False)
        self.assertEqual(codes(self.run_lint()), [])

    def test_utf8_bom_does_not_hide_frontmatter(self):
        path = note(self.root, "projects/demo/bom.md")
        path.write_text("\ufeff" + path.read_text(encoding="utf-8"), encoding="utf-8")
        self.assertNotIn("frontmatter-missing", codes(self.run_lint(), "projects/demo/bom.md"))

    def test_secret_detection_uses_ace_pattern_and_is_error(self):
        path = self.root / "projects/demo/secret.md"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(FM.format(type="note", status="active", updated="2026-11-24")
                        + "token = sk-ant-abcdefghijklmnopqrstuvwxyz1234567890\n", encoding="utf-8")
        findings = [f for f in self.run_lint() if f.path == "projects/demo/secret.md"]
        secret = next(f for f in findings if f.code == "secret-detected")
        self.assertEqual(secret.severity, "ERROR")

    def test_source_missing_is_an_error_for_new_agent_notes(self):
        note(self.root, "inbox/new.md", updated="2026-11-24")
        finding = next(f for f in self.run_lint() if f.code == "source-missing")
        self.assertEqual(finding.severity, "ERROR")

    def test_lint_can_limit_findings_to_selected_paths(self):
        note(self.root, "projects/demo/good.md")
        path = self.root / "projects/demo/unselected.md"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(FM.format(type="note", status="active", updated="2026-11-24")
                        + "token = sk-ant-abcdefghijklmnopqrstuvwxyz1234567890\n", encoding="utf-8")
        findings = lint.lint(self.root, today="2026-11-23", only_paths={"projects/demo/good.md"})
        self.assertFalse(any(f.path == "projects/demo/unselected.md" for f in findings))

    def test_line_caps(self):
        note(self.root, "projects/demo/_memory.md", "x\n" * 151, type="project-memory")
        note(self.root, "_meta/active-work.md", "x\n" * 41)
        self.assertEqual(codes(self.run_lint()), ["line-cap", "line-cap"])

    def test_broken_link_ignores_code_aliases_headings_and_attachments(self):
        (self.root / "_attachments").mkdir()
        (self.root / "_attachments" / "pic.png").write_bytes(b"")
        note(self.root, "target.md")
        note(self.root, "src.md", r"| [[target\|alias]] | " "[[target|alias]] [[target#h]] [[pic.png|300]] `[[inline]]`\n"
                                  "```\n[[fenced]]\n```\n[[missing]]\n")
        found = [f for f in self.run_lint() if f.code == "broken-link"]
        self.assertEqual([f.message for f in found], ["[[missing]] resolves to no note or file"])

    def test_stale_active_project_memory(self):
        note(self.root, "projects/demo/_memory.md", type="project-memory", updated="2026-09-01")
        self.assertEqual(codes(self.run_lint()), ["stale"])

    def test_superseded_adr_needs_link_to_successor(self):
        note(self.root, "projects/demo/adrs/0001-a.md", "# a\nreplaced\n", type="adr", status="superseded")
        note(self.root, "projects/demo/adrs/0002-b.md", "# b\n", type="adr")
        note(self.root, "projects/demo/_memory.md", "[[0001-a]] [[0002-b]]", type="project-memory")
        self.assertEqual(codes(self.run_lint()), ["superseded-unlinked"])

    def test_duplicate_basename_except_conventional_names(self):
        note(self.root, "projects/a/plan.md")
        note(self.root, "projects/b/plan.md")
        note(self.root, "projects/a/_memory.md", "[[plan]]", type="project-memory")
        note(self.root, "projects/b/_memory.md", "[[plan]]", type="project-memory")
        self.assertEqual(codes(self.run_lint()), ["duplicate-name", "duplicate-name"])

    def test_orphan_adr_is_reported_but_journal_is_not(self):
        note(self.root, "projects/demo/adrs/0001-a.md", type="adr")
        note(self.root, "journal/2026-11-20.md", type="journal")
        self.assertEqual(codes(self.run_lint()), ["orphan"])

    def test_journal_headers_follow_the_entry_format(self):
        note(self.root, "journal/2026-11-23.md", "# d\n\n## 09:15 claude - ai-hive\nok\n\n"
             "## 10:00 codex - offmesh | fix\nok\n\n## random heading\n", type="journal")
        found = [f for f in self.run_lint() if f.code == "journal-header"]
        self.assertEqual([f.message for f in found],
                         ["'## random heading' is not '## HH:MM <agent> - <project> [| <kind>]'"])

    def test_source_required_on_new_agent_notes_only(self):
        note(self.root, "inbox/new.md", updated="2026-11-23")
        note(self.root, "inbox/old.md", updated="2026-11-20")
        note(self.root, "inbox/sourced.md", "---\n", updated="2026-11-23")
        (self.root / "inbox/sourced.md").write_text(
            FM.format(type="note", status="active", updated="2026-11-23").replace("---\n", "---\nsource: commit abc\n", 1),
            encoding="utf-8")
        note(self.root, "projects/demo/playbook.md", updated="2026-11-23")  # written by ace.py, not a claim
        self.assertEqual([f.path for f in self.run_lint() if f.code == "source-missing"], ["inbox/new.md"])

    def test_project_memory_must_say_what_the_project_is(self):
        # 2026-11-23: a _memory.md without this section scored 0 for "personal website" and lost search
        note(self.root, "projects/a/_memory.md", "# a\n## Status\nok\n", type="project-memory", described=False)
        note(self.root, "projects/b/_memory.md", "# b\n## What it is\nA site.\n", type="project-memory", described=False)
        self.assertEqual([f.path for f in self.run_lint() if f.code == "memory-undescribed"], ["projects/a/_memory.md"])

    def test_runs_in_isolated_mode_like_agents_and_hooks(self):
        # agents launch `python -I hive.py serve`; -I drops the script dir from sys.path (bug found 2026-11-23)
        import json, os, subprocess, sys
        note(self.root, "a.md")
        here = Path(lint.__file__).resolve().parent
        env = {**os.environ, "HIVE_VAULT": str(self.root), "HIVE_DB": str(self.root / "h.sqlite")}
        cli = subprocess.run([sys.executable, "-I", str(here / "lint.py"), str(self.root)],
                             capture_output=True, text=True, timeout=60, env=env)
        self.assertEqual(cli.returncode, 0, cli.stderr)
        call = {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "hive_lint", "arguments": {}}}
        mcp = subprocess.run([sys.executable, "-I", str(here / "hive.py"), "serve"], input=json.dumps(call) + "\n",
                             capture_output=True, text=True, timeout=60, env=env)
        result = json.loads(mcp.stdout)["result"]
        self.assertFalse(result["isError"], result)
        self.assertIn("lint: 0 error", result["content"][0]["text"])

    def test_format_sorts_by_severity(self):
        note(self.root, "a.md", fm=False)
        note(self.root, "projects/demo/adrs/0001-a.md", type="adr")
        out = lint.format_report(self.run_lint())
        self.assertTrue(out.splitlines()[0].startswith("lint:"))
        self.assertLess(out.index("ERROR"), out.index("INFO"))


if __name__ == "__main__":
    unittest.main()

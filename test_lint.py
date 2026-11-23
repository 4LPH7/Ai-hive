import tempfile
import unittest
from pathlib import Path

import lint

FM = "---\ntype: {type}\nproject: demo\nagent: claude\nstatus: {status}\nupdated: {updated}\n---\n"


def note(root, rel, body="# t\n", type="note", status="active", updated="2026-11-20", fm=True):
    path = Path(root) / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    head = FM.format(type=type, status=status, updated=updated) if fm else ""
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
        self.assertEqual([f.path for f in self.run_lint() if f.code == "source-missing"], ["inbox/new.md"])

    def test_format_sorts_by_severity(self):
        note(self.root, "a.md", fm=False)
        note(self.root, "projects/demo/adrs/0001-a.md", type="adr")
        out = lint.format_report(self.run_lint())
        self.assertTrue(out.splitlines()[0].startswith("lint:"))
        self.assertLess(out.index("ERROR"), out.index("INFO"))


if __name__ == "__main__":
    unittest.main()

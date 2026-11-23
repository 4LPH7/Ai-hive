import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import lint
import triage

FM = "---\ntype: {type}\nproject: {project}\nagent: claude\nstatus: active\nupdated: 2026-11-20\n---\n"


def note(root, rel, body, type="note", project="demo"):
    path = Path(root) / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(FM.format(type=type, project=project) + body, encoding="utf-8")


class TriageTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name) / "vault"
        state = Path(self.tmp.name) / "state"
        self.patches = [mock.patch.object(triage, "STATE", state),
                        mock.patch.object(triage, "LABELS", state / "labels.jsonl"),
                        mock.patch.object(triage, "PROPOSALS", state / "proposals.json")]
        for p in self.patches:
            p.start()
        note(self.root, "projects/demo/_memory.md", "# demo - project memory\n> Canonical state.\n"
             "## What it is\nA demo app that sells widgets.\n", type="project-memory")
        note(self.root, "projects/demo/adrs/0001-db.md", "# 0001 - SQLite\nChose SQLite.\n", type="adr")
        note(self.root, "projects/_template/_memory.md", "# template\n", type="project-memory")
        note(self.root, "journal/2026-11-20.md", "# d\n\n## 09:00 claude - demo | fix\nfixed it\n\n"
             "## 10:00 codex - all (setup)\nmachine\n", type="journal", project="all")
        note(self.root, "_meta/playbook.md", "# Playbook\n\n## pitfalls\n\n"
             "- [pit-00001] helpful=1 harmful=0 :: Run tests before commit.\n", project="all")
        note(self.root, "inbox/idea.md", "# Idea\nmaybe use sqlite-vec\n", project="demo")

    def tearDown(self):
        for p in self.patches:
            p.stop()
        self.tmp.cleanup()

    def test_projects_use_first_prose_line_and_skip_templates(self):
        self.assertEqual(triage.projects(self.root), {"demo": "A demo app that sells widgets."})

    def test_questions_offer_projects_plus_global(self):
        qs = triage.questions(self.root)
        self.assertEqual(sorted(qs["project"]["criteria"]), ["demo", "global"])
        self.assertIn("decision", qs["kind"]["criteria"])

    def test_vault_examples_cover_every_labelled_source_but_not_inbox(self):
        labels = [answers for _, answers in triage.vault_examples(self.root)]
        self.assertEqual(sorted(json.dumps(a, sort_keys=True) for a in labels), sorted(json.dumps(a, sort_keys=True) for a in [
            {"project": "demo", "kind": "status"},      # _memory.md
            {"project": "demo", "kind": "decision"},    # ADR
            {"project": "demo", "kind": "status"},      # journal entry for demo
            {"project": "global", "kind": "status"},    # journal entry for all
            {"project": "global", "kind": "lesson"},    # global playbook bullet
        ]))
        texts = [text for text, _ in triage.vault_examples(self.root)]
        self.assertFalse(any("sqlite-vec" in t for t in texts))
        self.assertIn("fixed it", texts)  # journal entry body without its header, which names the project

    def test_inbox_items_skip_the_report(self):
        note(self.root, triage.REPORT, "# report\n")
        self.assertEqual(triage.inbox_items(self.root), ["inbox/idea.md"])

    def test_confirm_accepts_proposal_or_override_and_validates(self):
        triage.STATE.mkdir(parents=True)
        triage.PROPOSALS.write_text(json.dumps({"inbox/idea.md": {"project": "demo", "kind": "research"}}))
        triage.confirm("inbox/idea.md", "kind=decision", vault=self.root)
        row = json.loads(triage.LABELS.read_text(encoding="utf-8"))
        self.assertEqual(row["answers"], {"project": "demo", "kind": "decision"})
        self.assertIn("sqlite-vec", row["text"])
        with self.assertRaises(ValueError):
            triage.confirm("inbox/idea.md", "project=nope", vault=self.root)
        with self.assertRaises(ValueError):
            triage.confirm("inbox/other.md", vault=self.root)
        self.assertEqual(len(triage.confirmed_labels(triage.questions(self.root))), 1)

    def test_summarize_routes_review_when_any_question_is_unsure(self):
        qs = triage.questions(self.root)
        # shape of vegaml Learner.decide() in laya 0.5.3
        sure = {"answer": "demo", "confidence": 0.9, "prediction_set": ["demo"], "abstain": False}
        torn = {"answer": "noise", "confidence": 0.4, "prediction_set": ["noise", "status"], "abstain": False}
        s = triage.summarize({"answers": {"project": sure, "kind": torn}, "latency_ms": 400}, qs)
        self.assertEqual((s["project"]["answer"], s["kind"]["p"], s["route"]), ("demo", 0.4, "review"))
        s = triage.summarize({"answers": {"project": sure, "kind": {**torn, "prediction_set": ["noise"]}}}, qs)
        self.assertEqual(s["route"], "auto")
        s = triage.summarize({"answers": {"project": {**sure, "abstain": True}, "kind": sure}}, qs)
        self.assertEqual(s["route"], "review")

    def test_bundle_key_changes_only_with_the_questions(self):
        first = triage.bundle_path(triage.questions(self.root))
        note(self.root, "journal/2026-11-21.md", "# d\n\n## 09:00 claude - demo\nmore\n", type="journal")
        self.assertEqual(triage.bundle_path(triage.questions(self.root)), first)
        note(self.root, "projects/other/_memory.md", "# other\nAnother app.\n", type="project-memory")
        self.assertNotEqual(triage.bundle_path(triage.questions(self.root)), first)

    def test_report_is_a_lint_clean_proposal_note(self):
        rows = [{"note": "inbox/idea.md", "project": {"answer": "demo", "p": 0.91},
                 "kind": {"answer": "research", "p": 0.55}, "route": "review"}]
        (self.root / triage.REPORT).write_text(triage.render_report(rows, "2026-11-23 10:00"), encoding="utf-8")
        report = (self.root / triage.REPORT).read_text(encoding="utf-8")
        self.assertIn("| [[idea]] | demo (91%) | research (55%) | review |", report)
        bad = [f for f in lint.lint(self.root, today="2026-11-23") if f.path == triage.REPORT]
        self.assertEqual(bad, [])


if __name__ == "__main__":
    unittest.main()

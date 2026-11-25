import unittest
import ace

V1_0_0_PLAYBOOK = """---
type: note
project: demo
agent: claude
status: active
updated: 2026-11-20
---
# Playbook: demo

> ACE playbook.

## strategies

- [str-00001] helpful=5 harmful=1 :: Always run tests before committing code
- [str-00002] helpful=2 harmful=0 :: Use stdlib modules when possible

## pitfalls

- [pit-00001] helpful=0 harmful=2 :: Avoid global mutable state in worker processes

## commands

## conventions
"""

class PlaybookMigrationTest(unittest.TestCase):
    def test_legacy_format_migrates_seen_counter(self):
        sections = ace.parse(V1_0_0_PLAYBOOK)
        str1 = sections["strategies"][0]
        self.assertEqual(str1["id"], "str-00001")
        self.assertEqual(str1["helpful"], 5)
        self.assertEqual(str1["harmful"], 1)
        # G4 migration: seen defaults to helpful + harmful
        self.assertEqual(str1["seen"], 6)

        str2 = sections["strategies"][1]
        self.assertEqual(str2["seen"], 2)

        pit1 = sections["pitfalls"][0]
        self.assertEqual(pit1["seen"], 2)

    def test_migrated_playbook_renders_with_seen_counter(self):
        sections = ace.parse(V1_0_0_PLAYBOOK)
        rendered = ace.render("demo", sections, "2026-11-24")
        self.assertIn("- [str-00001] seen=6 helpful=5 harmful=1 :: Always run tests before committing code", rendered)
        self.assertIn("- [pit-00001] seen=2 helpful=0 harmful=2 :: Avoid global mutable state in worker processes", rendered)

        # Re-parse rendered text and verify roundtrip
        reparsed = ace.parse(rendered)
        self.assertEqual(reparsed["strategies"][0]["seen"], 6)
        self.assertEqual(reparsed["strategies"][0]["helpful"], 5)
        self.assertEqual(reparsed["strategies"][0]["harmful"], 1)

if __name__ == "__main__":
    unittest.main()

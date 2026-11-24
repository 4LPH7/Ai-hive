import unittest
import hive

SAMPLE_NOTE = """---
type: project-memory
project: alpha
agent: claude
source: session-2026-11-24
status: active
updated: 2026-11-24
---
# Alpha Architecture

## What it is
Alpha is a high-performance memory cache.

## Core Rules
- Do not commit secrets
- Maintain stdlib compatibility
"""

def frame_note(meta, title, body, rel="projects/alpha/_memory.md"):
    agent = meta.get("agent", "unknown")
    source = meta.get("source", "unknown")
    updated = meta.get("updated", "")
    age = hive.note_age(updated)
    header = f'> [!NOTE] Untrusted vault note: path="{rel}", agent="{agent}", source="{source}", age="{age}"'
    quoted = [f"> {line}" for line in body.strip().splitlines()]
    return header + "\n" + "\n".join(quoted)

def strip_untrusted_framing(framed_text):
    lines = framed_text.splitlines()
    body_lines = []
    for line in lines:
        if line.startswith("> [!NOTE] Untrusted vault note:"):
            continue
        if line.startswith("> "):
            body_lines.append(line[2:])
        elif line == ">":
            body_lines.append("")
        else:
            body_lines.append(line)
    return "\n".join(body_lines)

class UntrustedFramingTest(unittest.TestCase):
    def test_round_trip_fidelity(self):
        meta, title, body = hive.parse_note(SAMPLE_NOTE)
        framed = frame_note(meta, title, body)

        # Asserts structural markdown blockquote formatting (G6)
        self.assertIn('> [!NOTE] Untrusted vault note: path="projects/alpha/_memory.md"', framed)
        self.assertIn("> ## What it is", framed)
        self.assertIn("> Alpha is a high-performance memory cache.", framed)

        # Strip framing and assert exact body recovery
        recovered_body = strip_untrusted_framing(framed)
        self.assertEqual(recovered_body.strip(), body.strip())

        # Reconstructed note parsing gives identical metadata and title
        reconstructed = f"---\n" + "\n".join(f"{k}: {v}" for k, v in meta.items()) + f"\n---\n\n{recovered_body}\n"
        meta2, title2, body2 = hive.parse_note(reconstructed)
        self.assertEqual(meta, meta2)
        self.assertEqual(title, title2)
        self.assertEqual(body.strip(), body2.strip())

if __name__ == "__main__":
    unittest.main()

---
name: antislop-code
description: Enforce high-signal code comment hygiene, strip obvious narration, and eliminate generic AI boilerplate.
triggers:
  - antislop
  - anti-slop
  - clean comments
  - strip comments
  - comment hygiene
  - refactor style
  - code slop
---

# Anti-Slop Code Hygiene

Apply to all source code before landing commits:

## Rules of Engagement
1. **Never explain standard syntax**: Remove comments that state what code clearly expresses (e.g. `// Iterate through list`, `# Initialize variables`, `// Return result`).
2. **Explain the WHY, never the WHAT**: Keep comments only when they explain non-obvious business domain constraints, edge cases, or hardware/OS quirks.
3. **No conversational filler**: Zero polite remarks, introductory essays, or sign-offs in code or commit messages.
4. **Terse, Dense, High-Signal**: Use telegraphic phrases, bullet points, or exact technical specifications.
5. **Never touch working logic during slop cleaning**: Restrict modifications strictly to comments and whitespace.

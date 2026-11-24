---
name: systematic-debugging
description: 4-phase root-cause debugging protocol before attempting code edits.
triggers:
  - debug
  - debugging
  - fix bug
  - investigate crash
  - test failure
  - failing test
  - failing unit test
  - broken
  - error trace
  - reproduce issue
---

# Systematic Debugging Protocol

When encountering any bug, test failure, crash, or unexpected behavior:

## Phase 1: Reproduction & Empirical Observation
- Do NOT guess or edit code immediately.
- Run the failing command or write a minimal reproduction test fixture.
- Capture the exact error, trace, and returncode.

## Phase 2: Root-Cause Isolation
- Trace dataflow backwards from the failure site to input origin.
- Formulate a testable hypothesis for WHY the defect occurs.
- Verify assumptions by inspecting state (logs, print statements, or debugger).

## Phase 3: Minimal Surgical Fix
- Implement the simplest change that resolves the root cause.
- Follow SOLUTION EFFICIENCY: skip -> reuse -> stdlib -> native platform -> minimum code.
- Never add speculative complexity or unnecessary wrappers.

## Phase 4: Regression Verification & Lock
- Run the full test suite to guarantee zero regressions.
- Ensure the newly authored reproduction test fixture passes consistently.

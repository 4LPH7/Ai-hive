---
name: code-review
description: Comprehensive pre-merge review protocol auditing correctness, security, edge cases, and code hygiene.
triggers:
  - code review
  - review code
  - review pr
  - review diff
  - audit changes
  - pre-commit review
  - check bugs
---

# Code Review Protocol

Execute before declaring any task complete or submitting a branch/commit:

## 1. Correctness & Logic
- Check boundary conditions (empty collections, 0/1 limits, off-by-one errors).
- Verify error handling and fail-open/fail-closed semantics.
- Confirm state consistency across concurrency boundaries.

## 2. Security & Traversal Guards
- Path safety: Ensure all file paths canonicalized (`Path.resolve()` + `os.path.normcase`).
- Injection: Verify zero unescaped command interpolation, shell injection, or SQL formatting.
- Secret leaks: Confirm zero API keys, tokens, or credentials in diffs or test logs.

## 3. Style & Slop Hygiene
- Strip redundant AI commentary ("Loop over items", "Return result").
- Ensure error messages are structured, informative, and actionable.
- Verify zero dead code, orphaned imports, or debug prints left in production files.

## 4. Test Proof
- Verify every changed branch has an automated regression test.
- Run complete test suite (`cargo test`, `python -m unittest discover`).

---
name: test-driven-development
description: Test-driven development workflow ensuring deterministic regression locks before writing implementation code.
triggers:
  - tdd
  - write tests
  - add tests
  - unit test
  - integration test
  - test first
  - test coverage
---

# Test-Driven Development (TDD) Protocol

Follow this contract for all bug fixes, features, and refactors:

## 1. Red Phase: Author Failing Specification
- Write a concrete test asserting desired behavior BEFORE writing implementation code.
- Run the test and verify it FAILS for the expected reason (assert failure or missing symbol).
- Ensure the failure is clear, unambiguous, and reproducible.

## 2. Green Phase: Minimum Code Implementation
- Write the minimum amount of code necessary to make the failing test pass.
- Resist the urge to add anticipatory features or over-engineer abstractions.
- Run the test and confirm it passes cleanly.

## 3. Refactor Phase: Clean & Harden
- Eliminate code duplication, improve naming, and strip redundant AI comments.
- Verify tests remain 100% green without changing test assertions.
- Verify performance constraints (latency, memory footprint, startup time).

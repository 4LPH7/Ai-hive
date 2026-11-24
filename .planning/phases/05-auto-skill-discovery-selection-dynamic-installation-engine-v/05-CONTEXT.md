# Phase 5: Auto-Skill Discovery, Selection & Dynamic Installation Engine (v2.1) - Context

**Locked:** 2026-11-24
**Goal:** Automatically select the best installed skill for any agent task, and dynamically discover, install, and inject missing skills from remote registries (like `find-skills` / `npx skills`).

---

## 1. Locked Decisions

### D1. Cross-Agent Local Skill Indexer (`SKILL-01`)
- Scans installed skill directories across all major coding agent environments:
  - Antigravity / Gemini: `~/.gemini/config/skills`, `~/.gemini/antigravity/builtin/skills`, `~/.gemini/config/plugins/*/skills`
  - OpenAI Codex: `~/.codex/skills`, `.codex/skills`
  - Claude Code: `~/.claude/skills`, `.claude/skills`
  - Project-local: `.agent/skills`, `.skills`
- Parses `SKILL.md` YAML frontmatter (`name`, `description`, `triggers`, `argumentSchema`) and header sections.
- Stores in-memory/FTS index of skill capabilities, triggers, and semantic descriptors.

### D2. Intent Matcher & Confidence Thresholding (`SKILL-02`)
- Scores the incoming agent task / query against the skill catalog using BM25 token matching with trigger boosting.
- Confidence score normalized `[0.0, 1.0]`.
- Decision boundary:
  - `score >= 0.65`: High-confidence local match -> immediately select and load skill (`SKILL-RESOLVE`).
  - `score < 0.65`: Local miss -> trigger remote registry search (`SKILL-FIND`).

### D3. Remote Registry Search / `find-skills` Integration (`SKILL-03`)
- Integrates with the Skills CLI ecosystem (`npx skills find <query>`), Agent Marketplace catalog, and open GitHub skill repositories.
- Extracts candidate packages with install counts, star ratings, descriptions, and maintainer reputation.
- Filters out candidates with unsafe script execution or blacklisted patterns.

### D4. Dynamic Auto-Installation (`SKILL-04`)
- When `auto_install` is true (or policy is `auto`), automatically installs the top-ranked remote skill:
  - Via `npx skills add <package>` for npm-hosted skills.
  - Or directly downloading the validated `SKILL.md` (and associated scripts) into `.agent/skills/<name>/`.
- Ensures atomic installation with rollback if validation fails.

### D5. Dynamic Context & Instruction Injection (`SKILL-05`)
- Once resolved (locally or post-installation), reads the full `SKILL.md` content.
- Formats the skill instructions, required parameters, and execution hints for the agent.
- Emits structured response instructing the calling agent to follow the loaded skill workflow.

### D6. Dual-Core MCP & CLI Tool Interface (`SKILL-06`)
- Exposes `hive_skill` MCP tool in both Python (`hive.py`) and Rust (`src/mcp.rs` / `hive.exe`):
  ```json
  {
    "name": "hive_skill",
    "description": "Auto-select best skill for task, or discover and install from remote registry (find-skills).",
    "parameters": {
      "task": "Task description or intent",
      "auto_install": true,
      "scope": "local | all"
    }
  }
  ```
- Exposes CLI command: `hive skill "task description" [--install] [--dry-run]`.

### D7. ACE Playbook Reinforcement Loop (`SKILL-07`)
- Records skill invocation events (task, selected skill, local vs remote, outcome) into local session telemetry.
- ACE curator reinforces bullets recommending specific skills for task patterns that succeeded.

---

## 2. Requirements Matrix

| ID | Requirement | Status | Verification |
|---|---|---|---|
| SKILL-01 | Cross-agent skill indexer | Pending | Scans Antigravity, Codex, Claude Code, project roots |
| SKILL-02 | Intent matcher with confidence threshold | Pending | Ranks local skills; returns best match + score |
| SKILL-03 | Remote registry search (find-skills) | Pending | Queries `npx skills find` or marketplace on local miss |
| SKILL-04 | Dynamic auto-installation | Pending | Safely downloads/installs package to local skill folder |
| SKILL-05 | Context & instruction injection | Pending | Emits parsed `SKILL.md` instructions into agent context |
| SKILL-06 | MCP tool & CLI parity | Pending | `hive_skill` tool in Python and Rust + `hive skill` CLI |
| SKILL-07 | ACE reinforcement loop | Pending | Records skill outcomes; boosts winning skill associations |

---

## 3. Plan Decomposition (3 Waves)

- **Plan 05-01 (Wave 1: Local Indexer & Intent Matcher)**:
  - Implement skill directory discovery across Antigravity, Codex, Claude, and `.agent/skills`.
  - Implement `SKILL.md` frontmatter parser (`name`, `description`, triggers).
  - Implement BM25 / token similarity matcher with confidence scoring in Python and Rust.
  - Tests: unit tests for skill discovery, parsing, and ranking.

- **Plan 05-02 (Wave 2: Remote Discovery & Dynamic Installer)**:
  - Implement remote skill search client (`npx skills find` / registry fallback).
  - Implement auto-installer (`npx skills add` / raw `SKILL.md` fetch) with safety validation.
  - Implement skill context loader (reads `SKILL.md`, formats execution instructions).
  - Tests: integration test for simulated remote search and local staging.

- **Plan 05-03 (Wave 3: MCP Server Tool, CLI & ACE Feedback)**:
  - Implement `hive_skill` tool in `hive.py` and `src/mcp.rs`.
  - Implement `hive skill` CLI command in Python and Rust binary.
  - Wire ACE reflection telemetry to track skill efficacy on tasks.
  - Verification: end-to-end test suite (`test_hive_skill.py` & Rust integration test).

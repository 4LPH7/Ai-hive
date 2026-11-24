---
last_mapped_commit: 3262a0c60dadc1e9078e9af568cbe4ed8b190dcb
last_mapped_at: 2026-11-24
---
# Code Conventions & Engineering Standards

## Python Coding Standards

- **Target**: Python 3.10+ compatibility.
- **Dependencies**: 0 external packages. No `pip install` required.
- **Type Annotations**: Standard library typing hints (`typing.Optional`, `typing.List`, `typing.Dict`, `typing.Tuple`, etc.).
- **Line Endings**: LF (`\n`) across all files (including test fixtures and shell scripts).
- **Encoding**: UTF-8 without BOM; read inputs using `utf-8-sig` to safely handle Windows-generated UTF-8 BOM files.

## Error Handling & Defensive Boundaries

- **Fail-Open / Non-Crashing Servers**:
  - The MCP server loop must catch tool errors gracefully and return structured JSON-RPC responses (`isError: True`) rather than crashing the stdio process.
- **Path Traversal Prevention**:
  - All project slugs must match `^[a-z0-9._-]+$`.
  - All vault paths must be verified with `resolved_path.is_relative_to(VAULT.resolve())`.
  - Repository paths must be canonical direct subdirectories of configured project roots.
- **Argument Validation**:
  - Strict input validation against defined JSON schemas before invoking tool logic.

## Atomic Mutex & File Safety

- **Atomic Writes**:
  - Writes must write to `path.with_suffix(path.suffix + ".tmp")` followed by `os.replace`.
- **Secret Sanitization**:
  - Regular expressions for API keys (`sk-ant-`, `ghp_`, `xoxb-`, etc.) run as pre-commit errors.

## Git & Versioning

- **Commit Messages**: Conventional commits (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`).
- **Branch Strategy**: Fast-forward merges on `main`.

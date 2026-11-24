---
last_mapped_commit: 3262a0c60dadc1e9078e9af568cbe4ed8b190dcb
last_mapped_at: 2026-11-24
---
# Technology Stack

## Core Runtime & Language

- **Language**: Python 3.10+ (strictly pure stdlib)
- **Zero Third-Party Dependencies**: No `pip install` required for runtime or core testing
- **Platform Support**: Cross-platform (Windows, Linux, macOS)

## Standard Library Modules Leveraged

- **Database**: `sqlite3` (in-memory & file-backed FTS5 search index)
- **Filesystem & IO**: `pathlib.Path`, `os`, `sys`, `shutil`, `tempfile`
- **Serialization & Formats**: `json`, `re` (regex-based frontmatter/markdown parsing)
- **Math & Algorithms**: `math` (BM25 & PPR score calculations), `collections` (`defaultdict`, `deque`, `Counter`)
- **Process & Protocols**: `io`, `typing`, `time`, `datetime`, `subprocess`
- **Testing**: `unittest`

## CI/CD Pipeline

- **Runner**: GitHub Actions (`.github/workflows/tests.yml`)
- **Matrix**:
  - OS: `ubuntu-latest`, `windows-latest`
  - Python: `3.10`, `3.12`
- **Verification Commands**:
  - `python -W error::SyntaxWarning -m unittest discover -v`
  - `python hive.py eval` (golden retrieval set)
  - `python hive.py bench` (graph & query benchmark)

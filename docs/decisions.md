# Design decisions

The hive started from a much larger draft: a daemon over SQLite with sqlite-vec embeddings, rustworkx graphs,
seven memory layers, symbol leases and an HTTP port. Most of it was deliberately not built. Each row below
records what was chosen, what was rejected, and the evidence.

| Decision | Rejected alternative | Why |
|----------|---------------------|-----|
| Markdown vault is the only truth; every index is disposable | Dual-write to a database | Dual writes drift. A disposable index rebuilds from disk with no migration |
| Stdlib-only Python, stdio per agent, no daemon, no port | FastMCP daemon on localhost HTTP | Nothing on the network can write into memory; nothing to keep running; installs anywhere Python does |
| Read-only MCP server; agents write Markdown files | `hive_commit` write tool with optimistic locking | Agents already write files well, and git reviews them. One write path, not two |
| BM25 (SQLite FTS5) with type, status and project priors | Embeddings (sqlite-vec, fastembed) | BM25 is a strong baseline most agent-memory systems fail to beat (GroupMemBench, arXiv 2605.14498). On this vault it reaches Recall@5 0.97 already; embeddings would add a heavy dependency for little gain at under a few thousand notes |
| Personalized PageRank only for code structure, with hub dampening | PPR on every query, or raw N-hop traversal | Graph diffusion helps mainly when hubs are dampened (GAAMA, arXiv 2603.27910); it loses to plain BM25 on single-fact lookups (GraphRAG-Bench, arXiv 2506.05690) |
| graphify `graph.json` read directly | Re-importing the code graph into the hive's tables | graphify already does tree-sitter parsing, resolution, Leiden communities and incremental updates. A second copy is a second sync problem |
| Freshness from `manifest.json` mtimes | Comparing `built_at_commit` with `HEAD` | graphify leaves the graph untouched when topology is unchanged, so the commit lags even when the graph is current |
| `layer`/`type` as frontmatter, one store | Seven separate memory stores | Seven write paths are seven failure modes |
| Keep the existing vault layout | A new numbered layout (`01-Global/`, `02-Projects/`, ...) | Migrating every note would break links and dashboards for no retrieval gain |
| ACE deltas merged by a deterministic program | Letting an LLM rewrite the playbook | Full rewrites collapse into generic summaries (ACE, arXiv 2510.04618) |
| Stop hook asks the live agent to reflect | Background `claude -p` reflector | Headless sign-in was not available; the live agent also has the full session evidence |
| Deterministic lint before any LLM gardener | An LLM that "cleans up" the vault | Rule checks are free, exact and explainable; they catch most rot (caps, links, frontmatter) |
| `source:` required on new agent notes | Trusting agent-written claims | Self-written memory compounds errors; traceable claims can be checked and superseded |
| No ML model for inbox triage; you triage `inbox/` | [Laya](https://huggingface.co/convaiinnovations/laya) decision model proposing project and kind (tried and removed 2026-11-23) | On real inbox notes it was wrong on both at under 30% confidence while its uncalibrated gate said `auto`; it needed 30-40+ confirmations, a 4 GB torch venv and 15-minute CPU refits to sort a handful of notes. Not worth it at this vault size |
| Personal eval set kept out of the repo | Shipping the author's golden questions | They describe private projects; `examples/eval_golden.example.json` shows the format |
| Standalone Rust binary as primary core with Python fallback (ADR 0004) | Pure Python only or requiring heavy C-extensions/runtimes | Sub-millisecond cold start (<1 ms), bundled SQLite FTS5 with zero external dependencies, 2-3x faster graph PageRank, while maintaining pure Python stdlib parity |
| Supply-chain security audit gate for skills | Installing remote skills blindly | Third-party SKILL.md is executable prompt context; blocks Trojan Source (CVE-2021-42574) bidirectional Unicode, prompt injections, and shell exfiltration before writing to disk |
| Deterministic outcome scoring & gap mining (ADR 0005) | Unsupervised LLM self-editing of playbooks | Correlates test exit codes and git diffs against sessions; rewards or penalizes only injected bullets; clusters zero-hit queries into human-reviewed proposal notes in `inbox/` |
| OmniRoute tiered inference with inbox proposals (ADR 0006) | In-process LLM engines or uncontrolled cloud routing | Decouples model inference from storage; hard-locks private projects to localhost `hive-private`; scrubs secrets before egress; writes proposals exclusively to `inbox/` without touching canonical notes |

## Not built yet, and when it would be worth it

| Idea | Build it when |
|------|---------------|
| Embedding search | Recall@5 on your golden set drops below about 0.9, or the vault passes a few thousand notes |
| Symbol leases between agents | Two agents regularly edit the same repository at the same time |

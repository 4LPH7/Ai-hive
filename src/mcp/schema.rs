//! MCP tool descriptors (`tools/list`) and protocol constants.

use serde_json::{json, Value};

pub const SERVER_NAME: &str = "hive";
pub const SERVER_VERSION: &str = "1.2.0";
pub const MODERN_VERSIONS: &[&str] = &["2026-07-28"];
pub const LEGACY_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
pub const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
pub const INSTRUCTIONS: &str =
    "Shared memory for all coding agents (Obsidian vault + graphify graphs). Read-only. \
    Call hive_context at the start of a task, hive_search before asking the user for context, \
    hive_graph for code structure. Write memory as Markdown files following the vault's AGENTS.md.";

pub fn tool_list() -> Value {
    let read_only = json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false
    });
    let detail_prop = json!({"type": "string", "enum": ["concise", "detailed"],
        "description": "concise (default): ids, file:line, one line each; detailed: full"});

    let tools = vec![
        json!({
            "name": "hive_context",
            "description": "Call first on any task. Token-budgeted memory pack for a project: its active-work row, _memory.md, ADR list and, if a task is given, matching vault notes and ranked code-graph symbols.",
            "inputSchema": {
                "type": "object",
                "required": ["project"],
                "properties": {
                    "project": {"type": "string", "description": "Project slug: folder name under projects root, lowercased, spaces to hyphens"},
                    "task": {"type": "string", "description": "What you are about to do (optional, improves ranking)"},
                    "budget": {"type": "integer", "description": "Max tokens to return (default 1500)"},
                    "handoff": {"type": "string", "description": "Handoff id from `hive handoff write`; returns only that envelope, budgeted by the writer's role"}
                },
                "additionalProperties": false
            },
            "annotations": read_only,
            "_meta": {"anthropic/alwaysLoad": true}
        }),
        json!({
            "name": "hive_search",
            "description": "BM25 search over vault notes. Default scope is project-isolated ('project'); use scope='all' for cross-project.",
            "inputSchema": {
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "description": "Keywords"},
                    "project": {"type": "string", "description": "Project slug to filter/boost (optional)"},
                    "scope": {"type": "string", "description": "Search scope: 'project' (default, project + global) or 'all' (cross-project)"},
                    "limit": {"type": "integer", "description": "Max hits (default 8)"},
                    "detail": detail_prop
                },
                "additionalProperties": false
            },
            "annotations": read_only
        }),
        json!({
            "name": "hive_graph",
            "description": "Ranked code-graph neighbourhood from graphify (Personalized PageRank with hub dampening): symbols with file:line, edges between them, related vault notes, and graph staleness.",
            "inputSchema": {
                "type": "object",
                "required": ["repo", "query"],
                "properties": {
                    "repo": {"type": "string", "description": "Repo folder name or slug under projects root, or a path"},
                    "query": {"type": "string", "description": "Symbol names or keywords"},
                    "limit": {"type": "integer", "description": "Max symbols (default 15)"},
                    "detail": detail_prop
                },
                "additionalProperties": false
            },
            "annotations": read_only
        }),
        json!({
            "name": "hive_status",
            "description": "Index size per project and freshness of each repo's code graph.",
            "inputSchema": {
                "type": "object",
                "required": [],
                "properties": {},
                "additionalProperties": false
            },
            "annotations": read_only
        }),
        json!({
            "name": "hive_lint",
            "description": "Vault health against AGENTS.md rules: frontmatter, line caps, broken links, stale memory, superseded ADRs without successor, ambiguous names, orphans. Run before session end after writing notes.",
            "inputSchema": {
                "type": "object",
                "required": [],
                "properties": {"detail": detail_prop},
                "additionalProperties": false
            },
            "annotations": read_only
        }),
        json!({
            "name": "hive_skill",
            "description": "Auto-select best skill for task, or discover and install from remote registry (find-skills).",
            "inputSchema": {
                "type": "object",
                "required": ["task"],
                "properties": {
                    "task": {"type": "string", "description": "Task description or user prompt (required)"},
                    "auto_install": {"type": "boolean", "description": "Whether to auto-install missing skills from registry (default: true)"},
                    "scope": {"type": "string", "description": "Search scope: 'local' (installed only) or 'all' (local + remote fallback, default: 'all')"}
                },
                "additionalProperties": false
            },
            "annotations": read_only
        }),
    ];

    json!(tools)
}

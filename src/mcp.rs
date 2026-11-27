use crate::common::{default_db_path, default_projects};
use crate::index::Index;
use crate::lint::{format_report, lint};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

mod schema;
mod tools;
mod validate;
pub use schema::*;
pub use tools::*;
pub use validate::*;

pub fn handle_rpc(idx: &mut Index, req: &Value) -> Option<Value> {
    let id = req.get("id");
    let method = req.get("method")?.as_str()?;

    // PROV-01: the client names itself in `initialize` (legacy) or per request in `_meta` (2026-07-28).
    if let Some(name) = req
        .pointer("/params/clientInfo/name")
        .or_else(|| req.pointer("/params/_meta/io.modelcontextprotocol~1clientInfo/name"))
        .and_then(|v| v.as_str())
    {
        crate::ledger::set_context(crate::ledger::agent_from_hint(name).as_deref(), None);
    }

    match method {
        "initialize" => {
            let client_ver = req
                .pointer("/params/protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2024-11-05");

            let supported_ver =
                if MODERN_VERSIONS.contains(&client_ver) || LEGACY_VERSIONS.contains(&client_ver) {
                    client_ver
                } else {
                    "2024-11-05"
                };

            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": supported_ver,
                    "serverInfo": {
                        "name": SERVER_NAME,
                        "version": SERVER_VERSION
                    },
                    "instructions": INSTRUCTIONS,
                    "capabilities": {
                        "tools": {
                            "listChanged": false
                        }
                    }
                }
            }))
        }
        "notifications/initialized" => None,
        "ping" => Some(json!({"jsonrpc": "2.0", "id": id, "result": {}})),
        "tools/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": tool_list()
            }
        })),
        "tools/call" => {
            let tool_name = req
                .pointer("/params/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let empty_args = json!({});
            let tool_args = req.pointer("/params/arguments").unwrap_or(&empty_args);

            if let Err(err) = validate_tool_args(tool_name, tool_args) {
                return Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "isError": true,
                        "content": [{
                            "type": "text",
                            "text": format!("invalid arguments for {}: {}", tool_name, err)
                        }]
                    }
                }));
            }

            let call_res: Result<String, String> = match tool_name {
                "hive_context" => {
                    let project = tool_args
                        .get("project")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let task = tool_args.get("task").and_then(|v| v.as_str()).unwrap_or("");
                    let budget = tool_args
                        .get("budget")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(1500) as usize;
                    let handoff = tool_args
                        .get("handoff")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    tool_context(idx, project, task, budget, handoff)
                }
                "hive_search" => {
                    let query = tool_args
                        .get("query")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let project = tool_args
                        .get("project")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let scope = tool_args.get("scope").and_then(|v| v.as_str()).unwrap_or({
                        if !project.is_empty() {
                            "project"
                        } else {
                            "all"
                        }
                    });
                    let limit =
                        tool_args.get("limit").and_then(|v| v.as_u64()).unwrap_or(8) as usize;
                    match idx.search(query, project, limit, scope) {
                        Ok(hits) => {
                            if hits.is_empty() {
                                Ok("no matching notes".to_string())
                            } else if is_concise(tool_args) {
                                let lines: Vec<String> = hits
                                    .into_iter()
                                    .map(|h| {
                                        let title: String = h.title.chars().take(80).collect();
                                        format!("- {} ({}) {}", h.path, h.r#type, title)
                                    })
                                    .collect();
                                Ok(lines.join("\n"))
                            } else {
                                let lines: Vec<String> = hits
                                    .into_iter()
                                    .map(|h| {
                                        format!(
                                            "- {} ({}, {}, {}) {}\n  {}",
                                            h.path,
                                            h.r#type,
                                            h.status,
                                            h.updated,
                                            h.title,
                                            h.snippet
                                        )
                                    })
                                    .collect();
                                Ok(lines.join("\n"))
                            }
                        }
                        Err(e) => Err(e),
                    }
                }
                "hive_graph" => {
                    let repo = tool_args.get("repo").and_then(|v| v.as_str()).unwrap_or("");
                    let query = tool_args
                        .get("query")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let limit = tool_args
                        .get("limit")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(15) as usize;
                    tool_graph(idx, repo, query, limit, is_concise(tool_args))
                }
                "hive_status" => {
                    let _ = idx.refresh(true);
                    let mut lines = Vec::new();
                    let total: i64 = idx
                        .conn
                        .query_row("SELECT count(*) FROM notes", [], |r| r.get(0))
                        .unwrap_or(0);
                    lines.push(format!(
                        "vault {}: {} notes indexed ({})",
                        idx.vault.display(),
                        total,
                        default_db_path().display()
                    ));
                    Ok(lines.join("\n"))
                }
                "hive_lint" => {
                    let findings = lint(&idx.vault, None);
                    if is_concise(tool_args) {
                        Ok(crate::lint::format_concise(&findings))
                    } else {
                        Ok(format_report(&findings))
                    }
                }
                "hive_skill" => {
                    let task = tool_args.get("task").and_then(|v| v.as_str()).unwrap_or("");
                    let auto_install = tool_args
                        .get("auto_install")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    let scope = tool_args
                        .get("scope")
                        .and_then(|v| v.as_str())
                        .unwrap_or("all");
                    tool_skill(task, auto_install, scope)
                }
                other => Err(format!("tool {:?} not found", other)),
            };

            // DETAIL-02: large results go to the disposable artifact cache; errors and hive_context never do
            let call_res = call_res.map(|text| {
                if tool_name == "hive_context" {
                    text
                } else {
                    crate::artifact::offload(&text, &[&idx.vault, &default_projects()])
                }
            });
            match call_res {
                Ok(content) => Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{
                            "type": "text",
                            "text": content
                        }],
                        "isError": false
                    }
                })),
                Err(err_msg) => Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "isError": true,
                        "content": [{
                            "type": "text",
                            "text": err_msg
                        }]
                    }
                })),
            }
        }
        _ => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("Method not found: {}", method)
            }
        })),
    }
}

pub fn serve(vault: Option<PathBuf>, db_path: Option<PathBuf>) -> Result<(), String> {
    let mut idx = Index::new(vault, db_path)?;
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line_res in stdin.lock().lines() {
        let line = match line_res {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Ok(req) = serde_json::from_str::<Value>(trimmed) {
            if let Some(resp) = handle_rpc(&mut idx, &req) {
                let resp_str = serde_json::to_string(&resp).unwrap_or_default();
                writeln!(stdout, "{}", resp_str).map_err(|e| e.to_string())?;
                stdout.flush().map_err(|e| e.to_string())?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_and_tool_list() {
        let mut idx = Index::in_memory().unwrap();
        let init_req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"protocolVersion": "2026-07-28"}
        });
        let resp = handle_rpc(&mut idx, &init_req).unwrap();
        assert_eq!(resp["result"]["protocolVersion"], "2026-07-28");
        assert_eq!(resp["result"]["serverInfo"]["name"], "hive");

        let list_req = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        });
        let list_resp = handle_rpc(&mut idx, &list_req).unwrap();
        let tools = list_resp["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "hive_context"));
        assert!(tools.iter().any(|t| t["name"] == "hive_search"));
        assert!(tools.iter().any(|t| t["name"] == "hive_graph"));
    }

    #[test]
    fn test_initialize_client_info_sets_ledger_agent() {
        let mut idx = Index::in_memory().unwrap();
        let init_req = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-11-25", "clientInfo": {"name": "claude-code", "version": "2"}}
        });
        handle_rpc(&mut idx, &init_req).unwrap();
        // only this test sets process context, so the result is deterministic under parallel tests
        assert_eq!(crate::ledger::resolve_agent(None), "claude");
    }

    #[test]
    fn test_argument_validation() {
        assert!(validate_tool_args("hive_context", &json!({"project": "ai-hive"})).is_ok());
        assert!(validate_tool_args("hive_context", &json!({})).is_err());
        assert!(
            validate_tool_args("hive_context", &json!({"project": "ai-hive", "extra": 123}))
                .is_err()
        );
        assert!(validate_tool_args(
            "hive_context",
            &json!({"project": "ai-hive", "handoff": "2026-11-25-00000000"})
        )
        .is_ok());
        assert!(
            validate_tool_args("hive_context", &json!({"project": "ai-hive", "handoff": 1}))
                .is_err()
        );
        assert!(
            validate_tool_args("hive_context", &json!({"project": "ai-hive", "budget": -1}))
                .is_err()
        );
    }

    #[test]
    fn test_detail_validation_and_schema() {
        for (name, base) in [
            ("hive_search", json!({"query": "q"})),
            ("hive_graph", json!({"repo": "r", "query": "q"})),
            ("hive_lint", json!({})),
        ] {
            for (detail, ok) in [
                (json!("concise"), true),
                (json!("detailed"), true),
                (json!("full"), false),
                (json!(1), false),
            ] {
                let mut args = base.clone();
                args["detail"] = detail.clone();
                assert_eq!(
                    validate_tool_args(name, &args).is_ok(),
                    ok,
                    "{name} {detail}"
                );
            }
        }
        assert!(validate_tool_args("hive_lint", &json!({"other": 1})).is_err());
        assert!(validate_tool_args("hive_status", &json!({"detail": "concise"})).is_err());
        assert!(is_concise(&json!({})) && is_concise(&json!({"detail": "concise"})));
        assert!(!is_concise(&json!({"detail": "detailed"})));
        let tools = tool_list();
        let tools = tools.as_array().unwrap();
        assert_eq!(tools.len(), 6);
        for t in tools {
            let props = &t["inputSchema"]["properties"];
            let has = matches!(
                t["name"].as_str(),
                Some("hive_search" | "hive_graph" | "hive_lint")
            );
            assert_eq!(props.get("detail").is_some(), has, "{}", t["name"]);
            if has {
                assert_eq!(props["detail"]["enum"], json!(["concise", "detailed"]));
            }
        }
        assert!(tools[0]["inputSchema"]["properties"]["handoff"].is_object());
    }

    /// MCP-01/02 (Phase 9 gate): at most 6 tools and a fixed schema token budget (measured at v2.7.2 + 10%).
    /// Raising SCHEMA_TOKEN_CAP is a deliberate, reviewed change. Mirror of tests/python/test_schema_budget.py.
    #[test]
    fn test_schema_budget() {
        const MAX_TOOLS: usize = 6;
        const SCHEMA_TOKEN_CAP: usize = 1140;
        let tools = tool_list();
        assert!(tools.as_array().unwrap().len() <= MAX_TOOLS);
        let size = crate::common::est_tokens(&serde_json::to_string(&tools).unwrap());
        println!("MCP schema ~{size} est tokens (cap {SCHEMA_TOKEN_CAP})");
        assert!(
            size <= SCHEMA_TOKEN_CAP,
            "MCP tool schema is ~{size} est tokens (cap {SCHEMA_TOKEN_CAP}); trim descriptions first"
        );
    }
}

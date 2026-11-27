//! Argument validation for MCP tool calls.

use serde_json::Value;

/// DETAIL-01: MCP responses of hive_search/graph/lint default to concise; tool_* and the CLI stay detailed.
pub(crate) fn check_detail(v: &Value) -> Result<(), String> {
    match v.as_str() {
        Some("concise" | "detailed") => Ok(()),
        _ => Err(format!(
            "argument 'detail' must be one of [\"concise\", \"detailed\"], got {v}"
        )),
    }
}

/// `detail` from MCP args; concise unless the caller asks for detailed.
pub(crate) fn is_concise(args: &Value) -> bool {
    args.get("detail").and_then(|v| v.as_str()) != Some("detailed")
}

pub fn validate_tool_args(name: &str, args: &Value) -> Result<(), String> {
    let obj = args.as_object().ok_or_else(|| {
        format!(
            "arguments must be a JSON object, got {}",
            if args.is_null() { "null" } else { "non-object" }
        )
    })?;

    match name {
        "hive_context" => {
            if !obj.contains_key("project") || obj["project"].is_null() {
                return Err("missing required argument 'project'".to_string());
            }
            for (k, v) in obj {
                match k.as_str() {
                    "project" => {
                        if !v.is_string() {
                            return Err("argument 'project' must be string".to_string());
                        }
                    }
                    "task" | "handoff" => {
                        if !v.is_string() {
                            return Err(format!("argument '{k}' must be string"));
                        }
                    }
                    "budget" => {
                        if !v.is_i64() && !v.is_u64() {
                            return Err("argument 'budget' must be integer".to_string());
                        }
                        if v.as_i64().is_some_and(|b| b < 0) {
                            return Err(format!("argument 'budget' must be >= 0, got {v}"));
                        }
                    }
                    other => {
                        return Err(format!(
                            "unexpected argument {:?}; allowed: [\"budget\", \"handoff\", \"project\", \"task\"]",
                            other
                        ));
                    }
                }
            }
        }
        "hive_search" => {
            if !obj.contains_key("query") || obj["query"].is_null() {
                return Err("missing required argument 'query'".to_string());
            }
            for (k, v) in obj {
                match k.as_str() {
                    "query" => {
                        if !v.is_string() {
                            return Err("argument 'query' must be string".to_string());
                        }
                    }
                    "project" => {
                        if !v.is_string() {
                            return Err("argument 'project' must be string".to_string());
                        }
                    }
                    "scope" => {
                        if !v.is_string() {
                            return Err("argument 'scope' must be string".to_string());
                        }
                    }
                    "limit" => {
                        if !v.is_i64() && !v.is_u64() {
                            return Err("argument 'limit' must be integer".to_string());
                        }
                    }
                    "detail" => check_detail(v)?,
                    other => {
                        return Err(format!(
                            "unexpected argument {:?}; allowed: [\"detail\", \"limit\", \"project\", \"query\", \"scope\"]",
                            other
                        ));
                    }
                }
            }
        }
        "hive_graph" => {
            if !obj.contains_key("repo") || obj["repo"].is_null() {
                return Err("missing required argument 'repo'".to_string());
            }
            if !obj.contains_key("query") || obj["query"].is_null() {
                return Err("missing required argument 'query'".to_string());
            }
            for (k, v) in obj {
                match k.as_str() {
                    "repo" => {
                        if !v.is_string() {
                            return Err("argument 'repo' must be string".to_string());
                        }
                    }
                    "query" => {
                        if !v.is_string() {
                            return Err("argument 'query' must be string".to_string());
                        }
                    }
                    "limit" => {
                        if !v.is_i64() && !v.is_u64() {
                            return Err("argument 'limit' must be integer".to_string());
                        }
                    }
                    "detail" => check_detail(v)?,
                    other => {
                        return Err(format!(
                            "unexpected argument {:?}; allowed: [\"detail\", \"limit\", \"query\", \"repo\"]",
                            other
                        ));
                    }
                }
            }
        }
        "hive_status" => {
            if !obj.is_empty() {
                let first_key = obj.keys().next().unwrap();
                return Err(format!("unexpected argument {:?}; allowed: []", first_key));
            }
        }
        "hive_lint" => {
            for (k, v) in obj {
                match k.as_str() {
                    "detail" => check_detail(v)?,
                    other => {
                        return Err(format!(
                            "unexpected argument {:?}; allowed: [\"detail\"]",
                            other
                        ))
                    }
                }
            }
        }
        "hive_skill" => {
            if !obj.contains_key("task") || obj["task"].is_null() {
                return Err("missing required argument 'task'".to_string());
            }
            for (k, v) in obj {
                match k.as_str() {
                    "task" => {
                        if !v.is_string() {
                            return Err("argument 'task' must be string".to_string());
                        }
                    }
                    "auto_install" => {
                        if !v.is_boolean() {
                            return Err("argument 'auto_install' must be boolean".to_string());
                        }
                    }
                    "scope" => {
                        if !v.is_string() {
                            return Err("argument 'scope' must be string".to_string());
                        }
                    }
                    other => {
                        return Err(format!(
                            "unexpected argument {:?}; allowed: [\"auto_install\", \"scope\", \"task\"]",
                            other
                        ));
                    }
                }
            }
        }
        other => return Err(format!("unknown tool {:?}", other)),
    }

    Ok(())
}

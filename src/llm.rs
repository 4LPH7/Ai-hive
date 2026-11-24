use crate::ace::scrub_secrets;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmCombos {
    pub private: String,
    pub fast: String,
    pub deep: String,
}

impl Default for LlmCombos {
    fn default() -> Self {
        Self {
            private: "hive-private".to_string(),
            fast: "hive-fast".to_string(),
            deep: "hive-deep".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub enabled: bool,
    pub base_url: String,
    pub api_key: Option<String>,
    pub timeout_ms: u64,
    pub default_privacy: String,
    pub combos: LlmCombos,
    pub project_privacies: HashMap<String, String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            base_url: "http://127.0.0.1:20128/v1".to_string(),
            api_key: None,
            timeout_ms: 30000,
            default_privacy: "private".to_string(),
            combos: LlmCombos::default(),
            project_privacies: HashMap::new(),
        }
    }
}

pub fn load_llm_config(config_path: Option<&Path>) -> LlmConfig {
    let mut cfg = LlmConfig::default();
    let default_cfg = crate::common::find_default_config_path();
    let effective_path = config_path.or(default_cfg.as_deref());
    let toml_map = if let Some(p) = effective_path {
        if p.is_file() {
            std::fs::read_to_string(p)
                .map(|c| crate::common::parse_toml_config(&c))
                .unwrap_or_default()
        } else {
            HashMap::new()
        }
    } else {
        HashMap::new()
    };

    if let Ok(v) = std::env::var("HIVE_LLM_ENABLED") {
        cfg.enabled = v != "0" && v.to_lowercase() != "false";
    } else if let Some(v) = toml_map.get("llm.enabled") {
        cfg.enabled = v != "0" && v.to_lowercase() != "false";
    }

    if let Ok(v) = std::env::var("HIVE_LLM_BASE_URL") {
        cfg.base_url = v;
    } else if let Some(v) = toml_map.get("llm.base_url") {
        cfg.base_url = v.clone();
    }

    let api_key_env_var = toml_map
        .get("llm.api_key_env")
        .cloned()
        .unwrap_or_else(|| "OMNIROUTE_API_KEY".to_string());
    if let Ok(v) = std::env::var(&api_key_env_var) {
        if !v.is_empty() {
            cfg.api_key = Some(v);
        }
    } else if let Ok(v) = std::env::var("HIVE_LLM_API_KEY") {
        if !v.is_empty() {
            cfg.api_key = Some(v);
        }
    }

    if let Ok(v) = std::env::var("HIVE_LLM_TIMEOUT_MS") {
        if let Ok(ms) = v.parse::<u64>() {
            cfg.timeout_ms = ms;
        }
    } else if let Some(v) = toml_map.get("llm.timeout_ms").and_then(|s| s.parse::<u64>().ok()) {
        cfg.timeout_ms = v;
    }

    if let Ok(v) = std::env::var("HIVE_LLM_DEFAULT_PRIVACY") {
        cfg.default_privacy = v.to_lowercase();
    } else if let Some(v) = toml_map.get("llm.default_privacy") {
        cfg.default_privacy = v.to_lowercase();
    }

    if let Some(v) = toml_map.get("llm.combos.private") {
        cfg.combos.private = v.clone();
    }
    if let Some(v) = toml_map.get("llm.combos.fast") {
        cfg.combos.fast = v.clone();
    }
    if let Some(v) = toml_map.get("llm.combos.deep") {
        cfg.combos.deep = v.clone();
    }

    for (k, v) in &toml_map {
        if let Some(rest) = k.strip_prefix("projects.") {
            if let Some((slug, field)) = rest.split_once('.') {
                if field == "llm_privacy" {
                    cfg.project_privacies.insert(slug.to_string(), v.to_lowercase());
                }
            }
        }
    }

    cfg
}

pub fn resolve_combo(cfg: &LlmConfig, tier: &str, project: Option<&str>) -> String {
    let proj_privacy = project.and_then(|p| cfg.project_privacies.get(p)).map(|s| s.as_str());
    let is_private = match proj_privacy {
        Some("private") => true,
        Some("public") | Some("shared") => false,
        _ => cfg.default_privacy == "private",
    };

    if is_private {
        // Hard-lock: private workloads NEVER use cloud fallbacks
        cfg.combos.private.clone()
    } else {
        match tier.to_lowercase().as_str() {
            "fast" => cfg.combos.fast.clone(),
            "deep" => cfg.combos.deep.clone(),
            _ => cfg.combos.private.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<serde_json::Value>,
}

pub fn chat_completion(
    cfg: &LlmConfig,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    schema: Option<&serde_json::Value>,
) -> Result<String, String> {
    if !cfg.enabled {
        return Err("LLM disabled by configuration".to_string());
    }

    // Guardrail 1: Mandatory pre-egress secret scrubbing
    let scrubbed_system = scrub_secrets(system_prompt);
    let scrubbed_user = scrub_secrets(user_prompt);

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: scrubbed_system,
        },
        ChatMessage {
            role: "user".to_string(),
            content: scrubbed_user,
        },
    ];

    let response_format = schema.map(|s| {
        serde_json::json!({
            "type": "json_schema",
            "json_schema": {
                "name": "hive_curate_output",
                "schema": s
            }
        })
    });

    let req_body = ChatCompletionRequest {
        model: model.to_string(),
        messages,
        temperature: 0.1,
        stream: false,
        response_format,
    };

    let body_json = serde_json::to_string(&req_body).map_err(|e| format!("Serialization error: {}", e))?;
    let endpoint = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));

    let mut req = ureq::post(&endpoint)
        .timeout(Duration::from_millis(cfg.timeout_ms))
        .set("Content-Type", "application/json");

    if let Some(key) = &cfg.api_key {
        req = req.set("Authorization", &format!("Bearer {}", key));
    }

    let resp = req.send_string(&body_json).map_err(|e| format!("OmniRoute HTTP error: {}", e))?;

    let resp_text = resp.into_string().map_err(|e| format!("Failed to read OmniRoute response: {}", e))?;
    let resp_json: serde_json::Value = serde_json::from_str(&resp_text).map_err(|e| format!("Invalid JSON from OmniRoute: {}", e))?;

    if let Some(err) = resp_json.get("error") {
        return Err(format!("OmniRoute API error: {}", err));
    }

    let content = resp_json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.get(0))
        .and_then(|c0| c0.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|ct| ct.as_str())
        .ok_or_else(|| "Missing choices[0].message.content in OmniRoute response".to_string())?;

    Ok(content.to_string())
}

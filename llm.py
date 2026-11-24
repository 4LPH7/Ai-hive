"""OmniRoute Gateway Client: Thin HTTP client routing to local OmniRoute inference proxy.
"""
import json
import os
import urllib.error
import urllib.request
from pathlib import Path

import ace
import hive


def load_llm_config():
    cfg_raw = hive.load_config()
    enabled_val = os.environ.get("HIVE_LLM_ENABLED", cfg_raw.get("llm.enabled", "true"))
    enabled = str(enabled_val).lower() not in ("0", "false", "no")

    base_url = os.environ.get("HIVE_LLM_BASE_URL", cfg_raw.get("llm.base_url", "http://127.0.0.1:20128/v1"))
    api_key_env = cfg_raw.get("llm.api_key_env", "OMNIROUTE_API_KEY")
    api_key = os.environ.get(api_key_env) or os.environ.get("HIVE_LLM_API_KEY")

    try:
        timeout_ms = int(os.environ.get("HIVE_LLM_TIMEOUT_MS", cfg_raw.get("llm.timeout_ms", 30000)))
    except (ValueError, TypeError):
        timeout_ms = 30000

    default_privacy = os.environ.get("HIVE_LLM_DEFAULT_PRIVACY", cfg_raw.get("llm.default_privacy", "private")).lower()

    combos = {
        "private": os.environ.get("HIVE_LLM_COMBO_PRIVATE", cfg_raw.get("llm.combos.private", "hive-private")),
        "fast": os.environ.get("HIVE_LLM_COMBO_FAST", cfg_raw.get("llm.combos.fast", "hive-fast")),
        "deep": os.environ.get("HIVE_LLM_COMBO_DEEP", cfg_raw.get("llm.combos.deep", "hive-deep")),
    }

    # Extract project privacy settings
    project_privacies = {}
    for k, v in cfg_raw.items():
        if k.startswith("projects.") and k.endswith(".llm_privacy"):
            parts = k.split(".")
            if len(parts) >= 3:
                slug = parts[1]
                project_privacies[slug] = str(v).lower()

    return {
        "enabled": enabled,
        "base_url": base_url,
        "api_key": api_key,
        "timeout_ms": timeout_ms,
        "default_privacy": default_privacy,
        "combos": combos,
        "project_privacies": project_privacies,
    }


def resolve_combo(cfg, tier, project=None):
    proj_privacy = cfg.get("project_privacies", {}).get(project) if project else None
    if proj_privacy == "private":
        is_private = True
    elif proj_privacy in ("public", "shared"):
        is_private = False
    else:
        is_private = (cfg.get("default_privacy", "private") == "private")

    if is_private:
        return cfg["combos"]["private"]

    t = str(tier).lower()
    if t == "fast":
        return cfg["combos"]["fast"]
    if t == "deep":
        return cfg["combos"]["deep"]
    return cfg["combos"]["private"]


def chat_completion(cfg, model, system_prompt, user_prompt, schema=None):
    if not cfg.get("enabled", True):
        raise RuntimeError("LLM disabled by configuration")

    # Guardrail 1: Mandatory pre-egress secret scrubbing
    scrubbed_system = ace.scrub_secrets(system_prompt)
    scrubbed_user = ace.scrub_secrets(user_prompt)

    messages = [
        {"role": "system", "content": scrubbed_system},
        {"role": "user", "content": scrubbed_user},
    ]

    payload = {
        "model": model,
        "messages": messages,
        "temperature": 0.1,
        "stream": False,
    }
    if schema:
        payload["response_format"] = {
            "type": "json_schema",
            "json_schema": {
                "name": "hive_curate_output",
                "schema": schema,
            },
        }

    data = json.dumps(payload).encode("utf-8")
    endpoint = f"{cfg['base_url'].rstrip('/')}/chat/completions"

    req = urllib.request.Request(endpoint, data=data, method="POST")
    req.add_header("Content-Type", "application/json")
    if cfg.get("api_key"):
        req.add_header("Authorization", f"Bearer {cfg['api_key']}")

    timeout_sec = cfg.get("timeout_ms", 30000) / 1000.0
    try:
        with urllib.request.urlopen(req, timeout=timeout_sec) as resp:
            resp_body = resp.read().decode("utf-8")
            res_obj = json.loads(resp_body)
            if "error" in res_obj:
                raise RuntimeError(f"OmniRoute API error: {res_obj['error']}")
            return res_obj["choices"][0]["message"]["content"]
    except urllib.error.URLError as e:
        raise ConnectionError(f"OmniRoute connection failed: {e}")
    except (json.JSONDecodeError, KeyError, IndexError) as e:
        raise ValueError(f"Malformed OmniRoute response: {e}")

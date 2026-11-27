//! Frontmatter and body parsing shared by index, policy and codex. A leaf: depends only on regex.

use regex::Regex;
use std::collections::HashMap;

pub fn parse_note(text: &str) -> (HashMap<String, String>, String, String) {
    let clean = if let Some(stripped) = text.strip_prefix('\u{feff}') {
        stripped
    } else {
        text
    };

    let mut meta = HashMap::new();
    let mut body = clean;

    if let Some(after) = clean.strip_prefix("---") {
        if let Some(end) = after.find("\n---") {
            let front = &after[..end];
            for line in front.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    let key = k.trim();
                    if !key.is_empty()
                        && !key.starts_with(' ')
                        && !key.starts_with('\t')
                        && !key.starts_with('-')
                    {
                        let val = v.trim().trim_matches('\'').trim_matches('"');
                        meta.insert(key.to_string(), val.to_string());
                    }
                }
            }
            body = after[end + 4..].trim_start_matches(['-', '\n', '\r']);
        }
    }

    let re_heading = Regex::new(r"(?m)^#\s+(.+)$").unwrap();
    let title = re_heading
        .captures(body)
        .map(|cap| cap[1].trim().to_string())
        .unwrap_or_default();

    (meta, title, body.to_string())
}

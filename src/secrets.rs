//! Secret detection and scrubbing: one pattern, used by every module that writes or sends text (playbook bullets,
//! codices, traces, lint, LLM prompts, artifacts). A leaf module: it depends on nothing else in the crate.

use regex::Regex;
use std::sync::OnceLock;

const PATTERN: &str = r"(sk-[A-Za-z0-9_-]{20,}|ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|AKIA[0-9A-Z]{16}|xox[abprs]-[A-Za-z0-9-]{10,}|-----BEGIN [A-Z ]*PRIVATE KEY|(?:api[_-]?key|token|password|secret)\s*[:=]\s*\S{8,})";

fn re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(&format!("(?i){PATTERN}")).expect("secret pattern is valid"))
}

pub fn is_secret(text: &str) -> bool {
    re().is_match(text)
}

pub fn scrub_secrets(text: &str) -> String {
    re().replace_all(text, "[REDACTED_SECRET]").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detects_and_scrubs() {
        assert!(is_secret("token: abcdefghijklmnop"));
        assert!(is_secret("ghp_123456789012345678901234567890123456"));
        assert!(!is_secret("plain prose about tokens and passwords"));
        assert!(!scrub_secrets("key sk-abcdefghijklmnopqrstu").contains("sk-abc"));
    }
}

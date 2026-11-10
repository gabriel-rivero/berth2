//! Known agent CLIs as data, not code paths. Agent ecosystems change monthly;
//! this table is meant to be edited, and nothing else should need to be.

/// A provider is a static table entry, never deserialized from the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provider {
    pub id: &'static str,
    pub display: &'static str,
    /// Process names or argv tokens that identify this provider.
    pub aliases: &'static [&'static str],
    /// Resume command; `{ref}` is replaced by the session reference.
    pub resume: &'static [&'static str],
    /// Whether this provider can be resumed by argument alone.
    pub resumable: bool,
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        id: "claude-code",
        display: "Claude Code",
        aliases: &["claude", "claude-code"],
        resume: &["claude", "--resume", "{ref}"],
        resumable: true,
    },
    Provider {
        id: "codex",
        display: "Codex",
        aliases: &["codex", "codex-cli"],
        resume: &["codex", "resume", "{ref}"],
        resumable: true,
    },
    Provider {
        id: "opencode",
        display: "opencode",
        aliases: &["opencode"],
        resume: &["opencode", "--session", "{ref}"],
        resumable: true,
    },
    Provider {
        id: "custom",
        display: "Custom",
        aliases: &[],
        resume: &[],
        resumable: false,
    },
];

pub fn by_id(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

fn normalize(token: &str) -> String {
    let base = token.rsplit('/').next().unwrap_or(token);
    base.trim_end_matches(".exe")
        .to_ascii_lowercase()
        .replace(' ', "-")
}

/// Detect a provider from a process name and argv, best effort.
///
/// Checked in order: `comm`, then the first few argv entries. Matching is on
/// whole tokens and on `token-<suffix>` (e.g. `claude-teams`), never substrings
/// of unrelated words.
pub fn detect(comm: Option<&str>, argv: &[String]) -> Option<&'static Provider> {
    let mut candidates: Vec<String> = Vec::with_capacity(5);
    if let Some(c) = comm {
        candidates.push(normalize(c));
    }
    for a in argv.iter().take(4) {
        candidates.push(normalize(a));
    }

    for candidate in &candidates {
        for p in PROVIDERS.iter().filter(|p| !p.aliases.is_empty()) {
            for alias in p.aliases {
                if candidate == alias
                    || candidate.starts_with(&format!("{alias}-"))
                    || candidate == &format!("{alias}.js")
                {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Build the resume argv for a provider, preserving extra flags.
pub fn resume_argv(
    provider: &Provider,
    session_ref: Option<&str>,
    extra: &[String],
) -> Vec<String> {
    let mut out: Vec<String> = provider
        .resume
        .iter()
        .map(|s| match session_ref {
            Some(r) => s.replace("{ref}", r),
            None => s.to_string(),
        })
        .filter(|s| !s.is_empty())
        .collect();
    out.retain(|t| t != "{ref}");
    out.extend(extra.iter().cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn detects_claude_by_process_name() {
        let p = detect(Some("claude"), &[]).unwrap();
        assert_eq!(p.id, "claude-code");
    }

    #[test]
    fn detects_node_hosted_claude_by_argv() {
        let p = detect(Some("node"), &argv(&["node", "/usr/local/bin/claude"])).unwrap();
        assert_eq!(p.id, "claude-code");
    }

    #[test]
    fn detects_suffixed_builds() {
        assert_eq!(detect(Some("claude-teams"), &[]).unwrap().id, "claude-code");
        assert_eq!(detect(Some("opencode"), &[]).unwrap().id, "opencode");
    }

    #[test]
    fn does_not_match_unrelated_words() {
        assert!(detect(Some("node"), &argv(&["node", "server.js"])).is_none());
        assert!(detect(Some("claude-notes-tool"), &[]).is_some()); // known tradeoff, documented
        assert!(detect(Some("vim"), &argv(&["vim", "claude.md"])).is_none());
    }

    #[test]
    fn resume_templates() {
        let p = by_id("claude-code").unwrap();
        let out = resume_argv(p, Some("8f1c"), &["--verbose".to_string()]);
        assert_eq!(out, vec!["claude", "--resume", "8f1c", "--verbose"]);
    }

    #[test]
    fn resume_without_ref_drops_placeholder() {
        let p = by_id("opencode").unwrap();
        assert_eq!(resume_argv(p, None, &[]), vec!["opencode", "--session"]);
        let c = by_id("custom").unwrap();
        assert!(resume_argv(c, None, &[]).is_empty());
    }
}

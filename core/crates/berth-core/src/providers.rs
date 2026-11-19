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
        id: "copilot",
        display: "GitHub Copilot",
        aliases: &["copilot", "copilot-cli"],
        resume: &["copilot", "--resume", "{ref}"],
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

/// Extra arguments from a saved launch command: everything after the program,
/// minus the provider's own resume flag. A resumed launch keeps flags such as
/// `--allow-dangerously-skip-permissions` or `--model`.
pub fn launch_args(provider: &Provider, argv: &[String]) -> Vec<String> {
    let resume_flag = provider
        .resume
        .iter()
        .position(|token| *token == "{ref}")
        .and_then(|index| index.checked_sub(1))
        .map(|index| provider.resume[index]);
    let runtime_hosted = argv
        .first()
        .is_some_and(|first| matches!(normalize(first).as_str(), "node" | "bun" | "deno"));
    let mut out = Vec::new();
    let mut skip_value = false;
    for (index, arg) in argv.iter().enumerate().skip(1) {
        if skip_value {
            skip_value = false;
            continue;
        }
        // A runtime-hosted CLI carries its script path as the second token.
        if index == 1 && (runtime_hosted || is_provider_program(provider, arg)) {
            continue;
        }
        if Some(arg.as_str()) == resume_flag {
            skip_value = true;
            continue;
        }
        out.push(arg.clone());
    }
    out
}

fn is_provider_program(provider: &Provider, token: &str) -> bool {
    let base = normalize(token);
    provider.aliases.iter().any(|alias| {
        base == *alias
            || base == format!("{alias}.js")
            || base == format!("{alias}.mjs")
            || base == format!("{alias}.cjs")
    })
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
        assert_eq!(detect(Some("copilot"), &[]).unwrap().id, "copilot");
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
        let c = by_id("copilot").unwrap();
        assert_eq!(
            resume_argv(c, Some("9a2b"), &[]),
            vec!["copilot", "--resume", "9a2b"]
        );
    }

    #[test]
    fn launch_args_keep_provider_flags_and_drop_the_resume_pair() {
        let claude = by_id("claude-code").unwrap();
        let saved = argv(&[
            "/usr/local/bin/claude",
            "--allow-dangerously-skip-permissions",
            "--resume",
            "old-id",
            "--model",
            "opus",
        ]);
        assert_eq!(
            launch_args(claude, &saved),
            argv(&["--allow-dangerously-skip-permissions", "--model", "opus"])
        );
    }

    #[test]
    fn launch_args_skip_a_node_hosted_script_path() {
        let claude = by_id("claude-code").unwrap();
        let saved = argv(&[
            "node",
            "/usr/local/lib/node_modules/claude/cli.js",
            "--allow-dangerously-skip-permissions",
        ]);
        assert_eq!(
            launch_args(claude, &saved),
            argv(&["--allow-dangerously-skip-permissions"])
        );
    }

    #[test]
    fn launch_args_ignore_a_plain_program_name() {
        let opencode = by_id("opencode").unwrap();
        assert!(launch_args(opencode, &argv(&["opencode"])).is_empty());
    }

    #[test]
    fn resume_with_extras_appends_them_after_the_template() {
        let claude = by_id("claude-code").unwrap();
        let extras = argv(&["--allow-dangerously-skip-permissions"]);
        assert_eq!(
            resume_argv(claude, Some("sess-1"), &extras),
            argv(&[
                "claude",
                "--resume",
                "sess-1",
                "--allow-dangerously-skip-permissions"
            ])
        );
    }

    #[test]
    fn resume_without_ref_drops_placeholder() {
        let p = by_id("opencode").unwrap();
        assert_eq!(resume_argv(p, None, &[]), vec!["opencode", "--session"]);
        let c = by_id("custom").unwrap();
        assert!(resume_argv(c, None, &[]).is_empty());
    }
}

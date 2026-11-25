//! Launch profiles, custom commands, and recent-choice ranking.
//!
//! Three related things live here, because they share one launch shape:
//!
//! - A **profile** is a named way to start a terminal: a program, arguments,
//!   environment, working directory and shell. It replaces the single `shell`
//!   key without breaking it, since a profile with only a program is what that
//!   key meant.
//! - A **command** is a one-shot launch the user names, with the same fields.
//!   The difference is intent, not structure: a profile is what a new tab
//!   starts, a command is something you run on purpose.
//! - **Recent ranking** orders the picker by what the user actually chose, so
//!   the fourth time they open `wsl` it is at the top.
//!
//! Everything is parsed and ranked here, without I/O, so both shells and the
//! CLI agree on what a profile means and the ranking is testable.

use serde::{Deserialize, Serialize};

use crate::CoreError;
use crate::ErrorCode;

/// How many recent picks are remembered. Enough to rank a short list without
/// turning the ordering into a history of its own.
pub const RECENT_CAPACITY: usize = 16;

/// A program to run: a path or a name on `PATH`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Program {
    /// A bare string is the program alone, so a Windows path with spaces needs
    /// no quoting.
    One(String),
    /// A list is program plus fixed arguments.
    Argv(Vec<String>),
}

impl Program {
    pub fn argv(&self) -> Vec<String> {
        match self {
            Program::One(program) => vec![program.clone()],
            Program::Argv(argv) => argv.clone(),
        }
    }
}

/// One named launch: a profile or a custom command. The two differ only in how
/// the picker lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launch {
    pub name: String,
    #[serde(default)]
    pub command: Option<Program>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// The shell to run the command under, for the presets that need one
    /// (`wsl.exe -e bash -lc …`). Empty means the command runs directly.
    #[serde(default)]
    pub shell: Option<Program>,
    /// A tab label. Defaults to the name.
    #[serde(default)]
    pub label: Option<String>,
}

impl Launch {
    /// The command's own argv: the program plus its fixed arguments plus the
    /// extra ones. Returns an empty list for a launch with no command, which
    /// the caller treats as "the default shell".
    pub fn argv(&self) -> Vec<String> {
        let Some(command) = &self.command else {
            // `args` without a program is a config mistake, not a way to pass
            // arguments to the default shell: `validate` rejects it.
            return Vec::new();
        };
        let mut argv = command.argv();
        argv.extend(self.args.iter().cloned());
        argv
    }

    /// The argv the caller should spawn: the shell wrapper when one is set,
    /// otherwise the command itself. This is the only accessor a shell should
    /// use, so `launch.list` and `launch.open` cannot disagree.
    pub fn spawn_argv(&self) -> Vec<String> {
        self.shell_argv().unwrap_or_else(|| self.argv())
    }

    /// The environment as pairs, for the spawn request.
    pub fn env_pairs(&self) -> Vec<(String, String)> {
        self.env
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }

    /// What a tab started from this launch should be called.
    pub fn tab_label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.name)
    }

    /// Whether the launch names a program at all, directly or through a shell.
    pub fn has_command(&self) -> bool {
        self.command.is_some() || self.shell.is_some()
    }

    /// The full argument list when a shell is configured: the shell's own argv
    /// followed by the command, which is how `wsl.exe -e bash -lc` is spelled.
    pub fn shell_argv(&self) -> Option<Vec<String>> {
        let shell = self.shell.as_ref()?;
        let mut argv = shell.argv();
        argv.extend(self.argv());
        Some(argv)
    }
}

/// The `commands` and `profiles` sections of `config.json`, plus the ranking.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Launches {
    /// Named launch presets, offered when a new tab or window starts.
    pub profiles: Vec<Launch>,
    /// One-shot commands, offered in the palette and the CLI.
    pub commands: Vec<Launch>,
}

impl Launches {
    /// Find by name, profiles first. Names are matched exactly: they are keys
    /// a config author writes, not a fuzzy label.
    pub fn find(&self, name: &str) -> Option<&Launch> {
        self.profiles
            .iter()
            .chain(self.commands.iter())
            .find(|launch| launch.name == name)
    }

    /// Reject the shapes a shell cannot spawn: an empty name, a duplicate, or
    /// a command with an empty program. Returns the first problem, so a bad
    /// config file is reported rather than silently half-applied.
    pub fn validate(&self) -> Result<(), CoreError> {
        let mut seen = std::collections::HashSet::new();
        for launch in self.profiles.iter().chain(self.commands.iter()) {
            if launch.name.trim().is_empty() {
                return Err(invalid("a profile or command has an empty name"));
            }
            if !seen.insert(launch.name.as_str()) {
                return Err(invalid(format!(
                    "duplicate profile or command name {:?}",
                    launch.name
                )));
            }
            if launch.command.as_ref().is_some_and(|program| {
                program
                    .argv()
                    .first()
                    .is_none_or(|first| first.trim().is_empty())
            }) {
                return Err(invalid(format!(
                    "{:?} has an empty command program",
                    launch.name
                )));
            }
            // Arguments with no program would silently run the default shell
            // with nothing: a config mistake worth reporting.
            if launch.command.is_none() && !launch.args.is_empty() {
                return Err(invalid(format!(
                    "{:?} has arguments but no command to pass them to",
                    launch.name
                )));
            }
        }
        Ok(())
    }
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::BadRequest, message)
}

/// A tree the user saved by name, so it can be opened again. The tree is the
/// same [`crate::model::PaneNode`] the workspace uses, so restoring a layout is
/// ordinary workspace creation, not a second shape to keep in sync.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedLayout {
    pub name: String,
    /// The split tree, with leaves naming panes.
    pub root: crate::model::PaneNode,
    /// The cwd of each pane, by pane id. A layout is a shape plus where each
    /// pane starts.
    #[serde(default)]
    pub cwd: std::collections::BTreeMap<String, String>,
    /// The command each pane runs, by pane id, when it was not a plain shell.
    #[serde(default)]
    pub argv: std::collections::BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub saved_ms: u64,
}

/// Recent picks, most recent first. Kept in memory: it is a convenience
/// ordering, not state worth persisting.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Recent {
    names: Vec<String>,
}

impl Recent {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a pick, moving a repeat to the front.
    pub fn record(&mut self, name: &str) {
        if name.is_empty() {
            return;
        }
        self.names.retain(|existing| existing != name);
        self.names.insert(0, name.to_string());
        self.names.truncate(RECENT_CAPACITY);
    }

    /// Most recent first.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Rank `launches` for a picker: recently chosen first, in recency order,
    /// then everything else in its configured order. A name that is no longer
    /// configured is dropped rather than shown as a dead row.
    pub fn rank<'a>(&self, launches: &'a [Launch]) -> Vec<&'a Launch> {
        let mut ordered: Vec<&Launch> = Vec::with_capacity(launches.len());
        for name in &self.names {
            if let Some(launch) = launches.iter().find(|launch| &launch.name == name) {
                ordered.push(launch);
            }
        }
        for launch in launches {
            if !ordered.iter().any(|seen| seen.name == launch.name) {
                ordered.push(launch);
            }
        }
        ordered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(name: &str) -> Launch {
        Launch {
            name: name.to_string(),
            command: Some(Program::One(format!("/bin/{name}"))),
            args: Vec::new(),
            env: Default::default(),
            cwd: None,
            shell: None,
            label: None,
        }
    }

    fn launches(names: &[&str]) -> Vec<Launch> {
        names.iter().map(|name| launch(name)).collect()
    }

    #[test]
    fn given_a_bare_program_then_it_is_one_argv_entry_with_spaces_intact() {
        let program = Program::One("C:\\Program Files\\Git\\bin\\bash.exe".into());
        assert_eq!(
            program.argv(),
            vec!["C:\\Program Files\\Git\\bin\\bash.exe"]
        );
    }

    #[test]
    fn given_a_launch_then_argv_is_command_then_fixed_then_extra() {
        let launch = Launch {
            command: Some(Program::Argv(vec!["bash".into(), "-l".into()])),
            args: vec!["-c".into(), "make".into()],
            ..launch("build")
        };
        assert_eq!(launch.argv(), vec!["bash", "-l", "-c", "make"]);
    }

    #[test]
    fn given_a_launch_with_no_command_then_argv_is_empty() {
        let launch = Launch {
            command: None,
            ..launch("empty")
        };
        assert!(launch.argv().is_empty());
        assert!(!launch.has_command());
    }

    #[test]
    fn given_a_shell_then_the_shell_wraps_the_command() {
        let launch = Launch {
            command: Some(Program::One("make".into())),
            args: vec!["test".into()],
            shell: Some(Program::Argv(vec!["wsl.exe".into(), "-e".into()])),
            ..launch("wsl-build")
        };
        assert_eq!(
            launch.shell_argv().unwrap(),
            vec!["wsl.exe", "-e", "make", "test"]
        );
    }

    #[test]
    fn given_env_then_it_becomes_spawn_pairs() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("BER".to_string(), "1".to_string());
        let launch = Launch {
            env,
            ..launch("env")
        };
        assert_eq!(
            launch.env_pairs(),
            vec![("BER".to_string(), "1".to_string())]
        );
    }

    #[test]
    fn given_no_label_then_the_tab_is_named_for_the_launch() {
        assert_eq!(launch("wsl").tab_label(), "wsl");
        let labelled = Launch {
            label: Some("Ubuntu".into()),
            ..launch("wsl")
        };
        assert_eq!(labelled.tab_label(), "Ubuntu");
    }

    #[test]
    fn given_well_formed_launches_then_they_validate() {
        let set = Launches {
            profiles: launches(&["a", "b"]),
            commands: launches(&["c"]),
        };
        assert!(set.validate().is_ok());
    }

    #[test]
    fn given_a_duplicate_name_then_validation_fails() {
        let set = Launches {
            profiles: launches(&["same"]),
            commands: launches(&["same"]),
        };
        assert!(set.validate().is_err());
    }

    #[test]
    fn given_an_empty_name_then_validation_fails() {
        let set = Launches {
            profiles: vec![launch("")],
            commands: Vec::new(),
        };
        assert!(set.validate().is_err());
    }

    #[test]
    fn given_a_command_with_an_empty_program_then_validation_fails() {
        let set = Launches {
            profiles: Vec::new(),
            commands: vec![Launch {
                command: Some(Program::One("  ".into())),
                ..launch("blank")
            }],
        };
        assert!(set.validate().is_err());
    }

    #[test]
    fn given_names_then_find_searches_profiles_then_commands() {
        let set = Launches {
            profiles: launches(&["p"]),
            commands: launches(&["c"]),
        };
        assert_eq!(set.find("p").unwrap().name, "p");
        assert_eq!(set.find("c").unwrap().name, "c");
        assert!(set.find("missing").is_none());
    }

    #[test]
    fn given_recent_picks_then_the_picker_puts_them_first_in_recency_order() {
        let set = launches(&["a", "b", "c", "d"]);
        let mut recent = Recent::new();
        recent.record("c");
        recent.record("a");
        let names: Vec<&str> = recent.rank(&set).iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, vec!["a", "c", "b", "d"]);
    }

    #[test]
    fn given_a_repeated_pick_then_it_moves_to_the_front() {
        let mut recent = Recent::new();
        recent.record("a");
        recent.record("b");
        recent.record("a");
        assert_eq!(recent.names(), &["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn given_a_removed_name_then_ranking_drops_the_recent_entry() {
        let set = launches(&["a"]);
        let mut recent = Recent::new();
        recent.record("gone");
        recent.record("a");
        let names: Vec<&str> = recent.rank(&set).iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, vec!["a"]);
    }

    #[test]
    fn given_more_picks_than_capacity_then_the_oldest_fall_off() {
        let mut recent = Recent::new();
        for index in 0..RECENT_CAPACITY + 5 {
            recent.record(&format!("n{index}"));
        }
        assert_eq!(recent.names().len(), RECENT_CAPACITY);
        assert_eq!(recent.names()[0], format!("n{}", RECENT_CAPACITY + 4));
    }

    #[test]
    fn given_a_json_config_then_a_bare_program_and_a_list_both_parse() {
        let bare: Launch = serde_json::from_str(r#"{"name":"bash","command":"/bin/bash"}"#)
            .expect("a bare program");
        assert_eq!(bare.argv(), vec!["/bin/bash"]);

        let listed: Launch =
            serde_json::from_str(r#"{"name":"wsl","command":["wsl.exe","-e"],"args":["bash"]}"#)
                .expect("a list program");
        assert_eq!(listed.argv(), vec!["wsl.exe", "-e", "bash"]);
    }
}

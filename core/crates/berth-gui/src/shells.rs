//! Shell discovery for the settings panel.
//!
//! A terminal surface with no argv runs the platform default, which on Windows
//! is `%ComSpec%` (cmd.exe). Nobody installs bash or zsh there by default, but
//! Git for Windows, MSYS2, Cygwin and WSL each drop one in a predictable place,
//! so the panel offers what is actually installed instead of asking for a path.
//!
//! Probing happens once per process: [`choices`] is called on every settings
//! redraw.

use std::env;
use std::path::Path;
use std::sync::OnceLock;

/// One entry of the shell row's cycle.
pub struct Choice {
    pub label: String,
    /// argv handed to the PTY. Empty is the platform default shell.
    pub argv: Vec<String>,
}

/// The shells this machine can run, default first. Probed once.
pub fn choices() -> &'static [Choice] {
    static CHOICES: OnceLock<Vec<Choice>> = OnceLock::new();
    CHOICES.get_or_init(build)
}

/// How the settings row spells a configured argv: `default` when it is empty,
/// an installed shell's name when it matches, `custom` otherwise (the value
/// came from the file and the program may have been removed since).
pub fn label_for(argv: &[String]) -> String {
    if argv.is_empty() {
        return "default".to_string();
    }
    choices()
        .iter()
        .find(|choice| choice.argv == argv)
        .map(|choice| choice.label.clone())
        .unwrap_or_else(|| "custom".to_string())
}

fn build() -> Vec<Choice> {
    let mut choices = vec![Choice {
        label: "default".into(),
        argv: Vec::new(),
    }];
    for (label, program, args) in candidates() {
        if !is_runnable(Path::new(&program)) {
            continue;
        }
        let argv = std::iter::once(program).chain(args).collect::<Vec<_>>();
        if choices.iter().any(|choice| choice.argv == argv) {
            continue;
        }
        choices.push(Choice { label, argv });
    }
    choices
}

/// `(label, program, extra arguments)` for everything worth probing.
fn candidates() -> Vec<(String, String, Vec<String>)> {
    #[cfg(windows)]
    {
        windows_candidates()
    }
    #[cfg(not(windows))]
    {
        unix_candidates()
    }
}

#[cfg(windows)]
fn windows_candidates() -> Vec<(String, String, Vec<String>)> {
    let program_files =
        env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
    let system_root = env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    let local_app_data = env::var("LOCALAPPDATA").unwrap_or_default();

    let mut list: Vec<(String, String, Vec<String>)> = vec![
        (
            "pwsh".into(),
            format!(r"{program_files}\PowerShell\7\pwsh.exe"),
            Vec::new(),
        ),
        (
            "windows powershell".into(),
            format!(r"{system_root}\System32\WindowsPowerShell\v1.0\powershell.exe"),
            Vec::new(),
        ),
        (
            "git bash".into(),
            format!(r"{program_files}\Git\bin\bash.exe"),
            login("bash"),
        ),
        (
            "msys2 zsh".into(),
            r"C:\msys64\usr\bin\zsh.exe".into(),
            login("zsh"),
        ),
        (
            "msys2 bash".into(),
            r"C:\msys64\usr\bin\bash.exe".into(),
            login("bash"),
        ),
        (
            "cygwin zsh".into(),
            r"C:\cygwin64\bin\zsh.exe".into(),
            login("zsh"),
        ),
        (
            "cygwin bash".into(),
            r"C:\cygwin64\bin\bash.exe".into(),
            login("bash"),
        ),
        (
            "wsl".into(),
            format!(r"{system_root}\System32\wsl.exe"),
            Vec::new(),
        ),
    ];
    if !local_app_data.is_empty() {
        // A per-user Git install, the default for the Git for Windows installer.
        list.push((
            "git bash".into(),
            format!(r"{local_app_data}\Programs\Git\bin\bash.exe"),
            login("bash"),
        ));
    }
    list
}

#[cfg(not(windows))]
fn unix_candidates() -> Vec<(String, String, Vec<String>)> {
    let mut list: Vec<(String, String, Vec<String>)> = Vec::new();
    // /etc/shells is the system's own list of login shells: zsh and bash on
    // macOS, whatever the distribution ships elsewhere.
    if let Ok(text) = std::fs::read_to_string("/etc/shells") {
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let program = std::path::PathBuf::from(line);
            let Some(name) = program.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            list.push((name.to_string(), line.to_string(), login(name)));
        }
    }
    // fish, nu and zsh installed by a package manager often miss /etc/shells.
    for name in ["zsh", "fish", "nu", "bash"] {
        if list.iter().any(|(_, program, _)| program.ends_with(name)) {
            continue;
        }
        if let Some(program) = which(name) {
            list.push((name.to_string(), program, login(name)));
        }
    }
    list
}

/// A login shell so PATH and the user's rc files load the way `Terminal.app`
/// and Git Bash set them up.
fn login(name: &str) -> Vec<String> {
    match name {
        "bash" => vec!["--login".into(), "-i".into()],
        "zsh" => vec!["-l".into()],
        _ => Vec::new(),
    }
}

fn is_runnable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return std::fs::metadata(path)
            .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(not(windows))]
fn which(name: &str) -> Option<String> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_runnable(candidate))
        .map(|candidate| candidate.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_the_choices_when_built_then_the_default_comes_first_and_argv_is_unique() {
        let choices = choices();
        assert!(choices[0].argv.is_empty());
        assert_eq!(choices[0].label, "default");
        for (index, choice) in choices.iter().enumerate() {
            assert!(
                choices[..index]
                    .iter()
                    .all(|earlier| earlier.argv != choice.argv),
                "duplicate argv for {}",
                choice.label
            );
        }
    }

    #[test]
    fn given_an_empty_argv_when_labelled_then_it_is_the_default() {
        assert_eq!(label_for(&[]), "default");
    }

    #[test]
    fn given_an_unknown_program_when_labelled_then_it_is_custom() {
        assert_eq!(
            label_for(&["/does/not/exist/berth-test-shell".into()]),
            "custom"
        );
    }
}

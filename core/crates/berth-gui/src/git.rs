//! Read the branch a working directory has checked out, without spawning git.
//!
//! The status line wants the focused pane's branch, and it is drawn every
//! frame. Walking up to `.git` and reading `HEAD` costs a handful of stats,
//! where a `git rev-parse` per frame would fork a process. Worktrees and
//! submodules put a `gitdir:` pointer in a `.git` file instead of a directory,
//! so the pointer is followed the same way git does.

use std::path::{Path, PathBuf};

/// The branch checked out at `cwd`, or `None` outside a repository, on a
/// detached HEAD, or when `.git` cannot be read.
pub fn branch(cwd: &str) -> Option<String> {
    if cwd.is_empty() {
        return None;
    }
    let git_dir = git_dir(Path::new(cwd))?;
    read_branch(&git_dir)
}

/// Walk from `start` to the filesystem root looking for `.git`.
fn git_dir(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            return Some(dot_git);
        }
        if dot_git.is_file() {
            return parse_gitfile(dir, &dot_git);
        }
    }
    None
}

/// Resolve a `.git` file's `gitdir:` pointer. A relative pointer is relative
/// to the directory that holds the file.
fn parse_gitfile(dir: &Path, dot_git: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(dot_git).ok()?;
    let target = text.trim().strip_prefix("gitdir:")?.trim();
    if target.is_empty() {
        return None;
    }
    let target = Path::new(target);
    Some(if target.is_absolute() {
        target.to_path_buf()
    } else {
        dir.join(target)
    })
}

fn read_branch(git_dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let name = head.trim().strip_prefix("ref: refs/heads/")?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "berth-git-{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn given_a_branch_head_then_the_branch_name_is_returned() {
        let root = temp_dir("branch");
        write(&root.join(".git/HEAD"), "ref: refs/heads/feature/x\n");
        assert_eq!(branch(root.to_str().unwrap()), Some("feature/x".into()));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn given_a_subdirectory_then_the_branch_is_found_upwards() {
        let root = temp_dir("nested");
        write(&root.join(".git/HEAD"), "ref: refs/heads/main\n");
        let nested = root.join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(branch(nested.to_str().unwrap()), Some("main".into()));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn given_a_gitfile_pointer_then_the_worktree_branch_is_read() {
        let root = temp_dir("worktree");
        let worktree = root.join("wt");
        std::fs::create_dir_all(&worktree).unwrap();
        let actual = root.join("repo/.git/worktrees/wt");
        write(&actual.join("HEAD"), "ref: refs/heads/wt-branch\n");
        write(
            &worktree.join(".git"),
            &format!("gitdir: {}\n", actual.display()),
        );
        assert_eq!(branch(worktree.to_str().unwrap()), Some("wt-branch".into()));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn given_a_detached_head_then_there_is_no_branch() {
        let root = temp_dir("detached");
        write(&root.join(".git/HEAD"), "0123456789abcdef\n");
        assert_eq!(branch(root.to_str().unwrap()), None);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn given_no_repository_then_there_is_no_branch() {
        let root = temp_dir("plain");
        assert_eq!(branch(root.to_str().unwrap()), None);
        std::fs::remove_dir_all(root).ok();
    }
}

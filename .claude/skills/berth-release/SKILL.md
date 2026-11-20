---
name: Berth Release
description: Cut a Berth release end to end: bump the version, move the changelog out of Unreleased, run the test suite, commit and tag, and build the local platform's artifacts into ~/Downloads. Use whenever the user says "make a release", "cut a release", "release 0.15.1", "ship it", "put the updated app in Downloads", or asks to release work that just landed. Works on macOS, Linux, and Windows Git Bash.
---

# Berth release

## Pick the version

Patch releases stay inside the current milestone (`0.15.1`, `0.15.2`). Move to a
new minor only when `docs/04-roadmap.md` starts that milestone. Read the
`## Unreleased` section of `CHANGELOG.md` to see whether the change is a feature
or a fix, then propose the version. If the user did not name one, wait for their
approval before changing files.

## Before running

- Work in a clean worktree. This repository forbids editing the primary
  checkout; create one under `.claude/worktrees/<name>`.
- Make sure `CHANGELOG.md` has an `## Unreleased` section with the entries for
  this release. Write them first; the script does not invent content.
- When an agent runs this, attest added comments before the first `git commit`:
  `python3 ~/.claude/scripts/comment-style-gate.py --attest-comments "$PWD"`.
- After editing the script, check it with `bash -n scripts/release.sh`.

## Run

```sh
scripts/release.sh <version>
```

It does the whole mechanical sequence:

1. bumps `core/Cargo.toml`, refreshes `core/Cargo.lock`, and sets
   `packaging/arch/PKGBUILD`;
2. renames `## Unreleased` to the dated version heading;
3. runs `cargo fmt --check`, `cargo test --workspace`, and `cargo clippy`;
4. commits `release: <version>` and creates the annotated tag `v<version>`;
5. builds the local platform's artifacts:
   - **macOS**: `make app` and `make gui-app`, then verifies and copies
     `Berth.app` (AppKit shell) and `BerthGui.app` (portable window) into
     `~/Downloads`. It checks the version, the ad-hoc signature, and the binary
     architecture.
   - **Linux**: a `berth-gui-<version>-linux-<arch>.tar.gz` with both binaries
     in `~/Downloads`, plus a `.deb` when `dpkg-deb` is installed.
   - **Windows Git Bash**: a `berth-gui-<version>-windows-x86_64.zip` with both
     binaries in `~/Downloads`.

Use `--no-build` when only the version bump, commit, and tag are wanted (for
example in CI). Use `--build-only` to rebuild and copy artifacts for the
current version after a failed build; it skips the bump, commit, and tag. Use
`--push` only when the user asked to publish; it pushes `HEAD` and the tag, and
CI turns a `v*` tag into a GitHub release. A local tag is not a published
release.

On macOS the app build needs the Ghostty engine. A fresh worktree does not
contain it, so the script stages it from the primary checkout; if that copy is
missing too, run `scripts/fetch-engine.sh` first.

## Verify, then report

- `git show v<version> --stat` and `git tag -n1 v<version>` look right.
- `core/Cargo.toml` and `packaging/arch/PKGBUILD` show the new version.
- On macOS, say whether the app is arm64 or x86_64 from `file`, and confirm
  `codesign --verify --deep --strict` passed.
- On Linux and Windows, list the exact files in `~/Downloads`.
- If the artifact build fails after tagging, report it plainly: the tag exists,
  the artifact does not. Fix the build; do not move or delete the tag silently.
- If the release commit was made in a worktree, merge it to `main` before any
  push. Report the version, tag, commit, artifact paths, and test result.

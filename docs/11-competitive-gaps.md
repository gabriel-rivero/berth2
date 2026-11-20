# Competitive gap analysis

Audited 2026-11-20 at 0.14.1; 0.15 items are marked as they land. Berth's column is read
from source in this tree; the competitor columns come from vendor docs and release notes of
the same month, not from memory. The purpose is to separate three things that get
confused: **already shipped**, **missing and worth building**, and **deliberately not
ours**.

Ranked gaps and the milestone that owns each are in
[docs/04-roadmap.md](04-roadmap.md). This file is the evidence behind them.

## 1. Where Berth stands

The workspace shape is cmux's, finished: sidebar of workspaces with groups, split panes
each holding a tab stack, per-surface OSC metadata, session restore, a command palette,
an attention ladder, and a socket with subscriptions and replay. The terminal engine is
the split:

- **AppKit shell**: Ghostty owns emulation, rendering and input, so it inherits the whole
  modern protocol set. Berth's gaps there are UI, not VT.
- **Portable window (`berth-gui`)**: its own renderer over the `vt100` crate, which is why
  it has a good terminal and a narrow protocol surface (no ligatures). Synchronized
  output, the Kitty keyboard protocol, styled text and OSC 8 hyperlinks are in as of
  0.15.

That split is the single most important fact in this document: every VT-level gap below
is portable-only, and every macOS gap is UI-only.

## 2. Feature matrix

● shipped · ◐ partial · ○ absent · — not applicable to that shell.

| Area | Feature | Portable | AppKit | cmux | Common elsewhere |
| --- | --- | :-: | :-: | :-: | --- |
| **Emulation** | true colour, wide chars, RTL text | ● | ● | ● | ● |
| | ligatures | ○ | ● | ● | Ghostty, WezTerm, kitty |
| | Kitty keyboard protocol | ● | ● | ● | most 2026 GUI terminals |
| | synchronized output (DECSET 2026) | ● | ● | ● | near-universal |
| | styled and coloured underlines, strikethrough | ● | ● | ● | iTerm2, Ghostty, WezTerm |
| | OSC 8 hyperlinks | ● | ● | ● | ● |
| | OSC 52 clipboard write / read | ● / ○ | ● / ● | ● | ● |
| | focus reporting, cursor shapes (DECSCUSR) | ○ | ● | ● | ● |
| | iTerm2 / kitty / sixel images | ● | ● | ● / ● / ○ | ● |
| | resize reflow of wrapped lines | ○ | ● | ● | ● |
| | audio bell option | ○ | ● | ● | ● |
| **Scrollback** | configurable depth, wheel, scrollbar | ● | ● | ● | ● |
| | search | ● | ○ | ● | ● |
| | scrollback restore on relaunch | ○ | ○ | ● | iTerm2, kitty |
| | vi copy mode / keyboard selection | ○ | ○ | ● | iTerm2, WezTerm, Alacritty |
| | paste history | ○ | ○ | ○ | iTerm2, kitty |
| **Layout** | tabs, nested splits, drag dividers | ● | ● | ● | ● |
| | tabs inside a pane | ● | ● | ● | cmux only |
| | multiple windows | ○ | ○ | ● | ● |
| | pane zoom / maximise | ○ | ○ | ● | Ghostty, WezTerm, kitty |
| | tab tear-off, cross-window drag | ○ | ○ | ● | Ghostty, kitty |
| | tab overflow menu | ○ | ○ | ● | kitty, Warp |
| | broadcast input to all panes | ○ | ○ | ○ | kitty, tmux |
| | geometric (arrow-key) pane focus | ○ | ○ | ● | cmux, iTerm2 |
| | workspace groups, reorder, colours | ● | ◐ (no create UI) | ● | — |
| | quake / dropdown hotkey window | ○ | ○ | ● | Ghostty, iTerm2, kitty |
| **Workflow** | command palette | ● (15 verbs) | ● (13 verbs) | ● (indexes everything) | ● |
| | shell integration (OSC 133), jump to prompt | ○ | ○ | ● | ● |
| | command blocks / per-command status | ○ | ○ | ◐ | Warp, kitty marks |
| | declarative layouts, custom commands | ○ | ○ | ● | WezTerm, kitty, Warp |
| | profiles / automatic profile switching | ○ | ○ | ○ | iTerm2 |
| | triggers on output | ○ | ○ | ○ | iTerm2, Alacritty |
| | file drag-and-drop | ○ | ○ | ○ | iTerm2, kitty |
| **Agents** | typed lifecycle, attention, events | ● | ● | ◐ (free-form) | Berth only |
| | hook install for providers | ● (4) | ● (4) | ● (13) | — |
| | resume across relaunch | ● | ● | ● | — |
| | multi-agent rows in the sidebar | ◐ (count) | ◐ (count) | ● | — |
| | per-agent inbox / timeline | ○ | ○ | ● | — |
| | token and cost meter | ◐ (per row) | ◐ (per row) | ○ | Warp |
| | OS notifications | ● / ○ Windows | ● | ● | ● |
| | dock badge, sounds, menu bar item | ○ | ○ | ● | iTerm2 |
| **Session** | layout, tabs, cwd restore | ● | ● | ● | ● |
| | agent resume | ● | ● | ● | — |
| | live processes survive quit/update | ○ | ○ | ◐ (tmux/zellij) | WezTerm mux |
| | single-instance handoff | ○ | ○ | ● | ● |
| | window geometry persisted | ○ | ● | ● | ● |
| **Platform** | macOS / Linux / Windows | ● | ● (macOS) | ○ | — |
| | installers | ◐ (.deb, PKGBUILD) | ◐ (ad-hoc) | ● | ● |
| | auto-update | ○ | ○ | ● (Sparkle) | iTerm2, Warp |
| | terminal accessibility | ◐ (focused pane) | ○ | ◐ | Terminal.app |
| **Protocol** | event stream with replay | ● | ● | ○ | Berth only |
| | CLI for every method | ● | ● | ● | WezTerm, kitty |
| | typed client library | ○ | ○ | ○ | WezTerm, kitty |
| | SSH workspaces, remote tmux | ○ | ○ | ● | iTerm2, WezTerm, kitty |

## 3. The gaps that matter, in order

1. **The portable terminal is behind on protocol, not features.** Focus reporting and
   cursor shapes are what modern TUIs and agents test for; synchronized output, the
   kitty keyboard protocol, styled text and OSC 8 have landed. This is a bounded parsing
   and painting job, and it is the cheapest way to stop the portable window feeling like
   a different product.
2. **Multiple windows, pane zoom, tab tear-off.** Every competitor has them; Berth has one
   window and no zoom. This is the most frequent shape of "I can't work like this" for a
   terminal workspace, and the protocol already models the tree, so it is a model and
   snapshot change rather than a rewrite.
3. **Shell integration.** OSC 133 is table stakes in 2026: prompt marks, exit status,
   jump-to-prompt and per-command metadata. Berth consumes titles, cwd, bell and progress,
   so it has the plumbing and none of the payload.
4. **Live process durability.** cmux's answer is bolting on tmux; WezTerm owns a mux
   server. Berth's architecture already separates the domain core from the shell, so a
   detachable PTY host is the honest version of this and the one item that changes the
   process model.
5. **Agent depth is the differentiator that is still half-built.** The typed state machine
   and event stream exist; there is no inbox, no history beyond the live notification
   list, no workspace-level cost meter, and four providers against cmux's thirteen.
6. **Search and recall.** The portable window searches; the AppKit shell does not, and
   neither has copy mode, paste history or a search across workspaces.
7. **Distribution.** .deb and PKGBUILD exist, the notarization script is written but
   unused, and there is no auto-update. This is the difference between a repo and a
   product, and it is mostly CI work.

Every row of the matrix above is mapped to a milestone, or to a documented decision, in
the coverage ledger at the end of [docs/04-roadmap.md](04-roadmap.md).

## 4. ConEmu, the dead Windows incumbent

Last release 2023-07-24 and no maintainer activity since; its issue tracker has been
asking whether the project is over. It matters anyway: for a decade it was the Windows
terminal people configured instead of `conhost`, it is the closest ancestor of this
product's shape (tabs, splits, quake, task presets, a macro API), and its users are the
one audience with no obvious successor. Windows Terminal replaced most of them; cmux does
not run there; the portable window is the only terminal of this shape that does.

| ConEmu feature | Berth today |
| --- | --- |
| Tabs, splits and nested panes, drag resize | ● both shells |
| Quake dropdown on a global hotkey | ○ planned, 0.17 |
| **Shell selection** (`shell`, settings cycle: Git Bash, pwsh, MSYS2, Cygwin, WSL) | ● 0.14.1 |
| **Named task presets** (argv, env, cwd, shell, admin, hotkey) | ○ one shell per config |
| **Elevated tabs** (`ConEmuC /sudo`) | ○ |
| **Replace the default Windows terminal** | ○ |
| Session/layout restore | ● tabs, cwd, splits; no scrollback |
| Unicode, 256/true colour, mouse reporting, clipboard, paste guard | ● |
| Progress in tab/sidebar (OSC 9;4) | ● |
| Search in the buffer | ● portable, ○ AppKit |
| Tab rename / restart / duplicate | ◐ rename only |
| Hotkey rebinding | ● |
| **Automation API** (`GuiMacro`, `ConEmuC -GuiMacro`) | ● superseded by socket, CLI and events |
| Link and file-path detection | ● |
| **Output highlight rules** (regex → line style) | ○ |
| **Log console output to a file** | ○ |
| **Window transparency / background** | ○ |
| **Status bar and toolbar** | ○ |
| **Taskbar progress and jump lists** | ○ |
| Named configurations, portable XML settings | ◐ one `config.json` |
| Attach/detach a running console | ○ planned as 0.18 durability |
| Far Manager child GUI, DOSBox | N/A; ConPTY makes the hook layer obsolete |

What is worth taking, in order:

1. **Launch profiles.** `shell` is the first step toward ConEmu's Tasks. Windows users
   start WSL, Git Bash, admin PowerShell and a specific project directory from one menu;
   today that is one global key. A profile needs argv, env, cwd and a label, on both
   shells.
2. **Default-terminal handoff on Windows 11.** ConEmu had this from 2012. Without it the
   portable window is an app you remember to open, not the thing `cmd` and `pwsh` open
   into. It is the largest Windows adoption feature on the list.
3. **Quake dropdown**, already scheduled for 0.17, and **taskbar progress plus jump
   lists**, which is how a Windows user finds a running build again.
4. **Output highlight rules and session logging.** Both are cheap, both exist in iTerm2
   as Triggers and in ConEmu as Highlight and logging, and both are what an agent user
   wants when a 2,000-line run ends in one red line.
5. **Transparency and a status bar.** Polish, and the status bar is implied by the
   portable window having no place to show chord mode, broadcast state or pane zoom.

Elevated launch stays out of a pane: it needs a separate process at a different integrity
level, and mixing that into the terminal's PTY model is how Windows terminal projects
lose a week.

## 5. Where the competition is weak (our openings)

- **cmux costs memory and CPU at scale** and freezes under many tabs; its own issue
  tracker carries 16 GB for 187 surfaces. Berth's eight-pane budget of ~161 MB and idle
  ~0 % CPU is a real advantage that the roadmap should not spend.
- **cmux's agent state is inferred**, ours is typed and subscribable. Nothing in the
  competitor matrix beats that; the gap is in what we put on top of it.
- **Settings sprawl.** cmux ships 200+ settings and users cannot find the bell toggle.
  Every new key we add should earn its place and appear in the in-app settings panel.
- **macOS-only.** The portable window is the only agent-workspace terminal that runs on
  Linux and Windows. Parity work there has no direct competitor.

## 6. Not gaps (do not reopen)

Browser panes, cookie import, SSH workspaces, remote tmux mirroring, iOS, cloud VMs,
teams, plugin marketplaces, a TUI client, telemetry. They are excluded in
[docs/01-vision.md](01-vision.md#4-non-goals-for-v1) and appear here only so that a
competitor shipping one is not mistaken for a defect. Live-process durability is the one
borderline case: it is deferred to 0.18 as an architectural question (own daemon versus
attach), not as a copy of cmux's tmux fallback.

## 7. Sources

- cmux: [docs](https://cmux.com/docs/concepts), [changelog](https://cmux.com/docs/changelog),
  [API](https://cmux.com/docs/api), [session restore](https://cmux.com/docs/session-restore),
  [repo](https://github.com/manaflow-ai/cmux).
- Protocol adoption: [terminfo.dev extension matrix](https://terminfo.dev/api/v1/data.json);
  [Kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/);
  [synchronized output spec](https://github.com/contour-terminal/contour/blob/master/docs/vt-extensions/synchronized-output.md).
- Vendors: [Ghostty features](https://ghostty.org/docs/features),
  [WezTerm](https://wezterm.org/features.html),
  [kitty](https://sw.kovidgoyal.net/kitty/overview/),
  [Alacritty](https://github.com/alacritty/alacritty/blob/master/docs/features.md),
  [iTerm2](https://iterm2.com/features.html).
- cmux weaknesses: [HN discussion](https://news.ycombinator.com/item?id=49206763),
  [memory issue #4607](https://github.com/manaflow-ai/cmux/issues/4607).
- ConEmu: [documentation index](https://conemu.github.io/en/TableOfContents.html),
  [tasks](https://conemu.github.io/en/Tasks.html),
  [default terminal](https://conemu.github.io/en/DefaultTerminal.html),
  [GuiMacro](https://conemu.github.io/en/GuiMacro.html),
  [last release 230724](https://github.com/ConEmu/ConEmu/releases/tag/230724),
  [maintenance question #2608](https://github.com/ConEmu/ConEmu/issues/2608).

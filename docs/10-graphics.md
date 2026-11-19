# Graphics protocols (design)

Status: all three protocols render. iTerm2 and kitty were verified with real
tools; sixel is verified with `chafa --format sixel`. The PTY reader separates
APC and DCS sequences, reassembles chunked kitty transmissions, decodes
PNG/RGB/RGBA and sixel, stores images, answers the `a=q` capability probe and
paints kitty U+10EEEE placeholder cells from that store, including the row and
column diacritics. The portable window is smoke-tested under X11 and a headless
Weston Wayland session in CI.

## The question this answers

`docs/04-roadmap.md` leaves the renderer decision open on one point: do we need
terminal graphics? The answer is yes, so this is the plan for carrying images
through `vt100`-based `berth-gui` without moving to `libghostty-vt` or a GPU
renderer.

## Why it is not a small patch

The parser is `vt100`, which models text cells only. The bytes for an image are
consumed and dropped, and the renderer has no layer between the cell buffer and
the frame. Supporting images therefore needs three pieces that do not exist
today: a side-channel parser, an image store anchored to the grid, and a paint
pass.

## Design

1. **Side-channel scanner.** iTerm2 is done through `vt100`'s OSC callback.
   Kitty (APC) and sixel (DCS) are not: `vt100` 0.18 has no APC or DCS callback,
   so those bytes are consumed and dropped. A scanner in the PTY reader holds
   `ESC _ … ESC \` and `ESC P … ESC \` aside, feeds the preceding bytes to
   `vt100`, reads the cursor cell, stores the image, then continues after the
   terminator. It survives a sequence split across two reads; the state is
   `Normal`, `Apc`, `Dcs`, plus a pending lone `ESC`. A DCS that is not sixel
   (DECRQSS, XTVERSION) is replayed to `vt100` unchanged.
   The scanner also answers the `a=q` capability probe: `OK` when graphics are
   on and `ENOTSUPPORTED` for shared memory or when `graphics` is off. Without
   that reply, clients such as yazi assume no image support and never transmit.
2. **Image store.** Each surface owns a bounded list of images: protocol id,
   pixel buffer, pixel size, the cell rectangle or auto size, and an absolute
   line anchor (`scrollback_len + cursor_row` at insert). Scrolling and
   scrollback use that absolute anchor, so images move with the text they were
   printed after. Images are not persisted in `session.json`; a restore starts
   without them.
3. **Paint pass.** After the cell backgrounds and text of a pane, draw the
   images whose anchor is visible, clipped to the pane content rect. Scale to
   the cell rectangle, or to the pixel size capped by the pane. No GPU work.
4. **Limits.** A per-image pixel cap (for example 4096×4096), a per-surface
   cache cap with oldest-first eviction, and an option to ignore graphics
   altogether. Image data is untrusted input: validate dimensions before
   allocating.

## Protocol order

| Phase | Protocol | Decode | Tool support |
| --- | --- | --- | --- |
| 1 | iTerm2 OSC 1337 `File=` | base64 + PNG (landed) | `imgcat`, `viu`, `chafa`, `timg` |
| 2 | Sixel (DCS) | raster attributes, palette registers (RGB and HLS), repeats, `$` and `-` (landed) | `chafa`, `viu`, `timg`, `lsix`, `img2sixel` |
| 3 | Kitty graphics (APC `G`) | PNG / RGB / RGBA, chunks, ids, Unicode placeholders, `a=q`, `a=T`, `a=d`; no shared memory, z-index or direct placements | yazi, `kitten icat`, `viu`, `chafa`, Ghostty-native tools |

Phase 1 is the smallest complete path and covers the widest set of CLI tools.
Phase 2 is the compatibility workhorse on Linux. Phase 3 is the Ghostty-parity
target and the largest parser (transmission chunks, placement ids, delete
actions). Kitty's shared memory, z-index and direct placements are not
implemented, so clients that need them fall back to base64 or text.

## Interaction with the rest of the window

- **Selection and copy** still read text cells; images are skipped.
- **Search** stays text-only; match highlighting ignores image rectangles.
- **Scrollback** moves images with their anchor line. Images older than the
  configured scrollback are dropped with the lines.
- **Resize** keeps the pixel size and re-anchors to the cell rectangle; a
  terminal resize does not reflow an image.
- **Accessibility** describes an image as `image` with its name and size, not
  its pixels.

## Tests

- Unit tests for each sequence parser: fragmented chunks, base64 padding,
  malformed data, dimension limits, and a sequence split across reads.
- Golden PNG frames: a pane with a known image at a known cell, before and
  after scrolling, compared against committed output like the existing smoke
  capture.
- Real-tool checks: `imgcat`/`viu` for iTerm2, `chafa --format sixel` for sixel,
  `kitten icat` for kitty.

## Decision

Stay on winit + `vt100` + cosmic-text, with the side-channel scanner for the
protocols `vt100` cannot represent. A renderer migration is not justified for
graphics.

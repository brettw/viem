# Diff mode

Status: design draft. Nothing here is implemented.

Diff mode shows a document beside an earlier version of itself so the writer
can review and edit their changes.

## Layout

Diff mode is side by side:

- **Base** (left): the earlier version selected in the diff toolbar. It is
  read-only.
- **New** (right): the document's live buffer, editable in Code mode. The
  comparison always uses the live buffer, including unsaved edits, never the
  file on disk.

Showing the new side in Code reprojects the buffer as Code, as turning
Preview off does. It never converts the source (see
[source formats](../AGENTS.md#product)).

### Line numbers

Each view has its own line-number state. Diff mode and Code each keep an
application-wide saved setting, which records the last state set explicitly
in that mode. Entering diff mode uses the saved diff-mode setting, or shows
line numbers on both sides if none has been saved. Leaving diff mode uses the
saved Code setting.

### Alignment

An **Align sides** toggle, in both the diff toolbar and the Diff menu, controls
whether the two sides line up. Each diff view (a pair of sides) has its own
Align sides state. Changing it explicitly also saves that state in settings,
and new diff views start with the saved state, or on if none has been saved.
Changing one view's state does not change other open views.

- **On:** the two sides scroll together. Corresponding lines on the left and
  right start at the same height. Where one side has lines the other lacks,
  or a line takes more rows on one side (for example because it wraps), the
  other side gets blank filler space to keep them level. Filler is drawn by layout only: it is not source, cannot
  be edited, selected or given the caret, and never reaches the clipboard,
  search or line numbers.
- **Off:** each side is laid out as a normal file on its own, with additions
  and removals highlighted but no filler, and scrolls independently.

## Styles

Diff mode adds four styles to the Code stylesheet:

| Style | Kind | Marks | Typical appearance |
| --- | --- | --- | --- |
| Added line | Paragraph | A line on the new side that is wholly or partly added | Green background |
| Removed line | Paragraph | A line on the base side that is wholly or partly removed | Red background |
| Added text | Character | The added parts of an Added line | Darker green background |
| Removed text | Character | The removed parts of a Removed line | Darker red background |

The layers resolve in this order, each overriding only what it explicitly
declares:

1. The diff paragraph style (Added line or Removed line).
2. The line's ordinary styles, such as syntax highlighting. A syntax style
   that sets only a foreground keeps the diff background.
3. The diff character style (Added text or Removed text), for the changed
   parts only.

For example, indenting a line marks the whole new line as Added line and only
the inserted whitespace as Added text. The old line on the base side is a
Removed line with no Removed text, since nothing in it was deleted. Aligned
filler is blank and takes no diff style.

Diff mode is an exception to the rule that at most one named character style
applies at a point: Added text and Removed text apply on top of the named
syntax style beneath them.

These styles are separate from Vim's highlighting. They neither inherit from
nor change the existing `DiffAdd`, `DiffChange`, `DiffDelete`, `DiffText`,
`Added`, `Removed` and `Changed` Code styles, which syntax highlighting keeps
using for patch files and similar, and Vim color scheme import does not set
them.

Diff styling is a presentation layer: it never reaches source, the clipboard,
registers, dirty state or undo.

## Diff menu

A new top-level **Diff** menu sits immediately before **View**. Its first item,
**Diff Mode**, is a checkmark item that turns diff mode on and off for the
current window. Diff mode is per window: each window has its own on/off
state, base and file list.

Turning diff mode on selects the Unstaged (`git diff`) base when the document
is in a git repository, and Last saved otherwise. Outside a repository the git
bases are unavailable.

## Diff toolbar

A new diff toolbar selects the base:

| Base | Compares the buffer with | Git equivalent |
| --- | --- | --- |
| Last saved | The saved copy Viem keeps in memory; Refresh reads the file on disk | — |
| Unstaged | The file's staged version (the index) | `git diff` |
| HEAD | The file in the current commit | `git diff HEAD` |
| Branch… | The file on a chosen branch | `git diff <branch>` |
| File… | Any file chosen in a file picker | — |

"Unstaged" is a working name for the `git diff` base. Branch compares with the
branch tip, as `git diff main` does, not with the merge base.

A **Choose base file** button in the toolbar opens the native file picker and
makes the chosen file the base, so any two files can be compared explicitly.
The picker accepts every file, including extensionless files and dotfiles.
The base side shows the chosen file read-only; it is not opened as a document
and gains no buffer, history or dirty state.

A **Refresh** button reloads the selected base from its source; for Last
saved, that means reading the file on disk. The base otherwise stays fixed
while diff mode is open.

### Files

A diff can cover many files; for example, many files may differ from `main`.
The toolbar has a file dropdown listing the files in the current diff.
Choosing one shows that file in diff mode against the same base.

For the git bases the list holds tracked files that differ from the base,
including files whose only differences are unsaved edits in an open buffer.
Untracked files are ignored. For Last saved the list holds only the current
file. With a chosen base file the comparison is explicit, so the list is
empty and the arrows do nothing. Opening another document keeps the chosen
base file, so the new document is compared with it.

Left and right arrow buttons on either side of the dropdown move to the
previous and next file in the list, wrapping around at either end. Moving
away from a file with unsaved changes first shows the same native save prompt
as closing the window (Save, Don't Save, Cancel); Cancel stays on the current
file. Choosing a file from the dropdown behaves the same way.

A file deleted since the base is listed like any other. Its base side shows
the base version and its new side is blank and not editable.

Renamed files appear as `git diff` shows them: git's own rename detection,
including the user's `diff.renames` setting, decides what counts as a rename.
A renamed file is listed once as `old → new`, with the old path's base
version on the base side and the new file's live buffer on the new side.

Files that are not in the list can still be opened the usual ways (`:edit`,
the Open dialog, links and so on). They are shown in diff mode against the
selected base, and the dropdown shows that the current file is not one of the
listed files.

## Architecture

Diff mode has two layers below the views. It adds no new packages or
dependencies to the core or either frontend: the git source uses the
installed `git` program, and the diff algorithm is written in the core.

### Base sources

A base source supplies the base text for a document and hides which version
control system, if any, it comes from. For a document it reports which bases
are available and loads a chosen base as an immutable snapshot of bytes with
an identity. For a base it also lists the files that differ from it, which
fills the file dropdown. Refresh reloads both the base text and the file list.

Each toolbar entry is backed by a source:

- **Last saved** uses the saved copy Viem already keeps in memory to track
  unsaved changes. Refresh reads the file on disk instead.
- A **git** implementation provides Unstaged, HEAD and Branch.
- A **file** source reads a file chosen with Choose base file. Refresh reads it
  from disk again.

Other version control systems can be added as further implementations without
changing the diff layer or the toolbar.

Loading never blocks the UI. Sources run on workers, return immutable
snapshots, and install their results only if the window, file and base
selection still match; a stale or failed load changes nothing.

#### Git

The git source runs the `git` program directly rather than embedding a git
library. This is the first place Viem starts an external program, so it is a
deliberate exception to that policy, and it stays narrow:

- Run `git` with an argument list, never through a shell, from the file's
  repository.
- Use only read-only commands, with options that stop repository
  configuration from starting other programs (for example `--no-ext-diff`,
  `--no-textconv` and `core.fsmonitor=false`).
- Bound and cancel every invocation, and report a missing `git`, a file
  outside a repository or a failed command as an unavailable base rather
  than an error dialog.

Git bases apply git's line-ending conversion, so line endings that git
converts on checkout (`core.autocrlf`, `core.eol`, and the `.gitattributes`
`text` and `eol` attributes) do not show as differences, as with `git diff`.
Viem asks git for the file's attributes and settings and converts the base
itself. It does not use git's own filter pipeline, since clean and smudge
filters such as Git LFS run external programs.

### Diff computation

The diff layer compares the base snapshot with the live buffer and produces
the hunks the views draw. The new side is edited live, so the diff is updated
incrementally as changes happen rather than recomputed for the whole
document on each edit.

Edits tell the diff layer what the writer actually did, such as where text
was inserted. The incremental diff may use that knowledge, so it can differ
from a from-scratch diff of the same two texts. Reopening the document or
refreshing the base may therefore show a slightly different, equally valid
diff. Both must describe the two texts exactly: applying the hunks to the base
reproduces the buffer.

## Implementation status

This section is the hand-off point for resuming the work. Keep it current:
update it whenever a layer lands or a plan changes.

### Plan

Layers are built bottom-up; each has focused tests before the next starts.

1. **Diff computation** (`src/core/diff/`): line diff, inline (within-line)
   diff, the incremental diff state and per-side presentation. Pure logic with
   no document, command, layout or I/O dependencies.
2. **Base sources** (`src/core/diff/base/`): the source abstraction plus Last
   saved, chosen file and git implementations. Git runs the `git` program.
3. **Core integration** (`src/core/coordinator/diff.rs`, layout style
   cascade): per-view diff presentation installed like search matches. Diff
   lines substitute the paragraph style, changed spans overlay a character
   style, and aligned filler becomes paragraph margin. New Code styles in the
   default sheet and packaged themes.
4. **C ABI** (`include/viem_core.h`, `src/core/ffi/diff.rs`): diff sessions,
   base loading, file lists and line correspondence for scroll sync.
5. **macOS frontend**: Diff menu, diff toolbar, side-by-side panes, save
   prompt on navigation, settings persistence.
6. **Windows frontend**: the same behavior in WinUI.

### Progress

- [x] 1. Diff computation: `sequence.rs` (bounded Myers), `inline.rs`
  (word tokens narrowed to graphemes, bigram similarity for pairing lines),
  `state.rs` (`DiffState`: chunks, `apply_edit` windows, line
  correspondence), `presentation.rs` (per-side lines, spans, filler). Unit
  tests beside each file, including randomized edit sequences.
- [x] 2. Base sources: `base/mod.rs` (`BaseKind`, `BaseText`,
  `ChangedFile`, file bases, unsaved entries), `base/process.rs` (bounded,
  cancellable child processes), `base/git.rs` (`GitRepository`: discovery,
  index/HEAD/branch blobs, `git diff --name-status` lists, branches),
  `base/eol.rs` (checkout line endings). `Document::saved_source_bytes`
  returns the last loaded or saved source for Last saved; the document keeps
  it as a source snapshot shared with history, including after an
  asynchronous save, so it holds no flat copy. Git tests create temporary
  repositories and skip when `git` is missing.
- [x] 3. Core integration:
  - Styles: `StyleId::diff_added_line`/`diff_removed_line` (internal block
    styles "Added line"/"Removed line") and `diff_added_text`/
    `diff_removed_text` (internal character overlays "Added text"/"Removed
    text"), installed only in the default Code sheet
    (`code.rs::install_diff_styles`) with translucent green and red so they
    read in light and dark themes. Packaged themes inherit these defaults.
  - Layout: `layout/style.rs` `DiffLayoutInput` and `PresentationOverlays`;
    the cascade substitutes the diff line style for the paragraph (and the
    character context), adds filler to `margin_top`/`margin_bottom`, and
    overlays diff text after syntax and before search. The paragraph style
    cache keys on `DiffParagraphKey`. `ViewLayout::set_diff_presentation`
    invalidates only changed lines when told which.
  - Coordinator: `coordinator/diff.rs` `DiffSession` (`new`, `update`,
    `sync`, `set_aligned`, `corresponding_line`, `corresponding_top`,
    `clear`),
    `DocumentLines`, `Core::install_diff_presentation`,
    `Core::diff_filler_line_height`. `Document::layout_change_chain` feeds
    incremental updates; anything else recomputes. Tests in
    `coordinator/diff_tests.rs`.
- [x] 4. C ABI: `src/core/ffi/diff.rs` and the "Diff mode" block at the end
  of `include/viem_core.h`. Sessions (`viem_diff_session_create`, `_sync`,
  `_set_aligned`, `_corresponding_line`, `_corresponding_top`, `_info`,
  `_destroy`),
  `viem_diff_clear_view`, `viem_core_copy_saved_source`; owned results for
  worker-thread loads (`viem_diff_load_base`, `viem_diff_list_files`,
  `viem_diff_list_branches`, `viem_diff_result_*`); cancellation tokens
  (`viem_diff_cancellation_*`). Settings: `config.json` gains
  `diff.alignSides`, `diff.lineNumbers` (default true) and an optional
  `code.lineNumbers`. Tests: `src/core/ffi/tests/diff_tests.rs`; the ABI
  checker asserts the new constants and signatures.
- [~] 5. macOS frontend (built and covered by native tests; not yet tried
  by hand):
  - `src/mac/AppShell/EVDiffMode.swift`: protocols (`EVDiffBackend`,
    `EVDiffSessionControlling`, `EVDiffLineNumberControlling`), `EVDiffBase`,
    `EVDiffFile`, `EVDiffBaseLoader` (worker-thread C ABI calls),
    `EVDiffSplitView`, `EVDiffToolbar` (title-bar accessory: base popup,
    Choose Base File, Refresh, previous/next arrows around the file popup,
    Align Sides) and `EVDiffModeController` (per window: default base, file
    list, save review before moving files, line-number settings).
  - `EVDocumentWindowController.swift`: `EVDiffModeHost` conformance, Diff
    menu actions and validation, active-document hook, line-number toggles
    saved to `diff.lineNumbers` or `code.lineNumbers`.
    `EVMenuBuilder.swift`: the Diff menu before View.
    `EVConfigurationStore.swift`: diff and Code line-number settings.
  - `src/mac/Editor/Sources/EVCoreDiffMode.swift`: backend conformance
    (`makeDiffBase` builds a read-only, non-modifiable Code buffer with the
    document's language; `beginDiff`), `EVCoreDiffSession` (switches Preview
    or Text to Code and back, protects a deleted file's blank side, syncs in
    `noteSourceChange`, mirrors vertical scrolling while aligned). Mirroring
    asks the core for the other side's top (`_corresponding_top`: the line
    that corresponds to this side's top line, at the same fraction of it),
    because offscreen row heights are estimates and raw offsets drift.
    Ending a session restores the document's format through any open view
    of the document, since the compared pane may already have closed.
  - Tests: `src/mac/Editor/Tests/EVDiffModeTests.swift`;
    `EVMenuBuilderTests` checks the Diff menu's position and first item.
- [ ] 6. Windows frontend

### Next steps

1. Try the macOS app by hand (`make run`): Diff menu, toolbar, file
   navigation and its save review, Refresh, Choose Base File, Align Sides,
   line-number memory, and leaving diff mode from a Markdown document.
   Automated coverage: `scripts/test-mac.sh --filter 'EVDiffModeTests'`.
   Two `EVThemeTests` status-style tests fail independently of diff mode:
   commit 7a5971c raised the Midnight status size from 11 to 13 without
   updating their expectations. Every other XCTest and swift-testing test
   passes in a full `scripts/test-mac.sh` run. Two earlier full runs on
   2026-11-07 hung in the swift-testing phase inside Core Text font matching
   (a font-registry XPC reply that never arrived); later full runs of both a
   clean HEAD checkout and this branch completed, so that stall was
   environmental rather than caused by diff mode.
2. Apply the saved `code.lineNumbers` when a view enters Code outside diff
   mode (today it is applied when leaving diff mode and saved on explicit
   toggles).
3. Windows frontend (not started; needs a Windows host to build and test).
   `src/win/Interop/Native.g.cs` already has the diff ABI. Mirror the macOS
   split: a per-window controller in `src/win/Shell` (menu in
   `Shell/Menus.cs` before View, toolbar beside `Shell/FormattingToolbar*.cs`,
   layout in `Shell/EditorWindow.cs`, settings in `Shell/Preferences.cs`),
   and the base buffer plus session sync in `src/win/Core` (`CoreDocument.cs`,
   `CoreView.cs`) and `src/win/Editor/EditorPane.cs`. Base loads and file
   lists run on a worker and post results back with a generation check.
4. Record the deliberate exceptions in `AGENTS.md` once the feature ships:
   diff text styles stack on syntax styles, incremental diffs may differ from
   fresh ones, and the git source starts an external program.

### Implementation notes

- Code projections publish one block per hard line, so a diff line style can
  replace that line's paragraph style in the cascade without touching source.
- Search highlighting is the model for view-local presentation: matches are
  stored in `ViewLayout` keyed by document and revision, threaded into
  `DocumentLayoutStyles::resolve_region_for_presentation`, and included in the
  paragraph style cache key.
- The diff compares decoded hard lines, so line-ending differences between
  the base and the buffer never show as changes; git's checkout conversion
  still runs so the base pane's bytes match the working tree.
- Renames: the host loads a renamed file's base from `ChangedFile::base_path`.
  Copies are listed as added files whose base is the copy source.
- Listing changed files runs `git diff`, which may run clean filters the user
  configured (such as Git LFS) when it compares working-tree content. Base
  contents never do: they are read with `git cat-file blob`.
- Known cost: after an edit the core lays out the visible rows once without
  diff presentation (it is keyed to the previous revision) and again when the
  session installs the new one. Rebasing the presentation through the edit
  inside the core would remove the second pass.
- Filler height is the measured default Code row (`default_line_height` with
  the base paragraph's line spacing), so it is exact for uniform Code lines.
  A line that wraps to more rows on one side does not yet get extra filler on
  the other; scroll sync uses line correspondence to stay level instead.
- A session is created synchronously from both cores' current documents,
  so a very large initial diff blocks the calling thread within the work
  budget. Moving the first diff to a worker needs a snapshot-based create.
- `include/viem_core.h` and `src/core/ffi.rs` use CRLF line endings; edit
  them in binary mode (or with tools that preserve CRLF).
- `Document::layout_change_chain` exposes the bounded formatted change of
  recent transactions; the diff session uses it to update incrementally and
  falls back to a full diff for anything unbounded (history navigation,
  reprojection).

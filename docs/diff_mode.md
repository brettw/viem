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
**Diff mode**, is a checkmark item that turns diff mode on and off for the
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

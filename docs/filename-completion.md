# Filename completion

Ex file commands use Tab to select the first matching filename and cycle forward;
Shift-Tab selects the last and cycles backward. Completion matches the filename
prefix before the caret and preserves any text after it. Native command fields
and Vim key input share the same core implementation.

A unique directory is a completed path component. The next Tab discovers its
children, allowing `.age` → `.agents/` → `.agents/skills/` when each step has one
match. With multiple matches, Tab keeps cycling siblings even when the selected
match is a directory. Type `/` to accept that directory without adding a second
slash, then Tab to discover its children. Missing or empty directories leave
the draft editable and unchanged.

Ctrl-Y accepts the selected completion; Ctrl-E restores the prefix from the
current discovery step. After descending into a directory, Ctrl-E therefore
restores that directory, not the earlier abbreviated parent. Ordinary movement
and typing accept the selection before performing their normal action. Enter
executes the completed command. Completion itself never edits document content,
changes the working directory, or creates document undo entries.

## Comparison with zsh

Use zsh with `setopt menucomplete` and Emacs bindings as the traversal reference.
Bind Shift-Tab to `reverse-menu-complete` when auditing reverse cycling. The
[zsh completion guide](https://zsh.sourceforge.io/Guide/zshguide06.html) describes
menu cycling and directory-slash acceptance. Run comparisons in an isolated
temporary directory and inspect the command buffer without executing it.

The regression matrix covers:

| Situation | Expected traversal |
| --- | --- |
| Unique directory, including a dot-prefixed name | Next Tab discovers children |
| Several nested unique directories | Each Tab descends one component |
| Multiple files and directories | Cycle siblings, wrapping in either direction |
| Directory selected from multiple matches, then `/` | Accept the existing slash; next Tab discovers children |
| Already-entered directory ending in `/` | Discover children on the first Tab |
| Empty, missing, or removed directory | No change; later Tab can discover newly created entries |
| Relative `./` paths and directory symlinks | Preserve the entered parent spelling while descending |
| Spaces inside directory or file names | Preserve spaces throughout completion |
| Hidden entries | Require a dot prefix for the current component |

Viem retains these differences from shell completion:

- Matching and ordering are case-insensitive, preserving the actual filename
  spelling. zsh's default matching is case-sensitive.
- An Ex filename is one literal argument. Spaces need no shell quoting or
  backslashes; a unique file does not gain a trailing space or start completion
  of another argument on the next Tab.
- Ctrl-E and Ctrl-Y follow Viem's completion acceptance rules. Shell expansion,
  command-specific completion scripts, and glob patterns are not part of this
  filename completer.

Portable behavior tests live in `tests/all/filename_completion.rs`, discovery
tests beside `src/core/command/filename_candidates.rs`, and AppKit event routing
tests in `EVCommandPromptFilenameCompletionTests`. Run the portable traversal
tests with `python3 scripts/validate.py focused --filter filename_completion::`.

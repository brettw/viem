# Native selection and remaining gaps

Viem defaults to `autoselect`, with Vim's `keymodel` and `selectmode` empty.
Mouse selection gestures and Shift+navigation enter **SELECTION**: typing replaces the selected
text, Delete/Backspace enter Insert, and plain Left/Right collapse to the
corresponding edge. A selection started with `v`, `V`, or Ctrl-V keeps Vim Visual
behavior when extended with Shift+navigation or a pointer gesture.

Use `set noautoselect` to let Vim options govern entry. For example,
`set noautoselect keymodel=startsel,stopsel selectmode=mouse,key` enables Vim
Select for mouse and shifted navigation; adding `cmd` also changes
`v`/`V`/Ctrl-V entry. `gh`, `gH`, and `gCtrl-H` explicitly enter Vim Select
regardless of `autoselect`.

These options apply across existing and future documents in the running profile.
Changing them does not reclassify active selections or interrupt pending commands.
Put `set` commands in `startup.viem` to retain them across launches.

## Deliberate behavior differences

- A plain mouse click repositions the caret without leaving Normal, Insert, or
  Replace mode. Character-mode clicks target the character across its full
  width; Insert clicks still choose the nearest insertion boundary. A native
  drag begun on a character retains that whole initial grapheme in either
  direction, including after reversal; Insert-origin drags retain their chosen
  insertion boundary. On Windows, stationary pointer events and small click jitter
  do not start a selection; dragging begins after four layout units of movement.
- Settings > Editing > Caret hover effect defaults on. It previews the plain-click
  caret with a thin line or hollow block at 20% opacity in the theme caret color
  (Replace uses its underline). It hides at the current caret and while typing;
  the mouse must move beyond the drag threshold to show it again.
- Native selections are half-open insertion boundaries; Vim selections retain
  inclusive endpoints. Shift-Right at offset zero selects one character natively,
  but two with Vim `startsel`.
- Native Left/Right collapse without another movement. Vim `stopsel` stops
  selection and performs the motion. Without `startsel`, shifted horizontal
  arrows follow Vim word motions when `autoselect` is off.
- Native deletion enters Insert; Vim Select deletion returns to Normal. Ending
  a native selection otherwise resumes its originating Normal, Insert, or
  Replace mode. Escape retains its modal meaning.
- Native Selection bypasses Visual/Select mappings; insertion mappings still
  apply to replacement typing. Ctrl-G switches to Visual; Ctrl-O temporarily
  runs a Visual command.
- Native Copy preserves the selection, direction, active endpoint, caret, and
  mode. Vim `y` and explicit clipboard-register yanks retain their usual
  selection-ending rules.
- Double-click dragging extends by whole words using the same portable word
  classes as word selection. Reversing direction retains the original word;
  autoscrolling retains this granularity until mouse-up. A subsequent ordinary
  click or drag uses character boundaries again.
- Option-Up on macOS and Ctrl-Up on Windows move to the current paragraph's
  start, or the preceding paragraph's start when already there. Option-Down
  moves to the current paragraph's end, then successive paragraph ends;
  Ctrl-Down moves to the next paragraph's start. Shift extends the selection
  with the same boundaries. Semantic paragraphs include their internal hard
  breaks; soft wrapping and the active line-navigation policy do not change
  these destinations. Empty paragraphs remain navigation stops.

The references are [Vim Select mode](https://vimhelp.org/visual.txt.html#Select-mode)
and the [`keymodel`/`selectmode` options](https://vimhelp.org/options.txt.html).

## Native Paste

Command-V on macOS and native Paste on Windows follow gVim's clipboard `gP`
policy. With no selection, characterwise text inserts before the Normal caret
or at the insertion caret in Insert/Replace. Replace pastes insert text without
overwriting it. Linewise text inserts above the entire current hard/source line,
even when the caret is partway through it. An empty current line remains after
the pasted lines, and the caret follows the paste to column zero of that line.
Insert/Replace resumes its original mode. External text is linewise only when
it ends in CR/LF; Viem's private clipboard data can supply an explicit shape.

With a selection, native Paste uses Vim's change/delete-then-`gP` behavior and
finishes in Normal. Replacing a character selection with linewise text puts it
above the joined retained line. Replacing a line selection with linewise text
leaves an empty line after the inserted lines. The replacement is one atomic
edit and undo unit; deleted text goes to the small-delete register `-`, and the
clipboard is retained.

These rules come from Vim's [macOS mappings](https://github.com/vim/vim/blob/master/runtime/macmap.vim)
and [Paste helper](https://github.com/vim/vim/blob/master/runtime/autoload/paste.vim).

## Open work and policy choices

These differences are current limitations or possible native-editor refinements,
not a promise to implement every TextEdit convention. Changes need portable
policy and matching native input tests.

| Area | Current behavior | Follow-up |
| --- | --- | --- |
| Word movement | macOS Option-Left/Right and Windows Ctrl-Left/Right use Vim word boundaries. | Decide whether native navigation should distinguish word ends from next-word starts. |
| Windows triple-click | No dedicated line/paragraph selection gesture. | Add a gesture matching the macOS active line-policy behavior. |
| Native Cut mode | Native Cut uses Vim operators and can finish in Normal. | Decide whether native Cut should resume Insert. |
| Block clipboard edge cases | Replacing wrapped character selections with block data and multiline Paste during deferred block insertion are unavailable. | Prepare the changed geometry and a portable multiline block insertion contract. |
| Windows Ctrl-A | Routed to the Vim interpreter rather than native Select All; Select All is in the menu. | Any native Select All shortcut requires an explicit preference/product decision. |
| Rectangular IME | macOS stages block preedit locally; Windows rejects block composition. | Add a portable discontiguous composition contract before claiming parity. |

Regression entry points include
[selection options](../tests/all/ffi_selection_options.rs),
[macOS native selection](../src/mac/Editor/Tests/EVNativeSelectionTests.swift), and
[Windows input routing](../src/win/Diagnostics/SelectionInputTests.cs).

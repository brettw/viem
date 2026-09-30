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
  Replace mode. On Windows, stationary pointer events and small click jitter
  do not start a selection; dragging begins after four layout units of movement.
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

The references are [Vim Select mode](https://vimhelp.org/visual.txt.html#Select-mode)
and the [`keymodel`/`selectmode` options](https://vimhelp.org/options.txt.html).

## Open work and policy choices

These differences are current limitations or possible native-editor refinements,
not a promise to implement every TextEdit convention. Changes need portable
policy and matching native input tests.

| Area | Current behavior | Follow-up |
| --- | --- | --- |
| Word movement | macOS Option-Left/Right and Windows Ctrl-Left/Right use Vim word boundaries. | Decide whether native navigation should distinguish word ends from next-word starts. |
| Paragraph movement | macOS Option-Up/Down and Windows Ctrl-Up/Down move by rows, including Shift selection. | Add explicit paragraph-boundary navigation and selection. |
| Word drag | Double-click selects a word; dragging then extends by character. | Preserve word granularity throughout the gesture. |
| Windows triple-click | No dedicated line/paragraph selection gesture. | Add a gesture matching the macOS active line-policy behavior. |
| Clipboard edit mode | Native Cut and plain-text Paste use Vim operators and can finish in Normal. | Decide whether native replacement should resume Insert. |
| Windows Ctrl-A | Routed to the Vim interpreter rather than native Select All; Select All is in the menu. | Any native Select All shortcut requires an explicit preference/product decision. |
| Rectangular IME | macOS stages block preedit locally; Windows rejects block composition. | Add a portable discontiguous composition contract before claiming parity. |

Regression entry points include
[selection options](../tests/all/ffi_selection_options.rs),
[macOS native selection](../src/mac/Editor/Tests/EVNativeSelectionTests.swift), and
[Windows input routing](../src/win/Diagnostics/SelectionInputTests.cs).

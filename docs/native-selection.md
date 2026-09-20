# Native selection behavior

Viem defaults to `autoselect`, with Vim's `keymodel` and `selectmode` both
empty. Mouse and Shift+navigation selections enter the distinct **SELECTION**
state: typing replaces the selected text, Delete/Backspace enter Insert after
removing it, and plain Left/Right collapse to the corresponding edge. Selecting
with `v`, `V`, or Ctrl-V retains Vim Visual behavior, including when subsequently
extended with Shift+navigation or a pointer gesture.

Use `set noautoselect` to let the Vim options govern entry instead. For example,
`set noautoselect keymodel=startsel,stopsel selectmode=mouse,key` enables Vim
Select for mouse and shifted navigation. Adding `cmd` makes Vim `v`/`V`/Ctrl-V
enter Select too. `gh`, `gH`, and `gCtrl-H` explicitly enter Vim Select regardless
of `autoselect`. These states retain **SELECT**, **SELECT LINE**, or **SELECT
BLOCK** labels (Windows abbreviates the latter two).

`autoselect` supports `:set autoselect`, `noautoselect`, `autoselect?`,
`autoselect!`, `invautoselect`, and `autoselect&` (restore the enabled default).
All three options apply across existing and future documents in the same running
application profile. Put the desired `set` commands in `startup.viem` to retain
them across launches. Changing an option does not reclassify an active selection
or interrupt another view's pending command.

Native Selection uses half-open insertion boundaries; Vim selections retain
inclusive endpoints. For example, Shift-Right at offset zero selects the first
character natively, but the first two characters with Vim `startsel`. Vim
`stopsel` stops selection and performs the motion, whereas native Left/Right
collapse without an extra step. Vim Select Delete returns to Normal. Without
`startsel`, shifted horizontal arrows follow Vim word motions when `autoselect`
is off. Native Selection bypasses Vim Visual/Select mappings; insertion mappings
still apply to replacement typing. Ctrl-G explicitly switches to Visual, and
Ctrl-O temporarily runs a Visual command; Copy restores the originating policy.

Tests cover both policies, every keymodel/selectmode entry combination, option
propagation, mappings, counts, registers, Unicode boundaries, reversed ranges,
undo, dot-repeat, block selections, IME, and native status/clipboard presentation.
The semantics reference is [Vim 9.2 Select mode](https://vimhelp.org/visual.txt.html#Select-mode)
and the [`keymodel`/`selectmode` option reference](https://vimhelp.org/options.txt.html).

## Potential followups

| Area | Current behavior | Native-editor difference / proposed followup |
| --- | --- | --- |
| Word movement | Option-Left/Right on macOS and Ctrl-Left/Right on Windows use portable Vim word boundaries. | An AppKit `NSTextView` probe on macOS 26 moves Option-Right from the beginning of `one two` to offset 3; Viem moves to the next word at offset 4. Consider separate native word-end and word-start intentions. |
| Paragraph movement | Option-Up/Down on macOS and Ctrl-Up/Down on Windows currently use row movement, including Shift selection. | Add explicit paragraph-boundary navigation and selection commands. |
| Word drag | Double-click selects a portable word; subsequent dragging extends by character. | Native editors can retain word granularity while dragging. Preserve the pointer gesture's granularity in the core. |
| Windows triple-click | Windows has no dedicated triple-click line/paragraph selection. | Add a native gesture matching the documented macOS line-policy behavior. |
| Mode after clipboard edits | Native Cut and plain-text Paste use Vim operators, which can finish in Normal mode. | Consider resuming Insert after native clipboard replacement, so the next printable key always types text. |
| Windows Ctrl-A | The shortcut retains Vim's increment-number command; Select All is available from the menu. | A native shortcut preference could map Ctrl-A to Select All. |
| Rectangular IME | Character/line selection supports composition overlays. macOS stages Select Block preedit locally and commits through block replacement; Windows declines block composition. | Add a portable discontiguous composition contract and matching native tests before claiming parity. |

These are selection and navigation differences, rather than a claim that Viem
implements every TextEdit editing convention. Vim's Normal/Visual command model
and platform-specific shortcuts remain deliberate product behavior. Ending a
selection that began in Normal mode resumes Normal; selections begun in Insert
or Replace resume that mode. Escape retains its modal meaning.

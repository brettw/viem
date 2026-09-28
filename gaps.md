# Viem coverage and port audit

Audited against `main` at `3650197`. Command coverage was read from the dispatch
tables in `src/core/command/`; interface findings were traced through
`include/viem_core.h`, `src/core/ffi.rs`, and `src/mac/`. Line counts are
physical lines including comments. The baseline metrics below are retained;
subsequently completed regex, startup/mapping, search-presentation, and Ex-command
findings have been removed.

This document reads the shipped code, not `AGENTS.md`. Where the two disagree,
the finding describes the code.

| Measure | Value |
| --- | ---: |
| Rust core (`src/core`) | 191,018 lines |
| Swift, non-test (`src/mac`) | 25,347 lines |
| C ABI | 128 functions, 101 structs, 491 constants, version 5 |
| Test files | 280 (162 Rust, 118 Swift) |

## Contents

- [Summary](#summary)
- [1. Deliberate non-goals](#1-deliberate-non-goals)
- [2. Normal mode](#2-normal-mode)
- [3. Visual modes and text objects](#3-visual-modes-and-text-objects)
- [4. Search](#4-search)
- [5. Ex commands](#5-ex-commands)
- [6. `:set` options](#6-set-options)
- [7. Registers and marks](#7-registers-and-marks)
- [8. The frontend interface](#8-the-frontend-interface)
- [9. Port budget](#9-port-budget)
- [10. Recommended order](#10-recommended-order)

## Summary

Two things frame everything below.

**The architecture is sound and the boundary is real.**
`tests/architecture_boundaries.rs` mechanically asserts that `src/core` imports
no AppKit, Core Text, or Metal, that the document model never reaches into the
command controller, and that the controller never touches private document
storage. Every mutating ABI call carries an exact document/revision/layout
identity and fails closed on a stale one. Key aliasing (`Ctrl-H` to Backspace in
Insert, to Left in Normal; physical Tab to `Ctrl-I`) lives in the core, in
`src/core/command/input_keys.rs:52`, which is exactly right. Undo is a real
branching tree with node selection by change number.

**What leaked into the frontend is prose and policy, not state.** The frontend
keeps no second document, selection, or undo model; that discipline held. It
does compose every user-visible string, decide menu availability, synthesize Vim
keystrokes, and own caret shape, decoration geometry, the theme, and the config
schema. That is the part a C# frontend would build twice, and the part where
macOS and Windows would silently drift apart.

There is a matching gap in the test suite: the boundary test polices the core's
purity in one direction only. Nothing asserts that the frontend contains no
portable policy, which is precisely the rule the findings in section 8 break.

## 1. Deliberate non-goals

Listed so the gaps that follow read as gaps rather than as settled decisions.
`AGENTS.md:4701` places these outside the command commitment:

Vimscript/Vim9script, abbreviations, plugins, terminal jobs,
shell filters and `:!`, tags, quickfix, diff mode, folding, spellchecking, tab
pages, side-by-side splits, sessions/viminfo, remote server commands, and full
option/regex parity.

Normal-mode `U` is deliberately unsupported; see [AGENTS.md](AGENTS.md).

Viem Regex v2 is a specified dialect, not a shortfall. Two absences will still
be felt weekly by a Vim user: `\zs`/`\ze` match trimming and `\c`/`\C` inline
case switches. Scoped `(?i)`/`(?-i)` flags cover case switching. Capture groups
cover some replacement uses of match trimming, but do not change the overall
match extent, search destination, or highlighting as `\zs`/`\ze` do.

## 2. Normal mode

Coverage is broad and the grammar is complete: counts, registers,
operator-pending forms, doubled operators (`dd`, `gqq`, `gqgq`), `g`-prefixed
operators, and dot-repeat all work.

**Implemented.** `d c y > < = i I a A o O R x X s S D C Y p P J r ~ . u C-r`,
`h j k l w W e E b B 0 ^ $ | + - G % ( ) { } f F t T ; , H M L C-o C-i`,
`C-f C-b C-d C-u C-e C-y C-v C-q`, `` " m ' ` q @ v V / ? : n N * # ``,
`gg ge gE g_ gj gk g0 g^ g$ gv gJ gp gP g* g# gq gw gu gU g~`, `zt zz zb`,
`ZZ`, and the full `C-w` window family.

**Missing, in rough order of value:**

### `C-a` / `C-x`

Increment and decrement the number under the cursor. No trace anywhere in the
core. Self-contained, needs no frontend involvement at all, and among the
handful of commands a Vim user reaches for without thinking. Highest
value-per-line in this document.

### The `[` and `]` families

There is no bracket-prefix pending state at all, so `[[`, `]]`, `[{`, `]}`,
`[(`, `])`, and `[m`/`]m` are all absent. The editor already has bracket-pair
resolution for `%` and for the `i(`/`i{` text objects
(`src/core/command/text_object.rs:541`), so most of the machinery exists; only
the prefix and the motion wiring are missing.

### `ga`

Show the codepoint under the cursor. Worth more here than in Vim: this is an
editor that models graphemes, bidi, shaping clusters, four encodings, and BOM
state, and gives the user no way to ask "what character is this?" A natural
companion to the visible-whitespace work already shipped.

### `ZQ`

The `Z` prefix exists and is handled at `src/core/command/mod.rs:4255`, but only
`ZZ` is recognised; `ZQ` falls through to `unsupported`. A two-line fix for a
command people type by reflex.

### Lower priority

- `gi` (resume insert at the last insert position) and `gI`.
- `gn`/`gN` (operate on next match).
- `g;`/`g,` need a change list, which does not exist. The jump list does, so the
  shape is known.
- `&` and `g&` in Normal mode. Ex `:&` exists; the Normal-mode keys do not.
- `z` family beyond `zt zz zb`: `z.`, `z-`, `z<CR>`, `zh`, `zl`, `zH`, `zL`,
  `ze`, `zs`.
- `gm`, `gM`, `go`, `g?`, `g8`, `K`, `C-^`, `C-g`, `C-l`. Low value here.

## 3. Visual modes and text objects

### Visual Block is the strong one

Blockwise gets `I A c s r y d x ~ u U p P J > < =`, endpoint swap, and genuine
ragged-right `$` via `resolve_block_selection_to_line_end`
(`src/core/command/visual_block.rs:261`). More careful than most Vim emulations
manage.

### Charwise and linewise Visual are missing every uppercase operator

The dispatch at `src/core/command/mod.rs:9196` handles
`y d x c s > < = ~ u U p P J` and nothing else. Missing: **`D C S X Y R`**.

All six are pure muscle memory (`V` then `D`, or `Vj` then `Y`) and today
produce an error. Each is a linewise alias for an operator that already exists,
so this is a small, high-return patch.

### Text objects: one notable hole

Implemented: `` w W s p ( ) b [ ] < > { } B " ' ` ``, with real care taken over
escaping, decimal points inside sentences, East Asian terminators, and never
splitting a grapheme.

Missing: **`it` and `at`**, the tag-block objects. HTML files now use literal
Code mode, so these would require a source-oriented tag-object implementation.
The passive HTML parser used by Markdown and clipboard import is not an editing
model for Code files.

## 4. Search

- **History is unfiltered.** `navigate_history`
  (`src/core/command/mod.rs:15441`) steps through the whole list. Vim filters
  Up/Down by the prefix already typed and leaves `C-p`/`C-n` unfiltered. Today
  both behave like the latter.
- **`"/` and `":` are half-registers.** Both are readable from the prompt via
  `C-r` (`src/core/command/command_line_register.rs:65`) but neither is an
  addressable register, so `"/p` in Normal mode fails.
- **No search-count report.** Vim's `[3/17]` indicator has no equivalent, and
  the status line has no field for it.

## 5. Ex commands

The remaining gaps are narrower than the original command-family omissions:

- **Interactive confirmation inside `:global` is unsupported.** Global
  execution also rejects host/file commands and history navigation. Ordinary
  editing, `:normal`, printing, and nested current-line global predicates are
  supported; command pipelines are still outside the supported grammar.
- **Confirmation prompt scrolling is missing.** `:s///c` accepts
  `y`/`n`/`a`/`q`/`l`, Escape, and Ctrl-C, but not Vim's Ctrl-E/Ctrl-Y scrolling.
- **Shell and key-script file commands remain unsupported.** `:read !command`
  and `:source!` are not implemented. `:source` handles supported Ex commands
  and mappings, not a general Vimscript runtime.

## 6. `:set` options

Supported options have full Vim `set` grammar (`no`, `!`, `?`, `&`, `<`, `+=`,
`^=`, `-=`) and a real global/local split. Implemented: `wrap`, `ignorecase`,
`smartcase`, `wrapscan`, `hlsearch`, `incsearch`, `textwidth`, `fileformat`, `fileformats`, `autoindent`,
`tabstop`, `shiftwidth`, `softtabstop`, `expandtab`, `smarttab`, `list`,
`listchars`, plus two comment-continuation options.

The interesting finding is not which options are missing, but that three are
already fully implemented and simply not reachable from `:set`.

| Option | Already in the ABI |
| --- | --- |
| `linebreak` | `viem_core_view_set_linebreak` (`viem_core.h:2293`) plus a `VIEM_VIEWPORT_STATE_LINEBREAK` flag. |
| `fileencoding` / `bomb` | `viem_core_view_set_encoding_with_effects`, four encodings, `VIEM_DOCUMENT_STATE_HAS_BOM`. Exposed in the Format menu but not from `:set`. |
| `readonly` | `viem_core_set_read_only` and a dedicated `VIEM_COMMAND_STATUS_READ_ONLY`. |

These are wiring, not features. They also fix an asymmetry a user will notice:
the same setting behaves differently depending on whether you reach it from the
menu or the colon prompt.

**Not implemented:** `number`, `relativenumber`,
`scrolloff`, `joinspaces`, `gdefault`, `iskeyword`, `whichwrap`, `startofline`,
`report`, `formatoptions`, `comments`, `undolevels`, `selection`, `clipboard`,
`wrapmargin`, `modifiable`.

Of these, `scrolloff` and `joinspaces` are the cheapest wins and both are felt
constantly. `iskeyword` matters more than it looks: word motions, `*`,
`C-r C-w`, and manual completion all pin the keyword class to
Unicode-alphanumeric-or-underscore with no way to adjust it.

## 7. Registers and marks

### Registers

Named `a`-`z` with `A`-`Z` append, numbered `0`-`9` with correct Vim rotation
and the within-line delete exception, small-delete `-`, black hole `_`, unnamed
`"`, system `+`/`*`, last-insert `.`, filename `%`
(`src/core/command/registers.rs:757`). Blockwise registers round-trip. This is
thorough.

Missing: `/` and `:` as addressable registers (see section 4), `#` alternate
file, `~` drop register. `=` is explicitly refused, correctly; it needs an
expression evaluator.

### Remaining special marks

Local `a`-`z` and remembered Visual `'<`/`'>` marks are implemented, including
Ex addresses, jumps, and deletion. Still missing: `'[`/`']` (last change or
yank), `'.` (last change), `'^` (last insert), `''` and the doubled backtick
(previous position), and `A`-`Z` cross-file marks.

Marks use persistent text anchors, remap through source provenance across
edits, and participate in undo restoration.

## 8. The frontend interface

Findings are ordered by what they will cost once a C# frontend exists: by how
much product behaviour has to be re-derived from reading Swift, and how likely
the two platforms are to drift apart afterwards.

### B1. Every user-visible string is composed in the frontend — high

`displayMessage(for:)` at
`src/mac/Editor/Sources/EVEditorSurfaceController.swift:1574` renders `:marks`,
`:registers`, `:jumps`, `:set`, and `:s` print output in Swift: the column
headers (`"mark  line  col  text"`), the `^I`/`^J` escaping, the `no` prefix for
off booleans, the `name=value` form. Option names are re-spelled there too, two
of them by bare integer (`case 5: "ignorecase"`) because no constant was
exported for them. The status line composes its own mode labels and
`"Ln %@, Col %@"` at `:1288`.

The errors are worse. The core produces specific, well-typed failures
(`CommandStatus::Error(String)`, `CountError`, `RegisterReadError`,
`RegisterWriteError`, `ExError`, `VisualBlockError`) and the ABI discards all of
it: `src/core/ffi.rs:4281` collapses six variants into
`VIEM_COMMAND_STATUS_ERROR`, and `ViemCoreOutcomeV1` has no message field at
all. So the user sees `"... was rejected (Viem command status 7)"`, and the one
real Vim message in the product, `E45: readonly option is set (use ! to
override)`, is a string literal in Swift at
`src/mac/Editor/Sources/EVCoreDocument.swift:35`.

That is also why Vim's E-numbers are otherwise absent: there is nowhere for them
to live.

**Suggested shape.** Add a message channel to the outcome
(`viem_core_view_copy_last_message`, or a slice in a V2 outcome struct) and move
formatting into the core so it emits finished strings. Every E-number then lands
in one place, the Windows build gets identical text for free, and this stops
being a source of divergence before it becomes one.

### B2. Menu commands are implemented by synthesizing Vim keystrokes — high

`EVMenuCommand` declares 127 cases. `perform(menuCommand:)` executes two dozen
of them by replaying key sequences into the core:
`sendNormalSequence(["v","i","w"])` for Select Word, `["g","0","v","g","$"]` for
Select Visual Row, `["/"]` for Find, `["n"]` and `["N"]` for Find Next and
Previous. Alongside it, `presentation(for:)` re-derives enablement across
roughly forty cases from mode and format state (`isEnabled: isVisualMode`,
source-format lists, and so on).

This is Vim grammar and command-availability policy living in the view layer.
Both have to be rebuilt in C#, and the second implementation is where menu
behaviour quietly diverges.

**Suggested shape.** One core-owned command-id enum, with
`viem_core_view_perform_command(id, ...)` and
`viem_core_view_command_presentation(id) -> {enabled, checked, title}`. The core
already holds every input to both answers. The frontend's remaining job is
hanging ids on native menu items, which is genuinely platform work.

### B3. `viem_effect_batch_copy` takes 22 parameters — high

Ten `(array, capacity)` pairs plus handle and out-info (`viem_core.h:2156`),
all-or-nothing, with required counts returned on `BUFFER_TOO_SMALL`. On the
Swift side that is a ten-deep `withUnsafeMutableBufferPointer` pyramid at
`src/mac/Editor/Sources/EVCoreHostEffects.swift:283`. In C# it becomes ten
nested `fixed` blocks or ten pinned handles. Adding an eleventh effect kind
changes the signature, the ABI version, and both frontends.

**Suggested shape.** One serialized payload: either
`viem_effect_batch_copy_json` or a single byte buffer with a typed header and
per-section offsets. The precedent is already in this ABI:
`viem_core_copy_clipboard_json`, `viem_effect_batch_copy_clipboard_json`,
`viem_code_export_style_json`, and `viem_core_copy_syntax_style_names` all work
this way. New effect kinds then become data, not signature changes. Keep the
typed structs for layout and paint, where the per-frame cost is real; effects
fire once per command turn.

### B4. Caret and decoration appearance are decided in the view — medium

`ViemViewPresentationV1` publishes only `VIEM_CARET_SHAPE_CELL` and
`_BOUNDARY`. Everything else is Swift: `drawCustomCaret` and
`inactiveCaretRect` (`src/mac/Editor/Sources/EVEditorView.swift:3020`) decide
that Replace draws a 2px underline, Insert a 2px bar, Normal and Visual a filled
cell, and inactive an outline, plus a colour-glyph special case and a
glyph-redraw pass. Blink timing is a separate 273-line controller.

Underline and strikethrough geometry are magic numbers in the same file
(`:2869` and `:2879`): `baseline + max(1, descent * 0.35)` and
`baseline - ascent * 0.32`, one pixel thick. The core owns fonts, metrics, and
shaping, and hands the frontend a flag; the frontend invents the position.

**Suggested shape.** Add `VIEM_CARET_SHAPE_UNDERLINE` and let the core pick
shape from mode; publish blink phase and interval as core state; return
decoration rectangles from the paint snapshot the same way cluster bounds
already come back. Then "does Windows underline in the same place" stops being a
question anyone has to answer by eye.

### B5. The command line re-implements text motion, with string semantics that will not survive the port — medium

`moveCommandLineSelection` (`src/mac/Editor/Sources/EVEditorView.swift:1585`)
computes prompt cursor movement with Swift's `String.index(before:)` and
`index(after:)`, and `performCommandLineMenu` does cut, copy, paste, and
select-all against Swift String indices. Meanwhile the core already owns the
prompt: it validates every offset as a grapheme boundary in
`src/core/command/command_line_edit.rs:48`. The frontend is computing the target
that the core is about to check.

The portability failure here is concrete rather than theoretical. Swift's
`String.Index` steps by grapheme cluster; C#'s `string` steps by UTF-16 code
unit. Port this method as written and the arrow keys walk into the middle of an
emoji on Windows, then hand the core an offset it rejects — a bug that will not
reproduce on the machine it was written on.

**Suggested shape.** `viem_core_view_command_line_motion(kind, extend)`. The
frontend sends intents; it never computes an offset of its own.

### B6. Key routing policy lives in `keyDown` — medium

`EVEditorView.keyDown` (`:1280`) encodes a precedence ladder: literal input
first, then editor shortcuts, then Command-modified keys, then active
composition, then control characters, then special keys, then
`interpretKeyEvents`. It also folds control codepoints itself at `:1373`:
`native < 0x20 ? native + 0x40 : native == 0x7F ? 0x3F : scalar`. That ladder
and that fold are the difference between `C-@`, `C-2`, and `C-?` working or not,
and both are portable policy sitting in an `NSView` subclass.

Dedicated keypad input kinds are still absent.

**Suggested shape.** Move the control-codepoint fold and the literal-input
precedence rule into the core as a documented routing contract.

### B7. Model rules re-derived in Swift — medium

- **Style inheritance legality.** `legalParents`, `rolesCanInherit`, and
  `isDescendant`, including cycle detection, at
  `src/mac/Editor/Sources/EVCoreStyles.swift:287`. These are model invariants
  the core enforces on write; the frontend re-derives them to populate a picker.
  Divergence means one platform offers a parent the other rejects.
- **The theme.** `src/mac/AppShell/EVTheme.swift` owns the colour model, both
  built-in presets, and the validation ranges. Selection colour, status colours,
  and status font all come from there, not the core, so they are invented twice.
- **Config schema and defaults.** `src/mac/AppShell/EVConfigurationStore.swift`
  owns the `config.json` shape, every default, and incidental policy like the
  ten-entry recents cap. A second frontend means a second implementation of one
  filename.

**Suggested shape.** For styles,
`viem_core_style_relation_candidates(...)`. For theme and config, move the
schema, defaults, and validation into the core — which already parses JSON for
indentation, whitespace presentation, and filename associations — leaving the
frontend file I/O and the native colour picker.

### B8. Status-line state the core has but does not publish — low, cheap

`is_recording_macro()` exists at `src/core/command/mod.rs:2592` and is not
exposed through the ABI, so the status line cannot show `recording @q`. Nor is
the pending command text (Vim's `showcmd`, watching `"a2d` accumulate) or the
active count. `ViemViewPresentationV1` is `struct_size`-versioned, so all three
are additive.

### B9. Bindings are hand-maintained, and a third copy has no safety net — do this first

`include/viem_core.h` is written by hand: 128 exported functions, 101 structs,
491 constants, no cbindgen. There is a check —
`tests/ffi_core_surface.rs:4664` compiles a C file full of `_Static_assert`s
against the Rust layouts — but it is a hand-curated list, so a new struct is
unverified until somebody remembers to add it.

A C# binding would be a third hand-maintained copy of the same layouts with no
check at all. In Swift, a mismatched struct is usually caught by the C importer
reading the same header. In P/Invoke, a wrong `[StructLayout]` offset is not a
compile error; it is silent memory corruption, and it will surface as an
unreproducible crash far from the cause.

**Suggested shape.** Generate the header from Rust, or generate the assert list
exhaustively so every struct is covered by construction, then generate the C#
`[StructLayout]` definitions from that same source. This is the single
highest-leverage thing to do before any C# is written, because it is the only
finding here whose failure mode is silent.

### B10. The measurement provider is where the schedule risk is

Not a defect, a warning. `ViemTextMeasurementProviderV1` is a callback table the
core calls *into*: `shape_batch` may run on a worker thread under
`VIEM_PROVIDER_THREADING_ANY_WORKER`, responses return pointers the provider
must keep pinned until the next call, and `release_render_runs` may run on any
thread and after view detach. The macOS implementation is about 1,860 lines of
Core Text.

For C#: reverse P/Invoke from a native worker thread needs
`UnmanagedCallersOnly` or permanently rooted delegates, and the pinned-response
contract means response buffers must be native memory rather than managed
arrays. `VIEM_PROVIDER_THREADING_FRONTEND_MAIN` is available as an escape hatch
but serializes shaping onto the UI thread. The sane order is to write the
DirectWrite provider first, against the existing Rust provider tests, before any
Windows UI exists.

## 9. Port budget

25,347 lines of non-test Swift, split by whether a second frontend has to
re-derive the behaviour or merely re-express it in another UI toolkit.

| Category | Approx. lines | What it is |
| --- | ---: | --- |
| Genuinely platform work | ~21,500 | AppKit views and drawing, `NSTextInputClient` and IME, accessibility, window and pane lifecycle, `NSDocument`, the style and settings panels, and ~1,860 lines of Core Text shaping. All of this has a WPF/WinUI plus DirectWrite equivalent that must be written regardless. No finding here. |
| Portable policy currently in Swift | ~3,800 | Findings B1-B8: Ex and status-line string composition, the error message table, menu command synthesis and enablement, caret and decoration geometry, command-line motion, key routing precedence, style-inheritance legality, and the theme and config schemas. |

That ratio is the whole argument. Fifteen percent of the frontend is the
difference between the port being "build a Windows UI against a documented core"
and "build a Windows UI and reconstruct the product's behaviour by reading
Swift." Moving it into the core before starting also means the macOS app gets
tested against the new interfaces first, while there is still only one frontend
to fix.

One structural suggestion to lock it in: add a boundary test in the mirror image
of `tests/architecture_boundaries.rs` — an assertion over the frontend sources
that no user-visible string, no Vim key sequence, and no option name appears
outside the core. It is a blunt instrument, but the existing test proves the
pattern works on this codebase, and it is the rule the eight findings above were
quietly violating.

## 10. Recommended order

Ordered by cost of delay. Everything in the first four gets materially more
expensive once a second frontend exists.

1. **Generate the bindings (B9).** Header from Rust, or exhaustive layout
   asserts, plus generated C# structs from the same source. The only finding
   whose failure mode is silent memory corruption. Do it before writing any C#.
2. **Move display strings and errors into the core (B1).** Biggest divergence
   risk, and it simultaneously gives the product the Vim error messages it
   currently has nowhere to put.
3. **Core-owned command ids for menus (B2).** Deletes 127 cases of Vim-grammar
   knowledge and roughly 40 enablement rules from the frontend.
4. **Replace the 22-argument effect copy (B3).** One serialized payload,
   following the JSON precedent already in the ABI. New effect kinds stop being
   ABI changes.
5. **Command-line motion and remaining key-routing policy into the core
   (B5, B6).** Small, and B5 removes a bug that would only ever appear on
   Windows.
6. **The cheap command fills (sections 2, 3, 6).** `C-a`/`C-x`, Visual
   `D C S Y X R`, `ZQ`, `it`/`at`, `ga`, and wiring `linebreak`,
   `fileencoding`, and `readonly` to `:set`. Each is hours, not days.

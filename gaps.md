# Viem coverage and interface gaps

Reaudited on 2026-10-19 against the current checkout at `1a83642`. Findings
below were checked against command dispatch, native callers, the ABI and tests.
This is an unresolved backlog; historical line counts and port estimates are
omitted. Windows already has a WinUI/DirectWrite frontend.

This audit covers the existing command and interface backlog and adjacent
defects; it is not a complete Vim conformance inventory. Normal `U` and the
Viem Regex v2 dialect are documented product differences. Shell commands/filters,
`:source!`, terminal jobs and automatic plugin/runtime loading remain outside
the script host profile. See [AGENTS.md](AGENTS.md),
[Vim script support](docs/vimscript.md) and [split views](docs/window-panes.md)
for the current commitments.

## 1. Normal mode

No known gaps remain in the audited Normal command list.

## 2. Visual modes and text objects

No known gaps remain in the audited Visual command and text-object list.

## 3. Search and prompts

No known gaps remain in the audited search and prompt list. Windows prompt
keyboard selection is covered separately in B5 below.

## 4. Ex commands

- **Overlapping confirmed substitutions inside `:global` remain unsupported.**
  Confirmation now supports disjoint per-target substitutions, inherited flags,
  nested current-line predicates and Ctrl-E/Ctrl-Y scrolling. All choices stay
  bound to the original revision and commit in one undo unit. A body such as
  `:g/x/%s/x/xx/gc` revisits matches affected by earlier decisions, requiring
  sequential previews against tentative results; it is rejected before asking
  questions. For example, with two original lines containing `x`, accepting the
  first whole-buffer substitution changes both to `xx`; the second selected
  line's invocation must then offer the newly created matches. Merging previews
  computed from the original text cannot reproduce that behavior.

  Deferred design questions (no implementation is planned in this change):
  - How should each invocation preview the previous invocation's accepted edits
    in an isolated candidate while keeping the original `:global` target lines
    anchored through inserted, deleted or split lines?
  - How should the confirmation highlight, cursor and Ctrl-E/Ctrl-Y scrolling
    use layout for that candidate without publishing tentative source to other
    views? Original-revision layout cannot locate newly created matches.
  - How should `a`, `q`, `l`, Escape, Ctrl-C and per-target failures retain the
    appropriate completed decisions across invocations? The existing rules for
    per-substitution responses, global partial progress and final atomic
    publication need explicit tests; an external edit must cancel stale work.
  - How can candidate source, projections, layout and repeated match growth be
    bounded and cancellable while preserving exact bytes and one-step undo/redo?

  Any implementation must reconcile sequential candidate revisions with the
  current original-revision confirmation contract before lifting the rejection.
  Cover overlapping explicit/count ranges, nested predicates, zero-width and
  newly created matches, Markdown, mixed encodings/endings and multiple views.
  See [global confirmation](src/core/command/global_confirmation.rs) and
  [confirmation tests](tests/all/substitute_confirmation.rs).

## 5. `:set` options

No known gaps remain in the audited option list. Deliberately unsupported
options are recorded in [the option profile](docs/vimscript.md#deliberately-unsupported-options).

## 6. Registers and marks

- **`/` and `:` are not generally addressable registers.** Prompt Ctrl-R and
  script register reads can access the last search/Ex command, but
  [`is_valid_register`](src/core/command/registers.rs) excludes both names.
  Normal `"/p` and `":p` therefore fail.
- **Other missing registers:** `#` (alternate file), `~` (drop) and `=`
  (expression). The expression register is explicitly rejected by the prompt
  handler and is not accepted by Normal register selection. A script expression
  evaluator now exists, but is not wired to expression-register input.
- **Script reads lose the current filename (new).**
  [`script_register`](src/core/command/scripting.rs) returns an empty string
  unconditionally for `%`. In a document bound to `/tmp/gaps-audit.txt`,
  `:echo '[' . @% . ']' '[' . getreg('%') . ']'` prints `[] []`, while prompt
  `<Ctrl-R>%` inserts `/tmp/gaps-audit.txt`. Script reads should use the current
  artifact context as the other filename-register paths do.
- **Special marks are missing:** `'[`/`']` (last change or yank), `'.`
  (last change), `'^` (last insert), `''` and doubled backtick (previous
  position), and `A`-`Z` cross-file marks. The mark handlers in
  [`command/mod.rs`](src/core/command/mod.rs) accept local `a`-`z` and remembered
  Visual `<`/`>` marks only.

## 7. Shared frontend and ABI issues

These include observed platform differences and remaining interface maintenance
concerns. Native dialogs, event decoding and native caret blinking belong in the
frontends; their presence alone is not a policy defect.

### B1. Command diagnostics and Ex output still differ across the boundary

Non-Ex error details are lost in
[`command_status_to_ffi`](src/core/ffi.rs): count, register, general command and
Visual Block failures become a generic error status without their message.
`Unsupported` likewise exports only a status. Ex errors already use a message
effect through `OwnedEffectBatch::from_command`; the remaining error families
need equivalent detail.

Ex output is independently composed in
[Swift `displayMessage`/`formatOption`](src/mac/Editor/Sources/EVEditorSurfaceController.swift)
and [C# `HostEffects.Read`/`OptionName`](src/win/Core/HostEffects.cs), with
observable differences:

- Windows renders boolean options as `wrap=1`; macOS renders `wrap`/`nowrap`.
- Windows renders `fileformat` numerically and lacks a `fileformats` name-table
  entry; macOS renders the named formats.
- Windows prints signed `softtabstop` values through the unsigned scalar field;
  macOS decodes them as signed.
- Windows inserts raw register controls into output; macOS escapes them visibly.

[Windows command-status tests](src/win/Diagnostics/CommandStatusTests.cs)
currently expect `wrap=1`, while
[macOS host-effect tests](src/mac/Editor/Tests/EVHostEffectsIntegrationTests.swift)
expect named file formats and escaped register newlines. Shared formatting would
prevent these differences from being maintained as separate expectations.

### B2. Native actions still synthesize Vim grammar

[macOS menu dispatch](src/mac/Editor/Sources/EVEditorSurfaceController.swift)
and [Windows menus](src/win/Shell/Menus.cs) implement selection and find actions
with sequences such as `viw`, `g0vg$`, `/`, `n` and `N`. Enablement is also
re-derived by each frontend. The
[Windows wrappers](src/win/Core/CoreView.cs) add their own selection/return-mode
handling. Core command intentions and availability queries would let both
frontends invoke the same action without duplicating pending-command and mode
policy.

### B3. Effect copying still requires ten parallel buffers

[`viem_effect_batch_copy`](include/viem_core.h) takes 22 parameters: ten
array/capacity pairs, a handle and out-info. This produces ten nested buffer
scopes in [Swift](src/mac/Editor/Sources/EVCoreHostEffects.swift) and ten nested
`fixed` scopes in [C#](src/win/Core/HostEffects.cs). Adding another payload array
requires coordinated ABI and caller changes. This remains an interface
maintenance concern, rather than a demonstrated correctness failure; a single
payload or descriptor could reduce that coupling.

### B4. Text decoration positions differ between frontends

[macOS drawing](src/mac/Editor/Sources/EVEditorView.swift) positions underlines
at `floor(baseline + max(1, descent * 0.35))` and strikethrough at
`floor(baseline - ascent * 0.32)`.
[Windows drawing](src/win/Editor/DrawingCache.cs) uses `baseline + 2` and
`baseline - ascent * 0.3`. The same shared style therefore has independently
chosen decoration geometry. There is no shared position contract or returned
geometry for these text decorations.

### B5. Windows prompt Shift navigation does not extend selection (new)

[Windows key policy](src/win/Input/KeyPolicy.cs) preserves Shift on
Left/Right/Home/End, and
[`EditorPane`](src/win/Editor/EditorPane.cs) sends the modified navigation key
to core. However,
[`normalized_input_key`](src/core/command/input_keys.rs) strips navigation
modifiers in CommandLine mode, and ordinary prompt movement clears the
selection anchor in [`command/mod.rs`](src/core/command/mod.rs).

For `:abc`, Shift-Left leaves both anchor and active at byte 2 instead of
selecting `c`. macOS explicitly preserves the anchor in
[`moveCommandLineSelection`](src/mac/Editor/Sources/EVEditorView.swift), with
[Shift-navigation tests](src/mac/Editor/Tests/EVCommandPromptEditingTests.swift).
Windows ordinary arrows already use grapheme-aware core movement; the gap is
selection extension.

### B6. Keypad keys cannot have distinct bindings

The core input enum and C ABI have no dedicated keypad identities.
[`notation_key`](src/core/command/mappings.rs) aliases keypad Enter, navigation,
arithmetic and digit notation to ordinary keys. A mapping cannot distinguish
keypad Enter from Return, for example.

### B7. Style candidate rules and application config remain duplicated

Style-parent candidate filtering is implemented in both
[Swift `legalParents`](src/mac/Editor/Sources/EVCoreStyles.swift) and
[C# `LoadParents`](src/win/Shell/StyleWindow.cs), using namespace, role and
Base Paragraph rules. These are core model constraints being re-derived to
populate native pickers. A core candidate query would keep the offered choices
consistent with mutation validation.

The application `config.json` schema, defaults and validation still have
separate implementations in
[`EVConfigurationStore`](src/mac/AppShell/EVConfigurationStore.swift) and
[`Preferences`](src/win/Shell/Preferences.cs). They already disagree:
macOS rejects more than ten recent-document paths; Windows accepts longer
arrays and truncates the read list to ten.

### B8. Recording and pending-command status are not published

[`CommandInterpreter::is_recording_macro`](src/core/command/mod.rs) exists,
but [`ViemViewPresentationV1`](include/viem_core.h) exposes no recording
register, pending command text or active count. Native status bars cannot show
`recording @q` or the accumulating `"a2d` command from a shared core value.

### B9. Rust/header layout checks are not exhaustive

C# bindings are generated from the headers by
[`generate_bindings.py`](src/win/tools/generate_bindings.py), and Windows
validation checks their freshness. The remaining gap is agreement between Rust
and those headers: the explicit
[`check_c_abi`](examples/check_c_abi.rs) probe uses a hand-curated inventory and
selected offsets.

For example, `ViemCommandLineSelectionV1`, `ViemLayoutDecorationV1` and
`ViemViewRestorationV1` are absent from that probe. Header-to-C# generation
cannot detect disagreement with the Rust declarations. Native tests exercise
some of these APIs, but do not establish exhaustive size, alignment and field
offset agreement. Generate the comparison inventory, or derive all declarations
from one source. See [ABI validation](docs/abi-validation.md).

## Audit validation

Command and ABI assertions above were traced to current implementations and
existing tests. All 61 focused Rust tests passed through
`scripts/validate.py`, including temporary audit probes for unsupported Visual
arithmetic, prompt word insertion, filename-register reads and Shift-navigation
behavior.
The probes were removed after the audit. C# binding freshness was checked with
`python3 src/win/tools/generate_bindings.py --check`.

Native frontend findings are based on source and existing test expectations;
macOS and Windows native suites were not run for this Markdown-only update.

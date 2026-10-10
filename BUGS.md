# Bugs

## Consistency audit, October 9–10, 2026 (commit `f6c15de`)

This audit looked for editor states where the Rust core rejects ordinary
input (so typing, deleting, or moving the caret fails), panics, or behaves
inconsistently. It combined reading the code with a new fuzzer,
[`examples/fuzz_consistency.rs`](examples/fuzz_consistency.rs). No fixes were
made.

The fuzzer drives `Core::handle_with_layout`, the same entry point the FFI
uses for key and text input. Every core error and every non-success command
status counts as a finding; nothing is pre-classified as allowed. After each
action it also checks that:

- the caret is valid and drawable (the macOS frontend hides the caret when
  neither affinity has geometry);
- a current layout exists, or a same-size resize restores one, which is what
  `refreshLayoutIfNeeded` does;
- a fresh parse of `source_bytes()` gives the same text, blocks and styles as
  the incremental projection;
- undo restores the exact original bytes, and redo restores the edit;
- typing still works after an error.

Probe modes:

- `insert`: every op at every caret boundary, in both affinities.
- `normal`: about 85 Vim commands at every character. Replace-mode sequences
  such as `Rxy<BS><BS><Esc>` must restore the source exactly. In literal
  views, line-local commands (`x`, `cc`, `ciw`, `gUU` and so on) must not
  change the number of source lines.
- `selection`: deleting, replacing, pasting and formatting over boundary pairs.
- `copypaste`: native Copy of a selection, then Paste at another position,
  through the portable clipboard payload.
- `nav` and `navpos`: every arrow or motion key, from every position, at
  several widths. A key that doesn't move the caret before the document edge
  is reported as stuck.
- `walk`: random replayable sequences.

The seed corpus has about 310 Markdown documents, many degenerate, each
opened as WYSIWYG and as Source. It also has Code and Plain Text seeds and
the repository's own Markdown documentation. The probes ran about
4 million cases, at the default width and at 140 points, where most lines
soft-wrap. The walk ran 140,000 random sequences of 300 steps, in three
campaigns with different seeds, and its findings were minimized with the
`minimize` mode. A Vim differential compared about 180,000 Normal-mode,
Insert-mode, dot-repeat and Ex cases in Plain and Code against Vim. The
first two walk campaigns gave compositions the caret as their target even
when a selection existed. The macOS frontend passes the selection, so the
harness was fixed to match before the third campaign and the composition
probe, which found C78 and C81. Vim-semantics findings (C69–C72, C74,
C75, C77, C80) were compared against `/usr/bin/vim` 9.1 run with `-u NONE`.

The long runs used the helper scripts in `examples/fuzz_consistency/tools/`:

- `probe_driver.py` runs one seed per process with a timeout, so hangs
  become findings instead of stalling the run.
- `walk_driver.py` does the same for chunks of walks.
- `triage.py` groups findings and prints the shortest example of each.
- `vim_reference.sh` runs the installed Vim with no user configuration on a
  small buffer and prints the result, for checking Vim-semantics findings.
- `vim_diff.py` is a differential oracle. `fuzz_consistency vimcases
  OUT.jsonl` runs every Normal-mode op at every character of the Plain and
  Code seeds (or of `--sources list.json`) and records Viem's result.
  `vim_diff.py OUT.jsonl REPORT.jsonl` replays the same cases in Vim and
  writes the disagreements.

```sh
SEEDS=$(./target/release/examples/fuzz_consistency count) JOBS=16 \
  python3 examples/fuzz_consistency/tools/probe_driver.py \
  target/release/examples/fuzz_consistency target/consistency/insert insert --positions 30
python3 examples/fuzz_consistency/tools/triage.py target/consistency/insert/insert-findings.jsonl 'op_error' '' 3
./target/release/examples/fuzz_consistency minimize finding.json --want 'PANIC'
```

Character styles in Source view are skipped by default because of the C30
hang. Set `FUZZ_ALLOW_KNOWN_HANG=1` to include them, and
`FUZZ_PANIC_BACKTRACE=1` to print panic backtraces. `FUZZ_WIDTH=140` runs
the probes in a narrow view, so most lines soft-wrap. `FUZZ_OPS` limits
the insert, normal and selection probes to ops whose labels contain one of
its comma-separated substrings. `VIM_OPS` limits `vimcases` to exact op
specs, separated by `|`.

```sh
cargo build --release --example fuzz_consistency
./target/release/examples/fuzz_consistency probe --probes nav,navpos,insert,normal,selection --out target/consistency
./target/release/examples/fuzz_consistency walk --seed 1 --cases 500 --steps 300 --out target/consistency
# Replay a finding line saved from *-findings.jsonl into its own file:
./target/release/examples/fuzz_consistency replay finding.json
# Run an ad-hoc script: FORMAT WIDTH SOURCE ACTIONS_JSON
./target/release/examples/fuzz_consistency script Wysiwyg 70 $'| a | b |\n| - | - |\n| 1 | 2 |' \
  '[{"op":"key","key":"i"},{"op":"key","key":"Right"},{"op":"key","key":"Right"},{"op":"key","key":"Down"}]'
```

The `script` action JSON uses `{"op":"key","key":"Down"}`, `{"op":"text","text":"x"}`,
`{"op":"place","offset":5,"downstream":true,"extend":false}` and similar.
`examples/fuzz_consistency.rs` lists every action.

The mock shaper gives every character a fixed width and forms `fi`, `fl`,
`ffi` and `ffl` ligatures. Pixel-dependent findings were checked against the
CoreText provider's caret-stop rules where noted.

Severity: **S1** means the core panics, or the user can't type, delete or
move. **S2** means a wrong or lossy edit, or a display that differs from the
file on reopen. **S3** means an inconsistency or a confusing refusal.

---

## Summary: fix these first

The reported symptom, "the core throws an error and then I can't type or
scroll", has three distinct mechanisms in this audit:

- A panic or partial failure leaves the interpreter's stored positions on
  an old revision, so *every* later input fails (C0, C60, C78, C51, H1, H2).
- An ordinary edit at a particular boundary is rejected by verification, so
  that key always beeps at that caret (H5 and the WYSIWYG typing section).
- A failing layout export or Resize makes the frontend's all-or-nothing
  refresh fail, which blocks scrolling and clicking (C48, C49, H4).

| ID | Sev | Summary |
|---|---|---|
| C0 | S1 | `=G` (and other operators with jump motions) over multibyte text panics in jump-list bookkeeping. Every later key fails until the document is closed. |
| C60 | S1 | Source view: `yy`, `G`, `o`, `Esc`, `p` panics when the line starts with a non-ASCII character. Every key then fails until the user clicks. |
| H1/H2 | S1 | One stale mark, jump or revision blocks all input. Panics aren't contained; the FFI returns half-updated state. |
| C78 | S1 | Double-click a word, type two accented letters with dead keys (or any IME), type a letter, Cmd-Z: undo half-applies and every key then fails until a click. |
| C2 | S1 | `j`/`k`/Up/Down/Home/End error when the caret is inside an `fi`/`fl` ligature ("file", "first", "definition"). |
| C75 | S1 | `e`/`E` don't move from the last character of a word, so repeated `e` is stuck after the first word. |
| C3, C3b, C21, C21b | S1 | Tables: Up/Down stuck in columns past the viewport; the caret disappears moving Right through wide tables; a table at the start or end of the document can't be left; a wide table anywhere breaks every Visual Block command. |
| C53 | S1 | Bold/Italic fail on Select All in any multi-paragraph document, and on any selection that starts or ends with a space. |
| C19 | S1 | Tab on a newly created empty bullet fails, so Enter, Tab, type doesn't work. |
| C5 | S1 | Shift-Enter fails at the end of every hard-wrapped source line. |
| C30 | S1 | Source view: Code button and typing inside `**[bold link](u)**`, `__bold__` or `_it_` hangs the app forever (an unbounded loop). |
| C48 | S1 | Resize panics in narrow table layouts; afterwards the pane can't draw, scroll or hit-test. |
| C67 | S1 | Source view (the default Markdown view): `cc`/`S` on a line of a multi-line paragraph types the new text into the *next* line. Repeated `x` on a short line deletes the next line's text. |
| C50, C62 | S1 | `cc`/`S`/`Vc` on the final empty line fail in Plain and Source. Any Shift-selection fails right after Enter or `o` auto-indents. |
| C33, C34 | S1 | `:m`, `:t` (and `:sort` in Code) always fail in highlighted Code, and in any WYSIWYG document that contains a fenced code block. |
| C32 | S1 | Setext headings can't be deleted or changed. A failed `cc` then runs the typed text as Normal commands. |
| C4, C6, C7, C8, C9, C12, C15, C54 | S1 | Ordinary typing, Enter or Backspace rejected in emptied code blocks (including after `cc` on any code line), code spans, emphasis joins, list continuations, whitespace lines, table cells and list headings. |
| C1, C45, C61 | S1 | Further panics: Source table Enter, lazy list lines with non-ASCII text, Visual `J` across table cells. |
| C65 | S1 | Native Copy then Paste of formatted text fails inside or next to bold runs (`UnsupportedFormatting`). |
| C59 | S2 | `:global` is superlinear: 4,000 lines take 24–71 s on the main thread. |
| C70, C71, C72 | S2 | Vim differences that change text: `dd`/`yyp`/`cc` on the last line drop the final newline; `diw`/`ciw`/`daw` on whitespace or empty lines delete line breaks; `cc`/`S` discard indentation. |
| C74, C77 | S2 | In the app, `>>`/`<<` always shift by four spaces and ignore tabs, and `=` strips indentation. `dw` on a line's last word joins the next line. |
| C76 | S2 | Editing the only cell text of a one-column header-only table deletes the whole table. |
| C79 | S2 | On soft-wrapped lines, `J` is a silent no-op except on the last row, so it rarely joins prose paragraphs. `yyp`/`ddp` on a row put its text inside a word, and `cc`/`dd` on the last row merge the next line. |
| C80 | S2 | `:g/^$/d` leaves one of each run of adjacent blank lines; `:v/pat/d` skips lines after deleted blank lines. |
| C82 | S2 | In Code, selecting lines and pressing `J` (`VjJ`) silently does nothing. |
| C81 | S2 | While an IME composition replaces Select All (or any long selection), the selection export fails, so the frontend refresh fails until commit. |
| C73 | S2 | Source view: `gUU`/`VU` merge a paragraph with the next block. WYSIWYG `gUU` in a listed code block pulls the next item into the fence. |
| C55 | S2 | Typing over the whole text of a heading inside a list or quote drops or reorders the containers: in `- # heading`, double-click and type gives `# x`. |
| C68, C69 | S2 | Replace-mode Backspace and Ctrl-W delete text instead of restoring it: after a replacement changes Markdown structure (Source), and before the point where Replace started (all formats). |
| C23, C56 | S2 | Autoformat and structural edits leave the projection different from what the file reopens as. This is the most frequent walk finding. |

The fuzzer ran about 4 million probes across nearly 650 seeds,
140,000 random walks of 300 steps each, and about 180,000 cases against
reference Vim.

## Systemic issues

These aren't single reproductions. They are code paths that make a local
failure lock up the editor, and they explain why the user sees "an error
and then nothing works".

**H1. One invalid stored position blocks all input.**
`CommandInterpreter::capture_position_anchors` (`src/core/command/mod.rs:1626`)
runs before every input event (`Core::dispatch_core_event`). It returns an
error, aborting that input, when:

- `position_revision` differs from the document revision
  (`WrongSnapshot`); or
- any single mark or jump offset can't become a text point
  (`NotGraphemeBoundary`, `InvalidRange`).

Marks and jumps are raw byte offsets (`marks: BTreeMap<char, usize>`,
`jumps: Vec<usize>`). If any path leaves them unrebased (C0, C51, C60), every
key, Undo and Select All fails until the document is closed. Capturing
should tolerate damaged auxiliary positions, by dropping a stale mark or
jump with a diagnostic, and should resynchronize a stale revision through
history position maps rather than refusing input.

**H2. Panics are not contained.** `ffi_boundary` turns a panic into
`ViemStatus::Panic`, and the core lease is returned to the registry in
whatever state the unwind left it: partially published edits,
interpreter state that wasn't rebased, an open checkpoint. C0, C1, C45,
C48, C60 and C61 are all reachable from ordinary typing, resizing or Vim
commands. After
a caught panic the core should restore the pre-event checkpoint, or reset
view and interpreter state to a valid position on the current revision.

The fuzzer reached seven distinct panic sites from input:

- `.expect` in `record_jump` (C0);
- `RangeNode::leaf` (C1);
- `can_inherit_markdown_source_list_context` (C45);
- `append_following_viewport_tail` (C48);
- `text::line_start` (C60);
- a reversed slice in `project_local_candidate_with_limit` (C61);
- an underflowed index in `markdown_code::edited_source_fragment` (C61,
  Visual `J` over a table row containing emphasis).

About 2,700 `expect`, `assert`, `unreachable` and `panic` sites remain in
non-test core code.

**H3. A failed change operator turns the next keystrokes into commands.**
When `cc`, `S`, `C`, `cw` and so on fail (C32, C50), the view stays in
Normal mode. The text the user types next runs as Normal-mode commands. In
C32, typed `x` deletes a character. Given how often edits fail
verification, a failed change should probably still enter Insert, or the
frontend should refuse to forward further keys until the user
acknowledges.

**H4. The frontend refresh is all-or-nothing.** `EVEditorSurfaceController.refreshPresentation`
throws if any of these throws: `layoutExport`, `layoutPaintExport`,
`formattedSlice`, `compositionTextSlice`, `commandLineExport`, or
`visualSelectionExport` with any status other than outside-coverage or
unavailable. When it throws, `viewportState` and `layoutSnapshot` keep
their old identities.
`requestVerticalViewport` then passes the stale `viewportState` as
`expected`, the core rejects it as stale, and the user sees a beep and
"can't scroll". `refreshGeometryBeforeInteraction` also returns `false`,
which blocks hit-testing. A persistent core error in any single export,
or a failing same-size Resize (C48, C49), therefore blocks scrolling and
clicking indefinitely. Optional exports should degrade independently, and
the viewport identity should refresh even when an export fails.
Windows has the same shape. `EditorPane.Refresh`
(`src/win/Editor/EditorPane.cs:479`) clears `snapshot` (a blank canvas) and
reports the error when the same-size `View.Resize` or `View.Layout` throws,
so C48 and C49 blank the Windows pane.

**H5. Verification is the only fallback for ordinary typing.** Text,
Enter, Backspace and Delete are translated to minimal source patches,
reprojected, and rejected (`VerificationFailed` or
`FormattedPayloadCannotReproject`) if the candidate differs. That keeps
documents safe, but it means every gap in the translation (C4–C20, C32,
C41–C46, C52, C53) is a key that does nothing except beep. A degraded
second strategy for plain insertions and deletions, such as inserting
fully escaped text with numeric references, or rewriting the smallest
enclosing inline construct, would make "can't type" much rarer while
keeping verification.

**H6. Unbounded loops in style application.** C30 is an unbounded `loop` that
assumes progress. AGENTS.md requires bounded work. Other `loop {}`
constructs in `named_character.rs`, `typing.rs` and the list, quote and
table edit code should be audited for the same assumption.

## Lock-ups, panics and hangs

### C0 (S1, highest priority). Panic in jump-list bookkeeping freezes the view

This reproduces the reported "the core throws an error and then I can't
type or scroll" directly.

Plain Text or Markdown Source, document ` 中文` (a leading space, then
multibyte text):

1. `$`: the cursor is on `文`, at offset 4.
2. `gg`: this records a jump at offset 4.
3. `=G`: reindent to the end of the document. This removes the leading
   space, so the text becomes `中文`.

Step 3 commits the edit and then panics:
`a retained jump resolves to one hard line: NotCharacterBoundary { offset: 4 }`.
After that the view is unusable:

- Every key (Escape, `i`, typing, `j`, arrows) fails with
  `Document(WrongSnapshot { expected: Revision(1), actual: Revision(0) })`.
- Clicking (`PlaceCursor`) succeeds, but every following key, `u`, native
  Undo, Select All and Backspace fail with `Document(NotGraphemeBoundary(4))`.

Only closing the document recovers. The walk fuzzer hit this panic in
about 4,500 walks across many documents. The trigger is usually `gg=G`,
`:g/…/d` (the reported key is the Enter that submits it) or `G` after an
edit that shifted multibyte text. The failed action had already changed
the source (`failed_action_changed_source`), so the panic isn't atomic.

Root cause: `CommandInterpreter` keeps the jump list as raw byte offsets
(`self.jumps`, used by `record_jump` at `src/core/command/mod.rs:13779`).
AGENTS.md requires stable anchors for jumps. For an operator whose motion
is a jump (`=G`, `dG`, `d/…`, and so on), the edit is committed first.
`record_jump` then runs against the new document while `self.jumps` still
holds pre-edit offsets, and its `.expect("a retained jump resolves to one
hard line")` (line 13798) panics on an offset that is no longer a
character boundary. Because the panic unwinds past the coordinator's
rebasing and publication, the interpreter keeps its old revision, which
gives `WrongSnapshot`. The poisoned offset stays in `self.jumps`, so
`capture_position_anchors`, which runs before every input, keeps failing.

Two general problems make this worse:

- Panics in the core aren't contained. The FFI catches the unwind and
  returns `ViemStatus::Panic`, but leaves half-updated state in place.
- Raw offsets survive edits in the interpreter. These should be anchors,
  or be rebased before any post-edit bookkeeping.

With ASCII-only text the same ordering bug leaves wrong, but valid, jump
positions instead of panicking.

### C78 (S1). Undo after composed input over a selection freezes the view

All formats. In `hello world`:

1. Double-click `hello`, or select it any other way (Shift-arrows, a drag,
   Select All).
2. Type `é` and then `ñ` as committed compositions. The macOS dead keys
   (Option-E, Option-N), press-and-hold accents and every CJK input method
   use marked text.
3. Type an ordinary `x`. The text is `éñx world`.
4. Cmd-Z (the `NavigateHistory` undo intention).

The undo publishes part of the change (the source becomes `é world`), then
fails with `WrongSnapshot { expected: Revision(1), actual: Revision(2) }`.
The interpreter keeps a position from revision 1, so every later key, text
input and undo fails with `WrongSnapshot { expected: Revision(1), actual:
Revision(3) }`. That's the same lock-up as C0 and H1, and only a mouse click
recovers it. Both compositions are needed, and they must replace a
selection. Typing `x` instead of the second composition, or starting from
an Insert caret with no selection, works. With a paste as the third input,
Vim `u` after Escape also fails with `WrongSnapshot`. Undo then stays
broken, though other keys keep working. The walk found that variant in
Source `+ plus\n+ list\n\n- minus\n- list`: Select All, `中文`, `ñ`, paste
a table, Escape, `u`. The partial publication
also breaks AGENTS.md's rule that unavailable history targets "fail
without partial installation". The layout export fails too
(`Layout(InvalidTextOffset)`, and a same-size Resize doesn't restore it), so
under H4 the pane also stops drawing and scrolling.

The composition-over-selection probe hit this on almost every seed in every
format: about 12,000 failing cases, from a one-character selection in `a`
or `\n` to large selections in `docs/`. A single committed composition over
a selection behaves like typing over it. It fails exactly where typing
fails (C12, C14, C17, C53), and nowhere else.

### C81 (S2). An open composition over a selection breaks the selection export

All formats. Select All in `First paragraph.\nSecond line.` (29 bytes), then
begin a composition such as Japanese `ほん` or a dead-key `´`, and leave it
uncommitted. While the marked text is shown, the view's selection export
fails with `InvalidTextOffset(29)`, and the caret has no geometry. The
selection is still reported in document offsets (`0..29`), but the
presentation layout already holds the shorter marked text instead of the
selection. Any selection that extends past the composed text's length does
this, for example a drag from `paragraph` to the end. A small selection
near the start (a double-clicked first word) doesn't. Under H4 the macOS
refresh is all-or-nothing, so the marked text and caret can't update for
the whole composition. L7, an unexportable selection after a *rejected*
composition commit, looks like the same coordinate mismatch.

### C60 (S1). Panic: `yy`, `G`, `o`, `Esc`, `p` in Source view when the line starts with non-ASCII

Markdown Source, `é\nb`. This also happens with `か\nb`, `中文\nb`,
`👩\nb`, `é`, or any yanked line whose first character is multibyte.
`yy`, `G`, `o`, `Esc`, `p` applies the put (the text becomes `é\nb\né`)
and then panics with
`a command offset is a valid UTF-8 hard-line boundary: NotCharacterBoundary { offset: 6 }`
at `src/core/command/text.rs:54` (`line_start`, called from
`first_nonblank_document` in `CommandInterpreter::paste_impl`). After
that, every key, `u` and native Undo fail with `WrongSnapshot` until the
user clicks in the text; clicking resynchronizes the interpreter. Plain
Text with the same keys is fine, and so is `aé\nb`. The post-put cursor is
computed as one *byte* past the start of the new line, which is inside a
multibyte first character. This is the ordinary "yank a line, paste it at
the end" workflow.

### C1 (S1). Panic: typing after Enter inside a Source-view table row

Source view, `| a | b |\n| - | - |\n| 1 | 2 |\n| a | b |\n| - | - |\n| 1 | 2 |`
(a table whose body contains a second header-like row and a delimiter-like
row).

1. Press `i`, then put the caret after `| ` on the fourth line (text offset 32).
2. Press Enter. Source Enter inserts `\n\n`, giving `| \n\na | b |`.
3. Type `x`.

The core panics at `src/core/document/range_index.rs:818`:
`range-index input is ordered by start boundary`.
The FFI returns `ViemStatus::Panic` and gives the core back in whatever state
the panic left. After that, **every printable key typed at that caret panics
again**. Backspace still works. With only the first three rows plus
`| c | d |`, the same steps do not panic.

Note also that Source Enter splits the table row with a blank line, which
moves the remaining rows out of the table. See Q3.

Backtrace: `CommandPlan::prepare_model` → `Document::prepare_formatted_payload_edits`
→ `prepare_text_edits_with_patch_policy` → `Document::build_table_row_candidate`
→ `FormattedDocument::replace_table_row_metadata` →
`TableRows::splice_transformed` → `RangeNode::leaf`, where the
`expect("range-index input is ordered by start boundary")` fires.

The row-local table fast path is taken for an edit that changes table
structure, because the blank line already ends the table. It hands the
range index rows that are no longer ordered. It should detect the structural
change and fall back to full reprojection. Panic sites like this `expect` are
reachable from ordinary typing, and the FFI turns a panic into a recoverable
status without restoring a known-good state.

### C45 (S1). Panic: typing on a lazy list continuation line containing non-ASCII

Source view, `- y\na`, where `a` is a lazy continuation of the list item.

1. `G`, `A`, then type `é`. A smart-quoted `”` or any multibyte character
   works too.
2. Type `x`.

Step 2 panics: `start byte index 2 is not a char boundary; it is inside 'é'
(bytes 1..3 of string)`. Every later keystroke at the end of that line
panics too. The same happens with a lazy line that starts with an emoji
(`👩‍💻`). The lines `- y\n  a`, `- y\nab` and `> y\na` don't panic.

This is the most common panic in both walk campaigns, about 14,000 hits
each. The composition probe also reached it in `- a\n- b\n- c`: selecting
` b\n- ` and committing `é` leaves the lazy line `-éc`, and the next
character typed there panics.

Root cause: `Document::can_inherit_markdown_source_list_context`
(`src/core/document/transaction.rs:5908`) computes the line prefix as the
list item's marker length (`list_marker_range_for_block(&block)`, 2 bytes
for `- `). It applies that length to every hard line in the item, including
lazy continuation lines that have no marker, then byte-slices the line with
it. With ASCII-only lines it just computes the wrong prefix. With non-ASCII
it panics.

### C48 (S1). Panic in Resize leaves the view without any layout

WYSIWYG, `>> nested`:

1. Table → Insert, 2 columns and 1 body row, at offset 0.
2. Resize the view to 40 × 900.

The core panics with `called Option::unwrap() on a None value` at
`src/core/layout/engine.rs:1002`
(`RegionalLayoutSnapshot::append_following_viewport_tail`, reached through
`materialize_document_end` and `materialize_requested_viewport_with_focus`).
After that no current layout exists, and every same-size Resize panics
again. That Resize is exactly what `refreshLayoutIfNeeded` does before
drawing, hit-testing or scrolling, so the pane can't draw, scroll or be
clicked. The walk also hit this with
`| a | b | c |\n| :-: | --- | ---: |\n| 1 | 2 | 3 |` at 120 × 200, a size a
narrow split pane can reach (the minimum pane width is 100 DIP). The second
walk campaign also reached the same unwrap by *scrolling* to the end of a
narrow table document (`materialize_document_end` from a Scroll), so the
panic doesn't need a Resize.

Root cause: `append_following_viewport_tail` keeps only the rows below
`top`, falls back to the previous last row, and then unwraps
`rows.first()`. When the first regional line has no rows at all (a table
squeezed to zero usable width), `last` is `None` and the unwrap panics.

### C61 (S1). Panics: Visual `J` across table cells

WYSIWYG, `| a |\n| - |\n| 1 |\n| 2 |\n| 3 |\n| 4 |`, view 300 × 60. The
minimized 10-step trace (JSON actions for the `script` mode):

```
[{"height": 60.0, "op": "resize", "width": 300.0}, {"after": false, "op": "table_column"}, {"key": "Down", "op": "key"}, {"op": "text", "text": "**"}, {"key": "S-Up", "op": "key"}, {"key": "WordRight", "op": "key"}, {"key": "Esc", "op": "key"}, {"key": "v", "op": "key"}, {"key": "j", "op": "key"}, {"key": "J", "op": "key"}]
```

That is: Insert Column Left, Down, type `**`, Shift-Up, WordRight, Esc,
`v`, `j`, `J`. The final `J` panics with
`slice index starts at 5 but ends at 0` in
`markdown_block_styles::project_local_candidate_with_limit` (from
`preserve_retained_literals_in_regions`,
`prepare_structural_text_batch`, `CommandInterpreter::join_hard_lines`).
Simpler table joins fail with `VerificationFailed` or C37's
`TextDoesNotMatchLayout` instead.

A second, much simpler panic in the same command came from the second walk
campaign. WYSIWYG `| a | b |\n| - | - |\n| **c** | d |`, caret on `b`, `v`,
`j`, `J` (or `gJ`) panics with
`range end index 18446744073709551612 out of range for slice of length 1`
in `markdown_code::edited_source_fragment` (from
`markdown_split::remove_empty_emphasis`). The index is a negative number
that wrapped. The trigger is emphasis anywhere in the row the selection
reaches: `*c*`, `**c**`, and `c**c**` all panic. Typing still works
afterwards in this case.

### C30 (S1). Hang: character style inside emphasis or bold links (Source view)

Source view, `_ab_`, caret at offset 2 (between `a` and `b`). Toolbar Code
(`AssignNamedStyle(Character, "Code")` at a caret, which sets a pending
typing style), then type `x`. The core never returns. One probe process
on `_it_` was still running after 9 hours (268 CPU minutes) with one core
pegged. On macOS this freezes
the main thread, so the app beach-balls. The same happens in
`a _it_ b` at offset 4. `*ab*` at the same position finishes immediately
and gives `` *a*`x`*b* ``. WYSIWYG `_ab_` is fine.

The hang is not limited to underscores. A scan of every caret position
(8-second timeout each) also hangs:

- `**a *b* c**` at offsets 4, 5 and 8 (nested emphasis);
- `__bold__` at offsets 3–5;
- `**[bold link](u)**` at every offset inside the link (3–11).

Bold link text is common, so clicking the inline-code button there and
typing freezes the editor.

Applying a character style to a selection hangs through the same loop. In
Source view, select offsets 3..4, 3..5 or 4..5 inside `__bold__` and choose
Default Paragraph from the character-style menu
(`AssignNamedStyle(Character, "")`), which clears direct traits. The core
never returns. The selection probe also stalled on `***both***`,
`*a **b** c*`, `` **`code in bold`** `` and
`a\n\n<div>\n\n**md inside html**\n\n</div>\n\nb`.

Root cause: `Document::prepare_character_style_choice`
(`src/core/document/named_character.rs:256`) loops until no Strong or
Emphasis span overlaps the target range. In Source view it clears only the
`selected` part of the span through `prepare_typing_markdown_style(…, false)`.
For `_` emphasis the split spelling `_a_x_b_` still parses as one emphasis,
because intraword `_` can neither open nor close. The span is found again
on every pass, the patches are never empty, and each pass adds delimiters
to the scratch document. The loop has no progress check or iteration
bound. AGENTS.md requires bounded retries and degraded work.

### C51 (S1, latent). Multi-command text in Normal mode corrupts undo

Any format, for example Plain `abcdef\nghij`. Type `i q Esc`, then deliver the
text event `"xx"` or `"~~"` while in Normal mode
(`viem_core_view_send_text_with_host_context_v2`, i.e. `InputEvent::Text`),
then press `u` or use native Undo. Undo fails with
`WrongSnapshot { expected: Revision(2), actual: Revision(1) }` and leaves
the text partly undone. With source `---`, `i q Esc i q Esc`, the text
`"~~"`, then native Undo, every later key fails with `WrongSnapshot`
permanently: the C0-style lock-up.

The same commands sent as separate key events, through `:normal ~~`,
through a macro, or through a mapping all work. Both frontends normally
split Normal-mode text into key events (`EVEditorView.insertText` and
`EditorPane.DeliverText`). They decide from their cached presentation
mode, though, so a stale mode makes them send multi-character text to a
Normal-mode core. That happens when a refresh failed, which several bugs
above cause. The text path should run each character as its own
command-turn publication, or reject multi-command text in Normal mode.

## Caret movement, tables and document edges

### C2 (S1). Up/Down/`j`/`k` fail when the caret is inside a ligature cluster

Plain Text, Markdown (both views) and Code; any font that forms `fi`, `fl`,
`ffi` or `ffl` ligatures. Example document: `the file\nnext line here`.

Normal mode `w l j` puts the cursor on the `i` of `file`, inside the `fi`
cluster. `j` then returns
`Error("layout motion failed: PositionNotInLayout(VisualPosition { text_offset: 5, … })")`.
`k`, Up and Down fail the same way, in Normal and in Insert, and so do Home
and End (seen in `[^1]: lone footnote definition` at offset 23, inside
`fi`). The caret can't move vertically or to the line edges until it is
moved horizontally out of the cluster. Every
attempt beeps and shows the error.

The vertical motion code calls `locate(snapshot, current)` in
`move_visual_rows` (`src/core/command/layout_motion.rs`), which needs an exact
caret stop. The CoreText provider emits caret stops only at a cluster's start
and end (`CoreTextMeasurementProvider.swift:1250`), so a logical grapheme
boundary inside a ligature has no stop. Logical motions such as `l`, `x`,
mouse placement, search and undo can still leave the cursor there. This
should be common in prose: "file", "first", "find", "office" and so on in any
proportional font with standard ligatures. Code view's default monospace
font has no such ligatures. Users who pick a programming-ligature font
(Fira Code, JetBrains Mono, Cascadia Code) get multi-character clusters for
`->`, `=>`, `!=`, `==`, `::` and `<=`, so the same failure happens in Code
whenever the cursor sits inside one of those operators. The
`logical_endpoint_geometry` cluster fallback that drawing uses is not used
by `locate`.

`locate` (`src/core/command/layout_motion.rs:536`) searches for a caret
whose offset *and* affinity both equal the cursor's. If none exists it
returns `PositionNotInLayout`, which `handle_with_layout` doesn't retry.
That error is shared by every layout motion: j/k, Up/Down, Home/End, g0/g$,
H/M/L, paging, and Visual Block. So any cursor whose (offset, affinity)
pair has no exact stop fails all of them. Ligature interiors are one way
to get there; a stale affinity on a bidi or wrap boundary would be another.
`locate` should fall back to the cluster-containing row, or to the
opposite affinity, as the frontend does when drawing.

### C3 (S1). Up/Down stuck in a table column that extends past the viewport

WYSIWYG, any table wider than the view. Example: `| a | b |\n| - | - |\n| 1 | 2 |`
at a 70-pt view width; cell `b` sits at x = 88.

1. Press `i`, then Right Right to reach cell `b`.
2. Press Down. The caret doesn't move, and no error is reported.

Up from `2` and Normal `j`/`k`/Up/Down behave the same way. This happens
with every table spelling, including tables inside lists and quotes. The
left column works. The column works too once something else has scrolled it
fully into the horizontal viewport, or when the window is wide enough.

Root cause: `LayoutSnapshot::adjacent_visual_row`
(`src/core/layout/tables.rs:1636`) returns `None` when the same column's cell
in the adjacent row has no materialized rows in `table_navigation`. Sparse
horizontal materialization lays out only the cells near the horizontal
viewport. `move_visual_rows` (`src/core/command/layout_motion.rs:338`) treats
`None` as "at the document edge" whenever `snapshot.contains_document_end()`
or `contains_document_start()` is true, which it is for any document that
fits vertically, so it breaks out without moving. Missing horizontal geometry
should produce a horizontal layout demand, as the single-row path already
does through `horizontal_demand`, not a silent no-op. AGENTS.md says
"Missing regional layout is not a document edge".

### C3b (S1). A table at the start or end of the document traps the caret

WYSIWYG, `| a | b |\n| - | - |\n| 1 | 2 |`, the table being the last block.
From the last cell, Down, Right, Cmd-Down (DocumentEnd) and Option-Down
(ParagraphEnd) all leave the caret in cell `2`. Typing goes into the cell.
Normal-mode `o` inserts `<br>` inside the cell (`| 1 | 2<br>x |`) instead
of opening a paragraph below the table. There is no keyboard way to add text
after the table. Tab appends a row, and Enter adds a cell line break.

A table that starts the document behaves the same way. Up, Left,
Cmd-Up and Option-Up stay in cell `a`, and `O` or Enter at the start
produce `| x<br>a |` or `| <br>xa |`. There's no way to add a paragraph
above it.

`docs/markdown-tables.md` requires: "Movement beyond the table reaches
adjacent prose; provide an editable ordinary boundary at document edges
when insertion requires it." Together with C3 and C21, this is the most
likely source of the reported "cursor gets stuck in a table".

### C21 (S1). Caret disappears moving Right through a wide table

WYSIWYG, view width 300, with this table:

```
| one | two | three | four | five | six |
|---|---|---|---|---|---|
| 1 | 2 | 3 | 4 | 5 | 6 |
| alpha | beta | gamma | delta | epsilon | zeta |
```

In Normal mode, from the start of the document, press Right repeatedly.
Through the header row the viewport scrolls horizontally as expected. Moving
from cell `5` (x = 321, viewport left 31.6) to cell `6` (offset 38) leaves the
viewport unchanged. The caret then has no geometry for either affinity
(`NotACaretStop`), so the frontend hides it. Up and Down from there are stuck,
as in C3.

If the caret starts at cell `1` in a fresh view and moves Right, the same
step reveals correctly (viewport left 117.7). So the result depends on which
horizontal band was materialized earlier.

`materialize_revealed_horizontal_viewport` (`src/core/coordinator.rs:2483`)
reveals the caret only if `logical_endpoint_geometry` already succeeds. When
the destination cell isn't in the current horizontal band, nothing raises a
horizontal layout demand for the caret.

### C21b (S1). Visual Block is unusable in documents containing a wide table

WYSIWYG, `para\n\n| a | b | c | d |\n|---|---|---|---|\n| 1 | 2 | 3 | 4 |`
at view width 120, where the table is wider than the view. With the
cursor on `para`, far from the table, Ctrl-V then `I`, `A` or `d` (and
Ctrl-V `j` `I`) fail with `LayoutMotion(OutsideMaterializedCoverage(LayoutDemand { … }))`.
At width 600, or with a two-column table that fits, they work. The walk
hit this about 13,000 times, including through tables nested in list
items. Visual Block resolution asks for exact geometry that sparse
horizontal materialization of the overflowing table never supplies, and
`handle_with_layout` gives up after its retry budget (see C3 and C21).

### C3c (S2). A code block at the end of the document traps Insert mode

WYSIWYG, `` ```\ncode\n``` `` as the last block. In Insert mode at the end
of `code`, Down, Right and Cmd-Down leave the caret inside the block. Enter
(any number of times) only adds blank code lines. There's no Insert-mode
way to start a paragraph after the block: only Escape then `o` creates
`` ```\n\n x ``. A writing session that typed a fence and kept typing put
the following link, rule and paragraph inside the code block. The spec's
"editable ordinary boundary at document edges" rule for tables would apply
equally here (see Q11).

### C22 (S2). Word motions land on non-caret-stop offsets in empty table cells

WYSIWYG, `| | |\n|-|-|\n| | |` and `| a | b |\n| - | - |\n|  |  |`:

- Normal `WordRight` from offset 1 lands on offset 3.
- Insert or Normal `WordLeft` from offset 5 lands on offset 2.

Neither landing offset has caret geometry for either affinity, so the caret
is invisible. Both offsets are at the boundaries of empty cells.

## Typing, Enter, Backspace and Delete in WYSIWYG

### C19 (S1). Tab on a new, empty list item fails, breaking basic outlining

WYSIWYG, `- first item\n- second item`: `G`, `A`, Enter (a new empty
bullet), then Tab fails with `VerificationFailed`. Typing one character
first and then pressing Tab works. So the standard outlining gesture
(Enter, Tab, type) fails every time. A writing session typed from scratch
hit this on its first nested bullet. `- a\n- \n- c` with the caret on the
empty middle item fails the same way in WYSIWYG and Source, and the Indent
toolbar action fails too in Source.

### C5 (S1). Shift-Enter fails at the end of a source line inside a paragraph

WYSIWYG, `a\nb\nc`. The text is `a b c` because source line endings fold to
spaces. With the caret at offset 3 (after `b`, before the folded break), or at
offset 1 in `a\nb`, Shift-Enter fails with `VerificationFailed`. Shift-Enter
in the middle of a line works (`abc def` → `abc\\\n def`). Any Markdown file
with hard-wrapped prose hits this at every line end.

Mechanism: offset 1 sits *before* the space that represents the folded line
ending, so the expected text is `a\n b`. The natural source spelling
`a\\\n b` loses that space, because Markdown strips leading whitespace on
a continuation line. The translator doesn't protect it (as `&#32;`), so
verification rejects the edit. At offset 2, after the folded space,
Shift-Enter works and gives `a\n\\\nb`. A caret placed right after the last
word of a source line, by clicking or with the arrow keys, is on the failing
side. That is where a user naturally presses Shift-Enter to break a line.

Shift-Enter also fails with `VerificationFailed` in:

- an empty ATX heading (`#`, `##`);
- an empty list item (`-`);
- a thematic break (`---`, `***`, `___`), at offset 0;
- whitespace-only documents (`\t`, ` `);
- a setext heading.

For `[ref]: …` reference definitions and table-adjacent paragraphs it fails
with `FormattedPayloadCannotReproject`. After those failures typing `x` also
fails (`stuck_after_error`).

### C4 (S1). An emptied fenced code block rejects typing (Backspace, `cc`, `S`)

WYSIWYG, any fence: `` ```\nx\n``` ``, `` ```rust\nx\n``` `` or `` ``` a b\nx\n``` ``.

1. Put the caret after `x` (`i`, then caret at 1).
2. Press Backspace. The block is now empty, and the source is
   `` ```\n\n``` ``.
3. Type `y`.

Step 3 fails with `FormattedPayloadCannotReproject`, and so does every later
key at that caret. A freshly opened `` ```\n\n``` `` accepts the same typing,
and a fresh parse of the post-Backspace source matches the incremental
projection. So the stale part is the post-delete caret's insertion or typing
context, not the document. This blocks the common "clear the block, retype
it" flow.

The most common Vim path hits the same state. With the cursor on a code
line, `cc` (or `S`) followed by typing fails: `` ```\ncode\n``` `` gives
`VerificationFailed` for the typed `x`, and
`` text\n```\ncode\n```\ntext `` gives `AmbiguousProjection`. After
Escape the block is left empty and the typed text is lost. Line endings
make no difference.

### C6 (S1). Enter or typing inside an inline code span

WYSIWYG:

- `` `code` ``: Enter anywhere inside the span (offsets 1–3) fails with
  `VerificationFailed`. A paragraph split inside a code span should produce
  two code spans, or at least split the paragraph.
- `` `a\nb` `` (a code span crossing a source line ending): Enter at offset 1,
  and typing `>`, `#`, `*`, `[`, `_` or `-` after the folded space (offset 2),
  fail with `VerificationFailed`.
- Typing `` ` ``, `&`, `<`, `~` or `!` at the same place fails with
  `FormattedPayloadCannotReproject`.

### C7 (S1). Enter then Backspace inside emphasis can't rejoin

WYSIWYG `*it*`, caret at 1 (between `i` and `t`). Enter works and gives
`*i*\n\n*t*`. Backspace then fails with `VerificationFailed`, so the user
can't undo the accidental split by deleting. The same happens for `_it_`,
`*a\nb*`, and for `\na` at offset 1.

### C8 (S1). Deleting the only character of a list continuation line

WYSIWYG, `- a\n  b`: Backspace after `b` (offset 3) fails with
`VerificationFailed`. So does Delete before `c` in `- ab\n  c` (offset 3), and
Backspace at the end of `- ab\n  c\n- d`. Plain paragraphs (`ab\nc`) and
quotes (`> ab\n> c`) handle the same edit; the result is `ab ` with a
trailing space. List items with hard-wrapped continuation lines are very
common.

### C9 (S1). Joining paragraphs whose join would activate inline syntax

WYSIWYG `**a\n\nb**`. The text is `**a` / `b**`, literal because emphasis
can't cross paragraphs. Backspace at the start of the second paragraph
(offset 4), or Delete at the end of the first (offset 3), fails with
`VerificationFailed`. The join creates `**a b**`, which would become strong.
AGENTS.md requires local repair of "newly active punctuation". The same
failure appears at thematic-break and setext boundaries:

- `a\n***\nb`: second Backspace;
- `a\n=\n\nb`: Backspace at the end of the setext heading `a` (offset 1), and
  Delete at offset 0.

### C10 (S1). Editing inside an autolink

WYSIWYG `<https://auto.link>`. Backspace at offset 6 removes the `:`, so the
text is no longer an autolink, and fails with `VerificationFailed`. Delete at
offset 5 fails the same way. In `<mailto:a@b.c>`, typing `-` works, but a
following space fails with `FormattedPayloadCannotReproject`.

### C11 (S1). Upstream caret after a soft break in a block quote

WYSIWYG `> a\n> b`, text `a b`. With the caret at offset 2 and Upstream
affinity (end of the first visual segment), typing anything (`x`, `` ` ``,
`&`, `<`, `[`, `#`, `*`, `|`, `é`, an IME commit) fails with
`FormattedPayloadCannotReproject`. With Downstream affinity it works and
inserts at the start of `b`. Upstream affinity at that offset comes from
clicking at the end of a row, End, or Left from the next row. Nested quotes
(`> > nested\n> back`) behave the same way.

### C12 (S1). Whitespace-only documents and trailing whitespace-only lines

WYSIWYG:

- `" "`, `"\t"` or `"   \n  \n"`: with the caret after the whitespace (the
  only caret stop besides 0), every printable character, Tab, Shift-Tab,
  paste and IME commit fails with `FormattedPayloadCannotReproject`. Typing
  at offset 0 works. On `"   \n  \n"` at offset 2, typing stays blocked even
  after Escape and `i` (`stuck_after_error`).
- `abc\n\nend\n  ` (last paragraph's final source line is only spaces) and
  `a\n   `: `A` then `x` fails.

A user opening an otherwise empty file that contains a space or a newline
with indentation can't type at the end of it.

### C13 (S1). Empty block quote

WYSIWYG `>`. Typing a space, Tab or Shift-Tab at the only caret position
fails with `FormattedPayloadCannotReproject`.

### C14 (S1). Two consecutive reference definitions lock typing at the start

WYSIWYG `[ref]: /url "title"\n[other]: <x y>`. At offset 0, every printable
key, Tab, Shift-Enter, paste, Delete and Backspace-then-type fails with
`FormattedPayloadCannotReproject`. Each definition on its own accepts typing
at offset 0.

### C15 (S1). Enter in the last table cell when a paragraph follows the table

WYSIWYG `| a | b |\n| - | - |\n| 1 | 2 |\n\ntext after`. Put the caret at the
end of cell `2` (offset 7) and press Enter. The cell gets a `<br>`
(`| 1 | 2<br> |`). Typing `x` on the new cell line then fails with
`FormattedPayloadCannotReproject`, and keeps failing. Without the following
paragraph, or with only a trailing newline, it works and gives `2<br>x`. The
same failure happens with two tables separated by a blank line, and with a
table inside an ordered list item (`1. a\n\n   | x | y |…\n\n2. b`).

### C16 (S1). Enter at the start of the first paragraph after one leading blank line

WYSIWYG `\na` or `\nab`. The text is `\na`, with an editable empty paragraph
first. Enter at offset 1 (start of `a`) fails with `VerificationFailed`.
`\n\na` works.

### C17 (S1). IME composition next to an inline image

WYSIWYG `![img](local.png)` or `a ![img](i.png) b`. Begin and update a
composition at the caret after the image (offset 3 for a lone image). The
commit, or even a cancel, fails with
`Layout(MalformedMeasurement("invalid inline image range"))`. macOS uses
marked text for dead keys and press-and-hold accents, so typing é there
fails too.

The press-and-hold replacement path also fails elsewhere. That path is
`beginComposition(replacing:)` over the just-typed letter, then commit.
Inside code nested in bold (`` **`code in bold`** ``), typing `e` works,
but replacing it with `é` fails with `UnsupportedFormatting`. It also fails
inside `<ins>unclosed` (`VerificationFailed`), inside autolinks
(`<mailto:a@b.c>`), and in the C12 and C14 contexts.

### C18 (S1). Typing at the start of an HTML comment or raw HTML block

WYSIWYG `<!-- comment -->`, `<!-- unclosed comment` or
`<script>alert(1)</script>`. A space, Tab or Shift-Tab at offset 0 fails with
`VerificationFailed`. So does `\` at offset 0 in `</closing-only>` and
`<unknown-tag attr>`, and `&` or `<` at offset 17 in the script block. These
comments and blocks show as literal, editable text, so the user expects to
be able to type there.

### C20 (S2). ATX closing sequence

WYSIWYG `# Heading #`. Typing `#` at the end of the title (offset 7) gives
`# Heading\# #`. A following space then fails with
`FormattedPayloadCannotReproject`. Typing `-` then space, or `>` then space,
fails in the same place, including in `# Heading ###   ` and
`> # quoted heading`.

### C46 (S1). Replacing selected text at the start of single-tilde strikethrough

WYSIWYG, `~strike~` or `a ~strike~ b`. Select the first struck character with
the mouse (pointer gesture, then extend) and type `x`. This fails with
`VerificationFailed`. Deleting the same character (Backspace, `x`) works,
and `~~strike~~` works. Only replacement fails, so type-over a selection is
broken at that boundary. Related selection replacements also fail:

- Enter over a selection in `\na` (1..2) and in `# Heading #`;
- typing over a selection in a setext heading;
- pasting over a selection in `# Heading #`, in reference definitions, and
  in whitespace-only lines.

### C52 (S1). Typing punctuation over a heading's entire text fails with an internal error

WYSIWYG, `# abc` or `# abc def`. Select the whole heading text (Shift-End
from its start, or a mouse selection) and type `*`, `-`, `#` or `> `. This
fails with
`Document(FormattedTextStorage(InvalidRange { start: 1, end: 2, length: 1 }))`,
an internal storage error. Letters and digits work. Retyping a heading as
`*Note*` or `- item`, or starting it with `#`, is impossible without
deleting first. The one-character heading reached from `#` behaves the
same way.

The same type-over in a quote (`> abc`, select all, type `-`) fails with
`VerificationFailed`. Paragraphs and list items accept it.

### C54 (S1). Can't type into an empty heading inside a list item

WYSIWYG, `- # `, `- #`, `1. # ` or `- ## `. Typing `x` at the only caret
position fails with `FormattedPayloadCannotReproject`. The same thing in a
quote (`> # `) or at top level (`# `) works. This is also the state that
`cc` leaves behind on `- # h`: after `cc`, the typed text is rejected and the
heading stays empty.

### C57 (S1). Typing at the start of multi-paragraph HTML comments and `<pre>` blocks

WYSIWYG `<!--\ncomment\n\nspanning\n-->` at offset 0, or
`<pre>\npre block\n\n</pre>\n\nafter` at offset 11. Typing `` ``` ``, `#`,
`<`, Tab, Shift-Enter + `x`, or Indent fails, and the caret then stays
blocked: typing `y` after Escape and `i` fails too (`stuck_after_error`).
In `a\n- \nb`, Shift-Enter + `x` with Upstream affinity at offset 1 also
leaves typing blocked.

### C66 (S1). Deeply nested containers reject structural keys

- WYSIWYG `- > ```\n  > code\n  > ``` ` (a fence inside a quote inside a
  list item). Enter at the start of the code fails with
  `FormattedPayloadCannotReproject`. Backspace there fails with
  `UnsupportedFormatting`. AGENTS.md says "Backspace at a code paragraph's
  visible start removes code treatment."
- Source `> - > - > a\n> - > - > b`: Tab (structural indent) at offset 12
  fails with `AmbiguousProjection`.
- WYSIWYG `<!--\ncomment\n\nspanning\n-->`: Enter at offset 4 fails with
  `AmbiguousProjection`. So does pasting multi-line text there.

The same probe run with every seed converted to CRLF line endings
(`FUZZ_CRLF=1`) found no CRLF-specific failures. Every CRLF failure
reproduced identically with LF.

## Vim commands

### C50 (S1). `cc`, `S` and `Vc` on the final empty line of a newline-terminated file

Plain Text or Markdown Source, `a\nb\n` (any file that ends with a line
ending, which is most files). `G` moves to the final empty line. `cc`, `S`
and `Vc` then fail with a reversed range,
`InvalidRange { start: 4, end: 3, length: 4 }`, and so does `G V c` after an
indented line (`a\n\tb\n` gives `start: 5, end: 4`). Code mode handles the
same input. `o` works. Judging by the reported range, the change operator
computes the line's content range as `line_end..line_end - 1` when the line
is empty and last. That's inferred from the error, not traced.

### C62 (S1). Shift-selection fails right after Enter or `o` creates auto-indentation

Code, Plain Text or Markdown Source, `fn f() {\n    foo();\n}`. Put the
caret at the end of `    foo();` and press Enter in Insert mode, or use
`o`. The new line receives generated indentation (`    `). Shift-Right,
Shift-Left, Shift-Up, Shift-Down, Shift-Home and Shift-End then all fail
with `InvalidRange { start: 24, end: 24, length: 22 }`. The reported length
is the text length *without* the generated indentation, so selection
extension resolves the caret against a snapshot that doesn't include the
tentative indentation. Moving the caret first (Left), or Escape (which
removes the unused indentation), makes the selection work. In code
editing, Enter then Shift-Up is a common gesture. The walk fuzzer also
reached this from `:d` followed by `o` on a tab-indented line.

### C32 (S1). Setext headings can't be deleted or changed

WYSIWYG, `Setext\n===` or `Setext\n---`, cursor on the heading. These all
fail with `VerificationFailed`:

- `dd`, `D`, `cc`, `S`, `C`, `s`;
- `cw`, `ciw`, `dw`, `de`;
- `Vd`, `Vjd`, `dip`, `dap`, `das`, `d}`, `dG`, `dgg`;
- `:d`, and `dd.`.

`Setext\nline two\n===` additionally fails `o`, `A<CR>`, `yyp`, `yyP` and
`r<CR>`. With a following paragraph (`Title\n=====\n\nBody text.`) the
results are wrong rather than errors:

- `dd` "succeeds", but the source becomes `# Body text.`. The heading style
  moves onto the next paragraph instead of being deleted with its line.
- `cc` fails. The typed `x` then runs as the Normal-mode `x` command and
  deletes the `T` (`itle`). Any failed change operator turns the following
  insert text into Normal commands. This makes every insertion failure in
  this report potentially destructive.

Setext headings are common in older READMEs.

### C33 (S1). Code view: `:m`, `:t` and `:sort` always fail once highlighting arrives

Code (`x.rs`), `fn main() {\n    println!("hi");\n}\n` or even `a\nb\nc`.
After asynchronous syntax highlighting has been installed (about 100 ms
in the app), `:m+1`, `:t.` and (for the Rust sample) `:sort` fail with
`HardLineTransferProjectionMismatch`. They succeed only before
highlighting finishes, which is why a fresh-process test passes. The
fuzzer saw this only in long-running processes. Normal-mode
`ddp`/`yyp`/`>>`/`J` are unaffected.

Root cause: `transfer::projection_signatures`
(`src/core/document/transfer.rs:569`) includes every style span in the
per-line signature, including `StyleApplication::Automatic` syntax
decorations. `verify_projection` compares those signatures with a freshly
projected candidate, which never has syntax spans, so the comparison
always fails. AGENTS.md says syntax decorations are disposable and must
not affect document operations.

### C34 (S1). WYSIWYG `:t` and `:m` fail in any document with a code block, and on several constructs

`:t.` fails with `HardLineTransferProjectionMismatch` on:

- `\n` (an empty document with one line ending);
- `\na`;
- `a<br>b`;
- setext headings.

`:m+1` fails on `1) a\n2) b` (ordered list with `)` delimiters), `a<br>b`
and `a\n=\n\nb`. The probe saw 863 `:t.` failures and 481 `:m+1` failures
across the corpus.

The broadest case: in **any** WYSIWYG document containing a fenced code
block (`` ```rust\nfn x() {}\n```\n\npara ``), `:t.` and `:m-2` fail on
*every* line, including `para` outside the block, whether or not
highlighting has arrived. `:m-2` on the list item in
`# Title\n\nSome **bold** text.\n\n- a\n- b` fails too. This class was
the largest walk finding (about 30,000 hits). The signature comparison in
`transfer::verify_projection` probably includes code-block-specific block
or style attributes (language label, rich Code styling) that a fresh
candidate reconstructs differently. C33 is the same comparison failing
because of automatic syntax spans.

### C35 (S1). Source view: `o` in a table's last row

Source view, `| a | b |\n| - | - |\n| 1 | 2 |`, cursor on `1` (offset 21).
`o` fails with `InvalidRange { start: 30, end: 30, length: 29 }`, an
off-by-one past the end of the document. With the cursor on the leading
`|` it works. Tables in lists and quotes behave the same way.

### C36 (S1). `O` between consecutive ATX headings

WYSIWYG, `# A\n## B\n### C\n#### D\n##### E\n###### F`. With the cursor on
`B`, `O` fails with `VerificationFailed`. The three-heading version works.
`O` also fails in `\na` (cursor on `a`) and on the second of two
reference definitions.

### C37 (S2). Table rows: `J`, `gJ` and `2dd` report an internal layout error

WYSIWYG tables (plain, in lists, in quotes). On the header row, `J`, `gJ`,
`J.` and `2dd` fail with `Error("layout motion failed: TextDoesNotMatchLayout")`.
That is an internal invariant message. Either the command should explain
that cells can't be joined, or it should act on cells. Other linewise
commands give the proper "Editing across table cell boundaries requires a
cell selection or a row action." message (see Q5).

### C38 (S2). Linewise put fails after deleting or yanking several constructs

WYSIWYG:

- `` ```\ncode\n``` ``: `dd` leaves an empty fence. `p` then fails with
  `FormattedPayloadCannotReproject` (see also C4).
- `**[bold link](u)**`: `dd` leaves the source `**[](u)**`, an invisible
  empty link inside strong that stays in the file. `p` then fails.
- `# Heading #`: `yyP`, `yyp`, `ddP` fail with `FormattedPayloadCannotReproject`.
- `` ``code with ` tick`` ``: `yyp` fails with `UnsupportedFormatting`.
- `\na` and `` `**not bold**` ``: `yyp` fails.

Duplicating a line with `yyp` is one of the most common Vim idioms.

### C40 (S2). `>>` on reference definitions

WYSIWYG, `[ref]: /url "title"\n[other]: <x y>`, `[ref]\n\n[ref]: /url` or
`[a][ref]\n\n[ref]: https://example.com`. With the cursor at 0, `>>` fails
with `VerificationFailed`.

### C55 (S2). Replacing the whole text of a heading or item inside a quote or list corrupts the markers

WYSIWYG:

- `> # h`: `cc x Esc` produces source `> # > x`. It inserted a spurious
  `> ` *inside* the heading.
- `1. > heading`: `cc x Esc` produces `1. > > x`.
- `- > # h`: `cc x Esc` produces `- > # > x`.
- `> - > - > a\n> - > - > b`: `C x Esc` on the first item produces
  `> - > - x> \n…`, and a fresh parse shows `x> ` where the incremental
  projection shows `x`.
- `1. > # heading\n   > text\n2. b`: `cc`, `S`, `ciw`, `Vjc`, `O` produce
  `1. x> # \n…`. Fresh text `x> # ` differs from incremental `x`, so the
  markers become literal text on reopen.

Typing over a double-clicked word that is the paragraph's whole text does
the same, and sometimes worse:

- `- # heading`: double-click `heading`, type `x`. The source becomes `# x`
  and the list item is gone.
- `1. > # heading` gives `> # x`, dropping the ordered list.
- `1. > heading` gives `> 1. x`. The list and the quote swap nesting.
- `1. > # heading\n   > text\n2. b` gives `1. x> # \n…`, as above.

`> # heading` and plain `# heading` work. AGENTS.md says a whole-paragraph
replacement keeps the first paragraph's style. Here the list or quote
container is lost or reordered.

A related parser incompatibility: `> # > x` displays as a heading with text
`x`, treating `> ` inside the heading content as a nested quote. GitHub
renders a heading whose text is `> x`, and Viem itself parses top-level
`# > x` as `> x`.

### C59 (S2). `:global` is superlinear and freezes the editor on modest files

Release build, Plain Text, `alpha line\nbeta line\n` repeated, `:g/al/d`
(deleting every other line):

| lines | Plain | Code | Source | WYSIWYG (blank-line paragraphs) | `:g/al/s/a/b/` (Plain) | `:%s/a/b/g` (Plain) |
|---:|---:|---:|---:|---:|---:|---:|
| 1,000 | 4.0 s | 4.1 s | 3.2 s | 6.1 s | 11.5 s | 0.13 s |
| 2,000 | 10.0 s | 10.5 s | 7.9 s | 19.1 s | 26.0 s | 0.15 s |
| 4,000 | 24.4 s | 24.7 s | 18.4 s | 71.0 s | 67.5 s | 0.24 s |

Times roughly double with each doubling in size, and WYSIWYG is close to
quadratic. The whole command runs synchronously on the main thread, so the
app beach-balls. The walk fuzzer saw 1–3 s `:g/a/d` turns on the 11–36 KB
documents in `docs/`. AGENTS.md requires `:global` work to be bounded.
Each target line apparently runs as a full transaction with reprojection
and verification, instead of composing the per-line edits into one
transaction.

The next four entries were checked against Vim 9.1 (`/usr/bin/vim -u NONE`
with `ai et sw=2 sts=2 ts=2 bs=indent,eol,start`). The fuzzer's new
line-count check found them: in literal views, line-local commands such as
`x`, `cc`, `ciw` and `gUU` must not change the number of source lines.

### C70 (S2). Linewise commands on the last line drop the file's final newline

Most files end with a line ending, and these commands misbehave on their
last line. With `a\nb\nc\n` and the caret on `c`:

| Command | Vim | Viem |
|---|---|---|
| `dd` | `a\nb\n` | `a\nb` (Plain, Code, Source) |
| `dj` from `b` | `a\n` | `a` |
| `ddp` from `b` (swap the last two lines) | `a\nc\nb\n` | `a\nc\n\nb` |
| `yyp` | `a\nb\nc\nc\n` | `a\nb\nc\n\nc` (Plain, Source): the copy goes after a blank line and the file loses its final newline |
| `ccx<Esc>` | `a\nb\nx\n` | `a\nb\nx` (Code), `a\nbx\n\n` (Source, see C67) |
| `2ccx<Esc>` on `    b();` in `if a {\n    b();\n}\n` | `if a {\n    x\n` | `if a {\nx` (Code) |
| `J` | no change | `a\nb\nc` (Code). Plain leaves it alone. |

`dd` on `b` in `a\nb\n` gives `a`, and Plain `ccx<Esc>` on an indented
last line (`class A:\n    pass\n`) gives `class A:\nx`. AGENTS.md requires
"Preserve each existing delimiter and final-terminator presence". Plain
and Code also disagree on `ccx<Esc>` and `J`. The cause looks related to
C50 and Q18: the final line ending is modeled as an extra empty last line,
and the linewise range for the line before it takes both line endings.

### C71 (S2). `iw`/`aw` on whitespace cross line breaks

`word_class` (`src/core/command/text_object.rs:138`) treats `\n` as
whitespace, so the whitespace run under the cursor extends across hard
lines. In Vim, these objects stay within the line.

| Text, caret | Command | Vim | Viem |
|---|---|---|---|
| `x\n   b\n`, in the indent | `diw` | `x\nb\n` | `xb\n` |
| same | `daw` | `x\n\n` | `x` |
| `x  \n   b\n`, in the trailing spaces | `diw` | `x\n   b\n` | `xb\n` |
| `x\n\ny\n`, on the empty line | `diw` | no change | `xy\n` |
| same | `viwd` | `x\ny\n` | `xy\n` |
| `fn main() {\n    println!("hi");\n}\n`, in the indent | `ciwx<Esc>` | `fn main() {\nxprintln…` | `fn main() {xprintln…` |
| same | `yiwP` | eight spaces of indentation | inserts `\n    `, a new line |
| `a\n\n\nb\n`, on the second empty line | `ciwx<Esc>` | `a\n\nx\nb\n` | `axb\n` |
| same | `cwx<Esc>` | `a\n\nx\nb\n` | `a\n\nxb\n` |

In Source view, `diw` in the indentation of `x\n\n   y` gives `xy`, merging
the two paragraphs. The probe found these in every literal format and seed
with indentation or blank lines.

### C72 (S2). `cc` and `S` discard the line's indentation

With `autoindent` on (the default), Vim's `cc` keeps the first line's
indentation. Viem removes it. `if a {\n    b();\n}\n`, `S` (or `cc`) on the
second line, then `x`, gives `if a {\nx\n}\n`. Vim gives
`if a {\n    x\n}\n`. Tab indentation and Plain Text behave the same
(`  a\n  b\n` gives `  a\nx\n`). In code, `cc` to rewrite a statement is
very common, so every rewritten line has to be re-indented by hand. No
test covers `cc` with indentation.

### C73 (S2). Source view: linewise case operators merge a paragraph with the next block

Source view, `a\n\nb`. `gUU` (or `VU`) on `a` gives `A\nb`, so the blank
line is gone and the two paragraphs become one. `gUj` gives `A\nB`, and `guu`
on `# **Bold** heading\n\ntext` gives `# **bold** heading\ntext`. The
operator rewrites the line together with its following paragraph separator
and writes the projected text, which has one line ending, back over the two
source line endings. When the case doesn't change, the source is left
alone. In WYSIWYG, `gUU` on `x\ny\n\nb` gives `X Y\n\nb`: the soft line
break is respelled as a space, a patch outside the requested change.

WYSIWYG has a worse case. In `` - a\n  ```\n  code\n  ```\n- b ``, `gUU` (or
`g~~`) on `code` gives `` - a\n  ```\n  CODE\n  b\n  ``` ``. The next list item is
pulled inside the code block, and the closing fence moves below it. `gUiw`,
`~` and `vU` on the same word work.

### C74 (S2). `>>`, `<<` and `=` ignore the indentation options in the app

The app sends keys through the layout-aware path
(`Core::handle_with_layout`). There, the line operators use fixed values in
`src/core/command/line_mode.rs:668-677` and `:1324`:

- `>>` always inserts four spaces, whatever `shiftwidth`, `expandtab` or
  `tabstop` say. With the defaults (`sw=2 et`), `>>` on `a` gives `    a`
  (Vim gives `  a`). `V>.` gives eight spaces.
- `>>` on a tab-indented line puts the spaces before the tab:
  `\tb` becomes `    \tb`, mixing indentation.
- `<<` removes up to four leading *spaces* and ignores tabs. `\t\tb` is
  left unchanged, while Vim (`ts=2`) gives `  b`.
- `=` removes all leading whitespace. AGENTS.md says `=` "copies preceding
  nonblank indentation". `  a\n      b` with `==` on the second line gives
  `  a\nb`, where the spec gives `  a\n  b`.

Under Visual line meaning (the default outside Code), `visual_indent_edits`
inserts the spaces at the start of each *display row*, which on a wrapped
line is mid-sentence. Its row filter (`row.text_range.end < range.start`)
also includes the previous row, whose end touches the current row's start.
Take a 71-character line that wraps into three rows at 300 points. `>>` on
the second row gives `    The quick brown fox jumps     over the lazy dog…`,
and on the third row gives `…fox jumps     over the lazy dog and keeps     running…`.
Lines that don't wrap aren't affected.

The unit test `indent_outdent_and_reindent_are_undoable_line_edits` drives
the interpreter without a layout. That takes the options-aware path, where
`>>` adds two columns, so the test passes. The fuzzer and the app see the
fixed-width path. In Code, where `>>` and `<<` are used constantly, every
shift is wrong.

### C75 (S1). `e` and `E` don't move from the last character of a word

In `alpha beta gamma` with the caret on the final `a` of `alpha`, `e` stays
put, and pressing it again still doesn't move. Vim goes to the end of `beta`.
`E` does the same, and `2e` moves only one word. This happens in every
format. So `e`, the most common way to step through words, gets stuck after
its first use. `de` and `ce` from a word end affect only that one
character: `de` gives `alph beta gamma` (Vim: `alph gamma`).
`move_word_end` (`src/core/command/text.rs:364`) extends to the end of the
word under the caret without first stepping forward one character, which
Vim always does. No test covers forward `e`.

### C76 (S2). Deleting the text of a one-column header-only table deletes the table

WYSIWYG `| a |\n| --- |`. The table spec says deleting the last body row
"leaves a valid header-only table", and that Delete and Backspace "clear
selected cell contents, but retain every cell, row, the header". But `x`
on `a`, `A` then Backspace, or `ciwz<Esc>` on a longer header removes the whole
table. The source becomes empty, and with `ciw` the typed `z` is lost too.
In `|a|\n|-|\n\ntext` the table disappears and `\n\ntext` remains. `~` and
`rz` on that single character also replace the table with the bare letter.
Two-column header-only tables and one-column tables with a body row
behave correctly. To reach this state, delete the body row of a
one-column table, then edit its header.

### C77 (S2). Other operator and motion differences from Vim in literal views

These came from the Vim differential (`vimcases` plus
`tools/vim_diff.py`), and each was rechecked by hand:

| Text, caret | Keys | Vim | Viem |
|---|---|---|---|
| `one two\nthree\n`, on `two` | `dw` | `one \nthree\n` | `one three\n` (joins the lines) |
| same | `ywP` | `one twotwo\n…` | `one two\ntwo\n…` (yanks the line ending) |
| same | `2dw` | `one \n` | `one ` |
| `one two\n  three\n`, start of line 2 | `db` | `one \n  three\n` | `one   three\n` (joins the lines) |
| `alpha\n`, on the last `a` | `dl`, `yl`+`p`, `clx<Esc>` | `alph`, `alphaa`, `alphx` | no change, no change, `alphxa` |
| `alpha beta\n`, on the last `a` of `alpha` | `r<Enter>` | `alph\nbeta\n` | `alph\n beta\n` (keeps the space) |
| `one.\ntwo\n` | `v$d` | `two\n` | `\ntwo\n` (`$` doesn't select the line ending) |
| `p1 a.\np1 b.\n\np2\n` | `dip` | `\np2\n` | `\n\np2\n` (leaves an empty line) |
| `ab\n\ncd\n`, on `a` | `yl`, `j`, `p` | `ab\na\ncd\n` | `ab\n\nacd\n` (the text goes to the start of the *next* line) |
| `{\nab\n`, on `{` | `xp` | unchanged | `\n{ab\n` |
| `a\n   \n`, on the blank line | `Ix<Esc>` | `a\n   x\n` | `a\nx   \n` (`I` goes to column 0, not after the blanks) |
| same | `I<BS><BS><Esc>` | `a\n\n` | `   \n` (the first Backspace joins the lines, and the second deletes `a`) |
| `ab\ncd\n`, on `b` | `~x`, `~.`, `~i!<Esc>` | `a\ncd\n`, `ab\ncd\n`, `a!B\ncd\n` | `aB\ncd\n`, `aB\ncd\n`, `aB!\ncd\n` (`~` leaves the Normal caret on the line ending, so `x` and `.` do nothing) |
| `ab\n\ncd\n`, on `a` | `w` (also `W`; `b`/`B` from `c`) | stops on the empty line | skips to `cd` (Vim counts an empty line as a word) |

`dw` on the last word of a line and `p` on an empty line are the important
ones. Vim has a special rule for `dw` at the end of a line (`:help word`),
because deleting a line's last word is so common. Viem joins the next line
instead, and also removes its indentation. Characterwise `p` on an empty
line (Plain and Code) inserts after the line ending instead of on the empty
line, so yanking a word and putting it on a blank line merges it into the
following line. `P` works. `db` at the start of a line is Vim's
"exclusive motion ending in column 1" rule (`:help exclusive-linewise`).
`dl` at the end of a line fails because `l` can't move past the last
character, even in operator-pending mode, where Vim allows it.

### C79 (S2). Wrapped lines: `J` does nothing, `yyp`/`ddp` split words, and `cc`/`dd` merge lines

Plain Text, WYSIWYG and Source use Visual line meaning by default, and there
`J` joins the current *display row* with the next one. When the current hard
line soft-wraps, the next row belongs to the same line, so `J`, `2J`, `VJ`
and `gJ` succeed but change nothing. For example, take a 71-character line
followed by `second line` in a 300-point-wide view. The line wraps into
three rows, and `J` on the first or second row is a silent no-op. Only on
the third row does it join `second line`. In WYSIWYG every paragraph is one
hard line, and prose paragraphs usually wrap, so `J` to join two paragraphs
works only from the paragraph's last row. With Physical line meaning, or
with a wide enough window, the same keys join as expected. AGENTS.md applies
the line meaning to line operators, but a join that can't change anything
should fall through to the next hard line, or at least report why nothing
happened.

Other line commands on an interior row of a wrapped line act on that row,
as specified, but the put half goes wrong. `yyp` and `Yp` yank the row's
text characterwise and then put it after the caret's character rather than
after the row. On the second row (`over the lazy dog and keeps `), the
result is `…jumps oover the lazy dog and keeps ver the lazy dog…`, which
splits a word. `ddp` gives `…jumps rover the lazy dog and keeps unning…`.
`yyP` happens to work there.

The *last* row of a wrapped line has a different problem: the row's range
includes the hard line ending. Take Plain `First paragraph.\nSecond line.`
in a 140-point view, where the first line wraps into `First ` and
`paragraph.`. On the second row:

- `cc x<Esc>` and `S x<Esc>` give `First xSecond line.`;
- `dd` and `Vd` give `First Second line.`;
- `yyP` gives `First paraparagraph.\ngraph.\nSecond line.`.

The first two merge the next line into this one. AGENTS.md says visual-row
deletion "invents no newline", and it shouldn't remove one that still
separates two lines with text either. `C` and `D` on the same row keep the
line ending. The narrow-width Normal-mode probe found the `cc` case with
the line-count check.

### C80 (S2). `:g/^$/d` leaves blank lines, and `:v/…/d` skips lines

Plain Text and Code. `:g/^$/d`, the usual way to delete blank lines, leaves
one line of every run of adjacent empty lines. `a\n\n\nb\n` and
`a\n\n\n\nb\n` both give `a\n\nb`, where Vim gives `a\nb\n`. `:v/a/d` on
`a\n\nb\n\n\nc\n` gives `a\nb\n\n`: it keeps `b` and an empty line,
where Vim gives `a\n`. Without empty lines in the target set, the same
commands work (`:g/x/d` on `a\nx\nx\nb\n`, `:v/a/d` on `a\nb\nc\nd\n`).
Targets are bound as line-start anchors (`ex_normal_targets`,
`src/core/command/mod.rs:13151`). Deleting one empty line apparently
collapses the next empty line's anchor, so that target is skipped.
AGENTS.md says `:global` targets keep stable identities.

Two smaller differences come from the final-line model (Q18). The empty
"line" after the final line ending matches `^$`, so `:g/^$/d`,
`:g/^\s*$/d` and `:v/\S/d` also delete the file's final newline
(`a\n\nb\n\nc\n` gives `a\nb\nc`). And `:%j` across empty lines leaves out
the separating spaces: `a\n\nb\n` gives `ab` (Vim: `a b\n`).

### C82 (S2). Visual `J` does nothing under Physical line meaning (the Code default)

In Code (`x.rs` or `x.py`), with `one\ntwo\nthree\n`, `VjJ` (select two lines,
join) leaves the text unchanged and stays in Visual Line mode. `VJ`, `vJ`
and `vlJ` do the same, and no status is reported. In Vim each joins into
`one two`. Normal `J` and Visual `gJ` work in Code. Switching the view to
Visual line meaning makes Visual `J` work. Plain Text switched to
Physical line meaning fails the same way, so the problem follows the line
meaning, not the format. The Visual `J` handlers (`visual_join` at
`src/core/command/mod.rs:10223`, `mode_visual_join` at
`src/core/command/line_mode.rs:1446`) both leave Visual mode, so under
Physical meaning the key apparently never reaches them.

## Formatting actions (toolbar, menus, shortcuts)

### C53 (S1). Bold or Italic on a selection fails while the core presents it as available

WYSIWYG. The harness only sends Bold or Italic when
`Core::selection_semantic_style_presentation` reports `can_set()`, which is
what the toolbar shows. These selections still fail with
`VerificationFailed`:

- **Select All in any multi-paragraph document**: `one\n\ntwo`,
  `# Title\n\nBody text here.`.
- **Any selection that starts or ends with whitespace**: `one two`
  selecting `one ` (0..4) or ` two` (3..7), or `one two three` selecting
  4..8. The naive `**one **two` can't close, because a closing delimiter
  can't follow a space. The edit needs to move the delimiters inside the
  whitespace, as word processors do. Drag selections routinely include a
  trailing space.
- Selections spanning two headings (`# A\n## B`, 0..3) or two list items
  (`- a\n- b`, 0..3).
- Selections crossing a soft line break that end just after a space
  (`Line one\nLine two`, 1..14).
- Selections starting at, or spanning, passive HTML (`<ins>…`, `<b>…`),
  entities (`&amp; &lt; …`, 0..2), reference definitions, front matter,
  thematic breaks, or table cells.

The probe saw about 6,800 Bold and 6,900 Italic failures across 170
documents. In Source view, Bold and Italic over a selection crossing a hard
break (`a\\\nb` or `a  \nb`, 0..2) fail with `UnsupportedFormatting`, again
while presented as available.

Related selection failures from the same probe:

- Source view, Enter over a selection inside emphasis (`*it*` 1..4,
  `**bold**` 1..8) fails with the internal error `InvalidRange`.
- Source view, deleting or replacing a selection that covers most of a
  pipe table (`a | b\n- | -\n1 | 2`, 0..14) fails with `VerificationFailed`.
- WYSIWYG, Default Paragraph (clear character style) on a selection at the
  start of `<ins>under</ins> …` fails with `VerificationFailed`.

---

### C41 (S2). Heading styles fail on many ordinary paragraphs

WYSIWYG. Applying Heading 1 (`SetParagraphStyle(Heading1)`, which is what
Cmd-1 and the paragraph menu send) with the caret in the paragraph fails
with `VerificationFailed` for:

- a paragraph with trailing spaces: `trailing spaces   `;
- paragraphs with a hard break: `a\\\nb`, `line  \n  \nafter double space`;
- multi-line paragraphs inside containers: `> a\n> b`, `> > nested\n> back`,
  `- a\n  b\n  c`, `- a\n\n  continued\n- b`, `- > quote in list`;
- a lazy continuation (`text\n    not code (lazy)`), and an inline code
  span across a line ending (`` `a\nb` ``);
- setext headings (`Setext\n===`, which should simply change level);
- thematic breaks (`---`, `***`, `___`, `- - -`), `****`, `** **`, `-`;
- whitespace-only paragraphs (`" "`, `"\t"`);
- reference definitions, HTML blocks (`<div>…`), unclosed comments, and
  front matter.

Heading 1 fails with `AmbiguousProjection` on `a<br>b` and on a fence
containing a blank line. The menu presentation enables headings whenever
the definition exists, so all of these beep with an internal message.

### C42 (S2). Bold or Italic, then typing, fails next to inline constructs

WYSIWYG. Toggle Bold (or Italic) at a caret, which sets a pending typing
style, then type `x`. This fails with `VerificationFailed` at:

- offset 0 of `` `code` ``, `` # `code` heading ``, `` [`code` link](u) ``,
  `` ``code with ` tick`` `` and `` `**not bold**` ``, i.e. directly
  before an inline code span;
- offset 0 of autolinks `<https://auto.link>` and `<mailto:a@b.c>`;
- inside HTML comments and blocks (`<!-- comment -->` at 4,
  `a <!-- inline --> b` at 6, `<script>…` at 8);
- the end of `trailing backslash\`;
- inside reference definitions;
- several table cells, for example after `` `a|b` ``.

Typing a bold word right before inline code is common.

### C43 (S2). Bulleted or Numbered list fails on several paragraph shapes

WYSIWYG, Bulleted List or Numbered List with the caret in the paragraph,
fails with `VerificationFailed` for:

- paragraphs containing a hard break (`a  \nb`, `a\\\nb`, `a <br/> b <br> c`);
- multi-line quote paragraphs (`> a\n> b`, `> # h\n> text`, `> > nested\n> back`);
- headings with a closing sequence (`# Heading #`, `# Heading ###   `);
- setext headings;
- a list item that already has continuation lines (`- a\n  b\n  c`);
- lazy continuations;
- whitespace-only paragraphs and `` ` ` ``;
- fenced code (`` ```js… ``, an unclosed fence);
- `<div>` blocks;
- a paragraph directly followed by a table.

Block Quote on fails for setext headings, `<div>` blocks, the paragraph
before a lazily continued table, deep nested lists, and code fences inside
lists or paragraphs.

### C44 (S2). Code character style, then typing, fails in several contexts

WYSIWYG. Code character style (the toolbar Code button) at a caret, then
type `x`, fails with `VerificationFailed`:

- at offset 0 of `` `unclosed code `` (462 probes, many positions);
- inside a bare URL (`https://bare.link.example` at 8);
- at the start of passive HTML (`<p align="center">x</p>`, `<ins>under</ins>…`,
  `<details>…`);
- inside deeper or over-indented list items;
- inside `**md inside html**` within an HTML block;
- in table cells containing escaped pipes.

It fails with `UnsupportedFormatting` inside `` **`code in bold`** ``.

In Source view, Code, Bold and Italic typing styles fail at many caret
positions:

- `UnsupportedFormatting` after a hard-break backslash (`a\\\nb` at 2) or
  after an escape (`\# not heading` at 1);
- `OverlappingFormatting` inside `~~strike~~`;
- `UnsupportedFormatting` for Bold and Italic inside a list item containing
  a fence.

C30 covers the hangs.

### C31 (S2). Code typing style inside a delimiter run deletes the delimiters

Source view, `**bold**`. Toolbar Code at the caret, then type `x`:

- At offset 1, between the opening asterisks, the result is `` `x`bold** ``.
  The opening `**` is gone and the bold is lost.
- At offset 7, between the closing asterisks, the result is `` **bold`x` ``.
  The closing `**` is gone.

That is silent data loss from an insertion. Other offsets work, for example
offset 4 gives `` **bo**`x`**ld** ``.

### C29 (S3). Bogus `BlockIdentityExhausted` error

Source view, `text before\n| a | b |\n| - | - |\n| 1 | 2 |` (a paragraph
directly followed by a table). With the caret at offset 0, Remove List or
Paragraph style fails with `Document(BlockIdentityExhausted)`. Identity
exhaustion can't really happen in a four-line document, so some
block-identity reconciliation path is returning the wrong error. The user
sees a misleading message, and the action does nothing.

### C64 (S2). Insert Link over a selection that partly overlaps emphasis

WYSIWYG, `one **two** three`. Select 2..6 (`e tw`, crossing into the bold),
open the link popup and confirm a destination. This fails with
`VerificationFailed`. So do `one *two* three` 0..6 and `a*b*c` 0..2.
Selections entirely inside or outside the emphasis work. Over a selection
that partly overlaps an existing link (`plain [link](u) end` 3..9), the
popup is refused with `UnsupportedFormatting`. Link insertion at a caret
or over a selection is also refused for:

- selections crossing a paragraph break;
- carets inside inline or fenced code, and on thematic breaks;
- (Source view) right after a hard-break backslash (`a\\\nb` at 2), inside
  `` `` `` and in reference definitions, where it fails with
  `VerificationFailed`.

Image insertion fails at the same places. The macOS link popup is enabled
for any Markdown caret or selection, so each of these is a beep and an
error message.

### C65 (S1). Native Copy then Paste fails next to or inside formatted text

WYSIWYG. A native Copy (Cmd-C) puts the core's portable register on the
clipboard. Pasting that payload back with native Paste (Cmd-V) at a caret in
Insert mode fails as follows:

- `**bold**`: copy `b` (0..1) and paste at 1, inside the same bold run.
  Fails with `UnsupportedFormatting`.
- `a**b**c`: copy `a` (0..1) and paste at 1, immediately before the bold.
  Fails with `UnsupportedFormatting`.
- `a\n***\nb`: copy `a` and paste at the start of `b`. Fails with
  `VerificationFailed`.
- `| **bold** | _it_ |…`: copy 0..9 across cells and paste at 0. Fails with
  `AmbiguousProjection`.

A copy/paste probe (64,309 copy-selection × paste-caret pairs over all
seeds; `fuzz_consistency probe --probes copypaste`) found:

- About 3,900 `VerificationFailed` results. The commonest is copying text
  that includes a folded soft line break (`a\nb`: copy `a ` (0..2), paste
  at 1). Copying an empty paragraph (`\na`: 0..1, paste at 0) fails too.
- About 2,600 `UnsupportedFormatting` results. Copying part of an emphasis
  run and pasting it inside the same run fails (`*it*`: copy `i`, paste
  at 1). So does pasting next to emphasis (`a*b*c`) or inside a code span
  across a line ending (`` `a\nb` ``).
- About 3,100 `AmbiguousProjection` results for pasting a range copied
  across table cells.
- Display/file mismatches. Pasting text that starts with a space at the
  start of a list item (`- [ ] task`, `- > quote in list`) shows the
  leading space, but the source (`-  [ ] …`) reparses without it.
  Pasting before a U+FEFF at the start of the document hits C28.

When it succeeds next to formatting, the spelling can change
unnecessarily. In `**bold**`, copying all of `bold` and pasting at its end
rewrites the source to `<strong>bold**bold**</strong>`, replacing the
authored `**` with HTML. AGENTS.md says canonicalization may replace only
the smallest construct the edit requires.

## Markdown Source view

### C24 (S2). Source view: deleting a table's last body row leaves a stale projection

Source view, `| a |\n| - |\n| 1 |`, caret in the body row, Table → Delete
Row. The source becomes `| a |\n| - |\n` with a trailing newline. The
incremental text is `| a |\n| - |`, but a fresh parse gives `| a |\n| - |\n`.
Every table spelling in the corpus does this, including tables in lists and
quotes, tables followed by a lazy paragraph, and two-table documents. Source
Shift-Enter at the end of a table's last row shows the same text mismatch.

### C25 (S2). Source view: Enter then Backspace in a fence opener corrupts block ranges

Source view, `` ```\ncode\n``` ``. At offset 1, press Enter
(`` `\n``\ncode\n``` ``), then Backspace. The source returns to
`` ```\ncode\n``` ``, but the incremental projection's Code Block covers
`0..8` while a fresh parse gives `0..12`. The same happens with `~~~` fences,
`<div>\nblock\n</div>` and `` text\n```\ncode\n```\ntext ``. Later edits and
layout then run against wrong block boundaries. Typing does it too. In
`` ```\n\n\n``` ``, `ixy<BS><BS>` on the third backtick restores the source
exactly, but the incremental text is `` ```\n``` `` while a fresh parse
gives `` ```\n\n\n``` ``. The blank code lines stay hidden.

### C26 (S1). Source view: Enter in a list item containing a table

Source view, `1. a\n\n   | x | y |\n   | - | - |\n   | 1 | 2 |\n\n2. b`. Enter at
offset 8, at the boundary before the table inside item 1, fails with
`AmbiguousProjection`. Several Backspace and Delete positions around the
table give block mismatches against a fresh parse. In WYSIWYG, inserting
or deleting table rows and columns in this table leaves the block list
stale (`reproject_blocks`).

### C27 (S1). Source view: Enter then Backspace in an indented code block

Source view, `    a\n\n    b`. At the end (offset 11), Enter inserts
`\n    `, an auto-indented line. Backspace then fails with
`NotGraphemeBoundary(14)`, a coordinate error that should never reach the
user. The caret is at 16 and the error names 14, so a stale or mismapped
offset is involved.

### C39 (S2). Source view: tables inside list items leave a stale projection after most edits

Source view, `- item\n\n  | a | b | c | d |\n  |---|---|---|---|\n  | 1 | 2 | 3 | 4 |\n\n- next`
or `1. a\n\n   | x | y |\n   | - | - |\n   | 1 | 2 |\n\n2. b`. Almost any edit
inside the table leaves the incremental block list different from a fresh
parse: `x`, `X`, `3x`, `dw`, `de`, `cw`, `s`, `vd`, `<<`, `==`, `i<BS>`,
`a<Del>`, Ctrl-V block delete, and undo/redo of those. For example `x` at
offset 14 reports the header row as `Paragraph style "Table header"`, but
a fresh parse makes it part of the `ListItem`. Later structural commands
then act on the wrong blocks.

### C67 (S1). Source view: emptying a line in a paragraph sends the next keys to the following line

Markdown opens in Source view, so this affects the default editing view.
Take `first line\nsecond line\nthird line` with the caret on `second line`.
`cc` (or `S`, `0C`, `0d$`), then typing `new`, gives
`first line\n\nnewthird line`. The expected result is
`first line\nnew\nthird line`. Once the line is empty, it's a blank
separator line. Source view doesn't give blank separators a text row, so the
emptied line leaves the text projection (Q7). The caret falls onto the next
line, and the typed text is prepended to it. The paragraph is split and the
new text joins `third line`.

The same happens when the line is emptied character by character. On
`a\nbb\nc`, `xxx` deletes `b`, `b`, and then the `c` on the next line. Vim
would stop at the empty line, and holding `x` to clear a line is a common
habit. More shapes:

- `a\nbb\n\nc` (last line of a paragraph): `cc z` gives `a\n\n\nzc`, and
  `A<BS><BS>` gives `ac`, merging the two paragraphs.
- `> a\n> bb\n> c`: `cc z` gives `> a\n\nz> c`. The typed text goes before
  the next line's quote marker.
- `- a\n  bb\n  c`: `cc z` gives `- a\n\nz  c`, and `xxxiz` gives
  `- a\n  \n z `, deleting `c`.
- The last line before a final newline goes the other way. `a\nb\nc\n`
  with `cc x` on `c` gives `a\nbx\n\n`, and `- a\n  bb\n` with `cc x` on
  `  bb` gives `- ax\n\n`. The typed text is appended to the *previous* line.

A paragraph's last line with nothing after it (`a\nbb`), and single-line
paragraphs between blank lines (`a\n\nb\n\nc`), work because the emptied
line stays an editable empty paragraph. WYSIWYG and Plain Text work.
Replace-mode Tab over a line's only character hits the same problem
(C68). Fix direction: an emptied line should keep a text row and the caret
until the user leaves it, the way `o<Esc>` already leaves an editable empty
paragraph.

### C68 (S2). Replace-mode Backspace destroys source after a replacement changes Markdown structure

AGENTS.md says Replace Backspace restores the overwritten source. That holds
in ordinary prose. In Source view it fails when a replaced character changes
the block structure, because the restoration journal stores offsets that no
longer mean the same thing after reprojection. Undo still recovers the
original, but Backspace makes the damage worse. Found by the new restoration
probe (`R…<BS>…<Esc>` must leave the source unchanged):

- Fence opener, Source `` ```\n\n\n``` ``, caret on the third backtick:
  `Rxy<BS><BS><Esc>` gives `` ``x`` ``. After `x` breaks the fence, the
  blank lines collapse and the caret moves to the next line. `y` then
  overwrites the closing fence. The two Backspaces delete the blank lines
  and join the fence lines.
- A line's only character, Source `a\nb\n\nc` with the caret on `b`:
  `R<Tab><BS><Esc>` gives `ac`. The Tab turns the line into whitespace, the
  line leaves the projection (C67), and Backspace joins `a` and `c`,
  deleting `b` and three line endings. `- a\n  b\n  c` gives `- a  c`, and
  `> a\n>\n> b` (caret on the lone `>`) gives `> a> b`.
- An indented code line, Source `    a\n\n    b`, caret on the second
  space of `    b`: `Rxy<BS><BS><Esc>` gives `    a\n\n  b`. Two spaces are
  lost because the line stopped being indented code mid-replacement.
- Source `\n`: `R<Enter><BS><Esc>` gives an empty file. Source `   \n  \n`
  gives `   `, deleting the second line.

In WYSIWYG the same sequences mostly hit errors already listed:
`*it*` `R<Enter><BS>` is C7 (`VerificationFailed`), `\na` `R<Enter>` is C16,
and `# Heading #` `R<Enter>` is C20. Successful restorations in WYSIWYG
sometimes re-spell the source: `&amp;` comes back as `&`, `\*` as `*`,
`\[` as `[`, a soft break as a space, and a two-space hard break as `<br>`.
The text has the same meaning, but the spelling isn't restored, which
AGENTS.md asks for.

### C69 (S2). Replace-mode Backspace and Ctrl-W delete original text before the replacement started

All formats. In Vim, Backspace, Ctrl-W and Ctrl-U in Replace mode only undo
the replacement. Before the point where Replace started, they move the
caret without deleting anything. In Viem they delete the original text:

- Plain `First paragraph.`, caret on the second `p`: `Rxyz` then five
  Backspaces gives `First paragph.`. The first three restore `ph.`, and the
  next two delete `ra`.
- `Rxyz<C-w>` at the same place gives `First ph.`. Ctrl-W restores `ph.`
  but also deletes `paragra`, the rest of the word before the replacement.
  Vim gives `First paragraph.`. The probe found this at almost every
  mid-word position in every format (more than 1,200 hits).
- In Source, `a<br>b` with the caret on `r`: `Rxyz<C-w>` gives `a<r>b`.

`replace_delete_motion` (`src/core/command/insert_controls.rs:397`) walks
the replacement journal and then calls `delete_insert_mode_range` on
whatever is left of the word. Ctrl-U is already clipped to the session start
by `ctrl_u_delete_range_since_snapshot`. Related, and lower priority:
Insert-mode Ctrl-W ignores the Insert session start too. `ixyz<C-w>` in the
middle of `paragraph` deletes `paragraxyz`, while Vim's default
`backspace=indent,eol,start` stops once at the session start.

### C47 (S3). Source view: Indent over a selection computes a reversed range

Source view, `a  \nb` or `a<br>b`. Select 2..3 (pointer at 2, extend to 3)
and choose Increase Indent. It fails with
`InvalidRange { start: 2, end: 1, length: 3 }`. The paragraph isn't a list,
so the action should be unavailable or refused with a clear message. A
reversed range indicates a selection-to-line mapping bug in the indent
path.

### C58 (S2). Source view: Enter or Indent over a selection computes reversed ranges

Source view. Enter replacing a selection fails with
`InvalidRange { start > end }` for:

- `a\n- \nb` (selection 4..6, across an empty list item);
- `*\ta\n*\tb` (1..7);
- `*it*` and `**bold**` (see C53).

Increase Indent over `x<br/>\ny` (2..3) or `![a](u "t")![b](v)` (1..3)
fails the same way. In
`` - a\n\n- 👩‍💻\n  > q\n\n  ```\n  c\n  ```\n\n- b ``, Tab (structural indent)
over a selection from the end of `👩‍💻` down into the quote fails with
`VerificationFailed`. Starting the selection from Insert mode with
Shift-Down gives a raw `NotGraphemeBoundary(17)` instead. 17 is a valid
text offset but falls inside the emoji in the source, so a text offset is
being used as a source offset. `a|b\n-|-\nc|d` with `o` on the last row is C35.

## Layout, zoom and resize

### C49 (S2). Zoom and Resize fail with layout errors in table documents

These are reported as errors (beep plus message), and the zoom or size
change doesn't happen.

- WYSIWYG `| a | b | c |\n| :-: | --- | ---: |\n| 1 | 2 | 3 |`: Resize to
  120 × 200, Select All, then zoom to 50% (`SetScale(0.5)`). This fails
  with `Layout(OutsideMaterializedCoverage)`.
- WYSIWYG `| a | b |\n| -- | - |\n| é👩‍💻 | العربية |`: after a Shift-Down
  selection and some edits, zoom to 200% fails with
  `Layout(NotACaretStop { text_offset: 2 })`.
- WYSIWYG table documents after pasting multi-line text over a word
  selection: Resize to 300 × 900 fails with
  `Layout(OutsideMaterializedCoverage)`. `| a | b |\n| - | - |\n| 1<br>2 | 3 |`
  and the two-table documents fail too.

Zoom and Resize are presentation-only. They should never fail because a
selection endpoint or the caret has no geometry; they should recompute
coverage. A failing Resize also blocks the frontend's layout refresh.

## Projection consistency and real documents

### C23 (S2). Autoformatting `- ` at the start of a list item gives a stale projection

WYSIWYG `- a`. At offset 0, type `-` then a space. The source becomes
`- - a`, a nested list, but the incremental projection still reports the
block as `ListItem { level: 0 … }`. A fresh parse of the saved bytes says
`level: 1`. The display and later structural edits use the wrong nesting
until the document is reopened.

The same mismatch appears for `0. zero`, `1) a\n2) b`, `- a\n1. b` and
`-\ta\n-\tb`. In the nested case, typing `#` + space or `>` + space at the
same place reaches the same wrong block state.

Related inconsistency: typing `1. ` at the start of `- a` does not
autoformat. It writes `- 1.&#32;a` instead (see Q4).

### C28 (S2). Deleting text before a U+FEFF at the start of the document

`a\u{feff}b` in WYSIWYG or Source. Backspace after `a` leaves the source and
text `\u{feff}b`. A fresh parse of those bytes treats the leading U+FEFF as a
byte-order mark and drops it, giving text `b`. The incremental projection
still shows the character. On save and reopen, the zero-width no-break space
silently becomes a BOM: the content changes, and the BOM state differs from
what the edit implied.

### C56 (S2). More stale-projection cases from degenerate input

The incremental projection differs from a fresh parse after the edit, so
the display and later edits use a projection that differs from the file.

- Tab after a list marker (`*\ta\n*\tb`, `-\tfoo\n\n\tbar`): typing `- `,
  `# ` or `> ` at offset 0 leaves stale list blocks. `cc`, `S`, `ciw` on
  `bar` do the same.
- `1. a\n\n\n2. b`: typing `- `, `# ` or `> ` at offset 0.
- `- a\n\n      indented code in list\n- b`: `o x Esc` or `O x Esc` on the
  code line.
- `` > ```\n> unclosed fence in quote\n\nafter ``: `dip`, `das`, `vipd` on the
  fence body.
- `1. one\n1. one\n1. one\n\n   para in third`: `Vjcx` or typing over a 0..11
  selection.
- Footnote definitions (`foot[^1] note\n\n[^1]: The note.`): deleting or
  replacing part of the definition text leaves style spans that differ from
  a fresh parse. So does a reference definition with its destination on the
  next line (`[ref]:\n/url\n\n[ref]`, `C x`).
- From the second walk campaign, each needing no more than three actions
  after opening:
  - `<!-- unclosed comment`: `o`, then type `---`.
  - `1. > # heading\n   > text\n2. b`: double-click a word, then `O` (C55).
  - `[ref]\n\n[ref]: /url`: `A`, then type `]` and `é`.
  - `a\n- \nb`: `dw` on the empty item.
  - The most common new variant is C23 with `1. ` instead of `- `. Typing
    `1. ` at the start of any list item (`- a\n  b\n  c`, `1) a\n2) b`,
    `- عربي`) leaves a stale projection.

### C63 (S2). Failures on the project's own `docs/markdown_demo.md` (WYSIWYG)

Probing the demo at sampled positions reproduced several of the classes
above on real content:

- Bulleted or Numbered list and Heading 1 fail at the setext headings
  (offset 2436; C32, C41).
- Enter fails inside the inline code span
  `` `../examples/markdown-image-flow.png` `` (offset 5354; C6).
- Shift-Enter fails inside the standalone HTML comment (offset 7796; C18).
- Right, Left, word motions and paragraph motions in the large table land
  on offsets without caret geometry, around 8384–8463 (C22).
- `dd`, `D`, `J`, `V…d` and `:d` fail with `VerificationFailed` at offset
  6819, inside the two-space-indented fence. Minimal form:
  `` ` ```\n  This fence is indented.\n    Two additional spaces.\n  ```\n\n    Four spaces code.` ``.
  `dd` on either fence line fails whenever the fenced block is followed by
  an indented code block. `D`, `x` and `J` work.
- `dip`, `dap` and `vipd` on the first heading fail with `AmbiguousProjection`.
- `:g/a/d` takes over a second and then fails with `VerificationFailed`;
  `:t.` and `:m+1` fail with `HardLineTransferProjectionMismatch`.
- `README.md`: `:sort` fails with `UnsupportedFormatting`.

## Lower-priority findings with exact traces

These are real failures, but they need several specific steps. Each trace
is the JSON for `fuzz_consistency script FORMAT WIDTH SOURCE ACTIONS`; the
first `resize` action sets the view size.

- **L1. Pending Italic plus Enter at a list marker (Source).**
  Source `<ins>unclosed`.
  `[{"op":"paste","text":"- x\n- y\n"},{"op":"key","key":"O"},{"op":"text","text":"العربية"},{"op":"place","offset":1,"downstream":true,"extend":false},{"op":"semantic","style":"emphasis","enabled":true},{"op":"key","key":"Enter"}]`
  gives `Document(NotGraphemeBoundary(10))`, a raw coordinate error.
- **L2. Replacing a selected paragraph break after `dd.` and `r<CR>` (WYSIWYG).**
  Source `&bogus; &#xZZ; &`, view 700 × 1200 and later 120 × 30.
  `[{"op":"key","key":"i"},{"op":"text","text":"q"},{"op":"resize","width":120,"height":30},{"op":"word","offset":1,"extend":true},{"op":"key","key":"a"},{"op":"text","text":"---"},{"op":"text","text":"q"},{"op":"key","key":"Esc"},{"op":"key","key":"d"},{"op":"key","key":"d"},{"op":"key","key":"."},{"op":"key","key":"r"},{"op":"key","key":"Enter"},{"op":"key","key":"O"},{"op":"key","key":"S-Left"},{"op":"text","text":"é"}]`
  gives `Document(NotGraphemeBoundary(1))`.
- **L3. Visual Block delete in a table cell containing a `<br>` line and
  RTL text (WYSIWYG, width 700).** Source
  `| a | b |\n| - | - |\n| 1 | 2 |\n\ntext after`.
  `[{"op":"key","key":"a"},{"op":"key","key":"Enter"},{"op":"text","text":"العربية"},{"op":"key","key":"Esc"},{"op":"key","key":"C-v"},{"op":"key","key":"d"}]`
  gives `LayoutMotion(OutsideMaterializedCoverage(…))`. Simple RTL
  Visual Block deletes work.
- **L4. Very short viewports.** At a view height of 30, the content insets
  exceed the height. After Escape, the caret at offset 0 isn't inside the
  materialized coverage (`caret_not_materialized`), in WYSIWYG
  `**`code in bold`**` and Source `` ```\na\n\nb\n```\ntext ``. Below the
  minimum pane width it gets worse. WYSIWYG `- | a | b |\n  | - | - |\n  | 1 | 2 |`
  at 40 × 30: Table → Delete Row, scroll, then every later Resize (even
  to 300 × 200) fails with `LayoutInstall(HeightIndex(InconsistentLayoutSnapshot("a materialized viewport must contain visual rows")))`.
  The view never recovers. At a width of 80 or more, it does.
- **L5. `:s` with a hard break inside an inline code span.** WYSIWYG
  `` `a b c` ``, `:s/ b /\r/` fails with `FormattedPayloadCannotReproject`.
  It could split the span or use `<br>`. `:s/ b /\n/` (literal LF) fails
  in every WYSIWYG context; that may be intended, but the error should
  say the character isn't representable rather than
  "candidate source did not reproduce…".
- **L6. Insert-mode Ctrl-W ignores the Insert start.** See the end of C69.
  `ixyz<C-w>` inside `paragraph` deletes `paragraxyz`. Vim's default
  `backspace=indent,eol,start` stops once at the start of the insertion,
  and Viem's own Ctrl-U already does.
- **L7. A rejected composition leaves an unexportable selection.** When a
  committed composition over a selection is refused (for example
  `<ins>unclosed` with `unclosed` selected, `VerificationFailed`), the view
  stays in the selection, but its selection export fails with
  `InvalidTextOffset(8)` and the caret has no geometry. Under H4 the
  macOS refresh fails until the next key.

---

## Open questions (decide the intended behavior)

These are behaviors I couldn't classify as bugs from AGENTS.md, the table
spec or the code. Each needs a decision.

1. **Enter at a soft line break.** WYSIWYG `a\nb`, Enter right after `a`,
   gives source `a\n\n&#32;b`. The folded line ending becomes a leading
   space in the new paragraph, protected with a numeric reference. Should
   splitting at a folded break drop that space, as most word processors do
   with line wraps?
2. **Deleting the last character of a continuation line.** `ab\nc` with
   Backspace after `c` leaves `ab ` with a visible trailing space, and
   `ab\nc\nd` gives `ab&#32;\nd`. Should the folded break be removed with
   the line's last character?
3. **Source Enter inside a table row.** Source view, Enter in the middle of
   `| 1 | 2 |` inserts `\n\n`, which ends the table and turns the following
   rows into a paragraph (and leads to C1). The table spec says Source has
   "no table-specific Enter … rewrite", but a paragraph separator inside a
   row is rarely intended. Should Source Enter in a table row insert a
   single line ending?
4. **Autoformatting in an existing list item.** Typing `- ` at the start of
   `- a` autoformats to a nested bullet (`- - a`). Typing `1. ` writes
   `- 1.&#32;a` instead. Should ordered markers nest the same way, or
   should neither autoformat inside an existing item's first line?
5. **Line commands on table rows.** What should `J`, `gJ`, `2dd`, `gUU`,
   `g~~`, `>>`, `:sort` and `:g` do in WYSIWYG tables? They currently fail
   with a mix of "Editing across table cell boundaries…",
   `TextDoesNotMatchLayout`, `AmbiguousProjection` and
   `UnsupportedFormatting`. Possible rules: act on the current cell's text,
   act on whole rows, or refuse with one consistent message.
6. **Boundary Backspace and Delete in cells.** These are no-ops by design.
   Should they at least move the caret to the previous or next cell, as
   Word does, so the key doesn't feel dead?
7. **Source view hiding emptied lines.** In `a\nb\n\nc`, selecting `b` and
   pressing Backspace leaves source `a\n\n\nc`. Source view collapses the
   separator, so the line the user was editing disappears from view. Is
   that intended? Whatever the presentation, the caret shouldn't leave the
   line: C67 shows that `cc` and repeated `x` then edit the following line.
8. **Mid-paragraph table insertion.** Insert Table at a caret inside a
   paragraph is refused ("This insertion cannot preserve the surrounding
   inline structure.") at about 650 probe positions. Should it split the
   paragraph and insert the table between the halves?
9. **Formatting availability vs. refusal.** Increase/Decrease Indent outside
   lists, Heading/Code Block on thematic breaks and HTML blocks, and Bold on
   selections the core can't wrap are presented as available and then
   rejected. Should `selection_semantic_style_presentation` and the
   paragraph-style presentation run the same capability check as the edit,
   or should these always succeed through a fallback spelling (for example
   `<strong>`)?
10. **Ordinal overflow.** `999999999. big`, then `o`/`O` in Source: the next
    ordinal `1000000000.` is no longer a list marker under CommonMark's
    nine-digit limit. Continue as a paragraph, reuse the same ordinal, or
    refuse?
11. **Leaving a trailing code block (C3c) or table (C3b).** What gesture
    should create the paragraph after the block: Down at the last line,
    Enter on an empty last line, or a triple Enter?
12. **Failed change operators (H3).** When `cc`, `S` or `C` fails, should the
    view still enter Insert, as Vim does after a successful delete, so the
    following text is inserted rather than executed?
13. **Composition in Normal mode.** The core accepts `CompositionEvent::Begin`
    and commits marked text while in Normal mode. The fuzzer committed `か`
    without entering Insert. The frontends never send this, so should the
    core refuse it? The core also accepts a composition whose target is the
    caret while a native selection is active, and then inserts at the caret
    without replacing the selection. The macOS frontend always passes the
    selection as the target, so this only matters for other callers. The
    fuzzer first did it this way; C78 is the real failure it uncovered.
14. **Unsupported Vim keys.** `Ctrl-A`/`Ctrl-X` are unsupported in Normal
    mode (`Unsupported("normal key Ctrl('a')")`). So are the Visual-mode
    linewise commands `X`, `D`, `C`, `S` and `R`
    (`Unsupported("visual key Char('X')")`), which Vim users reach for after
    selecting lines. Intended, or gaps to list in `gaps.md`?
15. **Code-mode vertical motion on wrapped lines.** Code defaults to Physical
    Source line mode, so Up/Down/j/k on a single wrapped line don't move
    between its visual rows. This matches the spec, but the native Up/Down
    arrows also follow it in Insert mode. Should native arrows always move
    by visual row?
16. **Footnotes.** Footnotes are a listed gap, but edits inside footnote
    definitions already produce projections that differ from a fresh parse
    (C56). Should definitions be treated as opaque until supported?
17. **Literal CRs in mixed-ending files.** A file with mostly LF and some
    CRLF lines opens as `unix`, following Vim's `fileformats=unix,dos`. The
    CRs become literal content: WYSIWYG shows `ab\r c`, the caret can sit
    between CR and LF, and Backspace at a line start can delete an
    invisible CR instead of joining lines. This matches Vim's `^M`
    behavior. Should WYSIWYG show or protect these CRs, for example with a
    visible marker or by refusing to put the caret between CR and LF?
18. **The final line ending as an extra line.** Viem models `a\nb\nc\n` as
    four hard lines, the last one empty. Vim models it as three lines with
    an end-of-line flag. So `G` lands on an empty line Vim doesn't have,
    `:$d` deletes the final newline (`a\nb\nc`, Vim gives `a\nb\n`), and
    `:m $` from `b` gives `a\nc\n\nb` (Vim gives `a\nc\nb\n`). `:%norm Ax`
    also appends `x` to that extra line, adding a new last line, and
    `:g/^$/d` deletes it (C80). `G` and Ex addresses agree with each other,
    so this looks deliberate. AGENTS.md also says "no implicit final
    newline". But it is the root of C50 and C70, and Vim users will notice
    it on almost every file. Should the literal views treat a final line
    ending as a terminator, as Vim does?
19. **List continuation in Plain Text.** In Plain Text, Enter at the end of
    `- item`, `* item` or `1. one` continues the list (`- `, `* `, `2. `),
    and Enter on the empty continued item ends it. `o` doesn't continue.
    AGENTS.md describes list continuation for Markdown and says Code never
    runs it, but doesn't mention Plain Text. Should Plain Text continue
    lists, and if so, should `o` match Enter?
20. **Ctrl-W and Ctrl-U at a line start.** In Insert mode at column 0,
    Viem's Ctrl-W and Ctrl-U do nothing (`ctrl_w_never_crosses_a_hard_line`
    makes this deliberate). Vim with the default `backspace=indent,eol,start`
    joins the line with the previous one, as Backspace does. Keep the
    difference?

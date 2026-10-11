# Bugs

## Consistency audit, October 9–10, 2026 (commit `f6c15de`)

This audit looked for editor states where the Rust core rejects ordinary
input (so typing, deleting, or moving the caret fails), panics, or behaves
inconsistently. It combined reading the code with a new fuzzer,
[`examples/fuzz_consistency.rs`](examples/fuzz_consistency.rs). This document
tracks the remaining findings as they are researched and fixed. Completed
items are removed; changed interpretations are recorded under Modified items.
Existing Open questions remain deferred.

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
probe. Those composition history/export failures now have regression coverage. Vim-semantics findings (C69–C72, C74,
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

Character styles in Source view are included in the probes. Set
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

## Audit status

Confirmed editing, projection, command, layout and recovery defects have been
fixed and removed from the active inventory. Modified interpretations and
policy-dependent errors are documented below. Open questions remain
undecided; no policy changes for them were implemented.

The fuzzer ran about 4 million probes across nearly 650 seeds,
140,000 random walks of 300 steps each, and about 180,000 cases against
reference Vim.

## Modified items

**C53, reference-definition selections — Report:** Bold or Italic on selections
containing reference definitions failed while the action appeared available.
**Different handling:** Definitions deliberately retain their literal bracket
syntax; introducing emphasis there can change reference resolution. This
capability/conversion decision remains under open question 9, with its current
backend verification refusal unchanged. Ordinary and passive HTML bodies,
comments, front-matter-like prose, thematic-break boundaries and cross-cell
selections now have passing formatting regressions. Partial Source hard-break
syntax remains covered by the separate C53 entry and open question 24 below.

**C63, repeated global deletion.** The report described `:g/a/d` on the demo
as slow and ending in `VerificationFailed`. It now makes bounded, undoable
progress. Repeating it reaches the existing explicit cross-cell table-line
refusal covered by open question 5; that policy remains unchanged. Each
completed prefix matches reopened source and restores exact bytes with
undo/redo.

**C57, Indent outside lists.** The report also included the Indent formatting
action on visible comments and `<pre>` blocks. Whether that action should be
unavailable or introduce a different treatment is already open question 9;
that policy remains unchanged. Literal Tab input is fixed and verified.

**C41/C43, formatting special block owners — Report:** Heading assignment
failed on thematic rules, reference definitions, passive HTML, unclosed
comments and front-matter-like source; list/quote assignment failed on passive
`<div>` blocks. **Different handling:** These conversions depend on existing
open question 9: whether the controls should refuse the operation or introduce
a different source representation. That policy remains unchanged. The current
attempts still return backend `VerificationFailed` errors; they are deferred
errors, not newly implemented friendly refusals. Ordinary heading assignment,
including the empty list item `-`, deep list nesting and fences containing blank
lines, has regression coverage and is repaired.

**C42, pending formatting inside literal syntax — Report:** Bold or Italic,
then typing inside HTML comments, script bodies or reference definitions,
failed with `VerificationFailed`. **Different handling:** Inline delimiters do
not activate inside those literal owners, and inserting tags there would change
literal text or reference semantics. Whether these typing choices should be
unavailable or use another representation is part of open question 9. No policy
was implemented; these backend errors remain deferred. Supported passive HTML
body typing and escaped-pipe table Code boundaries have regression coverage.

**C44, pending formatting inside literal Source code.** The report listed
`UnsupportedFormatting` for pending Bold and Italic inside a fenced code block
in a list. Those source positions are literal code; deciding whether the
controls should be unavailable or should introduce supported structural syntax
is part of existing open question 9. This behavior remains unchanged. Source
carets inside partial escape/break syntax remain under open question 24.
The `OverlappingFormatting` report inside `~~strike~~` is fixed; Bold, Italic
and Code now succeed at every Source caret position with exact undo/redo.

**C56, final paragraph after a Setext heading.** The report called `a\n- \nb`
an empty list item and described `dw` leaving a stale projection. The current
parser correctly treats `-` as the underline of heading `a`; `b` is the final
ordinary paragraph. Deleting its body now retains its unselected boundary with
minimal supporting source syntax, including at EOF, and exact Undo restores the
original bytes.

- **C59 — Report:** Global deletion/substitution replayed one transaction per
  target and froze modest files, with the audit measuring 24–71 seconds for
  4,000-line cases. **Handling:** Independent complete-line deletion in Text,
  Code and Source, ordinary WYSIWYG paragraph deletion, and line-contained
  literal substitutions batch into one verified transaction when row semantics
  are proven. Other bodies retain ordered replay with a 250 ms work budget,
  checked between targets. Reaching that budget reports the processed prefix;
  completed edits remain one undoable action and remaining targets are not
  silently treated as completed. No asynchronous continuation contract was
  introduced. Tests cover 2,000-line batches, registers, reopen/Undo and an
  injected zero-duration budget that preserves exactly one completed target.

**C66, Source nested-list indentation.** The report described a backend error
when pressing Tab at offset 12 in `> - > - > a\n> - > - > b`. The two inner
items belong to different enclosing list items and quotes, so the second has
no preceding sibling available for indentation. The capability check now
uses container identities, and Tab follows the existing no-op behavior for
an item that cannot be indented. Source and caret remain unchanged.

- **L5 — Report:** `:s/ b /\r/` inside `` `a b c` `` failed; literal `\n`
  substitution also failed with an internal projection-verification message.
  **Handling:** Semantic line-break substitution now splits the code span into
  locally authored code paragraphs and restores exact source on undo. Literal
  LF keeps its distinct meaning: representable combinations retain it, while
  unsupported format/line-ending combinations explain the constraint and point
  to `\r` for a line break, without changing source or command history.

- **H5 — Report:** Ordinary typing had no second strategy when a minimally
  patched candidate failed verification; the audit suggested escaped text or
  rewriting the smallest inline construct. **Handling:** Specific verified
  repairs now cover delimiter joins, code spans, autolinks, reference boundaries,
  clipboard style collisions and literal HTML. Each repair remains local and
  preserves exact undo. Verification stays mandatory; a generic rewrite that
  could discard unsupported source was not introduced. Remaining concrete cases
  are tracked individually above.

- **C53 Source hard-break selection — Report:** Bold and Italic on Source
  `a\\\nb` or `a  \nb`, range 0..2, fail with `UnsupportedFormatting`.
  **Different handling:** These ranges stop inside the syntax contributing a
  hard break. Formatting those raw characters and formatting the represented
  text are different operations. No policy was chosen; see open question 24.

- **H2 — Report:** An unexpected input panic can return through the FFI with
  partially updated document, interpreter or checkpoint state; the report
  suggested restoring a checkpoint or resetting positions. **Handling:** Input
  dispatch now restores its exact document, shared command, view, composition
  and completion checkpoint on unwind, aborts abandoned nested checkpoints, and
  retires tentative layout work without reusing issued identities. Completed
  macro actions remain accepted and undoable. Tests inject panics before and
  after mutation, including nested edits and shared views. Known resize/layout
  panic sites are fixed directly; no guessed cursor reset was introduced.

- **C70 — Report:** Last-line deletion, change and put lost the existing final
  terminator; `J` on Code’s final nonempty row also joined its terminal empty
  row. **Handling:** Delete/change/put now retain the existing terminator and
  indentation, with Text, Code and Source regression coverage. The `J` case
  depends on the terminal-empty-line model in Open Question 18 and remains
  unchanged pending that decision.

- **C67 — Report:** Emptying a Source continuation line sends typing and
  repeated `x` into adjacent lines; the report proposed keeping a temporary
  displayed empty row. **Handling:** Changes and deletions retain the exact
  physical source caret when paragraph-separator folding removes its visible
  row. Typing returns to that source line, and repeated `x` stops at its empty
  boundary. Separator-row presentation remains unchanged pending Open Question 7.

- **H1 — Report:** A stale mark, jump or revision can block later input; the
  proposed recovery dropped damaged auxiliary positions and resynchronized
  revisions. **Handling:** Exact snapshot validation remains in place. Retained
  positions are captured as stable anchors and rebased through the committed
  transaction maps; the producing jump and composition/history paths were fixed
  instead of discarding positions or guessing their current locations. Regression
  tests cover continued input and Undo after those edits.

- **C3b/C3c — Report:** A leading/trailing table or a trailing code block
  traps Insert mode because arrows/Enter do not create an ordinary paragraph
  outside it. **Handling:** No new block-exit gesture has been implemented;
  this is the existing Open Question 11. Navigation between existing editable
  positions is investigated separately from creating a new paragraph.
- **H3 — Report:** A failed `cc`, `S`, `C`, or `cw` leaves Normal mode active,
  so following text executes commands; the audit suggested entering Insert
  even after failure or requiring acknowledgement. **Handling:** Failed
  transactions retain mode under the current atomicity requirement. The
  alternative remains Open Question 12; confirmed failures of otherwise valid
  changes are fixed without choosing that alternative.
- **Line addresses — Report context:** The audit predates the current absolute
  line-address rules. **Handling:** Numeric Ex addresses and counted absolute
  motions are interpreted as physical source lines in every view. Contextual
  addresses and relative counts retain the selected line domain. Regression
  tests use those current rules rather than restoring the audit-era behavior.

- **C53 whitespace — Report:** Move emphasis delimiters inside leading/trailing
  selection whitespace. **Handling:** Passive `<strong>`/`<em>` scopes apply
  formatting to the exact selection, including its spaces, while preserving
  text and source bytes outside the local wrappers. Paragraph separators have
  no character treatment; each selected paragraph body is formatted separately.

- **C74 — Report:** Layout-aware indentation ignored options and inserted whitespace
  inside wrapped paragraphs. **Handling:** Indent, Outdent and Reindent now use
  buffer options, and selecting a row excludes the preceding touching row.
  Applying indentation to selected displayed rows remains the current line-mode
  policy; no enclosing-paragraph indentation rule was introduced.
  Follow-up verification also corrected Insert-mode indentation's saved start
  boundary: Ctrl-D, Ctrl-T and soft-tab Backspace now keep Ctrl-U valid after
  changing a prefix, with exact undo/redo in Text, Code and Markdown Source.
- **C79 — Report:** Putting a yanked partial visual row splits a word, and joining
  adjacent wrapped rows silently changes nothing. **Handling:** Partial visual
  rows intentionally remain characterwise, so their puts retain ordinary
  characterwise placement. Deleting/changing the last partial row now preserves
  the following hard-line separator. Changing the join rule is Open Question 22.
- **C80 — Report:** Global deletion skips adjacent blank lines and strips final
  terminators. **Handling:** The current physical address implementation already
  fixes the supplied terminated-file traces. Regression coverage retains that
  behavior without changing the final-empty-line model in Open Question 18.

- **C64 — Report:** Link/image insertion across emphasis fails; popups are
  offered for paragraph-spanning selections, code, partial existing links and
  unrepresentable Source boundaries, then reject the edit. **Handling:** New
  links now split crossing emphasis scopes locally while retaining all selected
  and unselected formatting. The native frontends already honor the core's
  insertion capability. That capability now also excludes an escaped Source
  opener, the interior of a backtick delimiter, and reference definitions.
  Paragraph-spanning links, nested links and links inside literal code remain
  unavailable because Markdown cannot express those intentions there.

- **C61 — Report:** Visual `J`/`gJ` across table cells could reverse a local
  source range or underflow its relative offset, including after inserting a
  column and making a native selection. **Change:** Local syntax preparation
  now rejects unsupported discontiguous source ranges before slicing. Both
  reported trace families leave source intact and subsequent typing works.
  This fixes the panics; the table-command behavior in Q5 remains undecided.

- **C31 — Report:** Choosing Code and typing inside the opening or closing
  `**` run in Source deleted the original asterisks. **Change:** A Source caret
  inside a delimiter run authors the new code span exactly there and retains
  all existing characters. For example, offset 1 now produces
  `` *`x`*bold** ``. The resulting Markdown meaning follows that literal Source
  edit; the editor does not relocate or delete the retained delimiters to keep
  their previous styling.

- **C65 — Report:** Copying nonrectangular text across Markdown table cells
  and pasting into a header failed with `AmbiguousProjection`; related inline
  pastes also failed. **Handling:** Portable inline text and style runs now use
  verified semantic formatting when a raw source fragment would collide with
  destination markup or no single source fragment exists. The inline cases and
  table-body insertion pass, including cell breaks and exact undo/redo. A mixed
  header/body selection pasted into a header raises the unresolved inherited
  appearance question below; no new header-formatting policy was introduced.

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
   rows into a paragraph (the resulting split-table identity panic has been fixed). The table spec says Source has
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
   that intended? The C67 caret relocation is fixed: `cc`, typing and repeated
   `x` now retain the intended physical source line. The separator-row
   presentation question remains open.
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
16. **Footnotes.** Footnotes are a listed gap. The C56 incremental/reopened
    projection mismatches in the reported edits are fixed without adding
    footnote support. Should definitions be treated as opaque until supported?
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
    newline". But it is the root of C70, and Vim users will notice
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

21. **A leading content U+FEFF after an edit (C28).** The original report:
    deleting `a` from `a\u{feff}b` leaves a visible content U+FEFF in the live
    projection, but reopening interprets its bytes as a BOM and shows only `b`.
    The incremental and reopened documents must agree. WYSIWYG can preserve
    the character with a numeric reference; literal Source/Text/Code cannot
    use that spelling without changing visible text. Should those views add
    a BOM before the retained U+FEFF (changing BOM metadata), reject this edit
    with an encoding explanation, or deliberately turn the leading character
    into a BOM? No policy change has been implemented.

22. **Joining soft-wrapped rows (C79).** Under Visual line meaning, `J`, `gJ`
    and Visual `J` on rows within one hard line have no separator to remove.
    Should these remain no-ops, report a message, or advance to the next hard
    line? The current displayed-row rule is unchanged pending this decision.

23. **Pasting body text into a table header (C65).** A nonrectangular selection
    spanning bold header text and ordinary body text carries their resolved
    appearances. Pasting it into a body cell works. In a header, preserving the
    body text's non-bold appearance would require overriding the header's bold
    default, which Markdown's editable inline vocabulary cannot express. Should
    copied paragraph defaults inherit from the destination header, or should a
    paste that cannot preserve its resolved appearance be refused? The original
    reproduction is `| **bold** | _it_ |\n| --- | --- |\n| a | b |`, copying
    logical range 0..9 and pasting at 0. It now returns the existing unsupported
    formatting result atomically; this policy remains undecided.

24. **Formatting a partial source-syntax selection or caret (C53/C44).** Bold and Italic
    on Source `a\\\nb` or `a  \nb`, range 0..2, select `a` and only part of
    the hard-break spelling. Should formatting apply to the raw selected
    characters, to the represented visible text, or be unavailable until the
    selection contains complete syntax contributors? Ordinary visible-space
    and entity selections do not have this ambiguity. No behavior change has
    been implemented for these partial-syntax selections. The same question
    applies to pending Code/Bold/Italic after a hard-break backslash
    (`a\\\nb` at 2) or inside an escape (`\# not heading` at 1): retaining the
    existing backslash means it consumes a newly inserted formatting delimiter.
    Should typing preserve the raw spelling or change that syntax to make the
    new text carry the requested style? These caret cases are also unchanged.

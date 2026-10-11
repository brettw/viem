# Bugs

## Fix-verification audit, October 11, 2026 (commit `3016333`)

This pass checked whether the October 9–10 findings below were fixed, and
then repeated the full search for similar bugs, including the features added
since (line numbers, gutter row selection, absolute line addresses, Go to
Line, `#L` link navigation, the code-block language selector and split
panes). Severities use the scale defined in the next section.

### Repair status

The October 11 repair pass addresses the actionable findings in this audit:

- R1–R18: command semantics, Ex ranges, list and table editing, source
  reprojection, clipboard puts, Replace-mode restoration and page movement.
- Remaining reproducible portions of C0, C7, C10, C19, C23, C30, C32, C34,
  C38, C42–C44, C59, C65, C68 and C76, including the three reachable panic paths.
- N1–N14: paragraph scoping, autolinks, structural edits, quoted tables,
  gutter selections, physical line numbers and code-language cleanup.

Regression coverage checks the command/native entry paths, reopened source,
exact undo/redo, retained formatting, and bounded regional work. The tests live
in the existing audit, command, Markdown, clipboard and line-number modules in
[`tests/all`](tests/all). Policy-dependent portions remain deferred to the
existing open questions, including N11's table insertion/indentation choices,
N12's delete-versus-clear choice and N15's line-range navigation. N16 reported
no defect. The original observations below remain as reproduction evidence.

Repair validation replays all 829 scripted cases: 828 pass, with only
`C79/yyp-interior` remaining under open question 26. The full Rust all-targets
suite passes 3,373 tests (one ignored), including the large-document and
source-reopening checks.

Native validation is not fully green. `scripts/test-mac.sh` completes 1,068
XCTest cases with six failing cases (92 assertions/errors), plus 48 passing
Swift Testing cases. The failures are the three launcher timeout cases in
`EVBlockingEditorTests`, the plain-click comparison in `EVCaretHoverTests`,
the demo None-to-C selection in `EVCodeBlockLanguageTests`, and code-style
rename/deletion persistence in `EVCodeStylesTests`. These remain outside this
audit's repaired reproductions; the launcher failures also repeat in isolation.
The focused rich-clipboard and smart-quote native suites pass all 15 cases.

### How it was checked

- **Replaying every reported reproduction.** Each repro from the previous
  report (C0–C82, L1–L7) is now a scripted case in
  [`tools/regress_cases.py`](examples/fuzz_consistency/tools/regress_cases.py),
  726 cases in all, plus 103 cases for the findings below. The new cases use
  the IDs in this section (`R…`, `N…`, and the earlier ID for partly fixed
  items). `tools/regress.py` runs them against any `fuzz_consistency` build.
- **Old core versus new core.** The same harness was built against the audit
  commit (`2e5c626`) and against `3016333`, and both ran the same probes,
  replay cases and Vim differential. A case that passes on the old core and
  fails on the new one is a regression; one that fails on both is unfixed.
  Of the 726 original cases, 139 pass on the old core and 704 on the new
  one. None of the 103 new cases pass on the new core. Counting distinct
  finding signatures per seed across the insert, normal, selection,
  copy/paste and navigation probes, 1,585 occur only on the old core, 766 on
  both, and 430 only on the new core. Policy refusals (Indent outside a list,
  open question 9, table line commands) account for much of the overlap; the
  new-only signatures are R1–R18 and the refusals that replaced C30's hangs.
- **Full probe and walk campaigns** at 600 and 140 points, with line numbers
  on (`FUZZ_NUMBER=1`) and off, and 20,000 new random walks of 300 steps that
  now also toggle line numbers, select gutter rows, use Go to Line, follow
  `#L`/heading fragments and change code-block languages.
- **Vim differential**: about 119,000 Normal-mode, Insert-mode, dot-repeat and
  Ex cases in Plain and Code, compared with `/usr/bin/vim -u NONE`, with new
  word-motion cases (`ex`, `2ex`, `cWx`, `c2wx`, `d2w`, `dE`, `yeP` and
  others). 12,794 cases that disagreed with Vim on the old core now agree;
  1,718 that agreed now disagree, mostly R1 and R3–R5. A few of those, such
  as `vllJ` at the end of a line, are artifacts where Vim's `:normal` stops
  after a failed motion.
- **A new geometry fuzzer**, [`examples/fuzz_panes.rs`](examples/fuzz_panes.rs),
  for the split-pane model (9 million operations, see N16).
- **Code reading** of the new Ex address and region code, the gutter, link
  and code-language paths, and the macOS refresh path.

### Summary

Most of the October 9–10 findings are fixed. The reported lock-ups (C0, C1,
C30, C45, C48, C60, C78) no longer reproduce as reported, unexpected panics
are now contained by the input checkpoint (H2), and the macOS refresh now
degrades optional exports instead of failing (H4). The replay suite, the
probes and the walks all show large improvements.

The fixes also introduced regressions, and the new pass found serious bugs
that the first audit missed. Fix these first:

| ID | Sev | Summary |
|---|---|---|
| N1 | S1 | In Markdown Source and WYSIWYG, `}`/`{` jump to the document ends and `dap`, `dip`, `cip`, `d}` and `>ap` act on **the whole document**. `dap` in any paragraph deletes everything. |
| R1 | S2 | `cw` on a one-letter word, or on the last letter of any word, also deletes the next word, even across a line break. `I saw a cat.` with `cw` on `a` gives `I saw x.`. Regression from the C75 fix. |
| R2, R3 | S2 | Regressions in Ex line commands: `:d`, `:m`, `:t` and ranges ending at an unterminated last line with non-ASCII text fail with `AddressOverflow`; `:g/a/d` on `a\nb\na` fails with "text edits overlap". |
| R4 | S2 | `dap` on the last paragraph of a Plain or Code file deletes the previous paragraph's last line. Regression. |
| R8, R9, C0 | S1 | Three reachable panics: typing into a table duplicated with `:t.`, Italic over a selection that ends at a table-cell boundary, and the C0 jump-list panic through `:v`/`:g`. |
| N2, N3 | S1 | Can't type a space after an `<https://…>` autolink at the end of a paragraph; can't type into an empty cell of a table inside a block quote, including any table just inserted there. |
| R6, R7 | S1/S2 | The C19 fix broke Tab on an empty item after another empty item, and Enter, Tab, type now turns a tight list loose by inserting a blank line. |
| C65 | S1 | Native Copy then Paste still fails in many ordinary places: from one heading into another, into a bare URL, next to passive HTML. |
| N12 | S2 | Gutter row selection: deleting a selected heading row turns the next paragraph into a heading; selected table rows can't be deleted, typed over or pasted over. |

### Status of the October 9–10 findings

This table records the audit at `3016333`; the repair status above supersedes
its actionable "Partly" findings.

"Fixed" means the original repro and its listed variants now behave
correctly. "Partly" items are described under
[Earlier findings that are only partly fixed](#earlier-findings-that-are-only-partly-fixed).
"Deferred" items depend on the open question named.

| ID | Status | Notes |
|---|---|---|
| C0 | Partly | `=G`, `:g/…/d` and `G` cases are fixed. The same panic is still reachable through `:v`/`:g` followed by typing; it is now contained. |
| C1, C45, C48, C60, C61 | Fixed | No panic. C1's panic site is reachable again by another path (R8). C48's table now gets created, but it can't be typed into (N3). |
| C2, C3, C21, C21b, C22 | Fixed | |
| C3b, C3c | Deferred | Open question 11. |
| C4, C5, C6, C8, C9, C11–C18, C20 | Fixed | |
| C7, C10 | Partly | See below. |
| C19 | Partly | Fixed for the reported case; see R6 and R7. |
| C23 | Partly | `- `, `#` and `>` are fixed; typing `1. ` still leaves a stale projection. |
| C24–C27, C29, C31, C33, C35–C37 | Fixed | |
| C28 | Deferred | Open question 21. |
| C30 | Partly | No hangs anywhere. Selections that used to hang are now refused instead: in Source, Code or Default Paragraph on `b` in `**a *b* c**`, on `~~strike~~`, or on `__bold__` fails with `UnsupportedFormatting`, and Code on 1..3 in `***both***` reports an internal reversed range, `InvalidRange { start: 4, end: 0 }`. |
| C32 | Partly | All `d`/`c` forms are fixed. Setext `yyp`, `yyP`, `ddp` and `ddP` now fail (R10). |
| C34 | Partly | Fenced code blocks, setext headings and `<br>` work. `:t`/`:m` still fail on reference definitions, nested and continued list items and indented code. |
| C38 | Partly | Fence, ATX and code-span cases are fixed; `**[bold link](u)**` is not. |
| C39, C40, C41, C46–C58 | Fixed | Policy-dependent parts remain under Modified items. |
| C42, C43, C44 | Partly | Policy-dependent parts remain under Modified items; the rest is below. |
| C59 | Partly | Bounded and fast in Text, Code and Source; still about 10 ms per target in WYSIWYG. The batching also introduced R3. |
| C62, C66, C67, C69–C73, C75, C77, C80–C82 | Fixed | C75's fix caused R1 and part of R5. C81 no longer blocks the macOS refresh because the selection export is optional. |
| C63 | Fixed | Except the WYSIWYG `:g` cost under C59, and the demo's setext headings under R10. |
| C64 | Fixed | |
| C65 | Partly | The reported cases pass; many similar ones still fail. |
| C68 | Partly | Source cases are fixed; Code still loses indentation. |
| C74 | Disputed | Shifts now use the options. `>>` on a wrapped row still inserts spaces mid-sentence; see open question 25. |
| C76 | Partly | The table survives, but retyping a header cell authors `**…**`. |
| C78, C51 | Fixed | C51 now rejects multi-command text in Normal mode with a clear status. |
| C79 | Disputed | The line-ending cases are fixed. `yyp` on an interior row still splits a word (open question 26). |
| L1–L7 | Fixed | |
| H1 | Partly | See C0. |
| H2 | Fixed | Panics restore the pre-input checkpoint, but R8, R9 and C0 still panic. |
| H3 | Deferred | Open question 12. It still turns failed changes such as N5 and N6 into deletions. |
| H4 | Fixed | Gutter, selection, completion and search exports are optional on macOS. Layout, viewport and composition exports remain required. |
| H5 | Fixed | |
| H6 | Fixed | C30's loop is bounded. No probe or walk hung on the new core; the other loops weren't re-read. |

### Regressions introduced by the fixes

Each of these is worse on `3016333` than on the audit commit's core. R7,
R10 and R11 were already wrong there in a different way; the rest worked.
R12–R18 came from comparing each probe's findings between the two cores.

#### R1 (S2). `cw` deletes the following word

All formats. Vim's `cw` on a non-blank character changes to the end of the
*current* word. Viem now changes to the end of the *next* word whenever the
cursor is on the last character of a word, which includes every one-letter
word:

| Text, cursor | Keys | Vim | Viem |
|---|---|---|---|
| `I saw a cat.`, on `a` | `cwx<Esc>` | `I saw x cat.` | `I saw x.` |
| `I saw a cat.`, on `I` | `cwx<Esc>` | `x saw a cat.` | `x a cat.` |
| `ab\ncd\nef\n`, on `b` | `cwx<Esc>`, `cWx<Esc>` | `ax\ncd\nef\n` | `ax\nef\n` |
| same | `c2wx<Esc>` | `ax\nef\n` | `ax\n` |
| `a\n` | `cwx<Esc>` | `x\n` | `x` |

The next word is deleted silently, together with the line break when that
word is on the next line. In WYSIWYG tables, `cw`, `cW`, `ce` and `de` on a
one-letter cell now reach into the next cell and fail with "Editing across
table cell boundaries…"; because a failed change stays in Normal mode (H3),
the `x` typed next deletes the cell's text. `| a | b |…` with `cwx<Esc>`
leaves the cell empty. The Vim differential found about 1,100 such cases
in Plain and Code. `resolve_operator_extent` (`src/core/command/mod.rs:9237`)
maps `cw` to `OperatorMotion::WordEnd`, which uses `move_word_end`. The C75
fix in `498c9ca` made `move_word_end` always step past the current word end,
which is correct for `e` but not for `cw`. Vim's `cw` calls `end_word` with
`stop` set, which counts a cursor already on a word end as being at the end
of the first word.

#### R2 (S2). Ex line commands fail on a non-ASCII last line

All formats. When a range ends at the last line, the file has no final line
ending, and that line ends with a multibyte character, `:d`, `:.d`, `:$d`,
`:.,$d`, `:m0`, `:m-2`, `:t0`, `:.g/./d` and (in WYSIWYG) `:v/a/d` fail with
`ExError(Execute(AddressOverflow))`. Examples: `aé` with `:d`;
`a\nb\n中文` with `G:d`. `:s`, `:normal`, `:%d` and `:1,.d` work.
`exact_hard_lines` (`src/core/command/ex_regions.rs:310`)
looks up `range.text.end - 1`, a byte offset inside the last character, and
`line_at_offset` rejects it. `src/core/document/addressed_lines.rs:54` has the same pattern
behind an `AmbiguousProjection` error.

#### R3 (S2). `:g`/`:v` deletion fails with "text edits overlap"

Plain and Code, `a\nb\na` (no final line ending): `:g/a/d` fails with
`Error("text edits overlap")` and changes nothing. So do `:g/a/d` on
`**not markdown**\n# nope\n| a | b |` and `:v/a/d` on the JSON seed. The old
core gave `b`. The new batched deletion (C59) apparently builds the last
line's range so that it reaches back over the preceding line ending, which
overlaps the previous target's range.

#### R4 (S2). `dap` on the last paragraph deletes part of the previous one

Plain and Code. `First paragraph.\nSecond line.\n\nLast line.`, `G`, `dap`
gives `First paragraph.`: `Second line.` is deleted too. Vim gives
`First paragraph.\nSecond line.`. `a\nb\n\nc\n` with `dap` on `c` gives `a\nb`,
losing the final line ending (old core and Vim: `a\nb\n`).

#### R5 (S2). Other Vim-motion side effects

Found by the Vim differential, all in Plain or Code, all matching Vim on the
old core:

| Text, cursor | Keys | Vim | Viem |
|---|---|---|---|
| `ab\n`, on `b` | `de`, `dE` | `a\n` | `a` (final line ending deleted) |
| same | `yeP` | `abb\n` | `ab\nb\n` |
| `ab\ncd\n`, line 2 | `v$d` | `ab\n\n` | `ab\n` |
| `s = '''\nmulti\nline\n'''\n`, on `multi` | `d2w` | `s = '''\n'''\n` | `s = '''\n\n'''\n` |
| same, on `line` | `db` | `s = '''\nline\n'''\n` | `s = '''\n\nline\n'''\n` |
| `plain\n\ttext\n`, line 2 | `dd.` | (empty) | `plain` |
| `P.\nS.\n\nL.\n`, on the empty line | `dw` | `P.\nS.\nL.\n` | no change |
| ` * star\n`, on `*` | `r<CR>` | ` \n star\n` | ` \nstar\n` (autoindent lost) |

The `de`, `ye` and `v$d` rows and `dd.` lose or misplace the final line
ending (compare C70). `d2w`/`db` no longer apply Vim's exclusive-linewise
rule. `dw` on an empty line now does nothing because an empty line counts as
a word (the C77 fix) but the deletion is then empty.

#### R6 (S1). Tab fails on an empty list item after another empty item

WYSIWYG and Source, `- \n- ` or `1. \n2. `, Insert at the end of the second
item, Tab: `VerificationFailed`. The old core gave `- \n  - `. The C19 fix
handles an empty item after a nonempty one only.

#### R7 (S2). Enter, Tab, type makes the list loose

WYSIWYG `- a\n- b`, `GA`, Enter, Tab, `x` gives `- a\n- b\n\n  - x`, and
`1. a\n2. b` gives `1. a\n2. b\n\n   1. x`. The blank line is needed while the
new item is empty (an empty item can't interrupt a paragraph), but it is
kept after typing. It makes the whole list loose, so every item's spacing
changes, and Source view shows the inserted blank line. The supporting blank
line should be removed in the same transaction as the first typed character.

#### R8 (S1). Panic typing into a table duplicated with `:t.`

WYSIWYG, `| a |\n| --- |\n\nx` (any header-only one-column table followed by
a paragraph): `:t.`, `i`, type `q`. The core panics at
`src/core/document/range_index.rs:818` ("range-index input is ordered by start boundary",
through `replace_table_row_metadata`, C1's site). Every later character typed
in that cell panics again, so the cell can't be edited. `:t0` reaches it
too. On the old core `:t.` failed, so the state wasn't reachable.

#### R9 (S1). Panic: Italic or Bold over a selection ending at a cell boundary

WYSIWYG `| *a* | b |\n| - | - |`, select 0..2 (the cell text plus the cell
boundary), Italic. The core panics with `Option::unwrap()` on `None` at
`src/core/document/projection.rs:5418` (`source_spans.first().unwrap()` in
`splice_line_local_projection`, from `build_table_row_candidate`). The edit
is a no-op for that cell, so there are no source patches. Bold on
`| **bold** | _it_ |…` 0..5 does the same. The old core returned
`VerificationFailed`.

#### R10 (S2). Setext headings can't be duplicated or swapped

WYSIWYG `Setext\n===`: `yyp` and `yyP` fail with `UnsupportedFormatting`.
`Setext\n===\n\nx`: `dd` works, then `p` fails, so `ddp` loses the heading
until Undo. The old core succeeded but wrote the copy as a plain paragraph
and converted the original to `# Setext`.

#### R11 (S3). `xp` next to code spans fails

WYSIWYG ``a `b` c `d` e``, `xp` at the start fails with
`UnsupportedFormatting` after deleting `a`. The old core put the `a` inside
the code span instead. The expected source is ``&#32;a`b` c `d` e``.

#### R12 (S2). Source view shows a line break that isn't in a table row

Markdown Source, `| a |\n| - |\n| 1 |`. Shift-Enter after `| - ` in the
delimiter row, then `x`, writes `| a |\n| - \nx|\n| 1 |`, but the view shows
`x\n|`, with a line break that isn't in the file. `| a | b |\n| - | - |\n| 1 | 2 |`
with Shift-Enter before the last `|` does the same, and so does `cwx<Esc>` on
the leading pipe of a body row (which also hits R1). Whenever an edit leaves a
row without its leading pipe, the incremental Source text differs from a
fresh parse. The old core displayed these rows correctly.

#### R13 (S2). Shift-Enter in an empty middle list item

WYSIWYG `- \n- \n- `, Insert in the middle item, Shift-Enter, `x` writes
`- \n\n- <br>x\n- `: a blank line is inserted before the item, the list turns
loose, and the incremental blocks differ from a fresh parse. The old core
wrote `- \n- <br>x\n- `. This looks like the same supporting-blank-line rule
as R7.

#### R14 (S3). Typing a backslash at the end of an unclosed comment

WYSIWYG `<!-- unclosed comment`, `A`, type `\`: `FormattedPayloadCannotReproject`.
The old core accepted it.

#### R15 (S2). Replace-mode Enter and Backspace in a table cell leave `<br>`

WYSIWYG `| a | b |\n| - | - |\n| 1 | 2 |` (also inside a list item),
`R<CR><BS><Esc>` on `a` gives `| <br>a | b |…`. Backspace no longer
restores the cell. The old core restored the exact source.

#### R16 (S2). Source `:v`/`:g` deletion refuses fence lines

Markdown Source, ```` ```rust\nfn main() {}\n``` ````, `:v/a/d` fails with
"this edit is not supported for the selected content structure". Source view
exposes the fence lines as ordinary text, and the old core deleted them,
leaving `fn main() {}`.

#### R17 (S2). `:s/^/#/` in a list item leaves a stale projection

WYSIWYG `- a\n- b\n- c`, `j:s/^/#/` writes `- a\n- #b\n- c` (the same source
as the old core), but the incremental list blocks no longer match a fresh
parse. `1. a\n2. b\n3. c` behaves the same way.

#### R18 (S3). Page Down doesn't move in a document that fits the window

All formats, `a\nb\nc`: Page Down or Ctrl-F (Normal or Insert) leaves the
caret on `a`, and Page Up from `c` leaves it on `c`. The old core moved the
caret (to `b` in both cases), and Vim's Ctrl-F and most editors' Page Down
move toward the end. The new paging rule keeps the caret's screen position
when the page can't scroll, which looks like a dead key whenever the whole
document is visible.

### Earlier findings that are only partly fixed

**C0.** `xyz` (any format), `:v/a/d`, `A`, type `éé`, Escape, `:s/^/#/`
panics in `record_jump` (`src/core/command/mod.rs:14311`, "a retained jump resolves to one
hard line"). The panic is now contained and the next commands work, but that
`:s` keeps failing. The batched `:g`/`:v` deletion leaves a raw jump offset
that later typing doesn't rebase, so H1's claim that retained positions are
rebased anchors doesn't hold on this path. The shortest examples of both
walk panic signatures (75 hits) minimize to this sequence.

**C7.** Enter then Backspace no longer fails, but it rewrites the source:
`*it*` becomes `<em>i</em>*t*`, `_it_` becomes `<em>i</em>_t_`, and `*a\nb*`
becomes `*a*_&#32;b_`. Rejoining two halves of one emphasis should merge
them back into one span.

**C10.** WYSIWYG `<https://auto.link>` or `<mailto:a@b.c>` at the end of a
paragraph: `A`, then a space fails with `FormattedPayloadCannotReproject`
(N2).

**C23.** Typing `1. ` at the start of `0. zero`, `1) a\n2) b`, `- a\n1. b` or
`-\ta\n-\tb` gives sources such as `0. 1. zero`, a nested list on reopen,
while the incremental text is still `1. zero`.

**C32.** See R10.

**C34.** WYSIWYG, both cores: `:t.` on the definition line of
`[ref]\n\n[ref]: /url`, `:m+1` in `1. a\n   - b\n2. c`, `:m0` on the
continuation paragraph of `- a\n\n  continued\n- b`, and `:t.` in
`    a\n\n    b` all fail with `HardLineTransferProjectionMismatch`.

**C38.** WYSIWYG `**[bold link](u)**\n\nx`, `dd` gives
`<strong>[](u)</strong>x`: an invisible empty link inside `<strong>`, now
joined to the next paragraph.

**C43.** Bulleted List on `para` in `para\n| a | b |\n| - | - |\n| 1 | 2 |`, and
Block Quote on the last item of `- a\n  - b\n    - c\n      - d`, still fail
with `VerificationFailed`.

**C44 and C42.** WYSIWYG `- a\n  - b\n    - c\n      - d`: Code or Bold typing
style, then `x`, fails with `VerificationFailed` at any caret on the third or
fourth level (offsets 4–7). Plain typing works there.

**C59.** `:g/al/d` on 4,000 lines now takes 40–180 ms in Text, Code and
Source. WYSIWYG still costs about 10 ms per target. On the 344-line
`docs/markdown_demo.md`, `:g/a/d` stops at the 250 ms budget after 20–34
targets, so it takes nine invocations and then hits the table refusal.
After `:g` on a long file, the caret is left on the last affected line but
the view isn't scrolled to it. The batching also introduced R3.

**C65.** Copy then Paste through the portable payload still fails in about
one seed in three. Examples: copy `A` from `# A\n## B` and paste into `B`
(`VerificationFailed`); copy `h` from `https://bare.link.example` and paste
at 2, or copy `u` from `<ins>under</ins> <sup>sup</sup>` and paste at 5
(`UnsupportedFormatting`); copy across cells of a table in a list and paste
at the start (`AmbiguousProjection`). The reported cases pass.

**C68.** Code, `fn main() {\n    println!("hi");\n}\n`, cursor on the second
space of line 2, `R<CR><BS><Esc>` gives `fn main() {\n \nprintln!("hi");\n}\n`.
Three spaces and the line join are lost (both cores). WYSIWYG `>` with
`R<CR><BS><Esc>` deletes the `>`.

**C76.** `| abc |\n| --- |` with `ciwz<Esc>` gives `|**z**  |\n| --- |`. The
header's bold appearance becomes explicit `**` source (compare open
question 23), and the cell's leading space is lost.

### New findings

These reproduce on both cores unless noted, and weren't in the previous
report.

#### N1 (S1). Paragraph motions and objects span the whole Markdown document

Markdown Source and WYSIWYG, `# Title\n\nFirst para line one\nline two.\n\n- item a\n- item b\n\nLast para.`,
cursor in `First para`:

- `}` goes to the end of the document and `{` from the end goes to the start;
- `dap` leaves `# ` in WYSIWYG and an empty file in Source;
- `cipX<Esc>` replaces the whole document with `X`;
- `d}` deletes everything after the cursor;
- `>ap` indents every block, giving `# &#32; Title` and `- &#32; item a` in
  WYSIWYG;
- `yap` yanks the whole document.

The paragraph motions use Vim's blank-line rule on the projected text, but
neither view has blank lines there: WYSIWYG separates paragraphs with one
hard break, and Source collapses separator lines (open question 7). So the
whole document is one paragraph. Plain and Code are unaffected. Deleting a
paragraph with `dap` is one of the most common Vim edits, and here it
silently deletes the document. Paragraph motions in these views need
structural paragraph boundaries; open question 28 asks which ones.

#### N2 (S1). Can't type a space after an autolink at the end of a paragraph

WYSIWYG `<https://auto.link>`, `see <https://auto.link>` or
`<mailto:a@b.c>`: `A`, then space fails with `FormattedPayloadCannotReproject`.
A following `x` succeeds but goes inside the link (`<https://auto.linkx>`).
Typing at the end of a link continues it, as AGENTS.md specifies, but a
space can't be part of an autolink, so the user can't continue the sentence.
Bare URLs work.

#### N3 (S1). Empty cells of a table inside a block quote reject typing

WYSIWYG `> |  |  |\n> | - | - |\n> |  |  |`: typing in any empty cell fails
with `AmbiguousProjection`. Cells with text accept typing, but emptying one
with Backspace makes it fail. Insert Table inside any quote creates such a
table, so the cells of a newly inserted quoted table can't be filled. Tables
in nested quotes fail too. Tables in lists, in a quote inside a list item, and
in a list inside a quote work.

#### N4 (S2). Deleting rows into a code block strips the block

WYSIWYG ``p\n\n```\nc1\nc2\n```\n\nz``, `2dd` (or `Vjd`) on `p` deletes `p`
and `c1` but leaves `c2\n\n\nz`: the fences are gone and `c2` becomes an
ordinary paragraph. From a heading (``# T\n\n```…``) the result is `# c2`. The
deletion is joined into the first paragraph as a character deletion would
be. A complete-row deletion should keep the surviving code lines in their
block.

#### N5 (S1). Change commands on an autolink-only paragraph

WYSIWYG `<https://auto.link>`: `cc` and `S` fail with `VerificationFailed`.
Because a failed change leaves Normal mode (H3), the next typed `x` deletes
the `h`. `x\n\n<https://auto.link>` with `Gdd` fails too. (`dd` on a
one-paragraph autolink document now works.)

#### N6 (S2). Changing the paragraph after a setext heading

WYSIWYG `a\n=\n\nb`: `cc`, `S`, `C`, `ciw`, `cw` and `s` on `b` fail with
`AmbiguousProjection`. With H3, `ccx` deletes `b`.

#### N7 (S2). Formatting actions on common shapes

- Enter before an image-only paragraph (`![](local.png)`, Insert at 0) fails
  with `UnsupportedFormatting`, so a paragraph can't be added above an image.
- Block Quote off on `- > quote in list` fails with `VerificationFailed`.
- Paragraph style on `# Heading #` (ATX with a closing sequence) fails with
  `VerificationFailed`.

#### N8 (S1). Typing at the start of some headings and items

WYSIWYG `# \\a` (a heading whose text starts with an escaped backslash) and
`- </ins>`: typing at the start fails with `FormattedPayloadCannotReproject`.
The walks reached this repeatedly through `# \\…` headings created by typing.

#### N9 (S2). Display differs from the file after `r` and some typing

- WYSIWYG `- a\n- b`, `r-` on `a` (or `vr-`) writes `- -\n- b`: the item text
  shows `-`, but the file reopens as a nested empty list. Insert-mode typing
  escapes the same character (`- \-`). Every list shape in the corpus does
  this.
- WYSIWYG `- `, `A`, type `---` gives `- ---` (a thematic break in the item)
  while the incremental projection still has a list paragraph.
- WYSIWYG `:s/^/#/` on `- a\n\n- b`, `1) a\n2) b` or the continuation of
  `- a\n\n  continued\n- b` leaves stale list blocks.
- Source tables: see R12.

#### N10 (S2). Smaller Vim command failures

- Source `a\né👩‍💻`, `G0r<CR>`: `NotGraphemeBoundary(3)`, a raw coordinate
  error.
- WYSIWYG `[text]()`, `C x Esc` gives `[x](<>)[]()`: the emptied link stays in
  the file next to a new one.
- WYSIWYG `:m0`/`:t0` at narrow widths fail with "This physical destination
  is inside hidden formatting" when the current row is part of a wrapped
  heading.
- WYSIWYG wide tables at 140 points: `:$d`, `:.,$d`, `:m$` and `:t$` fail with
  `Layout(OutsideMaterializedCoverage)`, and so does a composition in a cell
  of a table inside a list item.

#### N11 (S3). Small typing and insertion inconsistencies

- Insert Table at the start of a quoted paragraph adds an empty quoted
  paragraph before the table (`> nested` becomes `> \n> \n> |  |  |…`). At
  top level it doesn't (open question 32).
- Typing into an empty cell of a table inserted at top level writes `|j  |`,
  without the leading space.
- `==` after a `/* … */` comment gives the next line the star column's
  indentation (` fn f() {}`), and `==` on a closing `}` copies the body's
  indentation. Both follow AGENTS.md's "copies preceding nonblank
  indentation" literally (open question 30).

#### N12 (S1/S2). Gutter row selection

These are new with the row-selection gutter:

- WYSIWYG `# T\n\nend`: select the heading row in the gutter, Backspace. The
  result is `# end`: the heading text is deleted, but the following
  paragraph takes the heading style. `> q\n\nz` gives `> z`. A whole-row
  selection is treated like a character selection crossing the paragraph
  boundary, which joins into the first paragraph.
- Copy a selected row, then Paste over the same selection: fails with
  `VerificationFailed` for headings, list items and a paragraph above a
  table. In `> q\n\nz`, it gives `> q\n>\n> z`, pulling `z` into the quote.
- In a table, the gutter selects the whole semantic row as specified, but
  Backspace, Delete, typing and Paste then fail with "Editing across table
  cell boundaries requires a cell selection or a row action." The message
  is wrong for a row selection. The table spec says Delete clears the
  selected cells; deleting the row would also be reasonable (open
  question 29).

#### N13 (S3). Line numbers overlap in header-only tables

`:set number`, WYSIWYG `| a |\n| --- |\n`, Enter in the header cell (giving
`| a<br> |`): source lines 1 and 2 get the same band (0–45 points), so the
numbers draw on top of each other and the hidden delimiter line is marked
current. Deleting body rows (`dG`, `Vjd`, Delete Row) and some compositions
over a selection in a table do the same. Apart from this, the line-number export and
the status location never failed in the line-number probes or the walks.

#### N14 (S3). Code-block language selector

The selector works for backtick and tilde fences, indented code (converted
to a fence), code in lists and quotes, unclosed fences and `<pre>`, with
exact undo. Two cosmetic leftovers: clearing the language of
`<pre lang="py">` leaves `<pre >`, and clearing ```` ``` {.rust} ```` leaves a
trailing space after the fence.

#### N15 (S3). `#L` links

`#L3` and similar fragments navigate correctly, including to hidden lines,
`#L03`, and numbers past the end. GitHub's range form `#L5-L7` is treated as
a missing heading and doesn't navigate. It could go to line 5 (open
question 31).

#### N16. Split panes: no defects found

[`examples/fuzz_panes.rs`](examples/fuzz_panes.rs) ran 30,000 sequences of
300 operations: split, close, status-bar and splitter drags, window and
status-height changes, equalize, resize, focus, rotate, exchange and move to
edge. After every operation it checked that the operations the macOS wrapper
guards with `precondition` (`update`, `drag`, `remove`, `copy`) succeed, that
frames tile the window without overlap, that panes keep 100 DIPs of width
and their status height, and that `can_split` agrees with `split`. Nothing
failed.

### Harness and tooling changes

- `fuzz_consistency case FILE` runs one JSON case and prints per-action
  results, state and invariant problems as JSON. `tools/regress.py` and
  `tools/regress_cases.py` replay every repro in this file with it; set `FC`
  to compare another build.
- New actions: `number`, `row_select`, `go_to_line`, `link_fragment` and
  `code_language`, used by the walks. `FUZZ_NUMBER=1` opens every probe with
  line numbers on. After every action the harness now also checks
  `physical_line_location` (the status bar) and, with line numbers on, the
  gutter export and its region invariants.
- Undo checks allow one undo per separate undo unit (link and image
  insertion, paste, and each committed composition), as AGENTS.md specifies.
  The previous single-undo check reported thousands of false undo
  mismatches.
- More word-motion cases in the Vim differential (`ex`, `Ex`, `2ex`, `gex`,
  `wx`, `bx`, `cWx`, `c2wx`, `cex`, `dE`, `yeP`, `d2w`, `}x`, `{x`, `fax`).
- [`examples/fuzz_panes.rs`](examples/fuzz_panes.rs): see N16.

Remaining harness noise worth knowing about when triaging: the
`code_language` action off a code block, zero-row Insert Table, a second
composition while one is open (`AlreadyActive`), row selection during an
open composition (`StaleLayout`), and Vim statuses such as "already at
newest jump" are reported but aren't bugs.

---

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

The implementation pass after this audit removed the findings it fixed and
recorded changed interpretations under Modified items. The October 11
fix-verification audit at the top of this file checks those fixes; it lists
regressions, findings that are only partly fixed, and new findings. Open
questions remain undecided; no policy changes for them were implemented.

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
   with line wraps? RESOLUTION: Yes
2. **Deleting the last character of a continuation line.** `ab\nc` with
   Backspace after `c` leaves `ab ` with a visible trailing space, and
   `ab\nc\nd` gives `ab&#32;\nd`. Should the folded break be removed with
   the line's last character? RESOLUTION: I don't understand, please expand on the options, I don't know what a "continuation" line is.
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

The following were added by the October 11 audit.

25. **Shifting a wrapped row (C74).** Under Visual line meaning, `>>`, `<<`
    and `V>` on the second or later display row of a soft-wrapped line insert
    indentation in the middle of the sentence (`jumps   over the lazy dog`),
    in Plain and WYSIWYG. Modified items keep this as the displayed-row
    policy. No user means to indent mid-sentence; should row-based shifts
    apply to the row's hard line, or be refused on continuation rows?
26. **`yyp` on a partial row (C79).** `yyp` on an interior display row puts
    the row's text characterwise after the cursor, splitting a word. Should a
    row yank remember that it was a whole row and put it after the row's end?
27. **`:N` past the end of the file.** `99G` clamps to the last line, but
    `:99` fails with `AddressOutOfBounds`. AGENTS.md asks for both behaviors,
    but Vim moves the cursor to the last line for `:99` (it also reports
    E16), and `:9999` is a common way to reach the end. Should a bare `:N`
    (go to line) move to the last line, while ranges given to other commands
    keep rejecting?
28. **Paragraphs in Markdown views (N1).** Once `}`, `{`, `ap` and `ip` stop
    spanning the whole document, what counts as a paragraph in WYSIWYG and
    Source: each block (heading, list item, table, code block), or runs of
    ordinary paragraphs as separated in the source? Vim users will expect
    `dap` on a list item to delete that item, or the whole list.
29. **Deleting a gutter row selection (N12).** Should Backspace on a
    gutter-selected heading row remove the row's paragraph (so `# T\n\nend`
    becomes `end`), as a linewise delete does, rather than joining the next
    paragraph into the heading? On a gutter-selected table row, should
    Delete clear the cells (the table spec's selection rule) or delete the
    row?
30. **Reindent around braces and block comments (N11).** `==` copies the
    preceding nonblank line's indentation, so a closing `}` keeps the body's
    indentation and a line after ` */` gets one space. Keep the literal rule,
    or skip comment-continuation lines and dedent closing brackets?
31. **GitHub line ranges (N15).** Should `#L5-L7` navigate to line 5 (and
    perhaps select through line 7), as GitHub does?
32. **Table inserted at the start of a quoted paragraph (N11).** Inside a
    quote, Insert Table at offset 0 adds an empty quoted paragraph before the
    table; at top level it doesn't. Which is intended?

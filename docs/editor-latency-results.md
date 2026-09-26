# Large-document editing and scrolling latency

This report records the interactive-latency work on the portable core for
large documents with complex formatting and syntax highlighting: what the probe
measures, what it found, what changed, and the before/after numbers.

## Probe

`examples/editor_perf.rs` drives `Core` the way a frontend does, with the mock
shaping provider, and records wall time per operation:

```sh
cargo build --release --example editor_perf
./target/release/examples/editor_perf <scenario> [size_bytes] [json_out]
```

Scenarios: `markdown` (WYSIWYG, `AGENTS.md` repeated to the requested size),
`markdown_source`, `html` (generated sections with headings, inline formatting,
links, code, lists, quotes and `pre`), `code` (this repository's Rust sources,
Tree-sitter highlighting enabled) and `text` (short plain lines). Each run
opens the document, attaches an 800×600 view, wheel-scrolls 300 steps, pages
down 60 times, jumps to the end/start/middle, types four sentences in the
middle of the document one character at a time with Enter between them, undoes
them, then attaches a second 480-wide view and repeats typing with both views
attached. Code runs also poll syntax between events and measure how long a
highlight publication takes to settle after an edit.

The numbers are core costs with mock shaping on a shared 4-core Linux
container; they are not native frame times. p50/p95 are over the recorded
samples of one run.

## Findings

Profiling (`pprof` sampling in a scratch harness) attributed the baseline costs
to a few whole-document operations that ran on every keystroke:

1. **Markdown WYSIWYG typing reprojected the whole document.**
   `prepare_formatted_payload_edits` routed literal formats and HTML/RTF to the
   bounded text-edit preparation, but not Markdown, so every character ran
   `project_markdown` over the complete source and then re-walked the whole
   new projection for history memory accounting. At 1.3 MB this was 826 ms per
   character.
2. **Persistent range stores were flattened through `Deref`.**
   `presentation_line_at_offset` (every layout) and `map_source_boundary`
   (HTML typing, undo) used slice methods on the persistent stores, which
   materialize a flat copy of every record after each edit invalidates it.
3. **Every edit discarded the view's layout cache and exact heights.**
   `synchronize_document_hard_line_count(…, document_is_stale = true)` dropped
   the regional cache and all exact heights whenever the document revision
   changed, so the whole viewport was re-wrapped, re-positioned and re-cloned
   per keystroke, and the scroll extent fell back to estimates.
4. **Typing into a fresh empty Markdown paragraph missed the regional path**
   because an empty paragraph has no provenance span to locate its physical
   source row, so the first character after Enter reprojected the document.
5. **Code highlight publication** re-ran a grapheme-cluster search at every
   run boundary and compared the metrics of every automatic style span in the
   old and new projections on each publication.

## Changes

- **Markdown shares the bounded text-edit route** for character payloads. An
  edit that route cannot verify (for example a backtick typed into a fenced
  list item) keeps the previous whole-document preparation as its reference
  behavior; the affected suites are unchanged.
- **Indexed lookups replace flattening.** `presentation_line_at_offset` uses
  the store's tree partition; `map_source_boundary` uses the source-ordered
  boundary index for exact contributor edges and scans provenance only for an
  interior point.
- **Layout state rebases across an edit.** Each committed state transaction
  records its exact formatted change (`Document::layout_change_between`):
  the replaced old/new hull, whether the projection was rebuilt regionally, and
  the line counts. `ViewLayout::rebase_document_change` then splices the height
  index for the lines of the touched blocks (plus one neighbour on each side,
  because paragraph spacing depends on the following paragraph), keeps cached
  lines before the change untouched, and records a pending line/offset shift
  for cached lines after it; the shifted copy is materialized only for an
  actual cache hit. Every cached line stores the style inputs it was computed
  from (paragraph style relative to the line, overlapping style runs, following
  paragraph spacing, document defaults) and is reused only while the freshly
  resolved inputs match, so a regional reparse that restyles an unchanged line
  cannot be served stale. Whole-document reprojections, history navigation and
  any inconsistent state keep the previous full invalidation.
- **Empty-paragraph insertion** resolves its physical source row through the
  rich insertion resolver when no provenance span exists.
- **Code publication** skips the cluster search for run edges between two
  ASCII bytes (always a grapheme boundary except CR LF) and skips the span
  metric comparison when no automatic style of either sheet can change
  metrics.

## Results

All measurements: Linux x86-64 container, 4 cores, `rustc 1.94`, release
build with the mock shaping provider, one run each. "Before" is commit
`5e31053` plus the probe; "after" is this change set. The syntax figures for
Code are a full Tree-sitter parse of 2 MB of Rust; the probe waits for two
idle seconds before declaring highlighting settled, so "settle" values include
that idle wait.

**4 MB plain text (108k lines)** — open 102.3 → 100.9 ms, first view 3.6 → 2.8 ms

| Operation | p50 before → after (ms) | p95 before → after (ms) | max before → after (ms) |
| --- | ---: | ---: | ---: |
| Type one character (176 samples) | 5.0 → 5.6 | 6.4 → 6.0 | 11.4 → 9.4 |
| Type with two views (25) | 7.8 → 4.3 | 8.4 → 4.8 | 8.5 → 5.1 |
| Enter (4) | 6.3 → 5.7 | 6.6 → 5.8 | 6.6 → 5.8 |
| Undo (4) | 3.8 → 4.4 | 3.9 → 11.2 | 3.9 → 11.2 |
| Wheel step, 60 units (300) | 0.0 → 0.0 | 2.5 → 2.7 | 3.0 → 3.2 |
| Page Down (60) | 2.1 → 2.5 | 3.5 → 3.9 | 104.2 → 106.5 |
| G / gg / :N jumps (15) | 1.4 → 1.6 | 3.6 → 3.8 | 5.7 → 5.6 |
| Second view: wrap toggle + G/Ctrl-U (10) | 2.2 → 2.4 | 2.8 → 3.1 | 2.8 → 3.1 |

**1.3 MB Markdown WYSIWYG (AGENTS.md ×3)** — open 683.0 → 662.9 ms, first view 2.3 → 2.3 ms

| Operation | p50 before → after (ms) | p95 before → after (ms) | max before → after (ms) |
| --- | ---: | ---: | ---: |
| Type one character (176 samples) | 826.4 → 5.1 | 909.4 → 6.9 | 1002.6 → 10.8 |
| Type with two views (25) | 840.1 → 15.6 | 900.0 → 17.0 | 964.9 → 17.2 |
| Enter (4) | 1474.1 → 1322.0 | 1498.5 → 1433.6 | 1498.5 → 1433.6 |
| Undo (4) | 171.4 → 137.5 | 241.4 → 197.8 | 241.4 → 197.8 |
| Wheel step, 60 units (300) | 0.0 → 0.0 | 3.3 → 3.2 | 8.6 → 8.2 |
| Page Down (60) | 4.4 → 4.6 | 8.3 → 8.8 | 8.9 → 11.3 |
| G / gg / :N jumps (15) | 2.6 → 2.8 | 28.8 → 30.0 | 42.0 → 44.0 |
| Second view: wrap toggle + G/Ctrl-U (10) | 38.4 → 21.2 | 41.7 → 25.7 | 41.7 → 25.7 |

**1.2 MB HTML WYSIWYG** — open 893.0 → 844.3 ms, first view 7.0 → 6.7 ms

| Operation | p50 before → after (ms) | p95 before → after (ms) | max before → after (ms) |
| --- | ---: | ---: | ---: |
| Type one character (176 samples) | 33.2 → 4.8 | 49.5 → 7.4 | 1317.8 → 977.1 |
| Type with two views (25) | 53.5 → 8.9 | 69.0 → 10.6 | 72.1 → 11.9 |
| Enter (4) | 1427.2 → 1163.5 | 1508.1 → 1252.6 | 1508.1 → 1252.6 |
| Undo (4) | 249.8 → 179.4 | 252.3 → 182.5 | 252.3 → 182.5 |
| Wheel step, 60 units (300) | 0.0 → 0.0 | 3.1 → 3.1 | 5.1 → 5.7 |
| Page Down (60) | 2.0 → 1.9 | 4.2 → 4.1 | 116.8 → 117.1 |
| G / gg / :N jumps (15) | 1.5 → 1.6 | 3.4 → 3.4 | 11.3 → 10.8 |
| Second view: wrap toggle + G/Ctrl-U (10) | 14.2 → 7.8 | 17.4 → 11.0 | 17.4 → 11.0 |

**2 MB Rust, Tree-sitter highlighting** — open 47.1 → 47.2 ms, first view 3.1 → 3.2 ms

| Operation | p50 before → after (ms) | p95 before → after (ms) | max before → after (ms) |
| --- | ---: | ---: | ---: |
| Type one character (176 samples) | 36.0 → 24.3 | 71.3 → 31.6 | 96.4 → 40.0 |
| Type with two views (25) | 39.1 → 25.7 | 41.2 → 30.3 | 43.3 → 45.3 |
| Enter (4) | 56.3 → 24.9 | 58.8 → 36.8 | 58.8 → 36.8 |
| Undo (4) | 36.3 → 46.0 | 37.9 → 64.0 | 37.9 → 64.0 |
| Wheel step, 60 units (300) | 2.9 → 2.1 | 4.2 → 2.8 | 6.6 → 7.7 |
| Page Down (60) | 15.9 → 4.6 | 31.7 → 21.7 | 45.2 → 23.3 |
| G / gg / :N jumps (15) | 3.5 → 3.0 | 7.6 → 10.9 | 29.7 → 36.3 |
| Second view: wrap toggle + G/Ctrl-U (10) | 3.8 → 4.7 | 4.9 → 5.9 | 4.9 → 5.9 |

Syntax (before): first highlight settle 2563.3 ms; highlight settle after an edit p50 2558.4 ms; poll p95 0.0 ms, max 48.3 ms.

Syntax (after): first highlight settle 2581.8 ms; highlight settle after an edit p50 2538.5 ms; poll p95 0.0 ms, max 23.7 ms.

**37 MB plain text (1M lines), after only** — open — → 1075.1 ms, first view — → 2.8 ms

| Operation | p50 before → after (ms) | p95 before → after (ms) | max before → after (ms) |
| --- | ---: | ---: | ---: |
| Type one character (176 samples) | — → 5.9 | — → 6.2 | — → 7.2 |
| Type with two views (25) | — → 4.5 | — → 4.9 | — → 5.2 |
| Enter (4) | — → 6.0 | — → 6.1 | — → 6.1 |
| Undo (4) | — → 4.7 | — → 8.5 | — → 8.5 |
| Wheel step, 60 units (300) | — → 0.0 | — → 2.4 | — → 2.9 |
| Page Down (60) | — → 2.3 | — → 3.8 | — → 92.6 |
| G / gg / :N jumps (15) | — → 1.5 | — → 3.7 | — → 5.6 |
| Second view: wrap toggle + G/Ctrl-U (10) | — → 2.3 | — → 3.1 | — → 3.1 |

## Code typing follow-up

After the changes above, Code typing with Tree-sitter highlighting still cost
about 24 ms per keystroke against 7 ms with highlighting off. Profiling showed
three per-keystroke costs proportional to everything the syntax service had
retained, rather than to the edit:

1. `SyntaxService::runs` concatenated and cloned every cached and retained run
   (each carrying two heap `String`s) and sorted them, and `rebase_input`
   then copied the whole set again to shift it past the edit.
2. `install_code_styles` rebuilt the presentation's automatic style store from
   every run, running name resolution and grapheme checks on each one.
3. `ViewLayout::rebase_document_change` rebuilt the regional layout cache by
   re-inserting every cached line, and re-measured each line's retained bytes
   by walking its rows.

The follow-up makes each of these proportional to the change:

- **Shared run names.** `SyntaxStyleName` and `SyntaxRun::origin` are
  `Arc<str>`. Tree-sitter already produces one name per capture; the Vim
  provider interns group and origin names per job. Copying a run copies two
  pointers.
- **An indexed run store.** The service keeps the displayed runs of the
  current input in one persistent interval index (`RunStore`) instead of a
  run list per cached result plus a retained list. An edit drops the runs
  inside the replaced hull and shifts the rest with one lazy coordinate
  change; a completed result splices only the index span its coverage
  overlaps. Result cache entries now record coverage only. Eviction removes
  runs from the front of the document, preferring runs outside current
  provider coverage. The store tracks referenced names with counts, so the
  Character menu's name list no longer scans the runs.
- **A publication delta.** The service records what changed since the last
  publication: the one edit hull and the regions whose runs were replaced.
  When the presentation lineage connects the previous publication to the
  current revision exactly, `install_code_presentation_delta` splices the
  previous presentation's automatic spans the same way and resolves only the
  runs inside replaced regions. Two edits between publications, a new input
  or configuration, or any inconsistency falls back to the previous
  whole-store installation. Metric changes are compared only inside the delta
  regions, and not at all when no Code style changes metrics (memoized per
  sheet revision).
- **In-place layout cache rebase.** Each cached line records its estimated
  bytes when it is installed. A rebase removes only the invalidated lines and
  lines crossing the hull, then adjusts the pending shift of later lines in
  place. When the line count is unchanged it rewrites no keys and no recency
  entries.

`in_place_cache_rebase_keeps_accounting_and_matches_fresh_layout` checks
running accounting and recency against the entries through typing, Enter
and Backspace, and compares the rebased rows with a freshly built layout.
Absolute y near line 10,000 differs by float summation order between a
spliced and a fresh height index, so the test compares row y relative to the
first compared row within two f32 ULPs.

**2 MB Rust, Tree-sitter highlighting, follow-up** — "before" is the
previous change set (`867d832`) and "after" is this one, built and run back
to back on the same container; open 52.4 → 53.4 ms, first view 3.6 → 3.6 ms

| Operation | p50 before → after (ms) | p95 before → after (ms) | max before → after (ms) |
| --- | ---: | ---: | ---: |
| Type one character (176 samples) | 27.7 → 3.5 | 40.2 → 4.7 | 50.7 → 5.8 |
| Type with two views (25) | 36.2 → 6.4 | 41.9 → 8.3 | 44.3 → 9.5 |
| Enter (4) | 33.2 → 4.0 | 35.2 → 4.2 | 35.2 → 4.2 |
| Undo (4) | 44.4 → 25.6 | 46.2 → 39.8 | 46.2 → 39.8 |
| Wheel step, 60 units (300) | 2.4 → 2.2 | 3.9 → 2.8 | 5.9 → 4.9 |
| Page Down (60) | 6.1 → 3.9 | 29.1 → 8.3 | 36.1 → 10.9 |
| G / gg / :N jumps (15) | 3.3 → 2.9 | 12.5 → 5.0 | 45.7 → 6.8 |
| Second view: wrap toggle + G/Ctrl-U (10) | 5.3 → 4.6 | 6.8 → 5.8 | 6.8 → 5.8 |

Syntax: first highlight settle 2611.6 → 2587.5 ms; highlight settle after an
edit p50 2592.3 → 2594.2 ms; poll p95 0.03 → 0.02 ms, max 27.1 → 1.8 ms. The
settle figures are dominated by the probe's two-second idle wait and the
Tree-sitter parse itself, which runs on a worker.

The container ran about 10–25% slower during this follow-up than for the
first results table. Interleaved runs of both builds on the Markdown and HTML
fixtures show no regression there: typing is equal or faster (Markdown
6.6–7.4 → 6.2–6.6 ms, HTML 7.9–8.3 → 6.4–6.9 ms at p50), and Enter, undo and
Page Down are within run-to-run noise.

Code typing with highlighting now costs about the same as plain-text typing
(2.5 ms per character on the 4 MB text fixture under the profiler). The
remaining main-thread time is viewport layout of the edited line and snapshot
publication.

## Markdown Source Return between continuation lines

In Markdown Source, pressing Return at the end of a line inside a
multi-line paragraph failed with `VerificationFailed`. For example, `:2`,
`A`, Enter on

```text
first line of prose
second line of prose
third line of prose
fourth line
```

authored two source line endings after `second line of prose`. The following
continuation line already owned one ending, so the source became three
consecutive endings. Markdown Source pairs endings into paragraph separators,
and three endings fold back to one displayed break, so the edit did not
produce the requested boundary.

Return now counts the displayed breaks adjacent to the caret and the physical
endings they already own, using the same neighbour walk that source-text
replacement uses (`markdown_source_break_neighbors`). It authors only the
endings needed for every displayed break to own a canonical pair. The example
above now writes four endings. Returning at a paragraph end and in the middle
of a line keeps writing one pair. Regression tests cover both cases, source
bytes and undo. The `markdown_source` probe scenario, which previously
aborted on this edit, now completes.

## Markdown Source typing

Typing one character in Markdown Source reprojected the whole document:
about 850 ms per keystroke on the 1.2 MB fixture. Work counters showed one
full projection per keystroke, which copied, decoded and projected every
source byte. Cost grew linearly with the file: 4.7 ms at 4 KB, 856 ms at
1.1 MB.

**Cause.** Typing reaches the document as a formatted payload.
`prepare_formatted_payload_edits` tried the regional text-edit route only for
WYSIWYG formats, so Markdown Source always took the payload path's complete
reparse. A second rule sent any edit next to a list item to the complete
reparse, which included the prose line directly above a list.

**Routing.** A Markdown Source payload without line breaks is now an ordinary
text splice and goes through `prepare_text_edits_with_patches`. That route
already keeps a proven line-local edit regional and reparses the whole
source when it is not. An edit it cannot prepare falls back to the payload
path, as before.

**Proving a regional result exact.** The Markdown Source parser runs over the
whole document in several passes, so a regional reparse is only correct when
the region does not depend on the rest of the document. A regional result is
now published only when `verify_markdown_source_region` proves two things:

1. The old region, parsed on its own, reproduces the old full projection for
   those rows: text, blocks, flow rows and style spans.
2. Every unchanged row around the edited rows parses as before, so the edit
   did not reshape its neighbours.

The region is chosen so that these checks can pass. It covers the edited
rows' whole paragraphs, since a link can span a paragraph's rows, and never
ends on an empty row. It takes in an adjacent list from its first item, and
starts and ends on paragraph boundaries when they fit within 64 rows. A
region cut inside a longer paragraph is accepted only when the rows outside
it keep their parser state. For plain paragraphs that state is whether a
hard break has already occurred, since this parser ends a paragraph at the
first row without a hard break that follows one. Edits in such a long
paragraph must also leave link syntax and the row's hard-break state alone.

An edit that cannot be proven is treated exactly like an edit without a
local region: the complete reparse publishes whatever the new source parses
to. The regional builder falls back to its complete candidate in the same
way, for entry points such as list actions and paragraph styles.

**List edits.** Edits inside a list item keep the old block partition, since
a region cannot see the enclosing list. They now also accept ordinary
punctuation. They require a letter to begin the row's content before and
after the edit, since the first character selects the row's block syntax,
and text inserted at a block's first character now belongs to that block.
An inherited partition that no longer covers the text fails verification
instead of panicking.

**Tests.** `tests/all/markdown_source_typing_work.rs` types 17 characters,
including Backspace and Markdown syntax, at every caret position of ten
documents. Each result must match a fresh parse of the same bytes, and the
source must contain exactly the typed text. A second test samples edits in
blocks longer than the regional limit, and a third asserts regional work for
typing on a 2,048-chapter document. On `main`, the first seven of those
documents had 4 edits whose incremental projection differed from a fresh
parse (Backspace in an indented code block's indentation), and 81 edits were
rejected. With this change every edit is accepted and matches a fresh parse.
`late_source_paragraph_edit_keeps_regional_work_after_many_hidden_separators`
now allows 6 projected rows instead of 3, since the region covers the
neighbouring paragraphs.

**1.2 MB Markdown Source** — `main` (`1679353`) and this change, built and run
back to back on the same container, two runs each

| Operation | p50 `main` (ms) | p50 this change (ms) | max `main` (ms) | max this change (ms) |
| --- | ---: | ---: | ---: | ---: |
| Type one character | 842–851 | 4.4–4.7 | 992–1,030 | 8.0–11.0 |
| Type with two views | 840–845 | 7.6–8.2 | 1,013–1,095 | 9.4–13.2 |
| Enter | 837–874 | 857–869 | 840–911 | 876–890 |
| Undo | 207 | 117–138 | 216–217 | 191–221 |

Markdown WYSIWYG, run the same way, is unchanged within run-to-run noise.

## Markdown Source Enter

Enter in Markdown Source reparsed the whole document: about 820 ms on the
1.2 MB fixture. Enter adds a row, and the regional route above refused any
edit that adds or removes a row. The complete candidate also rejected Enter
at positions where the new paragraph reshapes the text beside the break.

**Rows that move.** `line_local_projection_region` now accepts Markdown
Source edits that add or remove line breaks. In `verify_markdown_source_region`,
rows after the edited rows are compared with their counterparts moved by the
change in row count. The same mapping gives kept rows their identities in
`splice_line_local_projection`. A row the edit adds gets a fresh identity,
shared with the paragraph it starts. The region's source rows are rebuilt from
the regional parse's line endings whenever their count changes. That can
happen when the formatted rows do not change, as when an empty list item
becomes a paragraph.

**What the proof adds.** Consecutive source endings pair into paragraph
separators. A run of them just outside the region would join endings an edit
adds or removes at the region's edge if the edge row became blank, so such a
result is rejected. A break inside a paragraph longer than 64 rows can cut a
link spanning it, so it keeps the complete reparse. An edit inside a list
that cannot keep the old block partition, such as Enter continuing an item,
now reparses the whole list, up to 64 rows, instead of the whole document. A
regional attempt that fails partway through falls back to the complete
candidate instead of returning an error.

**Enter that reshapes text.** Enter beside whitespace at a row edge, for
example after a row's trailing hard-break spaces, starts a paragraph whose
edge whitespace the parser trims. The complete candidate's text check then
refused the keystroke. The inserted endings are the authoritative source
edit. A proven regional parse now supplies the formatted edit: the smallest
change covering the requested break and the parse's text. Unproven cases
publish the complete reparse, as typing a delimiter already does.

**Tests.** `tests/all/markdown_source_typing_work.rs` now also presses Enter,
and types `\n`, `a\nb` and `\n\n- x`, at every caret position of the ten
documents. It also presses Enter in the long-block samples. Every accepted
result must match a fresh parse, and a refused Enter must leave the document
unchanged. A new test asserts regional work on the 2,048-chapter document
for these edits:
- Enter at the end of every sample line, including list items, a quote and
  hard-break rows;
- continued typing and Backspace over the new row;
- leaving a list through an empty item.

A survey of 22,590 edits (Enter, Backspace, and typed breaks at every
position of 15 documents) compared this change with `main`. Everywhere `main`
accepted an edit, both produce the same source bytes and a projection equal
to a fresh parse. `main` refused 496 Enter keystrokes; 127 are still refused,
all before a quote marker or inside nested-list continuations. These fail
the same way on `main` and are listed below.

**1.2 MB Markdown Source**: `main` (`0193b9f`) and this change, built and
run alternately on the same container, five runs each.

| Operation | p50 `main` (ms) | p50 this change (ms) | max `main` (ms) | max this change (ms) |
| --- | ---: | ---: | ---: | ---: |
| Enter | 798–838 | 5.1–5.3 | 809–1,518 | 6.7–7.5 |
| Undo | 107–118 | 7.5–10.7 | 174–186 | 13.1–13.9 |
| Type one character | 4.2–4.3 | 4.4–4.7 | 7.1–9.0 | 6.9–8.4 |
| Type with two views | 7.3–7.6 | 8.5–9.2 | 9.0–11.0 | 10.2–12.2 |

Typing is measured after the Enter rounds. On `main`, each Enter's complete
reparse also discards the view's layout cache, so the typing that follows
starts from a smaller cache. With the Enter rounds skipped, `main` types
with two views in 8.4–9.5 ms, the same as this change.

## Remaining work

- **Code publications after an unbounded change** (a new input identity
  without a recorded hull, history navigation, a configuration change, or two
  edits between publications) still install the presentation from every run
  in the store. Undo in Code therefore still costs about 30 ms.

- **Enter in HTML and Markdown WYSIWYG** (paragraph split, and exiting an
  empty list item) still prepares a whole-document candidate:
  `prepare_html_paragraph_split` decodes the complete source to find the
  paragraph owner, `prepare_rich_list_style` runs `html::list_patches` over
  the whole document, and the regional candidates reject a newline because a
  split changes the block topology. A regional split candidate needs
  `splice_line_local_projection` to accept a block-count change for rich
  formats the way it does for literal ones.
- **Undo/redo** restore recorded snapshots, so the layout rebase treats them
  as unbounded changes; they cost one viewport relayout.
- The first Page Down into a region without layout coverage still lays out a
  full band (about 100 ms on the 4 MB text fixture).
- Unlimited history retention (used by the probe) grows the memory ledger's
  hash table, which shows up as a few percent of HTML typing; the shipped
  policy prunes.
- **Enter in Markdown Source** is refused before a quote marker (`>`) and
  in some nested-list continuation rows (`VerificationFailed` or
  `AmbiguousProjection`, as on `main`). Enter inside a paragraph longer than
  64 rows, and Enter on an empty list item followed by a lazy continuation
  row, still take the complete reparse.
- **Long quotes** (more than 64 rows) still take the complete reparse when
  typed into.

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

## Remaining work

- **Code typing with highlighting** is still about 24 ms per keystroke against
  7 ms with highlighting off. Each publication remaps the retained syntax runs
  overlapping the edit, rebuilds the automatic style store from every retained
  run (`install_code_styles`), and the viewport materialization publishes the
  ready result in the same turn. Splicing the retained runs and the style
  store by the edit's hull instead of rebuilding them would remove most of it.

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
- **Pre-existing bug found by the probe (not fixed here):** in Markdown
  Source, `:2`, `A`, Enter on the four-line file
  `first line of prose\nsecond line of prose\nthird line of prose\nfourth line\n`
  fails with `VerificationFailed`; it reproduces on the commit before this
  change set, so the `markdown_source` scenario cannot complete yet.

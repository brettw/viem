# Prose editing and paragraph-flow validation

This change adds `:sort`, structural prose flow, the View menu's **Flow Source
Paragraphs** option, HTML trailing blank lines, and inline-formatting exit.

## Defects reproduced during the quality pass

- With Georgia Italic at 56 pt, putting the block caret on the `i` in `fifty`
  erased ink from the preceding `f` that crossed into the block. Native
  before/after screenshots confirmed the defect and the restored overhang.
  Ordinary text rendering has no per-character clip. The block now redraws
  every intersecting glyph fragment, allowing for the antialiasing fringe.
- At the end of an HTML document, the first Return worked and the second
  returned a format-verification error. Appropriate `<br>` elements now retain
  the requested empty rows, including inside preserved whitespace wrappers.
- After `A` at the end of nested bold/italic HTML text, the native Bold menu
  incorrectly reported Off. Command-B consequently enabled bold again. The
  menu now uses the insertion boundary's inherited style; turning bold off
  retains italic and places subsequent input outside the bold element.
- `:sort iu` initially left the caret near the end of the sorted range because
  an explicit post-edit cursor target was remapped a second time. The target
  now remains authoritative. Visual `:` now opens a prompt with the selected
  hard-line range instead of treating the following `s` as a Visual substitute.
- The expanded native matrix caught Markdown failures in `G0o` followed by
  Unicode input, `ggyyGp`, styled input at the start of a strong span in a
  paragraph spanning source lines, and repeated Return at EOF. These sequences
  are retained as regression tests for projection and reverse-edit changes.
- An IME overlay near the start of a later long paragraph could request an
  inverted checkpoint-cache range. The lookup is now guarded. Single-line
  marked input in a long wrapped paragraph uses bounded, shared-tree slices.
- A distant slice of flowed source text could lose its first paragraph's
  indentation and style identity. Layout now retrieves that paragraph's style
  through an indexed point query, including during composition. Following
  paragraph context no longer captures all its inline styles.
- A native 296 KB paragraph stress check exposed high idle CPU and sluggish
  controls: caret blinking redrew every cluster in an offscreen shaping slice,
  and each glyph repeated color-space conversion. Paint now culls against the
  damaged visible region, preserves neighboring antialiased overhang, and
  resolves foreground colors once per paint run. Synthetic stroke and native
  emoji colors retain raster coverage.

## Automated coverage

- `tests/all/paragraph_flow.rs` and coordinator composition tests: source-flow
  geometry and paragraph identity across distant checkpoints, resize, and IME;
  bounded shaping in 300 KB paragraphs, exact cancellation, and cache reuse.
- `tests/all/ex_sort.rs`: grammar, ranges, reverse/unique/numeric/regex keys, source
  preservation, rich paragraphs, Unicode, encodings and mixed delimiters,
  stale requests, registers, history, anchors, and a 10,000-row sort.
- `tests/all/markdown_typing_boundaries.rs`: source caret exits from nested/triple
  bold and italic delimiters, mid-run formatting splits, Unicode, malformed
  input, exact undo/redo, and local work in a large source buffer.
- `tests/all/html_typing_boundaries.rs` and `tests/all/pending_typing_style.rs`: empty
  HTML, trailing rows, nested inline context, source caret movement, pending
  styles, Unicode, and exact undo/redo.
- `src/mac/Editor/Tests/EVProseQualityTests.swift`: native menu state and editing
  commands across formats, repeated formatting cycles, emoji input, two views,
  metrics invalidation, zoom, widths from 1 to 800 points, and exact source undo.
- `src/mac/Editor/Tests/EVParagraphFlowIntegrationTests.swift`: native View menu
  and C ABI, independent source views, retained editing coordinates and source
  bytes, edit/undo while flowing, and invalid requests.
- `src/mac/Editor/Tests/EVLayoutPaintIntegrationTests.swift`: italic overhang
  under the caret after resizing and font-cache invalidation; viewport-bounded
  drawing resources in a 20,000-line document; visible-only drawing from a
  288 KB flowed paragraph after resizing and metrics invalidation.
- `src/mac/CoreTextProvider/Tests/CoreTextRenderRegistryTests.swift`: cached
  colors preserve foreground, transparency, synthetic stroke, and emoji ink.

Native tests use `scripts/test-mac.sh`, which isolates configuration from the
user's preferences. Computer-use checks run in a separately identified
`ViemTodoValidation.app` with fixtures and configuration under `/private/tmp`.
The user's running editor is not restarted or used for testing.

## Final results

- `cargo test --offline --all-targets --no-fail-fast`: **1,368 passed**.
- `scripts/test-mac.sh`: **267 XCTest + 26 Swift Testing tests passed**.
- `cargo fmt --all -- --check`, `git diff --check`, and compatibility JSON
  validation pass. `.build/Viem.app` was rebuilt and signed after the final
  checked source-caret change.
- Computer use verified Visual/ranged/unique/numeric sorting and undo, HTML
  repeated Return and saved `<br>` rows, and actual source caret movement past
  HTML and Markdown closing markup. Saved fixtures confirm that unrelated
  italic formatting remains active after bold is disabled.
- Screenshots verified paragraph flow in both source views, retained code and
  explicit breaks, and the restored italic overhang beneath the block caret.
- Repeated native checks exercised a 2,000-paragraph Markdown document and a
  296 KB single paragraph, including distant search, macOS dead-key composition,
  commit, and undo. The latter also confirmed that the painting fix removed the
  earlier sustained CPU load and sluggish controls.

## Deliberate scope

Sorting supports plain/source rows and source-preserving Markdown paragraphs
and balanced HTML paragraph siblings. RTF and ambiguous rich owners fail
without flattening the document. Floating-point and locale-dependent sorting
are not supported. The supported sort flags and source paragraph-flow policy
are specified in `AGENTS.md`.

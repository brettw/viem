# TODO

Follow-ups from background pre-rendering (`FULL_PRELAYOUT_SOURCE_BYTES`,
[prelayout.rs](src/core/coordinator/prelayout.rs),
[native background layout](docs/windows-background-layout.md)).

## Windows image parity for background pre-render

Bring the Windows inline-image pipeline to parity with macOS
(`src/mac/CoreTextProvider/Sources/CoreTextImageResources.swift`,
`CoreTextMeasurementProvider.swift`, `CoreTextRenderRegistry.swift`):

- Speculative-layout workers read unknown local image dimensions from the file
  header (same safety checks as decoding, no pixel decode) and record a
  `measured` state, so speculative geometry is final. Probes never start bitmap
  loads; the main thread never touches the filesystem.
- Render resources don't capture bitmaps. Drawing looks up the current bitmap by
  location while its layout geometry matches, so a decoded bitmap whose
  dimensions layout already used only repaints, with no metrics-generation bump.
  Report a dimension change as a geometry change only if layout used the
  previous dimensions.
- Visibility admission (one screen beyond each edge) starts bitmap loads
  directly instead of forcing a reshape.
- Draw from a copy resampled to the device size on a background queue (bounded
  16 entries / 48 MiB), drawing the original until it is ready.

The Windows scheduler and test changes (`src/win/Core/BackgroundLayout.cs`,
`CoreView.cs`, `Diagnostics/BackgroundLayoutTests.cs`) were written without a
Windows toolchain and have never been compiled. Build and run them with
`scripts/test-win.ps1` first. Add tests mirroring
`src/mac/CoreTextProvider/Tests/CoreTextImageResourcesTests.swift`.

The macOS Core Text provider now caches resolved fonts and builds render
resources only for new glyph signatures; the Windows provider may have the same
per-request font resolution cost.

## Remaining per-keystroke cost

A keystroke in the 95 KB Markdown benchmark costs about 9.4 ms natively, about
4.9 ms of it in core input handling and the rest drawing the damaged region;
`viem-shape-bench` measures about 3 ms of core work with Core Text shaping.
Remaining costs, in order of size:

- History retention accounting grows with session length; see
  [Incremental history retention accounting](#incremental-history-retention-accounting).
- Reusing a cached line deep-copies its rows, because caret points embed the
  layout and document revisions; publication copies them again into the flat
  snapshot rows. Stamping identity at export would let unchanged lines be shared.
- Markdown list editing (Enter, renumbering, removing list treatment) builds
  `list_structure()` from the complete block store, retaining a block-record
  copy with each edited snapshot (see "Remaining flat-text consumers" in
  [docs/performance.md](docs/performance.md)).

## Incremental history retention accounting

Every edit makes history commit work proportional to the whole session rather
than to the change. In `src/core/document/history.rs`:

- `History::commit_with_maps` (including replacing the state of an open
  grouped unit) calls `collect_retained_buffers`, which visits every leaf of the
  new state's source rope (`SourceSnapshot::visit_retained_buffers` in
  `src/core/document/source.rs`), hashes each buffer identity and stores a
  sorted `Vec<RetainedBuffer>` on the node.
- `enforce_retention` then rebuilds reference counts for every retained node
  from those vectors (`buffer_reference_counts`), and `refresh_node_memory` /
  `refresh_live_memory` recapture memory-ledger roots.
- `SourceSnapshot::replace` adds a new leaf for every edit and never coalesces
  pieces, so each typed character adds rope pieces. The leaf walk therefore
  grows with the number of edits in the session, the recount with history size
  times pieces per state, and each node's buffer list is itself retained
  metadata proportional to its piece count.

Measured with `viem-shape-bench --typing 8000` on the 95 KB Markdown fixture: the
median keystroke rises from about 2.2 ms to 3.0 ms over 8,000 keystrokes (about
0.1 ms per 1,000). After 4,000 keystrokes these functions are about 20% of a
keystroke's core time.

Possible directions, possibly combined:

- Keep buffer reference counts persistently, updating them when a node is
  added, has its state replaced, is released or compacted, instead of
  recounting every node per commit.
- Derive a new state's buffer set from its predecessor by walking only rope
  subtrees that are not shared by pointer, as `shared_source_edge` already does
  for diffs.
- Coalesce adjacent small typed pieces into shared buffers, if byte identity,
  patch locality and the pointer-based source diff stay correct.

Retention semantics must not change: which nodes are pruned and when,
`retained_source_bytes`, the memory ledger, and command-checkpoint rollback
(which recomputes a restored node's `retained_buffers`). Done when a work counter
shows per-commit accounting bounded by the change across thousands of
single-character commits, the history and retention tests pass, and
`python3 scripts/validate.py memory` stays within its limits.

## Smaller items

- Hard lines larger than the long-line slice size (64 KiB) are left to demand
  layout, so a document containing them never reports "complete" and can still
  shift the scroll extent when they are reached. Lines up to 64 KiB are now
  prepared as single-line jobs.
- Comment Markdown in Code documents is still recognized only for visible
  comments (an `AGENTS.md` requirement), so it can arrive after scrolling.
- Continuing a list at the end of a Markdown document stays regional only when a
  nonblank row precedes the terminal line ending. With a blank final row the
  terminator pairs into a separator and the edit reprojects completely.

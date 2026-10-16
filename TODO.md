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

- History commits walk every source-rope piece (`collect_retained_buffers`) and
  recount every retained node's buffers (`enforce_retention`,
  `buffer_reference_counts`). Every typed character adds pieces, so the median
  keystroke grows by about 0.1 ms per 1,000 keystrokes in a session (2.2 → 3.0 ms
  over 8,000 in `viem-shape-bench --typing 8000`). Keep the accounting
  incremental instead of recomputing it per commit.
- Reusing a cached line deep-copies its rows, because caret points embed the
  layout and document revisions; publication copies them again into the flat
  snapshot rows. Stamping identity at export would let unchanged lines be shared.
- Markdown list editing (Enter, renumbering, removing list treatment) builds
  `list_structure()` from the complete block store, retaining a block-record
  copy with each edited snapshot (see "Remaining flat-text consumers" in
  [docs/performance.md](docs/performance.md)).

## Smaller items

- Retained geometry costs about 25 MiB of live memory per fully pre-rendered
  view of the 95 KB Markdown benchmark (about 265× source, including native
  glyph resources); three such documents with two views each use about 160 MiB
  live, 200 MiB footprint (`viem-shape-bench --memory 3,2`). Decide whether
  inactive documents or panes should drop to the incremental window.
- Hard lines larger than the long-line slice size (64 KiB) are left to demand
  layout, so a document containing them never reports "complete" and can still
  shift the scroll extent when they are reached. Lines up to 64 KiB are now
  prepared as single-line jobs.
- Comment Markdown in Code documents is still recognized only for visible
  comments (an `AGENTS.md` requirement), so it can arrive after scrolling.
- Continuing a list at the end of a Markdown document stays regional only when a
  nonblank row precedes the terminal line ending. With a blank final row the
  terminator pairs into a separator and the edit reprojects completely.

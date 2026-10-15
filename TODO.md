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

## Speed up the Core Text shaping provider

Pre-layout of a 95 KB Markdown document costs about 80 ms of core work with
mock shaping but about 1 s of worker time with Core Text: roughly 0.3 ms per
provider shape call, about 2,900 calls. This bounds how soon a newly opened
document is fully prepared, demand layout when scrolling outruns preparation,
the ~90 ms first frame, and per-keystroke re-shaping of the edited paragraph.

Profile outside the hardened `xctest` host (`sample` gets no stacks there):
attributed-string construction, `TextIndexMap`, typesetter/line creation,
glyph and caret extraction, render-resource installation, arenas. Reduce the
dominant costs with identical rendering. Measure with the opt-in benchmark in
[docs/performance.md](docs/performance.md) (`worker_ms`, `first_frame_ms`,
`typing_ms`).

## Cut per-keystroke core cost in Markdown

Typing costs about 11–13 ms per keystroke natively in a 95 KB Markdown
document, and about 5 ms of core work with mock shaping (`editor_perf`
`type_char` is ~2.3 ms on its own fixture). This predates pre-rendering: typing
with it disabled is no faster. Core profile:

- ~30% `DocumentLayoutStyles::resolve_region` for the whole visible region on
  every keystroke (from `prepare_layout_job` in
  `materialize_immediate_viewport`). Resolve only lines whose inputs can have
  changed; reuse resolved styles for unchanged cached lines.
- ~14% `poll_markdown_code_syntax` reinstalling every code block's runs after
  each edit. Shift unchanged code-block spans across an edit instead (a Code
  analog exists: `install_code_styles_incrementally`).
- ~21% `compute_layout_job`, ~14% viewport installation.

## Smaller items

- Continuing a list at the very end of a Markdown document reprojects
  completely (no local change). That discards retained geometry and exact
  heights until speculation prepares them again.
- Retained geometry costs about 21 MB of core estimate for a 95 KB document
  (~220× source), plus native glyph resources, per view. Measure whole-app
  memory with several documents and split views open, and decide whether
  inactive documents should drop to the incremental window.
- Hard lines larger than one speculative job (16 KiB) are left to demand
  layout, so a document containing them never reports "complete" and can still
  shift the scroll extent when they are reached.
- Comment Markdown in Code documents is still recognized only for visible
  comments (an `AGENTS.md` requirement), so it can arrive after scrolling.
- Scrolling immediately after opening can still produce one ~30 ms core frame
  while the first syntax publication and visible relayout land.

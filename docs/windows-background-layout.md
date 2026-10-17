# Native background layout

macOS and Windows connect the [portable speculative-layout policy](../src/core/coordinator/prelayout.rs)
to native workers. The core chooses the work and validates every dependency
when a result is installed; foreground input proceeds while workers run.

## Window and retention

`FULL_PRELAYOUT_SOURCE_BYTES` (100 KiB) is the single tuning point. A document
whose source is no larger is prepared completely and its regional geometry is
retained without eviction (a 256 MiB estimated-byte guard remains). A larger
document prepares a window of that much text around the viewport, two thirds in
the direction of travel, and evicts cached lines farthest from the viewport
first; lines outside the window are trimmed before the budget would evict lines
inside it, and preparation stops when the budget is nearly full. Within the
window, chunks are taken nearest-first: visible lines not yet retained whole,
three screens ahead, three behind, then the rest ahead and behind. A chunk holds
at most 128 hard lines and 16 KiB of text; a larger line is prepared alone up to
the 64 KiB long-line slice size. Beyond it, visible layout uses resumable
slices, except that a retained document, which the threshold bounds, prepares
such a line whole so its height is exact and visible layout then reuses those
rows without shaping. Larger documents leave giant lines to demand layout. An
edit to a giant line returns visible layout to slices until the line is
prepared again. The core answers "complete" for a retained document in
constant time.

Installation publishes exact heights to the height index, so the scroll extent
converges before the lines are visible. Heights above the visible snapshot move
its text; the core retranslates that snapshot under a new revision so the text
keeps its screen position, and frontends adopt it at their next interaction.
Speculative results carry a geometry generation: paint-only changes (syntax
colors, search highlights, height-only resize) neither cancel nor reject them,
and a line-local metric change rejects only results that overlap its lines.

Table rows from speculative jobs materialize every column and carry their
measured widths back to the view. A cached table row is reused only while the
table's widths are exact, unaffected by later edits and equal to those the row
was positioned with; rows with progressive (very long) cells stay demand-laid.

Syntax follows the same threshold: a retained Code document is highlighted
completely once visible text is covered and editing has paused for 250 ms, and a
retained Markdown document highlights every fenced block, visible blocks first,
publishing at most four offscreen completions per poll.

## macOS ownership and scheduling

Each `EVCoreViewSession` owns viewport pre-layout and table-width refinement
instances of [EVBackgroundLayout](../src/mac/Editor/Sources/EVBackgroundLayout.swift).
They share one serial utility queue across the app. Presentation refresh captures
eligible work after initial layout and viewport/dependency changes. Caret-only
movement does not restart idle work; there is no polling timer. When a window
completes after installing new geometry, the surface refreshes once input has
been quiet for 150 ms, and only if the published extent or revision changed.

Worker providers read unknown local image dimensions from file headers before
shaping, so speculative geometry is final. Render resources do not capture
bitmaps: drawing looks up the current bitmap by location while its geometry
matches, so a decode with already-known dimensions only repaints. Bitmaps are
admitted for the visible screen and one screen beyond each edge. Drawing uses a
copy resampled to the device size on a background queue, drawing the original
until it is ready (bounded to 16 entries and 48 MiB).

Capture and installation run on the main actor. Computation uses only an immutable
request and a separately captured Core Text provider. Worker response arenas are
independent; measurement identities and the native render registry are shared so
accepted glyph leases remain drawable by the view. Closing a view cancels its
request without waiting for computation, and retained work releases its request,
provider and uninstalled result when completion arrives.

Viewport installation is cache-only: it does not clear presentation exports,
move visible text, the caret or the scroll anchor, or request a redraw.
Table-width refinement retains its separate visible-update behavior. Each
scheduler chains one captured chunk at a time, recapturing from the current
viewport after completion until the core reports the window complete. Eight
consecutive discarded results stop speculation until the next viewport or
dependency change. Portable policy bounds the window and cache storage; the
frontend does not enlarge those budgets.

Edits, width, metrics, environment and horizontal changes, composition and detach
cancel obsolete work. Scrolling (including direction reversal and distant jumps)
and paint-only configuration changes keep in-flight work, which installation
validates against its geometry and the current window. Rapid input may outrun
preparation and synchronously shape visible content. It never waits for an
offscreen worker.

## Windows ownership and scheduling

Each `CoreView` owns viewport pre-layout and table-width refinement instances
of [BackgroundLayout](../src/win/Core/BackgroundLayout.cs). One worker slot is
shared across the app; capture runs at low dispatcher priority, computation on a
thread-pool worker with a provider captured on the UI thread, and installation at
normal priority so a finished chunk is published ahead of later input. The same
cancellation, discard and idle rules as macOS apply.

Worker providers read unknown local image dimensions from file headers, with the
same safety and admission checks as decoding, so speculative geometry is final;
the UI thread never touches the filesystem and lays out a placeholder instead.
Image knowledge lives in the view's provider, separately from bitmaps. A decoded
bitmap whose dimensions layout already used only repaints; a dimension change
advances the metrics generation and reports the location only when layout used
the previous dimensions. Drawing shows a bitmap only while it has the dimensions
its render resource was laid out with. Bitmaps are admitted for the visible
screen and one screen beyond each edge, and drawing uses a copy resampled to the
device size on a background thread (bounded to 16 entries and 48 MiB), drawing
the original until it is ready.

Shaping selects font families, including app-local designs, through retained
DirectWrite collections. Win2D resolves family strings and font-file URIs again
for every layout, which made font setup tens of milliseconds per shaping request.

## Presentation recovery

Both native frontends export current layout geometry independently of optional
paint, selection, paragraph decoration, and table-control exports. A failed
optional export uses default colors or omits that overlay for the frame, records
a bounded diagnostic, and retries on the next refresh. macOS also isolates text
and command-line exports. Recovery invalidates drawing caches when the available
presentation changes, even if the layout identity is unchanged.

Retained geometry is usable only when the core confirms its complete current
identity. Failed geometry export cannot make an older layout current. Viewport
requests read the current core viewport directly, so scrolling can recover while
an export is unavailable. Optional export failures preserve current hit-testing;
pointer placement waits for exact geometry if geometry itself is unavailable.
No recovery path replays an input or changes source.

## Validation

[Native Mac tests](../src/mac/Editor/Tests/EVBackgroundLayoutTests.swift) cover
cancellation and discard of stale work after edits, resize and metrics changes,
visible text stability, release after close, and a small document with tables and
a figure that, once prepared, scrolls end to end without foreground shaping or
any scroll-extent change. [Image resource tests](../src/mac/CoreTextProvider/Tests/CoreTextImageResourcesTests.swift)
cover worker dimension probes, repaint-only bitmap arrival and resampled bitmaps.
Core policy tests live beside [the policy](../src/core/coordinator/prelayout.rs).

The opt-in [interactive benchmark](../src/mac/Editor/Tests/EVPrelayoutBenchmarkTests.swift)
drives real wheel events and 2× drawing at 60 Hz with run-loop servicing between
frames, for 95 KB Markdown and Rust and a 1 MB Markdown document, scrolling
immediately, after preparation, and with preparation disabled. It reports input
latency split into core and drawing, main-thread busy spans attributable to
background work, main-thread shaping calls, scroll-extent changes, worker time
and typing latency. Run it optimized: the default native-test profile is not
representative.

```sh
scripts/test-mac.sh --filter EVBackgroundLayoutTests
scripts/test-mac.sh
```

The wrapper also rebuilds the app. The [performance guide](performance.md)
describes broader measurements and their limits. Glyph registry estimates and
fixture checks are not complete native memory accounting or physical display
latency measurements.

[macOS presentation tests](../src/mac/Editor/Tests/EVPresentationCacheIntegrationTests.swift)
inject optional export failures and check scrolling, pointer placement, source
preservation, identity validation, and recovery. The corresponding Windows
[presentation tests](../src/win/Diagnostics/PresentationTests.cs) check optional
export fallback, pointer placement, and recovered drawing-cache equivalence.

Windows references remain the [scheduler](../src/win/Core/BackgroundLayout.cs),
[regression tests](../src/win/Diagnostics/BackgroundLayoutTests.cs), the image
resource checks in [image interaction tests](../src/win/Diagnostics/ImageInteractionTests.cs)
(`BackgroundImageResources`), and the [development guide](../src/win/README.md).
`scripts/test-win.ps1 -Optimized -ProfileDocument <file> -ProfileScenario preview`
measures Markdown Preview switches, including per-phase shaping and syntax-wait
CPU time. Both frontends use the same
[C ABI](../include/viem_core.h): prepare captures a request, compute returns a
candidate, install consumes that candidate even on stale rejection, and release
retires requests or uninstalled candidates exactly once.

# Native background layout

macOS and Windows connect the [portable speculative-layout policy](../src/core/coordinator/prelayout.rs)
to native workers. The core selects bounded gaps near the current viewport and
validates every dependency when a result is installed. Both frontends keep the
existing cache budgets and allow foreground input to proceed while workers run.

## macOS ownership and scheduling

Each `EVCoreViewSession` owns viewport pre-layout and table-width refinement
instances of [EVBackgroundLayout](../src/mac/Editor/Sources/EVBackgroundLayout.swift).
They share one serial utility queue across the app. Presentation refresh captures
eligible work after initial layout and viewport/dependency changes. Caret-only
movement does not restart idle work; there is no polling timer.

Capture and installation run on the main actor. Computation uses only an immutable
request and a separately captured Core Text provider. Worker response arenas are
independent; measurement identities and the native render registry are shared so
accepted glyph leases remain drawable by the view. Closing a view cancels its
request without waiting for computation, and retained work releases its request,
provider and uninstalled result when completion arrives.

Viewport installation is cache-only: it does not clear presentation exports,
change visible layout identity, move the caret or viewport, or request a redraw.
Table-width refinement retains its separate visible-update behavior. Each
scheduler chains one captured chunk at a time, recapturing from the current
viewport after completion. Viewport work permits at most 32 captures per viewport
or dependency change, including stale retries. Portable policy bounds the region
and cache storage; the frontend does not enlarge those budgets.

Edits, resize, metrics/configuration changes, direction reversal, distant jumps,
composition and detach cancel obsolete work. Nearby movement in the same direction
can retain useful work; installation still validates its complete identity and
current eligibility. Rapid input may outrun preparation and synchronously shape
visible content. It never waits for an offscreen worker.

## Validation

[Native Mac tests](../src/mac/Editor/Tests/EVBackgroundLayoutTests.swift) cover
worker computation, idle scheduling, three prepared pages without foreground
shaping, unchanged visible layout/caret/scroll/export state, foreground-versus-worker
Core Text pixels, stale work after edits/resize/metrics changes, and release after
close. Large-document fixtures check that unrelated suffix growth does not increase
idle glyph retention. Ordinary and uninterrupted rapid paging compare the same
bytes with scheduling enabled and disabled; input timings exclude idle waits.

```sh
scripts/test-mac.sh --filter EVBackgroundLayoutTests
scripts/test-mac.sh
```

The wrapper also rebuilds the app. The [performance guide](performance.md)
describes broader measurements and their limits. Glyph registry estimates and
fixture checks are not complete native memory accounting or physical display
latency measurements.

Windows references remain the [scheduler](../src/win/Core/BackgroundLayout.cs),
[regression tests](../src/win/Diagnostics/BackgroundLayoutTests.cs), and
[development guide](../src/win/README.md). Both frontends use the same
[C ABI](../include/viem_core.h): prepare captures a request, compute returns a
candidate, install consumes that candidate even on stale rejection, and release
retires requests or uninstalled candidates exactly once.

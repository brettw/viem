# Windows background Markdown layout

Windows now connects the core's immutable regional layout jobs to an independent
worker. After the initial viewport is available, low-priority dispatcher work
captures small jobs and installs completed geometry. Shaping and wrapping happen
on the worker; it never calls the mutable core or UI. Normal input still prepares
newly exposed text synchronously when scrolling outruns the worker or jumps to a
region that has not been prepared.

The target extends beyond existing viewport overscan by three times the observed
hard lines on screen, capped at 128 lines and half the cache's line budget. Each
job contains at most 32 hard lines and 16 KiB of text. (The initial measurements
below used the original eight-line chunks; see the
[startup/paging follow-up](windows-startup-paging-performance.md).) Individual paragraphs
larger than that retain the existing bounded on-demand layout path. The worker
starts in the forward direction and follows subsequent scrolling. There is one
pending job per view, one computing worker for the application, and at most 32
chunks per viewport generation. No timer runs while idle. Caret-only movement
does not restart work.

The worker owns its measurement surface and response storage. Native glyph data
is immutable after construction and retained by explicit thread-safe leases;
the UI shaper and worker share resource identities, not mutable shaping state.
Win2D supports calls from any thread and handles its own synchronization; no
application lock spans shaping or drawing. See Microsoft's
[Win2D threading documentation](https://microsoft.github.io/Win2D/WinUI3/html/M_Microsoft_Graphics_Canvas_CanvasDevice_Lock.htm).

Edits, resize, font/device changes, reversing direction, distant scroll jumps,
and closing a view cancel obsolete requests. Installation checks revision,
configuration, metrics, provider identity, cancellation and the current scroll
band. Cache-only installation leaves the visible snapshot and scroll origin
untouched, including its layout revision. Exact heights enter the height index
when a demanded viewport incorporates the cached geometry.
Visible layout and cache-only jobs have independent current-job tokens, so a
foreground refresh does not cancel a still-useful background chunk. See the
[scroll cache follow-up](windows-scroll-performance.md) for the subsequent fixes
and measurements; the measurements below describe the initial implementation.

Geometry remains limited to 2,048 hard lines, 8,192 rows or 32 MiB of estimated
geometry, whichever is reached first. The worker retains no separate shaping
cache. Native glyph objects and the in-flight chunk add bounded overhead outside
those geometry estimates; the estimates are not total process RSS.

## Measurement

The optimized benchmark can compare identical builds and fixtures with only
background work toggled. It waits for the initial band, then performs 75 Page
Downs and 25 Page Ups. Reports distinguish foreground and background shaping,
capture/install time, input/drawing time and peak live glyph resources.

```powershell
.\scripts\test-win.ps1 -Optimized -ProfileDocument AGENTS.md -ProfileScenario page -ProfileIntervalMs 100
.\scripts\test-win.ps1 -NoBuild -Optimized -ProfileDocument AGENTS.md -ProfileScenario page -ProfileIntervalMs 100 -DisablePrelayout
```

Use `-ProfileIntervalMs 16` to stress scrolling faster than ordinary reading.
Run comparisons after compilation completes, using the same document bytes,
viewport and build. These measure synchronous input and drawing work, not native
key dispatch or compositor latency.

Measured on Windows x64 on 2026-09-19 at 144 DPI, with a 704.667 × 422.667
viewport and identical `AGENTS.md` bytes (SHA-256
`2533783bf6c9567b0dc1e0b4054ee1f38f9becbd3f7ed583757fd40cdd4994e3`).
Both runs use optimized C#/release Rust, with no concurrent compilation.

| Paging interval | Metric | Disabled | Background |
| --- | --- | ---: | ---: |
| 100 ms | Input plus draw, mean | 45.75 ms | 23.00 ms |
| 100 ms | Input plus draw, p95 | 112.30 ms | 35.70 ms |
| 100 ms | Foreground characters shaped | 108,333 | 0 |
| 100 ms | Peak live glyph resources | 117,316 | 133,549 |
| 16 ms | Input plus draw, mean | 39.66 ms | 30.28 ms |
| 16 ms | Input plus draw, p95 | 108.67 ms | 75.62 ms |
| 16 ms | Foreground characters shaped | 108,333 | 91,394 |
| 16 ms | Peak live glyph resources | 117,316 | 127,229 |

At 100 ms, UI-side capture averaged 0.093 ms and installation 0.377 ms;
their maximums were 0.443 ms and 1.182 ms. At 16 ms, scrolling outran the worker
and 44 of 62 speculative jobs were discarded. That run shaped 113,627 characters
in the background as well as its foreground work, so speculation trades some
additional CPU for reduced input latency. The 100 ms run shaped 146,498
background characters, including work ahead of the final viewport and cancelled
work. Cache budgets were identical. Full reports are in
[windows-background-layout.json](windows-background-layout.json).

## Regression coverage

Portable tests cover bounded capture in 10,000-paragraph Markdown, reuse during
paging, source preservation, cache limits, oversized paragraphs, cancellation
and stale results after edits/resize/metrics/direction changes. C ABI tests check
pointer validation, ownership, single computation, cancellation and worker
computation while the mutable core is checked out elsewhere.

Native tests verify worker thread identity, an idle scheduler, unchanged current
presentation, prepared-page foreground shaping, identical styled/bidi/emoji
pixels, stale work rejection, and native resource release after closing a view
with queued or active work.

Validation passed: 184 portable layout/coordinator unit tests, the additional
C ABI worker/ownership test, 12 layout/cache integration tests, and 314 Windows
integration checks. The shipping Release Windows build was rebuilt. Native
macOS tests were not run on this Windows host; the scheduler added here is for
Windows, with its planning and installation policy in the portable core.

## TODO(macOS): Connect the native scheduler

The Mac frontend has not been connected to this feature. Implement the scheduling
mechanism in `src/mac`; reuse the portable policy rather than duplicating its
region selection. Reference `src/win/Core/BackgroundLayout.cs`,
`src/core/coordinator/prelayout.rs` and `include/viem_core.h`.

- Give `EVCoreViewSession` in `src/mac/Editor/Sources/EVCoreDocument.swift`
  ownership of pending work. Trigger it after initial layout and relevant
  viewport/dependency changes, including those observed by
  `EVEditorSurfaceController.refreshPresentation`. Skip unavailable layout and
  composition; do not restart for caret-only motion or poll while idle.
- On the main actor, call `viem_core_view_prepare_prelayout` with direction
  `+1` or `-1`. Zero means there is no eligible work. Off the main actor, call
  `viem_layout_work_compute` using only the captured request and worker provider.
  Return to the main actor for `viem_core_view_install_prelayout`, which consumes
  the result, checks current dependencies and populates only the cache. Do not
  clear presentation exports, move the viewport or request a redraw just because
  background work completed.
- Extend `CoreTextMeasurementProvider` with independent worker response-arena
  ownership while sharing compatible measurement identity, metrics generation,
  render owner and `CoreTextRenderRegistry` leases. Its `AnyWorker` declaration
  allows off-main shaping; it does not by itself make concurrent UI/worker use
  of callback response pointers safe. A new provider's separate registry is also
  insufficient, even if given the same environment ID. Audit font resolution,
  generation invalidation and lease release; keep actual drawing on the main
  thread under the existing render policy. Never hold a lock across shaping or
  make UI input wait for the worker.
- Preserve the existing bounds: about three screens beyond overscan, at most
  128 lines in the band, eight lines/16 KiB per chunk, one pending job per view,
  one computing worker across the app, and at most 32 chunks per viewport
  generation. Keep cache budgets unchanged. Chain another chunk only after
  completion, preparing from the current viewport instead of retaining a queue.
- Cancel with `viem_layout_work_cancel` on edits, resize/zoom/font changes,
  reversal, distant jumps, detach and close. Release requests and uninstalled
  results with `viem_layout_work_release` exactly once. Keep callback contexts
  and leases alive through in-flight work; integrate with `detach`, `deinit`
  and `provider.retireResources()` without blocking the main actor.
- Add Mac tests corresponding to `src/win/Diagnostics/BackgroundLayoutTests.cs`:
  worker thread use, three prepared pages with no foreground shaping, stable
  visible layout/caret/scroll position, Core Text rendering equivalence, idle
  scheduling, stale-result rejection and resource release during close. Include
  large-document/cache-bound tests and compare `AGENTS.md` WYSIWYG paging with
  the feature enabled/disabled at ordinary and rapid input rates. Run the native
  Mac suite and build the Mac app before marking this TODO complete.

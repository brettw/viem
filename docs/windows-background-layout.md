# Background layout follow-up

Windows connects the portable speculative-layout policy to a native worker;
macOS still needs that connection. Current references are
[portable policy](../src/core/coordinator/prelayout.rs),
[Windows scheduler](../src/win/Core/BackgroundLayout.cs),
[Windows regression tests](../src/win/Diagnostics/BackgroundLayoutTests.cs), and
[the C ABI](../include/viem_core.h). Use their current bounds and ownership
contracts rather than reproducing a second policy in the frontend.

## TODO(macOS): Connect the native scheduler

The ownership point is `EVCoreViewSession` in
[EVCoreDocument.swift](../src/mac/Editor/Sources/EVCoreDocument.swift).
`CoreTextMeasurementProvider` permits worker shaping, but its callback response
storage is not a safe shared UI/worker response lifetime merely because the
provider is sendable. A second provider with an unrelated render registry is
also insufficient, even if given the same measurement identity.

- Give each session ownership of pending work. Trigger after initial layout and
  relevant viewport/dependency changes, including
  `EVEditorSurfaceController.refreshPresentation`. Skip composition/unavailable
  layout; caret-only motion must not restart work and idle must not poll.
- On the main actor, capture with `viem_core_view_prepare_prelayout` using travel
  direction `+1` or `-1`; a zero handle means no eligible work. Compute off the
  main actor with `viem_layout_work_compute`, using only the captured immutable
  request and worker provider. Return to the main actor to install with
  `viem_core_view_install_prelayout`.
- Installation is cache-only: it must not clear presentation exports, change
  the visible layout revision, move the viewport or request a redraw. Keep
  visible-layout and speculative-job lifetimes independent. Capture can run
  at low priority; bounded completed-result installation must not starve behind
  later input, forcing already-computed geometry to be shaped again.
- Give worker callbacks independent response arenas with compatible shared
  measurement identity, metrics generation, render owner and glyph leases.
  Audit font resolution, invalidation and final lease release. Draw on the main
  thread, never hold a lock across shaping, and never make UI input wait for
  the worker.
- Reuse core selection/bounds and unchanged cache budgets. Keep one pending
  chunk per view, one computing worker across the app and bounded retries per
  viewport generation. Chain another chunk only after completion, recapturing
  the nearest gap from the current viewport rather than retaining a queue.
- Cancel on edits, resize/zoom/font changes, direction reversal, distant jumps,
  detach and close. Use `viem_layout_work_cancel`; release requests and
  uninstalled results with `viem_layout_work_release` exactly once. Installation
  consumes its result, including stale rejection. Keep callback contexts and
  leases alive through in-flight work and integrate retirement/deinitialization
  without blocking the main actor.

Before closing this task, add native tests for worker-thread computation, idle
scheduling, three prepared pages without foreground shaping, unchanged visible
layout/caret/scroll position, equivalent Core Text pixels, stale-result rejection
and resource release with queued or active work on close. Include bounded
large-document/cache tests and ordinary/rapid paging comparisons with the same
fixture bytes. Run the native Mac suite and build the app. The
[performance guide](performance.md) describes the measurement tools and limits;
old Windows timing results are not acceptance evidence for a Mac implementation.

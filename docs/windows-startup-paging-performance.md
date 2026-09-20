# Windows initial display and repeated paging

Measured on Windows x64, Intel Xeon E5-1650 v3 at 3.50 GHz, on 2026-09-19.
This follows the earlier
[paging](windows-page-performance.md), [background layout](windows-background-layout.md)
and [scroll cache](windows-scroll-performance.md) investigations.

The comparison uses one frozen copy of `AGENTS.md`, 416,312 bytes, SHA-256
`99de8cea9e3eae2bcc06c07c3d73227a889920052c4a7e61f6d725e6706e2fe7`.
All launches use isolated preferences. Measurements run after compilation and
without another benchmark running. The larger-window profile copies only the
saved window dimensions, not the user's other settings.

## Changes

- View creation receives the final margins before its first layout. Reapplying
  identical margins does not reflow a current snapshot. Initial prose layout
  starts with a small paragraph band and grows only until the viewport is covered.
- Windows starts the horizontal scrollbar collapsed. Previously, hiding it after
  the first draw changed the canvas height and caused another layout.
- Command-line file launch skips the temporary empty editor. Parsing the newly
  opened document runs on a worker before the view is attached.
- Font resolution shares one read-only system font index, prepared alongside
  shell loading, instead of rebuilding it for each family. Picker lists remain
  lazy; no whole-system font-face enumeration was added.
- Page refills convert missing visual rows to paragraphs using observed wrapping
  density. Exact coverage checks still decide when a command can proceed.
- Immutable snapshot copies share row geometry. Command checkpoints no longer
  copy every glyph and caret; changes detach storage and preserve older snapshots.
- Windows combines adjacent compatible glyph drawing calls. Whole LTR clusters
  use positions already captured from the native glyph run, while bidi and split
  runs retain DirectWrite's character-region query. Repeated identical glyph ink
  queries share a small cache limited to one shaping request.
- Provider responses allocate caret arrays once per fragment and encode each
  fallback font name once, instead of allocating both for every character.
  Captured glyph parts use value storage and avoid temporary cluster-map arrays.
- Background chunks batch up to 32 short/empty hard lines under the unchanged
  16 KiB limit. Capturing new work stays at low dispatcher priority; completed
  cache entries install at normal priority so later input can use finished work.

The existing geometry cache remains bounded by 2,048 hard lines, 8,192 rows and
32 MiB of estimated geometry, whichever is reached first. Shaping retains its
separate 32 MiB budget. Scrolling does not clear these caches, but a long trip can
evict the oldest pages. These budgets are not total process memory limits.

## Method

Startup uses the shipping Release executable and a real file argument. It records
process start to the first document draw, plus activation to that draw. For one
second afterward, it verifies that canvas size, viewport top, first-row baseline
and canvas position remain unchanged. It does not measure compositor presentation
or cold-boot disk latency.

Paging uses optimized C# with diagnostics and Release Rust. The `roundtrip`
scenario waits for the initial pre-layout band, then sends 30 Page Downs, 30 Page
Ups, and repeats both directions. Each step measures the production core command,
presentation refresh and native offscreen drawing. The yield between steps is
reported separately and excluded from CPU timings. Native key dispatch and
compositor latency are not included. Foreground shaping counts distinguish
layout reuse from cache misses.

```powershell
.\scripts\test-win-startup.ps1 -NoBuild -Runs 3 -ProfileDocument AGENTS.md
.\scripts\test-win.ps1 -Optimized -ProfileDocument AGENTS.md -ProfileScenario roundtrip -ProfileIntervalMs 16
```

Both scripts accept `-ConfigFile` to copy a window configuration into their
isolated profile. Use identical fixture bytes and viewport dimensions for
before/after comparisons. Timings are observations, not brittle test thresholds.

## Results

The matching before/after paging runs use a 704.667 × 422.667 DIP canvas at
144 DPI, with a 16 ms yield between commands. Values include input and drawing.

| Operation | Before mean / p95 (ms) | After mean / p95 (ms) |
| --- | ---: | ---: |
| First 30 Page Downs | 22.23 / 61.08 | 14.15 / 38.90 |
| First 30 Page Ups | 21.88 / 38.92 | 9.75 / 27.11 |
| Repeated Page Downs | 19.48 / 44.35 | 15.28 / 47.74 |
| Repeated Page Ups | 17.28 / 31.60 | 10.29 / 26.78 |

All three return/revisit phases shaped **zero** characters on the foreground
thread, both before and after. The improvement comes from doing less work with
cached geometry and drawing. It is not evidence that the previous run discarded
all its layout cache. Tail times still vary; the repeated-down p95 did not improve.

Release process-to-first-text time was 2,407 ms on the first baseline launch and
1,960 ms on its repeat. Final launches measured 2,073 ms, then 1,362 and 1,472 ms.
Activation-to-first-text fell from 1,197–1,334 ms to 718–834 ms. Foreground text
shaped before the first frame fell from 8,983 to 2,028 characters in this window.
The old canvas changed height from 408.667 to 422.667 DIP after first draw; every
frame in each final launch retained its original size, scroll top, baseline and
canvas position during the one-second observation period.

The larger saved window gives a 1258.667 × 858 DIP canvas. Final Release startup
measured 1,614 and 1,434 ms and passed the same geometry-stability check.
Paging was also tested at that size:

| Large-window phase | Earlier iteration, 16 ms yield, mean / p95 | Final, 16 ms yield, mean / p95 | Final, 100 ms yield, mean / p95 |
| --- | ---: | ---: | ---: |
| First Page Downs | 97.88 / 202.42 | 81.26 / 150.10 | 38.36 / 92.05 |
| First Page Ups | 40.21 / 106.97 | 26.29 / 61.40 | 37.13 / 87.46 |
| Repeated Page Downs | 25.95 / 44.78 | 26.40 / 93.42 | 30.60 / 73.76 |
| Repeated Page Ups | 30.34 / 70.46 | 25.04 / 54.32 | 28.70 / 45.70 |

The earlier large-window run already included the startup, glyph-batching and
ink-query fixes; it is an intermediate comparison, not the original baseline.
These stress results are not uniformly faster. Rapid paging still outruns the
worker, and 30 large screens can exceed the geometry budget: the final first-up
phase reshaped 7,201 characters, while its first ten returning pages reshaped
none. With a 100 ms yield, first-down foreground shaping dropped from 113,965 to
2,544 characters. Background work, allocations and geometry assembly still cause
occasional longer turns even when no foreground shaping is required. This change
does not claim that every page completes within one display frame.

Full per-step results, shaping counts, stage timings and startup geometry are in
[windows-startup-paging-performance.json](windows-startup-paging-performance.json).
A preliminary run overlapped another benchmark and is excluded from these results.

## Validation

- 249 layout-related Rust unit tests, including large documents, cache reuse,
  invalidation, initial padding and immutable geometry sharing.
- 563 command unit tests and 77 C ABI, cache, job, block-quote and decoration
  integration tests; `cargo check --tests` also passed.
- 324 native Windows checks, including comparisons against DirectWrite cluster
  positions, glyph batching pixel equivalence, worker/foreground pixel equivalence,
  metrics invalidation and closing with pending work.
- Three small-window and two large-window startup stability runs.
- Optimized diagnostics and Release Windows builds; 142 generated C ABI functions
  verified. Rust and frontend binaries were rebuilt together for core ABI 6.

The native suite encountered its existing intermittent menu-focus assertion once
on the final run; an unchanged rerun passed. Native macOS tests were not run on
this Windows host. The shared layout changes apply there, while its background
scheduler remains covered by the existing macOS TODO.

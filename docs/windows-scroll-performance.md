# Windows wheel-scroll cache reuse

The initial background-layout benchmark exercised Page Down, which did not
expose redundant work in the absolute viewport-origin path used by the wheel,
touchpad and scrollbar. A 60-unit wheel tick rebuilt the visible layout even
when it remained entirely within installed overscan. That changed the layout
revision, repeated geometry exports and cancelled the pending background job.
The retained paragraph cache itself was not cleared, but useful prepared work
and native drawing commands were repeatedly discarded.

The shared core now validates the current snapshot's document, configuration,
measurement identity and metrics generation, then reuses it when it covers the
requested viewport. Only the origin and viewport anchor change. Requests outside
coverage, stale geometry and sparse horizontal layouts continue through the
existing atomic demand-layout path, including font-invalidation retries.

Visible layout and speculative cache fills now have separate current-job tokens.
Preparing or publishing another visible region no longer cancels a compatible
background chunk. Either job can finish first without superseding the other.
Installation still checks every dependency and retains only chunks within the
current materialized region and adjacent pre-layout band; edits, reflow and
abandoned regions still discard work. This also avoids repeated cancellation
during rapid Page Down, where the visible snapshot necessarily changes.

Windows retains two native command lists for text and backgrounds, covering the
viewport plus one screen in the direction of travel. Covered vertical scrolling
translates their replay instead of issuing every glyph call again. Recording
remains bounded; background layout caches keep their existing limits. Whitespace
markers are viewport-specific and still invalidate their drawing on scroll.
Theme, size/DPI, layout and relevant option changes also invalidate drawing.

The native regression test scrolls a large Markdown fixture containing styled
text, Arabic, emoji, headings, lists and quotes. It checks that small scrolls
preserve the layout revision, foreground shaping count and command-list count,
and compares translated drawing against a fresh recording pixel for pixel. A
source-view check verifies whitespace markers are refreshed. Portable tests
cover large-document cache retention, pending-job preservation, metrics
invalidation, distant scrolls, both job completion orders, replacement/token
ownership and exact within-paragraph positions across retries.

The wheel benchmark follows the production viewport-origin boundary, applying
75 downward 60-unit steps followed by 25 upward steps with a 16 ms yield. It
records synchronous input and drawing work; these times do not include input
dispatch or compositor latency.

```powershell
.\scripts\test-win.ps1 -Optimized -ProfileDocument AGENTS.md -ProfileScenario scroll -ProfileIntervalMs 16
```

## Measurements

Measured on Windows x64 on 2026-09-19, at 144 DPI and a
704.667 × 422.667 DIP viewport. Every run used the same saved AGENTS.md bytes
(SHA-256 `96f39cdd0c22953af1ec5250d4f7a498dcbb36c9fcd5249a841cb1582eb9ef56`),
optimized C#/release Rust, and no concurrent compilation. Background pre-layout
was enabled in every run. Complete reports are saved in
[windows-scroll-performance.json](windows-scroll-performance.json).

| 100 wheel steps, 16 ms yield | Before | After |
| --- | ---: | ---: |
| Input plus draw, mean | 18.29 ms | 8.36 ms |
| Input plus draw, median | 17.62 ms | 4.78 ms |
| Input plus draw, p95 | 32.09 ms | 32.38 ms |
| Core operation, mean | 4.54 ms | 0.66 ms |
| Native drawing-command rebuilds | 100 | 16 |
| Foreground characters shaped | 750 | 0 |
| Background characters shaped | 59,239 | 22,391 |
| Discarded background jobs | 37 of 56 | 7 of 31 |
| Peak live glyph resources | 28,442 | 33,336 |

The common scroll path is substantially cheaper, while occasional drawing-cache
refills still dominate the tail; this run does not show a p95 improvement.
Command recordings include an extra screen, increasing their bounded resource
retention. Existing geometry and shaping cache budgets are unchanged. Glyph
resource counts are object counts, not process-memory measurements.

Rapid paging (100 Page Down/Up steps, 16 ms yield) separately exercised the job
tracking change. Its comparison already includes the wheel/drawing fixes:
discarded jobs fell from 41 of 70 to 3 of 62; foreground shaping fell from 64,795
to 56,850 characters and background shaping from 122,453 to 106,393. Mean input
plus draw was 31.79 ms before and 30.96 ms after, but p95 was 87.83 ms before and
103.75 ms after. Fast paging can still outrun the worker, so reduced cancellation
does not establish a tail-latency improvement. Peak live glyph resources were
175,321 in the final rapid run.

With a 100 ms paging yield, the final build shaped 303 foreground characters
across 100 pages, installed 80 of 82 background jobs, and averaged 25.39 ms for
input plus draw (p95 49.91 ms). Native tests separately verify that three fully
prepared pages require no foreground shaping.

Validation: 245 layout-related unit tests, 50 viewport tests, five vertical-scroll
tests, 12 layout/cache integration tests and all 320 Windows integration checks
passed. The Release Windows app was rebuilt. Shared core snapshot reuse and job
tracking apply to both platforms; drawing-command reuse is Windows-specific and
the Mac background scheduler remains the documented TODO. Native Mac tests were
not run on this Windows host.

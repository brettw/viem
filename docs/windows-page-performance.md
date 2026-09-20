# Windows Markdown paging performance

Later startup and repeated Page Down/Up measurements are recorded in
[windows-startup-paging-performance.md](windows-startup-paging-performance.md).

Validated on Windows x64 on 2026-09-19. The fixture is `AGENTS.md` from
`37d6d993c1bc3b7a731fc94e754c5ef7234522d9`, opened in Markdown WYSIWYG mode.
Its SHA-256 is `47c78f4d44750f7b48657bfe2d97f79110cf3e171cade5caf248c21ab54e669f`.

## Causes and changes

The DirectWrite provider repeatedly fetched localized font-name dictionaries
and face metrics for each shaped cluster. Font names were read both to report
fallback and to detect emoji. These values are now read once per native glyph
run and shared by its clusters. The metadata follows the existing render leases;
there is no separate font cache or startup font scan.

The portable layout engine also retained wrapped hard-line geometry without
using it to satisfy later regional requests. Requests now capture immutable
cache hits only within their requested band. Computation reuses their rows and
rebinds caret revisions, while resolving paint for the new request. Document,
configuration, measurement environment, metrics generation, and render-resource
policy changes prevent reuse. Partial long-line and sparse horizontal results
are excluded.

The existing geometry limits remain 2,048 hard lines, 8,192 visual rows, and
32 MiB of estimated geometry, whichever limit is reached first. Shaping has a
separate 32 MiB payload budget and 1 MiB decoration budget. These estimates are
not total process RSS: native resources, the installed viewport and other
document state have separate lifetimes. The initial optimization added no background queue,
whole-document pre-layout, or increased cache budget. Previously unseen text
still used bounded synchronous layout on Windows' UI-confined provider. The
subsequent [background pre-layout change](windows-background-layout.md) moves
nearby speculative shaping to an independent worker without increasing these budgets.

## Optimized comparison

Both runs used a 704.667 by 422.667 layout-unit viewport at 144 DPI and identical
fixture bytes. The baseline was built from the original revision in an isolated
checkout with only the benchmark instrumentation added. The recorded baseline
and changed runs were taken after compilation finished.

| Synchronous work, milliseconds | Original | Changed |
| --- | ---: | ---: |
| Page input plus drawing, mean | 79.06 | 37.55 |
| Page input plus drawing, 95th percentile | 235.49 | 114.05 |
| Page input plus drawing, maximum | 283.21 | 159.68 |
| Core turn, mean | 67.74 | 24.58 |
| Shaping batch, mean | 112.26 | 34.87 |

Both runs shaped 108,333 characters in 53 batches and peaked at 117,316 live
glyph resources. The changed run read font metadata 1,153 times. The original
run's font-metadata counter was not instrumented. Final visible/overscan geometry
was identical in size: 82 rows, 6,097 clusters, and 46 hard lines.

The reduction removes avoidable work; cold-page shaping still produces some
long turns. This is not a claim that all layout runs off the UI thread.
The full measurements are in [windows-page-performance.json](windows-page-performance.json).

## Reproduction

```powershell
.\scripts\test-win.ps1 -Optimized -ProfileDocument AGENTS.md -ProfileScenario page
```

The optimized harness keeps diagnostics enabled while using release Rust and
optimized C#. It writes to `target/windows-profile`, uses isolated preferences,
warms the initial viewport, exits the pointer warm-up selection into Normal
mode, and executes 75 Page Downs followed by 25 Page Ups, with a 16 ms yield
between turns. Each step calls the production input/core boundary and records
the drawing into a native offscreen target.

Measurements cover synchronous command handling, presentation exports and
bitmap drawing. They do not measure native key dispatch, compositor latency,
or loading/parsing the document. Timings are observations rather than test
thresholds. Reports include viewport dimensions, shaping volume and peak live
render-resource count. Compare identical fixture bytes, viewport and build
settings, with no concurrent compilation.

## Regression coverage

- A 2,000-paragraph Markdown test proves cached pages skip both shaping and
  wrapping, and compares their exact row geometry with fresh computation.
- Dependency changes, including render-owner changes, force recomputation.
- Existing line, row and byte eviction tests keep retained memory bounded.
- Native tests page through large Markdown, revisit cached pages, invalidate
  font metrics, and verify source bytes remain unchanged.
- Existing native pixel comparisons cover styled text, bidi and emoji drawing.

Validation passed: 237 portable layout unit tests, 12 cache/job integration
tests, 36 additional viewport/format/atomicity integration tests, and 305
Windows integration checks. Both Debug and Release Windows builds succeeded.
Native macOS tests were not run on this Windows host.

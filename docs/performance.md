# Performance tools and remaining work

Keep measurement output in a temporary directory or `target/`, not in `docs/`.
Compare identical fixture bytes, profiles, build modes, view sizes and provider
settings. Record source/build identity and run without concurrent compilation.
An old benchmark result is not evidence of current performance.

## Large-file memory

The [allocator probe](../examples/large_file_memory.rs) exercises the real core
in fresh serial release processes. The [runner](../scripts/measure-large-file-memory.py)
owns two fixture sets: standard literal and Markdown opening/editing/views and
supplemental encodings, dense lines, giant words, search/save/history/reclamation.
The Markdown cases repeat dense rich blocks (headings, inline markup, lists,
quotes, tables and fences) in formatted and Source presentation, including a
switch between them. The checked-in [limits](../scripts/fixtures/large-file-memory-limits.json) are
regression inputs, not recorded results; do not relax them to make a change pass.

Rich projections must not keep per-scalar records. Parser input stores runs of
scalars that map onto their own encoded bytes, and provenance stores bounded
divisible runs plus explicit contributors; both expand exactly on query. Range
stores dereference to a cached flat slice for compatibility: `iter()`, indexing
or other slice methods on a whole store retain a full copy with that snapshot,
including in history, so editing paths use tree queries, `partition_point`,
`get_range` or a one-off `to_vec`. `Document::text()` likewise retains the
flattened text with the state that materialized it.

When contributor source starts are monotonic, source-boundary queries use source
extent aggregates on the existing formatted-order tree. Reordered contributors
retain a separate source-keyed index. Regional splices shift the shared-tree
aggregates lazily and verify source ordering across the replacement and both
seams; a failed ordering proof retains the verified reprojection fallback.

Markdown opening shares one block-syntax parse among quote, list, indented-code,
paragraph-flow and table recognition. The parser still consumes complete input;
sharing its result removes repeated construction without making opening
progressive.

Table metadata uses persistent range storage. Source and formatted offsets on
unchanged suffixes shift at the tree root; nested row stores share their records
and apply the same translation when queried. Regional splices rebuild only the
affected tables and preserve table, row and cell identities.
Identity reconciliation receives explicit old and new table index ranges, so
source shifts cannot hide the previous owner of an unchanged row or cell.
Full reconciliation reads rows through temporary vectors and preserves shared
paragraph attributes when their values are unchanged.

A Markdown presentation switch stores one shared pair of old/new extents per
change. Association and deletion-recovery queries derive their result from those
extents instead of retaining separate policy maps in both directions. Composition
materializes the existing compiled representation so anchor lookup never follows
an accumulating chain of transitions. This is still an extent map, rather than a
direct map through source provenance.

Structural Markdown edits can reproject a bounded band that includes unchanged,
independent paragraphs around the edited owners. The old standalone parse must
match the current projection; edge paragraphs must retain their full semantics,
and outer separators must remain explicit paragraph boundaries before and after
the patches. Lists, quotes, passive HTML, heading-start splits and Source paragraph
deletions use this guarded path when those proofs succeed. Global references,
oversized owners and unprovable boundaries retain the verified full fallback.

Word navigation queries the immutable hard-line/text snapshot and batches ASCII
runs while streaming Unicode graphemes across tree leaves, without constructing
a document-wide grapheme array. Semantic object and hard-break boundaries remain
authoritative. Rich-paste whole-range
checks use the text tree's length. Work statistics separately count compatibility
text materialization, including read-only commands, so a warm cache cannot hide
an unbounded first query in locality tests.

The shape cache stores adjacent clusters with identical geometry and relative
caret stops as runs when that representation is smaller. Each original native
render handle retains its own ownership. Cache hits expand the exact public
cluster records; this reduces retained cache storage without changing the
provider ABI or visible layout representation. Validate edits, resizing, metrics
changes, resource lifetime and eviction against freshly shaped geometry.

```sh
cargo build --release --offline --example large_file_memory
python3 scripts/measure-large-file-memory.py --output /tmp/viem-memory.json --limits scripts/fixtures/large-file-memory-limits.json
python3 scripts/measure-large-file-memory.py --supplemental --output /tmp/viem-memory-extra.json --limits scripts/fixtures/large-file-memory-limits.json
```

`--limits` is optional; omitting it collects observations without enforcing
ceilings. `--binary` selects a preserved executable. The runner records source,
probe and binary hashes, checks exit status, and with limits checks required
phases, useful undo and live/cumulative-peak requested heap. The ceilings were
established for arm64 release builds; a different environment needs an explicit
comparison, not silent threshold changes.

Requested Rust allocation sizes exclude allocator overhead and native allocations.
The probe also reports macOS process RSS/footprint where available. A cumulative
peak is a high-water mark, not a per-operation allocation total; retained RSS
after dropping a document is not by itself evidence of a leak. Fixtures use mock
shaping and no syntax providers or native UI. They do not establish whole-app
memory bounds or a universal bytes-per-source-byte multiplier. See the
[fuzzing guide](fuzzing.md) for the separate retained-history ledger probe.

## Editing and native presentation

The [core latency probe](../examples/editor_perf.rs) opens, scrolls, types,
undoes and attaches multiple views with mock shaping:

```sh
cargo build --release --offline --example editor_perf
./target/release/examples/editor_perf markdown 4194304 /tmp/viem-editor.json
```

The available scenarios are defined by the probe. Its `html` fixture opens as
literal Code, not a removed HTML editing format. Results are core operation
samples, not native frame/compositor latency. Thread CPU timing is Linux-specific;
`thread_cpu_total_ms` is `null` on other platforms or when a complete set of CPU
samples is unavailable. Wall-time measurements remain available on every platform.
The Markdown fixture uses the current `AGENTS.md`, so preserve identical input
bytes when comparing builds. Syntax-provider performance commands and supported
profiles live beside the [Vim](../src/core/document/syntax/vim/PROFILE.md) and
[Tree-sitter](../src/core/document/syntax/treesitter/PROFILE.md) implementations.

The complete Code pipeline also has a blocked-worker/bounded-interaction gate
in the normal Rust suite, including million-line and 100 MiB mixed-encoding
fixtures. To collect its timings, run it alone in release mode:

```sh
VIEM_CODE_PIPELINE_REPORT=target/code-pipeline-benchmark.json cargo test --release --offline --lib pinned_code_pipeline_performance -- --nocapture --test-threads=1
```

The optional report uses mock measurement, so it does not establish native draw
latency or progressive cold loading.

macOS presentation/cache regression checks use the native test wrapper:

```sh
scripts/test-mac.sh --filter 'EVPresentationCacheIntegrationTests|EVPointerDrawingPerformanceTests|EVBackgroundLayoutTests'
```

Measure macOS startup through the requested document's first editor draw in
fresh release processes:

```sh
scripts/build-mac-app.sh release
python3 scripts/measure-mac-startup.py --document AGENTS.md --profile-directory ~/.viem --output target/mac-startup
```

The runner copies the source and profile into temporary directories and isolates
the single-instance endpoint and recovery files. It never closes an existing
editor. Omit `--profile-directory` for default settings; use `--executable` to
compare a preserved app bundle. JSON reports include binary/source hashes,
process-to-first-draw timing (including loader startup), main-to-first-draw
timing, and opt-in native milestones. Milestones overlap; do not add their
durations. This measures the draw callback, not physical display latency or a
cold boot. Report the first launch of newly built output separately from
repeated launches, and run without concurrent compilation.

For stack investigation, use a separate output directory with `--runs 1
--sample --sample-delay 0.2`. Sampling can substantially delay dynamic library
loading; sampled runs are not timing comparisons. The app retains no startup
trace unless `VIEM_STARTUP_REPORT` is set.

For Windows input, drawing, pre-layout and startup measurements:

```powershell
.\scripts\test-win.ps1 -NativeAot -ProfileDocument docs/markdown_demo.md -ProfileScenario page -ProfileIntervalMs 100
.\scripts\test-win.ps1 -NoBuild -NativeAot -ProfileDocument docs/markdown_demo.md -ProfileScenario page -ProfileIntervalMs 100 -DisablePrelayout
.\scripts\test-win.ps1 -NoBuild -NativeAot -ProfileDocument docs/markdown_demo.md -ProfileScenario scroll -ProfileIntervalMs 16
.\scripts\test-win-startup.ps1 -ProfileDocument docs/markdown_demo.md
```

Windows scripts save reports and isolated profiles under `target/windows-validation`.
The `-NativeAot` mode exercises the native publishing path used by Release;
`-Optimized` alone measures the managed diagnostic build.
Use a larger fixed fixture for large-document claims. Input profiles measure
synchronous input/drawing work; startup ends at the first editor draw callback.
Neither measures physical display latency or a cold boot. Separate first
launches of newly built output from repeated launches, and do not add overlapping
startup trace scopes. See the [Windows development guide](../src/win/README.md)
for other scenarios and build/profile options.

## Open work

- **Progressive first display:** document construction still decodes and builds
  the initial state before returning. Compact storage does not make opening
  progressive. Any incremental/disk-backed design must preserve immutable
  snapshots and stable encoding interpretation after a late invalid byte.
- **Remaining flat-text consumers:** sentence/pair scans, case replacement and
  Visual Block range resolution now use snapshot queries. Structural formatting,
  specialized paragraph keys, link-context typing, layout-dependent command
  motion and replay still contain `Document::text()` calls. Full-reprojection
  identity reconciliation also materializes old and new snapshots; traversing
  the complete fallback does not require retaining compatibility copies. These
  paths need separate measurements and local verification before every editing
  command can claim a bounded working set.
- **Cold giant-line work:** bounded retained geometry does not bound first-pass
  metric discovery or a distant cold jump. Current long-line tests include
  linear source scanning with bounded shape fragments. Measure adversarial
  graphemes, bidi transitions and dense style changes as well as plain ASCII.
- **Complete native accounting:** the core allocator probe omits native shapers,
  glyph storage and syntax trees. Measure component ownership and whole-app
  live/peak memory through opening, disjoint edits, history branches, multiple
  views, background jobs and release. Dense decoding exceptions need separate
  coverage from ordinary literal runs.
- **View switches:** changed extents still scale with hidden syntax. Direct
  provenance mapping must preserve deleted-content recovery, discontiguous range
  identity and composed-map behavior before it can replace these extents.
- **Structural fallback coverage:** guarded bands do not make every Markdown
  structural edit regional. Measure large contiguous containers, global reference
  dependencies, table-adjacent structural edits and rejected boundary proofs.
  Whitespace-only guards cannot prove separator pairing, and emptied structural
  owners currently use the full fallback to replace their caret contributors.
  The ATX independence proof can still decode a long hidden prefix before
  rejecting an oversized band.
- **Table row searches:** table coordinate lookup descends the tree directly,
  and regional identity reconciliation visits only affected old and new tables.
  Some generic row predicates still use ordinal binary search, adding a second
  logarithmic factor within the affected table.
- **Lazy table-cache accounting:** borrowed table and row lookups can grow their
  caches after the immutable allocation ledger first records them. Accounting
  that growth needs a bounded update mechanism; rescanning every populated
  cache on each edit would make unrelated queries part of editing cost.
- **Opening scratch storage:** pulldown-cmark still parses complete input and
  projections still retain whole-document construction intermediates. A streaming
  design must preserve global Markdown dependencies and exact source mappings.
- **Provenance indexing:** source extents currently add metadata to generic
  range-tree nodes, including stores that do not use reverse lookup. A
  provenance-specific summary could avoid that cost. Measure lookup arithmetic
  and cache locality before changing the representation; reordered projections
  still need their source-keyed index. Run sizing remains unchanged; alternatives
  trade tree storage against lookup work within a run.
- **Layout geometry:** cached geometry can share runs, but visible layout still
  expands per-cluster caret records, and native glyph payloads remain separately
  owned. Further compression needs native frame and scroll measurements as well
  as allocation accounting.

Unicode line-breaking data, offline regeneration and the intentional numeric
conformance tailoring difference are documented with the
[pinned data](../data/unicode/15.0.0/README.md). Preserve those tests when changing
layout performance; there is no separate historical validation report to update.

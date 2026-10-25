# Performance tools and remaining work

Keep measurement output in a temporary directory or `target/`, not in `docs/`.
Compare identical fixture bytes, profiles, build modes, view sizes and provider
settings. Record source/build identity and run without concurrent compilation.
An old benchmark result is not evidence of current performance.

## Large-file memory

The [allocator probe](../examples/large_file_memory.rs) exercises the real core
in fresh serial release processes. The [runner](../scripts/measure-large-file-memory.py)
owns standard literal and Markdown opening/editing/views, supplemental encodings,
dense lines, giant words, search/save/history/reclamation, and cold rich-paste
fixture sets.
The Markdown cases repeat dense rich blocks (headings, inline markup, lists,
quotes, tables and fences) in Markdown Preview and as Code (Preview off),
including a switch between them. The checked-in [limits](../scripts/fixtures/large-file-memory-limits.json) are
regression inputs, not recorded results; do not relax them to make a change pass.

Rich projections must not keep per-scalar records. Parser input stores runs of
scalars that map onto their own encoded bytes, and provenance stores bounded
divisible runs plus explicit contributors; both expand exactly on query. Range
stores have no implicit slice view. Their explicit `compatibility_slice`, and
the projection's `blocks()` and `style_spans()` built on it, retain a full copy
with that snapshot, including in history, so editing paths use tree queries,
`items`, `partition_point`, `get_range` or a one-off `to_vec`.
`Document::text()` likewise retains the flattened text with the state that
materialized it. Work statistics count each new compatibility copy of records
and text, so locality tests can assert that an edit made none.

Strikethrough uses the same sparse direct-property spans as underline, so its
appearance no longer depends on an editable named style. An isolated arm64
release allocator comparison (2026-10-24, Rust 1.98.1) measured 2,080,000 bytes
of property payloads for 10,000 spans, versus 130,000 bytes for their former
style-name strings: 208 versus 13 bytes per span, excluding identical range
storage and allocator overhead. The additional 1.86 MiB per 10,000 spans is an
accepted cost of sharing the direct-formatting representation; it scales with
strikethrough ranges, not ordinary characters. Standard, supplemental and
rich-paste memory checks remained within their existing ceilings.

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
List grammar discovery uses block-tree summaries to skip unrelated empty
paragraph runs. Complete list owners remain dependencies when an edit changes
their structure or spacing. Empty continuation carets supply the parser's list
context, and emptying an item beside prose adds only its required source separator.

Word navigation queries the immutable hard-line/text snapshot and batches ASCII
runs while streaming Unicode graphemes across tree leaves, without constructing
a document-wide grapheme array. Semantic object and hard-break boundaries remain
authoritative. Rich-paste whole-range checks use the text tree's length.
Multiline payload preparation splices the text tree and verifies both text and
hard-break intent in a proven parser region. Prepared candidates carry the old
and new regions so clipboard verification checks pasted styles and every retained
style inside those regions while sharing the untouched remainder. A mismatch in
an independently parsed region that covers the edit rejects that candidate
without reparsing unrelated source. If an edited edge row becomes blank, the
capture includes neighboring paragraph guards so separator endings pair in their
original context. Candidates without a locality proof retain
complete verification, including payloads that change break metadata without
changing visible text. Literal carriage-return payloads also retain complete
verification because DOS interpretation can combine them with an adjacent
retained line feed outside a regional line body. Single-line payloads reaching
this fallback have already failed the ordinary local path and keep complete
verification. List paragraph insertion finds its owning item through the block
tree's navigation summaries instead of enumerating the document's list structure.

Work statistics count clipboard style blocks and paragraph text inspected, as
well as source decoding and projection work. They separately count compatibility
text materialization, including read-only commands, so a warm cache cannot hide
an unbounded first query in locality tests.

The shape cache stores adjacent clusters with identical geometry and relative
caret stops as runs when that representation is smaller. Each original native
render handle retains its own ownership. Cache hits expand the exact public
cluster records; this reduces retained cache storage without changing the
provider ABI or visible layout representation. Validate edits, resizing, metrics
changes, resource lifetime and eviction against freshly shaped geometry.

The regional line cache and the installed regional snapshot share each laid-out
hard line. A newly built line moves its record vectors into exact allocations
before it is shared: shrinking a vector in place can keep its larger allocator
block, so growth slack would otherwise be retained for the life of the cache.

```sh
cargo build --release --offline --example large_file_memory
python3 scripts/measure-large-file-memory.py --output /tmp/viem-memory.json --limits scripts/fixtures/large-file-memory-limits.json
python3 scripts/measure-large-file-memory.py --supplemental --output /tmp/viem-memory-extra.json --limits scripts/fixtures/large-file-memory-limits.json
python3 scripts/measure-large-file-memory.py --rich-paste --output /tmp/viem-memory-paste.json --limits scripts/fixtures/large-file-memory-limits.json
```

`--limits` is optional; omitting it collects observations without enforcing
ceilings. `--binary` selects a preserved executable. The runner records source,
probe and binary hashes, checks exit status, and with limits checks required
phases, useful undo and live/cumulative-peak requested heap. The ceilings were
established for arm64 release builds; a different environment needs an explicit
comparison, not silent threshold changes.

The rich-paste set uses fresh dense Markdown destinations with DOS endings and
UTF-8 or UTF-16 source. It pastes a whitespace-prefixed styled fragment into an
ordinary paragraph or an empty list continuation before measuring undo and redo.
Its operation samples record additional live and peak heap, allocation calls,
elapsed time and work counters; separate operation ceilings prevent opening's
cumulative peak from hiding paste regressions.

Requested Rust allocation sizes exclude allocator overhead and native allocations.
The probe also reports macOS process RSS/footprint where available. A cumulative
peak is a high-water mark, not a per-operation allocation total; retained RSS
after dropping a document is not by itself evidence of a leak. Fixtures use mock
shaping and no syntax providers or native UI. They do not establish whole-app
memory bounds or a universal bytes-per-source-byte multiplier. See the
[fuzzing guide](fuzzing.md) for the separate retained-history ledger probe.

After editing pauses, a source of at most 1 MiB prepares its other Markdown
view in the background and retains it until the next edit or switch, so a
Preview switch need not reparse. In the 4 MiB `switch` fixture a Preview
projection costs about 120 MiB live and a Code projection about 50 MiB, so the
cap bounds the speculative copy near 30 MiB. Larger sources switch synchronously.

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

Interactive scrolling and typing with real Core Text shaping and drawing use the
opt-in [pre-rendering benchmark](../src/mac/Editor/Tests/EVPrelayoutBenchmarkTests.swift).
Build it optimized; the native-test profile and debug Swift distort timings:

```sh
VIEM_RUST_PROFILE=release VIEM_PRELAYOUT_BENCHMARK=1 scripts/test-mac.sh -c release -Xswiftc -enable-testing --filter EVPrelayoutBenchmarkTests
```

Each `PRELAYOUT_BENCH` line names a fixture and mode (`immediate`, `idle` after
preparation, or `disabled`). Compare `extent_changes`, `main_shapes`, the
`scroll_core_ms`/`scroll_draw_ms` percentiles and `scroll_busy_ms` (main-thread
work not caused by input) across builds on identical fixtures. Main-thread busy
spans include the test window's first display; they are not physical display
latency.

The `xctest` host is hardened, so `sample` collects no stacks from it. The
[shape benchmark](../src/mac/ShapeBench/main.swift) drives the release core with
the real Core Text provider in an ordinary process, on the same fixture: it
times the first layout, complete pre-layout (computed on its own thread) and
typing in the middle of the document, and counts provider calls. Attach
`sample <pid>` while it runs; `--iterations` lengthens the run:

```sh
cargo build --release --lib
touch src/mac/CViemCore/shim.c
VIEM_RUST_PROFILE=release swift build -c release --product viem-shape-bench
.build/release/viem-shape-bench --iterations 3 --typing 400
```

Each run prints an order-sensitive `response_digest` of every shape response.
Provider changes that must keep identical rendering compare it with the previous
build using `--no-figures` (inline-image render handles embed a per-load
identity) and with figures under `--no-handle-digest`. `--memory DOCUMENTS,VIEWS`
instead reports the process footprint as each fully pre-rendered view is added.
It has no window, drawing or syntax-provider workers, so it bounds core and
shaping cost rather than complete frame latency.

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

- **Native shaping throughput:** speculative layout of 95 KB of Markdown takes
  about 400 ms of worker time with Core Text (about 90 µs per provider
  request). Resolved fonts are cached per font-catalog generation and render
  resources are built only for glyph signatures not already registered. Core
  Text's own line creation and caret enumeration are now about a third of the
  provider's time.
- **Typing latency:** a keystroke in a 95 KB Markdown document costs about 9 ms
  natively, about half of it drawing. Core work reuses unchanged paragraphs'
  resolved styles across viewport captures, shifts unchanged code-block spans
  across edits, and shares retained lines between the snapshot and the line
  cache. History commits still walk every source piece and recount retained
  buffers, so keystroke cost grows slowly with session length; reused cached
  lines are still copied to restamp caret identities.
- **Structural edits at the document end:** a guarded band may retain the
  document's terminal line ending when every patch ends before it and a
  nonblank row precedes it. A blank final row still reprojects completely.

- **Progressive first display:** document construction still decodes and builds
  the initial state before returning. Compact storage does not make opening
  progressive. Any incremental/disk-backed design must preserve immutable
  snapshots and stable encoding interpretation after a late invalid byte.
- **Remaining flat-text consumers:** sentence/pair scans, case replacement and
  Visual Block range resolution now use snapshot queries. Structural formatting,
  specialized paragraph keys, link-context typing, layout-dependent command
  motion and replay still contain `Document::text()` calls. Full-reprojection
  identity reconciliation also materializes old and new snapshots; traversing
  the complete fallback does not require retaining compatibility copies.
  Markdown list editing (Enter, renumbering, removing list treatment) builds
  `list_structure()` from the complete block store, which retains a block-record
  copy with each edited snapshot even when the projection edit is regional.
  These paths need separate measurements and local verification before every editing
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

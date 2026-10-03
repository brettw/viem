# Performance tools and remaining work

Keep measurement output in a temporary directory or `target/`, not in `docs/`.
Compare identical fixture bytes, profiles, build modes, view sizes and provider
settings. Record source/build identity and run without concurrent compilation.
An old benchmark result is not evidence of current performance.

## Large-file memory

The [allocator probe](../examples/large_file_memory.rs) exercises the real core
in fresh serial release processes. The [runner](../scripts/measure-large-file-memory.py)
owns two nine-case fixture sets: standard literal opening/editing/views and
supplemental encodings, dense lines, giant words, search/save/history/reclamation.
The checked-in [limits](../scripts/fixtures/large-file-memory-limits.json) are
regression inputs, not recorded results; do not relax them to make a change pass.

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
samples, not native frame/compositor latency; thread CPU timing is Linux-specific.
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

For Windows input, drawing, pre-layout and startup measurements:

```powershell
.\scripts\test-win.ps1 -Optimized -ProfileDocument docs/markdown_demo.md -ProfileScenario page -ProfileIntervalMs 100
.\scripts\test-win.ps1 -NoBuild -Optimized -ProfileDocument docs/markdown_demo.md -ProfileScenario page -ProfileIntervalMs 100 -DisablePrelayout
.\scripts\test-win.ps1 -NoBuild -Optimized -ProfileDocument docs/markdown_demo.md -ProfileScenario scroll -ProfileIntervalMs 16
.\scripts\test-win-startup.ps1 -ProfileDocument docs/markdown_demo.md
```

Windows scripts save reports and isolated profiles under `target/windows-validation`.
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
  Visual Block range resolution now use snapshot queries. Other command motion,
  selection and replay paths still call `Document::text()`. Audit those paths
  before claiming every command has a bounded working set.
- **Cold giant-line work:** bounded retained geometry does not bound first-pass
  metric discovery or a distant cold jump. Current long-line tests include
  linear source scanning with bounded shape fragments. Measure adversarial
  graphemes, bidi transitions and dense style changes as well as plain ASCII.
- **Complete native accounting:** the core allocator probe omits native shapers,
  glyph storage and syntax trees. Measure component ownership and whole-app
  live/peak memory through opening, disjoint edits, history branches, multiple
  views, background jobs and release. Dense decoding exceptions and rich
  projections need separate coverage from ordinary literal runs.

Unicode line-breaking data, offline regeneration and the intentional numeric
conformance tailoring difference are documented with the
[pinned data](../data/unicode/15.0.0/README.md). Preserve those tests when changing
layout performance; there is no separate historical validation report to update.

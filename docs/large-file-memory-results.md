# Large-file memory implementation and validation

The compact literal-document, history, cache, and giant-line changes are
implemented. The original analysis and baseline are preserved in
[large-file-memory-analysis.md](large-file-memory-analysis.md) and
[large-file-memory-measurements.json](large-file-memory-measurements.json).

The subsequent [first-party line-breaking replacement](first-party-line-breaking.md)
removes the external line-breaking implementation and repeats all 18 memory
fixtures. The tables and raw results below preserve the preceding implementation's
measurement history; the follow-up report records the replacement's validation.

The implementation follows the first four stages of the proposed plan, with
Plain Text and Code sharing the compact literal infrastructure. Progressive
opening, disk-backed storage, and lazy rich-format projections remain later
work. The results below distinguish measured core heap, process memory, and
structural/native tests; they do not equate a Rust-only probe with total AppKit
or Tree-sitter memory.

**Final comparison with the original nine fixtures**

All values in this table are MiB of requested Rust heap. “Peak” is the
cumulative high-water mark through the named checkpoint. Every original
fixture passes the pinned regression ceilings.

| Fixture / checkpoint | Live: original → final | Peak: original → final | Live reduction |
| --- | ---: | ---: | ---: |
| 1 MiB ordinary lines / open | 134.93 → 3.13 | 258.01 → 6.50 | 97.7% |
| 4 MiB ordinary lines / open | 539.64 → 12.46 | 1,031.97 → 25.94 | 97.7% |
| 16 MiB ordinary lines / open | 2,158.49 → 49.76 | 4,127.83 → 103.68 | 97.7% |
| 1,000,000 short lines / open | 628.76 → 123.45 | 1,230.82 → 181.83 | 80.4% |
| 1 MiB single line / open | 131.85 → 2.28 | 254.56 → 5.13 | 98.3% |
| 4 MiB Text / two views, scroll + wrap | 616.91 → 29.79 | 1,736.87 → 35.63 | 95.2% |
| 4 MiB Code / two views, scroll + wrap | 556.10 → 29.79 | 1,031.97 → 35.62 | 94.6% |
| 4 MiB Text / explicit flat read | 543.64 → 16.48 | 1,031.97 → 25.94 | 97.0% |
| 100 MiB ordinary lines / open | 13,889.45 → 317.09 | 26,955.60 → 656.94 | 97.7% |

The 100 MiB source now retains about **3.17 bytes of heap per input byte**,
versus about 139 originally. This satisfies the main design objective: ordinary
literal text uses compact runs plus source/text and line metadata instead of
large mapping records per character. The original plan did not promise a
numerical multiplier. The much denser million-line input still costs about
64.7 heap bytes per source byte; the measured 80.4% reduction is substantial,
but line metadata is the next storage target for that workload.

The independent process measurement agrees with the direction: 100 MiB open
peak RSS fell from **26,556.50 MiB to 870.27 MiB**. Current RSS and footprint,
including after release, remain available in the raw JSON. These are process
metrics from the core probe, not measurements of the complete native app.
The instrumented through-open checkpoint fell from 9.81 s to 1.34 s in these
single runs; no latency-percentile claim follows from that observation.

The newline transaction no longer raises the ordinary 4 MiB document peak
from 1,032 to 1,737 MiB: both final opening and newline checkpoints stay at
25.94 MiB. On the 1 MiB giant line, the first edit's cumulative peak fell from
686.4 MiB to 5.13 MiB. Every original completed edit now retains undo under the
shipped policy. The explicit flat read still adds an output-sized 4 MiB cache,
which is intentionally visible in the comparison.

Raw final results:
[original fixture rerun](large-file-memory-final.json),
[supplemental fixtures](large-file-memory-supplemental.json).
Both record the same source and binary hashes; the baseline is unchanged.

**Supplemental acceptance cases**

These extend the original measurements and do not invent unavailable old
baselines. Heap values are MiB.

| Case / checkpoint | Live | Cumulative peak | Result |
| --- | ---: | ---: | --- |
| 100 MiB UTF-8 / save after search, distant edits, undo/redo | 317.27 | 656.94 | Undo remains available |
| 100 MiB UTF-16LE / open | 208.86 | 392.58 | Bounded conversion runs; edits retain undo |
| 100 MiB UTF-16BE / open | 208.86 | 392.58 | Bounded conversion runs; edits retain undo |
| 100 MiB Latin-1 / open | 317.09 | 656.94 | Edits retain undo |
| 2 MiB single word / two views + wrap | 9.54 | 11.55 | Sparse demanded glyph geometry |
| 2 MiB word with prefix/tail and preceding line / two views + wrap | 34.76 | 59.33 | All rows preserved; fixed-size shape cache |
| 1,000,000 short lines / save after exercise | 123.54 | 181.83 | Undo remains available |
| 16 MiB CRLF / open | 90.21 | 150.27 | Exact line-ending exceptions |
| 1 MiB forced-invalid UTF-8 / open | 210.87 | 312.66 | Dense diagnostic exceptions remain expensive |

The 100 MiB exercise ends at **0.17 MiB live** after deleting almost all text
and explicitly pruning history. The million-short-line exercise reaches
**0.11 MiB** at the same checkpoint. Dropping each document returns requested
heap approximately to baseline. These observations validate independently
releasable backing chunks and sparse history; they do not promise immediate
OS reclamation of allocator-cached pages.

**Implemented changes and their intended effect**

| Change | Intended effect and validation | User-visible consequence |
| --- | --- | --- |
| Compact decoding and literal provenance | Ordinary UTF-8, UTF-16 and Latin-1 use bounded mapping runs. Lookup scans at most one conversion run, and UTF-8 identity lookup uses arithmetic. Dense mapping/edit oracles cover byte boundaries, BOMs, invalid bytes, mixed endings and graphemes. | Exact text and source preservation; no appearance change. |
| Regional literal edits | Text gains Code's regional line-topology behavior. Huge-line and distant edits use separate bounded windows. Tests inspect bytes decoded and persistent paths copied. | Enter, joins, typing, paste and undo keep their editing semantics. |
| Shared paragraph attributes | A block is 32 bytes instead of the original 328 bytes. Default attributes share one immutable allocation; formatting changes copy attributes on write. Ledger edges also deduplicate shared defaults. | Existing paragraph identities and independent formatting remain intact. |
| Releasable backing chunks | Source and formatted buffers use independently allocated chunks of at most 64 KiB, apart from indivisible Unicode content. A small surviving piece no longer retains an entire original file allocation. | Deleted bytes remain while undo or another snapshot needs them, then become reclaimable. |
| Streaming search, hash and sparse undo summaries | Regex scans reuse a 4 KiB window plus Unicode context and bounded line batches. SHA-256 consumes source pieces. Distant undo copies changed source ranges rather than the intervening document. | Search results, save identity and undo semantics remain exact. Explicit whole-document output still requires output-sized storage. |
| Useful default undo | The default allows 256 MiB of additional history above the live document. Total, live-state and additional-history estimates are reported separately. Explicit combined limits remain available. | Large documents retain useful undo. Total allowed memory grows with the unavoidable live document cost. |
| Shared, budgeted layout | Shape caches have byte and entry limits; immutable fragments and font names share storage. Height-tree staging copies only changed persistent paths. | Evicted text costs reshaping when revisited. Existing off-screen height estimates can refine the scrollbar. |
| Native render leases | One fragment lease is shared across Rust caches/snapshots. The last lease releases native glyph resources; response arenas pin resources only for their callback lifetime. Native registry dictionaries shrink after eviction. | Rendering remains exact; old raw drawing exports retain their existing exact-layout validity requirement. |
| Horizontal geometry on demand | Giant rows retain visible/focused clusters and compact advance/bidi summaries. Tests compare complete and streamed multilingual geometry, selection, shaping boundaries and invalidation. | First cold metric scans can take time. Visual Block requests complete rectangular geometry when needed, so that operation can temporarily use more memory. |
| Byte-budgeted Code sessions | Idle syntax sessions have a 512 MiB aggregate retention target, and unusable capped trees are released. Existing primary/fallback and style-continuity policies remain in force. | Returning to an evicted session may require reparsing; previously retained/default highlighting can appear while work catches up. |

**Iterations driven by measurement**

The first implementation reduced the 100 MiB ordinary-line fixture from
13,889 MiB retained Rust heap to 455 MiB. Measurement and review then found an
opening diagnostic that recreated a flat copy, and excessive default block
metadata. Removing that copy and sharing the full block attributes reduced the
same fixture to about 317 MiB and the million-short-line fixture from 172 MiB
to 123 MiB in the second pass.

Supplemental measurements then exposed a distinct wrapped-overflow problem:
an unbreakable two-megabyte word still retained enormous row geometry. The
[second-pass supplemental report](large-file-memory-supplemental-iteration-2.json)
preserves that failure. The [third pass](large-file-memory-supplemental-iteration-3.json)
fixed the all-word line but exposed the route through a preceding short line.
The final fix covers both: the prefix/word/tail two-view case fell from
1,059.58 MiB retained after its second view and 2,437.83 MiB peak through
scrolling to 34.76 MiB and 59.33 MiB through scrolling. Its remaining fixed
cost includes exact 64 KiB prefix shaping cached per view. The new guard uses
48 MiB live and 64 MiB peak; no original fixture ceiling was raised.

**Measurement method and limits**

The nine original fixtures run in fresh, serial release subprocesses. Input
bytes and original operations are unchanged; the shipped default history
policy is deliberately changed as described above. The tracker records live
and cumulative peak requested Rust allocation sizes, allocation counts and
macOS `TASK_VM_INFO` RSS/footprint. Peaks are process high-water marks, not
isolated per-operation deltas. Allocator retention, compression and native
allocation explain why requested heap and process memory differ.

The original fixtures explicitly use UTF-8 and disable syntax providers. Views
use the mock shaper. Supplemental UTF-16/Latin-1 fixtures contain the same
ASCII logical pattern, at the specified physical byte count; UTF-16 therefore
contains half as many characters. They are additional observations, not a
like-for-like comparison with the older mixed-encoding syntax benchmark.
Dense-invalid and CRLF cases are reported separately; no universal multiplier
is promised for exceptional encodings or rich formats.

Read-only compatibility APIs can still explicitly materialize complete flat
text. The pinned `flat` fixture demonstrates that cost. History estimates
refresh during mutation/navigation accounting, so they are not an instantaneous
malloc census after an arbitrary read-only cache materialization. Common paths
addressed here use bounded reads; the remaining legacy whole-text consumers
and output-sized register/export operations require further audit.

Native tests cover real Core Text geometry, generation changes, callback
replacement, final lease release on worker threads after detach, provider
reuse, and dictionary reclamation. These tests establish ownership and bounded
exports; they do not measure the complete application's RSS or the cost of
large native parse trees. Tree-sitter's existing cooperative soft limits also
remain distinct from a total-process memory ceiling.

**Validation**

Rust validation covers all **157 library, integration-test and example
targets**: **2,276 tests passed**, with 4 existing tests ignored. Coverage was
checked against Cargo metadata, combining the full-target run, its remaining
targets, and focused reruns after the failures described below. The entire
library suite was rerun after the final production fix: 1,148 passed. Run
`cargo test --offline --all-targets` to reproduce the complete target selection.

The release probe passed all 18 original and supplemental cases against the
checked-in memory/undo ceilings. Final source, probe and executable hashes
were checked against the recorded artifacts after the run. `make debug`
rebuilt, packaged and signed the native application successfully.

The full native `scripts/test-mac.sh` run passed **487 XCTest tests
and 36 Swift Testing tests**, including real Core Text giant-line geometry,
wrapped prefix/overflow/tail rows, multiview behavior, font changes, save,
clipboard and undo integration. The run used the local module cache and macOS
service access required by the AppKit tests, plus the wrapper's temporary
`VIEM_CONFIG_DIR`. A direct Swift invocation inherited local configuration and
reported a startup style warning in two tests; the same binaries passed that
entire 13-test group with isolated configuration. Native lease tests separately
cover final release after detach, generation-safe provider reuse and registry
capacity reclamation.

The new core regressions check bounded bytes decoded for local/newline/huge-line
edits, exact encoding/BOM/line-ending preservation, sparse distant history,
additional-budget accounting and post-deletion chunk release. Layout tests
cover persistent-height sharing, byte budgets, multilingual complete-versus-
streamed geometry, invalidation, distant navigation, and Visual Block's
complete-geometry path. The FFI provider contract and all consumers were
rebuilt together; the new retain/release callbacks use provider ABI 3.

One existing HTML cache-invalidation test set a 10,010-entry allowance while
leaving the new default 32 MiB byte allowance intact. Measurement showed
277 legitimate evictions and exactly 279 subsequent shape requests: those
277 paragraphs plus two changed fragments. Its setup now explicitly allows
64 MiB as well as 10,010 entries, verifies that all 10,000 fragments are warm
with zero evictions, and retains the original at-most-two-request assertion.
Production budgets and the pinned memory-fixture limits were unchanged.
The direction-invalidation fixture similarly needed an explicit 2 MiB
decoration allowance: its original 1 MiB budget evicted 273 ordinal labels.
It now verifies both caches are fully warm and retains its at-most-two-request
bound. A decode-accounting test that formerly expected Plain Text Enter to
rebuild the entire document now uses HTML to exercise the full fallback;
literal newline behavior is covered by the new regional-work regressions.

The command matrix also caught a real Visual Block search regression: opening
a command prompt temporarily changes command mode, but retains its rectangular
selection. Layout cleanup now follows release of that selection, preserving
its exact layout through forward/backward searches. The complete command
matrix and the related Visual Block and input-layout suites pass after the fix.
The final memory measurements and native validation were repeated afterward.

Applying user style defaults correctly changes both total and live-state
memory estimates without creating history. That existing test now permits
the new diagnostic fields to change while comparing all semantic history
fields. Independent current-only and full ledgers can differ by a few bytes
of hash-table bookkeeping; the fresh-recount regression verifies their graphs
and bounds current-only additional overhead at 4 KiB. These estimates remain
distinct from the allocator probe's requested-byte measurements.

**Remaining work**

Opening still performs an eager compact decode/index pass. Cold giant-row
metric discovery and distant wrapped jumps can require linear work. The
million-short-line case still has meaningful per-line source/hard-line/index
cost. Rich normalization/provenance and dense decoding exceptions have different
costs from ordinary literal runs. More aggressive height coarsening, cross-pane
shaping reuse, progressive projection and disk backing should be evaluated
against those residual costs. The AGENTS.md TODO remains open for this broader
acceptance work.

Remaining flat-text command consumers include some word/sentence/text-object
operators, Visual selections, indentation/case operations, and Ex navigation.
The optimized common typing, horizontal movement and search paths do not prove
that every command has a bounded working set. Existing paragraph-flow paths
also need separate profiling before claiming all rich layout is bounded.
Giant-line summaries scale with shaping fragments and bidi-run transitions;
dense style changes, per-character bidi alternation and one indivisible huge
grapheme need their own worst-case measurements. Revising a giant line can
invalidate its summary and require another linear metric scan.

Sparse history summaries inspect piece metadata in a distant changed hull.
Heavily repeated or moved backing ranges can exceed the matching-work allowance
and fall back to a conservative exact replacement hull; ordinary distant edits
use sparse patches. This preserves correctness but leaves a more specialized
memory optimization for later.

Reproduce the comparisons after building:

```sh
cargo build --release --offline --example large_file_memory
python3 scripts/measure-large-file-memory.py --output /tmp/viem-memory.json --limits docs/large-file-memory-limits.json
python3 scripts/measure-large-file-memory.py --supplemental --output /tmp/viem-memory-extra.json --limits docs/large-file-memory-limits.json
```

The checked-in limits cover all required checkpoints, useful undo, and
post-pruning reclamation. The original ceilings were pinned after iteration 2
with 25% headroom rounded to MiB; the final implementation must pass them
without relaxing them. The new giant-word test requires at most 48 MiB live
and 64 MiB cumulative peak requested heap. These are regression guards for the
specified release fixtures, not advertised total-process memory limits.

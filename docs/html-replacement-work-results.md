# HTML replacement work measurements

The deterministic counters include selection/context capture, discarded scratch
candidates, IME preparation and commit, and the first continued typing event.
Opening is measured separately. `measure_document_work` uses opt-in thread-local
counters; nested measurements contribute to their parent, independent threads do
not, and panic unwinding restores the enclosing measurement. Disabled counters
allocate no per-query diagnostic records.

## Reproduction

Measured September 20, 2026 on a Mac13,2 with Apple M1 Ultra, 128 GB RAM,
macOS 26.6.2 (25G83), arm64. Rust 1.98.1 (48a229cea, September 1, 2026),
Cargo 1.98.1 (797e8a9bc, August 5, 2026). Tests use the repository's test
profile (`opt-level = 1`, debug information) and `MACOSX_DEPLOYMENT_TARGET=26.0`.
The counter profiles use the mock text measurement provider. Native macOS
timings are reported separately below; Windows latency was not measured.

```sh
MACOSX_DEPLOYMENT_TARGET=26.0 cargo test --locked --test html_replacement_work
MACOSX_DEPLOYMENT_TARGET=26.0 cargo test --locked --test html_replacement_work -- --ignored --nocapture
```

`HTML_REPLACEMENT_PROFILE_COUNTS=1000,10000,100000` selects paragraph counts
for the ignored matrix. Each fixture measures beginning, middle, and final
paragraph replacements, both native selection and IME, and continued typing.
The fixture families are plain paragraphs, bold/link/entity paragraphs, and
those rich paragraphs encoded as UTF-16LE. Separate profiles cover a 2 MB
paragraph, 256 nested spans, and a 1 MB attribute on the selected link.
The regular growth tests additionally select the plain, bold, link, and entity
contributors independently and keep a second view attached. A separate regular
regression matches the native fixture exactly:
`<p>alpha <b>bold</b> omega</p>\n`, replacing all four bold characters and then
continuing typing at the beginning, middle, and end of 1,000/10,000 paragraphs.

Byte counts sum actual calls, including repeated processing of the same bytes.
Lexical HTML tokenization and HTML5 tree tokenization are separate counters.
Projection counts extend the existing candidate-work statistics; full-source
and decoding counters also see work before a model transaction exists.
Index newly-retained bytes report the maximum new charge in a single memory-ledger
visit: initial opening charges the complete index, while later visits skip known
shared nodes and charge only new paths. Separate counters measure memory-graph
allocation visits, newly registered allocations, release visits and releases,
including scratch histories. History's memory visitor remains the authority for
shared live-snapshot memory.
The recorded broad-work reasons identify a complete HTML grammar candidate,
a lexical index splice whose safety/convergence was not proven, and a regional
fragment whose recovered ancestry or inherited style context could not be
reproduced safely. They do not
yet classify every structural or style policy that caused a complete candidate.

Timing is supplementary. CSV `operation_pair_elapsed_ms` is the elapsed time of
replacement **plus** its continued typing event; both rows report the pair,
not the individual event. Opening counters cover document construction only;
the paragraph-matrix opening times also include fixture and view setup.
Concurrent development builds and the mock layout provider make these timings
unsuitable as platform latency guarantees.

## Scanner baseline provenance

[Baseline counters](html-replacement-work-baseline.csv) contain 130 measurements.
The instrumented executable was built with `html_typing.rs` from commit
`5ce0791`, temporarily replacing only that file; the in-progress indexed version
was saved and restored byte-for-byte immediately after compilation. The copied
executable and rlib were run independently of subsequent source edits. IME uses
`Z` after the native `X` replacement, ensuring that its commit changes text.

This is a **scanner baseline**, not an isolated benchmark of the entire original
commit. Other local projection improvements were already present. In particular,
the large-paragraph baseline already uses bounded candidate projection, so its
before/after comparison isolates the complete scans used to recover scope context.

The rich fixture is `<p>ab<b>cd</b><a href='/x'>ef</a>&amp;</p>`. At 100,000
paragraphs, replacing one bold character near the end materialized 8,400,007
source bytes and decoded 8,400,599 bytes, despite projecting only 13 formatted
bytes. IME decoded 8,400,308 bytes; its continued typing decoded a further
4,200,303 bytes. The 2 MB paragraph's native replacement decoded 4,000,850 bytes
while projecting only 271 bytes. These gaps are why candidate-only work counters
were insufficient.

## Deterministic acceptance checks

The regular full-path tests require zero complete source materializations and
zero complete projection candidates. Each replacement and continued typing
scope must materialize/decode/tokenize fewer than 32 KiB, visit fewer than 512
scope entries, rebuild fewer than 512 index entries, visit fewer than 4,096 index
nodes, and make fewer than 16,384 memory-ledger allocation/release visits for these
small local structures. These are generous upper bounds rather than exact implementation
counts; unrelated content must not increase any byte-based bound.

Huge-paragraph coverage includes native replacement, IME model preparation and
commit in a single unbroken word, and full core IME replacement in both an
unbroken word and wrapped prose.
Actual deep nesting and large selected attributes are measured separately:
reading/copying their affected syntax can legitimately scale with its own size.

## Results after indexing and bounded preparation

[After counters](html-replacement-work-after.csv) contain 156 measurements:
132 opening/local-structure/main-matrix rows and 24 rows for the exact native
whole-bold fixture. The complete profile and regular work suite passed all eight
tests; three instrumentation unit tests also passed.

A focused 36-case regression also checks space replacement through native,
per-key Vim change, and IME in both selection directions and three encodings.
Supporting space normalization retains the preceding character's style/link;
only authored text inherits the replacement style. Continued typing, the Insert
Ctrl-U floor, fresh-source reopening, and exact undo are checked.

Replacing a bold character near the end of the UTF-8 rich fixture now has constant
byte-processing work as unrelated paragraphs increase:

| Paragraphs | Full source bytes | Decoded bytes | Lexical tokenized bytes | Scope entries visited | Index nodes visited | Memory allocation visits |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1,000 | 0 | 593 | 80 | 5 | 138 | 510 |
| 10,000 | 0 | 593 | 80 | 5 | 193 | 678 |
| 100,000 | 0 | 593 | 80 | 5 | 241 | 950 |

All three cases project 13 formatted bytes and rebuild 18 index entries.
At 100,000 paragraphs the equivalent IME replacement decodes 299 bytes and its
continued typing decodes another 299 bytes. UTF-16LE native replacement decodes
674 bytes and IME decodes 342 bytes. None of these edits materializes the complete
source, builds a complete projection, or records a broad-work fallback.

The exact native whole-bold fixture exposed an additional slow path: preserving
the two surrounding spaces requires multiple nearby source edits. Their shared
verification window is now projected together while the published source patches
remain separate. At the end of 10,000 paragraphs, native replacement decodes
807 bytes, lexically tokenizes 266 bytes, visits 475 index nodes and makes 696
memory allocation visits. IME decodes 444 bytes, tokenizes 52 bytes, visits 218
index nodes and makes 688 memory allocation visits. Continued typing decodes
283 bytes. Each has zero full-source materializations and full projections.

For the 2 MB unbroken-word paragraph, native replacement now decodes 806 bytes
instead of the scanner baseline's 4,000,850; full core IME replacement decodes
407 bytes. The earlier IME overlay error
`Layout(LongLineSliceNeedsMoreText { text_offset: 0 })` was fixed by bounded
composition layout capture and overflow shaping. The full core path, continued
typing, and the separate model commit path now pass.

The deep nesting fixture visits 771 scope entries for native replacement and
1,028 for IME, proportional to its 256 actual ancestors. The 1 MB selected-link
attribute requires copying its own syntax, but source decoding remains local
(546 bytes for native replacement; 277 for IME). These are deliberately outside
the small-local-structure scope-entry bound.

## Native macOS timings

The fresh debug AppKit build passed all ten selected replacement, whitespace,
and pending-style tests. `testReplacementLatencyAcrossLargeHTMLDocuments`
measures the exact whole-bold fixture at the beginning, middle, and final
paragraph. Each interval includes accessibility selection and its range check,
replacement with `X`, continued typing of `Y`, and synchronous presentation
updates; IME additionally sets marked text before committing `X`. Opening,
navigation to establish visible layout coverage, result checks, source
serialization, and undo are outside the timed interval.

| Paragraphs | Native replacement + typing | IME replacement + typing |
| ---: | ---: | ---: |
| 1,000 | 24.8–30.4 ms | 23.6–38.4 ms |
| 10,000 | 82.0–105.2 ms | 47.4–93.1 ms |

Ranges are the three observed positions from one run, not statistical
percentiles. Swift and Rust used debug builds (Rust `dev`, unoptimized), so these
numbers are not directly comparable to the optimized core-test timings. The
tests also verify inserted text, retained bold formatting, and exact original
source after undo. Reproduce this native group with:

```sh
scripts/test-mac.sh --filter 'EVRichTextReplacementTests|EVHTMLTypingWhitespaceTests|EVPendingTypingStyleTests'
```

## Broader validation

The full Rust regression run also exercises the shared core and C ABI. The
Windows binding check verifies all 147 exported functions; native Windows
execution and latency measurements remain pending on a Windows host.

Final focused validation passed all 1,512 enabled library tests, the 1,008-case
small HTML replacement oracle (including recovered markup, UTF-8/UTF-16,
whitespace, saved text, styles, and links), and the 36-case space/session matrix.
Memory tests cover shared/pruned index snapshots and iterative release with
50,000 nested unclosed scopes, avoiding call-stack growth on deeply nested input.

One unrelated regression test already fails on pristine commit `5ce0791`:
`viewport_end::wrapped_document_end_resumes_chunks_and_publishes_only_the_terminal_viewport`.
After resizing a long plain-text paragraph, the terminal layout retains 115
rows where the test requires fewer than 100. The same failure was reproduced
from an isolated archive of that commit. Its text coverage and viewport endpoint
remain correct; this change does not alter that viewport behavior.

A separate pre-existing core batching limitation was reproduced on both trees:
one Normal-mode `InputEvent::text("llvllc")` for
`<p>A <b>B</b> C</p>\r\n<p>D</p>` rejects a stale selection boundary.
Sending the same Vim keys as separate events passes. This is distinct from the
native replacement regression fixed and covered above.

## Remaining measured costs

Index memory deserves a separate compact-storage followup. Opening the 4.2 MB
UTF-8 fixture with 100,000 rich paragraphs creates 1,000,000 index entries and
charges 373,418,415 bytes (about 356 MiB) for its index under the existing
conservative memory ledger. This is a retained-allocation accounting value,
not measured process RSS. The corresponding local replacement charges only
16,119 new index bytes; shared retained nodes are skipped. Scratch planning no
longer rebuilds a complete memory ledger.

Bounded source work is not a complete latency guarantee. In this run the 2 MB
unbroken-word native replacement plus continued typing took about 461 ms with
the mock layout provider; IME took about 673 ms, despite the local source counts
above. Long-word caret/layout and other command work remain candidates for a
separate profile. Native debug latency also still rises with document size after
source processing becomes local; a frontend/layout profile should identify the
remaining cost before setting latency thresholds.

Broad structural/global-style edits can still require a complete grammar parse.
The fallback counters distinguish full projection, lexical nonconvergence, and
recovery/style-context rejection; they do not yet identify every higher-level
policy that selected a full parse.

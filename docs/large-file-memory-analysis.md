# Large-file memory analysis

Analysis of commit `d68b03a29f639a2673c5c3d566e84e82b7a75d1b`, September 2026.
Priority: Plain Text, then Code, then formatted Markdown/HTML.

The first investment should be **exact, compact literal-document storage and
incremental text edits**. Most of the current cost exists before layout or
highlighting begins. This can be fixed without visible compromises. More
aggressive lazy layout and syntax policies are useful next steps, especially
for long lines and multiple views, but cannot remove the main amplification.

The existing AGENTS.md TODO remains valid and incomplete. This investigation
adds independent requested-heap and process measurements, identifies additional
paths needing attention, and proposes implementation choices. It does not
change the editor or its product requirements.

**What was measured**

[The reusable probe](../examples/large_file_memory.rs) runs the real Rust
document core, with a global allocator tracking requested live/peak bytes and
allocation counts. On macOS it independently reads its own `TASK_VM_INFO`,
including current/peak RSS and physical footprint. The field offsets were
checked against the installed macOS SDK. Each fixture ran in a fresh, serial
release subprocess on macOS 26.6.2 arm64, Rust 1.98.1. The normal history policy
was used throughout. Full raw results and build identity are in
[large-file-memory-measurements.json](large-file-memory-measurements.json).

These are single diagnostic runs, not latency percentiles. Allocator hooks add
overhead. Input is generated in memory and explicitly opened as UTF-8, so this
does not measure encoding detection, disk loading, or the native opening path.
CPU model/RAM were not independently re-queried for this run. The generated
fixtures isolate size and line density: `lines` has
128-byte ASCII lines with spaces and LF, `short` repeats `a\n`, and `long`
contains only `a`. Thus these are not the mixed Unicode/line-ending fixtures in
the earlier Code report. The 100 MiB case measures opening and release only.

Opening, before any views or syntax providers:

| Plain Text fixture | Live requested heap | Peak requested heap through open | OS peak RSS through open |
| --- | ---: | ---: | ---: |
| 1 MiB, 128-byte lines | 134.9 MiB | 258.0 MiB | 342.9 MiB |
| 4 MiB, 128-byte lines | 539.6 MiB | 1,032.0 MiB | 1,372.6 MiB |
| 16 MiB, 128-byte lines | 2,158.5 MiB | 4,127.8 MiB | 5,537.0 MiB |
| 100 MiB, 128-byte lines | 13,889.5 MiB (13.56 GiB) | 26,955.6 MiB (26.32 GiB) | 26,556.5 MiB (25.93 GiB) |
| 2,000,000 bytes, 1,000,000 LF breaks | 628.8 MiB | 1,230.8 MiB | 1,867.5 MiB |
| 1 MiB, one hard line | 131.8 MiB | 254.6 MiB | 334.0 MiB |

The ordinary-line cases retain roughly 135–139 bytes of requested heap per
source byte. The million-short-lines case retains about 330 bytes per source
byte. The latter has 1,000,001 logical lines because its final LF introduces an
empty last line. These ratios describe these fixtures, not all encodings or
documents. The 100 MiB open took 9.81 seconds in the instrumented run; this is
construction time, not an AppKit first-frame measurement.

Requested heap means Rust allocation sizes still owned by the process. It
includes the real allocation ledger but excludes malloc bookkeeping, internal
realloc copying, C allocations outside Rust's allocator, and native UI state.
RSS includes resident process pages; physical footprint follows the OS ledger.
Untouched allocated pages, freed-but-cached pages, and allocator behavior mean
neither process metric must equal requested heap. Reported compression was zero
at all saved checkpoints; this does not establish the absence of all OS paging
activity between checkpoints. All peaks are cumulative high-water marks through
that checkpoint, not reset for each operation; subtracting successive peaks
does not measure an operation's temporary allocation.

At the 100 MiB open checkpoint, RSS had already fallen to 16.16 GiB from its
25.93 GiB peak. After dropping the document, Rust requested heap returned to
approximately baseline while RSS remained around 14 GiB. This demonstrates
release of Rust-owned allocations, but not immediate return of every page to
the OS; it is not by itself evidence of a retained-document leak. Avoid comparing
different fixtures in one long-lived process using RSS alone.

The earlier [Code baseline](code-pipeline-performance.json) reported a
conservative history charge of roughly 14.6 GiB for a different 100 MiB UTF-8
fixture. That was not measured RSS. The new measurements corroborate the
underlying amplification without changing the meaning of the old numbers.
Complete allocation attribution by component and native AppKit measurements
are still missing, so these results do not close the TODO.

**Editing and operation costs**

The probe performs two separately completed `Document::replace` transactions:
insert `x` at byte zero, then insert LF immediately after it.

| Case | Observed result |
| --- | --- |
| Plain Text, 1 MiB ordinary lines | First insertion retained undo. The LF transaction lost all navigable undo; requested-heap peak rose from 258.0 to 434.3 MiB. |
| Plain Text, 4 MiB ordinary lines | No undo after the first completed insertion. LF raised requested-heap peak from 1,032.0 to 1,736.9 MiB. |
| Plain Text, 16 MiB ordinary lines | LF raised requested-heap peak from 4,127.8 to 6,947.3 MiB; OS peak RSS reached 9,796.7 MiB. |
| Code, same 4 MiB source, syntax disabled | Approximately the same opening heap as Text. The LF transaction stayed regional and did not increase the opening high-water mark. Default-budget undo was still unavailable. |
| Plain Text, 1 MiB single hard line | Even the single-character insertion raised requested-heap peak from 254.6 to 686.4 MiB. No layout was involved. |
| Read `text()` after an ordinary edit, 4 MiB Text | An additional 4,194,328 requested bytes remained allocated. The immediately reported history estimate did not reflect that lazy allocation. |

The 1 MiB undo result is possible even though its final current-state estimate
is below 256 MiB: retaining both old and fully rebuilt projections exceeds the
combined budget. These tests close each transaction; an open UI typing group
temporarily protects its parent/result and is a different checkpoint.

For the 4 MiB fixtures, creating two mock-shaped views and repeatedly moving
between the start/end while toggling wrapping retained roughly 16 MiB beyond
the post-edit document state. That is secondary to the document's hundreds of
MiB in this fixture. Repeated text may deduplicate shaping effectively; this
does not establish a native, varied-text, or whole-document-scroll cache bound.

**Why the document is so large**

1. **Character-sized mapping records.** [encoding.rs](../src/core/document/encoding.rs)
   lines 270–302 allocate a `DecodedSpan` for every scalar, including ASCII.
   [line_endings.rs](../src/core/document/line_endings.rs) line 261 builds
   another string and scalar-level units. [projection.rs](../src/core/document/projection.rs)
   lines 4418 and 1046 create one forward provenance record and two reverse
   boundary records per unit. Plain Text and Code use the same literal path.
   The retained records alone occupy about 96 bytes per ordinary scalar before
   tree overhead, text, and allocation accounting. The two affinities prevent
   simple adjacent-boundary deduplication.
2. **Expensive default paragraph records.** Each literal hard line receives a
   `Block`, including an owned `Paragraph` style name and inline optional
   formatting structs. The measured 64-bit `Block` size is 328 bytes:
   `BlockProperties` is 104 and `CharacterProperties` is 160 bytes. A million
   block values alone occupy about 313 MiB, before their strings and trees.
   This is a structural size calculation, not an allocation-stack attribution.
   The separate source-line index already packs line lengths into small leaves.
3. **Several generations of temporary storage coexist.** Decoded and normalized
   strings/vectors, provenance arrays, reverse arrays, and final persistent
   leaves overlap during construction. [range_index.rs](../src/core/document/range_index.rs)
   lines 408 and 726 also clone records into leaves. Small retained objects do
   not guarantee a small opening peak.
4. **Text topology edits fall back to whole projection.**
   [transaction.rs](../src/core/document/transaction.rs) line 7104 rejects LF/CR
   changes from the regional path except for Code; line 6286 flattens and rebuilds
   the candidate. The regional path itself bounds hard-line count, not bytes,
   and can decode an entire enormous line. Both defects were exercised above.
5. **Other full copies remain.** `text()` and range `as_slice()` accessors can
   retain flat compatibility caches. Initial save-point hashing copies all
   source bytes and creates a padded SHA input
   ([source.rs](../src/core/document/source.rs), lines 234 and 512).
   [RegexInput](../src/core/command/search_regex.rs), line 139, flattens snapshot text,
   allocates two `Vec<bool>` buffers of text-byte length, and requests all hard
   lines as a temporary vector. These are inspected operation costs, not
   measured components of the opening table.
6. **Ownership is shared but allocation granularity is too large.** Initial
   source storage is one large immutable byte allocation; 4 KiB formatted
   leaves also share a whole-file string. A tiny surviving slice can pin that
   entire buffer after a large deletion. The history ledger correctly shares
   allocation identities, but its hash entries and child vectors also have
   real cost. Distant grouped undo edits can additionally copy the full source
   hull between edits ([transaction.rs](../src/core/document/transaction.rs),
   line 5869).

**The first implementation track: exact improvements**

| Approach | Design and expected benefit | Visible side effects |
| --- | --- | --- |
| Compact literal mappings | Represent unchanged UTF-8 with identity/constant-offset runs over stable chunks. Use strides/checkpoints for converted text and explicit exceptions for BOMs, CRLF/CR, invalid bytes, and unequal encoding widths. Index both directions. Eliminates persistent records per ordinary scalar. | None required. A lookup may decode a bounded neighborhood internally. |
| Stream projection construction | Fuse or stream decoding, line interpretation, and literal projection; construct persistent leaves directly. Share immutable valid UTF-8 backing where applicable. Stream digest construction. Reduces temporary overlap and allocation count. | None required. A compact initial validity/index scan can remain. |
| Byte-bounded literal edits | Extend structural and disjoint regional editing to Text and split work by bounded chunks, including inside huge hard lines. Maintain compact mapping runs by splitting/coalescing persistent paths. | None required; Enter, joins, undo and source line endings remain exact. |
| Compact default blocks/styles | Pack line/block descriptors, intern style identifiers, represent inherited defaults implicitly, and allocate sparse declarations only where present. Particularly valuable for millions of short lines. | None required; preserve stable paragraph identities and styling behavior. |
| Remove accidental flattening | Use chunk/range iterators for commands, saving, hashing, and search. Query indexed hard-line boundaries instead of two full byte-indexed boolean arrays. Use sparse/lazy undo summaries. | Same results. Operations whose actual output is huge may need cancellable progress rather than one large synchronous allocation. |
| Chunk backing from initial load | Use moderate independently releasable immutable buffers for source and formatted storage, sharing where valid. Account for each allocation once. | None required. Deleted data legitimately remains available while undo or jobs still reference it. |

These changes should make common literal storage scale with source bytes,
compact line metadata, mapping exceptions, and changed persistent paths, rather
than many large records per character. A useful design budget is:

```text
retained memory ≈ immutable source + necessary decoded text
                + compact chunk/line/exception indexes
                + incremental retained history
                + explicitly budgeted syntax and per-view caches
```

This is an accounting model, not a measured post-optimization multiplier. In
valid UTF-8 identity regions, source and formatted text may share bytes while
retaining distinct nominal coordinate types. Dense exceptional input still
needs a measured worst-case bound.

Blindly merging existing `ProvenanceSpan` values is incorrect: current interior
lookup treats contributors as indivisible. Runs need explicit mapping kinds
and indexed lookup rather than expansion back into character tables. Preserve
grapheme validation, encoding boundaries, CRLF behavior, snapshot identities,
association/affinity, and exact bidirectional mapping. Rich entities and hidden
syntax still require relational provenance. The current source-boundary lookup
also scans provenance ([projection.rs](../src/core/document/projection.rs),
line 2462); compaction should fix lookup complexity as well as storage.

**Undo requires an explicit policy decision**

The shipped 256 MiB target counts the live document plus retained history.
Compaction should make that policy much more useful, but a sufficiently large
document can still exceed any fixed combined budget. I recommend exposing
live-state and prunable-history costs separately, then deciding whether to give
history an additional allowance above unavoidable live storage. This preserves
useful undo at the cost of a larger total memory allowance. A combined hard
target instead means explicitly limited undo depth for large documents.

That is a product change to the retention specification, not an accounting
optimization. Raising a benchmark budget or omitting real allocations would
conceal the current defect. Maintain true total-memory diagnostics either way.

**Layout: what is already lazy, and what remains unbounded**

The proposed approximate-scrollbar behavior already exists. Height indexing
starts as one estimated run; visible regions become exact. Styles are resolved
for requested regions. Wrapped giant lines use 64 KiB work slices and a bounded
checkpoint cache. The 64 KiB slice target can be exceeded by one indivisible
oversized grapheme. A first distant jump inside a wrapped giant line may still
compute preceding wrap state synchronously in successive slices: bounded
retention does not establish bounded cold-jump latency. The default offscreen
regional layout allowance is 32 MiB,
2,048 hard lines, and 8,192 rows per view. Swift normally receives only
materialized layout text. See [jobs.rs](../src/core/layout/jobs.rs), line 754,
[engine.rs](../src/core/layout/engine.rs), line 23, and
[EVEditorView.swift](../src/mac/Editor/Sources/EVEditorView.swift), line 1175.

However, these limits do not cover every retained layer:

- Unwrapped huge lines bypass slicing and capture/shape the whole line. The
  existing 2 MiB test explicitly expects this ([jobs.rs](../src/core/layout/jobs.rs),
  line 2479). The active viewport is outside the regional cache limit. Full
  cluster/caret arrays can then cross into Swift as well.
- Shape caches cap entry count, not total bytes. Cached fragments contain
  cluster/caret arrays and repeated font names; cache hits deep-clone fragments.
- Learned exact heights survive regional eviction. Varied heights can require
  a run per visited line. Layout installation deep-clones the boxed height tree
  ([engine.rs](../src/core/layout/engine.rs), line 1900;
  [height_index.rs](../src/core/layout/height_index.rs), line 438).
- [CoreTextRenderRegistry.swift](../src/mac/CoreTextProvider/Sources/CoreTextRenderRegistry.swift),
  lines 23–38 and 201, retains unique native glyph resources until a metrics
  generation change or view detach, with no explicit memory budget. Identical
  signatures deduplicate, so this is not one resource per source character;
  distinct text/context/style combinations can nevertheless accumulate.
- Each pane owns a separate Rust engine and native provider/registry. Shared
  documents do not imply shared shaping or glyph resources.

| Layout option | Benefit | Small side effects |
| --- | --- | --- |
| Byte-budget shaping and native resources; share immutable fragments and intern fonts | Bounds actual retained data and reduces cloning. Pin resources referenced by active snapshots; explicit render-handle leases must connect Rust eviction with native release. | Revisiting evicted regions costs reshaping CPU; final appearance is unchanged. |
| Persistent height tree | Shares unchanged paths during atomic layout staging instead of deep-cloning every learned height. | None. |
| Coarsen old exact heights into budgeted block summaries | Bounds height metadata even after traversing varied documents. Keep visible rows exact and preserve their text anchors. | Scrollbar thumb size/position and distant drag targets can refine. The visible editing row should remain in place. |
| Horizontal virtualization for unwrapped enormous lines | Keep exact horizontal viewport/overscan plus advance, tab, bidi and shaping checkpoints. Avoid retaining offscreen cluster geometry for an entire row. | Horizontal extent may refine; a first far horizontal jump can take longer. Hit testing and caret geometry must be exact before interaction. |
| Share width-independent shaping across panes | Reduces duplicate font/cluster storage; wrapping remains per-view. Requires resolving native provider ownership. | Usually none; panes may evict one another's offscreen cache under a shared budget. |

For uniform monospaced plain text, exact unwrapped vertical height can often
come from line count without shaping all lines. Proportional fonts, fallback,
bidi, variable metrics and wrapping require the general estimate/refinement
path. A fixed-cell fast path must only run when its preconditions hold.

**Code: budget the parse, not just the colored output**

Code input shares the persistent formatted text tree. Syntax presentation is
already regional and bounded: 256 KiB requests, 16 cached regions, a 4 MiB run
budget, and two workers ([service.rs](../src/core/document/syntax/service.rs),
line 16). Previously accepted appearance is retained while new work is pending;
new uncached text may temporarily use defaults.

Tree-sitter still parses the whole host document, then queries regional output.
Production has cooperative native soft limits of 256 MiB per session and 1 GiB
process-wide; they are not a ceiling for total editor RAM. The Vim scanner has
bounded retained state/checkpoints. Idle provider retention is count-based
(eight sessions), which is a poor proxy for actual memory.

Previous diagnostic Tree-sitter runs recorded about 522 MiB native peak for an
8.27 MiB C fixture and 718 MiB for an 8.58 MiB Swift fixture. These are separate
existing measurements, with a 768 MiB diagnostic allowance and production edit
preflight disabled, **not new production-RSS measurements**. See
[performance-baseline.json](../src/core/document/syntax/treesitter/performance-baseline.json)
and [performance.rs](../src/core/document/syntax/treesitter/performance.rs), line 9.
They show why AST cost may become the next major multiplier after document
compaction. Production already falls back when repair is too expensive, as the
[production-policy baseline](../src/core/document/syntax/treesitter/production-policy-baseline.json)
demonstrates.

| Code option | Benefit | Small side effects |
| --- | --- | --- |
| Evict idle syntax sessions by retained bytes, and discard unusable over-budget trees | Releases expensive cold ASTs instead of keeping up to eight regardless of size. Budget active, pending and retained work together. | Returning to a buffer may briefly show retained/default colors while it reparses. |
| Select Tree-sitter versus checkpointed Vim scanning using resource estimates | Avoids starting predictably over-budget whole-file parses. Use grammar-specific observations and bounded sampling, not one universal file-size cutoff. | Highlighting can be less precise or different on large files; multiline context may be provisional. Font-changing syntax styles can refine wrapping. |
| Smaller nearby prefetch with sparse distant checkpoints | Keeps work focused around all visible views. | Fast distant scrolling can outrun highlighting; sequential reading benefits from prefetch. |
| Intern names/origins and share packed immutable syntax runs | Reduces duplicate strings and result copies within the bounded presentation cache. | None. |
| Partition parse trees by proven language regions, later | Potentially makes trees evictable by region. Requires language-specific context/dependency rules. | Arbitrary approximate viewport parsing can misread strings, comments and preprocessor state. This is substantially riskier than existing lexical fallback. |

Code syntax must never control source bytes, logical grapheme boundaries,
registers or undo. Delayed color alone has no geometric effect; delayed font,
weight, size or other metric changes may move wrap points. Preserve viewport
and caret anchors during that refinement rather than silently restricting the
existing typography feature set.

**Later choices: progressive loading, backing files, rich formats**

After exact compaction, a more ambitious literal path can retain a compact
source/line index and decode/project only needed immutable chunks. This helps
legacy encodings and files much larger than available memory. Distant jumps,
first searches or whole-document commands may wait for cold chunks. If line
indexing is also progressive, total counts and scroll extents may initially be
provisional. Prefer keeping cheap logical counts exact while making expensive
geometry lazy.

Opening policy matters: BOM-less encoding detection currently checks whole-file
UTF-8 validity before falling back to Latin-1. Progressive display must not
silently reinterpret already edited content after encountering a late invalid
byte. A streaming validity/index pass with compact storage is a safer first
milestone than changing detection policy.

Disk-backed immutable chunks or a mapped private backing store can bound
resident source/history data further. Side effects are occasional page-in
latency, extra disk use and I/O. Merely mapping a mutable user file does not
provide the required immutable source snapshot when an external process changes
or truncates it. Preserve a stable backing artifact and recovery semantics.
Disk backing does not cure enormous eager metadata and should come later.

Markdown/HTML benefit from compact decoder mappings, ordinary-text provenance
runs, shared style IDs and bounded layout too. Preserve exceptional entities,
hidden markup and source runs. A second stage can lazily materialize formatted
blocks from a compact lossless syntax index, carrying parser checkpoints and
declared global dependencies. Cold areas may appear later and scroll estimates
can refine; reference definitions, CSS and open containers can invalidate distant
content. Edits must wait for exact local structure/provenance rather than
operate on an approximate interpretation. This is more complex than literal
Text/Code and should follow them.

**Lessons from other editors**

- Vim packs line data and indexes into blocks under a tree of pointer blocks.
  Its memory-file layer can release cold unlocked blocks to a backing file.
  Borrow compact paged storage and explicit cache ownership, while retaining
  Viem's Unicode/provenance model. Vim itself permits an oversized block for a
  huge line; do not copy that aspect as a solution to Viem's giant-line problem.
  Sources: [Vim 9.2 memline](https://raw.githubusercontent.com/vim/vim/v9.2.0000/src/memline.c),
  [Vim memfile](https://raw.githubusercontent.com/vim/vim/master/src/memfile.c).
- Neovim demonstrates the distinction between regional presentation and parse
  storage: its LanguageTree contract always parses empty-range regions, usually
  the host root, while requested ranges constrain other regions such as
  injections. Viewport highlighting therefore does not establish a
  viewport-sized AST. [Neovim LanguageTree documentation](https://neovim.io/doc/user/treesitter/#LanguageTree%3Aparse()).
- VS Code's official text-buffer redesign describes a 35 MB file with 13.7
  million lines consuming roughly 600 MB because of per-line objects. Its
  piece tree shares immutable buffers and line metadata. The close parallel is
  Viem's default block overhead. Viem already has balanced persistent storage;
  changing the tree name alone would leave its per-character mappings intact.
  [VS Code text-buffer reimplementation](https://code.visualstudio.com/blogs/2018/03/23/text-buffer-reimplementation).

**Recommended order and acceptance evidence**

1. Compact the literal mapping and construction pipeline; give Text the same
   incremental line-topology capabilities as Code; bound huge-line edits by
   bytes. Pack default block/style metadata in this track so the million-line
   fixture also improves. These changes should preserve appearance and behavior.
2. Remove full-copy command/hash/undo paths, improve backing allocation
   granularity, and make useful default-policy undo an explicit acceptance gate.
3. Bound all layout/native layers, share immutable shaping, and fix unwrapped
   giant-line virtualization. Use coarser offscreen heights if their metadata
   still grows too far; accept scrollbar refinement while preserving anchors.
4. Apply a byte-based Code provider/session policy, favoring bounded scanning
   when whole-file parsing will not fit. Retain the existing visible-style
   continuity rules.
5. Consider progressive decoded projections or disk-backed chunks only if the
   resulting measurements still warrant them; adapt rich formats afterward.

Add per-component current/peak allocation counters or allocation-stack profiles
for source, decoding, mappings, reverse indexes, blocks, ledger, history, search,
layout, native resources and ASTs. Re-run separate processes at increasing file
sizes, including the existing 100 MiB UTF-8/UTF-16/Latin-1 fixtures, malformed
encodings, mixed endings, million-short-lines and multi-megabyte-line fixtures.

Measure open, first native display, Enter/join, local and disjoint edits, search,
save, undo/redo branches, large deletion and subsequent pruning, full varied-text
scrolling, repeated wrap/zoom/resize, and one/two/four panes. Verify reclamation
after closing panes/jobs/snapshots. Use structural work bounds and randomized
mapping/reverse-edit oracles alongside latency percentiles. Preserve byte-exact
no-op saves, patch locality, source encoding, valid snapshot identities,
grapheme-safe editing, and the existing layout-invalidation fixtures.

Reproduce one probe case with:

```sh
cargo build --release --offline --example large_file_memory
target/release/examples/large_file_memory plain lines 4194304 document
```

The JSON report records all case arguments. Use a separate invocation for each
case. `views` uses mock shaping with syntax disabled, `flat` reads `text()` after
the first edit, and `open` skips editing. The probe is intentionally smaller
than the complete acceptance suite described above.

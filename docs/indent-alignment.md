# Align indents to previous line

The Format menu offers **Align indents to previous line** immediately after
**Word wrap**. The item is enabled only in Code and appears unchecked whenever
it is disabled, retaining the view's setting for when Code is shown again.
It defaults on and applies only in Code with a proportionally spaced font.
The native provider reports the resolved font's fixed-pitch trait;
font names and a few measured sample characters are not reliable substitutes.
The setting belongs to the view. It changes presentation without changing
source, logical columns, selections, registers, dirty state or undo history.

For each indented line, its anchor is the nearest preceding nonblank line whose
leading ASCII space/tab indentation is no greater. A blank line is empty or
contains only spaces and tabs; alignment skips back across any run of them. Indentation is compared in portable
columns using the view's tabstop. The first body character is placed at that
column's native caret position on the anchor. Body columns count grapheme display
cells, including wide characters; tabs advance to the configured tabstop. For
example, six leading spaces
can align a continuation beneath the `a` in `  if (a`, even though the font's
spaces, letters and punctuation have different advances. When the target lies
beyond the anchor's body, the remaining columns use ordinary indentation units.

## Persistent index

`src/core/layout/indent_anchor_index.rs` contains the independent index. It is an
implicit AVL sequence: subtrees store line counts, height, block count and the
minimum eligible indentation. No node stores an absolute line number or an
explicit parent/dependent list. A predecessor search prunes subtrees whose
minimum is greater than the target and searches the remainder from right to
left. Changing indentation can therefore reparent an arbitrarily large suffix
without rewriting any of its entries.

Leaves pack at most 128 `usize` column counts and 128 eligibility bits. Splice
boundaries merge or redistribute underfilled leaves. Interior blocks retain at
least 64 entries, so repeated edits cannot create one allocation per line.
Snapshots share immutable nodes and copy only changed paths and boundary blocks.

| Operation | Work |
| --- | --- |
| Clone an index | O(1) |
| Look up a line or its anchor | O(log n + 128) |
| Find a preceding/following indentation threshold | O(log n + 128) |
| Replace a range with m new summaries | O(log n + m + 128) construction; releasing unretained deleted blocks is proportional to those blocks |
| Find a transitive dependency span | O(log n + 128) |
| Enumerate k immediate dependents | O((k + 1 + b)(log n + 128)), where b counts intervening excluded lines with smaller indentation |

Here n is the number of indexed lines. A line's transitive dependents extend
until the next eligible line with *strictly smaller* indentation. Equal-indent
lines remain linked in the logical forest. Immediate children are successive
decreasing indentation minima; threshold searches skip their intervening
subtrees. The lazy iterator supports stopping after the visible dependents.

Each line carries a separate eligibility policy bit. Blank lines are
ineligible; every other line is eligible, including comments and preprocessor
lines. Exclusion changes neither the sequence's ordinals nor its query
interface. A blank line still has its own anchor, so its indentation aligns
like any other line, but no line anchors to it. The transitive span can
therefore contain nondependent holes, and immediate-child enumeration can
additionally visit excluded low-indent lines. Synchronization takes indentation
minima over all changed lines, including blank ones, which keeps invalidation
conservative when an edit makes a line blank or nonblank. No comment or
preprocessor classification is implemented here.

The million-line structural test bounds the initial tree's requested node and
column payload below 11 bytes per line. That accounting excludes allocator
headers and Arc control blocks; it is not a measurement of process memory.
Retained snapshots share payload until edits diverge. Tests also bound tree
height, nodes visited/copied, column entries copied, and fragmentation after
10,000 edits; they do not substitute timing for structural bounds.

## Snapshot synchronization

`IndentSnapshot` adds the semantic projection and tabstop to the index. It
indexes only the prefix needed by requested geometry, extending in cancellable
batches of 128 lines. Displaying an initial viewport does not index an unrelated
suffix. The first request for a distant line must establish the intervening
summaries in the layout computation; subsequent requests reuse them. A long whitespace
prefix is scanned through borrowed rope chunks with cancellation checkpoints.

On an edit, persistent text-tree comparison skips shared subtrees and returns a
conservative changed extent. Semantic presentation-line queries expand that
extent to old/new line hulls. A local hull splices new indentation summaries
while retaining the unchanged suffix. Foreground synchronization examines at
most 128 old/new lines and 4,096 prefix bytes across the whole change. An edit
crossing the indexed prefix, a broad old or new line hull, or a prefix scan that
exhausts that shared budget truncates to the proven prefix and lets cancellable
layout work extend it. The old-hull limit is checked before computing minima;
a broad syntax metric range conservatively invalidates through EOF instead of
enumerating every indexed line. Tabstop changes reset the index.

These are synchronization bounds, not a total input-latency bound. Immediate
visible layout can run synchronously in the coordinator. It must finish an
uncached required prefix, and a first distant lookup must build the intervening
summaries. Those paths have cancellation checkpoints but can still take work
proportional to the required prefix bytes and unindexed preceding lines.

Literal LF bytes are not a substitute for semantic line boundaries: forced Mac
line endings can expose an LF as ordinary content. An O(1) identity check of
the presentation-line storage prevents reuse after a semantic partition change,
even when rendered bytes and line counts are equal. Composition uses an
immutable sparse overlay with its own temporary text and line boundaries.

Invalidation examines both old and new dependencies. A known smaller-indent
barrier bounds the affected suffix; without one, invalidation reaches EOF,
including work another candidate may already have captured. Zero-indent lines
have invariant x=0, so a body edit's geometric dependency can stop at the next
zero-indent line despite the equal-indent edge in the logical forest.
A proven single-line body edit with an unchanged leading prefix can similarly
stop at the next equal-indent line, whose own indentation x remains unchanged.
The coordinator applies that already expanded range directly; only raw syntax
metric changes request an additional dependency expansion.

At most 256 cached prefix endpoints/column counts avoid repeatedly scanning
giant indents across fragments and jobs. Edits preserve or rebase an entry only
when the unchanged prefix/suffix proves its coordinates and contents valid.
This cache is disposable and does not determine anchor correctness.
The scan records whether a prefix contains tabs explicitly. A cached all-space
prefix can absorb a bounded whitespace edit through arithmetic on the changed
extent, including ordinary space typing, Backspace and one inserted tab. Its
provenance then records that tab; equal byte and column counts never imply an
all-space prefix. Replacing the first body character with an ASCII body
character also preserves the existing prefix proof, and makes a cached blank
line nonblank. These common local cases
avoid a giant-prefix rescan during the next immediate layout. Cold prefixes and
edits requiring an uncached/tabbed prefix still have the layout cost noted above.

## Native geometry

Geometry resolves indentation lazily. Equal-indent chains have the same x, so a
strictly-smaller predecessor query skips the chain in one step. Remaining
ancestors are processed iteratively with cancellation. Bounded disposable width
caches avoid repeated work within a layout computation.

An offscreen left-to-right anchor shapes only the prefix needed to reach the
target column, with the same surrounding text, styles, native caret stops,
fallback and ligatures used by ordinary shaping. Bidi text can have a larger
directional dependency; it uses the existing streamed unwrapped layout to obtain
the authoritative caret position. Tabs and wide graphemes can contain
the target column; interpolation stays within that logical cell. Leading
whitespace clusters and their caret stops receive the resulting width together,
keeping hit testing and invisible-character markers consistent. Long-prefix
continuations resume column scanning without flattening the whole prefix.

The isolated tests live in `tests/all/indent_anchor_index.rs`; private snapshot
tests live in `src/core/layout/indent_snapshot_tests.rs`. Native tests and
coordinator integration tests cover the setting and resulting geometry. Native
latency requires the normal native measurement workflow; structural and mock
provider tests alone do not establish it.

## Measurements and validation

On 2026-10-22, the release `indent_anchor_probe` ran in separate fresh processes
on this arm64 macOS host, with no concurrent builds or test runs. Its counting
allocator includes requested Arc headers and node/column allocations, but not
allocator metadata, RSS, source storage or native geometry. The initial fixture
has a zero-indent root followed by equal eight-column indents. Reindent keeps an
old snapshot alive; insertion and deletion change three lines in the middle.

| Lines | Retained requested bytes | Build | Root reindent | Insert three | Delete three |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1,000 | 9,440 | 0.019 ms | 5.13 µs | 5.50 µs | 2.63 µs |
| 100,000 | 950,048 | 0.405 ms | 5.63 µs | 6.46 µs | 4.67 µs |
| 1,000,000 | 9,500,000 | 3.911 ms | 5.46 µs | 11.67 µs | 5.17 µs |

The million-line tree has height 14 and 7,813 blocks. Root reindent copied 257
column entries and created 39 nodes, retaining 2,272 additional bytes with the
old snapshot alive. The 20,000 anchor/threshold queries allocated nothing and
took 4.73 ms total. Enumerating 16,383 immediate dependents in a separate
descending-indent fixture allocated nothing and took 0.63 ms. Snapshot clones
allocated nothing at every size; releasing all indexes returned to the initial
allocation baseline. These single-run timings are observations, not latency
contracts; the deterministic tests enforce structural work bounds instead.

Reproduce the isolated probe under the shared build owner:

```sh
python3 scripts/validation_support.py -- cargo run --release --offline --example indent_anchor_probe -- 1000000
```

The measured evidence is in
`target/validation/20261022-104851-indent-measure/`. The complete Rust suites,
macOS native build/ABI/tests, and standard, supplemental and rich-paste memory
limits passed in `target/validation/20261022-104059-209191/`. Windows generated
bindings passed their consistency check; Windows native execution still needs a
Windows host.

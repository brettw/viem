# Viem product and engineering specification

This file is the normative product and architecture specification for this
repository. It also gives implementation guidance to coding agents. In this
document, **MUST**, **SHOULD**, and **MAY** have their usual requirements
meanings.

## Product intent

Viem is a modal, keyboard-first text editor for human-language writing. It
keeps the composable command model and editing feel of gVim while providing a
word-processor-quality text surface:

- proportional or monospaced fonts;
- kerning, ligatures, font fallback, and OpenType shaping;
- different font families, sizes, and attributes in different text spans;
- lossless, format-aware projections from source such as plain text, Markdown,
  HTML, or RTF into an editable formatted view;
- optional soft word wrapping to the current window width;
- visual-row navigation when wrapping is enabled; and
- responsive editing and scrolling in large documents.

The goal is not source compatibility with Vim, a Vimscript runtime, or a
pixel-for-pixel gVim clone. "Vim compatible" in this project means that every
command explicitly listed in this file follows Vim's command grammar and
observable editing semantics, except where this file defines a deliberate
word-processing behavior.

The first frontend is macOS. The core must remain suitable for later Windows
and terminal frontends, but those frontends are not part of the current scope.

## Working rules for this repository

- Put portable document-state logic under `src/core/document`, Vim command
  interpretation under `src/core/command`, and layout logic under
  `src/core/layout`. The `src/core` crate root is their composition boundary and
  public facade; it must not become an unstructured alternative location for
  their implementations.
- Put AppKit, Core Text, macOS input, drawing, clipboard, accessibility, and
  application lifecycle code under `src/mac`.
- `src/core` MUST NOT import AppKit, Core Text, Metal, or other platform UI
  frameworks. Platform services are injected through narrow interfaces.
- Do not make a feature macOS-only merely because macOS is the first frontend.
  Keep the policy and state portable; keep the mechanism in `src/mac`.
- Do not claim support for a Vim command until its counts, registers,
  operator-pending form, undo grouping, and relevant edge cases have tests.
- New layout work MUST include cache-invalidation tests and a large-document
  test. A correct full-document relayout on every edit is not acceptable.
- Product behavior in this file takes precedence over Vim behavior where the
  two differ. Otherwise, use Vim 9.2 documentation as the behavioral reference.
- Keep this file current when a requirement or supported-command decision
  changes.

## Implementation languages

- Implement the portable core under `src/core` in Rust.
- Implement the macOS frontend under `src/mac` in Swift using AppKit, not
  SwiftUI.
- A future native Windows frontend will use C# and WinUI 3.
- Connect frontends to the Rust core through a narrow C ABI. Keep
  ownership explicit and make performance-sensitive exchanges batch-oriented.

### API evolution

All consumers of Viem's Rust APIs and C ABI are in this repository. API and ABI
backward compatibility is **not required**. Implementations MAY remove or change
functions, types, layouts, and versioned entry points whenever this simplifies
the design, provided all in-repository callers, providers, declarations, and
tests are updated together. Do not retain adapters, aliases, or old versions
solely for compatibility with earlier builds. Core and frontend artifacts must
be rebuilt together after an ABI change.

This policy does not relax ownership, pointer validation, snapshot identity,
threading, or atomicity requirements. It also does not authorize changes to
persisted document formats or user-visible editing behavior; those remain
governed by their explicit requirements below.

### macOS build and packaging

The native macOS frontend and its unit tests use the repository-root Swift
Package Manager manifest. Its targets mirror the `src/mac` ownership boundaries
for the C bridge, Core Text provider, editor, AppKit shell, and executable.
`scripts/build-mac-app.sh` first builds the Rust static library, then the Swift
targets, and finally assembles and ad-hoc signs `.build/Viem.app` for local
development and end-to-end testing. A future distribution/archive workflow MAY
add an Xcode project without changing those source ownership boundaries.

The root Makefile exposes `make debug` (the default), `make release`, and
`make clean`. Build targets delegate to the packaging script. `make run-debug`
and `make run-release` build the corresponding configuration and launch
`.build/Viem.app`; `make run` aliases `make run-release`. Cleaning removes the
repository-local `.build` and `target` directories, including path-dependent
Swift/Clang module caches that must be discarded after renaming the checkout.

The initial deployment target is macOS 26.0 so the frontend can use the current
`NSTextInsertionIndicator` API without a second caret implementation.

## Vocabulary and coordinate systems

Use these terms consistently in code, tests, and documentation:

- **Source artifact**: the authoritative persisted representation: one or more
  ordered/named byte sequences plus the container, format, and encoding
  metadata needed to reproduce its physical serialization. It need not be a
  flat string or a single file.
- **Source snapshot**: an immutable revision of a source artifact.
- **Lossless syntax model**: a format-specific structural view that retains
  every source byte, including delimiters, whitespace, comments, unknown
  constructs, and malformed input.
- **Formatted document**: a derived, normalized UTF-8 model containing logical
  blocks, text, style spans, objects, and provenance. It is editable through
  semantic intentions but is never an independent persistence authority.
- **Paragraph**: a paragraph-bearing logical block and the unit to which one
  paragraph style is assigned. Depending on the format, it may contain one or
  more formatted hard lines or may itself define the hard-line boundary.
- **Style sheet**: the immutable normalized collection of block and character
  style definitions associated with one formatted snapshot. Document and
  paragraph styles are roles within the block-style namespace.
- **Direct formatting**: sparse block or character property declarations
  attached to content after named-style assignment; it does not mutate the
  named style.
- **Document**: a source artifact, its configured transformation pipeline, and
  cached derived projections.
- **Buffer**: an editing session for a document plus undo history, registers,
  marks, and buffer-local state. A buffer can be displayed by multiple views.
- **View**: a presentation of a formatted document with its own viewport, wrap
  width, layout cache, scroll position, cursor, selection, and view options.
- **Hard line**: a logical line boundary in the formatted document. A format
  transformation determines how it maps to source constructs. Soft wrapping
  never changes hard lines or source bytes.
- **Visual row**: one displayed fragment of a hard line after wrapping. With
  wrapping off, a hard line has one visual row.
- **Snapshot point**: an exact boundary in one explicitly identified immutable
  source or projection snapshot. It is lightweight and becomes stale rather
  than silently moving when a new revision is committed.
- **Persistent anchor**: a stable identity plus local boundary, insertion
  association, recovery policy, and provenance used for state that must survive
  edits and reprojection.
- **Text point**: a snapshot point at a legal logical boundary in formatted
  content. Public editing APIs use text points and ranges, never pixels.
- **Caret point**: a layout-snapshot-specific visual realization of a text point
  and boundary affinity at a legal shaping caret stop.
- **Association**: whether an anchor remains before or moves after content
  inserted exactly at its boundary. It controls edit remapping, not geometry.
- **Boundary affinity**: the upstream or downstream association of a logical
  text point with preceding or following content. It selects side-dependent
  context such as typing-style inheritance and, at a soft-wrap, bidirectional,
  or other split-caret boundary, the visual side. It does not control edit
  remapping.
- **Caret stop**: a legal visual insertion or navigation position returned by
  shaping. It may not correspond one-to-one with a byte, Unicode scalar, or
  glyph.
- **Desired x**: the horizontal position, in layout units, retained while the
  cursor moves vertically through visual rows of unequal fonts and widths.
- **Layout units**: platform-independent floating-point units. The macOS
  frontend maps them to device-independent points and backing pixels.
- **Projection snapshot**: an immutable formatted result identified by source
  revision, pipeline configuration, and transformation generations.
- **Provenance map**: the bidirectional relation between ranges/nodes in two
  adjacent transformation stages, including insertion association, boundary
  affinity where applicable, and synthetic content.
- **Metrics generation**: an identity for the current font resolver and text
  measurement environment. It changes when results may differ.

The formatted document is expressed in valid UTF-8. Logical user edits,
selections, and motion ranges MUST NOT split an extended grapheme cluster.
Visual cursor placement additionally obeys shaping caret stops. Source byte
positions and formatted text positions are distinct types and must never be
confused. Naked numeric offsets MUST NOT be retained across edits as persistent
marks, selections, or cache keys; use anchors with stable source/projected
identities and explicit association. A local offset paired with an immutable
piece/leaf identity and revision is part of an anchor, not a naked document
offset.

## Position and range algebra

The core uses exact snapshot-bound points for immediate computation and
persistent anchors for state that survives a transaction. Requiring every
temporary operation to allocate a persistent anchor is unnecessary; retaining a
snapshot point after its snapshot is no longer current is an error.

### Coordinate domains and snapshot points

The following are distinct nominal types and have no implicit conversions:

- `SourcePoint`: a byte boundary in one named source-artifact part and source
  snapshot;
- `DecodedPoint`: a boundary in one encoding projection;
- stage-specific syntax/projection points used internally between transforms;
- `TextPoint`: a logical formatted-content boundary in one projection snapshot;
- `CaretPoint`: a visual realization of a `TextPoint` in one layout snapshot;
  and
- `LayoutPoint`: an x/y location in layout units.

Conceptually, snapshot points have this shape; concrete tree-specific spelling
may differ:

```text
SnapshotPoint {
    document identity,
    snapshot or projection identity,
    stable leaf/part identity,
    leaf/part revision,
    validated local boundary
}
```

A source point's local boundary is a byte boundary; encoding and format stages
are responsible for accepting or rejecting a patch that would split a construct
they require to remain indivisible. A text point's local boundary is either an
extended-grapheme boundary in a text leaf or a boundary before/after an atomic
formatted item.

Points from different documents, domains, or revisions cannot be ordered,
subtracted, combined into a range, or passed to an API expecting the other
domain. Such operations return a structured `WrongDocument`, `WrongDomain`, or
`WrongSnapshot` result. APIs never interpret an old numeric position in the
current snapshot and never silently clamp an invalid boundary.

Document-wide ordinal byte, grapheme, hard-line, or block indexes may be derived
temporarily through tree aggregates. They are snapshot-local values for
algorithms and external reporting, not persistent position identities.

### Logical formatted boundary space

The formatted block/text tree defines one total logical document order. Its
conceptual atomic content items are:

- extended grapheme clusters stored compactly in text leaves;
- explicit hard-line boundary items; and
- atomic inline/embedded objects when those are supported.

Positions exist between items, never inside a grapheme cluster, hard-line
boundary, or atomic object. This is a conceptual algebra: implementations do not
create one allocation or tree node per grapheme.

Each hard-line boundary contributes one normalized U+000A to APIs that require
a flat logical UTF-8 view, including search, registers, and regular-expression
matching. The block tree still distinguishes a hard break inside a paragraph
from a boundary that also separates paragraphs. There is no implicit newline at
the end of the document. Source line-ending spelling remains the responsibility
of the transformation pipeline.

An object has positions before and after it and no public interior position. Its
plain-text register/search representation is an explicit property of the object
kind rather than an invented source character.

### Persistent anchors and mapping

Marks, cursors, selection endpoints, jumps, viewport anchors, undo restoration
positions, and long-lived cache dependencies use persistent anchors. A formatted
anchor conceptually contains:

```text
TextAnchor {
    document identity,
    stable projected identity and local validated boundary,
    BeforeInsertion | AfterInsertion association,
    Upstream | Downstream boundary affinity,
    deletion recovery policy,
    recoverable source provenance
}
```

A source anchor uses a stable source-piece/part identity and local byte boundary
with the same association and an applicable recovery policy. The local boundary
is meaningful only together with the immutable piece/leaf identity and revision.

Insertion association and boundary affinity are orthogonal:

- `BeforeInsertion` remains before text inserted exactly at the anchor;
- `AfterInsertion` moves after text inserted exactly at the anchor;
- `Upstream` chooses the visual side associated with logically preceding
  content at a split caret; and
- `Downstream` chooses the side associated with logically following content.

Insertion association never selects adjacent semantic context, a visual row, or
bidi caret. Boundary affinity never decides whether an anchor moves across an
insertion.

Default associations are semantic rather than universal:

- a typing caret is `AfterInsertion` for its own insertion;
- normalized selection boundaries associate inward—the lower boundary after an
  insertion at that boundary and the upper boundary before it—so unrelated
  boundary insertions are not silently selected;
- a named mark or viewport anchor remains associated with its following content
  when possible and otherwise with its preceding content; and
- commands and history records may specify another association explicitly when
  required by Vim semantics.

Every committed source transaction and incremental projection produces a
forward change map from each changed domain's prior snapshot to its new
snapshot. The identity map changes nothing. Two maps compose only when they have
the same document and domain and the first map's target snapshot is the second
map's source snapshot; composition is associative. A projection additionally
provides the cross-domain provenance relation described below.

A change map is not presumed invertible or globally monotonic: deletion loses
identity, and an explicit move may preserve stable content identities while
changing their order. Mapping a persistent anchor is the primitive operation.
Mapping a range is therefore not generally equivalent to mapping only its two
endpoints; it returns an ordered `RangeSet` or a structured ambiguous or
unresolvable result. Move-aware maps preserve the identity of moved content.
Undo and redo use recorded states and maps rather than synthesizing an inverse.

For an ordinary splice, an anchor before it remains unchanged; one after it
follows the structural shift; one at a pure insertion uses its association. An
anchor whose associated content is deleted collapses to the deletion boundary,
preferring following content and then preceding content when the document ends.
An empty hard line supplies an explicit recoverable boundary.

Resolving an anchor against a snapshot returns a status, not only a point:

```text
Exact(point)
Moved(point)
CollapsedByDeletion(point)
RecoveredFromProvenance(point)
Ambiguous(candidates)
Unresolvable(reason)
```

Stable projected identity is tried first after reprojection and composed source
provenance is the fallback. Ambiguous or unresolvable anchors are never guessed
for a source-changing operation. Presentation state may use its declared
recovery policy and surface a diagnostic when exact recovery is impossible.

Mapping may be implemented lazily through persistent tree structure and
composable change maps. Committing an edit MUST NOT walk every later anchor or
shift a flat list of document offsets. Undo and redo restore their recorded
anchors from history state rather than attempting to invert arbitrary current
anchor mappings.

### Ranges, range sets, and selections

All internal source and formatted ranges are ordered and half-open:

```text
Range { start, end } // start is included; end is excluded
```

A range's endpoints must have the same document, domain, and snapshot identity;
`start` must not follow `end`; and both endpoints must be legal boundaries.
Empty ranges are valid. Range construction validates these invariants and
returns an error rather than swapping, snapping, or clamping endpoints.

A source patch range is additionally confined to one source-artifact part. A
multi-part or discontiguous source edit uses an ordered patch/range set. A
formatted `RangeSet` contains sorted, non-overlapping segments. Adjacent
segments may remain distinct when their identities carry row or block semantics;
ordinary set normalization may coalesce them only when that distinction is not
observable.

A character selection preserves direction as two anchors. Resolving it against
an explicit target snapshot, then normalizing it, produces an ordered half-open
range without discarding which endpoint is active:

```text
DirectedSelection { anchor, active }
```

Visual Line stores a stable ordered hard-line span rather than a character
range. Visual Block stores top/bottom `TextAnchor` values whose boundary
affinities identify the endpoint rows, plus left/right layout x coordinates.
Resolving it against an exact layout snapshot hit-tests each visual row and
produces a tagged `RangeSet`.

Motion results retain their semantic kind:

```text
Characterwise { selection: DirectedSelection,
                endpoint: Inclusive | Exclusive }
HardLinewise(HardLineSpan)
VisualRowCharacterwise { selection: DirectedSelection,
                         endpoint: Inclusive | Exclusive }
VisualBlock(BlockSelection or resolved RangeSet)
```

Endpoint policy belongs to the resolved motion, not the operator. A Vim-
inclusive endpoint is converted exactly once when a motion or Visual selection
is resolved for an operator: the associated final atomic content item is
included and the result becomes half-open. An exclusive endpoint is already a
boundary and is not advanced. Operators consume typed half-open extents and do
not independently adjust endpoints. A multi-range block edit is one atomic
semantic intention and source transaction.

The algebra exposes explicit checked operations such as compare, normalize a
directed selection, intersection, union, subtraction, advance by grapheme,
hard-line start/end, and rebase through a named position map. It does not expose
unchecked integer arithmetic on public point types.

### Cursor representation

Every view cursor is a persistent text anchor. Its resolved `TextPoint` is a
boundary and its boundary affinity associates an adjacent atomic content item when
a mode needs a character-shaped cursor:

- At an ordinary cluster start, downstream affinity associates the following
  grapheme.
- At hard-line end, upstream affinity associates the preceding grapheme.
- An empty hard line has a boundary with explicit empty-line caret and block
  geometry rather than a fictitious character.

Normal mode draws its block around the associated grapheme/object. Insert mode
draws a thin caret at the boundary itself. Replace mode draws beneath the
associated replaceable item. Thus `0` resolves to line start with downstream
affinity, `$` resolves to line end with upstream affinity, `i` inserts before
the associated item, and `a` inserts after it without maintaining a separate
character-index coordinate system. On an empty line or empty document, both
commands insert at the sole legal boundary.

At a soft-wrap or bidi boundary, affinity also selects the correct visual row
and caret side. Visual horizontal movement and hit testing operate on layout
caret stops and return a new text point plus affinity. Desired x remains
separate view state used only for vertical movement.

### Grapheme boundaries and shaping caret stops

Logical editing validity is based on portable extended-grapheme boundaries.
Font choice, fallback, OpenType shaping, view width, and platform shaper behavior
MUST NOT change the text deleted, yanked, searched, case-converted, or placed in
a register by a logical command.

A `CaretPoint` is a `TextPoint` plus boundary affinity and a legal caret-stop
identity in one exact layout snapshot. The provider should expose a caret stop
at each grapheme boundary it can represent, but it may report a stricter
indivisible shaping cluster. Visual movement and pointer hit testing then skip
the unavailable interior stop. A logical command may still use an extended-
grapheme endpoint without expanding its edit to a font-dependent shaping
cluster; after the edit, layout reshapes the result. If a logical endpoint has
no independent caret geometry, selection/highlight drawing uses the containing
cluster geometry without changing the logical range.

Caret points cannot be retained after their layout snapshot becomes stale. The
underlying text anchor is retained and resolved against the new exact layout.

### Relational provenance and reverse edits

Mapping a formatted point or range to source is a relation, never an assumed
single offset or contiguous range. A provenance query returns a structured
result equivalent to:

```text
Exact(SourceRangeSet)
Synthetic
Ambiguous(candidate SourceRangeSets)
PartiallyMapped { mapped ranges, formatted gaps }
```

Reverse-edit translation consumes that result and either produces an explicit
minimal patch set or returns unsupported, ambiguous, stale, or needs-policy.
Synthetic or partially mapped content is not silently dropped from an edit.

Ordinary editing uses one shared minimal-source rule: a caret names the visible
insertion location within its hard line or paragraph, including an empty body;
hidden opening/closing syntax does not create competing editing locations.
Line ownership selects the context at line edges, and boundary affinity selects
adjacent inline context within a line. A nonempty text edit consumes the smallest
complete set of contributing source runs while retaining intervening hidden
syntax. If one indivisible source entity represents several editable characters,
rewrite that entity while preserving its unselected prefix and suffix. This
source expansion does not expand the logical selection or its position map.
Independent logical edits that share an indivisible source contributor rewrite
that contributor once while retaining their separate logical change ranges.
Text edits, formatted payloads, clipboard replacement, and character formatting
share this resolver; text and payload translation share the encoding path.
Structural breaks and list/paragraph ownership use their adapter's semantic
operation, followed by normal candidate verification. At a valid rich-text caret,
an insertion must find the equivalent editable position in the containing
paragraph, normally the innermost applicable inline scope. Hidden syntax must
not make that position unusable. Atomic objects expose explicit before/after
positions and may be deleted or replaced only as complete source contributors.
Stale identities and incomplete provenance are not resolved by deleting the
nearest source hull. User-facing errors describe the unsupported
edit or format constraint; they never expose an "unambiguous source range" error.

### Complexity and API requirements

Resolving or comparing points and finding a hard-line boundary in one snapshot
are `O(log n)` apart from required local Unicode boundary work. Rebasing through
`k` uncompacted position maps may additionally cost `O(k)`; implementations
MUST compact or checkpoint map chains so repeatedly resolving long-lived anchors
has amortized logarithmic cost rather than growing without bound. Batch APIs
resolve related anchors/ranges against one snapshot traversal for Visual Block,
multi-cursor-like internal operations, accessibility, and multi-view remapping.

Public and C ABI operations carry domain and revision identities explicitly or
through validated opaque handles. Required structured failures include wrong
document/domain/snapshot, invalid Unicode boundary, inverted range, stale
layout, ambiguous provenance, synthetic/read-only content, and unresolvable
anchor. Callers must request an explicit rebase; APIs never substitute the
current revision automatically.

## Source authority and transformation pipeline

The ground truth of every document is its source artifact. Formatted content
and layout are disposable projections that can be regenerated on demand. The
required conceptual pipeline is:

```text
SourceArtifact (original bytes or package parts)
    -> EncodingProjection (valid UTF-8 plus byte provenance)
    -> optional TextLineEndingProjection (logical breaks plus byte provenance)
    -> LosslessFormatProjection (format syntax plus semantic provenance)
    -> zero or more SemanticTransformations
    -> FormattedDocument (UTF-8 text, blocks, objects, and style spans)
    -> LayoutSnapshot (shaping, wrapping, hit testing, and geometry)
```

The implementation MAY fuse stages for efficiency, but their responsibilities,
revision identities, provenance, invalidation, and reverse-edit behavior must
remain observable and testable. Plain text uses an identity format projection
after decoding. Markdown, HTML, and RTF are motivating format adapters, not an
implicit commitment that every feature of each format is in the first release.

### Preservation guarantees

Every editable format pipeline MUST define and test these guarantees:

1. **Identity**: opening and saving without a source-changing edit reproduces
   the exact physical source artifact, including container bytes and parts.
2. **Patch locality**: after an edit, every original byte outside the declared
   source patch set remains byte-for-byte identical.
3. **Semantic edit**: projecting the patched source produces the formatted
   edit the user requested.

A patch set may include supporting changes outside the selected formatted
range when the source format requires them, such as an RTF font/color table,
an HTML encoding declaration, or a shared style definition. Such patches must
be minimal, explicit, and reported as part of the same atomic transaction.
Never regenerate an entire document merely because a small local edit is
easier to serialize that way.

Equivalent syntax is not interchangeable for untouched source. For example,
Markdown `**bold**` and `__bold__`, HTML entities with different spellings,
attribute order, comments, whitespace, and RTF control-word spelling must be
retained unless the edit necessarily changes that source region.

### Source storage and lossless syntax

- Store textual sources in a persistent balanced byte piece tree, rope, or
  B-tree-like structure. Original and newly inserted byte buffers are
  immutable; revisions reuse unchanged pieces.
- Permit non-flat source artifacts. A future packaged format may contain named
  parts and relationships while still exposing transactional byte ranges per
  part. It must also retain untouched entry bytes, ordering, compression, and
  container metadata needed by the identity guarantee.
- A lossless parser must represent all input, including trivia, unsupported
  constructs, parse errors, and skipped bytes. Unknown content is preserved as
  opaque syntax with known source extent; it is not silently discarded.
- Syntax nodes and source pieces have stable identities across incremental
  reparses when their content and interpretation are unchanged.
- A source snapshot can always serialize itself without consulting the
  formatted projection. Serialization of an unmodified snapshot is byte-exact.
- Format detection is read-only and records the chosen adapter and confidence.
  Ambiguous detection requires an explicit policy; it must not rewrite input.

### Encoding projection

- Raw source bytes remain authoritative even when their encoding is not UTF-8.
- The encoding stage emits valid UTF-8 together with a mapping from every
  output range to its source bytes and decoder state. It is streaming and
  checkpointable for large and stateful encodings.
- A BOM or in-format encoding declaration is source syntax and must be
  preserved. The chosen encoding and converter variant are document metadata.
- Invalid or unmappable source byte sequences are represented by visible
  replacement/diagnostic projections tied to the original opaque bytes. Merely
  opening or saving must not replace those bytes with a Unicode replacement
  character.
- Unchanged decoded text is saved by copying its original bytes, not by
  re-encoding it.
- New or changed Unicode text is encoded using the original encoding when it
  is representable. Otherwise the format adapter must return a policy result:
  use a semantically exact format escape, change the document encoding and any
  declarations, or reject/request a user decision. Silent substitution or data
  loss is forbidden.

### Shared text line-ending projection

Line-ending interpretation is a reusable pipeline component, not behavior
reimplemented by each format adapter and not a base class from which adapters
inherit. A text-like adapter composes a `TextLineEndingProjection` immediately
after decoding and before its lossless format projection. Plain text, Markdown,
and HTML use this component. Formats whose source grammar owns all line-break
semantics may omit it.

The component consumes decoded Unicode with source-byte provenance and emits a
sequence containing ordinary text and logical source-line-break tokens. Each
token retains whether its original spelling was LF, CRLF, or CR and the exact
decoded and source-byte ranges that produced it. The later format adapter
decides whether a source-line-break token becomes a formatted hard line, a
paragraph boundary, collapsible whitespace, trivia, or no visible content. For
example, raw source line endings do not automatically become visible hard lines
in HTML.

The normalized interface is shared, but semantic paragraph construction remains
adapter-specific. Adapters use composition and delegation rather than subtype
inheritance:

```text
DecodedText
    -> TextLineEndingProjection(fileformat/open policy)
    -> PlainText | Markdown | HTML lossless format projection
```

#### Detection and interpretation

The supported file-format names and their read interpretations follow Vim:

- `unix`: LF is a logical source-line break. A preceding CR remains ordinary
  content.
- `dos`: CRLF and bare LF are logical source-line breaks. A bare CR remains
  ordinary content.
- `mac`: CR is a logical source-line break. LF remains ordinary content.

NEL, Unicode line separator, Unicode paragraph separator, and other characters
are ordinary content at this stage unless a later format adapter deliberately
interprets them. A trailing DOS Ctrl-Z is likewise preserved as source content;
any future compatibility projection that hides it must remain lossless and
explicit.

`fileformat` is a concrete buffer-local value: `unix`, `dos`, or `mac`.
`fileformats` is an ordered open-policy list using those values. On macOS its
initial default is `unix,dos`, matching Vim on Unix-like systems; users may add
`mac`. The initial fallback `fileformat` is `unix`. A new empty buffer uses the
first `fileformats` item, or the fallback when the list is empty. Opening an
existing source follows these rules:

1. An explicit open request may force one interpretation without detection.
2. If `fileformats` is empty, the configured initial `fileformat` is used.
3. If `fileformats` contains one item, that interpretation is used.
4. With multiple items, choose `dos` when at least one line ending exists, all
   discovered line endings are CRLF, and `dos` is allowed; otherwise choose
   `unix` when any LF exists and `unix` is allowed; otherwise choose `mac` when
   CR exists and `mac` is allowed; otherwise use the first allowed item. As in
   Vim, detection may prefer `mac` when CR appears before the first LF and a
   bounded initial sample contains more CR than LF.

The chosen value and detection evidence are buffer-local pipeline metadata
shared by every view of that buffer and included in the document state needed
to serialize its source snapshot. Detection is read-only. The component records
whether the value was detected, forced, or defaulted, and diagnostics identify
mixed or suspicious endings without rewriting them.

Existing source-line-break tokens retain their exact original spelling during
ordinary edits and no-op saves. A newly inserted source-line break uses the
current `fileformat`: LF for `unix`, CRLF for `dos`, and CR for `mac`. Absence
or presence of a final line terminator is represented explicitly and preserved;
line-ending conversion does not add or remove a final terminator.

#### Changing `fileformat`

Pipelines containing this component expose `fileformat` as a shared capability,
regardless of whether their later adapter is plain text, Markdown, or HTML. The
commands `:set fileformat?`, `:set fileformat=unix|dos|mac`, their `ff`
abbreviations, and corresponding `:setlocal` forms query or change it. An
adapter must delegate these commands to the component rather than implement
them itself. A pipeline without the component reports the option as unsupported
for that buffer.

Changing `fileformat` after opening does not reinterpret which current
characters are logical breaks. It requests one atomic source transaction that
changes every existing source-line-break token to the target spelling, updates
the buffer-local metadata, and leaves formatted text and hard-line identities
unchanged. The declared patch set therefore includes all converted delimiters;
this explicitly requested whole-document operation is not subject to the usual
local-edit work bound. It is cancellable before commit and is one undo unit.

Before committing, the component simulates decoding and line-ending projection
of the candidate bytes under the target mode. If literal CR or LF content would
become a delimiter, combine with a delimiter, or otherwise change the logical
token sequence on reopen, translation returns a structured
`LineEndingConversionWouldReinterpretContent` policy result. The default is to
reject. A future explicit force policy may authorize Vim-like reinterpretation,
but the verified semantic intention must then describe the resulting content
change rather than claiming it is formatting-only.

The component provides incremental checkpoints, provenance composition, change
summaries, and reverse translation like every editable projection stage. Format
adapters request insertion or replacement of logical source-line-break tokens;
they never spell CR/LF bytes themselves. This keeps detection, new-break
spelling, conversion, validation, and diagnostics identical across all
participating formats.

#### Plain-text adapter use

The base plain-text pipeline is decoding, the shared line-ending projection,
and an otherwise identity lossless format projection. Each logical source-line
break ends one formatted hard line and one paragraph; consecutive breaks create
empty paragraphs, and the final unterminated segment is still a paragraph. Its
synthetic Base Document, Base Paragraph, and Base Character styles provide
display defaults, with zero paragraph spacing by default so the result has
gVim-like line placement. Plain text exposes no source-backed named styles or
direct formatting capabilities.

Ordinary decoded characters, including CR or LF characters not recognized as
delimiters under the selected open interpretation, remain formatted content and
retain byte provenance. The frontend may draw visible control representations,
but the adapter does not remove or normalize them. Aside from the explicitly
documented visual-row navigation behavior, plain-text commands operate on the
same logical lines and content as gVim fixtures.

### Formatted document model

The formatted document is normalized for editing and layout. It contains:

- a hierarchy of logical blocks such as paragraphs, headings, lists, tables,
  quotations, and embedded/opaque objects as adapters support them;
- valid UTF-8 text and explicit hard-line/paragraph boundaries;
- resolved and semantic inline style spans;
- document and paragraph properties needed by layout;
- stable projected identities where possible; and
- provenance for every text range, style, boundary, and object.

Provenance is relational rather than assuming a one-to-one offset map. Several
source ranges may produce one formatted item; one source construct may produce
several formatted items; and generated formatted content may have no direct
source range. Provenance records affinities at hidden delimiters and at
many-to-one boundaries such as HTML entities.

Formatted snapshots are immutable and cached by source revision plus pipeline
configuration. They may be materialized incrementally by region. Cache eviction
must never affect correctness because any projection can be regenerated.

### Normalized style system

The formatted document has a platform-independent, immutable style sheet. It
expresses semantic block and character styling without exposing CSS, AppKit,
Core Text, RTF, or another source format's object model to general core code.
Format adapters project their native styling systems into this model and retain
the original syntax and provenance needed for lossless reverse edits.
Source-language cascade rules remain adapter responsibilities to the extent an
adapter claims them. The initial HTML adapter deliberately supports only the
element mappings, Viem-owned class rules, and inline declarations specified in
"HTML and RTF import and round-trip adapters"; it does not claim general CSS
selector or cascade support. The generic style resolver does not reinterpret
source CSS or RTF control state.

The style sheet has a revision identity, stable style identities, and two
namespaces:

- A **block style** applies to a typed block node. Its role declares which
  property domains are applicable. The initial roles are Document and
  Paragraph. The document root uses a Document-role block style; a paragraph
  style is a Paragraph-role block style containing paragraph layout declarations
  and character declarations that provide the paragraph's default text
  appearance. A heading paragraph style can therefore set spacing and
  indentation as well as font family, size, weight, or color.
- A **character style** applies to a formatted text range and contains only
  character declarations. It does not change paragraph geometry.

Style identity is an opaque stable ID, not the user-visible name or an array
index. Renaming or reordering a style does not invalidate assignments to it.
Each style has at most one parent in the same namespace. Multiple inheritance
is forbidden. Every non-root block style ultimately derives from Base Document
and every non-root character style ultimately derives from Base Character. A
block-style child may narrow its parent's general applicability to a concrete
block role, but it may not broaden a specialized parent to an incompatible role.
Parent links must be acyclic; a missing parent, role violation, or cycle
produces a diagnostic and deterministically falls back to the applicable base
path without discarding source syntax.

Deleting a non-base style is allowed only when the same atomic intention
reassigns its children and every content assignment, or when none exist.
Otherwise deletion is rejected; it never leaves silently dangling style IDs.

Every style sheet defines three distinguished styles:

- **Base Document** is the root of the block-style hierarchy and has no parent.
  It is the default assignment for the formatted document root. It provides the
  canvas background and padding together with inheritable default character
  declarations such as font and foreground color. The initial generated Base
  Document requests **SF Pro at 14 layout units** as its default font; format-
  backed document styles may override that request through the normal cascade.
- **Base Paragraph** is a Paragraph-role child of Base Document and provides
  complete paragraph-layout values. It is the default paragraph-style
  assignment when an adapter does not provide a more specific assignment and
  the fallback selection for style-editing UI. Every other Paragraph-role
  style derives through it.
- **Base Character** is the root of the character-style hierarchy. Its sparse
  declarations refine the document defaults without preventing paragraph styles
  from overriding them.

These styles cannot be deleted. Together with engine emergency values they
produce complete paragraph and character results, so layout never depends on an
unrecorded platform default. Adapters may synthesize them from application,
document, source, or pipeline defaults and must identify which declarations are
source-backed versus generated.

#### Declarations and values

A style definition is sparse. For each property it either has no declaration,
in which case cascade resolution continues, or it has an explicit typed value.
Values such as normal weight, no underline, zero spacing, or transparent color
are explicit values and are distinct from absence. Clearing formatting removes
the declaration at the requested layer; it does not write a guessed value from
an ancestor.

Source-language constructs such as CSS `inherit`, relative units, or RTF state
transitions remain represented in the lossless syntax model. The adapter
projects their semantic declaration and records dependency/provenance edges so
that a change to an ancestor invalidates every dependent result. Unsupported
expressions may be projected as resolved read-only values with a capability
diagnostic rather than being approximated on reverse edit.

Every property belongs to a schema-defined domain with an applicability and
inheritance rule. The initial domains are Document Canvas, Paragraph Layout,
and Character. A style definition may declare only properties allowed by its
role: for example, a Document-role style cannot declare future list-numbering
properties, while a Paragraph-role style cannot change the document canvas
padding. Inapplicable declarations are diagnostics, not silently ignored
values. Target-only properties affect the assigned block; inheritable properties
also contribute to applicable descendants.

Initial Document Canvas properties include:

- canvas background color; and
- logical start, end, top, and bottom padding.

The Base Document style's Character declarations define the default content
font request, size, and foreground color. Selection, caret, diagnostics, gutter,
window chrome, and other editor-interface colors remain view/theme properties;
they are not document content styles and are adjusted independently for
accessibility.

Initial character properties include:

- an ordered font-family/fallback request, separate from the concrete font
  identities returned by the shaping provider;
- font size in layout units, base numeric weight, slant, and semantic Bold;
- optional foreground/background colors (unspecified foreground uses the theme);
- underline and strike decoration;
- language and writing-direction override;
- OpenType feature settings; and
- letter spacing and baseline shift.

Bold is a sparse semantic Boolean separate from the selected face's base weight.
Resolution adds 300 to that base weight, capped at 1000; native face selection
uses the policy in the style-editor section. Italic changes the slant request.
Portable color and font requests are value types and contain no native handles.
The resolved character style retains whether foreground/canvas colors are
unspecified, allowing the frontend to resolve theme defaults at paint time.

Initial paragraph properties include:

- space before and space after;
- logical start and end indents and a first-line indent, with signed values so
  hanging indents are representable;
- line spacing as `normal`, a font-metric multiplier, `at-least`, or `exact`;
- logical alignment: start, end, or center; and
- base writing direction.

Justified alignment, custom tab-stop collections, borders, backgrounds,
keep-with-next, and pagination properties are reserved extensions rather than
silently accepted initial features. Property records are versioned and
extensible; an unknown property is retained with provenance by the adapter but
has no layout effect until the core declares support for it.

Absolute distances use layout units and are independent of backing scale and
zoom. Relative values are resolved against explicitly recorded inherited font
or containing-block inputs. Invalid numbers, non-positive font sizes, and other
out-of-domain values produce deterministic diagnostics and schema-defined
fallbacks.

#### Cascade and assignments

The formatted document root is a styleable Document block. It stores a
Document-role block-style ID, normally Base Document, plus sparse direct
Document Canvas and default-character declarations when the adapter needs to
represent source constructs such as an HTML `body` inline style. Each paragraph
stores a compatible Paragraph-role block-style ID plus sparse direct paragraph
and paragraph-default-character declarations. Character-style assignments and
direct character formatting are separate range maps over formatted text. At a
given text position there is at most one assigned named character style, but
different direct properties may cover independently overlapping ranges.

Document Canvas properties are resolved in this order:

1. engine emergency values;
2. ancestor-to-descendant applicable declarations from Base Document through
   the block style assigned to the document root; and
3. direct Document Canvas declarations on the root.

Paragraph geometry is resolved in this order, with later declarations winning:

1. engine emergency values;
2. applicable inheritable block declarations from Base Document;
3. ancestor-to-descendant declarations from Base Paragraph through the
   Paragraph-role style assigned to the paragraph;
4. a future structural block contribution, such as list-item geometry; and
5. direct paragraph declarations on the paragraph.

Character appearance is resolved in this order:

1. engine emergency values;
2. character declarations from Base Document through the Document-role style
   assigned to the root, followed by direct root default-character declarations;
3. Base Character declarations;
4. character declarations from Base Paragraph through the assigned paragraph
   style, followed by direct paragraph-default-character declarations;
5. ancestor-to-descendant declarations explicitly present in the assigned
   character-style chain, without reapplying Base Character over the paragraph
   defaults;
6. a future structural contribution for generated content such as a list
   marker; and
7. direct character declarations.

This ordering makes the document style the common visual foundation, while a
named paragraph style can change a heading's font and a named character style
and then direct bold, italic, font, size, or color formatting can override it.
Direct formatting never mutates or implicitly creates a named style.

Direct character formatting is canonicalized per property: applying a property
replaces that property's value only in the selected range, splitting existing
runs as needed, while leaving unrelated properties intact. Clearing it removes
that property's direct declaration and reveals the underlying named-style or
paragraph result. Equivalent adjacent assignments and declaration runs are
coalesced. This avoids making rendering depend on the historical order in which
overlapping bold, italic, and font spans were applied.

Every resolved property retains contribution metadata identifying its winning
declaration and dependencies. Style-definition changes invalidate assignments
to that style and all transitive descendants, but unrelated styles and text
remain valid. Resolution is cacheable by style-sheet revision, style IDs,
direct-declaration identity, and structural-context identity.

#### Editing, insertion, and provenance

Style operations are typed semantic intentions, including applying a compatible
named block or character style, setting or clearing a direct property, and
editing a style definition. Document- and Paragraph-role assignments use the
same block-style intention with different target kinds. Each adapter reports
these capabilities separately: support for displaying a style does not imply
that its definition, assignment, or every direct property can be
reverse-projected.

Style definitions, assignments, direct declarations, and generated defaults all
carry provenance. A source-backed style edit follows the same minimal-patch,
reprojection, verification, and undo transaction path as text. A generated
configuration style may be edited only through an explicit configuration
intention. A synthetic read-only style cannot be edited.

Inserted text inherits the character-style assignment and direct character
declarations at the caret side selected by its boundary affinity. Inside a run
this is unambiguous; at a run boundary upstream affinity chooses the preceding
run and downstream affinity chooses the following run. Anchor association
controls how the caret remaps across that insertion and does not select the
typing style. At paragraph start or end, the only interior side is used. Empty
paragraphs retain an explicit paragraph-style assignment and typing-character
declarations even though they contain no text.

Choosing an assignable character style with no selection sets a view-local
pending named style for subsequent typing. This retains the style's identity,
separate from direct character declarations, and immediately updates character
style menu state. The gesture itself does not change source bytes or revision,
mark the buffer dirty, or add an undo entry. Choosing it in Normal mode carries
it into the next Insert or Replace session. Text and its pending style commit
together in one verified transaction; failed preparation preserves the pending
style and editor state.

With a selection, character-style assignment applies to the selected text in
each containing block. Format wrappers must remain inside paragraph, heading,
list-item, and preformatted containers, splitting into multiple local patches
where necessary. It must preserve unselected text and formatting, paragraph
structure, and untouched source spelling. Markdown's native Code and Base
Character assignments use inline-code delimiters and their removal; styles
without a representable Markdown assignment remain unavailable.

Splitting a paragraph normally copies its paragraph-style assignment and direct
paragraph declarations to the new paragraph. A paragraph style may name a
`next_paragraph_style`; when present, Enter at the paragraph's terminal boundary
uses it for the new paragraph. Joining paragraphs keeps the first paragraph's
style and paragraph declarations for the result, including across different
list, quote, heading, code, and ordinary container boundaries. Retained explicit
inline formatting remains attached to its text.

In rich-text editing modes, `o` and `O` create a paragraph using the originating
paragraph style's following-style rule, defaulting to that same style when no
next style is configured. `O` uses that rule even though the new paragraph is
placed before its origin. The originating paragraph and the direction are
explicit in the semantic request; its insertion offset alone is insufficient.
Counts and repeat evaluate the rule for each newly opened paragraph, and opening,
styling, and subsequent typing share one Insert undo unit. Literal source modes
retain source-line editing.

Markdown rich-text editing preserves body-leading ASCII spaces and tabs with
native numeric character references when literal source whitespace would be
consumed as structural indentation. These references project as whitespace
outside code; code and source-visible modes retain their literal spelling.
Typing reference syntax as ordinary text escapes it, and a no-op save preserves
the original source bytes.

#### Rich-text deletion boundaries

Backspace at the visible beginning of a list item or code paragraph removes
that structural treatment and assigns the normal paragraph style without
deleting text. The reset removes every enclosing structural layer that would
otherwise keep that paragraph in a list, quote, or code treatment. At the beginning of every other paragraph it removes the previous
paragraph boundary and joins into the preceding paragraph. With no preceding
content or resettable treatment it is a no-op. Within a paragraph it deletes the
preceding grapheme, including when the caret begins an inline character-style
range; inline tags are not extra deletion stops.

Forward Delete removes the following grapheme or paragraph boundary and is a
no-op only at document end. Selection deletion removes exactly the selected
logical items and joins every crossed paragraph boundary into the paragraph at
the selection's beginning. Flat edits, formatted payloads, clipboard replacement,
and keyboard commands share this structural translation. Source patches that
share structural delimiters are composed before one atomic publication; logical
selection ranges and position maps remain unchanged by supporting source edits.

#### Lists and future structured blocks

Lists are document structure, not a bullet character embedded in text and not
merely a paragraph-style flag. The formatted document exposes a snapshot-bound
list-structure query with List and List Item nodes carrying stable item and list
identities, nesting level, parent/child relationships, marker/numbering policy,
and explicitly tagged marker ranges. Each item records every paragraph it
contains; only its first paragraph carries the generated marker. Independent
source list containers remain distinct even when adjacent. The initial list
container follows its first item's identity; surviving item identities support
recovery when that item is deleted. Paragraphs inside an item continue to use
Paragraph-role block styles. Eight generated internal Paragraph styles provide
four levels each for Bulleted List and Numbered List: `BulletedList1` through
`BulletedList4` and `NumberedList1` through `NumberedList4`. The style editor
shows these definitions; the Paragraph menu shows an internal list style only
when it is the current uniform assignment. This flag is separate from internal
source-syntax styles. Existing deeper lists remain lossless and render using
the fourth style plus the additional structural inset. Authored legacy `ListN`
definitions remain readable. Supported source list properties can override
defaults; list commands create or change the actual bullet/number structure.
Generated markers have explicit synthetic provenance and caret/edit rules.

Future List-role styles will use the same block-style record and inheritance
mechanism, with list-only marker and numbering declarations rejected for
Document and Paragraph roles. They may contribute indentation and spacing at
the reserved structural cascade layer and reference a Character style for the
generated marker.

HTML and Markdown expose the Paragraph style `Block quote`, mapped to native
`blockquote` containers and Markdown `>` prefixes. WYSIWYG quotes use the same
indentation and noneditable left border in both formats. Source HTML retains
its tags and applies quote presentation to their source paragraph. Markdown
Source retains every `>`; quote indentation and border apply when Flow Source
Paragraphs is enabled and are suppressed when it is disabled. Assigning or
removing quotation treatment uses local source patches and preserves inline
formatting, text, hard-line structure, and exact history bytes. Paragraph-style
commands assign one structural treatment at a time: changing a list item,
heading, or code paragraph to Block quote replaces that treatment, and changing
a quote to a list removes its quotation treatment. Only selected paragraphs
change; neighboring items and containers retain their source bytes. Imported
nested containers remain lossless and editable until an explicit style change
normalizes the affected paragraph.

In WYSIWYG, choosing Block quote without a selection at the end of a nonempty
ordinary paragraph creates a blank quote after the existing prose and places
the insertion cursor there. An existing empty final paragraph is reused; an
empty final row within ordinary prose becomes its own quote paragraph. The
insertion and style assignment are one atomic, undoable transaction.
Enter continues a nonempty quotation in a new paragraph; the generated quote
style therefore uses itself as its next-paragraph style. Enter in an empty quote
removes its quotation treatment. Backspace at the beginning of a quote joins it
into the preceding paragraph; list and code treatment follow the reset rule above.
Deleting the entire selected document also removes content-level paragraph
wrappers, leaving an empty ordinary paragraph ready for typing; untouched
document metadata and source encoding are retained.
Future tables, callouts, and other container kinds remain outside this scope.

### Semantic edit intentions and reverse projection

The UI and Vim command engine issue typed intentions against a projection
snapshot, for example replacing text, setting or clearing bold, changing a
block kind, inserting a link, or deleting a formatted range. They MUST NOT
directly persist mutations to derived style spans.

Committed edits follow this path:

1. capture the semantic intention, directed selection/cursor anchors including
   insertion association and boundary affinity, and the exact projection
   revision on which
   the command operated;
2. ask the transformation stages, from last to first, to translate that
   intention into edits to their respective inputs;
3. compose the result into a minimal source patch set or return an explicit
   unsupported, ambiguous, stale, or needs-policy result;
4. tentatively apply the source patch set to a new source snapshot;
5. incrementally project the affected output and verify that it satisfies the
   semantic intention; and
6. atomically commit the source snapshot, undo record, projected state,
   registers, marks, cursor, and invalidations—or commit nothing.

Formatting syntax is adapter policy. A Markdown adapter may preserve nearby
`**` versus `__` convention; an HTML adapter must deliberately choose among
`<b>`, `<strong>`, a class, or inline CSS; an RTF adapter must respect group and
formatting state. Each adapter defines a deterministic local-style-preservation
rule and a configurable default for newly authored syntax.

Plain text cannot persist bold or other rich styles. Its adapter reports that
capability as unsupported, allowing the frontend to disable the action or
offer an explicit conversion to a richer format. Do not create a hidden style
sidecar unless a future source format expressly defines one.

IME marked text and similar native composition MAY use a short-lived projected
overlay before commit. Cancellation discards it. Committing it uses the same
verified source-transaction path as every other edit.

### Transformation stage contract

Every stage declares whether it is reversible, an editable projection, or
one-way/generated. An editable stage provides conceptual operations equivalent
to:

- `project(input_snapshot, previous_output, input_changes)` returning output
  and provenance;
- `translate(edit_intent, input_snapshot, output_snapshot)` returning input
  edits or a structured result;
- `capabilities(output_range) -> supported edit intentions`; and
- `invalidate(input_changes) -> affected output dependencies`.

The concrete API need not use these names. It must additionally provide:

- immutable input/output revision identities;
- stable node/range identities where possible;
- composable provenance with insertion association and boundary affinity where
  applicable;
- deterministic results for the same inputs and configuration;
- bounded incremental work and cancellation;
- structured diagnostics and policy requests; and
- verification of reverse-projected semantic edits.

No arbitrary forward transform is presumed editable. Smart typography,
generated tables of contents, filtered content, or other lossy/synthetic output
is read-only unless the stage defines an unambiguous reverse rule. The pipeline
composes edit capabilities: an edit is offered only if every stage on its path
can translate it.

### Incremental projection and pipelining

- Each projection cache is keyed by its input revision, transform identity and
  version, configuration, and relevant external-resource generation.
- A source edit invalidates only dependent syntax and formatted regions, then
  flows invalidation forward through later stages and layout.
- Stateful formats use restart checkpoints and reprocess until parser or
  transformation state converges with an unchanged checkpoint. Unclosed
  Markdown constructs, RTF groups, HTML parsing state, shared definitions, and
  CSS may legitimately widen invalidation; the stage must expose why.
- Opening a large document may index or parse source progressively. Producing
  the first interactive viewport must not require formatting unrelated content
  unless a declared global dependency makes it necessary.
- Background projection is cancellable and revision checked. A stale top-level
  result is never installed. Independently keyed subresults may enter a shared
  cache only when every content identity and dependency generation is
  revalidated as unchanged.
- A layout-only change such as window width or zoom does not invalidate source,
  decoding, syntax, or semantic projections. It invalidates only the applicable
  shaping/wrapping/layout layers.
- Pipeline stages may be reordered only when their contracts declare that the
  result and reverse-edit semantics commute. Transformation order is otherwise
  document configuration and part of the projection identity.

## HTML and RTF import and round-trip adapters

HTML and RTF are editable source formats, not lossy import/export filters.
Their adapters obey the source-authority, preservation, semantic-intention,
and verified reverse-projection requirements above. Opening either format,
including malformed or partially unsupported input, never grants permission to
normalize or regenerate the file.

Both adapters maintain two related structures:

1. a lossless concrete syntax representation containing every original byte,
   delimiter, escape, spelling choice, comment, unknown construct, error, and
   opaque payload; and
2. a semantic interpretation containing only the visible text, blocks, styles,
   objects, and dependencies that Viem understands.

The semantic interpretation may use repaired, implied, or inherited structure,
but provenance always returns to the concrete source nodes that produced it.
Untouched concrete nodes serialize from their original byte slices. Unsupported
syntax may affect neither display nor editing, but it is never discarded.

### Common safety and edit-boundary rules

- Import is passive. The adapters MUST NOT execute scripts, macros, fields, OLE
  objects, event handlers, or other active content; fetch URLs, stylesheets,
  fonts, images, templates, or subdocuments; or instantiate a browser/web view
  to determine formatted output.
- Nonprinting and unsupported content remains in the lossless tree with stable
  source anchors. Ordinary edits to nearby visible text do not delete or move
  it. A visible edit whose reverse mapping would cross or consume hidden opaque
  content returns a structured ambiguous/unsupported result unless the adapter
  can preserve that content at an equivalent boundary.
- A source construct has an explicit canonicalization boundary. Editing only
  descendant text does not touch its tags, attributes, controls, or other
  metadata. Editing a formatting property contributed by that construct may
  replace the smallest declared formatting construct with the adapter's
  canonical representation. Source inside that declared replacement is part of
  the reported patch set and no longer receives the untouched-byte guarantee.
- Canonicalization never expands to an ancestor, sibling, unrelated style
  definition, or whole document merely for serializer convenience. Required
  shared-table changes and style dependents are explicit supporting patches in
  the same atomic transaction.
- All reverse edits are tentatively reparsed and reprojected before commit.
  They commit only if visible text, block boundaries, style assignments,
  effective supported properties, opaque-content anchors, and source
  well-formedness expectations satisfy the intention.

### HTML adapter

#### Parsing, preservation, and active content

The HTML adapter uses `text/html` parsing semantics, including error recovery,
implied elements, optional tags, raw-text elements, and character references.
A browser-like semantic tree is not sufficient for round trip, because HTML
parsing can repair structure and discard syntax distinctions. The adapter
therefore retains a separate lossless token/concrete tree containing original
tag-name case, start/end-tag presence, attribute order, duplicate attributes,
quote style, whitespace, comments, doctypes, character-reference spelling,
parse errors, and bytes outside the document element.

`script` and `style` are raw-text elements. Script contents and all event-handler
attributes are preserved but never executed. `head` metadata, comments,
`template` content, scripts, non-Viem style elements, linked stylesheets, and
other nonprinting nodes produce no editable body text. Unsupported visible
elements retain their source structure; their unambiguous visible descendant
text may still be projected using supported inline semantics. Unsupported
atomic content may instead appear as an opaque object with no editable interior
when omitting it would conceal a visible document item. Its source extent remains
one complete contributor for whole-object deletion and replacement.

The adapter does not apply external stylesheets, arbitrary selectors, layout
scripts, or browser default CSS. It interprets only:

- the element-to-structure and element-to-format mappings in this section;
- the first applicable Viem-owned class named on an element;
- supported declarations in an element's inline `style` attribute; and
- the canonical Viem-owned inline stylesheet described below.

All other CSS, including unknown properties, unsupported values, `@` rules,
selectors outside the canonical subset, and additional style elements, is
opaque source. It is preserved byte-for-byte while untouched and ignored by
the formatted projection.

#### HTML block and inline projection

The initial structural mapping is:

- `body` supplies the formatted document's body content;
- `p` creates a paragraph assigned Base Paragraph unless an applicable
  Viem-owned paragraph class overrides that assignment;
- `h1` through `h6` create paragraphs assigned the adapter-provided Heading 1
  through Heading 6 paragraph styles respectively, again subject to an
  applicable Viem-owned paragraph class;
- `br` creates a formatted hard-line boundary inside the current paragraph;
- a block container with a recognized paragraph-style assignment owns a logical
  paragraph, including when empty; source-flow layout uses the same ownership;
- visible phrasing content outside an explicit supported paragraph is grouped
  into the minimum anonymous Base Paragraph blocks necessary to represent it;
  and
- unsupported containers are structurally transparent only when their visible
  descendant text and boundaries can be projected without ambiguity.

Heading 1 through Heading 6 have stable adapter-defined style identities,
derive from Base Paragraph, and exist even when no Viem stylesheet is present.
Changing a paragraph's block kind between Base Paragraph and a heading rewrites
the corresponding `p`/`h1`…`h6` tags. Editing a heading style definition writes
or updates its canonical rule in the Viem-owned stylesheet; it does not replace
heading elements with generic paragraphs.

A verified paragraph merge MAY use a flow-capable `div` with the same assigned
paragraph style when a retained child such as a table cannot remain inside `p`.
Recovered HTML source order is materialized only for the affected contributors,
with unchanged text, paragraph assignments, character styles, and hard lines,
before applying the requested edit as one atomic transaction.

In normal HTML text contexts, source whitespace that HTML treats as
collapsible projects to the corresponding visible spacing with many-to-one
provenance. Original whitespace spelling remains untouched until an edit
necessarily replaces that source range. Preformatted or otherwise unsupported
whitespace behavior is preserved as opaque/read-only unless the adapter has an
explicit reversible mapping for it.

The initial semantic inline element mappings are:

- `b` and `strong` contribute bold weight;
- `i` and `em` contribute italic slant;
- `u` contributes underline;
- `s`, `strike`, and `del` contribute strike decoration; and
- `span` contributes no property by itself but can carry a supported class or
  inline declaration.

`lang` contributes the supported language property and `dir` contributes the
supported writing-direction override when their values are understood. Other
attributes do not affect the normalized projection unless this section later
adds an explicit mapping.

For example, `<b foo="bar">text</b>` projects `text` as bold. The unknown
`foo` attribute remains byte-identical through text edits inside the element
and through unrelated edits elsewhere. If a style operation changes the
formatting contributed by that exact `b` element, its start/end tags form the
canonicalization boundary and may be replaced by canonical markup; `foo` is
then inside the declared patch and need not be retained.

#### Supported inline CSS

An inline `style` attribute is parsed as a CSS declaration list using
forward-compatible parsing. Only declarations that convert exactly to the
normalized property schema are applied. Invalid declarations, unsupported
properties, unsupported values, and declarations using units or expressions
that cannot be reversibly represented are ignored semantically and preserved
as source while the formatting construct remains untouched.

The initial supported Character-property mappings are:

- `font-family` -> ordered font-family/fallback request;
- `font-size` -> font size;
- `font-weight` -> numeric weight;
- `font-style` -> slant;
- `color` and `background-color` -> foreground and background color;
- `text-decoration-line` -> underline and strike decoration;
- `letter-spacing` -> letter spacing;
- `vertical-align`, for supported length, `super`, and `sub` values -> baseline
  shift;
- `font-feature-settings` -> OpenType feature settings; and
- `direction` -> writing-direction override.

The CSS property is `font-family`; `font-face` is not a supported declaration
(`@font-face` is a stylesheet rule) and is preserved but ignored. The initial
supported Paragraph-property mappings are:

- `margin-block-start` and `margin-block-end` -> space before and after;
- `margin-inline-start` and `margin-inline-end` -> logical start and end
  indents;
- `text-indent` -> first-line indent;
- `line-height` -> the supported line-spacing kind/value;
- `text-align` values `start`, `center`, and `end` -> logical alignment; and
- `direction` -> base writing direction.

The canonical writer uses one documented absolute unit for each length domain
and lowercase property names in schema order. It does not claim to preserve the
semantics of arbitrary CSS shorthand, variables, `calc()`, viewport units,
media queries, or selector cascades.

For `vertical-align`, Viem normalizes `super` to an upward baseline shift of
one third of the element's effective font size and `sub` to a downward shift of
one fifth of that size. `baseline` is an explicit zero shift. The effective
font size includes the document, paragraph, named character, inherited inline,
and element's own valid declarations, independently of CSS declaration order.
This deterministic import policy does not depend on browser or platform font
metrics. The resulting normalized property is an absolute point length;
authored direct formatting writes that length in `pt`, and reprojecting an
unchanged keyword reevaluates it against the then-effective font size.

Element semantics and style sources resolve in this order, with later sources
winning for supported properties:

1. adapter defaults and the `p`/heading/inline-element mapping;
2. the selected Viem-owned paragraph or character style; and
3. supported inline `style` and `lang`/`dir` declarations.

If duplicate attributes or otherwise malformed syntax reports more than one
candidate under HTML parsing semantics, the semantic projection uses the first
effective attribute while the lossless tree preserves them all.

#### Canonical direct formatting

New direct inline formatting uses a deterministic wrapper:

- bold as the primary conventional property uses `b`;
- italic uses `i`;
- underline uses `u`;
- strike uses `s`; and
- when other or multiple Character properties are required, the first
  applicable conventional wrapper above is retained and remaining properties
  are written in one canonical `style` attribute; if no conventional wrapper
  applies, use `span style="…"`.

Thus bold plus a font request may be written as
`<b style="font-family: …">…</b>`. Removing bold while retaining that font
rewrites it canonically as `<span style="font-family: …">…</span>`. Paragraph
direct formatting is written on the existing paragraph-bearing element's
`style` attribute.

When a supported formatting change targets an existing formatting element or
`style` attribute, the adapter may rewrite that declared canonicalization
boundary using only supported canonical attributes/declarations. Unknown
attributes or declarations inside that boundary may therefore be removed.
Untouched ancestors, descendants outside the formatted range, and sibling
attributes remain original bytes. Partial-range edits split wrappers as needed,
preserving the original wrapper bytes around unaffected left/right content when
their source structure can remain valid.

HTML whitespace follows CSS Text processing across inline element boundaries.
In normal and nowrap contexts, spaces, tabs, and segment breaks collapse;
leading and trailing collapsible whitespace at hard-line boundaries is removed.
Source segment breaks join words with a space. Form feed, NBSP, and other
Unicode separators are not ordinary collapsible spaces. `pre`, `pre-wrap`, and
`break-spaces` preserve spaces and tabs; `pre-line` preserves segment breaks
while collapsing spaces and tabs. These values inherit and can be overridden.

WYSIWYG formatted edits interpret spaces and tabs in their HTML context. Each requested
space that would collapse MUST become a nonbreaking U+00A0, serialized as
`&nbsp;`, rather than generating a whitespace style wrapper. Ordinary word
spaces MUST use literal source spaces. Subsequent typing SHOULD replace an
editor-generated protective NBSP with an ordinary space when it becomes safe;
that supporting edit belongs to the same verified transaction and position map.
Explicitly inserted or imported NBSPs remain nonbreaking. Typed hard breaks
remain hard breaks rather than collapsible source newlines. In whitespace-
preserving contexts, typing keeps literal spaces and tabs.

This policy also applies to paste, substitution, deletion, paragraph splitting,
and the semantic formatted replacement APIs. When deleting or splitting exposes
an existing space to collapse, a minimal supporting patch MUST protect it with
`&nbsp;` and include its U+00A0 replacement in the formatted position map.
These edits MUST NOT generate `white-space` spans. Imported/source-authored
whitespace syntax retains its original bytes outside declared patches. Exact
source restoration retains its recorded bytes and character semantics. Every
edit MUST verify reopen equivalence and exact undo/redo, including changes in
UTF-8 byte length when a space becomes NBSP.

New text is escaped canonically for its HTML context. Existing character
references such as `&amp;`, `&#38;`, and `&#x26;` retain their original spelling
until their source range is edited.

#### Native HTML elements and optional style definitions

HTML authoring MUST use native elements whenever they represent the selected
style: `p` for Paragraph, `h1` through `h6` for headings, `li` inside `ul` or
`ol` for list items, `pre` for Code Block, `blockquote` for Block quote, and
`code` for inline Code. Applying Bulleted List to a blank paragraph creates an
editable `<ul><li></li></ul>`;
typing then inserts inside `li`. List markers are decorations, never inserted
text or characters in style spans. Existing ordered lists retain their
numbering and nested lists retain their structure. Applying a deeper built-in
level to a plain paragraph creates the necessary native list/item ancestors.
An ancestor `li` containing only a nested list is structural and does not
invent an empty formatted paragraph; a genuinely empty item remains editable.
A named level MUST NOT be
simulated by a styled `p` or by an invented class that changes the apparent
level without changing list structure. Unsupported structural changes return a
structured failure rather than producing misleading markup.

Structural assignments replace only the needed tag names and container
syntax. They preserve unrelated attributes, comments, inline markup, and source
spelling. Adjacent selected paragraphs SHOULD share one list container. An
ordinary numbered list starts with `<ol>`; a `start` attribute is needed only
for a non-default ordinal or to override an existing conflicting declaration.
Removing a complete list removes its containers instead of leaving empty
sibling lists. Native assignment does not add a class or a CSS rule merely to
identify a built-in style. An empty class attribute left by removing owned
style assignments is removed.

Format > Style includes **Include style definitions in file** in both HTML
WYSIWYG and HTML Source. This is portable buffer-local state shared by the two
views, defaulting to off for new documents. Opening a file containing recognized
owned native style definitions restores the enabled state; an explicit owned
marker preserves an enabled state when no non-default rule is necessary.
Changing it is one atomic, undoable source transaction. Undo and redo restore
both the exact source and the option; changing views preserves the option.

When the option is off, built-in style definitions come from the saved HTML
settings and MUST NOT be copied into the source on assignment or ordinary
editing. Direct formatting remains source-backed because it was applied
separately from a style. A custom style without an HTML-native representation
uses a class and its required owned definition even with this option off.
Turning the option off removes only recognized owned native definitions;
custom definitions, direct formatting, and unrelated or unsupported CSS remain
intact. Merely opening or saving never rewrites source CSS.

When the option is on, Viem writes the necessary CSS for its style sheet using
native selectors. Default HTML behavior MUST NOT produce redundant declarations:
zero text indent, normal letter spacing, normal baseline alignment, and other
browser-default values are omitted unless an override is needed. List styling
belongs to `li`, with descendant `li` selectors for deeper levels, while `ul`
and `ol` retain their ordinary container and marker semantics. Native defaults
do not require per-paragraph or per-character metadata classes.

#### Canonical Viem style sheet and classes

New owned definitions use the exact marker
`<style id="viem-styles" data-viem-version="2">`. A style element without a
recognized marker, or content outside the supported grammar inside a marked
element, is never silently adopted or rewritten. Version 1 remains readable as
an import format; all authoring uses version 2. The next explicit persisted
stylesheet operation, such as editing a definition or changing Include Style
Definitions, migrates recognized version-one rules to version two in the same
verified source transaction and undo unit. This migration may update recognized
owned rules beyond the individual definition being edited, but preserves
unrelated CSS, comments, unknown rules, and body content. Ordinary text editing
and no-edit open/save do not trigger migration. Rules retain their source order:
fully recognized elements change version in place, while mixed elements split
into adjacent owned and opaque runs without moving rules across intervening CSS.
Multiple recognized owned elements may coexist for this purpose.

An owned style element is inserted near the beginning of `head`, after an
encoding declaration whose placement is constrained. If a full document lacks
`head`, the transaction inserts the minimum explicit head. For an accepted
HTML fragment, it inserts the element at the fragment's beginning. No element
is inserted for a native style assignment while inclusion is off.

Canonical version-two selectors are `body`, `p`, `h1` through `h6`, `li` and
its repeated descendant forms, `pre`, `code`, `.viem-p-<stable-id>` for Paragraph
styles, and `.viem-c-<stable-id>` for Character styles. Classes ordinarily name
custom styles; imported legacy classes may also retain native style IDs so
existing body elements and assignments remain intact. Stable class-ID suffixes
are lowercase hexadecimal UTF-8, independent of display names.
Native selectors supply their built-in identity, role, and default links;
ordinary sparse CSS carries browser-representable properties. Empty native
rules are omitted. Class rules additionally carry required stable ID, name,
role, and optional parent and next-style links as namespaced metadata. Their CSS
is computed for class context rather than assuming native heading, list, or code
element defaults. When a native style retains a legacy class, authoring keeps
its class and native rules synchronized so newly assigned native elements use
the same definition.

A deleted native block definition emits the effective Paragraph CSS after
resetting its defaults, so passive readers match its paragraph fallback.
The exact earlier version-two deletion rule containing only `font: inherit`
and `margin: 0` remains readable; subsequent stylesheet authoring updates it.

Only normalized distinctions that CSS cannot recover exactly need residual
`--viem-prop-<schema-key>` declarations. These include relative bold and
at-least line spacing. `--viem-inherit` records sparse inherited properties
where browser interoperability requires a derived declaration. Direct
formatting is never flattened into a named-style rule. CSS declaration order,
whitespace, quoting, and escaping are fixed by the version-two golden fixtures;
unsupported versions and noncanonical rules remain opaque.

Version 1 retains its original authoritative namespaced-property grammar:
`--viem-style-id`, `--viem-style-name`, `--viem-style-role`, optional
`--viem-based-on` and `--viem-next-style`, and
`--viem-prop-<schema-key>` for explicit normalized properties. Values are CSS
double-quoted strings; control characters, quote, backslash, and `<`, `>`, `{`,
`}` use lowercase hexadecimal CSS escapes followed by a space. Standard CSS
in these legacy rules is derived interoperability output. The legacy reader
validates exact canonical spelling rather than guessing another interpretation.

After migration, editing a definition patches only its owned rule and the
necessary dependent rules. Creating, renaming, rebasing, or deleting a custom
style preserves unrelated stylesheets and assignments. Reopening with the same
saved defaults reconstructs the same normalized assignments and effective
formatting.

A custom Paragraph style class is attached to its paragraph-bearing element,
including `li` for a list item. A custom Character style class is attached to a
`span` enclosing the assigned range. With several recognized class tokens of
one role, the first supplies the normalized assignment. Assignment changes
remove recognized tokens of that role, retain unsupported tokens in their
original order, and add a selected custom class only when native HTML cannot
represent the style.

### RTF adapter

#### Parsing, state, preservation, and safety

The RTF adapter follows RTF 1.9.1's group stack, control-word/control-symbol,
destination, binary-data, code-page, Unicode escape, and property-state rules.
It builds a lossless token/group tree in addition to its semantic formatting
state. Original brace placement, control-word spelling and delimiter, numeric
spelling, insignificant source line breaks, escaped characters, fallback bytes,
unknown controls, destinations, and binary payloads remain exact source.

RTF text decoding depends on header code page, font character set, `\ucN`
fallback count, and `\uN` escapes. The RTF adapter may fuse grammar-aware
decoding with its format projection, but it must still expose the revision,
valid UTF-8, byte provenance, invalid-byte preservation, and reverse-encoding
behavior required of the conceptual EncodingProjection. RTF does not use the
shared TextLineEndingProjection: source CR/LF used to format the RTF stream is
not document content; `\par` and `\line` carry formatted break semantics.

Recognized non-body destinations such as `\fonttbl`, `\colortbl`,
`\stylesheet`, and `\info` are parsed for supported dependencies but do not
emit body text. Unknown ignorable destinations beginning with `{\*` are
retained as opaque groups and skipped semantically. Unknown non-ignorable
controls follow RTF state rules while remaining in the lossless tree.
Pictures, objects, fields, headers/footers, annotations, macros, data stores,
and other unsupported destinations are never executed, updated, fetched, or
instantiated. An unsupported item that is visibly positioned in body content
may project as an atomic opaque object with no editable interior. Whole-object
deletion or replacement owns its complete source group.

For fields, Viem may display an unambiguous stored result destination, but it
never evaluates or refreshes the instruction. A visible edit that would make
the preserved instruction and result inconsistent is rejected unless a future
field-edit policy explicitly owns both.

#### RTF body and formatting projection

The adapter evaluates supported formatting as scoped state:

- opening/closing braces push and restore state;
- `\plain` and `\pard` reset Character and Paragraph state respectively;
- `\par` ends a paragraph and `\line` inserts a hard line within a paragraph;
- `\b`/`\b0`, `\i`/`\i0`, underline controls, and strike controls map to
  weight, slant, underline, and strike;
- `\fN` and `\fsN` map through the font table to font request and size;
- `\cfN` and supported background/highlight controls map through the color
  table;
- supported language, direction, character-spacing, baseline, and feature
  controls map to their corresponding Character properties; and
- `\liN`, `\riN`, `\fiN`, `\sbN`, `\saN`, `\slN`/`\slmultN`,
  `\ql`/`\qc`/`\qr`, and paragraph-direction controls map to the supported
  Paragraph properties when exactly representable.

Unsupported controls and values remain source but contribute no normalized
property. Justification and other reserved properties are not approximated as
a supported alignment.

The RTF `\stylesheet` destination supplies named styles. Paragraph style
`\s0` maps to Base Paragraph. Other `\sN` definitions map to Paragraph
styles; `\*\csN` definitions map to Character styles. `\sbasedonN` and
`\snextN` map to parent and following-paragraph relationships when valid.
Recognized styles named Heading 1 through Heading 6 map to the corresponding
adapter heading identities when doing so is unambiguous. Section and table
styles remain opaque until their normalized block roles are specified.

Style handles, rather than names or source order, provide stable identity.
The initial stable IDs are `RtfP<N>` and `RtfC<N>` for native Paragraph and
Character handles respectively, with `\s0` represented by `Paragraph`.
The standard Heading 1 through Heading 6 actions resolve a unique matching
native heading style, or create one with an unused handle when absent. Renaming
a heading keeps its native handle and stable identity.
Effective RTF style selection follows the RTF formatting-state stream; it is not
treated as an unordered list of classes. Direct controls after style selection
become direct declarations over the affected formatted ranges.

#### RTF canonical writing and targeted edits

New RTF syntax and any formatting construct that must be regenerated use one
versioned canonical RTF 1.9.1 representation:

- balanced groups with deterministic control ordering and delimiters;
- standard `\sN` and `\*\csN` stylesheet entries for Paragraph and Character
  styles;
- `\sbasedonN` and `\snextN` when those relationships are present;
- direct formatting as the smallest balanced group or explicit property delta
  that scopes exactly to the intended range; and
- canonical escaped text/`\uN` output with the document's declared code-page
  and `\ucN` policy.

Existing font, color, and style-table entries retain their order, numbering,
spelling, and unused entries. A new font, color, or style uses a previously
unused handle/index and is appended canonically; existing entries are never
renumbered merely to compact a table. The body patch and required table patch
commit together.

Where safe, changing one supported direct property patches or inserts only its
control word and preserves unrelated or unknown controls in the same group. If
RTF state interactions make that ambiguous, the adapter wraps the selected
range in a new canonical group with an explicit override rather than
regenerating surrounding content. It rejects the edit if neither operation can
preserve the required group/destination semantics.

Editing a supported style definition may canonicalize that one style-definition
group. Unsupported controls inside that group are part of the declared
canonicalization boundary and may be removed; other style definitions and
header destinations remain original bytes. Descendant style groups are patched
only when the canonical representation materializes a changed dependent value.
Applying, removing, or changing a named style patches the smallest applicable
`\sN` or `\csN` body control region.

RTF's canonical Viem extension preserves semantic base weight with
`\viemweightN` and OpenType features with scoped private control words. These
controls are ignored by conventional RTF readers; ordinary `\b` and other
standard controls provide the interoperable appearance fallback. Viem must retain
feature state through partial clearing, named styles, `\plain`, and reopening.
HTML owned version-1 style metadata accepts additive `character-bold` and
explicit generated-style deletion declarations; inline base-weight/bold helpers
must not leak into the owned-sheet fallback grammar.

### HTML and RTF conformance tests

Each adapter has corpus, property, and targeted golden tests. At minimum:

- Opening and immediately saving complex browser/author-generated HTML and
  Word/other-writer RTF is byte-identical, including malformed syntax, mixed
  encodings, unknown controls, comments, duplicate/oddly quoted attributes,
  scripts, arbitrary CSS, ignorable destinations, binary data, fields, and
  embedded-object groups.
- A targeted body-text edit changes only its declared source range and required
  escaping bytes. Surrounding tags, attributes, scripts, CSS, RTF controls,
  destinations, tables, and original whitespace remain byte-identical.
- Editing text inside `<b foo="bar">…</b>` preserves both tags and `foo`.
  Changing the bold formatting exercises the declared HTML
  canonicalization boundary and produces the canonical expected markup.
- Supported inline CSS properties project to the correct normalized direct
  declarations; unsupported properties remain unchanged and have no layout
  effect. Multiple applicable Viem class tokens select the first, while
  unrelated class tokens survive assignment changes.
- Creating, renaming, rebasing, editing, applying, and deleting canonical
  Viem HTML class styles update only the owned style rules, affected
  assignments, and materialized dependent rules. Reopening reconstructs the
  same stable identities, sparse declarations, parent/next links, and effective
  values.
- Native HTML style tests cover blank List Level 1 assignment followed by
  typing, grouped list containers, existing numbering and nesting, empty
  paragraph anchors, native heading/code assignments, exact undo/redo, source
  patch locality, and clean-reopen equivalence. Export tests exercise both
  inclusion settings, saved defaults, custom styles, direct formatting, and
  omission of browser-default CSS declarations.
- RTF tests cover nested state, `\plain`/`\pard` resets, font/color tables,
  Unicode and code-page text, `\par`/`\line`, paragraph and character style
  definitions, based-on/next relationships, and unknown controls adjacent to
  supported ones.
- Editing an RTF style definition changes only its declared style group and
  necessary dependent/table patches. Applying a style or direct property
  preserves unrelated group state and opaque destinations.
- No test may observe script, field, macro, object, external resource, or
  embedded payload execution or network/file access.
- Incremental parsing/projection after every edit equals a clean projection of
  the patched source, and undo/redo restores exact source bytes and formatted
  state.

## Required user-visible behavior

### Rich text and typography

- The formatted document MUST support style spans over arbitrary text ranges.
- At minimum, a resolved text style contains font family/fallback request,
  size, weight, slant, foreground color, underline/strike state, language,
  writing direction override, and OpenType feature settings.
- Adjacent equivalent spans SHOULD be coalesced. Text and style intentions are
  reverse-projected and committed as undoable source transactions.
- Shaping MUST support proportional advances, kerning, ligatures, combining
  marks, emoji sequences, font fallback, and mixed font sizes on one row.
- Unicode bidirectional text MUST be shaped and drawn correctly. Logical
  formatted-text order remains the basis of search, registers, and semantic
  edit intentions; hit testing and left/right visual movement use resolved
  visual order and affinity. Source storage retains its own byte order.
- A visual row's ascent, descent, and leading are derived from all fragments on
  that row. Rows are not assumed to have a uniform global height.
- Internal register operations SHOULD preserve formatted structure and rich
  styles together with enough portable semantics for the destination adapter
  to translate a paste. WYSIWYG system Copy publishes plain text, macOS rich
  text (RTF), and a versioned private Viem fragment containing selected source
  bytes, pipeline metadata, and resolved styling. Compatible contiguous private
  pastes reconstruct the selected source through verified local transactions.
  Rectangular copies retain the exact source fragments for their selected
  segments separately, without including intervening unselected source;
  rectangular paste retains its plain-text editing behavior. Source
  views copy the original selected source as plain text only. Copy Source
  (`Shift-Command-C`) publishes only plain text containing the source fragment
  corresponding to the selection, including markup in WYSIWYG views. Paste
  and Match Style ignores the private fragment and uses plain text. Malformed
  external private data cannot prevent ordinary commands or plain-text paste.

### Application theme and settings

The theme is an application preference, shared across views and independent of
source-backed named styles. It defines text foreground and canvas background,
caret and selection colors, status foreground/background and font family/size,
and top/left/right/bottom document-edge padding. Color values are portable sRGB.
The macOS Settings window has a category sidebar, including Documents and Theme,
with a live preview, native color controls, typography controls, edge-padding
controls, presets, and restore-defaults action.

A missing foreground or canvas color means **Default**, resolved through the
theme at painting time. Generated base styles leave these colors unspecified.
An explicit document color, including black or white, takes precedence and is
not confused with Default. Default is not serialized as an explicit theme color.
Theme changes never modify source bytes, style declarations, history, or dirty
state. Color changes require only repainting. Theme padding belongs to document
coordinates, so scrolling carries it off the visible edge; it is additive with
explicit document-style padding and invalidates only affected view geometry.
Padding changes preserve viewport anchors and keep large-document layout local.

Application preferences have one versioned JSON authority at
`~/.viem/config.json`. Theme, Smart Quotes, and status-bar visibility use this
store; Settings controls write the same values. Valid legacy preferences migrate
once. Reads validate the complete configuration, writes are atomic, and unknown
keys survive updates. Invalid or unsupported versions are reported without
overwriting the user's file. `VIEM_CONFIG_DIR` may override the directory for
isolated development and testing.

Format defaults live beside it in `text_style.json`, `html_style.json`,
`markdown_style.json`, and `rtf_style.json`. A document loads the matching sparse
style defaults before source declarations are applied. The cascade is built-in
styles, user format defaults, source definitions/assignments, then direct
formatting. An inherited default remains unset in source: changing an unrelated
property must not serialize an inherited font, color, or other declaration.
Explicit assignment of a custom default style materializes only declarations
needed to represent that assignment in a source-backed format. Source and
WYSIWYG variants share their format's defaults. Loading defaults is presentation
configuration and never changes source bytes, dirty state, or undo history.

Format > Style contains Edit document style and Save as default <format> style.
HTML also exposes Include style definitions in file, as specified above.
Saving defaults exports the current style configuration to the corresponding
JSON file. Existing open buffers keep their current configuration; subsequently
opened buffers load the saved defaults.


### Continuous canvas and paragraph layout

The editor canvas is an unpaginated continuous surface. It has a finite usable
width determined by the view width, view-owned chrome/gutter insets, resolved
Document-style padding, and zoom, and an unbounded logical vertical extent
represented through the estimated/exact height index. The resolved Document
Canvas background paints the content canvas; the document's inherited Character
declarations establish its default font and text colors. Pages, page breaks,
headers, footers, columns, footnotes, and widow/orphan rules do not participate
in interactive layout. A future print or export feature may build a separate
paginated projection without changing the interactive document or serializing
visual wraps.

Paragraph layout follows the resolved paragraph style:

- Start and end are logical edges resolved using the paragraph's base writing
  direction. The first-line indent is relative to the resolved start indent;
  negative values provide hanging indents.
- The available width for a paragraph is the canvas usable width minus its
  resolved start and end indents. Structural block contributions may further
  reduce or offset that box in the future.
- Space between adjacent paragraphs is the sum of the first paragraph's space
  after and the second paragraph's space before. Spacing does not collapse.
  Document top and bottom padding are separate from paragraph spacing.
- `normal` line spacing uses the maximum shaped ascent, descent, and leading on
  each visual row. A multiplier scales that natural row height. `at-least`
  takes the greater of the natural and requested heights. `exact` uses the
  requested advance while retaining unclipped ink bounds for damage and
  accessibility geometry.
- Start, end, and center alignment position the shaped visual row inside the
  paragraph content box. Justification is unsupported until its breaking,
  expansion, hit-testing, and editing behavior is specified.

Paragraph spacing and indentation affect wrapping and the view height index but
never create source newline characters. Canvas width, view chrome inset, or
Document padding changes invalidate view wrap plans and paragraph geometry, not
source, syntax, unrelated style assignments, or width-independent shaping.

### Wrapping and resize reflow

- Wrapping is a per-view option. Soft wraps are layout artifacts and MUST NOT
  insert, remove, or serialize newline characters.
- `wrap` controls whether soft wrapping is active. Wrapped text always uses
  Unicode-appropriate word/line-break opportunities at the usable text width;
  there is no separate `linebreak` option or character-wrapping mode.
- If no legal word boundary fits, the complete unbreakable segment overflows
  to the right through its next legal break or hard-line end. Neither grapheme
  boundaries nor internal layout-cache boundaries may split that segment.
- Changing the window size, side insets, gutter width, zoom, or any other value
  that changes usable text width MUST reflow the text for that view.
- Resize reflow MUST update visible rows synchronously for the next frame. It
  MUST NOT reshape unchanged text merely because the width changed; it should
  reuse width-independent shaped fragments and recompute line breaks.
- Off-screen wrap results and height aggregates may be recomputed lazily after
  a resize. Scrollbar extent may temporarily use estimates, but visible text,
  hit testing, selection, and the caret must always use exact layout.
- During reflow, preserve a stable text anchor at the top of the viewport and
  keep the active caret visible. Do not preserve a stale numeric scroll offset
  if doing so would make the user's text jump unpredictably.
- With wrapping off, hard lines do not reflow when the window narrows. The view
  scrolls horizontally and otherwise behaves like gVim.

### Meaning of "line" and per-view line mode

Each view has a portable line-mode policy, initially **Visual**. Clicking the
status-bar location toggles Visual (an eye icon) and Physical Source (a file
icon). These are original vector icons. The mode is independent of `wrap` and
is not persisted in source. RTF does not expose Physical Source mode; changing
a buffer to RTF returns any physical-mode views to Visual.

- Visual mode counts the exact displayed rows, including soft wraps. Without
  wrapping, these are formatted hard lines. Physical Source mode counts the
  shared line-ending projection's source-line tokens, including invisible
  syntax and comments in text formats.
- Standalone and operator-pending `j`/`k`, line start/end motions, doubled line
  operators, `C`/`D`, line-oriented insertion, and Visual Line use the selected
  mode. Counts, registers, replay, dot repeat, and undo retain that policy.
- Explicit `g` visual-row motions keep their visual meaning. Ex addresses and
  ranges retain their explicit formatted hard-line domain.
- Native Select All selects the entire formatted document through its exact
  terminal boundary, independently of Visual/Physical Source line policy or
  the previously active selection mode. Wrapped rows and internal breaks in
  the last paragraph are included.
- Visual vertical motion retains desired x and uses shaping caret stops;
  physical motion retains its source column and a checked source position when
  the destination has no visible text. Source positions must be invalidated or
  explicitly remapped on revision changes, never interpreted in a new snapshot.
- Deleting a visual row deletes its formatted content. An interior wrapped row
  does not acquire an invented newline or paragraph boundary. A complete rich
  paragraph/list-item deletion is structural and consumes its generated label.
  A label-only visual row has an explicit list-clearing operation. Deleting a
  physical line removes its authoritative source extent, including delimiters
  and markup, and reparses. Neither operation guesses a contiguous reverse map.
- Physical line registers preserve source spelling; visual fragments use
  characterwise extents unless an actual complete hard-line extent was selected.
- Location reporting must not lay out all preceding text. When a global visual
  ordinal is not known exactly, display exact `hard line · visual row` instead.
  Physical locations use exact source-line and source-column ordinals.

### Modes and caret

Required stable modes are:

- Normal mode (also called command mode in product language);
- Insert mode;
- Replace mode;
- Visual Character, Visual Line, and Visual Block modes; and
- Command-line mode for `:`, `/`, and `?` input.

Operator-pending, prefix-pending, register-pending, and single-Normal-command
from Insert mode are explicit transient states in the core command machine.
Escape cancels a transient state without leaving a partially applied edit.

The frontend renders mode from core state:

- Normal and Visual modes: a filled character block around the associated
  grapheme, shaping cluster, or atomic object under the cursor;
- Insert mode: a thin vertical insertion caret at the caret stop;
- Replace mode: an underline or low horizontal bar under the cluster that will
  be replaced; and
- Command-line mode: a thin vertical insertion caret in the command line.

These mode-to-appearance rules, the custom block and underline rendering rules,
focus behavior, and caret-color policy are portable requirements. A future
Windows frontend follows them using Windows-native facilities where suitable.
The decision to use `NSTextInsertionIndicator` for a vertical caret is specific
to the macOS frontend and is not part of the core or Windows contract.

The block caret is custom rendered; do not attempt to stretch a platform's thin
insertion indicator into a block. Its logical extent is the associated atomic
content item selected by the cursor's boundary affinity. Its visual extent uses
the exact selection/highlight geometry returned by the current layout for that
item. It is not a fixed monospace cell. When several graphemes form a visually
indivisible shaping cluster, the block uses the containing cluster geometry
without changing the logical cursor position or the range a command will edit.

For ordinary monochrome text, draw the active block with an opaque caret-color
fill, then redraw the covered shaped glyph fragment clipped to the block in a
color chosen for accessible contrast with that fill. This may reuse the text
renderer's selection pass, but it must not create a logical selection. For a
color glyph, emoji, embedded object, or other content that cannot be recolored
faithfully, use a translucent caret-color fill with a solid caret-color outline
so the original content remains recognizable. Normal and Visual modes use the
same block treatment; selection painting must still make the active endpoint
unambiguous.

The block redraw includes ink from every intersecting shaped fragment, including
neighboring italic overhangs and ink from tightly spaced rows. The overlap query
allows a device-pixel antialiasing fringe around reported outline bounds. The
block's logical selection geometry does not expand to those ink bounds.
Native painting culls clusters against the damaged visible region using ink
and typographic bounds, including the antialiasing fringe. Offscreen context
retained for shaping a long paragraph must not be redrawn on every caret blink.
Paint colors are resolved once per run; cached colors preserve transparency,
synthetic stroke, and native color-glyph behavior.

The Replace caret is custom rendered as a solid caret-color underline or low
horizontal bar spanning the same associated-item geometry. An empty hard line,
empty document, or end-of-line position has explicit block and underline
geometry based on the current caret height and a minimum width derived from the
active font's en width (one half em); never assume a glyph exists under the
caret. Empty documents use the current resolved typing font, not a status or
command-line font.

Leaving Insert mode preserves a final empty hard line/paragraph and places the
Normal block caret at its existing empty boundary. It must not move backward
over the preceding paragraph separator.

When an editor view is not the active first responder, its caret is a
nonblinking hollow outline in the same geometry instead of a filled block,
vertical insertion indicator, or underline. An active caret blinks according to
the platform's text-cursor and accessibility preferences where those are
available. Processing a key or changing the caret position makes it visible and
restarts the applicable idle/blink behavior.

Caret and selection colors belong to the application theme. Every custom
caret and native vertical insertion indicator uses the same theme caret color.
For recolored monochrome glyphs, convert sRGB components to linear light and
compute relative luminance `0.2126 R + 0.7152 G + 0.0722 B`. Choose black or
white according to whichever has the higher contrast ratio against the opaque
caret fill; do not use a 50% brightness threshold. Theme changes redraw visible
carets without changing document state or width-independent text shaping.

### Visual Block with proportional text

Visual Block is a display-space rectangle, because character columns are not
meaningful with proportional fonts. It is stored as the `BlockSelection`
defined by the position algebra: stable top/bottom row anchors and left/right
layout x coordinates. Resolve it only against an exact layout snapshot by
hit-testing each visual row into a tagged, ordered `RangeSet`; do not flatten it
to one character range or retain stale row numbers. Block edits reverse-project
that complete range set as one atomic source transaction. When wrapping or
width changes during an active block selection, resolve the stable row anchors
again and recompute the row intersections from the stored x edges.

## Vim command surface

This section defines the initial required command set. Standard Vim counts and
the optional register prefix apply wherever Vim permits them. Unsupported
commands must report a non-destructive error; they must not silently do
something approximately similar.

### Command grammar

The Normal-mode parser MUST support:

- `[count]command`;
- `"{register}[count]command`;
- `[count]operator[count]motion`, with the two counts multiplied;
- doubled operators for hard-line operations;
- multi-key prefixes such as `g` and `z`; and
- a clean cancellation path for Escape at every pending stage.

Commands resolve to typed motions or actions before document mutation.
Operators consume a motion result tagged as characterwise, hard-linewise,
visual-row characterwise, or visual-block. Avoid implementing every
operator-motion pair as a separate command.

### Normal-mode movement

Required movements are:

- basic: `h`, `l`, `j`, `k`, arrow keys, Space, Backspace;
- visual-row: `gj`, `gk`, `g0`, `g^`, `g$`;
- hard-line: `0`, `^`, `$`, `g_`, `|`, `+`, `-`, Enter;
- words: `w`, `W`, `e`, `E`, `b`, `B`, `ge`, `gE`;
- character find: `f{char}`, `F{char}`, `t{char}`, `T{char}`, `;`, `,`;
- document: `gg`, `G`, `{count}G`, `{count}%`, Control-Home, Control-End;
- structure: `%`, `(`, `)`, `{`, `}`;
- viewport: `H`, `M`, `L`, `Ctrl-F`, `Ctrl-B`, `Ctrl-D`, `Ctrl-U`,
  `Ctrl-E`, `Ctrl-Y`, `zz`, `zt`, `zb`; and
- marks/jumps: `m{a-z}`, `` `{mark} ``, `'{mark}`, `Ctrl-O`, `Ctrl-I`.

Sentence and paragraph motions should use Unicode-aware human-language rules
with Vim-compatible blank-line behavior. Their exact segmentation rules must
be test fixtures, not ad hoc calls to a frontend API.

Control-Home and Control-End move to the document's first and last legal text
points, independent of the Visual/Physical line setting. Insert and Replace
retain their mode; active Visual selections extend to the destination. In a
command prompt, these keys move to the prompt's beginning or end.

### Search

Required search commands are `/pattern`, `?pattern`, `n`, `N`, `*`, `#`,
`g*`, and `g#`. Search operates on logical UTF-8 text in the formatted
projection and is independent of wrapping. Matches may cross style boundaries.
Search/replace changes are reverse-projected like other edits. The first
implementation uses the Viem Regex v1 dialect below rather than Vim's full
regular-expression language. Search history belongs in core state.

#### Viem Regex v1

Viem Regex v1 is the sole pattern language for `/`, `?`, operator-pending
searches, `:substitute`, and any later command documented as accepting a search
pattern. It is a stable product interface, not an alias for whatever syntax a
particular regex library version happens to accept. The implementation may use
the Rust `regex` crate or another engine only when it produces the behavior
defined here.

Patterns are valid UTF-8 and Unicode mode is mandatory. The matching atom is a
Unicode scalar value, not a source byte, UTF-8 code unit, grapheme cluster,
shaping cluster, glyph, or terminal cell. Disabling Unicode or enabling a byte
mode is unsupported. Pattern source is retained exactly for history, repeat,
register inspection, diagnostics, and any future persistence; compiled engine
state is disposable.

The supported pattern syntax is:

- literal Unicode scalar values and a backslash escape for a metacharacter;
- `.` for one scalar other than U+000A;
- bracketed classes, negated classes, scalar ranges, and nested class union,
  intersection (`&&`), difference (`--`), and symmetric difference (`~~`);
- Unicode-aware `\d`, `\D`, `\s`, `\S`, `\w`, and `\W`;
- Unicode general-category, script, binary-property, and property-negation
  forms such as `\p{Letter}`, `\p{Script=Greek}`, and `\P{Whitespace}`;
- greedy quantifiers `?`, `*`, `+`, `{n}`, `{n,}`, and `{n,m}`, with a trailing
  `?` for the corresponding lazy form;
- capturing groups `(pattern)`, noncapturing groups `(?:pattern)`, named groups
  `(?P<name>pattern)` and `(?<name>pattern)`, and alternation `|`;
- Unicode word-boundary assertions `\b` and `\B`;
- `^` and `$` as zero-width formatted hard-line start and end assertions at
  every pattern position, and `\A` and `\z` as logical-document start and end;
- escapes `\t`, `\r`, `\n`, `\xNN`, and `\u{scalar-value}`; and
- inline or scoped `i`, `s`, and `x` flags, such as `(?i)word`, `(?-i:Word)`,
  `(?s:.)`, and `(?x: a \s+ phrase )`. `s` permits `.` to match U+000A and
  `x` ignores unescaped pattern whitespace, including inside a class, and
  treats an unescaped `#` outside a class through the next pattern U+000A or
  pattern end as a comment.

No other inline flag is part of version 1. In particular, `m` is unnecessary
because `^` and `$` always use formatted hard-line semantics, and an inline
flag may not disable Unicode mode. A literal `^` or `$` is written `\^` or
`\$`. An unsupported escape or construct is an error; it is never treated as
literal merely because the selected engine would do so.

The following Vim pattern families are deliberately unsupported:

- magic and case switches `\v`, `\m`, `\M`, `\V`, `\c`, and `\C`;
- escaped capture/group, alternation, conjunction, and quantifier spellings
  such as `\(`, `\)`, `\%(`, `\z(`, `\|`, `\&`, `\+`, `\=`, `\?`,
  `\{n,m}`, and shortest-match `\{-...}` forms;
- pattern backreferences `\1` through `\9` and syntax-highlight external
  captures `\z1` through `\z9`;
- lookaround and atomic postfixes `\@=`, `\@!`, `\@<=`, `\@<!`, bounded
  lookbehind forms such as `\@123<=`, and `\@>`;
- match-boundary controls `\zs` and `\ze`;
- Vim keyword boundaries `\<` and `\>`; Viem's `\b` and `\B` use the
  Unicode regex word definition and are independent of word-motion tailoring;
- end-of-line-inclusive `\_x` forms, including `\_.`, `\_^`, `\_$`,
  `\_[...]`, and `\_`-prefixed character classes;
- buffer-relative assertions beginning with `\%`, including document,
  Visual-selection, cursor, mark, line, byte-column, and virtual-column
  assertions;
- other `\%` forms including `\%[...]`, numeric character escapes,
  `\%C`, and Vim regex-engine selection with `\%#=`;
- the Vim class meanings of `\i`, `\I`, `\k`, `\K`, `\f`, `\F`, `\p`,
  `\P`, `\x`, `\X`, `\o`, `\O`, `\h`, `\H`, `\a`, `\A`, `\l`, `\L`,
  `\u`, and `\U`, plus Vim-only bracket classes such as `[:ident:]`,
  `[:keyword:]`, and `[:fname:]`; Viem accepts only the separately specified
  `\A`, `\p{...}`, `\P{...}`, `\xNN`, and `\u{...}` forms and meanings;
- `\Z` combining-character-insensitive matching, Vim equivalence classes,
  Vim collation elements, and Vim's automatic composing-character inclusion;
  and
- `~` as the previous substitute string. In Viem Regex v1, `~` is literal.

Some accepted spellings intentionally differ from Vim and therefore require
specific compatibility tests and documentation:

- `\b` is a Unicode word boundary, not a Backspace character;
- `\A` is logical-document start, not Vim's nonalphabetic class;
- `\w`, `\d`, and `\s` and their negations are Unicode-aware rather than
  Vim's ASCII- or option-specific classes; and
- `^` and `$` are hard-line assertions wherever they occur, rather than Vim's
  context-sensitive magic tokens.

A compatibility validator must recognize the complete unsupported families
before engine compilation. It must return `UnsupportedRegexAtom` naming the
first offending atom even when the underlying engine would accept that
spelling with a different meaning. A substring blacklist is insufficient.

#### Matching domain and hard lines

The matcher consumes the flat logical UTF-8 representation of one exact
formatted projection snapshot. Each semantic hard-line item contributes one
U+000A, as defined by the position algebra. A literal U+000A formatted scalar
has the same regex scalar value but retains different typed provenance for any
later edit. Soft wraps contribute nothing. Style boundaries contribute
nothing. An atomic object contributes its object-kind-specific plain-text
search representation.

By default `.` does not match U+000A; `(?s:.)` does. `\n` matches U+000A,
whether it represents a hard-line item or literal formatted content. `^` and
`$` consult formatted hard-line structure rather than testing source bytes or
assuming every U+000A is a hard line. `\A` and `\z` refer to the logical
formatted document, not one source-artifact part.

Search can cross character-style, direct-formatting, paragraph-style, and
source-piece boundaries. It crosses a hard-line or paragraph boundary only
when the pattern explicitly consumes its U+000A or uses dot-all mode. A search
never sees HTML tags, Markdown delimiters, RTF controls, or other source syntax
that has no formatted representation.

An addressed `:substitute` uses the same matcher and logical stream. A selected
match must be wholly contained in the addressed hard-line span. Without `g`,
the first non-overlapping match whose start belongs to each addressed hard line
is selected; with `g`, every non-overlapping contained match is selected. A
multiline match belongs to the hard line containing its start and is never
selected a second time for a later line.

#### Case behavior

Matching is case-sensitive by default. When the corresponding options are
implemented, `ignorecase` makes the pattern case-insensitive and `smartcase`
restores case-sensitive matching when the pattern contains an unescaped
literal scalar with the Unicode `Uppercase` property outside a character
class. Character-class contents and property names do not trigger smart case.

The `i` or `I` flag on `:substitute` overrides the option-derived default for
the whole pattern. An inline `(?i)` or `(?-i)` then overrides that default in
its lexical scope. Case-insensitive matching uses Unicode simple case folding;
it does not perform locale-specific casing or normalization.

#### Grapheme-boundary safety

Raw matching is scalar-based, but it may not weaken the document's
grapheme-boundary invariants. A navigation result is usable only when its start
resolves to a legal `TextPoint`; the search continues past unusable candidates.
Search decoration may represent a scalar-level match inside a grapheme, but
drawing uses the containing shaping-cluster geometry and does not create a
cursor, selection endpoint, or editing range there.

Every source-changing selected match, including a zero-width match, must have
start and end at legal formatted extended-grapheme boundaries. Preparation
validates all selected matches before producing any patches. If one fails, the
entire command returns `RegexMatchSplitsGraphemeCluster` without changing the
source, histories, registers, cursor, selection, or undo state; endpoints are
never rounded or expanded. Iteration after an empty match advances to the next
legal logical boundary and must always make progress.

#### Substitute replacement language

The pattern and replacement are different languages. Viem Regex v1 defines
this replacement syntax for `:substitute`:

- ordinary Unicode text inserts itself;
- `&` and `\0` insert the complete match;
- `\1` through `\9` insert the corresponding numbered capture;
- `\g{name}` inserts a named capture;
- `\\` inserts one backslash and `\&` inserts one literal ampersand;
- `\t` inserts U+0009;
- `\n` inserts a literal U+000A formatted scalar with no hard-break marker;
  and
- `\r` inserts one semantic formatted hard-line item.

Captured text retains typed hard-line markers, object representations where
permitted, styles, and provenance when inserted; it is not flattened and then
re-inferred from U+000A bytes. A missing optional capture inserts nothing. A
reference to a capture not declared by the compiled pattern is an error.

Dollar-form references such as `$1`, Vim's previous-replacement `~`, case
conversion escapes, expression replacement with `\=`, literal control-key
spellings, and every unlisted backslash escape are unsupported in version 1.
They return `UnsupportedReplacementAtom`; an unknown escape never silently
drops its backslash.

#### Compilation, execution, and caching

Matching must be linear in the searched input for a fixed compiled pattern,
with an `O(m * n)` worst-case bound where `m` is compiled-pattern size and `n`
is searched-input length; no version-1 feature may add input-dependent
backtracking. The core enforces configurable pattern-length,
compiled-program-size, capture-count, and search-work limits. Resource-limit
failure and user cancellation are non-destructive structured results.

Compiled patterns may be cached by exact pattern text, dialect version, and
effective compile flags. Match results, incremental-search state, and search
decorations are bound to an exact projection snapshot and cannot be reused
after a relevant edit without a validated change map or rescan. Searching may
scan the requested document range, but an ordinary edit must not synchronously
rescan the whole document merely because match highlighting is enabled.

Required structured failures include `InvalidRegex`, `UnsupportedRegexAtom`,
`UnsupportedReplacementAtom`, `RegexResourceLimit`,
`RegexMatchSplitsGraphemeCluster`, `StaleProjection`, and `Cancelled`. Tests
cover every accepted construct, every unsupported Vim family, the accepted
spellings whose Vim meanings differ, Unicode properties and case folding,
decomposed graphemes, empty matches, hard lines versus literal U+000A, matches
across style/source-piece boundaries, multiline substitution, cancellation,
resource limits, snapshot invalidation, and atomic reverse-projection failure.

### Operators, changes, and insertion entry points

Required operators are `d`, `c`, `y`, `>`, `<`, `=`, `g~`, `gu`, and `gU`.
Required shorthand/change commands are:

- `dd`, `D`, `cc`, `C`, `yy`, `Y`, `>>`, `<<`, `==`;
- `x`, `X`, `s`, `S`, `r{char}`, `R`, `~`;
- `J` and `gJ`;
- `p`, `P`, `gp`, and `gP`; and
- `i`, `I`, `a`, `A`, `o`, and `O`.

Replacement operands retain whether a U+000A is literal content or a semantic
hard-line item; frontends MUST pass a normalized Enter key separately from a
text-input event. Normal `[count]r<Enter>` replaces `count` graphemes on the
current hard line with exactly one semantic hard-line item and leaves the
cursor at the start of the following line. Characterwise and linewise Visual
`r<Enter>` follow Vim's distinct behavior: every selected content grapheme is
replaced by a literal U+000D while existing semantic hard-line items are
preserved. Visual Block `r<Enter>` removes the selected content and inserts
exactly one semantic hard-line item in each nonempty selected visual row,
regardless of the rectangle width; empty or short rows with no selected content
remain unchanged. Outside HTML visual typing's whitespace normalization, a
literal CR or LF supplied through text input remains literal and may be rejected
atomically when the active source pipeline cannot
reverse-project it without changing the logical hard-line sequence. Dot repeat
retains this typed operand distinction.

The `=` operator initially performs deterministic indentation defined by core
configuration. It must not invoke a language-specific formatter implicitly.

Marks and searches are composable operator motions. `` `{a-z} `` uses the
mark's exact position as an exclusive characterwise motion, while `'{a-z}`
uses the marked hard line's first nonblank position and is linewise; counts do
not alter a named-mark destination. `/pattern`, `?pattern`, `n`, `N`, `*`, `#`,
`g*`, and `g#` are exclusive search motions and multiply operator and motion
counts where Vim does. An operator search records its jump only after the
operator succeeds. Escape from its command line cancels the complete pending
operator without changing text, registers, search state, or the jumplist.

### Text objects

Required text objects are:

- word/WORD: `iw`, `aw`, `iW`, `aW`;
- sentence/paragraph: `is`, `as`, `ip`, `ap`;
- quotes: `i"`, `a"`, `i'`, `a'`, `` i` ``, `` a` ``; and
- pairs: `i(`, `a(`, `ib`, `ab`, `i[`, `a[`, `i{`, `a{`, `iB`, `aB`,
  `i<`, `a<`.

Text objects operate on logical formatted content and are not changed by soft
wrapping or font metrics. Their ranges retain provenance for reverse edits.

### Insert and Replace modes

Required behavior includes ordinary Unicode text input, Escape/Ctrl-[, Enter,
Tab, Backspace, Forward Delete, arrow movement, Home/End, Page Up/Down,
`Ctrl-W`, `Ctrl-U`, `Ctrl-R {register}`, and `Ctrl-O {normal-command}`.

macOS marked-text/IME composition is required. An active composition is a
temporary marked range, updates visually as one composition, and commits as a
single source transaction/undo unit. Cancelled composition restores the
pre-composition projection without changing source. Escape/Ctrl-[ and AppKit's
cancel responder action discard input-context candidate state, including the
press-and-hold accent picker even when it has no marked-text range. Cancelling
an active marked range preserves its prior mode and source; dismissing a picker
must not delete the already committed base letter.
Core commands must not see partially decoded key events as text.

Insert/Replace source transactions are grouped according to the undo-unit rules
below. In particular, explicit cursor moves, `Ctrl-O`, paste boundaries, and
IME commits create deterministic undo breaks.

### Visual modes

Required commands are `v`, `V`, `Ctrl-V`, `Ctrl-Q`, `gv`, `o`, `O`, Escape, all supported
motions, supported operators, `x`, `s`, `r`, `J`, `~`, `u`, `U`, `>`, `<`,
`y`, `d`, `c`, `p`, and `P`. Visual selection is inclusive in Normal/Visual
Vim terms while internal APIs use explicit half-open ranges.

Backspace (the macOS Delete key) and Forward Delete delete the active selection
using the same operator, register, repeat, and undo behavior as Visual `d`, then
return to Normal mode. This includes mouse selections started from Normal mode
and character, line, or block selections. With no selection, Normal-mode
Backspace retains its leftward motion and Forward Delete retains `x` behavior.

### Registers, repeat, undo, and macros

- Required registers: unnamed (`"`), numbered delete registers `1`-`9`, yank
  register `0`, named `a`-`z`, append aliases `A`-`Z`, small-delete `-`, black
  hole `_`, system clipboard `+` and `*`, last-insert `.`, and current filename
  `%` where applicable.
- A text register payload explicitly records which U+000A byte offsets denote
  formatted hard breaks. An unmarked U+000A remains literal formatted content;
  adapters spell only marked breaks according to the destination pipeline.
  In a blockwise payload, U+000A separates display rows and is not itself a
  semantic hard break. Appending registers rebases these markers without
  inferring semantics from the payload text.
- A named register recorded as a macro stores its normalized key/text event
  program alongside, but separately from, its UTF-8 text-register projection.
  Macro playback uses the event program and never reparses human-readable
  inspection notation such as `<Left>`. Text-register inspection MAY render
  non-text events with that notation. When such a register is put, text and
  character events materialize directly, Tab materializes as U+0009, Enter as
  literal U+000D with no hard-break marker, Escape as U+001B, and Ctrl-letter
  or Ctrl-[ as its corresponding C0 scalar. Backspace, Delete, arrow, Home,
  End, and Page keys have no unambiguous formatted-text representation: an
  ordinary put, `:put`, or Insert-mode `Ctrl-R` containing one returns the
  structured `MacroContainsNonTextKeys` error without changing the document;
  inspection notation is never inserted as a substitute. Literal printable
  text such as `<Left>` in a text register remains literal and is never parsed
  as a key event. Overwriting a named macro register with text removes its
  event program; uppercase append preserves an existing program and appends
  the text payload as normalized character or semantic-break events.
- Required commands: `u`, `Ctrl-R`, `.`, `q{a-z}`/`q`, `@{a-z}`, and
  `@@`.
- Explicit `"*c` copies using the `*` clipboard register: in Visual mode it
  yanks the selection, and in Normal mode it accepts the same motion/count
  grammar as `"*y`. Other uses of `c` retain their change semantics.
- Normal-mode `U` is intentionally unsupported. There is no separate saved-line
  undo state; use `u` and `Ctrl-R` for undo and redo. Visual-mode `U` and the
  `gU` operator retain their uppercase-conversion behavior.
- Undo history follows the branching transaction model below. It is not a pair
  of linear command stacks.
- Dot repeat records a semantic change action with its inserted payload and
  count, not a replay of frontend-specific key codes.
- Macros record normalized core command/text events. Replaying a macro is
  deterministic and guarded against unbounded recursion.

### Undo history and transaction model

Undo and redo navigate previously committed document states. They MUST NOT
execute an inverse command, reverse a patch against the current document, or
rerun the original semantic intention. Reverse projection may depend on
context, format policy, and transformation versions, so rerunning it would not
be a reliable way to recover an exact earlier state.

Keep these concepts distinct:

- A **source transaction** is one atomic, verified transition from one source
  snapshot to another. It may contain patches to several ranges or artifact
  parts. A transaction either commits completely, together with its immediate
  command side effects, or has no effect.
- An **undo unit** is one user-visible change reversed by one `u`. It may
  contain several consecutive source transactions, such as the incremental
  updates made during one Insert session.
- A **history node** is the immutable, finalized result of one undo unit. The
  root node represents the state loaded or created before the first change.
- An **open undo unit** has a fixed parent and before-state, but its resulting
  snapshot is replaced as further source transactions join the group. It is
  finalized at an undo break. Intermediate source revisions remain valid for
  layout and background work but are not separate user-visible undo steps.

The history store records at least:

- a monotonically increasing change number and a stable history-node identity;
- one parent, zero or more ordered children, and a preferred redo child;
- the resulting immutable source snapshot, including source-artifact metadata
  required to interpret and serialize it exactly;
- the initiating edit's before and after restoration positions, represented as
  stable anchors with insertion association and boundary affinity rather than raw
  offsets;
- persistent before and after snapshots of required buffer-local marks;
- a typed summary of the semantic change for diagnostics and future history
  UI, without making that description responsible for undo or redo; and
- the exact transaction/change summaries needed for invalidation,
  instrumentation, and auditing. Retained patches are an optimization and
  diagnostic aid, not the authority for restoring the state.

A finalized node's source snapshot, restoration positions, mark snapshots,
semantic summary, and change summary are immutable. The tree index may append
children and may change a node's preferred-child link as the user navigates.
The root has no parent or initiating-edit fields.

Formatted projections, layout snapshots, native drawing objects, and cache
contents are never history state. They are regenerated or reused after a
history navigation from the selected source snapshot.

#### Creating and grouping undo units

The following grouping rules are required:

- One Normal, Visual, native editing, or Ex change command, including its
  count and every patch in a compound block edit, is normally one undo unit.
- A change operator followed by Insert input, such as `cw` or `cc`, groups the
  operator's deletion and the following Insert session into one undo unit.
- An `i`, `I`, `a`, `A`, `o`, `O`, or `R` session is one undo unit from entry
  until Escape, a required undo break, or another event below closes it. The
  line creation performed by `o` or `O` belongs to that same unit.
- Backspace, Forward Delete, Enter, and Tab remain in the current Insert or
  Replace undo unit when they are handled as ordinary editing input.
- An explicit cursor move in Insert or Replace mode closes the current unit
  before moving. Later text starts a new unit without requiring the user to
  leave the mode.
- `Ctrl-O` closes the current Insert/Replace unit before executing its one
  Normal command. A change made by that command is a separate unit, and later
  inserted text starts another unit.
- A paste or `Ctrl-R {register}` insertion is its own unit and closes adjacent
  typed-text units. One committed IME composition is likewise its own unit;
  marked-text updates and cancellation create no source transaction or history
  node.
- One dot repeat is one undo unit even when the recorded semantic change is
  compound. One macro replay or `:normal` invocation is also one undo unit
  containing all source transactions completed by its normalized commands. If
  a later command in the replay fails, earlier transactions remain committed
  in that unit and the replay stops with its diagnostic.
- A single `:substitute`, font/style action, or other command that changes many
  ranges is one atomic source transaction and one undo unit unless the command
  explicitly documents a different policy.

An undo/redo request, buffer close, save, mode transition that ends editing, or
dispatch of an unrelated change first finalizes any open undo unit. Active IME
marked text is cancelled before history navigation; it is never implicitly
committed by undo or redo. A failed or unsupported source transaction does not
alter the open unit. A successful command that produces no authoritative
source or source-metadata change does not create a history node. Navigation,
selection, scrolling, presentation-only option changes, register inspection,
yank without deletion, search, and setting a mark are not undo units. A
source-affecting option such as `fileformat` follows its documented source
transaction and undo policy.

#### Branching, undo, and redo

The buffer owns one history tree and one current-node pointer:

- Committing a finalized undo unit adds a child to the current node and moves
  the pointer to that child. Existing children are retained, so editing after
  undo creates a sibling branch rather than destroying the abandoned future.
- `u` moves to the parent, and `Ctrl-R` moves to the preferred child; their
  Normal-mode counts repeat the operation. `:undo` and `:redo` perform one
  corresponding step. `:undo {change-number}` selects that exact history node.
- Moving to a parent records the child just left as that parent's preferred
  redo child. Creating or explicitly selecting a child also makes it preferred.
  Thus immediate undo/redo is predictable even when the parent has branches.
- If the requested parent or preferred child does not exist, the command
  reports a non-destructive boundary error and leaves all state unchanged.
- Core history APIs enumerate branches and select a node by stable identity or
  change number even if the first frontend does not yet expose all of Vim's
  time-navigation commands. Selecting a node is atomic and updates the same
  preferred-child links as stepwise navigation.
- A read-only history status query reports the current change number, whether
  undo and redo are available, and the semantic summaries of the parent and
  preferred child so frontends can label and enable native menu items without
  duplicating history state.
- Redo installs the stored child snapshot; it does not reapply patches, update
  delete/yank registers, request format policy, or rerun command side effects.

History navigation first resolves and validates its target, then installs its
source snapshot, regenerates or obtains the matching projection, updates the
current-node pointer and saved/dirty state, remaps live anchors, and publishes
one coherent state change. If the target cannot be made usable, nothing is
installed.

#### State restored by history navigation

Undo history is buffer-owned, while cursor, selection, viewport, and mode are
view-owned. On undo or redo:

- the exact source artifact and source metadata in the target node are
  restored;
- required buffer-local marks are restored from the transition's before or
  after mark snapshot, matching Vim's treatment of marks as saved with text;
- the invoking view exits transient/Visual state to Normal mode, clears its
  selection, and places its cursor at the transaction's before restoration
  position for undo or after restoration position for redo;
- every other live view of the buffer retains its mode and presentation state,
  while its cursor, selection, and viewport anchors are remapped through source
  provenance to the restored snapshot; and
- each view invalidates only the projection and layout dependencies reported by
  the installed history transition and then reveals its remapped caret when
  required.

Registers, macro recording/playback state, dot-repeat state, search pattern and
history, jump lists, command-line history, view options, scroll offsets, and
the file identity are not restored and redo does not replay their original
side effects. Register and repeat updates caused by an ordinary edit still
commit atomically with that edit: if the edit fails, those side effects do not
occur. This atomicity does not make them part of the later undo payload.

#### Save points, retention, and persistence

A successful write records the current source-snapshot identity as the
buffer's persisted save point and may annotate the current history node with a
monotonic write number. Saving does not create an undo unit. A failed write
does not move the save point. The buffer is clean exactly when its current
source-snapshot identity is the persisted identity. Configuration-only history
nodes share that source identity and remain clean; a source-changing edit makes
it dirty and returning to the exact saved source snapshot makes it clean again.
Dirty-state queries compare retained identities in constant time, never
materializing or hashing the document. An asynchronously completed save carries
the captured source identity even when its history node has been pruned.
Changing file identity with a successful `:saveas` is not undone.

History retention has configurable node and retained-byte budgets. The byte
budget includes retained source, derived projections and their materialized
caches, position maps, and history bookkeeping. A conservative heap estimate
is acceptable; serialized source-byte length alone is not. Structurally shared
allocations are charged once, and retained-memory accounting for a new
persistent snapshot must not traverse unchanged shared subtrees. Source-buffer bytes remain a separate
diagnostic metric. Pruning removes the oldest non-current leaf
branches first. If the retained current ancestry alone exceeds the budget, the
oldest retained state is promoted to a new root, making older changes
explicitly unavailable without affecting the current document. The active
node and an open unit's parent and result are never pruned. A pruned saved node
may lose its navigable history entry, but the persisted snapshot identity and
artifact digest remain available for dirty-state comparison.

The first release keeps undo history in memory only. A future persistent undo
file must be versioned and bound to an exact physical source-artifact digest,
adapter identity/version, encoding configuration, and required transformation
configuration. A mismatch must reject the history without changing the opened
document.

### Command-line and Ex commands

Required command-line editing includes left/right movement, Home/End,
Backspace/Delete, history Up/Down, Escape, and Enter. Prompt text supports
mouse selection, Shift+arrow selection, and the native Cut/Copy/Paste/Select All
commands. Prompt edits use validated prompt identity and never target the
document behind the prompt. Command output replaces the prompt with selectable,
read-only text and an explicit close button. A new `:` replaces that output with
an editable prompt; ordinary editor input dismisses the output.

Filename arguments to `:edit`/`:E`, `:write`, `:saveas`, `:wq`, `:xit`,
`:split`/`:vsplit`, and `:cd`/`:chdir` support Rust-backed prefix completion.
Tab selects the first case-insensitive alphabetical match and cycles forward;
Shift+Tab starts with the last match and cycles backward. Matches use the
filename's actual spelling; directories include a trailing `/`, and directory
commands offer only directories. Relative paths use the application's working
directory; `~/` uses the current user's home directory. Dotfiles require a dot
prefix. Typing `/` immediately after a selected directory completion accepts
its existing slash, so the next completion searches inside that directory.
Other typing or caret movement accepts the suggestion and acts normally.
Ctrl-E restores the input before the active completion cycle, Ctrl-Y accepts
it, and Escape cancels the prompt. Completion never changes document history.

In Visual Character, Line, or Block mode, `:` opens an Ex prompt prefilled with
the selected logical hard-line range. The range is bound to that document
revision; an intervening edit in another view makes execution stale rather
than silently applying the old numeric addresses. Escape changes no source,
and `gv` can restore the remembered selection.

`:e`/`:edit` replaces the active pane after the usual unsaved-change review.
The case-sensitive `:E` opens a document in a new native window. `:pwd` displays
the working directory; `:cd`/`:chdir` changes the application's directory for
relative file requests. `:update` writes only a modified buffer. `:s` remains
substitute; filename-like misuse reports `:w` and `:saveas` as the save commands.
`:checktime` and window activation compare the bound artifact with the exact
last-loaded/saved fingerprint off the main thread. Changed, replaced, deleted,
or unreadable files produce non-destructive output. Ordinary saves guard against
external overwrite; `:e!` explicitly reloads and `:w!` authorizes overwrite or
recreation. A successful reload/save refreshes the baseline. Native saves may
ask for an explicit overwrite choice; automatic checks never reload a buffer.

The editor provides a native contextual Cut/Copy/Paste menu through right-click,
Control-click, and keyboard contextual-menu access. Wheel deltas use AppKit's
already preference-adjusted direction exactly once.

Required Ex commands and common unambiguous abbreviations are:

- files/views: `:edit`, `:enew`, `:split`/`:sp`, `:vsplit`/`:vs`,
  `:close`/`:clo`, `:write`, `:saveas`, `:quit`, `:qall`, `:wq`,
  `:xit`, `:wall`, and force `!` variants where meaningful;
- editing: `:undo`, `:redo`, `:delete`, `:yank`, `:put`, `:join`,
  `:copy`, `:move`, `:sort`, and `:normal` for the supported Normal command subset;
- search/change: `:substitute` with ranges and repeat flags, `:&`, and `:~`;
- navigation/info: numeric line addresses, `:goto`, `:marks`, `:registers`,
  `:jumps`, and `:pwd`; and
- options: `:set`, `:setlocal`, `:set wrap`, `:set nowrap`, `:set fileformat?`, and
  `:set fileformat=unix|dos|mac` (where one value is supplied), including the
  `ff` abbreviation and corresponding `:setlocal` forms. The global
  `fileformats` open-policy option supports query and ordered assignment even
  though changing it does not reinterpret an already open buffer.

Search switches `ignorecase` (`ic`), `smartcase` (`sc`), and `wrapscan` (`ws`)
are buffer-shared Boolean policy, defaulting to false, false, and true. Their
`:set` and `:setlocal` forms support enable/disable, toggle, query, and reset.
A compound option command validates atomically before publishing any changes.
They follow Regex v1 case rules and affect searches, repeats, and substitute;
`wrapscan` controls navigation wrapping. They do not change persisted source.

Ranges always use hard lines. File dialogs, unsaved-change prompts, and error
presentation are frontend responsibilities driven by typed core requests and
results. Write commands serialize the authoritative source artifact, preserving
unchanged source slices exactly; they never export a newly normalized formatted
document as a substitute for the source.

`:[range]sor[t][!]` defaults to all hard lines. It supports `i` (Unicode
case-insensitive comparison), `u` (remove duplicate full lines), mutually
exclusive `n`/`x`/`o`/`b` numeric keys, and an optional Regex v1 pattern. The
pattern skips through its match by default; `r` selects the match itself as the
key, and `//` reuses the last search without changing search history. Pattern
matching uses `ignorecase` but not `smartcase`. Missing numeric keys sort first;
numeric keys are signed, saturating 64-bit integers. Ascending order is stable;
`!` reverses it, including equal-key runs. Unique keeps the first resulting full
line under the selected case policy. `f`, locale `l`, count/register arguments,
and command chaining are not supported.

Sorting is one verified source permutation and undo unit; original encoding,
final terminator, and delimiter spellings are retained. Source-visible formats
sort literal displayed source rows and reparse their styling. WYSIWYG sorting
supports complete Markdown paragraphs with one hard line and balanced sibling
HTML `p`/heading elements, retaining each paragraph's source syntax and styles.
RTF sorting and ambiguous or split rich paragraph owners return an unsupported
result without changing source.

### Stacked document views

`:split`/`:sp` and `:vsplit`/`:vs` create stacked panes. With no filename they
create an independent view of the same buffer; with a filename they open or
reuse that document. Despite the familiar `vs` alias, side-by-side panes are
not supported. Each pane has its own cursor, selection, viewport, wrapping,
line-mode policy, status bar, and scrollbar. Split creation distributes the
available height and allows native divider resizing. Closing a pane keeps a
shared document alive in its other panes. Save, style commands, and validation
route to the focused pane. Closing the final view reviews unsaved changes.
Each distinct document being closed receives one native unsaved-changes review;
accepting Save or Delete/Don't Save must not trigger a second review. Cancel
keeps the window and its unsaved content available for a later close attempt.

Finder file drops target the receiving editor pane. The first dropped file
replaces that pane's document when it is clean, including an empty untitled
document. If the target has unsaved changes, the file opens in a new window.
Additional dropped files open in new windows. Opening failures preserve the
target; asynchronous opening rechecks the original pane identity and dirty
state before replacement. File drops open files without inserting their paths
into the prose or overwriting files on disk.

Document identity resolves standardized paths and symlinks and compares native
file identity for hard links. Opening an already represented file reuses the
same backend and history; it must not create divergent buffers for aliases.

### Editing-session locks and recovery

Named documents claim an exclusive recovery slot when opened or first named.
The preferred spelling is `.filename.viem.swp`, with numbered alternate slots
when occupied. A nonwritable source directory may use an application recovery
directory keyed by canonical target identity. Existing Viem slots and Vim
`.filename.swp` files trigger Open Read-Only, Edit Anyway, and Cancel choices;
Recover is available when a valid Viem snapshot is present. Foreign swap bytes
are never guessed or rewritten. A session only replaces/removes its own slot.

Read-only is a portable buffer policy: it allows editing, registers, and history,
but an unforced write reports E45. `:w!` and equivalent explicit force forms
permit the write. Native Save asks Save Anyway/Cancel before writing. Recovery
loads the saved full source plus format, encoding, and line-ending interpretation,
retains the original target filename, and starts dirty even at the undo root.
Only acknowledgement of a successful current Save/Save As clears that state;
failed or alternate writes do not.

After four seconds without edits, capture an immutable source snapshot and
write its complete bytes and interpretation metadata to the owned slot. Encoding
and I/O run on a utility queue; superseded jobs cannot overwrite newer snapshots.
The original source changes only on explicit Save, never autosave-in-place.
Create private temporary files exclusively, synchronize them, and atomically
replace the owned slot. Rebinding after Save As retains the last valid backup
until the new target's current snapshot commits. Failed opening or rebinding
must not cancel or delete an existing valid backup. Closing a document cancels
pending jobs and removes only owned slots; closing one shared pane is not a
document close. Crash leftovers remain available for recovery.

### Native macOS editing affordances

The macOS frontend supports mouse placement/drag selection, scroll gestures,
standard copy/cut/paste/select-all menu items, drag selection auto-scroll, and
font selection for the active range. Native commands dispatch the same core
semantic intentions and verified source transactions as keyboard commands.
They must not maintain a second selection, source, or undo model in AppKit.
The macOS Undo and Redo menu actions dispatch to the core history API. An
`NSUndoManager` adapter, if required for AppKit integration, is only a proxy for
core status and commands and never registers or executes independent inverse
closures.

### macOS main menu

The initial main-menu order is `Viem`, `File`, `Edit`, `Format`, `Paragraph`,
`Character`, `View`, `Window`, and `Help`. There are no `Navigate` or `Command` top-level menus.
Vim motions, mode changes, command-line entry, registers, marks, and macros
remain available through the Vim command grammar and any separately specified
UI; they are not duplicated into speculative menu hierarchies.

The menu hierarchy is:

- **Viem**
  - About Viem
  - Settings… (`Command-,`)
  - separator
  - Services (system supplied)
  - separator
  - Hide Viem (`Command-H`)
  - Hide Others (`Option-Command-H`)
  - Show All
  - separator
  - Quit Viem (`Command-Q`)
- **File**
  - New (`Command-N`)
  - Open… (`Command-O`)
  - Open Recent
    - dynamically listed recent documents
    - separator
    - Clear Menu
  - separator
  - Close (`Command-W`)
  - Save (`Command-S`)
  - Save As… (`Shift-Command-S`)
  - Duplicate
  - Rename…
  - Move To…
  - Revert To
    - Last Saved Version
    - Browse All Versions…
  - separator
  - Document Format…
  - Text Encoding
    - UTF-8
    - Latin-1
    - UTF-16 LE
    - UTF-16 BE
  - Line Endings
    - Unix (LF)
    - Windows (CRLF)
    - Classic Mac (CR)
  - separator
  - Page Setup…
  - Print… (`Command-P`)
- **Edit**
  - Undo *Action* (`Command-Z`)
  - Redo *Action* (`Shift-Command-Z`)
  - separator
  - Cut (`Command-X`)
  - Copy (`Command-C`)
  - Copy Source (`Shift-Command-C`)
  - Paste (`Command-V`)
  - Paste and Match Style (`Option-Shift-Command-V`)
  - Delete
  - separator
  - Select All (`Command-A`)
  - Select
    - Word
    - Sentence
    - Paragraph
    - Hard Line
    - Visual Row
  - separator
  - Find
    - Find… (`Command-F`)
    - Find and Replace…
    - Find Next (`Command-G`)
    - Find Previous (`Shift-Command-G`)
    - Use Selection for Find (`Command-E`)
    - Jump to Selection (`Command-J`)
  - separator
  - Transformations
    - Make Uppercase
    - Make Lowercase
    - Toggle Case
  - separator
  - Start Dictation…
  - Emoji & Symbols (`Control-Command-Space`)
- **Format**
  - Show Fonts (`Command-T`)
  - Bold (`Command-B`)
  - Italic (`Command-I`)
  - Underline (`Command-U`)
  - Strikethrough
  - Bigger
  - Smaller
  - Ligatures
    - Use Default Ligatures
    - Use All Ligatures
    - Use No Ligatures
  - Kerning
    - Use Default Kerning
    - Use No Kerning
  - Baseline
    - Superscript
    - Subscript
    - Raise
    - Lower
  - OpenType Features (available features of the current resolved font)
  - Show Colors
  - Text Color…
  - Highlight Color…
  - separator
  - Document Style
    - Base Document
    - dynamically listed named document styles
    - separator
    - Edit Styles…
  - separator
  - Paragraph
    - Alignment
      - Start
      - Center
      - End
    - Writing Direction
      - Automatic
      - Left to Right
      - Right to Left
    - Increase Indent
    - Decrease Indent
    - Paragraph Spacing…
    - Line Spacing
      - Normal
      - Single
      - 1.5 Lines
      - Double
      - Custom…
  - separator
  - Copy Style
  - Paste Style
  - Clear Direct Character Formatting
  - Clear Direct Paragraph Formatting
  - Clear All Direct Formatting
- **Paragraph**
  - Base Paragraph (`Command-0`)
  - Heading 1 through Heading 6 (`Command-1` through `Command-6`)
  - dynamically listed paragraph styles, including generated list levels
  - separator
  - Edit Styles…
- **Character**
  - Base Character
  - dynamically listed named character styles
  - separator
  - Edit Styles…
- **View**
  - Show Status Bar
  - separator
  - Word Wrap
  - Flow Source Paragraphs
  - Show Invisible Characters
  - separator
  - Zoom In
  - Zoom Out
  - Actual Size
  - separator
  - Enter Full Screen (`Control-Command-F`)
- **Window**
  - Minimize (`Command-M`)
  - Zoom
  - separator
  - New Window for Document
  - system-supplied window placement and tiling commands
  - separator
  - Bring All to Front
  - separator
  - dynamically listed document windows
- **Help**
  - system-supplied menu search
  - separator
  - Viem Help
  - Vim Command Reference
  - Keyboard Shortcuts
  - Supported Vim Commands
  - Document Format Compatibility
  - Round-Trip and Source Preservation
  - separator
  - Release Notes
  - Report a Problem…

The future Windows frontend uses `Control-0` through `Control-6` for the same
paragraph/heading assignments. Menu validation follows the focused pane and
current format capabilities. Native menu tracking must retain item identity and
must not rebuild the menu structure under a held mouse button; presentation
updates must preserve normal click-drag highlighting and selection.

Menu separators are presentation elements, not commands. Ellipses indicate
that the item opens a panel, sheet, chooser, or other interaction before taking
effect. Use standard macOS shortcuts and localized system titles where the
platform supplies them; do not repurpose a standard shortcut for unrelated
Vim behavior.

Standard Edit actions use native responder-chain selectors. When a settings,
style, or other native text field has focus, Cut, Copy, Paste, Select All, Undo,
and Redo operate on that field. When the editor surface has focus, its responder
adapts those selectors to the corresponding core commands and validation.

`Word Wrap` reflects the view-local `wrap` option. Resizing a wrapped view
reflows automatically and has no menu command. `Line Endings` is a radio group
in the File menu over the buffer's `fileformat` value and follows the verified
conversion rules in "Changing `fileformat`". `Text Encoding` is a File submenu
with the four supported encodings; the current encoding is checked and choosing
another encoding performs the same verified, undoable conversion as the core API.

`Flow Source Paragraphs` is a portable per-view option, initially off, available
in Markdown Source and HTML Source. It suppresses nonstructural physical line
breaks in layout while retaining source characters, editing coordinates, and
serialization. Semantic paragraph boundaries and preformatted code retain
their breaks. Toggling it changes only view layout; it does not change document
history or other views of the same buffer.

Zoom In and Zoom Out advance through 25, 33, 50, 67, 75, 80, 90, 100,
110, 125, 150, 175, 200, 250, 300, 400, and 500 percent, saturating at
25 and 500 percent. On macOS their shortcuts are Command-Equals and
Command-Hyphen. The portable checked scale API accepts intermediate scales
within that range; each control chooses the next strictly adjacent stop.

Menu validation comes from current core state and pipeline capabilities.
Actions that cannot apply to the current selection or adapter are disabled.
Rich-formatting actions are disabled for plain text; `Document Format…` may
offer an explicit conversion when an appropriate adapter exists. Style and
formatting items show a checkmark, mixed state, or no mark as appropriate.
Character and Paragraph menus reserve the same mark column for every item,
so labels align whether or not the item is checked. Active named styles remain
checked when menu validation refreshes their command state.
Undo and Redo use the core-provided action label. The Services, window
management, recent-document, open-window, and Help-search contents remain
system or dynamically supplied.

Bare Vim keys such as `i`, `v`, `.`, and `:` MUST NOT be registered as global
`NSMenu` key equivalents because they would interfere with text input and
mode-dependent command interpretation. Every editor-content menu action
dispatches the same typed core command or semantic intention as its keyboard
equivalent and does not create separate AppKit editing, selection, formatting,
source, or undo state.

### macOS style editor

Use [`docs/Word style.png`](<docs/Word style.png>) as the visual reference for
the style editor's overall density, labeled properties at the top, large
formatting area, bordered live preview, resolved-format summary, and bottom
action row. It is a composition reference rather than a behavioral or
pixel-exact template. Use native AppKit controls, metrics, typography, focus
rings, accessibility behavior, and current macOS window appearance. Do not
copy Word's `Format` section picker, template/Quick Style/automatic-update
checkboxes, or `Cancel` and `OK` buttons.

#### Window behavior and ownership

- Although it is colloquially a dialog box, the style editor is a modeless
  auxiliary window or panel. It is never an application-modal dialog or a
  document-modal sheet. The user can focus and edit any document while it is
  open.
- Its title is **Styles**, with the compact utility-panel title bar and window
  buttons used by the native font picker. Formatting controls, preview, and
  resolved summary have no section headings. Omit the inherited-formatting
  instruction, live-apply footer text, and separator above the Close button;
  relevant availability and error messages remain visible when needed.
- Exactly one style-editor window exists application-wide. Invoking any
  `Edit Styles…` action while it is closed creates it. Invoking one while it is
  open brings the existing window forward, retargets it to the invoking
  document, and selects the requested style by stable style ID.
- Merely moving the document caret or selection does not silently retarget an
  open editor. Retargeting occurs through an explicit Edit Style action or the
  editor's Style picker.
- The window has a **Close** button at the bottom trailing edge. It has no
  **Apply**, **Cancel**, or **OK** button because valid changes are already
  applied. The standard window close command and `Command-W` have the same
  effect when the style editor is key. Closing never rolls changes back.
- The window coordinator, current target document identity, and selected stable
  style ID belong to `src/mac`. Style definitions and mutations remain owned by
  core. The frontend never keeps a private editable copy of a style sheet.

#### Layout and common controls

From top to bottom, the content is:

1. a properties section containing:
   - **Style**, a pop-up that selects a style in the target document and groups
     Document, Paragraph, and Character styles;
   - **Name**, an editable text field;
   - **Style type**, a read-only value showing Document, Paragraph, or
     Character; and
   - **Based on**, a pop-up for the style's parent;
2. a native macOS tab row immediately below the name/base-style section, with
   **Character** and **Paragraph** tabs;
3. the controls for the selected tab;
4. a bordered, live preview using the real core style resolver and Core Text
   shaping path;
5. a scrollable, read-only summary of explicit declarations, inherited values,
   their contributing base styles, and the resulting effective properties; and
6. a bottom action row containing the **Close** button.

Style type is immutable after style creation. The Based on picker contains only
parents allowed by the selected style's namespace and role and excludes the
style itself and its transitive descendants. It cannot create an inheritance
cycle. Base Character and Base Document have no editable parent; Base
Paragraph's parent is fixed to Base Document. The distinguished base styles
remain editable where their declarations permit it, but cannot be deleted or
have their role changed.

Every property control must distinguish **Inherited** (no declaration at this
style layer) from an explicit value, including explicit normal weight, no
decoration, zero spacing, or transparent color. The UI shows the effective
inherited value while making it visually clear that the selected style does not
declare that value. A `Use Inherited` or equivalent action removes the
declaration rather than copying the current ancestor value into the style.

The first version of this two-tab editor changes Character and Paragraph
declarations. It does not edit Document Canvas background or padding; a future
Document tab or separate canvas UI must be specified before those properties
are exposed. Document-role styles may be selected to edit their allowed
Character declarations, but their Paragraph tab is disabled.

#### Character tab

The Character tab edits Character declarations. It is enabled for Character,
Paragraph, and Document styles because all three supported roles may contribute
default character appearance. For a Paragraph style, these controls edit the
paragraph's default character declarations rather than assigning a separate
Character style.

Expose controls for all initial Character properties:

- ordered font-family and fallback requests;
- font size in layout units;
- a native font-face picker (Regular, Light, Bold, Italic, etc.) and separate
  Bold and Italic toggles, with no generic numeric weight/slant fields;
- native foreground/background color swatches, including Default/Inherited;
- underline and strike decoration;
- language;
- writing-direction override;
- an original SVG feature button opening the selected font’s supported OpenType
  feature menu, with checkmarks and the same catalog as Format > OpenType Features;
- letter spacing; and
- baseline shift.

Character controls form compact grouped rows, with original consistent SVG
icons and separate B/I/U actions. Paragraph controls use corresponding alignment,
indentation, and spacing groups. The reference images guide density and grouping;
the interface must not be a tall generic attribute list.

Font face establishes the base weight/slant. Bold is a separate portable semantic
property: add 300 to the base weight (capped at 1000), then choose the next
available face at least that bold, falling back to the strongest available face.
Italic selects an intrinsic italic face when possible. Unsupported traits use
native synthesis. Applying or removing Bold must retain the chosen base face.
Source adapters preserve this distinction through their owned style metadata,
with conventional interoperable bold fallback where necessary.

Font-family fallback order requires an ordered editor rather than a single-font
field. Native font and color panels may be used as transient choosers, but they
must update the same selected style and must not become alternate persistence
or undo authorities.

#### Paragraph tab

The Paragraph tab is enabled only for Paragraph-role styles. It is visibly
disabled for Character and Document styles and cannot retain keyboard focus
when disabled.

Expose controls for all initial Paragraph Layout properties:

- space before and space after;
- logical start indent and end indent;
- signed first-line indent, including hanging indents;
- line-spacing kind (`normal`, multiplier, `at-least`, or `exact`) and the
  numeric value required by the selected kind;
- logical alignment (`start`, `center`, or `end`);
- base writing direction; and
- **Following paragraph style**, which edits `next_paragraph_style` and offers
  Same Style plus every compatible Paragraph-role style.

Controls that are meaningless for the chosen line-spacing kind are disabled
without erasing their last valid draft value. Only the active kind and its
applicable value form the committed declaration.

Numeric style fields have native up/down steppers on their right edge. A
step uses the displayed resolved value and creates an explicit declaration;
inherited reset remains available. Invalid drafts disable the stepper without
committing. Indents, paragraph spacing, baseline offsets, and tracking retain
their supported signed ranges. Held autorepeat is one continuous undo gesture.

#### Live application, preview, and undo

- Every valid control change immediately issues a typed style-definition
  intention against the exact current core snapshot. Successful reverse
  projection commits through the normal verified source transaction path and
  updates every view of the document without waiting for the window to close.
- The preview is not a private draft. It renders the currently committed style
  after cascade resolution. A Character style preview shows the style in
  representative surrounding text. A Paragraph style preview shows preceding,
  current, and following paragraphs so font, indentation, alignment, line
  spacing, and before/after spacing are visible together.
- A pop-up or button choice is one undo unit. Continuous gestures such as color
  dragging, stepping, or scrubbing apply live but coalesce into one undo unit
  per gesture. Contiguous typing in a text field applies each valid change live
  while coalescing according to the core's text-input undo grouping rules.
- An incomplete or invalid intermediate text-field value remains visibly local
  to that control and does not mutate core. Show an inline validation state and
  retain the last committed style value until the entry becomes valid.
- If an adapter rejects a style edit as unsupported, ambiguous, stale, or
  policy-dependent, commit nothing, restore the affected control from current
  core state, and present the structured diagnostic. Other controls and the
  document remain usable.
- Undo, redo, source reprojection, or another frontend action may change the
  selected style while the window is open. The editor observes style-sheet
  revision changes and refreshes its fields, inheritance state, preview, and
  summary from core without manufacturing another edit.

#### Selection validity and deletion

Deleting any non-base style that is in use reassigns its content to the
corresponding Base Paragraph or Base Character, rather than to the deleted
style's parent. Definitions, assignments, and supporting source metadata change
atomically and undo restores all of them. Generated heading/list definitions
must not silently reappear after reparsing; an adapter may persist an explicit
deletion marker in its owned schema. The three distinguished base styles remain
undeletable. Required tests cover local edits and reopen after deletion, source
locality, and deeper generated list levels.

The selected style is tracked by document identity and stable style ID, never
by menu index, name, or stale array position. On every style-sheet update and
before sending an edit, the window revalidates that identity against the latest
snapshot.

If the selected style was deleted from another action, reverse projection, or
history navigation, the editor must not apply a pending callback to the deleted
ID or to whichever style reused its former list position. It immediately
selects **Base Paragraph** in the same document, switches the style type and tab
enablement accordingly, and reloads every control and preview from that style.
Base Paragraph is the guaranteed default paragraph style and is undeletable.

If the target document closes, the editor retargets to Base Paragraph in the
current key document when one exists; otherwise it remains open in a disabled
no-document state or closes according to normal macOS auxiliary-window policy.
It must never continue editing a closed document through retained UI callbacks.

Required macOS integration tests cover single-window reuse and retargeting,
continued document editing while the window is open, live application and undo
grouping, Character/Paragraph tab enablement, inherited versus explicit values,
base-style parent restrictions, external undo/redo refresh, deletion fallback
to Base Paragraph, stale callback rejection, and target-document closure.

### Explicitly deferred compatibility

The following are outside the initial command commitment unless a later change
adds them here: Vimscript/Vim9script, user mappings and abbreviations, plugins,
terminal jobs, shell filters and `:!`, tags, quickfix, diff mode, folding,
spellchecking, code syntax highlighting, Vim tab pages and side-by-side splits,
sessions/viminfo, remote server commands, and full Vim option/regex parity.
Architecture must not gratuitously prevent these, but do not build speculative
subsystems for them now.

## Core architecture

The core is organized around testable services rather than platform widgets.
Names below are conceptual; language-specific spelling may differ.

### Core module and dependency boundaries

The portable Rust core follows a model/controller split. The split is a strict
dependency and mutation boundary, not merely a naming convention:

```text
src/core coordinator --> command
src/core coordinator --> layout
src/core coordinator --> document
command -------------> layout public API
command -------------> document public API
layout --------------> document public API
document -X---------> command
```

`src/core/document` is the model. It owns authoritative and derived document
state, state invariants, and the operations that can change that state.
`src/core/command` is the Vim-compatible controller. It interprets normalized
input and decides which model operation or view action is intended. Layout is a
separate derived service because it consumes document snapshots but has
view-specific configuration, caches, geometry, and background scheduling.

The required initial directory responsibilities are:

```text
src/core/
    lib.rs or equivalent       public facade and C-ABI-facing core handles
    coordinator.*              serial ownership and atomic publication
    document/
        source and artifact storage
        encoding, format, and semantic projections
        formatted block/text tree, styles, and position algebra
        buffer/view editing state, transactions, and undo history
        persistence capabilities and semantic edit intentions
    command/
        normalized command input and Vim grammar
        modes and pending parser states
        motions, text objects, operators, and insert/replace behavior
        Ex/search command interpretation
        repeat and macro command representation
    layout/
        segmentation, shaping-provider integration, and wrapping
        view height indexes, viewport layout, hit testing, and geometry
    services/                   only genuinely cross-cutting portable contracts
```

The leaf filenames are not normative and related responsibilities may begin in
one file before being split. The `document`, `command`, and `layout` module
boundaries and the dependency rules below are normative. Do not create a
general `common`, `util`, or `shared` module as a way to evade those boundaries;
small dependency-free value types may live at the crate root only when at least
two sibling modules genuinely own neither concept.

A service interface normally belongs to the portable module that consumes it:
for example, the artifact-storage contract belongs with `document` and the text-
measurement contract belongs with `layout`. `services` is reserved for a
genuinely cross-cutting clock, scheduler, or similar dependency-free contract.
It contains no AppKit implementation and MUST NOT depend back on both sibling
modules in a way that creates a cycle. Platform implementations remain under
`src/mac` and are injected through the core facade.

#### Document model boundary

`document` owns at least:

- source artifacts, snapshots, piece trees, format adapters, transformation
  pipelines, formatted trees, styles, provenance, anchors, and range types;
- document and buffer state, including file identity, dirty state, capabilities,
  undo history, registers, marks, search history, and the persistent portions of
  attached view state such as cursor and selection anchors;
- typed model requests, including semantic edit intentions such as replace text,
  apply formatting, insert a break, or change block kind; history navigation;
  save serialization; and model-level view-state changes;
- preparation, reverse projection, verification, and atomic commit of source
  transactions; and
- immutable read snapshots and bounded query APIs used by commands, layout,
  accessibility, and frontends.

Model request, semantic intention, and document result types belong to
`document`, not `command`, because native UI actions and future controllers must
be able to use the same model operations without manufacturing Vim commands.
The model may store command-relevant values such as register contents or named
marks, but it does not know which keystroke, count, operator, or Ex spelling
caused a requested state transition.

Tree nodes, mutable indexes, parser state, cache implementations, and format-
specific syntax types are private to `document` submodules. Consumers receive
immutable snapshots, opaque stable identities, iterators/batches with bounded
lifetime, and typed query results. `command` MUST NOT import or pattern-match on
piece-tree nodes, projection-tree nodes, concrete format syntax, or mutable
document internals, even when Rust crate visibility would technically allow it.

`document` MUST NOT import `command` or accept `ParsedCommand`, `Motion`, Vim
key codes, counts, operator names, or mode-machine state. This prohibition keeps
the dependency graph acyclic and permits the complete model and transaction
suite to run without instantiating a Vim interpreter.

#### Command controller boundary

`command` owns at least:

- Normal, Insert, Replace, Visual, command-line, and transient pending states;
- count, register-prefix, multi-key-prefix, operator, motion, and text-object
  grammar;
- Vim-specific inclusive/exclusive and linewise motion semantics;
- selection of register effects, undo grouping directives, dot-repeat actions,
  macro recording/replay, and command-line/search history behavior; and
- translation from normalized input into a revision-bound `CommandPlan`.

The command interpreter may read only a `CommandContext` composed of immutable
document/projection state, the relevant view editing state, implemented options
and capabilities, and an exact layout snapshot when the command requires visual
rows. Queries are explicit and side-effect-free. A command that does not need
layout must not acquire or wait for it.

A resolved plan is conceptually:

```text
CommandPlan {
    document/projection/layout revision preconditions,
    optional document::ModelRequest,
    planned buffer/view/controller state effects,
    undo-group directive,
    presentation or platform requests
}
```

A `ModelRequest` distinguishes an atomic semantic edit, history navigation,
persistence preparation, and other model operations. A compound or
discontiguous edit is one request containing one atomic semantic intention, not
a list that may partially commit.

The exact Rust representation may use enums and specialized variants rather
than one broad structure. It MUST remain a typed value: commands do not call
arbitrary model closures, retain mutable document references, directly patch
source bytes, mutate derived spans, or publish UI effects while resolving.
Unsupported, incomplete, cancelled, or failed commands produce typed outcomes
and no partially applied model change. Updating the command parser's own pending
state while accumulating a multi-key command is not a document mutation.

#### Coordinator and atomic application

The buffer coordinator at the `src/core` composition boundary owns concrete
instances of document, command, and per-view layout/controller state. It is the
only layer allowed to orchestrate all three; it contains sequencing and
publication logic, not a second implementation of their domain rules.

For a mutating command, the coordinator:

1. captures a `CommandContext` from mutually compatible immutable snapshots;
2. asks `command` to resolve input into a `CommandPlan`;
3. verifies the plan's revision preconditions;
4. asks `document` to prepare, reverse-project, and verify its semantic
   intentions without publishing them;
5. atomically installs the prepared document transaction and the plan's
   register, mark, cursor/selection, undo-group, repeat, mode, and invalidation
   effects; and
6. schedules or returns typed layout, redraw, persistence, clipboard, dialog,
   or diagnostic requests.

Before publication, every success-dependent state effect must be validated and
infallible to install; step 5 performs no provider call, callback, allocation
whose failure can become partial state, or external I/O. If preparation,
verification, policy resolution, or a revision check fails, the source snapshot
and all success-dependent plan effects remain unchanged. Parser cleanup and an
error diagnostic may still be published. Non-mutating plans use the same
revision check and coordinator turn but need no prepared document transaction.
Native frontend actions may enter at step 3 with a `document::ModelRequest`;
they do not pass through the Vim grammar, but they use the same preparation,
commit, history, and invalidation path.

This coordinator is the serial owner described by the concurrency model. The
module boundary does not imply one thread per module, and it introduces no lock
between `command` and `document`. Background work still receives immutable
snapshots and returns revision-tagged candidates to the coordinator.

#### Public surface and tests

`src/core/lib.rs` exposes task-oriented facade operations and opaque handles,
not the complete public surface of every internal module. The C ABI wraps this
facade and evolves together with its in-repository callers under the API
evolution policy above. Positions, edits, queries, and layout data crossing the
ABI remain explicit, revision-tagged, ownership-safe, and batch-oriented.

Testing follows the same boundary:

- `document` unit and property tests construct snapshots and semantic intentions
  directly, with no key-event or Vim parser setup;
- `command` unit tests use small immutable command contexts and deterministic
  layout/query fakes to verify parsing and resulting plans without committing a
  document;
- `layout` tests use immutable formatted snapshots and fake shapers without a
  command interpreter; and
- coordinator integration tests run plans through preparation and atomic commit,
  including stale revisions, unsupported intentions, undo grouping, and failure
  rollback.

At least one architectural test or compile-time visibility check MUST ensure
that `document` has no dependency on `command` and that `command` does not reach
private document storage. Mocking a document by duplicating its tree internals
inside command tests is forbidden; test through the same snapshot/query contract
used in production.

### Core source and projection storage

- Source storage follows the byte-preserving balanced structures defined in
  "Source storage and lossless syntax". A flat mutable string is forbidden as
  the authoritative representation.
- The formatted projection also uses a persistent balanced block/text tree.
  Its nodes maintain aggregates needed for logarithmic navigation: UTF-8 byte
  length, Unicode scalar/grapheme metadata as needed, hard-line count, and
  block metadata.
- Projected leaves have stable identities and local revisions so unchanged
  formatted content can retain segmentation and shaping cache entries across
  source edits.
- Formatted lookup by position or hard line is `O(log n)` apart from required
  Unicode boundary work. Source patching has the analogous logarithmic bound
  plus changed bytes and incremental transformation work.
- Both source and formatted structures handle millions of short lines and a
  single extremely long line. Algorithms must not use line length as an
  implicit safe upper bound.
- Derived style runs use a range structure that permits logarithmic queries and
  does not require walking all following spans after a local projection change.
- Persistent marks, selections, jumps, cursors, and viewport anchors use the
  `TextAnchor` contract: stable projected identity, insertion association,
  boundary affinity, deletion recovery policy, and recoverable source provenance.
  They are logically rebased through each transaction/projection position map;
  an implementation may resolve that composed mapping lazily and MUST NOT walk
  every anchor after a local edit.
- A committed source transaction reports exact byte patches, affected source
  identities, before/after source revisions, projection changes, and any
  supporting format/encoding changes. It also publishes the composable position
  maps needed to rebase source and formatted anchors.

### Buffer, view, and command state

Keep document-global and view-local state separate. The source artifact,
pipeline configuration, projection caches, registers, marks, search history,
undo history, and file identity belong to the document/buffer/session as
appropriate. Cursor, selection, desired x, wrap width, scroll anchor, viewport,
and layout caches belong to a view. Two views of one buffer may have different
widths, wrapping settings, and layout caches while sharing source snapshots,
formatted projections, and width-independent shaping results.

Logical ownership does not require one monolithic `ViewState` structure. The
coordinator composes state along the module boundaries:

- `document::BufferState` owns shared source/projection/history state and the
  model stores required for registers, marks, search history, and persistence;
- `document::ViewEditState` owns stable cursor and selection anchors that must
  be remapped or restored with document transactions;
- `command::BufferCommandState` owns dot-repeat and macro state, while
  `command::ViewCommandState` owns mode, pending grammar, and desired x; and
- `layout::ViewLayoutState` owns wrap width/options, scroll and viewport state,
  height indexes, and view-specific layout caches. Its scroll anchor uses the
  `document::TextAnchor` value type without transferring ownership to the model.

The exact structs may be finer-grained, but these ownership assignments and
shared-versus-view-local semantics must remain observable. In particular, a
second view never shares a cursor, mode, desired x, viewport, or pending command
with the first view.

The full command-dispatch path preserves Vim's separation of Normal command
parsing and operator execution while respecting the module boundary:

1. the frontend adapter sends normalized portable key, text, composition,
   pointer, or native-command events through the facade;
2. `command` parses counts, prefixes, registers, operators, and motions without
   mutating document state;
3. `command` resolves motions/text objects against a formatted projection
   snapshot and, when needed, a view layout snapshot;
4. `command` produces a typed, revision-bound `CommandPlan`;
5. the coordinator asks `document` to reverse-project any semantic edit to a
   minimal source patch set, tentatively apply it, reproject, and verify it;
6. the coordinator atomically commits the prepared source transaction, undo
   record, mode, cursor/selection, registers, repeat state, and invalidations;
   and
7. the facade returns state changes, diagnostics/policy requests, and redraw or
   layout requests to the frontend.

Commands that require visual rows explicitly receive a valid layout snapshot.
Most edits, searches, word motions, and linewise operations must remain usable
in headless core tests with fake format, encoding, and measurement providers.

### Concurrency, scheduling, and locking

The core uses serial ownership for mutable editor state and immutable snapshots
for parallel work. It MUST NOT protect an entire document with a reader/writer
lock or allow background tasks to read mutable buffer or view structures.

#### Ownership domains

- Each buffer has one logical **buffer coordinator** that is the sole writer of
  its source-current pointer, undo history, marks, registers, command state,
  projection-current pointer, and attached view states. The coordinator may be
  an actor or a serial executor; it does not require a dedicated operating-
  system thread.
- All commands and native edit intentions for views of the same buffer are
  messages processed to completion in coordinator order. A long operation must
  be decomposed into bounded work rather than occupying the coordinator while
  doing whole-document parsing, shaping, or layout.
- Different buffer coordinators may run concurrently. Session-level operations
  such as `:wall` orchestrate buffer messages and do not obtain several buffer
  locks or directly mutate their state.
- AppKit view objects, menus, windows, and native drawing resources are confined
  to the macOS main thread. The frontend sends normalized events to the core and
  applies returned, revision-tagged presentation updates on the main thread.
- A provider explicitly declares any additional thread confinement. The core
  scheduler obeys it without changing ownership of buffer state.

Serial ownership is the logical equivalent of a buffer-level mutation lock,
but arbitrary callers never acquire such a lock. No mutex is required to read
a source, projection, or layout snapshot after obtaining an immutable retained
reference to it.

#### Background jobs

Projection, segmentation, shaping, wrapping, and height refinement may run on a
bounded worker pool. A job captures only immutable inputs, including:

- retained source and/or projection snapshots;
- the bounded source, formatted, or hard-line region requested;
- view configuration and its generation when layout is view-specific;
- metrics, transform, and external-resource generations on which it depends;
- a priority and monotonically ordered job identity; and
- a cooperative cancellation token.

Workers own their temporary state. They return immutable result packages to the
buffer coordinator and never install results, advance history, move anchors,
mutate a view, or call frontend UI code directly.

The coordinator validates a returned package before making it observable:

- A projection or positioned layout result is installed only when its complete
  source/projection revision and every relevant configuration generation match
  the current target.
- A stale top-level result is discarded. A shaped or segmented subresult from a
  stale job may be admitted to a content-keyed cache only after the coordinator
  verifies that its stable projected identities, local revisions, bounded
  context, style, language/direction inputs, and metrics generation are all
  unchanged.
- Validation and installation are one coordinator turn, so an edit cannot
  interleave between the check and publication.

Work priorities, from highest to lowest, are changed visible rows required for
caret/hit-testing correctness, newly exposed scroll rows, viewport overscan,
and off-screen estimates or pre-layout. Pending work for superseded revisions,
intermediate live-resize widths, or abandoned viewports is cancelled and
coalesced. There is at most one current background layout generation per view;
new demand may extend or replace its bounded region rather than enqueueing an
unbounded backlog.

Cancellation is checked at bounded parse checkpoints, projected leaves,
shaping fragments, and hard-line/wrap units. Obsolete jobs must release retained
snapshots promptly enough that continuous typing cannot keep an unbounded chain
of old source revisions alive. Cancellation does not make a partially produced
result observable.

The coordinator retains the cooperative cancellation token for the one current
layout job of each attached view. A replacement becomes current, and only then
cancels its predecessor, after the replacement has passed preparation and
registration; a cancelled or invalid replacement request therefore does not
orphan valid work. Resize or view-configuration invalidation, a metrics-
generation change observed by the coordinator, a shared document commit, and
view removal cancel affected current work. Successful installation retires its
matching token without cancelling it, and cancellation observed after the
installation linearization point does not roll back the installed snapshot.
Removing a view is a serial, nonblocking coordinator operation: it cancels that
view's layout work, discards uncommitted marked-text/IME overlay state without
editing the document, and closes an Insert/Replace edit group owned by the view
so already committed text remains one complete undo unit. It does not disturb
other views' jobs or presentation state.

View and layout-job identities are monotonically allocated and never reused,
including after view removal. Exhaustion is a typed failure; convenience APIs
that cannot return it may fail explicitly rather than wrap. When an exact visual-
row command reaches partial snapshot coverage, its non-mutating result carries
a typed, revision- and generation-bound layout demand. That demand identifies
the missing edge and the complete bounded hard-line interval for a replacement
viewport request, so the frontend does not guess an expansion direction or
range. A demand whose source, layout, configuration, or metrics identity is no
longer current is rejected as stale.

The synchronous frontend input boundary satisfies these demands and retries the
same still-uncommitted input before returning its result. This applies to Page
Up/Down and other layout-dependent commands in every editing mode; a request for
more layout must never silently consume the keystroke. Retries preserve pending
counts, registers, selection, and undo grouping, and publish command effects
only once. Paging replaces the cached region with a bounded band around the
viewport and required command endpoints, rather than retaining every earlier
page. The low-level command API continues to expose typed demands to callers
that schedule layout themselves.

#### Permitted synchronization primitives

- Immutable snapshot and buffer-piece lifetimes may use atomic reference
  counts. Cancellation tokens and simple generation/closed flags may use
  atomics with documented ordering.
- The initial cache design SHOULD keep cache indexes, memory accounting, and
  eviction policy coordinator-owned. Workers receive immutable cache hits and
  return candidate entries, so expensive computation requires no cache lock.
- A cache proven by profiling to need concurrent access MAY use sharded locks.
  A shard lock protects only lookup, insertion, eviction metadata, and memory
  accounting. Cached values are immutable, shaping/projection occurs outside
  the lock, and duplicate computation is preferable to waiting on a
  single-flight lock.
- FFI handle tables and scheduler queues MAY use short internal locks. Public
  mutable operations still enqueue work to the appropriate coordinator instead
  of exposing locked buffer state to Swift.

Code MUST NOT hold a lock while parsing, projecting, shaping, wrapping, drawing,
performing filesystem or clipboard I/O, waiting for a worker or coordinator,
crossing the C ABI into a provider, or invoking a callback. Code MUST NOT hold
more than one cache-shard or registry lock at once. Lock-protected code does not
call user, adapter, frontend, or provider code. These rules take precedence
over avoiding harmless duplicate cache work.

An edit never waits for an off-screen layout job that captured an older
snapshot. It commits a new immutable revision, performs or requests only the
bounded exact visible work required by the next frame, and lets the coordinator
cancel, reuse, or discard older results according to the validation rules.

### Platform interfaces

At minimum, define these narrow directions of dependency:

- **Text measurement/shaping provider**: implemented by `src/mac`, consumed by
  core layout.
- **Filesystem/document provider**: reads and atomically writes source-artifact
  parts and returns data or errors; core owns source preservation, edit policy,
  save serialization, and dirty state.
- **Clipboard provider**: exchanges plain text and, when supported, portable
  rich-text payloads; core owns register semantics.
- **Clock/scheduler hooks**: allow cancellable background pre-layout without
  making core correctness depend on real time.

All interfaces need deterministic fakes in `src/core` tests.

## Text measurement and shaping contract

The core owns layout policy and caches. The frontend supplies authoritative
font resolution and shaping measurements. Do not reduce the interface to
`width(string, font)`: that is insufficient for ligatures, bidirectional text,
fallback, hit testing, and mixed styles.

A shaping request contains:

- immutable projection revision and formatted text range;
- the UTF-8 text slice plus enough bounded context on each side for correct
  shaping at a cache-fragment boundary;
- resolved style runs, language, script/direction inputs, and feature flags;
- a requested scale and metrics generation; and
- whether render data or metrics-only data is needed.

A shaping response contains platform-neutral data sufficient for core layout:

- logical formatted range and visual run order;
- glyph/cluster sequence or an opaque render-run handle plus its lifetime;
- cluster-to-formatted-text mapping; source provenance remains available
  through the formatted projection;
- legal caret stops with upstream/downstream boundary affinity;
- per-cluster or per-caret advances;
- ascent, descent, leading, ink bounds, and typographic bounds;
- resolved fallback font identities; and
- direction and any diagnostics for unsupported/missing glyphs.

Requirements for the provider contract:

- Results are deterministic for the same request and metrics generation.
- It never returns a caret stop inside an indivisible shaping cluster.
- The absence of a visual caret stop at a logical grapheme boundary does not
  expand or otherwise change the logical range of an edit.
- Cache-fragment boundaries must not change shaping. The request supplies
  context and the response identifies the stable interior that can be cached.
- Native objects do not leak into general core APIs. Opaque handles have an
  explicit owner, thread rule, and metrics-generation lifetime.
- The provider announces a new metrics generation when font availability,
  fallback, feature resolution, or scale makes previous measurements stale.
- Core does not assume calls must run on the UI thread. The macOS
  implementation documents any stricter rule and schedules accordingly.

Unicode line-break opportunity detection belongs in portable core code so all
frontends make the same wrapping choices. Shaping supplies the actual advances
and legal cluster boundaries. Locale-sensitive hyphenation is deferred unless
added with a portable policy and provider capability.

## Core layout model

Layout consumes an immutable formatted projection snapshot, a view
configuration, and a measurement provider. It produces a layout snapshot
containing:

- exact visual rows covering the requested viewport plus overscan;
- each row's projected hard-line identity, formatted range, baseline,
  ascent/descent, bounds, and wrap-continuation flags;
- positioned shaped fragments in visual order;
- bidirectional mappings between `CaretPoint` values and `LayoutPoint` values,
  each tied to this exact layout revision;
- caret geometry for every requested legal caret stop and explicit fallback
  geometry for logical endpoints inside a visually indivisible cluster;
- selection rectangles, including discontiguous bidirectional selections; and
- exact visible bounds plus estimated or exact total vertical extent.

Layout snapshots are immutable and revision-tagged. A result computed for an
old projection revision, view configuration, or metrics generation must never
be installed into the active view.

### Required cache layers

Keep these concerns separately cached:

1. **Formatted segmentation metadata** — grapheme and line-break metadata tied
   to projected leaf revisions.
2. **Shaping fragments** — width-independent shaped results keyed by formatted
   projection identity/content, resolved style, shaping context,
   direction/language, and metrics generation.
3. **Wrap plans** — visual-row breaks and row metrics keyed by hard-line
   projected identity/revision, usable width, wrap options, and
   shaping-fragment keys.
4. **View height index** — a balanced aggregate tree of exact or estimated
   heights/visual-row counts per hard line for mapping formatted positions to y
   coordinates and y coordinates back to projected neighborhoods.
5. **Viewport/display cache** — positioned rows and frontend draw resources for
   the visible region and bounded overscan.

The shaping cache should be shareable between views. Wrap plans and height
indexes are view/configuration-specific. All caches have bounded memory and an
LRU or equivalent eviction policy; correctness cannot rely on an entry being
present.

### Incremental invalidation

After a source transaction is verified, the projection pipeline supplies an
exact formatted change summary to layout:

1. install the new immutable formatted projection revision;
2. invalidate segmentation and shaping fragments that overlap its changed
   formatted ranges plus the bounded shaping context needed on either side;
3. invalidate wrap plans for affected projected hard lines; if hard-line
   boundaries were inserted or removed, update the line/height tree
   structurally in logarithmic time;
4. update aggregate estimated heights without shifting a flat array of every
   later line;
5. synchronously shape/wrap only invalid visible rows needed for the next
   frame, using cached unaffected fragments;
6. retain the viewport text anchor while exact heights above it change; and
7. optionally pre-layout bounded overscan/background regions with cancellable,
   revision-checked work.

Wrapping is independent per formatted hard line, so a localized projection
change normally does not invalidate the wrap plan of later hard lines. Their
absolute y positions change through height-tree aggregates rather than by
rewriting each row. A source transformation may report a wider formatted
change because of syntax or semantic dependencies; layout follows that explicit
change summary. Context-dependent shaping or bidirectional resolution may widen
invalidation within an affected hard line, but never silently to the whole
formatted document.

On a usable-width change:

- keep valid segmentation and shaping fragments;
- start a new wrap/layout configuration generation in `O(1)` rather than
  walking all hard lines to mark entries stale;
- recompute exact wrap plans for visible hard lines and overscan;
- use per-line estimates for untouched regions and refine them lazily;
- update total extent as estimates become exact while preserving scroll
  anchors; and
- cancel work for obsolete intermediate widths during live resize.

On a style definition, assignment, or direct-formatting change, resolve the
property difference and invalidate by effect:

- font, size, weight, slant, language/direction, OpenType features, letter
  spacing, or baseline changes invalidate affected shaping and downstream wrap
  and height results;
- paragraph indents, spacing, line spacing, alignment, or structural
  contributions invalidate affected paragraph wrap/position/height results but
  retain width-independent shaping when its character inputs are unchanged; and
- Document padding changes the usable content width and invalidates view wrap
  generations and height estimates in `O(1)`, with exact visible replacement;
  and
- canvas background, text color, and other paint-only changes invalidate display
  resources and damage regions without reshaping or rewrapping.

A base-style or ancestor definition change follows dependency edges to all
affected assignments rather than scanning unrelated text. A global view
default-font or metrics-generation change may make all shaping logically stale,
but invalidation is generation-based and `O(1)`; replacement remains
viewport-driven.

### Extremely long hard lines

Never require one monolithic shaped object for a hard line. Shape long lines in
bounded fragments with enough context to make cached interiors stable. Store
prefix checkpoints for advances, candidate breaks, and wrap continuation so
work can be resumed. Operations that truly require preceding wrap state may be
incremental and time-sliced, but visible output and hit testing must become
exact before interaction. Background work is cancellable on edit, resize, or
scroll.

### Scrolling and anchoring

- Vertical scrolling is continuous in layout units, not integer terminal rows.
- Cursor reveal scrolls the minimum needed subject to configured context.
- The primary scroll anchor is a persistent `TextAnchor` with source provenance,
  insertion association, boundary affinity, and an offset from the viewport edge,
  not only an absolute y value.
- Mapping a far-away y coordinate may begin from height estimates, then refine
  the local neighborhood. Refinement must not strand the viewport on unrelated
  text.
- Horizontal scroll state is meaningful only when wrapping is off or content
  intentionally overflows.
- Every document view has native scrollbars. The vertical scrollbar follows
  the macOS Show scroll bars preference, including live preference changes:
  legacy scrollbars remain visible, while overlay scrollbars appear during
  scrolling and fade after a short idle delay. Scrollbar interaction changes
  the core-owned viewport without moving the editing caret.
- The horizontal scrollbar is available only when a displayed row intersecting
  the current vertical viewport extends beyond its width. Its range reflects
  those visible rows, excluding overscan and wider rows elsewhere in the
  document. Scrolling vertically clamps the horizontal origin to the new
  visible range. Showing and hiding the horizontal scrollbar fades with a short
  delay to avoid flicker at viewport boundaries. Visibility changes must not
  trigger repeated reflow; legacy scrollbar gutters remain stable while fading.
- Scrollbar extents use bounded current layout and the existing document-height
  estimates. Updating a scrollbar must not shape or scan the whole document.

## macOS frontend requirements

`src/mac` owns:

- app/window lifecycle, menus, dialogs, document open/save panels;
- a custom editor view and drawing pipeline;
- Core Text font matching and shaping/measurement implementation;
- conversion of layout fragments into draw operations;
- keyboard interpretation, text input, dead keys, and IME marked text;
- mouse/trackpad input, scrolling, and drag selection;
- pasteboard integration;
- font panel and rich-style controls;
- caret blink and mode-specific presentation; and
- accessibility exposure for text, ranges, selection, insertion point, and
  visible lines, including VoiceOver-compatible geometry queries.

AppKit text storage must not become a second source of truth. If native text
APIs require adapter objects, they expose the current formatted projection and
send edits back as core semantic intentions. The macOS frontend never parses,
normalizes, or serializes the authoritative source on its own.

The frontend draws only damage regions returned by state/layout changes where
practical. It may cache native glyph/draw resources separately from core
metrics, keyed by layout snapshot and metrics generation.

### macOS caret realization

The initial macOS frontend may require the latest generally available macOS
major release at the time it ships. Record the resulting numeric deployment
target in the project configuration. Compatibility fallbacks for older macOS
releases are not required merely to avoid current AppKit text-cursor APIs.

On macOS, use AppKit's `NSTextInsertionIndicator` for the thin vertical caret in
the document's Insert mode and in command-line text entry. This is a native
macOS presentation choice. It must not leak an AppKit type or native-indicator
assumption through the C ABI or into `src/core`.

- Add the indicator as a view above the custom text surface and update its frame
  from the exact `CaretPoint` geometry. Keep it a thin insertion indicator; do
  not resize it to implement Normal or Visual mode blocks.
- Set `displayMode` to `.automatic` while its text-input surface is the active
  first responder and the applicable mode uses a vertical caret. Set it to
  `.hidden` in Normal, Visual, or Replace mode, when the view resigns first
  responder, and whenever the portable inactive-outline presentation is used.
- Preserve the native indicator's system blinking, dictation effects, input
  source/Caps Lock accessories, tracking behavior, and accessibility styling.
  Notify the AppKit text-input system when scrolling or zooming begins and ends
  as required by the current API.
- Resolve `SystemCaretColor` to `NSColor.textInsertionPointColor`. Assign the
  resolved color to `NSTextInsertionIndicator.color` and use that identical
  resolved color for the custom block, underline, and inactive-outline drawing.
  If an `Explicit(PortableColor)` preference is added, convert it to `NSColor`
  once in the centralized appearance resolver and feed both paths from that
  result.
- Re-resolve dynamic system color when effective appearance, accent color, or
  accessibility contrast changes. Do not cache a permanently resolved RGB value
  for `SystemCaretColor`.

Normal and Visual block carets, the Replace underline, color-glyph fallback,
empty/end-of-line geometry, and inactive outline are drawn by Viem according to
the portable requirements in "Modes and caret". A future Windows frontend uses
the same portable appearance and centralized color preference but chooses its
own native or custom implementation for the thin vertical caret.

## Performance requirements and acceptance fixtures

Complexity and bounded work are release requirements, not later optimization.

- Opening a document may scan source once when required for detection/indexing,
  but initial interactive display must not wait for full-document semantic
  projection, shaping, or wrapping unless a declared global dependency makes
  that unavoidable.
- A local insertion/deletion is logarithmic in source/projected tree size plus
  changed bytes, incremental transformation work, affected shaping context,
  and visible reflow.
- Typing in a short visible hard line performs no work proportional to total
  source bytes, projected hard-line count, or total visual-row count.
- Resizing a wrapped view performs no full-document shaping and no eager
  traversal merely to invalidate every hard line.
- Scrolling lays out the newly exposed region plus bounded overscan, not all
  text between old and new viewport locations.
- Cache memory has configurable budgets and remains bounded during repeated
  resize, zoom, and multi-view use.

Maintain automated fixtures for at least:

1. one million short hard lines;
2. 100 MiB mixed-length UTF-8 and legacy-encoded source artifacts;
3. one multi-megabyte hard line with wrapping both on and off;
4. mixed fonts and sizes, ligatures, combining marks, emoji, and right-to-left
   runs in the same hard line;
5. two views of one buffer at different widths; and
6. rapid live resize through many widths followed immediately by an edit.

The source/transform suite additionally includes plain text, Markdown, HTML,
and RTF samples as adapters are implemented, with alternate equivalent syntax,
comments/trivia, malformed and unknown constructs, legacy encodings, and
characters that cannot be represented in the original encoding.

Performance tests should instrument source bytes decoded and parsed, formatted
bytes projected, bytes segmented and shaped, hard lines wrapped, cache hits,
height-tree operations, and discarded stale tasks. Assert bounded work
structurally; avoid brittle wall-clock-only tests.

## Correctness test strategy

- **Source property tests**: randomized byte patches preserve source-tree
  aggregates, unchanged byte slices, part identities, and source anchors.
- **Projection property tests**: randomized incremental projections equal
  clean projections and preserve UTF-8, formatted-tree, style-span,
  provenance, insertion-association, and boundary-affinity invariants.
- **Position/range property tests**: randomized edit sequences verify point
  ordering, half-open intersection/union/subtraction, directed-selection
  normalization, sorted non-overlapping range sets, insertion association,
  deletion collapse, moved-content identity, identity maps, and associative map
  composition. Tests reject cross-document, cross-domain, cross-snapshot,
  invalid-boundary, and inverted-range operations; exercise split range maps,
  stable-identity and provenance recovery; and verify that inclusive and
  exclusive Vim endpoints are converted to half-open extents exactly once.
- **Round-trip tests**: no-op saves are byte-identical; changed saves alter
  only declared patches; forward projection after a reverse edit satisfies the
  semantic intention.
- **Encoding tests**: legacy and invalid byte input survives no-op saves, maps
  correctly to valid UTF-8, and never silently substitutes unrepresentable
  edits.
- **Line-ending tests**: LF, CRLF, CR, mixed endings, literal CR/LF content,
  empty files, and files with and without a final terminator exercise every
  detected, forced, and defaulted mode. Plain text, Markdown, and HTML use the
  same conformance suite. No-op saves are byte-identical; inserted breaks use
  `fileformat`; conversions declare every patch and preserve the logical token
  sequence or return the required policy result.
- **Style tests**: randomized acyclic block and character style trees resolve
  identically with and without caches. Tests cover sparse inheritance, explicit
  normal values, Document-to-Paragraph defaults, role/property applicability,
  canvas background/padding, paragraph character defaults, named character
  styles, independently overlapping direct-property spans, boundary-affinity
  typing-style inheritance independently of anchor association, empty
  paragraphs, definition invalidation, cycles/missing parents, structural
  contribution precedence, provenance, and reverse-edit capabilities.
- **Pipeline tests**: composed provenance and reverse edits match an equivalent
  unfused pipeline; stale, generated, ambiguous, and unsupported edits return
  the required structured result.
- **Undo tests**: randomized grouped transactions round-trip exact source bytes,
  source metadata, required buffer-local marks, and invoking-view restoration
  positions through undo/redo and alternate branches. Tests cover every undo
  break, edit-after-undo branch creation, preferred-child selection, multi-view
  anchor remapping, saved/dirty transitions, history pruning, and
  the rule that registers and other non-history command state are not replayed
  or restored. Redo from a stored snapshot must equal the original committed
  result without invoking reverse projection again.
- **Command table tests**: every supported command covers counts, registers,
  mode transitions, operator composition, cancellation, dot repeat, and macro
  replay.
- **Vim differential tests**: for supported behavior that this file does not
  deliberately change, run the plain-text identity adapter, execute equivalent
  keystrokes against a pinned Vim version, and compare resulting formatted
  text, cursor, registers, and mode when feasible.
- **Layout equivalence tests**: incremental results equal a from-scratch layout
  for the same projection/configuration using a deterministic fake shaper,
  including inherited and direct styles, paragraph spacing/indents, empty
  paragraphs, and continuous-canvas height aggregation.
- **Cache tests**: source edits, projection changes, font-generation changes,
  wrapping toggles, and resize invalidate exactly the required layers. Stale
  async results are rejected.
- **Concurrency tests**: a deterministic scheduler permutes edits, undo/redo,
  resize, scroll, cancellation, worker completion, and view destruction. Assert
  that only matching revisions become observable, stale reusable fragments are
  admitted only after dependency revalidation, cancelled jobs release old
  snapshots, and reentrant fake providers cannot observe a held core lock.
- **Geometry tests**: round-trip `TextPoint` plus boundary affinity to
  `CaretPoint`, then to `LayoutPoint` and back, including bidi, ligatures,
  visually indivisible shaping clusters, mixed sizes, hard-line end, empty
  lines, stale layout rejection, and Visual Block rectangles resolved to tagged
  range sets.
- **macOS integration tests**: compare Core Text output to emitted row geometry,
  verify IME lifecycle and native undo/menu routing, and exercise accessibility
  range/geometry APIs. Verify that mode and first-responder transitions show
  exactly one of the native vertical indicator or the applicable custom caret;
  both paths use the same resolved system or explicit caret color; dynamic
  appearance changes are propagated; and block, color-glyph, underline, empty
  line, end-of-line, bidi, and inactive-outline cases use exact layout geometry.

## Architecture lessons adopted from Vim

Vim is a guide, not a code template. Preserve these useful separations:

- Vim's `normal.c` parses Normal/Visual commands and collaborates with operator
  code; Viem likewise separates grammar, motion resolution, semantic edit
  intentions, and verified source mutation.
- Vim's buffer/window distinction maps naturally to Viem's shared buffer and
  per-view layout state.
- Vim's `memline.c` stores line data in a block tree; Viem also needs balanced,
  aggregate storage for authoritative source and derived formatted text,
  adapted for provenance, rich spans, and Unicode positions.
- Vim remembers screen state and uses validity/invalidation levels to minimize
  redraw; Viem extends the idea into segmentation, shaping, wrapping, height,
  and viewport cache layers.
- Vim treats folding as a presentation transform between buffer lines and
  displayed lines; Viem treats soft wrapping the same way and never writes
  visual rows back into document text.
- Vim keeps alternate undo branches; Viem's transaction history does too.

Do not copy Vim's terminal-cell assumptions. Pixel advances, variable row
heights, shaping clusters, bidi affinity, and resize-driven reflow are
fundamental Viem concepts rather than frontend patches.

Primary references:

- [Vim command index](https://github.com/vim/vim/blob/master/runtime/doc/index.txt)
- [Vim motions and operators](https://github.com/vim/vim/blob/master/runtime/doc/motion.txt)
- [Vim changes](https://github.com/vim/vim/blob/master/runtime/doc/change.txt)
- [Vim regular-expression patterns](https://github.com/vim/vim/blob/master/runtime/doc/pattern.txt)
- [Vim Visual mode](https://github.com/vim/vim/blob/master/runtime/doc/visual.txt)
- [Vim development design decisions](https://github.com/vim/vim/blob/master/runtime/doc/develop.txt)
- [Vim Normal command dispatcher](https://github.com/vim/vim/blob/master/src/normal.c)
- [Vim memline block tree](https://github.com/vim/vim/blob/master/src/memline.c)
- [Vim screen cache](https://github.com/vim/vim/blob/master/src/screen.c)
- [Vim window redraw/invalidation](https://github.com/vim/vim/blob/master/src/drawscreen.c)
- [Vim undo branches](https://github.com/vim/vim/blob/master/src/undo.c)
- [Rust `regex` syntax and execution guarantees](https://docs.rs/regex/latest/regex/)

Primary source-preservation and transformation references:

- [Roslyn full-fidelity syntax model](https://learn.microsoft.com/en-us/dotnet/csharp/roslyn-sdk/work-with-syntax)
- [Tree-sitter incremental parsing](https://tree-sitter.github.io/tree-sitter/using-parsers/3-advanced-parsing.html)
- [ProseMirror document model and transactions](https://prosemirror.net/docs/guide/)
- [Pandoc reader/filter/writer pipeline](https://pandoc.org/filters.html)
- [Apple attributed-string persistence](https://developer.apple.com/documentation/foundation/nsattributedstring)
- [WordprocessingML document structure](https://learn.microsoft.com/en-us/office/open-xml/word/structure-of-a-wordprocessingml-document)
- [LibreOffice Writer filter test model](https://github.com/LibreOffice/core/blob/master/sw/qa/extras/README)
- [ICU character conversion behavior](https://unicode-org.github.io/icu/userguide/conversion/converters.html)
- [Bidirectional lens round-trip laws](https://www.cis.upenn.edu/~bcpierce/papers/wagner-thesis.pdf)

Primary HTML and RTF adapter references:

- [WHATWG HTML syntax](https://html.spec.whatwg.org/multipage/syntax.html)
- [WHATWG HTML parsing](https://html.spec.whatwg.org/multipage/parsing.html)
- [W3C CSS Style Attributes](https://www.w3.org/TR/css-style-attr/)
- [Microsoft RTF 1.9.1 specification](https://officeprotocoldoc.z19.web.core.windows.net/files/Archive_References/%5BMSFT-RTF%5D.pdf)

Primary macOS caret references:

- [Adopting the system text cursor in custom text views](https://developer.apple.com/documentation/appkit/adopting-the-system-text-cursor-in-custom-text-views)
- [`NSTextInsertionIndicator`](https://developer.apple.com/documentation/appkit/nstextinsertionindicator)

## Decisions still intentionally open

Do not silently settle these while implementing an unrelated feature. Record a
decision in this file or an architecture decision record first:

- which format adapters beyond plain text ship initially;
- the canonical syntax of any future adapter not specified above;
- user policy for Unicode edits not representable in the source encoding;
- exact Unicode word/sentence segmentation tailoring;
- whether rich system clipboard formats are required for the first release;
- additional style defaults and visual design choices not fixed above;
- hyphenation and justification; and
- concrete latency and memory budgets for supported hardware.

## Format controls, Markdown authoring, and lists

The status bar exposes a native popup for source format, with a small vertical
triangle and hover highlight. Encoding and line endings appear only in their
File submenus, not in the status bar. A choice is a checked core transaction
shared by the buffer's views and reversible with undo. Format
selection within a format family or to plain text changes interpretation while
preserving source bytes. An explicit HTML-to-Markdown or Markdown-to-HTML
conversion instead translates the formatted text and representable styling to
new source syntax as one undoable transaction, including source-visible variants.
This explicitly requested conversion may replace the entire source and reports
lost unsupported information through the command output bar. It is distinct
from no-op saves and ordinary local edits, which remain lossless.
Mode/format changes preserve each view's insertion cursor and visible text as
closely as possible. Unchanged source provenance and explicit conversion
correspondence carry text anchors into the new projection. Hidden or removed
syntax recovers at the nearest surviving content according to the anchor's
recovery policy. The viewport retains its top text anchor and fractional row
offset rather than resetting to the document start or preserving stale pixels.
For format changes, an upstream Insert/Replace caret follows preceding content
and stays before newly exposed closing syntax; downstream follows subsequent
content. This conversion policy does not change ordinary typing associations.
Encoding selection transcodes source syntax and verifies the new projection.
An explicit conversion to Latin-1 may replace unrepresentable scalars with `?`,
reporting the count in the output bar; undo restores the exact original bytes.
Ordinary typing in an existing Latin-1 document remains strict and must never
silently substitute. Line-ending selection delegates
to the shared conversion component. RTF disables the generic encoding and
line-ending controls because its grammar owns those interpretations.

The two Markdown views share one physical Markdown serialization:

- **Markdown WYSIWYG** is the existing projection with formatting delimiters
  hidden. Ordinary source line endings within a prose paragraph become spaces;
  blank source lines separate paragraphs. Each pair of source endings is one
  semantic paragraph separator; repeated pairs retain editable empty paragraphs.
  An unmatched terminal source ending is trivia, including after a closing code
  fence. Two-space and backslash hard breaks remain within their paragraph;
  endings inside preformatted code remain content. Newly authored paragraph
  boundaries use two current-fileformat endings. Splitting heading or list text
  may add a supporting ending to preserve a following paragraph. Untouched
  source bytes remain exact.
- **Markdown Source** keeps formatting delimiters and ordinary source-line
  endings editable, with parsed styles applied to their source spans. Blank
  source lines used to separate paragraphs project to styled paragraph
  boundaries, without additional raw blank rows. Their pairing and intentional
  empty-paragraph semantics match WYSIWYG. Ordinary source continuation lines
  share one paragraph style and retain internal line breaks when source flow is
  off; paragraph spacing applies only around the complete paragraph. Code-block
  whitespace remains literal. Paragraph separators retain their complete source provenance, so editing and
  saving preserve physical bytes outside the explicit patch set. Editing a
  delimiter reparses and updates formatting immediately. Formatting commands
  update source delimiters, and switching views preserves source bytes. Enter
  in ordinary prose starts a new paragraph using two current-fileformat source
  endings; preformatted code retains literal line breaks. Enter on
  an empty list item removes its marker and inserts the source separator needed
  for subsequent typing to remain a separate, unnumbered ordinary paragraph.

Opening a Markdown file retains the existing WYSIWYG default. The status popup
can select the source-visible Markdown view. New bold uses `**`, italic uses
`*`, and headings use one through six `#` characters followed by one space.
Untouched alternative delimiters and physical line endings remain exact.

Switching Markdown Source and WYSIWYG must preserve source bytes and anchors
without quadratic work in document length and formatting-change count.
Reprojection maps use ordered batch traversal of provenance and changes;
the frontend refreshes each affected view once and exports only its viewport
text. Large-document regressions must cover the first switch in both
directions, rather than relying only on a warmed projection cache.

The built-in Code character style and Code Block paragraph style use the system
monospace family and dark green (`#006400`). HTML `<code>` and `<pre>` and
Markdown inline/fenced backticks project to these roles; code whitespace remains
editable and preserved. New simple HTML bold and italic formatting uses `<b>`
and `<i>` where those tags express the requested change. More complex or
interacting properties use sparse CSS declarations as needed. Existing untouched
HTML spelling remains exact.

Markdown and HTML Paragraph defaults have 7pt space before and 7pt space after
at the default 14pt font size. Their additive spacing yields the common one-em
gap between adjacent paragraphs. Code Block inherits this outer spacing;
Markdown Code Block also has a 32pt logical start indent. User defaults and
explicit source style declarations can override these defaults without
materializing them in untouched source.

Each fenced Markdown block or HTML `<pre>` is one paragraph, including in
source-visible views. Its internal source endings produce explicit line breaks
within that paragraph, so spacing is applied only around the block. These
breaks preserve code indentation and blank rows and are distinct from automatic
word wrapping. HTML entities are decoded in WYSIWYG code; source-visible code
retains the literal source. Enter inside HTML preformatted content inserts a
`<br>` when needed to preserve the requested line on reprojection.
In WYSIWYG, Shift-Enter inserts an explicit line break within the current paragraph:
HTML uses `<br>`, Markdown uses a backslash followed by a source line ending,
and RTF uses `\line`. Markdown uses inline `<br>` where a physical source
ending would change paragraph structure, such as headings and empty items.
Bare inline `<br>` and `<br />` project as breaks; escaped tags and code spans
retain their literal text. Markdown quote/list continuation syntax keeps the break
inside the same paragraph and item. In preformatted Markdown code and plain
text, the source line ending itself expresses the break. This is distinct from
automatic soft wrapping and from Enter's paragraph-splitting behavior. Source
views retain literal source-line-ending insertion for Shift-Enter.
Typing spaces or tabs in an HTML context that already preserves whitespace
uses literal source whitespace; it does not add nested preservation spans.
Explicit HTML whitespace overrides retain their own semantics.

Markdown ordered and bulleted item continuations flow together into item
paragraphs, including lazy continuations and indented continuation paragraphs.
Ordered display labels count from the first source ordinal; untouched source
marker spellings remain exact. Markdown and HTML list levels default to a
32pt logical start inset per level, zero paragraph spacing, zero first-line body
indent, and hanging labels. The label gutter is independent of the signed
first-line body indent, so explicit positive and negative values remain active.
The measured label occupies the hanging area; first-row item text and following
rows align at the body inset, including when the ordinal gains a digit. A
heading or code paragraph within an item retains its own paragraph style and
the enclosing list inset. In every WYSIWYG format, bullets and ordered labels,
including their following gap, are layout decorations outside the formatted
text. They have no text, register, selection, hit-test, or caret positions. The
first body grapheme is the first editable character. At a wrapped list row end,
`$` and `A` target the final body grapheme and its following boundary before
wrap-separator whitespace. Source-visible views retain literal list syntax.

The two HTML views similarly share one physical HTML serialization:

- **HTML WYSIWYG** displays the interpreted document. Typed `<`, `&`, quotes,
  and other syntax-sensitive characters are encoded as appropriate HTML text
  or entity syntax and must not accidentally create markup. Nonstructural
  source line endings collapse with HTML whitespace. Return and open-below at
  the end of a document retain editable blank rows using `<br>` where needed;
  later input, Backspace, and undo preserve those semantic boundaries.
- **HTML Source** displays every decoded source character, including tags,
  comments, attributes, entities, and uninterpreted script/style contents.
  Encoding and logical line-ending normalization still use the shared pipeline;
  original bytes and delimiter spellings remain authoritative. Source edits
  immediately update semantic formatting, and formatting actions update the
  corresponding source markup. Switching views preserves source bytes.

HTML Source overlays configurable internal character styles on brackets, tag
names, attribute keys, attribute values, attribute equals signs, entity names,
and uninterpreted content. Their names start with `* HTML`. Generated defaults
declare only foreground colors; all other properties come from the underlying
content style. Automatic applications form a separate sparse overlay, so they
do not reset the underlying font, weight, size, or semantic styling. Internal
definitions appear in Edit Styles, but cannot be manually assigned and do not
appear in Paragraph or Character assignment menus. Current-style queries and
typing inheritance ignore the automatic layer. These definitions use stable
identities and generated buffer configuration, like generated Markdown styles.
Their customization survives edits and history navigation in that buffer,
never changes HTML bytes, and returns to defaults when a document is reopened.

With Flow Source Paragraphs disabled, HTML Source uses physical source lines as
its displayed paragraph units, except that each preformatted block shares one
paragraph across its source lines.
Recovered semantic paragraph styles contribute character defaults and named
style identity to their source spans. Paragraph spacing and alignment remain
editable source properties and take full effect in WYSIWYG; independent HTML
paragraphs on one physical source line cannot each align that same displayed
line differently. With Flow Source Paragraphs enabled, semantic blocks have
independent presentation paragraphs, including adjacent blocks on a single
physical source line and empty blocks. Their paragraph styles, spacing,
alignment, and indentation apply as in WYSIWYG, with the surrounding source tags
additionally visible. Nonstructural physical breaks flow within the corresponding
structural presentation paragraph; preformatted internal breaks remain literal.
These presentation boundaries do not change source bytes or source coordinates.
The default source view does not invent additional visible line breaks.

### Authored-input assistance

In HTML Source prose, typing a single `<` inserts `<>` and leaves the caret
between them. As the opening name is authored, Viem maintains a generated end
tag: `<b|></b>` becomes `<br|>` when `r` is typed because `br` is a void element.
All standard HTML void elements and explicit self-closing tags omit the end
tag. Attributes retain literal quote syntax. Typing `>` at the generated
opening delimiter advances over it rather than duplicating it; Backspace while
authoring the opening tag updates its generated suffix, and backspacing the
initial `<` removes the empty generated pair.

Only the most recent automatic insertion is tracked, using persistent anchors
and exact generated-text validation. Changing generated text, moving the caret
away, leaving Insert mode, or an unresolvable rebase retires that annotation.
Existing and pasted source is never automatically repaired. Assistance does not
run inside tags, attribute values, comments, entities, or uninterpreted raw
text. Each assisted keystroke is one verified transaction within the enclosing
Insert undo group; counted insertion, dot, and macros retain its input intent.

Settings includes an **Editing** category with **Smart quotes**, initially off.
This application preference is propagated to every view and never changes
document source merely by being toggled. It transforms individually authored
straight quotes in prose, not pasted text, registers, command prompts, or
syntax-required quotes in HTML Source and Markdown code/link/tag constructs.
Beginning of text or a logical line, whitespace, opening brackets, opening
quotes, and hyphen/en-dash/em-dash favor opening `‘` or `“`. Letters, digits,
closing punctuation, and other preceding content favor closing `’` or `”`;
apostrophes within words therefore close. HTML tags are ignored when finding
the surrounding prose, entities contribute their decoded text, hidden content
is excluded, and paragraph tags supply a line boundary. Context queries are
bounded; when preceding prose cannot be established within that bound, quotes
retain their literal spelling.

### Caret formatting and native selection

With no selected text in Insert or Replace mode, supported character-formatting
actions update a sparse, view-local typing override tied to the exact caret.
They do not insert empty HTML/Markdown/RTF wrappers, change source, or create an
undo unit. The next nonempty insertion combines text and its requested style
into one verified transaction. Repeated typing retains the override; explicit
caret movement, leaving the insertion mode, or changing projection retires it.
Menu checkmarks and typography queries show inherited style plus pending
overrides, without including automatic source-syntax colors. Formatting with a
selection continues to modify that exact range. Bold, Italic, and other
supported character actions work in RTF as well as the compatible HTML and
Markdown views.

Turning off an inherited inline property at the end of its element exits that
formatting context. In a source-visible view the caret moves over the matching
closing markup; in a formatted view its visible boundary remains unchanged and
the insertion context moves outside the element. At an interior text position,
the action retains that position and splits formatting around subsequent input
rather than skipping the remaining text. Nested unrelated formatting remains
in effect. Toggling a property without typing does not change source or history.

Marked text remains an IME overlay until commit, when its text and pending
formatting become one transaction. In Replace mode, Backspace restores the
recorded local source patches for each overwritten grapheme, including original
formatting and generated delimiters. It must not reconstruct the old state
from plain text alone or retain a full-document copy for each keystroke.

`Command-I` remains the standard Italic action. As an additional requested
editing shortcut, `Option-I` toggles the typing Italic property in Insert and
Replace mode when no input-method composition is active. In that context it
takes precedence over the keyboard layout's Option-I dead key; other native
fields retain their normal keyboard behavior. Underline's U icon includes a
visible underline.

A paragraph-style menu choice applies to the paragraph containing the caret
when there is no selection, and to the selected paragraph span otherwise.
Double-click selects the portable word under the pointer, respecting grapheme
boundaries. Triple-click selects the line using the active view's Visual or
Physical Source line policy. These native gestures invoke the same core
selection algebra and operator behavior as keyboard selection.

The Paragraph menu starts with Bulleted List, Numbered List, Indent, and
Unindent, followed by a separator and the paragraph-style choices. List commands
apply to the current paragraph or selected paragraphs. Indent and Unindent
change actual item nesting by one level, including the item's contained
paragraphs and child lists. Indent requires a preceding sibling to become the
parent and cannot move any selected descendant past the fourth level. Unindent
requires an existing parent; top-level items cannot be unindented. Availability
uses the same verified preparation as execution, including each adapter's
source constraints. Existing deeper source lists can still be unindented.
Modern RTF list items use local level-selector patches when their authored list
table has a compatible target level; legacy flat RTF lists and unavailable
target levels leave Indent and Unindent disabled.
Remove List remains available among the Format paragraph controls. Enter
continues an item; Enter on an empty item exits the list.
At the visible beginning of a list item in Insert mode, Tab and Shift-Tab invoke
the same verified Indent and Unindent actions when the format can express them.
An unavailable nesting change leaves the item unchanged. Source views also
recognize the semantic body beginning; Markdown Source recognizes both the
literal marker beginning and the boundary after its marker. Backspace at the
visible WYSIWYG beginning removes list treatment regardless of whether the caret
arrived by typing, navigation, or pointer placement; hidden tags and insertion
history do not redefine the beginning.
Numbered continuation and repeat calculate the next ordinal from current
structure. One list action and its supporting source patches form one undo unit.
New plain-text lists and Markdown source use `- ` or decimal `1. ` markers, with
sequential numbers across the selected items. Source-visible Markdown displays
and edits those markers; Markdown WYSIWYG renders canonical bullets and ordered
labels as described above. Markdown/HTML/RTF WYSIWYG labels are shaped and painted
from list metadata by layout; source marker syntax remains losslessly preserved.

Linewise deletion of complete list items removes their source structure and
selected paragraph boundaries as one structural edit. Characterwise deletion or
replacement of the entire body retains an empty list item. Deleting partial
visual rows retains the containing item. For HTML/RTF, surviving items retain their
displayed ordinals. The transaction may add explicit HTML `li value` attributes
or scoped RTF numbering overrides to preserve those ordinals; source tables and
unrelated opaque content remain untouched. Markdown numbering follows the
container's starting ordinal while preserving untouched source label spellings.
Deleting a hard line within an HTML list paragraph retains its item and any
unselected continuation paragraphs or nested lists. Only completely selected
item structure is removed; surviving descendants are not implicitly selected.
Enter advances following item numbers within the same list until an explicit restart or container boundary. HTML `li value`
restarts bound that change; RTF updates the affected legacy numbering controls
or adds scoped overrides while preserving table handles. New RTF paragraphs
clear inherited modern numbering with `\ls0`; the original selector resumes
at the existing following-paragraph boundary when necessary.

HTML lists use `ul`/`ol` and `li`, retaining unrelated attributes and descendant
markup. Canonical RTF list paragraphs use scoped groups with `\ls0\li400\fi-200`,
`\pntext`, and the standard `\pn` destination: `\pnlvlblt` for bullets or
`\pnlvlbody\pndec\pnstartN` with `\pntxta .` for decimal numbering. Clearing a
list uses a scoped `\ls0\li0\fi0` and `\pnlvlbody` reset. The scope includes
the existing paragraph terminator when present so independent RTF readers apply
its paragraph properties consistently. Existing source outside
the declared list-control patches remains byte-identical.

Word-style RTF `\listtable` and `\listoverridetable` definitions also project
decimal and bullet levels selected by `\lsN` and `\ilvlN`. Decimal levels use
the current level's number followed by a period. Start-at and format overrides,
nested level restarts, and list indentation are interpreted from their table
definitions. Cached `\listtext` remains untouched source and does not duplicate
the generated marker. Body edits retain both tables and their original handles.

Ctrl-Q is an alias for Ctrl-V in every supported Visual Block entry and toggle
path. Its rectangle resolves each proportional-font row to the nearest legal
caret/grapheme boundary and produces a logically discontiguous range set.
Counts, registers, operator execution, repeat, and undo follow the same path as
Ctrl-V.

Reliability validation includes native input and pointer interaction after font
metrics invalidation, resizing, wrapping, format changes, and undo/redo. A
metrics-generation change during layout retries only disposable layout work;
it never replays an input event or source transaction. Blank canvas beyond a
fully materialized document edge resolves to that edge. A genuinely missing
layout region remains an explicit coverage error.

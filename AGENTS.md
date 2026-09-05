# eVim product and engineering specification

This file is the normative product and architecture specification for this
repository. It also gives implementation guidance to coding agents. In this
document, **MUST**, **SHOULD**, and **MAY** have their usual requirements
meanings.

## Product intent

eVim is a modal, keyboard-first text editor for human-language writing. It
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
- Connect frontends to the Rust core through a narrow, stable C ABI. Keep
  ownership explicit and make performance-sensitive exchanges batch-oriented.

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
Source-language cascade rules remain adapter responsibilities: for example, an
HTML adapter resolves selectors, specificity, and CSS inheritance before
exposing normalized declarations and dependencies. The generic style resolver
does not reinterpret source CSS or RTF control state.

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
  declarations such as font and foreground color.
- **Base Paragraph** is a Paragraph-role child of Base Document and provides
  complete paragraph-layout values. Every other Paragraph-role style derives
  through it.
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
- font size in layout units, numeric weight, and slant;
- foreground and optional background color;
- underline and strike decoration;
- language and writing-direction override;
- OpenType feature settings; and
- letter spacing and baseline shift.

Bold and italic are command/UI conveniences that set numeric weight and slant;
they are not separate booleans that can disagree with those properties.
Portable color and font requests are value types and contain no native handles.
The resolved character style passed to shaping contains concrete values for
every required property.

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

Splitting a paragraph normally copies its paragraph-style assignment and direct
paragraph declarations to the new paragraph. A paragraph style may name a
`next_paragraph_style`; when present, Enter at the paragraph's terminal boundary
uses it for the new paragraph. Joining paragraphs keeps the first paragraph's
style for the result unless the format adapter reports that source semantics
require an explicit policy choice.

#### Future structured blocks

Lists are document structure, not a bullet character embedded in text and not
merely a paragraph-style flag. The formatted block tree is designed to add List
and List Item nodes carrying list identity, nesting level, marker/numbering
policy, and optional style references. List-role styles use the same block-style
record and inheritance mechanism, while the property schema permits list-only
marker and numbering declarations for that role and rejects them for Document
or Paragraph roles. Paragraphs inside an item continue to use Paragraph-role
block styles. A list style may contribute hanging indentation and spacing at the
reserved structural cascade layer and may reference a character style for its
generated marker. Generated markers have explicit synthetic provenance and
caret/edit rules.

The same structural contribution mechanism may later support quotations,
tables, callouts, or other block containers without adding format-specific
fields to paragraph styles. These future node kinds and properties are not part
of the initial implementation or command commitment.

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
  to translate a paste. Plain-text system clipboard interchange is required;
  rich clipboard interchange is desirable but may be added separately.

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
- `wrap` controls whether soft wrapping is active. `linebreak` controls whether
  wrapping prefers Unicode word/line-break opportunities; it defaults on.
- With `wrap` and `linebreak` on, text wraps at Unicode-appropriate word/line-
  break opportunities to the view's current usable text width. With `wrap` on
  and `linebreak` off, it wraps at legal shaping-cluster boundaries. A cluster
  wider than the row may overflow rather than be split illegally.
- If no legal word boundary fits, the layout MAY fall back to a legal grapheme
  or shaping-cluster boundary so progress is always possible.
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

### Meaning of "line" while wrapped

The central deliberate departure from gVim is vertical navigation:

- With wrapping on, standalone `j`, `k`, Down, and Up move by visual row.
- With wrapping off, they move by hard line as in Vim.
- `gj` and `gk` always move by visual row; while wrapped they are aliases for
  `j` and `k`.
- Vertical movement retains desired x in layout units and hit-tests that x on
  the destination row. The desired x is reset by an explicit horizontal move,
  mouse placement, or edit that establishes a new caret position.
- Counts count visual rows in wrapped mode and hard lines otherwise.
- Visual character/block selection endpoints follow the same movement rule.
- `0`, `^`, `$`, `g0`, `g^`, and `g$` retain Vim's meanings: the first three
  address the hard line and the `g` forms address the visual row. This keeps
  both operations available.
- Doubled linewise operators (`dd`, `cc`, `yy`, `>>`, `<<`) and Visual Line
  mode operate on hard lines, not soft-wrapped rows. A soft wrap is never a
  stored line boundary.
- In operator-pending mode, `j` and `k` retain Vim's hard-line, linewise
  semantics. This avoids making delete/yank results depend on window width.
  `gj` and `gk` are visual-row characterwise motions when explicitly used.
- Ex command ranges and line numbers always address hard lines.

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

- Normal and Visual modes: a block around the shaped cluster under the cursor;
- Insert mode: a thin vertical insertion caret at the caret stop;
- Replace mode: an underline or low horizontal bar under the cluster that will
  be replaced; and
- Command-line mode: a thin insertion caret in the command line.

The caret must remain visible against selection and text colors, blink according
to platform accessibility preferences, and stop blinking while keys are being
processed. Empty lines and end-of-line positions need explicit geometry; never
assume a glyph exists under the caret.

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
- document: `gg`, `G`, `{count}G`, `{count}%`;
- structure: `%`, `(`, `)`, `{`, `}`;
- viewport: `H`, `M`, `L`, `Ctrl-F`, `Ctrl-B`, `Ctrl-D`, `Ctrl-U`,
  `Ctrl-E`, `Ctrl-Y`, `zz`, `zt`, `zb`; and
- marks/jumps: `m{a-z}`, `` `{mark} ``, `'{mark}`, `Ctrl-O`, `Ctrl-I`.

Sentence and paragraph motions should use Unicode-aware human-language rules
with Vim-compatible blank-line behavior. Their exact segmentation rules must
be test fixtures, not ad hoc calls to a frontend API.

### Search

Required search commands are `/pattern`, `?pattern`, `n`, `N`, `*`, `#`,
`g*`, and `g#`. Search operates on logical UTF-8 text in the formatted
projection and is independent of wrapping. Matches may cross style boundaries.
Search/replace changes are reverse-projected like other edits. The first
implementation may use a clearly
documented Unicode regular-expression dialect rather than Vim's full regex
dialect; incompatible Vim atoms must produce an error rather than be
misinterpreted. Search history belongs in core state.

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
remain unchanged. A literal CR or LF supplied through text input remains
literal and may be rejected atomically when the active source pipeline cannot
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
pre-composition projection without changing source.
Core commands must not see partially decoded key events as text.

Insert/Replace source transactions are grouped according to the undo-unit rules
below. In particular, explicit cursor moves, `Ctrl-O`, paste boundaries, and
IME commits create deterministic undo breaks.

### Visual modes

Required commands are `v`, `V`, `Ctrl-V`, `gv`, `o`, `O`, Escape, all supported
motions, supported operators, `x`, `s`, `r`, `J`, `~`, `u`, `U`, `>`, `<`,
`y`, `d`, `c`, `p`, and `P`. Visual selection is inclusive in Normal/Visual
Vim terms while internal APIs use explicit half-open ranges.

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
- Required commands: `u`, `Ctrl-R`, `U`, `.`, `q{a-z}`/`q`, `@{a-z}`, and
  `@@`.
- `U` restores the hard line on which the latest eligible change was made,
  even if the cursor subsequently moved to another line. Its baseline is an
  exact source-backed line image identified by stable projected identity, not
  a flattened formatted string or retained numeric offset. Repeated eligible
  changes to that same line retain the original baseline; a change on another
  line replaces it. A topology change which makes the identity ambiguous
  invalidates the slot explicitly. `U` swaps the current and retained images,
  is itself one ordinary source transaction, and participates in branching
  undo/redo and dot repeat.
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

`U` is a change command, not history navigation. The buffer retains a line-undo
slot containing the stable hard-line identity and exact source-backed baseline
from before the current consecutive run of changes confined to that line.
Further changes confined to the same hard line keep the baseline; a change
involving another line or a hard-line boundary replaces or clears the slot as
applicable. `U` issues a typed line-baseline restoration, from which the adapter
produces the minimal patches needed to restore the saved source slices and then
verifies the projected line. It creates a new undo unit. After it commits, the
replaced line becomes the new baseline so another `U` can reverse the previous
`U` in Vim-compatible fashion. Adapters report a non-destructive error if the
saved baseline can no longer be restored unambiguously.

#### Save points, retention, and persistence

A successful write records the current source-snapshot identity as the
buffer's persisted save point and may annotate the current history node with a
monotonic write number. Saving does not create an undo unit. A failed write
does not move the save point. The buffer is clean exactly when its current
source-snapshot identity is the persisted identity; undoing away from a saved
node makes it dirty and returning to that exact snapshot makes it clean again.
Changing file identity with a successful `:saveas` is not undone.

History retention has configurable node and retained-byte budgets and must
account for structurally shared source buffers rather than charging each
snapshot its apparent full size. Pruning removes the oldest non-current leaf
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
Backspace/Delete, history Up/Down, Escape, and Enter.

Required Ex commands and common unambiguous abbreviations are:

- files: `:edit`, `:enew`, `:write`, `:saveas`, `:quit`, `:qall`, `:wq`,
  `:xit`, `:wall`, and force `!` variants where meaningful;
- editing: `:undo`, `:redo`, `:delete`, `:yank`, `:put`, `:join`,
  `:copy`, `:move`, and `:normal` for the supported Normal command subset;
- search/change: `:substitute` with ranges and repeat flags, `:&`, and `:~`;
- navigation/info: numeric line addresses, `:goto`, `:marks`, `:registers`,
  and `:jumps`; and
- options: `:set`, `:setlocal`, `:set wrap`, `:set nowrap`, `:set linebreak`,
  `:set nolinebreak`, `:set fileformat?`, and
  `:set fileformat=unix|dos|mac` (where one value is supplied), including the
  `ff` abbreviation and corresponding `:setlocal` forms. The global
  `fileformats` open-policy option supports query and ordered assignment even
  though changing it does not reinterpret an already open buffer.

Ranges always use hard lines. File dialogs, unsaved-change prompts, and error
presentation are frontend responsibilities driven by typed core requests and
results. Write commands serialize the authoritative source artifact, preserving
unchanged source slices exactly; they never export a newly normalized formatted
document as a substitute for the source.

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

### Explicitly deferred compatibility

The following are outside the initial command commitment unless a later change
adds them here: Vimscript/Vim9script, user mappings and abbreviations, plugins,
terminal jobs, shell filters and `:!`, tags, quickfix, diff mode, folding,
spellchecking, code syntax highlighting, multiple Vim splits/tab pages,
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
not the complete public surface of every internal module. The stable C ABI wraps
this facade. Rust module types may evolve internally without expanding the ABI;
positions, edits, queries, and layout data crossing the ABI remain explicit,
revision-tagged, ownership-safe, and batch-oriented.

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
including after view removal. Exhaustion is a typed failure; compatibility APIs
that cannot return it may fail explicitly rather than wrap. When an exact visual-
row command reaches partial snapshot coverage, its non-mutating result carries
a typed, revision- and generation-bound layout demand. That demand identifies
the missing edge and the complete bounded hard-line interval for a replacement
viewport request, so the frontend does not guess an expansion direction or
range. A demand whose source, layout, configuration, or metrics identity is no
longer current is rejected as stale.

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
  anchor remapping, saved/dirty transitions, history pruning, line undo, and
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
  range/geometry APIs.

## Architecture lessons adopted from Vim

Vim is a guide, not a code template. Preserve these useful separations:

- Vim's `normal.c` parses Normal/Visual commands and collaborates with operator
  code; eVim likewise separates grammar, motion resolution, semantic edit
  intentions, and verified source mutation.
- Vim's buffer/window distinction maps naturally to eVim's shared buffer and
  per-view layout state.
- Vim's `memline.c` stores line data in a block tree; eVim also needs balanced,
  aggregate storage for authoritative source and derived formatted text,
  adapted for provenance, rich spans, and Unicode positions.
- Vim remembers screen state and uses validity/invalidation levels to minimize
  redraw; eVim extends the idea into segmentation, shaping, wrapping, height,
  and viewport cache layers.
- Vim treats folding as a presentation transform between buffer lines and
  displayed lines; eVim treats soft wrapping the same way and never writes
  visual rows back into document text.
- Vim keeps alternate undo branches; eVim's transaction history does too.

Do not copy Vim's terminal-cell assumptions. Pixel advances, variable row
heights, shaping clusters, bidi affinity, and resize-driven reflow are
fundamental eVim concepts rather than frontend patches.

Primary references:

- [Vim command index](https://github.com/vim/vim/blob/master/runtime/doc/index.txt)
- [Vim motions and operators](https://github.com/vim/vim/blob/master/runtime/doc/motion.txt)
- [Vim changes](https://github.com/vim/vim/blob/master/runtime/doc/change.txt)
- [Vim Visual mode](https://github.com/vim/vim/blob/master/runtime/doc/visual.txt)
- [Vim development design decisions](https://github.com/vim/vim/blob/master/runtime/doc/develop.txt)
- [Vim Normal command dispatcher](https://github.com/vim/vim/blob/master/src/normal.c)
- [Vim memline block tree](https://github.com/vim/vim/blob/master/src/memline.c)
- [Vim screen cache](https://github.com/vim/vim/blob/master/src/screen.c)
- [Vim window redraw/invalidation](https://github.com/vim/vim/blob/master/src/drawscreen.c)
- [Vim undo branches](https://github.com/vim/vim/blob/master/src/undo.c)

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

## Decisions still intentionally open

Do not silently settle these while implementing an unrelated feature. Record a
decision in this file or an architecture decision record first:

- build/package layout;
- which format adapters beyond plain text ship initially;
- each adapter's default authoring policy, such as Markdown delimiter and HTML
  element/style choices;
- user policy for Unicode edits not representable in the source encoding;
- exact Unicode word/sentence segmentation tailoring;
- exact regular-expression syntax supported by `/` and `:substitute`;
- whether rich system clipboard formats are required for the first release;
- default Base Document/Base Paragraph/Base Character values, view chrome
  insets, colors, and other visual design choices;
- hyphenation and justification; and
- concrete latency and memory budgets for supported hardware.

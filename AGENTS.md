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

Code is an additional literal-text format with pluggable syntax highlighting,
bundled Tree-sitter languages, and Vim syntax fallback. Its source remains
fully visible and editable while optional highlighting is computed separately.

The goal is not source compatibility with Vim, a Vimscript runtime, or a
pixel-for-pixel gVim clone. "Vim compatible" in this project means that every
command explicitly listed in this file follows Vim's command grammar and
observable editing semantics, except where this file defines a deliberate
word-processing behavior.

The native frontends are macOS and Windows. Windows follows the macOS editor's
presentation and behavior with Windows controls and the platform differences
recorded below. A terminal frontend remains outside the current scope.

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
- Implement the Windows frontend under `src/win` in C# using WinUI 3 and
  Win2D/DirectWrite. Windows UI, file, clipboard, input, and lifecycle mechanisms
  belong there; document and command policy remains in the Rust core.
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

Packaging copies `assets/icon/Viem.icns` into the app bundle's
`Contents/Resources` before signing. The app's `CFBundleIconFile` declaration
selects this icon for both debug and release builds.
Packaging also verifies the pinned `assets/vim` snapshot, replaces the bundle's
`Contents/Resources/vim` subtree with it, and verifies the packaged copy before
signing. `scripts/vim-runtime.py` uses Python 3's standard library to verify
file inventories and SHA-256 hashes. Builds do not require an installed Vim or
download runtime files. The snapshot retains nested syntax helpers, original
bytes and attribution, and the source runtime's license.
After signing, packaging updates the app bundle directory's modification time
so Launch Services detects changed icons and bundle metadata on the next launch.

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
  style definitions associated with one formatted snapshot. Paragraph styles form the block-style namespace; source-root declarations
  retain source-format document context separately.
- **Direct formatting**: sparse block or character property declarations
  attached to content after named-style assignment; it does not mutate the
  named style.
- **Document**: a source artifact, its configured transformation pipeline, and
  cached derived projections.
- **WYSIWYG view**: a format which presents block structure and named styles
  instead of the syntax spelling them: Markdown, HTML, and RTF.
- **Source view**: a format whose own markup is visible, editable text:
  Markdown Source and HTML Source.
- **Code**: a literal-text format with automatic syntax styles, distinct from
  the Normal/Insert/Replace command modes and from rich-format Code styles.
  It has no WYSIWYG counterpart. All pre-existing format-family predicates
  below are false for Code; `Format::is_code` is true only for Code.
- **Rich text**: a WYSIWYG view whose source persists arbitrary character and
  paragraph declarations: HTML and RTF. Markdown carries structure but only a
  fixed inline vocabulary, so it is structured without being rich text.

  These families are `const fn` predicates on `Format` (`is_wysiwyg`,
  `is_source_view`, `is_rich_text`, `is_markdown`, `is_html`,
  `has_rich_source`, `has_structural_lists`, and `is_code`). Implementations
  MUST ask through these predicates rather than spelling a `matches!` set of
  variants, so that adding a format is a deliberate decision at each predicate
  instead of a search for every call site. Their membership is pinned by test.
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

Deleting valid visible Markdown text, including the last character of an inline
code span and selections across physical or visual rows, MUST remain possible.
The translation owns supporting delimiter removal, merging adjacent code spans,
and preserving retained folded spaces when deleting text changes how the source
is parsed. Those repairs remain minimal explicit patches in the same verified,
undoable transaction; candidate verification is never bypassed.

Ordinary valid WYSIWYG deletions MUST also succeed through native selections,
Visual Line, and caret Delete/Backspace at structural and document boundaries.
Selection normalization must retain enough pre-edit identity to rebase remembered
selections, marks, and history endpoints; a former EOF offset cannot be reused in
the shortened document. An emptied implicit paragraph needs explicit supporting
syntax when its unselected paragraph boundaries would otherwise disappear on
reopening. Fully selected structural owners may be removed, but an unselected
empty paragraph at a half-open range edge remains outside the edit. Deleting an
atomic object owns its fallback body and embedded opaque syntax, while unrelated
document metadata and hidden content remain intact. Tests MUST exercise command
entry paths, saved/reopened projections, and exact undo/redo, not only prepared
document text edits.

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
- Inserting immediately after a dangling final UTF-16 byte repairs that byte
  to encoded U+FFFD when necessary to keep subsequent code units aligned. This
  is an automatic local supporting patch in the insertion transaction; it
  preserves the visible diagnostic and requires no modal question. Undo restores
  the exact malformed bytes. Edits elsewhere do not repair unrelated bytes.
- Unchanged decoded text is saved by copying its original bytes, not by
  re-encoding it.
- New or changed Unicode text is encoded using the original encoding when it
  is representable. Otherwise the format adapter must return a policy result:
  use a semantically exact format escape, change the document encoding and any
  declarations, or reject/request a user decision. Silent substitution or data
  loss is forbidden.
- HTML and RTF use exact native escapes for otherwise unencodable input.
  Markdown WYSIWYG prose uses numeric references where they reproduce the
  requested Unicode character. Code spans/fences and literal/source views retain
  exact literal semantics; an entity spelling must not masquerade as a character
  there. Ordinary input never silently changes the file's encoding.

### Shared text line-ending projection

Line-ending interpretation is a reusable pipeline component, not behavior
reimplemented by each format adapter and not a base class from which adapters
inherit. A text-like adapter composes a `TextLineEndingProjection` immediately
after decoding and before its lossless format projection. Text, Code, Markdown,
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
    -> PlainText | Code | Markdown | HTML lossless format projection
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
regardless of whether their later adapter is Text, Code, Markdown, or HTML. The
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
synthetic Base Paragraph style provides display defaults, with zero paragraph spacing by default so the result has
gVim-like line placement. Plain text exposes no source-backed named styles or
direct formatting capabilities.

Ordinary decoded characters, including CR or LF characters not recognized as
delimiters under the selected open interpretation, remain formatted content and
retain byte provenance. The frontend may draw visible control representations,
but the adapter does not remove or normalize them. Aside from the explicitly
documented visual-row navigation behavior, plain-text commands operate on the
same logical lines and content as gVim fixtures.

Code uses this same identity projection and source-editing behavior, with the
global Code stylesheet and independently scheduled syntax overlay specified in
"Code format and syntax highlighting". Highlighting is not a prerequisite for
projection, command execution, or saving.

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

- A **block style** applies to a typed paragraph block node. It contains
  paragraph layout declarations and character declarations that provide the
  paragraph's default text appearance. A heading paragraph style can therefore set spacing and
  indentation as well as font family, size, weight, or color.
- A **character style** applies to a formatted text range and contains only
  character declarations. It does not change paragraph geometry.

Style identity is an opaque stable ID, not the user-visible name or an array
index. Renaming or reordering a style does not invalidate assignments to it.
Disposable syntax style-name references are resolved separately as specified
under Code; they are not authored stable-ID assignments.
Each style has at most one parent in the same namespace. Multiple inheritance
is forbidden. Every non-root paragraph style ultimately derives from Base
Paragraph. A character style with no parent inherits the current paragraph's
appearance; named character parents contribute their sparse declarations on
that same contextual foundation.
Parent links must be acyclic; a missing parent, role violation, or cycle
produces a diagnostic and deterministically falls back to the applicable base
path without discarding source syntax.

Deleting a non-base style is allowed only when the same atomic intention
reassigns its children and every content assignment, or when none exist.
Otherwise deletion is rejected; it never leaves silently dangling style IDs.

Every style sheet defines one distinguished style, **Base Paragraph**. It is
the root of the paragraph hierarchy, has no parent, provides complete paragraph
layout and default character values, and cannot be deleted. It is the default
paragraph assignment and the fallback selection for style-editing UI. Its
initial generated font is **SF Pro on macOS and Segoe UI on Windows, at 14
layout units**; other hosts use `system-ui`. Code uses the system
monospace family at the same size. Every other paragraph style derives through
it. Adapters identify source-backed and generated declarations.

**Default Paragraph** is the user-facing character-style choice meaning no
named character style is applied. It is not a stored definition or assignment,
cannot be edited, and never appears in the style editor's Style picker. It
resolves to the containing paragraph's current character appearance. It is the
default parent choice for character styles; internally an absent parent ends
the named character chain. Explicit character-to-character inheritance remains
available, including Code syntax style links. Choosing Default Paragraph clears
named character styles in the selection, or establishes unstyled subsequent
typing at the caret without a source edit until text is inserted.

There are no Base Document or Base Character definitions. View margins belong
to Settings > View, not to a style. Source-root declarations needed to preserve
HTML or RTF source semantics remain distinct from editable named styles.

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
role: for example, a Paragraph-role style cannot change the document canvas
padding. Document Canvas declarations belong to source-root context rather
than an editable named style. Inapplicable declarations are diagnostics, not silently ignored
values. Target-only properties affect the assigned block; inheritable properties
also contribute to applicable descendants.

Initial Document Canvas properties include:

- canvas background color; and
- logical start, end, top, and bottom padding.

Base Paragraph's Character declarations define the default content font
request, size, and foreground color. Selection, caret, diagnostics, gutter,
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
- OpenType feature settings;
- letter spacing; and
- script position: Normal, Superscript, or Subscript.

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

Each paragraph stores a Paragraph-role block-style ID plus sparse direct
paragraph and paragraph-default-character declarations. Source-root context
retains direct canvas and default-character declarations when needed for source
constructs such as HTML body styling, without a separate named document style.
Character-style assignments and direct character formatting are separate range
maps. At most one named character style applies at a position; absence is
Default Paragraph. Different direct properties may overlap independently.

Paragraph geometry resolves engine emergency values, the Base Paragraph to
assigned paragraph chain, applicable structural geometry, and direct paragraph
declarations, with later declarations winning. Source-root canvas declarations
remain source-format context; application view margins are independent.

Character appearance resolves the complete paragraph appearance first,
including Base Paragraph, source-root defaults, the assigned paragraph's
ancestors, and direct paragraph character declarations. Then apply only the
sparse declarations in the assigned character chain, ancestor to descendant,
followed by structural contributions and direct character declarations. An
absent character parent or assignment contributes no additional properties.
Source-authored defaults retain their precedence over generated defaults.

For example, a Code character style declaring Courier and green, with no size,
is 12 point in a 12-point paragraph and 20 point in a 20-point heading. It never
reapplies Base Paragraph's size over the heading. Direct formatting does not
mutate or implicitly create a named style.

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

Replacing a nonempty text selection inherits the character style and direct
character declarations of its first selected character in document order,
independent of selection direction. The context is captured before deleting
the range and applies to continued typing. An interior style or link that is
completely consumed must not leak into a replacement beginning in ordinary
text. A replacement beginning in styled or linked text retains that context,
even when it consumes the entire original run. Explicit pending typing
formatting takes precedence over this inherited context.

Leading paragraph separators are skipped when finding that first selected text
character; ordinary spaces still count. A selection containing only separators
uses its pre-edit insertion context and does not inherit a hyperlink from a
neighboring paragraph. Empty paragraphs supply their explicit typing context or
paragraph defaults. Replacing a whole paragraph retains the first selected
paragraph's style assignment and direct paragraph declarations, including when
the selection consumes the entire document. Joining paragraphs retains the first
paragraph's style, with retained text keeping its character formatting.

Choosing an assignable character style with no selection sets a view-local
pending named style for subsequent typing. This retains the style's identity,
separate from direct character declarations, and immediately updates character
style menu state. The gesture itself does not change source bytes or revision,
mark the buffer dirty, or add an undo entry. Choosing it in Normal mode carries
it into the next Insert or Replace session. Text and its pending style commit
together in one verified transaction; failed preparation preserves the pending
style and editor state.

Direct character choices without a selection likewise remain pending view
state. Moving the caret or starting a new selection discards pending character
choices without creating source syntax, dirty state, or an undo entry. Typing
outside an existing character scope must not manufacture an empty copy of that
scope at a split boundary. When an HTML text edit consumes the final content of
a character-only scope, remove its now-empty delimiters, including nested
scopes, in the same verified undoable transaction. Preserve contained comments,
unrelated authored empty scopes, unknown elements, and scopes carrying anchors
or other non-formatting metadata. Paragraph owners are not character cleanup
targets.

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

Backspace at the visible beginning of a nested list item reduces its list
indentation by one level without deleting text. Repeated Backspace reaches the
top list level one step at a time; at the top level it removes the list
treatment and assigns the normal paragraph style. Backspace at the visible
beginning of a code paragraph likewise removes that structural treatment. A
top-level reset removes every enclosing structural layer that would otherwise
keep that paragraph in a list, quote, or code treatment. At the beginning of every other paragraph it removes the previous
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

Deleting a displayed row obeys the same rules as deleting a character range.
HTML line/owner deletion must protect retained spaces that become exposed at a
paragraph edge using the existing nonbreaking-space policy. Markdown deletion
and replacement must escape retained literal punctuation when newly adjacent
text or a new line boundary would otherwise activate markup. These are local
supporting source repairs, not reasons to reject an ordinary visible-text edit.
Completely consumed Markdown emphasis/link scopes remove their now-empty
delimiters; replacement typing recreates the first character's semantic context.
Candidate verification remains an internal consistency guard; a mismatch is an
editor defect, not a user-facing limitation of deleting formatted text.

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

### Links in Markdown and HTML

Markdown inline links `[label](destination)` and HTML anchors with `href`
contribute an automatic Link character style, blue and underlined by default.
The style remains editable in the format's style sheet; explicit authored
character styles and direct declarations take precedence over its defaults.
Markdown WYSIWYG shows
the formatted label; Markdown Source styles the entire inline link construct.
HTML WYSIWYG styles visible anchor contents; HTML Source styles only the content
between the anchor tags, excluding the opening and closing anchor tags.
Recognition is passive and keeps source bytes unchanged. Inline and fenced code
do not acquire Markdown link styling. Escaped punctuation, balanced destination
parentheses, angle destinations, optional titles, and character references are
recognized without fetching their targets. Reference-style Markdown links and
autolinks are outside this initial inline-link feature.

Right-clicking actual link content puts Open link first in the edit context
menu, followed by a divider. Keyboard context menus use the current caret.
Opening revalidates the exact document revision and resolves the destination
from current source; stale menus cannot open an old target. Native URL APIs
open HTTP, HTTPS, and file destinations in the default web browser, with relative
destinations resolved against the containing document's URL. No destination is
passed to a shell. Existing percent escapes are preserved, additional invalid
URI bytes are encoded once, and control characters or executable URL schemes
are rejected. Other document interaction does not launch links.

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

Typing into an empty HTML WYSIWYG document authors an explicit `p` element.
Nonempty text edits in an anonymous prose paragraph materialize that paragraph's
`p` owner when needed, preserving its existing inline content and surrounding
source. Existing paragraph owners (including headings, list items, preformatted
blocks, and assigned block containers) are reused. This is local editing policy,
not normalization on open, save, or navigation; unrelated paragraphs and empty
containers retain their original bytes. Empty character-scope cleanup does not
authorize collapsing paragraph boundaries or removing empty paragraph owners.
Deleting only a paragraph's text retains its empty owner and paragraph style.
Deleting a paragraph separator merges the adjacent paragraphs and retains the
first paragraph's style; intentional blank paragraphs remain until their own
boundaries are deleted. Explicit whole-document content deletion, including
linewise deletion of every paragraph, leaves one normal empty `p`. Replacement
typing can still restore the first selected paragraph's captured context.
Deleting nothing in an already empty normal paragraph leaves its source intact.
These operations preserve unrelated empty containers, comments, hidden metadata,
and source-only whitespace. Supporting source patches may move an untouched
empty container to the following boundary when it cannot remain inside the
merged paragraph, retaining its exact bytes and preventing style leakage.
An empty preformatted paragraph's insertion boundary follows any initial line
ending ignored by HTML parsing; typing retains that original source spelling
without exposing a new visible line.

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
- `s`, `strike`, and `del` contribute strike decoration;
- `sup` and `sub` contribute exclusive superscript and subscript; and
- `span` contributes no property by itself but can carry a supported class or
  inline declaration.

`lang` contributes the supported language property and `dir` contributes the
supported writing-direction override when their values are understood.
Explicit Automatic direction writes `dir="auto"` in HTML, including when it
must override an inherited LTR/RTL value. It is distinct from clearing a direct
declaration. RTF has no corresponding explicit automatic-direction control, so
that menu choice is disabled there. Other attributes do not affect the
normalized projection unless this section later adds an explicit mapping.

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
- `text-decoration-line`, and the `text-decoration` shorthand when its value is
  only `none`, `underline`, and/or `line-through`, -> underline and strike decoration;
- `letter-spacing` -> letter spacing;
- `vertical-align` values `baseline`, `super`, and `sub` -> script position;
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

Script position is a semantic enum, independent of font size: Normal,
Superscript, or Subscript. Superscript and subscript are mutually exclusive;
selecting the active choice again restores Normal. Rendering uses 70% of the
resolved font size and offsets superscript upward by one third of the original
size or subscript downward by one fifth. Normal retains the original size and
position. HTML `sup`/`sub` and CSS `vertical-align: super/sub/baseline` map to
these values; new simple inline edits use `sup`/`sub` tags. RTF uses
`\super`, `\sub`, and `\nosupersub`. Arbitrary CSS vertical lengths and RTF
`\up`/`\dn` remain untouched source but do not contribute formatting.
There is no editable numeric baseline-shift property.

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
- supported language, direction, character-spacing, script-position, and feature
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
- Font kerning is always implicitly enabled in shaping, layout, and rendering,
  including typography previews. It is not an editable or serialized style
  property and has no menu control. An existing source OpenType `kern`
  declaration is preserved but cannot disable kerning in Viem. Letter spacing
  is independent tracking; a zero value MUST NOT suppress font kerning.
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
  views and Code copy the original selected source as plain text only. Code
  register and clipboard payloads exclude automatic syntax styles. Copy Source
  (`Shift-Command-C`) publishes only plain text containing the source fragment
  corresponding to the selection, including markup in WYSIWYG views. Paste
  and Match Style ignores the private fragment and uses plain text. Malformed
  external private data cannot prevent ordinary commands or plain-text paste.
  On Windows, Vim's `+` and `*` register names both read and write the one
  system clipboard. Transient clipboard-owner contention is retried, and an
  unavailable optional rich representation falls back to available plain text.
- External plain-text paste normalizes CRLF and standalone CR to semantic hard
  breaks. A valid private register payload determines characterwise, linewise,
  or blockwise shape. Without one, Vim's clipboard fallback applies: text ending
  in CR or LF is linewise, while all other text is characterwise; internal line
  breaks alone do not make a payload linewise. A linewise system-clipboard write
  therefore ends its plain-text representation in a line break, including when
  the source's final physical line has none. Private payloads retain their
  declared break semantics. When pasting into HTML WYSIWYG, NUL becomes the
  visible U+2400 SYMBOL FOR NULL (`␀`), because HTML cannot retain the exact
  character. These transformations never ask modal questions or rewrite the
  clipboard. Exact internal registers, command-prompt input, literal-input
  commands, and representable NUL characters retain their existing semantics.

### Format interpretation and conversion

Every file-opening path MUST accept files whose type or extension is unknown
to Viem, including extensionless files and dotfiles such as `.vimrc`. When no
format is recognized, open the original bytes in Text using the shared lossless
encoding and line-ending projections. Native file-type admission MUST NOT
reject these files before the Text fallback runs. Recognized format defaults
and explicit format choices still take precedence.

Format changes carry an explicit operation in the portable core. Reinterpret
changes only the adapter applied to the current source artifact: every source
byte, encoding, BOM, and line-ending spelling remains unchanged. It must never
implicitly convert markup, including between Markdown and HTML.

Convert explicitly serializes the current formatted semantics into Text,
Markdown, or HTML. Source views first use their corresponding WYSIWYG
projection. Code uses its literal text, without syntax styles, as Text input
to conversion. This is a best-effort lossy operation: retain representable
paragraphs, headings, lists, code, and inline formatting; unsupported formatting
falls back to ordinary visible text. Unsupported objects use available alternate
or descendant text, with a readable placeholder when no text is available.
This explicit whole-document operation may replace all source bytes. It uses
the shared transaction, source correspondence,
anchor remapping, reprojection, and undo machinery rather than a frontend
serializer. Existing conversion-loss messages remain available.

For conversion, blank physical lines delimit Text paragraphs. A single newline
inside a Text paragraph remains a hard line break, represented by an HTML `br`
or a Markdown hard break. Each pair of breaks separates paragraphs; surplus
pairs create empty paragraphs and an unmatched break remains internal. Preserve
these empty paragraphs where the destination can represent them.
Conversion to Text places a blank line between
paragraph blocks, preserves internal hard breaks, and removes formatting and
generated list markers while retaining their text.

Each successful operation is one undo unit restoring both source and format.
Reinterpret and Convert are distinct from encoding conversion. The File menu
offers Text, Markdown, and HTML conversion targets and additionally Code as a
reinterpretation target. Selecting Code in the format popup or Reinterpret as
preserves source bytes and reveals their literal decoded text; Code introduces
no new serialization or separate Convert to Code action. Choosing a different
rich/structured family selects its WYSIWYG display when available. A same-family
source/view switch elsewhere can still reinterpret without changing bytes.

### Application theme and settings

The theme is an application preference, shared across views and independent of
source-backed named styles. It defines text foreground and canvas background,
caret and selection colors, status foreground/background and font family/size,
with color values in portable sRGB. The macOS Settings window has View, Theme,
and Editing categories; there are no Code or Documents categories. Code styles
remain editable through the ordinary modeless Styles inspector opened from a
Code view by F8 or the menu. Theme contains
its live preview, color controls, status typography, presets, and restore action.
View contains independent top/left/bottom/right text margins in pixels, with
defaults of 10 pixels on every side. Tab and Shift-Tab move forward and backward
through the Settings values, committing the field being left. The margin fields
follow their displayed order: top, left, bottom, right.
Margins are application view preferences stored under
`view.margins` in config.json and applied to every view, independently of theme
presets and document style definitions.

A missing foreground or canvas color means **Default**, resolved through the
theme at painting time. Generated base styles leave these colors unspecified.
An explicit document color, including black or white, takes precedence and is
not confused with Default. Default is not serialized as an explicit theme color.
Theme changes never modify source bytes, style declarations, history, or dirty
state. Color changes require only repainting. View margins inset content in
document coordinates and are additive with
source-authored canvas padding. Margin changes preserve viewport anchors and
invalidate only affected view geometry, keeping large-document layout local.
The top and bottom view margins define the vertical area for keeping an active
row visible. If the row's ink and typographic bounds already lie inside that
area, typing MUST leave the displayed rows stationary, including when appending
at a hard-line end. Revealing a row is a no-op in that case; it MUST NOT align
the row to the bottom or recenter it. Only a row outside the area permits the
smallest scroll needed to bring it inside. A cached layout region's boundary
is not a document edge and MUST NOT clamp a preserved viewport.
The bottom margin also contributes to the document's scroll extent. Margins
are not paint clips: scrolled content continues to draw through them to the
status line. If the row and margins cannot both fit, reduce the reserved area
enough to show the row; physically oversized rows keep
the baseline-priority policy below. At document end, scroll extent includes the
last row's full ink and natural height before adding bottom padding, even when
exact line spacing advances by less than that height.

Application preferences have one versioned JSON authority at
`~/.viem/config.json`. Theme, Smart Quotes, Code preferences, recent files, document-window geometry, and status-bar
visibility use this store; Settings controls write the same values. Valid legacy
preferences migrate once. Reads validate the complete configuration, writes are
atomic, and unknown keys survive updates. Invalid or unsupported versions are reported without
overwriting the user's file. `VIEM_CONFIG_DIR` may override the directory for
isolated development and testing.

All native profile-directory resolution goes through `EVProfileDirectory`:
an explicitly injected directory takes precedence over `VIEM_CONFIG_DIR`, then
the default is `~/.viem`. Configuration, Code styles, and startup commands use
that same resolved directory; consumers MUST NOT compute their own profile path.

`startup.viem` in this profile is an optional UTF-8 file of configuration Ex
commands. It is read once per application profile at startup; restarting reloads
changes. Missing files are silent and are not created. A UTF-8 BOM and CRLF are
accepted. The native loader bounds input to 1 MiB and reports unreadable,
oversized, or malformed files without overwriting them. Blank lines and lines
whose first nonblank character is `"` are ignored; a leading `:` is optional.
Errors include the file path and one-based line number, and later valid lines
still apply. Parsing and configuration policy live in the portable command
module, while filesystem access belongs to the frontend.

Startup accepts the supported `set`/`setlocal` settings, `nohlsearch`, and the
mapping commands below. Settings initialize every document and view after JSON preferences have
been applied, without changing source, dirty state, or document history. A
source-changing option such as `fileformat` cannot be assigned by startup.
`fileformats` assignment is also rejected until startup can configure the
pre-open decoding policy; accepting it after opening would have no effect.
Startup does not execute document edits, file/window actions, or Vimscript.
JSON preferences and `startup.viem` retain separate persistence: startup is an
explicit command override and is never rewritten by Settings controls.

Editing contains **Indentation and tabs** and **Visible whitespace** sections
alongside the existing editing controls. Indentation
defaults live at `editing.indentation`; leading whitespace and marker defaults
live at `editing.whitespacePresentation`. Missing fields inherit the defaults
specified below. Updates validate the complete candidate and preserve unknown
keys, including nested keys. Successful settings changes update open views;
invalid input leaves the last valid settings and the file untouched.

The Text width control in Editing uses this same store and the buffer-default
inheritance policy under **Hard-line reflow**; it is not a theme, per-view wrap
width, or source-backed style property.

The `recentDocuments` array stores the ten most recently opened or saved files
as full absolute paths, newest first. Successfully reopening an already open
file also moves it to the front. Canonical path and file identity prevent
duplicates, including symlink and hard-link aliases; using an existing entry
promotes it instead of adding another. Successful native and Ex opens/saves
share this list. Failed or cancelled file reads/writes, unnamed buffers, recovery
autosaves, and writes of an unopened copy do not add entries. Updates merge
against the current settings file and preserve unrelated settings. Clear Menu
clears this persisted list. Missing files remain listed until explicitly cleared
or displaced by newer entries; a failed reopen does not change recency.

Open Recent normally displays the filename, including its extension. When
different files have the same menu title, prepend parent path components to
each colliding title until every title is unique. Use only the suffix needed
for disambiguation, retain newest-first ordering, and keep the full path in
the menu item's open target and tooltip. The menu reads the application
settings authority rather than a separate operating-system recent-file list.

Per-document format defaults live beside it in `text_style.json`,
`html_style.json`, `markdown_style.json`, and `rtf_style.json`. A document loads
the matching sparse style defaults before source declarations are applied. The
cascade is built-in styles, user format defaults, source definitions/assignments, then direct
formatting. Built-in defaults seed ordinary editable definitions: Heading 1 is
a sparse delta on Base Paragraph, and its size, weight, and paragraph spacing
are declarations visible in the style editor. The same applies to other built-in
paragraph, character, internal, and Code styles. Clearing a declaration inherits
from its parent or contextual paragraph; it must not uncover a hidden copy of
that style's original default. Clearing every own declaration therefore leaves
only parent inheritance. Source-defined styles supply their own declarations
without an extra per-style fallback underneath them. Loading defaults does not
flatten parent values into child definitions.
An inherited default remains unset in source: changing an unrelated
property must not serialize an inherited font, color, or other declaration.
Explicit assignment of a custom default style materializes only declarations
needed to represent that assignment in a source-backed format. Source and
WYSIWYG variants share their format's defaults. Loading defaults is presentation
configuration and never changes source bytes, dirty state, or undo history.

Format > Style contains Edit Styles and Save as default <format> style.
HTML also exposes Include style definitions in file, as specified above.
A trailing separated Reload style sheet re-reads the global Code
`code_style.json` from disk; it stays enabled in every format because that sheet
is application-wide. Saving defaults exports the current style configuration to
the corresponding JSON file. Existing open buffers keep their current
configuration; subsequently opened buffers load the saved defaults.

Code instead uses the live application-wide `code_style.json` authority
specified below. It is not a copy of per-document format defaults: a global
Code style change updates existing Code buffers as well as future ones.


### Code format and syntax highlighting

#### Literal content and editing

Code is a selectable source format alongside Text, Markdown, and HTML. It
always displays every decoded content character, including delimiters, tags,
entities, comments, indentation, and empty lines. Only the shared encoding and
line-ending projections interpret physical serialization; invalid bytes retain
their diagnostic projection and original-byte provenance. Code MUST NOT hide,
collapse, replace, or synthesize content through syntax concealment, folding,
Markdown/HTML interpretation, or paragraph flow. Soft wrapping remains a view
option and does not change text. Every interpreted source-line break ends one
hard line and paragraph, with no invented final newline.

Code unconditionally suppresses smart quotes in every input path, even when
Smart Quotes is enabled, the language is unknown, or highlighting is missing.
Typing, Replace, `r`, IME/accessibility commits, paste, and register puts retain
the supplied quote characters. Existing curly quotes are not converted back.
HTML tag assistance, Markdown marker escaping, and prose/list continuation do
not run in Code. Syntax availability never changes editing semantics.

Code has no source-backed formatting, manually assigned character/paragraph
styles, direct formatting, or pending typing styles. Rich paste takes its plain
text representation. Syntax styles never enter saved source, rich clipboard
payloads, registers, semantic format conversion, or document undo history.
Reinterpreting into or out of Code preserves bytes using the normal transaction
and anchor-remapping rules; undo restores that format choice.

Explicit format selection takes precedence on open. Existing Markdown, HTML,
and RTF opening defaults remain unchanged. Otherwise, a recognized code-language
filename or load-time marker can select Code where opening would use Text;
unrecognized input remains Text. Once Code is selected, detecting a language
such as Markdown or HTML does not select its WYSIWYG format. Language and
source-format selection are separate buffer state.

Vim-script filenames, including `.vimrc`, `_vimrc`, `vimrc`, their gVim
counterparts, and `.vim` files, select Code with the `vim` language. Vim-script
highlighting uses the bundled Vim syntax files and their group links; it
does not use a Tree-sitter Vim grammar.

#### Global Code stylesheet and named syntax runs

There is one application-wide stylesheet named `code`, persisted atomically as
`~/.viem/code_style.json` beside `config.json`, respecting `VIEM_CONFIG_DIR`.
It uses the normalized style schema, with built-in defaults plus sparse saved
overrides, stable definition IDs, validation, and immutable revision identity.
It is shared live by all Code buffers. Valid changes update every open Code
view without modifying source, dirty state, or document undo history; undoing
a document edit does not restore an older global stylesheet.

The file is read once per application profile at startup and is never polled.
Format > Style > Reload style sheet re-reads it on demand, republishing it to
every open Code buffer even when the file appears unchanged, and reports a
missing, oversized, or malformed file without overwriting it. An in-app style
edit that finds the file changed underneath it refuses the write and reloads.

Core serializes mutations of the global authority and publishes an immutable
revision to buffer coordinators, without locking all buffers together. Layout
jobs retain the stylesheet revision independently of their text snapshot and
validate it on installation; shared style changes never mutate a retained
projection or install geometry resolved with an obsolete Code sheet.

The sheet contains Base Paragraph and named syntax character styles.
Code's initial Base Paragraph requests the system
monospace family at 14 layout units; default foreground/background use the
application theme. Base Paragraph defines line spacing, initially Single
(`normal`), with zero space before/after and zero paragraph indents. All Code
paragraphs use this default paragraph style. Syntax providers cannot set
paragraph geometry.

Providers emit coalesced named character-style runs over immutable input
ranges, using a `SyntaxStyleName` rather than retaining document offsets or
authoring style assignments. Tree-sitter capture names are canonicalized by
removing their leading `@` and uppercasing only the first letter: `@comment`
uses `Comment` directly and `@comment.documentation` uses
`Comment.documentation`. There is no separate `@comment` alias definition.
Vim names identify effective highlight groups after
the compiled, cycle-checked group-link mapping. The original language/group or
capture identity remains available for inspection. Both providers use the same
Code stylesheet; they never supply platform colors or native font objects.
Before stylesheet lookup, resolve overlapping candidates using the backend's
versioned priority/tie rules and parent/child-region precedence to one effective
named run at each position. Arrival order of asynchronous jobs never selects
the winning style. An unknown winning name uses the default appearance; it does
not expose a lower-priority candidate with a defined style.

Code character-style names are nonempty, case-sensitive, and unique within
their namespace; duplicate definitions are rejected atomically. Resolve the
emitted name by exact lookup in the current Code stylesheet. A missing name
renders with the default Code character appearance inherited from
its base styles and paragraph, with no additional syntax declaration. It MUST
NOT raise an editing error, create a style implicitly, retain an earlier color,
or guess a similarly named or less-specific style. Any group/capture aliasing
is an explicit versioned provider mapping, applied before this lookup. A
missing style differs from missing provider coverage: it does not activate Vim
fallback. Built-in definitions SHOULD cover the standard names emitted by the
bundled packages; users may add other names for separately loaded syntaxes.

Built-in syntax definitions MUST use explicit ordinary character-style
inheritance to share appearance across providers. For example,
`Comment.documentation` is based on `Comment`; `Keyword.function` is based on
`Keyword`, then `Statement`. A dotted built-in style's parent is its name with
the final dotted segment removed, with required intermediate definitions present
in the default sheet. Providers share the same definition when their canonical
names match. Shared groups own
the default paint, and descendants start with no redundant local declarations.
Editing a parent changes every inheriting property, including font metrics;
individual descendants may override properties or choose another valid parent.
This is a hierarchy of real definitions shown in the style editor, not a
fallback lookup for missing names. Existing default resolved colors are retained.

Code stylesheet storage version 3 records canonical names, linked defaults, and
sparse overrides. Version-2 sheets with `@` capture definitions are migrated on
read, including parent references and suppressed defaults. Where a former
capture alias and its canonical definition both have overrides, the capture's
own declarations win for the overlapping properties. Removing only one of the
old duplicate root definitions does not remove its surviving counterpart when
they become one shared style. A displaced custom display name is retained as
an independent user definition with its prior named appearance. If collapsing
aliases would create an inheritance cycle, retain the displaced parent's prior
appearance as an ordinary user definition and redirect the affected old parent
references to it. Existing custom names take precedence over newly canonicalized
names; a colliding migrated capture receives a unique display-name suffix while
exact provider lookup continues to use the existing custom definition. Version-1
copied declarations are unsupported. Reloads preserve explicit overrides even when
equal to default values.
Loading does not rewrite the stylesheet file or add undo history.

In Code, the Character menu MUST expose the global Code character definitions
and syntax names referenced by accepted, retained highlighting results for the
current buffer. Inspecting or opening this menu MUST NOT parse unvisited text,
wait for a provider, create definitions, or change source. Choosing a defined
style opens that definition in the global Styles editor instead of assigning it
to the selection. A referenced name without a definition appears as an explicit
**Define <name>…** action; only choosing that action creates an empty inherited
definition and opens it for editing. Its appearance stays at the Code default
until the user supplies declarations. Merely resolving a missing name still
never creates a definition. The menu uses the actual case-sensitive syntax name
and preserves normal menu tracking while highlighting changes asynchronously.

Character > Edit Styles… opens the current character style in the global Code
sheet; Paragraph > Edit Styles… opens the current paragraph style in that sheet.
Use the corresponding base style when the selection has no single current style.
Format > Document Style > Edit Styles… opens the global base document definition.
When opened from a Code view, the Styles editor follows subsequent caret and
selection movement in that view using the policy below while retaining global
sheet ownership. These are global style edits shared live by Code buffers, with
the style editor's own undo history. Selection formatting and manually assigning
named styles to Code text remain unavailable.

Syntax name references are disposable, unlike authored stable-ID assignments.
Adding, renaming, or removing a definition invalidates their resolution; names
left unresolved use the default appearance. Definition children still follow
the normal acyclic inheritance and deletion/reparenting rules. Syntax output
does not prevent deleting an otherwise removable definition.
Persist suppression of deleted or renamed built-in definitions in the saved
sheet, so defaults do not silently reappear on reload; restoring defaults is an
explicit style-editor action. User edits and missing-name resolution survive restart.

Named syntax styles support the normalized character properties, including
font, size, weight, and slant. Paint-only differences repaint without parsing,
querying, reshaping, or rewrapping. Font/metrics-affecting differences invalidate
affected shaping and downstream layout by the existing style-effect rules;
paragraph line-spacing changes invalidate paragraph layout. Generation changes
mark broad dependencies lazily, with exact replacement for visible content.
This applies both when a user edits a definition and when asynchronous syntax
coverage introduces or removes a run using a different font, size, weight, or
slant. Unrelated paint-only coverage MUST NOT discard valid height estimates
merely because another syntax definition has font properties. Preserve the
viewport's text anchor when styled row heights change and reject layout work
whose already-published style dependencies have become stale.

Typing in Code MUST NOT recenter or otherwise move the vertical viewport just
because the source revision invalidates its prior layout. Preserve the visible
caret row's screen baseline through local edits and asynchronous syntax metric
changes. Normal line breaks, wrapping, and explicit cursor motion can advance
the caret; reveal it with the minimum necessary scroll. Committing input-method
marked text preserves the baseline shown by its composition layout. When a new
styled row would be clipped, shift only enough to make its typographic and ink bounds
visible. If a row is taller than the viewport, prioritize a visible baseline
without alternating between impossible top and bottom reveals. A view whose
caret is offscreen keeps its viewport text anchor instead. These policies remain
portable and use exact local geometry, without laying out the gap to an offscreen
caret or measuring the whole document.

In particular, an already-visible editing line MUST NOT jump to the bottom of
the window after a keystroke or syntax completion. Preserve its screen baseline
through the viewport anchor while it remains fully visible. If surrounding
layout geometry is unchanged, the numeric viewport origin also stays unchanged;
changed heights above the anchor may require correcting that origin to keep the
same text on screen. A partial layout region is not a document edge. Missing
cached rows below or above the preserved anchor MUST NOT clamp the requested viewport
to that region. Materialize the missing local coverage for the anchored
viewport, then apply only the visibility adjustment actually required by the
current caret row. This applies with wrapping on or off, across long-line layout
chunks, and with either syntax provider or no highlighting.

Rehighlighting MUST retain the previously accepted appearance of surviving text
while replacement results are pending. Rebase this temporary presentation with
the exact committed text maps, including discontiguous edits and history, rather
than clearing the entire overlay or shifting positions by a single changed hull.
Inserted text may use the default appearance until styled. Retained runs are
presentation only; they do not establish current provider coverage or suppress
fresh analysis. Replace them only within newly accepted coverage, including an
accepted empty result that clears obsolete highlighting. Reject stale results
and clear incompatible language/provider configurations. Bound retained spans
and snapshots under the syntax cache budget; newly exposed uncached content may
display defaults during fast scrolling. An ordinary edit must not flash existing
highlighted text through an unstyled intermediate frame.

The vertical scrollbar need not predict the final styled height of unvisited
content. Unknown heights MAY use estimates from the default Code font and
paragraph spacing, refined as nearby content is styled and laid out. Obtaining
an exact scrollbar MUST NOT force whole-document highlighting or layout.
Syntax styles cannot conceal text or change the logical editing boundary space.
Provider regex/node boundaries remain internal until mapped to legal display
ranges; they do not create new grapheme or shaping caret stops.

#### Language detection and Code configuration

Vim syntax uses the shared pinned snapshot in `assets/vim/runtime/syntax`.
There is no user-configurable syntax directory or associated path/chooser/
restore control. Native frontends
resolve its installed resource path and pass it to the core; the Rust core has
no platform-specific default directory. macOS packages it at
`Contents/Resources/vim/runtime/syntax`. Windows build and publish outputs place
the same snapshot at `Resources/vim/runtime/syntax` beside the executable,
resolved from `AppContext.BaseDirectory`, independently of the working directory.
Both verify source and packaged inventories/hashes and replace only the owned
`Resources/vim` subtree, including nested helpers, license and provenance files.
Validation is documented in `docs/windows-vim-runtime-followup.md`.

The retired `code.vimSyntaxDirectory` setting is ignored and removed on the
next settings write, preserving unrelated fields. Application resource paths
are never persisted. Includes stay within the bundled syntax root.
An absent or unreadable bundled directory makes that
Vim source unavailable, but never prevents opening, editing, or using bundled
Tree-sitter. The main Settings window has no Code category. Syntax load and
compiler diagnostics remain available through the existing document/core
diagnostic mechanisms. The global Code stylesheet is edited through the
ordinary modeless Styles inspector opened from a Code view by F8 or the menu.
That editor has an explicit global target, distinct from a document target;
valid edits persist and apply live. Its undo grouping belongs to the global
style-editing session, not to any document. Document-specific
style/default-saving and content-formatting actions are disabled in Code.
Ordered filename associations remain supported in `config.json` even though
Settings does not provide an editor for them.

Language detection is portable declarative policy inspired by Vim filename and
file-content detection, not execution of filetype autocommands. In Automatic
language selection, precedence is:

1. a supported `ft=` or `filetype=` Vim modeline;
2. a user filename association, then bundled basename/extension/glob rules;
3. a shebang when filename detection is unresolved; and
4. registered bounded content signatures, then unknown language.

An explicit buffer language override, including None, precedes all automatic
rules. Recognize modeline forms such as `vim: ft=rust` and
`vim: set filetype=rust:` in the first and last five logical source lines.
Inspect at most 64 KiB total of decoded content for all load-time detection,
using bounded head/tail reads and indexes rather than traversing an enormous
line. A marker truncated by the limit does not match. Shebang detection handles
direct interpreters and `/usr/bin/env`, including supported `-S`/assignment
forms. Ambiguous extensions such as `.h` and `.m` use registered bounded
disambiguators and documented deterministic defaults, with user associations
taking precedence. Conflicting modelines use the last valid assignment in
document order. An unknown explicit marker preserves that language selection
as unavailable instead of silently choosing a conflicting filename result.

Markers are data: read only supported language selectors, never expressions,
paths, shell commands, arbitrary Vim options, `source`, or autocommands. General
modeline execution is not enabled. The initial profile does not interpret
`syn=`/`syntax=` as a second override. The bundled syntax directory supplies
highlighting programs; it does not imply executing sibling `filetype.vim`,
`scripts.vim`, `ftdetect`, or Vim9 helpers. Bundled and user filename tables and
package language aliases define the supported detection profile explicitly.

Capture detection inputs and the selected language/reason on load or explicit
reload/redetection; first entry into Code also detects when no selection exists.
Ordinary text edits, scrolling, and syntax completion MUST NOT rescan markers
or silently switch languages. Filename changes may rerun Automatic detection;
explicit overrides remain. Detection cannot block first display on a full-file
syntax parse or syntax-program load.

#### Provider contract, selection, and coverage

Portable implementations live under `src/core/document/syntax`; Vim command
interpretation stays in `command`. A `SyntaxProvider` consumes an immutable
`SyntaxInputSnapshot`, revision-bound change maps, language-region identity,
requested ranges, configuration generations, and a budget. It returns immutable
named runs, coverage, diagnostics, and opaque reusable analysis state. Backends
declare context needs, coordinate limits, execution/threading requirements,
and supported capabilities. The interface MUST accommodate both a checkpointed
Vim scanner and Tree-sitter's whole-region tree plus regional queries.

Syntax input is normalized decoded UTF-8 with physical source-line boundaries,
before format hiding and paragraph flow. Source-to-syntax and syntax-to-display
coordinates have explicit snapshot identities and provenance. Read through a
persistent tree/chunk facade; do not flatten a document or whole long line on
the highlighting path. A source patch in UTF-16 or a CRLF source is not itself
a valid parser-coordinate edit. Full decoding/projection fallbacks on ordinary
Code newline edits violate the large-document requirements below.

The initial preferred Tree-sitter language families are **C, C++, Rust, Swift,
Objective-C, C#, JavaScript, TypeScript, and Python**. Bundle compatible grammar
artifacts, scanners, highlight queries, and required query dependencies for all
nine. The package IDs include `c`, `cpp`, `rust`, `swift`, `objc`, `c_sharp`,
`javascript`, `typescript`, and `python`; TypeScript also bundles the separate
`tsx` grammar. JavaScript includes JSX queries. Detection aliases include Vim
`cs`, `javascriptreact`, and `typescriptreact` without confusing filename,
language-family, grammar, and query identities.

Vim script uses the bundled native Vim syntax program. For other selected
languages, prefer a registered compatible Tree-sitter package.
If there is no implementation, it cannot load, required queries are unsupported,
or current coverage is unavailable/exceeds policy, use the corresponding Vim
syntax if available; otherwise use default Code styling. Initial Tree-sitter
parsing may use ready Vim output while it completes. Both backends and fallback
are budgeted. Do not run both across the entire document simply to blend their
colors. Additional grammar/query or Vim packages may be loaded separately
without changing the editor's core command or rendering code.

Coverage is explicitly `Exact`, `Provisional`, or `Missing`. Exact means
complete/current for the chosen provider and supported configuration, not
well-formed source: a completed Tree-sitter parse may contain error nodes.
Provisional uses current text with declared heuristic context. Missing uses
ready fallback coverage or default styling. A provisional checkpoint or stale
result cannot establish exact coverage.

Install coverage and runs atomically, validating document, input, format,
language region, provider, grammar/rules, queries, and configuration identities.
A completed primary query with no captures replaces old fallback colors with
default styling. Fallback fills unavailable coverage, never gaps between
captures or names absent from the Code stylesheet. Stale top-level packages are
discarded; subresults are reusable only after all content and dependency
identities are revalidated. Highlight completion changes neither source nor
text-projection revision and creates no document undo item.

#### Vim syntax compatibility and incremental execution

Separate loading a `.vim` program from executing its rules. A native loader
supports a documented, versioned declaration/setup subset with file/line
diagnostics. An import-time compiler may evaluate broader vetted runtime files
through a pinned Vim and export a structured program. Preserve evaluated rule
order, case/keyword environment, includes/clusters, and synchronization settings;
human-readable `:syntax list` output is not the serialization contract. Cache
programs by compiler version, transitive input hashes, configuration, and bounded
buffer-sniffing inputs. Compilation/includes are cancellable and memory-bounded.
The installed MacVim 9.1.1887 files are compatibility fixtures; normative Vim
behavior remains 9.2. Neither path enables arbitrary Vimscript during editing.

Native syntax profile 3 accepts bounded setup expressions, lists/dictionaries,
loops, syntax-generating helpers and wrappers, confined runtime includes,
case-insensitive group/option names, configurable keyword characters, and the
four Vim regex magic modes. Group names retain their first spelling. Final
keyword settings apply to all rules, including external-delimiter templates.
Setup can inspect a bounded 16 KiB filename as well as the input prefix; renaming
invalidates setup and its generation. Unused helper bodies remain data and do
not execute during editing. Syntax folding, spelling, and concealment flags are
presentation metadata: Code keeps all source text visible. Concrete highlight
attributes establish group/link precedence; the Code stylesheet supplies visual
properties. Compatibility failures remain atomic diagnostics. Pinned Makefile
fixtures require native group comparisons and bounded large-document repair and
cache-reuse tests alongside the existing syntax gates.


The supported program includes keywords, ordered match/region start/skip/end
rules, containment/clusters, nextgroup/whitespace behavior, transparency,
end-control flags, offsets, group links, and sync/display hints. Pin supported
regex constructs in compatibility tests. Vim syntax patterns are separate from
Viem Regex v2 search patterns: preserve Vim semantics where supported and reject
unsupported atoms rather than silently translating them with different meaning.
A fast regular matcher and a budgeted compatibility VM may share one compiled
pattern model. External delimiter captures must survive in region state.
Numeric absolute-line and byte-column assertions use the immutable source's
hard-line index, including comparison forms. Absolute-line predicates invalidate
from the beginning after edits; byte-column predicates use whole-line
dependencies. Neither requires scanning preceding source text. Editor-state-
dependent cursor/mark/Visual and virtual-column assertions remain unsupported.

Retain structurally shared complete restart states densely near requested
regions and sparsely elsewhere, with bounded span caches. States include region
stacks, delimiter captures, pending transitions, and inspected context; hash
equality alone is not proof of equality. Checkpoints use stable input identities
and local boundaries. Dirty suffixes lose exact authority through lazy interval
or generation updates, without visiting every later checkpoint or span.

After an edit, restart before the earliest affected dependency and process until
complete state and unchanged context converge at a valid boundary. Reuse then
extends only to the next independently dirty region. Track reads from successful
and failed pattern attempts, future end/skip searches, and lookaround, or use
proven conservative bounds. `sync linebreaks` is a hint, not a universal proof
that earlier highlighting is unaffected.

Bounded `minlines`/`maxlines`, sync patterns, and C-comment recovery may provide
provisional viewport colors. Exact lineage comes from the beginning or a
validated exact checkpoint. `fromstart` does not authorize foreground scanning.
Skip offscreen `display` rules only under the supported semantics that preserve
continuing state. A private continuation suspended inside a regex is distinct
from a reusable safe line checkpoint. Long-line chunks are not artificial EOLs.
Exhaustion leaves unknown/provisional coverage; never clear state and pretend
the following text is exactly outside a region.

#### Tree-sitter analysis and query compatibility

Keep one mutable worker-owned parser session per active language region and
immutable completed tree instances for readers. Concurrent readers/workers use
separate `Tree::clone`/`ts_tree_copy` handles sharing underlying tree structure;
do not access one native tree handle concurrently. Feed rope chunks directly via
the parser input callback. Use a private copy of a completed tree, apply exact
ordered `InputEdit` records, then incrementally parse the new snapshot with that
edited tree. Published trees and raw node positions are never mutated in place
or retained as Viem persistent anchors. Share analysis across views.

An initial host parse generally covers the whole language region and runs off
the UI/coordinator path; viewport highlighting queries are separate jobs.
Included ranges represent declared embedded-language regions, not arbitrary
viewport slices that omit necessary language context. A cancelled parse does
not publish a partial new tree. Resume only with its frozen input, old tree,
grammar, and included ranges; reset before superseding that continuation.
Coalesce pending edits. A completed stale tree may seed later parsing only after
the exact intervening edits are applied, never as current display results.

Do not require a contiguous-source convenience highlighter that reparses from
scratch; maintain parser/tree/query state explicitly. Invalidation combines
actual text edits, structural `changed_ranges`, and query dependencies. Text
predicates, formerly failed matches, ancestor/sibling patterns, local-variable
scopes, and injections can change results outside structurally changed nodes.
Maintain conservative dependency indexes and lazy dirty regions; same-shape
identifier edits must still reevaluate applicable text predicates.

Packages declare an upstream or versioned Neovim query profile. Compile against
the exact grammar and validate every required capture convention, predicate,
directive, inheritance/extension, and dependency before activating the package.
The Neovim profile includes named captures, priority metadata, supported
equality/membership/text-match predicates, offsets, and injection declarations.
Implement host handlers in portable Rust. Neovim `match?` uses its specified Vim
regex semantics; `lua-match?` is a distinct optional Lua-pattern capability.
Unknown/custom Lua handlers and unsupported directives produce diagnostics;
they are not silently true or ignored. Optional locals queries require explicit
scope analysis and invalidation; a highlights-only profile cannot claim them.

Query only requested regions plus required context, evaluate predicates against
their complete inputs, then clip output to display coverage. Range filters do
not prove bounded query work. Budget traversal, predicates, captured text,
pending matches, and output spans separately; match-limit exhaustion is
incomplete coverage. Query-generation changes preserve valid parse trees;
stylesheet/theme changes preserve trees, queries, and provider name runs.

Embedded languages form an explicitly owned parent/child region graph. Each
child records parent analysis/configuration, selected language, included ranges,
and coordinate mapping, and may select Tree-sitter or Vim independently.
Changing/removing an injection invalidates its child even if its text survives.
Prioritize visible children and retain valid offscreen metadata. Combined
injections couple their ranges as one analysis/dependency group. Bound discovery
work, recursion, child count, total input, and retained trees. Child precedence
over parent syntax is explicit and cannot alter authored text or formatting.

#### Platform packages, scheduling, and resource limits

Core owns analysis policy and state; platform services locate packages, load
native libraries or optional Wasm artifacts, and supply appropriate executors.
Validate OS/architecture, runtime ABI, grammar/query revisions, and required
capabilities. Keep package owners alive while parsers, trees, or queries refer
to them. The frontend ABI exchanges opaque retained handles and bounded batches,
never exposed tree nodes, mutable parser objects, or platform colors. Additional
platform providers implement the same snapshot/coverage contract.

Opening/displaying text, committing an edit, drawing, and moving the viewport
MUST NOT wait for or execute a syntax compiler, parser, regex, or query job.
Small bounded filename/marker detection and publication are separate from
provider execution. Essential input/layout work has priority over syntax.
At most one job runs per mutable provider session, with one coalesced replacement
request; a bounded pool and fair per-buffer quotas prevent injection/fallback
work from starving other buffers. Jobs return immutable packages for checked
coordinator installation, with no provider calls while holding core locks.

Use deterministic work budgets plus cooperative deadline checks, including
inside Vim matching and query predicates. Account for grammar compilation,
stacks/captures, continuations, retained trees/snapshots, cached spans, queued
results, and injection state under explicit configurable byte budgets. Repeated
failure is keyed by relevant content, entry state, dependencies, program, and
budget profile; unrelated edits or repaints must not retry the same capped work.
Eviction does not require reparsing before the next interactive frame.

Exact repair may legitimately reach EOF after a delimiter edit. Time slicing
and fallback bound responsiveness, not arbitrary total repair work. Native
Tree-sitter scanners are cooperatively cancellable only after their callbacks
return. Do not promise hard in-process preemption or strict allocator limits
for arbitrary native grammars; providers requiring enforceable termination
use replaceable process workers or another execution mechanism that enforces
those limits. Worker isolation must also permit continued editing on failure.

Backend coordinate limits are checked before conversion/calls. In particular,
Tree-sitter's 32-bit byte/point domain cannot represent arbitrary large inputs.
Oversized inputs use Vim/default styling or independently valid mapped language
regions; never truncate coordinates or split arbitrary code into false parses.

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
- The `textwidth` reflow setting below controls explicit source edits by `gq`
  and `gw`; it does not constrain soft wrapping or resize layout.
- `wrap` controls whether soft wrapping is active. Wrapped text always uses
  Unicode-appropriate word/line-break opportunities at the usable text width;
  there is no separate `linebreak` option or character-wrapping mode.
- If no legal word boundary fits, the complete unbreakable segment overflows
  to the right through its next legal break or hard-line end. Neither grapheme
  boundaries nor internal layout-cache boundaries may split that segment.
- Code continuation rows use the configured wrapped-line margin specified
  under Code wrapped-line indentation. This changes the row's content
  box while retaining the same Unicode line-break policy.
- Changing the window size, side insets, gutter width, zoom, or any other value
  that changes usable text width MUST reflow the text for that view.
- Resize reflow MUST update visible rows synchronously for the next frame. It
  MUST NOT reshape unchanged text merely because the width changed; it should
  reuse width-independent shaped fragments and recompute line breaks.
- Off-screen wrap results and height aggregates may be recomputed lazily after
  a resize. Scrollbar extent may temporarily use estimates, but visible text,
  hit testing, selection, and the caret must always use exact layout.
- Live resize uses previously visible hard lines as a work estimate when the
  document revision is unchanged, then extends reflow until the new viewport
  has exact coverage. Reset height estimates must not turn wrapped paragraphs
  into several screens of synchronous offscreen work. Tests cover bounded work
  in large documents, fresh-layout equivalence, and invalidation.
- During reflow, preserve a stable text anchor at the top of the viewport and
  keep the active caret visible. Do not preserve a stale numeric scroll offset
  if doing so would make the user's text jump unpredictably.
- With wrapping off, hard lines do not reflow when the window narrows. The view
  scrolls horizontally and otherwise behaves like gVim.

### Meaning of "line" and per-view line mode

Each view has a portable line-mode policy, initially **Physical Source** for
Code and **Visual** for every other format. Clicking the status-bar location
toggles Visual (an eye icon) and Physical Source (a file icon). These are
original vector icons. The mode is independent of `wrap` and is not persisted
in source. RTF does not expose Physical Source mode; changing a buffer to RTF
returns any physical-mode views to Visual.

- Visual mode counts the exact displayed rows, including soft wraps. Without
  wrapping, these are formatted hard lines. Physical Source mode counts the
  shared line-ending projection's source-line tokens, including invisible
  syntax and comments in text formats.
- Standalone and operator-pending `j`/`k`, line start/end motions, doubled line
  operators, `C`/`D`, line-oriented insertion, and Visual Line use the selected
  mode. Counts, registers, replay, dot repeat, and undo retain that policy.
- The `gq`/`gw` source-reflow operators are an explicit exception: their
  implicit line motions and doubled forms count complete hard lines.
  Explicit visual motions and existing Visual selections retain their domains,
  then reflow expands their checked extents to the intersected hard lines.
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
- Visual Character, Visual Line, and Visual Block modes;
- Select Character, Select Line, and Select Block modes;
- native Selection, distinct from Vim Select and Visual; and
- Command-line mode for `:`, `/`, and `?` input.

Operator-pending, prefix-pending, register-pending, and single-Normal-command
from Insert mode are explicit transient states in the core command machine.
Escape cancels a transient state without leaving a partially applied edit.

The frontend renders mode from core state:

- Normal and Visual modes: a filled character block around the associated
  grapheme, shaping cluster, or atomic object under the cursor;
- Insert, Select, and native Selection: a thin vertical insertion caret at the caret stop;
- Replace mode: an underline or low horizontal bar under the cluster that will
  be replaced; and
- Command-line mode: a thin vertical insertion caret in the command line.

The core publishes what the caret occupies; frontends resolve only its
geometry. A caret target is either a **character cell** covering exactly one
grapheme of hard-line content, or an **insertion boundary** between graphemes
carrying the boundary affinity that selects its visual row. Modes that address
characters (Normal, Visual, Replace) publish a cell wherever the cursor has a
character and a boundary where it has none, such as an empty line or the end of
the document. Insert and command-line modes always publish a boundary. A
frontend MUST NOT re-derive this choice from the mode, the cursor offset, and
the affinity together; those inputs do not say by themselves whether the cursor
names a character or a gap. Affinity MUST NOT be reachable from a cell.

These mode-to-appearance rules, the custom block and underline rendering rules,
focus behavior, and caret-color policy are portable requirements. A future
Windows frontend follows them using Windows-native facilities where suitable.
The decision to use `NSTextInsertionIndicator` for a vertical caret is specific
to the macOS frontend and is not part of the core or Windows contract.

The block caret is custom rendered; do not attempt to stretch a platform's thin
insertion indicator into a block. Its logical extent is the published character
cell. Boundary affinity chooses which visual row an insertion point occupies at
a soft wrap; it never chooses which character a block covers. Selecting the
block's item by affinity draws it one grapheme early after `$` and `<End>`,
which leave an upstream boundary affinity on the last character of the line.
Its visual extent uses
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

An active caret requires the focused text surface, active window, and active
application. When any of these is inactive, its caret is a nonblinking hollow
outline at 75% opacity, retaining the current mode's block, thin insertion, or
underline geometry. An active caret blinks according to
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
  Local marks and the remembered Visual endpoints `<` and `>` support exact
  and linewise jumps. `:delmarks` removes named local or Visual marks, including
  lowercase ranges such as `a-f`; `:delmarks!` clears lowercase local marks.

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
Search/replace changes are reverse-projected like other edits. Search uses
the Viem Regex v2 dialect below rather than Vim's full regular-expression
language. Search history belongs in core state.

#### Search highlighting and incremental preview

`hlsearch` (`hls`) and `incsearch` (`is`) are buffer-shared Boolean options,
both false by default. They support the normal `:set`/`:setlocal` Boolean
grammar and may be initialized by `startup.viem`. Every view of the buffer
observes accepted search-pattern and option changes. `hlsearch` highlights
matches of the accepted pattern; `incsearch` previews the pattern being typed
in `/` and `?`, including counted, Visual, and operator-pending searches.
Supported `:substitute` prompts preview matching text within their addressed
hard-line range, respecting escaped delimiters and case flags. They do not
preview replacement text or apply replacements before Enter.

Incremental preview reveals the candidate match without changing the
authoritative cursor, selection, registers, search history, or pending
operator's origin. Ctrl-G and Ctrl-T choose the next and previous preview
match; changing the prompt resets that navigation. Counts retain their normal
search/operator meaning. Enter executes the accepted search or command using
the chosen match where applicable. Escape restores the original cursor,
selection, and viewport without accepting the pattern. Empty, invalid,
unmatched, or resource-limited previews do not commit editing state.

`:nohlsearch` and `:noh` temporarily suppress accepted-pattern highlighting
without changing `hlsearch` or the saved pattern. A subsequent search or an
explicit `hlsearch` setting change clears that suppression; querying the
option does not. Incremental preview remains available while accepted-pattern
highlighting is suppressed.

All search presentation uses the built-in internal Character style
**Incremental match**. Its default declaration is a translucent yellow
background only. The Style Editor exposes its declarations in the final
**Internal** group of its Style picker, and normal user style-default
persistence saves customizations. Style-application menus and
assignment APIs MUST exclude/reject it; it cannot become authored content or
be renamed, removed, or reparented. Explicit declarations overlay the complete
existing paragraph, automatic/syntax, named Character, and direct-formatting
cascade. Unspecified properties retain their underlying values; explicit
font/metric properties participate in shaping and layout invalidation.

Highlight coverage may expand to containing graphemes and indivisible shaping
clusters for display. Paint normalization extends only the internal style's
explicit paint properties across a cluster, preserving unrelated underlying
formatting. Logical match ranges and editing endpoints remain unchanged.
Highlighting and preview MUST NOT change document source, dirty state, undo
history, or saved style assignments.

The portable core owns matching, preview, range expansion, and invalidation;
frontends schedule cooperative polls and draw the resulting ordinary paint
runs. Work and results are bound to an exact document/projection revision,
pattern, effective options, viewport, and style configuration. A query, edit,
viewport, or relevant style change retires stale work or layout results.
Highlight scanning yields after at most 8,192 input steps per poll and also
checks a 100,000-transition work quota at complete byte transitions. It preserves
regex state across polls and retains at most 16,384 matches intersecting the
viewport plus 4,096 bytes of overscan on each side. It may scan preceding text
or a match's tail to preserve multiline assertions and greedy-match semantics;
the default regex program and total-work limits still apply. Incremental
preview has a separate 1,000,000-work-unit limit. Resource exhaustion stops the
affected presentation query without mutating the document. Ordinary edits MUST
NOT trigger a
synchronous full-document highlight rescan. Tests MUST cover stale work,
paint-only shaping reuse, style invalidation, and bounded large-document work.

#### Viem Regex v2

Viem Regex v2 is the sole pattern language for `/`, `?`, operator-pending
searches, `:substitute`, `:sort`, and any later command documented as accepting
a search pattern. It is a stable product interface, not an alias for whatever
syntax a particular regex library version happens to accept. The implementation may use
the Rust `regex` crate or another engine only when it produces the behavior
defined here. Version 2 adds directional word-boundary assertions to version 1;
previously accepted patterns and replacements retain their meaning. Version 2
is the sole supported dialect.

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
- Unicode word-boundary assertions `\b` and `\B`, and directional
  word-boundary assertions `\<` and `\>`;
- `^` and `$` as zero-width formatted hard-line start and end assertions at
  every pattern position, and `\A` and `\z` as logical-document start and end;
- escapes `\t`, `\r`, `\n`, `\xNN`, and `\u{scalar-value}`; and
- inline or scoped `i`, `s`, and `x` flags, such as `(?i)word`, `(?-i:Word)`,
  `(?s:.)`, and `(?x: a \s+ phrase )`. `s` permits `.` to match U+000A and
  `x` ignores unescaped pattern whitespace, including inside a class, and
  treats an unescaped `#` outside a class through the next pattern U+000A or
  pattern end as a comment.

No other inline flag is part of version 2. In particular, `m` is unnecessary
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
- `~` as the previous substitute string. In Viem Regex v2, `~` is literal.

Some accepted spellings intentionally differ from Vim and therefore require
specific compatibility tests and documentation:

- `\b` is a Unicode word boundary, not a Backspace character;
- `\<` and `\>` assert the start and end of a Unicode regex word using the
  same word class as `\w`, `\b`, and `\B`. Vim defines these assertions
  through its configurable keyword class; Viem does not use `iskeyword` or
  word-motion tailoring for them. Any future keyword-tailoring option MUST
  preserve these regex meanings;
- `\A` is logical-document start, not Vim's nonalphabetic class;
- `\w`, `\d`, and `\s` and their negations are Unicode-aware rather than
  Vim's ASCII- or option-specific classes; and
- `^` and `$` are hard-line assertions wherever they occur, rather than Vim's
  context-sensitive magic tokens.

A compatibility validator must recognize the complete unsupported families
before engine compilation. It must return `UnsupportedRegexAtom` naming the
first offending atom even when the underlying engine would accept that
spelling with a different meaning. A substring blacklist is insufficient.
`\b{start}`, `\b{end}`, `\b{start-half}`, `\b{end-half}`, and other
`\b{...}` or `\B{...}` variants remain unsupported. `\<` and `\>` are the
only supported directional word-boundary spellings; neither has a negated
form. Inside a bracketed class, `[\<]` and `[\>]` are malformed patterns and
return `InvalidRegex`.

The Unicode regex word class is the union of `Alphabetic`, `Mark`,
`Decimal_Number` (`Nd`), `Connector_Punctuation` (`Pc`), and `Join_Control`.
`\<` holds when the following scalar belongs to this class and the preceding
scalar does not; `\>` holds when the preceding scalar belongs and the
following scalar does not. Outside the logical document is non-word context.
Both assertions are zero-width. Combining marks and join controls participate
in this scalar classification; the grapheme-safety rules below still govern
navigation and edits. These semantics do not claim equivalence to Vim's
keyword or script-specific classification.

Viem's keyword extraction for `*`, `#`, `C-r C-w`, and manual completion is a
separate, grapheme-based policy: a grapheme belongs when its first scalar is
alphanumeric or `_`. For example, superscript two (`²`) belongs to that keyword
class but not the regex word class, while undertie (U+203F) belongs to the regex
word class but not the keyword class. `*` and `#`, including operator-pending
forms, retain their generated `\b{escaped keyword}\b` as the last-search
pattern, available through the `/` register. They MUST NOT be rewritten as
`\<{escaped keyword}\>`: keyword extraction does not guarantee regex-word
scalars at both edges, so these patterns are not universally equivalent. `g*`
and `g#` retain the escaped literal pattern without boundary assertions.

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

The pattern and replacement are different languages. Viem Regex v2 defines
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
spellings, and every unlisted backslash escape are unsupported in version 2.
They return `UnsupportedReplacementAtom`; an unknown escape never silently
drops its backslash.

#### Compilation, execution, and caching

Matching must be linear in the searched input for a fixed compiled pattern,
with an `O(m * n)` worst-case bound where `m` is compiled-pattern size and `n`
is searched-input length; no version-2 feature may add input-dependent
backtracking. The core enforces configurable pattern-length,
compiled-program-size, capture-count, and search-work limits. Resource-limit
failure and user cancellation are non-destructive structured results.

Every compiled pattern carries the explicit dialect version. A compiled-pattern
cache key includes that version, exact pattern text, effective compile flags,
and resource limits. The current identity uses version 2.
Any future persisted pattern metadata must also identify its dialect version.
Match results, incremental-search state, and search decorations are bound to an
exact projection snapshot and cannot be reused after a relevant edit without a
validated change map or rescan. Searching may
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

Required operators are `d`, `c`, `y`, `>`, `<`, `=`, `g~`, `gu`, `gU`, `gq`,
and `gw`. The hard-line reflow operators `gq` and `gw` have their own
subsection below.
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

#### Literal-next input

In Insert, Replace, and command-line input, Ctrl-V and Ctrl-Q quote the next
input using Vim's literal-next grammar. Normal and Visual mode retain their
Visual Block entry/toggle behavior. Pending literal input is portable command
state; frontends route it before native editing shortcuts and expose its state
through the view presentation.

Quoted Tab inserts U+0009 regardless of `expandtab`, `softtabstop`, or `smarttab`.
Quoted control characters insert their values without executing editing
commands: Escape stays in the current input mode, and Return inserts literal
CR rather than a structural break. With Mac line endings, quoted CR inserts
literal LF content, following Vim. Quoted LF and NUL insert U+0000. Viem stores
that actual scalar in both documents and command lines instead of copying
Vim's internal LF-as-NUL representation. Supported named special keys insert
their key notation, such as `<Left>`, `<BS>`, and `<Del>`; raw Ctrl-H and DEL
remain distinct control characters. In Replace mode a named key's notation
consumes only one original grapheme.

Numeric entry accepts up to three decimal digits, `o`/`O` plus three octal
digits, `x`/`X` plus two hexadecimal digits, `u` plus four hexadecimal digits,
or `U` plus eight hexadecimal digits. A non-digit after entered digits commits
the value and is handled normally; with no digits after a radix prefix, the
next input is quoted. Byte-valued decimal and octal entry clamp values above
255. Unicode entry rejects surrogate values and values above U+10FFFF rather
than creating invalid UTF-8.

Quoted input bypasses automatic indentation, comment continuation, smart
quotes, and HTML typing assistance, while retaining normal source projection
verification and atomic failure for unrepresentable content. It belongs to the
current Insert/Replace undo group; counts, dot repeat, macro replay, and Replace
Backspace retain the literal intent rather than reapplying typing assistance.

The `gq` and `gw` operators are specified under **Hard-line reflow** below.
They are implemented in `src/core/document/reflow.rs` (portable formatter)
and `src/core/command/reflow.rs` (operator glue) and are supported commands.

Marks and searches are composable operator motions. `` `{a-z} `` uses the
mark's exact position as an exclusive characterwise motion, while `'{a-z}`
uses the marked hard line's first nonblank position and is linewise; counts do
not alter a named-mark destination. `/pattern`, `?pattern`, `n`, `N`, `*`, `#`,
`g*`, and `g#` are exclusive search motions and multiply operator and motion
counts where Vim does. An operator search records its jump only after the
operator succeeds. Escape from its command line cancels the complete pending
operator without changing text, registers, search state, or the jumplist.

### Indentation and whitespace presentation

#### Editing columns and automatic indentation

Literal Text, Code, Markdown Source and HTML Source use Neovim-style logical
indentation columns. The defaults are `autoindent`, `tabstop=2`, `shiftwidth=2`,
`softtabstop=2`, `expandtab`, and `smarttab`. WYSIWYG structural breaks keep
their format-aware behavior; a generic indentation operation must not create
source syntax or bypass reverse-edit verification.

Settings > Editing > Indentation and tabs exposes automatic indentation, whether Tab
inserts spaces or hard tabs, hard-tab width, indentation width, soft-tab width,
smart Tab in leading whitespace, and separate comment-continuation switches
for Enter and `o`/`O`. Hard-tab width accepts 1–1024 columns; indentation width
accepts 0–1024, with 0 inheriting hard-tab width; soft-tab width accepts
−1–1024, with −1 inheriting indentation width and 0 disabling soft-tab editing
outside the leading-whitespace smart Tab rule.
These are buffer defaults. Their long Vim names and `ai`, `ts`, `sw`, `sts`,
`et`, and `sta` aliases are exposed through `:set`/`:setlocal`, including queries,
boolean toggles, reset, and `<` to resume inheritance. The comment switches
are `continuecommentsonenter` and `continuecommentsonopenline`. Overrides are
per field and shared by every view of a buffer; changing one does not freeze
the other inherited defaults. Option changes never enter document history.

Tab advances to the next soft-tab stop, or an indentation stop in leading
whitespace with smart Tab enabled. Space insertion uses the required number
of spaces; hard-tab insertion uses tabs and spaces to reach the same logical
column. A tab advances to a stop rather than adding a constant width. Insert
Backspace removes whitespace to the preceding applicable stop, splitting a
hard tab into retained spaces when necessary. Insert Ctrl-T/Ctrl-D move to the
next/previous indentation stop. `0` followed by Ctrl-D removes the leading
indentation and the typed zero; `^` followed by Ctrl-D does the same but restores
that indentation on the next Enter. The `>`/`<` operators shift by indentation
width. The deterministic `=` provider copies
the preceding nonblank indentation; language indentation engines, `cindent`,
`indentexpr`, and implicit external formatting are not part of this feature.
Counts, operator motions, Visual forms, dot repeat and undo grouping retain
their command-specific semantics. Replace-mode Tab is one restoration unit:
Backspace restores all characters overwritten by that Tab press. Replace-mode
Enter inserts the break and prefix without overwriting the following body;
Backspace removes its generated indentation by the applicable soft stops,
then restores the original split including whitespace consumed at that boundary.

Enter and `o`/`O` copy the current hard line's indentation with autoindent
enabled, reconstructing its logical width using the configured spaces/tabs
policy. Splitting a line uses indentation before the insertion boundary and
consumes leading whitespace in the moved suffix. The established Visual-mode
`O` split at an interior soft-wrap boundary can leave its insertion caret in
the preceding fragment's body; that position receives no generated indentation
or comment leader. Indentation is inserted only at a resulting hard-line start.
Unchanged automatically generated indentation is removed when leaving an
otherwise empty inserted line. Authored blank lines and manually adjusted
indentation are retained. Paste and input-method commit insert their supplied
text without replaying Enter or Tab assistance for its contents.

Code comment continuation is enabled by default for Enter and `o`/`O`. It uses
the same language profile, full-line leader recognition and block context as
hard-line reflow. Preserve `//`, `///` and `//!` exactly, and preserve an
established starred or unstarred `/* ... */` body. A new conventional block
continues with ` * `; entering `/` immediately after that generated prefix
closes it as ` */`. After a block closer, resume the opener's indentation without
a comment leader. A leading `*` without block-comment context is ordinary text.
Only a full-line block opener establishes this profile's context; delimiters in
quoted code or after other code do not. Neither continuation nor reflow depends
on syntax-highlighting availability, asynchronous coverage, or style names.
Comment context and the `=` provider's preceding-nonblank lookup inspect at
most 512 preceding hard lines. If the required context lies outside that bound,
comment continuation falls back to ordinary indentation, and `=` uses zero.

#### Physical width of leading whitespace

Settings > Editing > Indentation and tabs has two independent width choices, **Code**
and **Other formats**, each offering **Use spaces** and **Use paragraph en**.
Code defaults to paragraph en; other formats default to spaces. Code means the
literal Code format, not Normal/Insert command mode or a rich-text Code style.

Leading whitespace is the ASCII space/tab prefix of a hard line. A soft wrap
does not start a new prefix. In paragraph-en mode, each leading space occupies
half the size of the default Paragraph font, excluding character styles, actual
space-glyph advance and tracking. Hard-tab stops use the same units. Thus an
eight-column tab stop is equivalent to eight leading spaces and eight ens;
the default two-column stop is equivalent to two. There is no independent
eight-en hard-tab override. In Use spaces mode, spaces retain their ordinary
font-dependent advances and tab intervals use the current font's space advance.
Nonleading spaces retain ordinary shaping in either mode. Logical indentation
and reflow columns remain independent of fonts, zoom and window width.

Whitespace geometry participates in full, regional and streamed long-line
layout, caret/selection geometry and exact snapshot identities. Changes to
the applicable font metrics, width basis or tabstop invalidate affected layout
and checkpoints. Large documents must retain bounded viewport layout and reuse
unchanged regions. A marker toggle must not change the resulting geometry.

#### Code wrapped-line indentation

Settings > Editing > Indentation and tabs includes **Wrapped line indent
(Code)**, persisted as `editing.whitespacePresentation.codeWrappedLineIndent`
in the existing versioned `config.json` authority. It is a whole number from
0 through 1024, with a fixed default of **4**, twice the shipped indentation
default of two. Missing values use four. Changing `shiftwidth` or `tabstop`
does not change this setting's value. Negative, fractional, malformed, null,
Boolean, and overflowing values are rejected atomically without changing the
prior settings or configuration bytes. Settings changes update existing and
future views through the live whitespace-presentation configuration.

The margin applies only when `Format::is_code()` and the view's `wrap` option
are both true. The first visual row keeps its existing content box. Every
continuation row starts at the hard line's content origin plus the measured
advance of its original leading ASCII space/tab prefix, plus the configured
number of Code whitespace units. All continuation rows use this same origin;
the indentation does not accumulate. Zero retains the original line's measured
indentation and adds no extra units. A soft wrap does not establish a new
leading-whitespace prefix.

Original prefix measurement uses the existing Code whitespace policy, including
mixed spaces and tabs. Each additional unit in Use paragraph en occupies half
the default Paragraph font size, excluding character styles and tracking. In
Use spaces, each additional unit uses the space advance of the original hard
line's first character's effective font and tracking, falling back to the
default Paragraph style for an empty line. Both measurements follow view zoom
and the applicable font metrics generation.

The continuation indentation is a paragraph-like layout margin. It synthesizes
no text, whitespace markers, caret stops, or selectable content. Actual source
whitespace stays editable and visible under the existing marker policy. Source
bytes, dirty state, document undo history, registers, clipboard, search, and
accessibility text are unchanged by the setting. Continuation hit testing,
selection, caret geometry, and vertical motion use the shifted content box.

The margin reduces continuation-row usable width and is not clamped for deeply
indented lines or narrow windows. Existing Unicode word wrapping and complete
unbreakable-segment overflow still apply when no segment fits; indentation does
not introduce syntax-specific break opportunities or character wrapping.
Full, regional, and streamed long-line layout MUST agree. Original indentation
measurements participate in bounded wrap checkpoints, so later viewport work
does not repeatedly rescan an arbitrarily long prefix. Changes to the setting,
original prefix, applicable font metrics, Code width basis, or tabstop invalidate
the affected wrap plans, checkpoints, and height geometry while reusing
unchanged width-independent shaping and unrelated hard lines. Tests cover
both width bases, mixed indentation, styled fonts, zero and maximum values,
format/wrap exclusion, geometry and source preservation, settings validation,
cache invalidation, multiple views, and bounded large-document layout.

#### Visible whitespace

Settings > Editing > **Visible whitespace** contains an enable checkbox, an
**Edit Style…** button and a
character entry for every Vim 9.2 `listchars` category: `eol`, `tab`, `space`,
`multispace`, `lead`, `leadmultispace`, `leadtab`, `trail`, `extends`, `precedes`,
`conceal`, and `nbsp`. It defaults to enabled with
`tab:>-,trail:*,extends:>,precedes:<`; every other entry is blank. Blank omits
that category. Tab/leadtab take two or three printable single-column Unicode
characters; multispace/leadmultispace take a nonempty sequence; the other
categories take one. Invalid characters and malformed entries are rejected
atomically. `leadtab` requires `tab`; otherwise leading tabs inherit `tab`.
An omitted tab category uses Vim's `^I` fallback. Pattern repetition and
leading/trailing precedence follow Vim. `nbsp` includes U+00A0 and U+202F.
The Ex `:set` parser preserves escaped spaces, including a final space filler,
and collapses doubled backslashes; `\x`, `\u`, and `\U` numeric character
escapes remain available to the `listchars` value parser.

`:set list`, `nolist`, `list!`, `listchars=…`/`lcs=…`, queries, and `<`/reset
operate on view-local overrides of the live application defaults. The marker
toggle in View > Show Invisible Characters uses that same `list` override and
is unavailable in WYSIWYG; it does not maintain a second marker preference. The
style is the application-owned character style **Visible whitespace**, whose
only default declaration is dark-blue foreground, sRGB `#00008B`. Other
character properties inherit from the underlying text. Its modeless editor
uses application-settings undo, not document history; this style is never
assigned to text or inherited by typing.

Markers apply to Text, Code and source views. They are suppressed whenever
`Format::is_wysiwyg()` is true, including rich-format Code blocks. `conceal`
configures any future literal-source conceal decoration; it does not reveal
hidden WYSIWYG syntax. `extends` and `precedes` decorate viewport clipping of
unwrapped lines. Marker ink fits/clips to existing whitespace geometry and
does not add source characters, caret stops, wrapping width or line height.
Search, registers, clipboard, accessibility, undo and serialization see the
actual text. Exact-snapshot marker exports use the same active presentation text
as their layout, including input-method marked text and manual completion
previews. Base-layout fallback must also use base document text.

Tests MUST cover option inheritance and atomic rejection, tab and soft-tab
stops, comment/reflow agreement, count/repeat/undo behavior, WYSIWYG exclusion,
marker precedence and Unicode validation, mixed-font geometry, cache
invalidation, multiple views, composition and bounded large-document layout.

### Hard-line reflow

This subsection specifies the implemented `gq`, `gw`, and `textwidth`
behavior. Implementations MUST keep satisfying its command, preservation, and
regression requirements.

#### Text width and settings

Settings > Editing includes an integer field labeled **Text width
(columns)**, defaulting to **80**. Persist the application default as
`editing.textWidth` in the existing versioned `config.json` authority. Missing
values use 80; validation and atomic writes preserve unrelated settings.
The value is a positive unsigned 32-bit integer. Zero, negative, fractional,
malformed, and overflowing values are rejected without changing the prior value.
Vim's zero-width fallback to window width is outside this initial feature.

The effective width is buffer-owned and shared by every view of that buffer.
New buffers inherit the application default. Changing the Settings value updates
open buffers that still inherit it, but preserves explicit buffer overrides.
`:set textwidth=72`, `:set tw=72`, and their `:setlocal` forms set an override in
the current buffer; they do not write the application default. Both spellings
support `?` queries of the effective value. A buffer can explicitly return to
inheritance with `:setlocal textwidth<` or `:setlocal tw<`. These option changes
do not change source, dirty state, or document undo history. Reopening a document
starts with the application default rather than persisting a local override in
the source artifact.

Width counts the complete output line, including indentation, list markers, and
comment leaders. Use a portable Unicode column-width policy, with combining
sequences kept together and tabs advancing to the buffer's effective `tabstop`
stops (two columns by default). It MUST NOT
depend on UTF-8 byte length, font choice, proportional glyph advances, zoom,
window width, or syntax styling. An unbreakable token may exceed the target;
neither that token nor a grapheme cluster is split to force a fit.

Changing `textwidth` does not reformat existing text or enable automatic hard
wrapping during typing, paste, or input-method commit. The existing per-view
`wrap` option and source Paragraph Flow remain independent presentation choices.

#### Commands and range semantics

Implement `gq` as a composable operator using the shared command grammar,
including multiplied operator/motion counts, existing motions and text objects,
and cancellation at each pending stage. Required forms include `gq{motion}`,
`gqq`, `gqgq`, counted line forms, `gqj`, `gq}`, `gqip`, `gqap`, and Visual `gq`.
Implement `gw{motion}`, `gww`, `gwgw`, and Visual `gw` with the same formatter
and distinct cursor policy. `gq}` is the paragraph-forward form; `gq]` alone
is incomplete Vim grammar. Section motions such as `]]` and `[[`, and therefore
`gq]]`, are not added by this feature.

Resolve the motion or selection with the existing checked endpoint algebra,
then format the source hard lines it covers. Characterwise endpoints obey Vim's
exclusive-end rules before conversion to a hard-line span. Visual Character
and Line use their covered hard lines; Visual Block formats each intersected
hard line in full, once, rather than reflowing a pixel-width rectangle. Line
counts in doubled forms and implicit line motions such as `gqj` use hard lines
regardless of soft wrapping or the view's Visual/Physical line setting.
Explicit visual-row motions and Visual selections retain their normal domains
before expansion to hard lines. Source outside those hard lines is unchanged,
and paragraph recognition MUST NOT expand the edit beyond them.

`gq` leaves the cursor at the first nonblank of the last formatted line, following
Vim 9.2's command-specific endpoint behavior. `gw` restores its original text
location through a persistent anchor and the committed change map; deleted
whitespace follows the normal anchor recovery policy. Successful Visual forms
leave Visual mode. Cursor movement after reflow follows the same minimum-reveal
scroll policy as other commands; it does not force bottom alignment or
recentering when the caret is already visible.

#### Paragraphs and comment leaders

The initial formatter supports Text and Code, operating on their literal hard
lines. Markdown Source, HTML Source, and WYSIWYG Markdown, HTML, and RTF require
separate format-aware semantics and initially return a non-destructive
unsupported-format result. They MUST NOT acquire structural paragraph breaks
through a generic text reflow implementation. Formatting is internal portable
policy, without invoking external formatters or evaluating Vimscript.

Within the selected span, join and greedily wrap each ordinary text paragraph
at inter-word whitespace. Preserve its indentation, word order, punctuation,
blank paragraph separators, and recognized list structure. Use one space
between joined words; do not split words or URLs. Preserve bullet/number markers
and hanging continuation indentation, and do not join adjacent list items.
Initially recognize `-`, `+`, or `*`, or decimal digits followed by `.` or `)`,
with following whitespace as list markers. Retain their spelling and numbering.
Differing indentation separates paragraphs unless it is a recognized list
continuation. A prefix that already exhausts the width does not cause an empty
line or repeated splitting; the following unbreakable token may overflow.

Code additionally uses declarative comment profiles selected by the buffer's
language. Comment recognition and reflow semantics MUST be independent of
asynchronous syntax coverage, highlight-group names, theme/style properties,
and which highlighter is available. The C-family profile supports standalone
`//`, `///`, and `//!` leaders and `/* ... */` comments with optional interior
`*` leaders. One declaration serves every canonical language that spells its
comments that way: C, C++, Objective-C, Rust, Swift, C#, JavaScript,
TypeScript, TSX, Go, Java, PHP, and CSS. A language whose comments start with
`#`, `--`, or `"` has no profile yet; its leaders reflow as ordinary words
until one is declared. Nested block comments are not tracked, so a language
that allows them ends its block at the first closing delimiter.
Recognize full-line leaders after indentation, preferring the
longest applicable leader; a delimiter occurring inside code or a string is not
by itself a full-line leader. Interior `*` leaders require a matching block
comment context rather than treating arbitrary leading asterisks as comments.

Reflow a comment paragraph's body while retaining indentation and recreating
its leader on each resulting continuation line. Preserve block-comment opener
and closer tokens and the existing middle-leader convention when line counts
change. Delimiter-only opener and closer lines stay on their own lines. Blank
comment-only lines remain separators. A change between `//`,
`///`, `//!`, another comment kind, or non-comment text starts a new paragraph;
the formatter MUST NOT join across that boundary. Lists inside comments retain
their hanging indentation after the repeated comment prefix. Initially, a line
containing code followed by a trailing comment remains byte-identical and forms
a paragraph boundary; reflow of such mixed lines is deferred until explicit
continuation rules are specified.

For example, at width 40:

```cpp
// One paragraph split across
// several short lines.
```

becomes:

```cpp
// One paragraph split across several
// short lines.
```

#### Transactions and validation

Each reflow is one atomic source transaction and one undo unit, including
multiple paragraphs or selected hard lines. It leaves registers unchanged.
Dot repeat records the operator, motion/selection extent semantics, and count,
and uses the target buffer's effective width when repeated. Macro replay uses
the same core command path. Undo and redo restore recorded source and cursor
state rather than invoking the formatter again. A source-identical result
creates no history node and does not dirty the buffer.

Produce minimal whitespace/leader patches through the shared lossless editing
pipeline. Preserve unchanged physical bytes, existing retained line-ending
spellings, encoding, and final-terminator presence. New hard breaks use the
buffer's `fileformat`; do not normalize unrelated mixed line endings. Reflow
must not pass generated text through smart-quote or other typing assistance.
Unsupported encoding, stale input, cancellation, or resource exhaustion leaves
the entire requested edit unapplied and does not replace the prior dot recipe
or mutate registers. Bound preparation to the selected content and necessary
local boundary/comment context; a small reflow MUST NOT flatten,
highlight, or relayout the entire document.

Required tests cover command counts and operator composition, `gq` versus `gw`
cursor placement, Visual variants, Escape, registers, dot/macro replay, single
undo/redo, and no-op dirty/history behavior. Add golden cases for indentation,
tabs, Unicode widths and graphemes, overlong words/URLs, lists, all C/C++ leaders,
block comments, blank/changed leaders, mixed trailing-comment boundaries,
partial-paragraph selections, encoding, mixed line endings, and final
terminators. Test Settings persistence/validation, numeric Ex aliases/queries,
default propagation, local overrides, and multiple views of one buffer.
Compare supported Vim behavior against pinned Vim 9.2 and explicitly fixture
the deliberate differences above. Include a large-document local-reflow test,
cache invalidation, cancellation/staleness, and source-byte locality checks.

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

#### Manual word completion

Insert mode supports Ctrl-N and Ctrl-P over words in the current formatted
document. Ctrl-N discovers matches in forward document order and Ctrl-P in
backward order, wrapping once. Matching is case-sensitive prefix matching at
the caret, using the same Unicode-alphanumeric-or-underscore grapheme classes
as word motions. Empty prefixes are permitted. The occurrence being completed
is excluded; duplicate insertion strings are shown once. WYSIWYG views search
visible formatted words across character styles, while literal/source views
search their visible source text. Completion does not copy candidate styles.

Only Ctrl-N/P navigate the completion cycle. Every other key, including Escape,
Ctrl-E, Ctrl-Y, Enter, Tab, Backspace, and arrow keys, accepts the displayed
candidate and then performs its ordinary action. This deliberately differs
from Vim's special completion meanings for Ctrl-E/Y and popup cursor keys.
The original typed prefix is a selectable entry in the cycle. Completion in
Replace mode and deferred Visual Block insertion is not yet supported.

The core owns the session, source selection, candidate order, selected index,
and search progress. The frontend displays a native nonactivating popup from
that state and schedules bounded core work while requested. Search runs in
cooperative slices between input events, including prefix discovery and Unicode
segmentation in huge lines/clusters; it never flattens the document or blocks a
key while scanning the rest of the buffer. New results append without reordering
existing items. Requests past the known results wait for discovery or exhaustion.
Search memory and candidate word lengths are bounded; truncation is explicit.

Cycling uses a view-local formatted preview and does not modify authoritative
source, dirty state, or undo history. For unwrapped presentation lines exceeding
the bounded layout-slice limit, selection is shown in the popup only until
acceptance; cycling MUST NOT rebuild their whole-line width/bidi summaries.
Acceptance inserts the selected missing suffix through ordinary verified
semantic typing, preserving the existing
prefix and destination typing style. It belongs to the current Insert undo
unit. Counts, dot repeat, the last-insert register, and macros retain accepted
text, not candidate navigation or asynchronous search timing.
If the following key fails after acceptance, the accepted completion remains;
the outcome reports its source change and position map together with the key's
failure diagnostic. No committed completion is hidden by an error-only return.
Literal-next, register operands, and pending Ctrl-G input keep their existing
key ownership.
Snapshot changes, view destruction, and abandoned sessions discard search work;
stale results cannot revive a popup. Native input-method or menu/pointer actions
accept completion through the core before constructing new revision-bound
targets. IME and completion have separate state and undo policies.

The core supplies popup geometry at the beginning of the word being completed,
resolved against the current presentation layout. Candidate cycling does not
anchor to the moving insertion caret; the popup follows the word only when its
actual placement changes, including wrapping, alignment, scrolling, and zoom.
The native popup compensates for its actual text-cell padding so its words'
leading edge aligns with the word's leading edge, subject to screen boundaries.
The word's resolved bidirectional context determines popup direction: RTL
popups mirror native layout and align text to the right edge. Candidate order
and Control-N/P behavior remain backend-owned and unchanged.

#### Ordinary insertion controls

Required behavior includes ordinary Unicode text input, Escape/Ctrl-[, Enter,
Tab, Backspace, Forward Delete, arrow movement, Home/End, Page Up/Down,
`Ctrl-W`, `Ctrl-U`, `Ctrl-R {register}`, `Ctrl-O {normal-command}`,
`Ctrl-E`, `Ctrl-Y`, `Ctrl-G u`, and `Ctrl-G U`.

Control-key aliases are portable command policy. In Insert/Replace and prompts,
Ctrl-H is Backspace, Ctrl-I is Tab, and Ctrl-J/M is Enter. In Normal/Visual,
Ctrl-H is Left, Ctrl-J/N is Down, Ctrl-P is Up, and Ctrl-M is Enter; physical
Tab is the newer-jump command Ctrl-I. Ctrl-Left/Right move by word, including
Insert/Replace and prompt input. Native modifiers MUST survive input routing.
Literal-next input and pending register operands retain the original key
identity before aliases are resolved. Ctrl-[ cancels like Escape in every mode.

Ctrl-C cancels pending command grammar or leaves Insert/Replace, Visual, and
prompt input. It retains edits already entered and suppresses unfinished Insert
entry-count expansion. During deferred block insertion, it retains insertion
on the first row only; a block change still deletes its complete selected area.
During Insert/Replace Ctrl-O, Ctrl-C ends the suspended insertion in Normal
mode, retains its completed text and repeat recipe, and closes its session.

Ctrl-E/Y copy one complete extended grapheme from the next/previous hard line
at the caret's tab-expanded logical column. Tab stops and Unicode widths use
the same portable column definitions as indentation; fonts, character styles,
and soft wrapping do not affect which character is copied. A column inside a
tab or wide grapheme selects that entire item. Missing lines/columns are no-ops.
Copied tabs remain hard tabs even with expandtab, and copied text bypasses
typing assistance. Copying reads only the current prefix and neighbouring line.
Counts and dot repeat retain the copied value; macros reexecute the command.

Replace Ctrl-W/U restore overwritten text through the same source-preserving
replacement journal as Backspace. They remove newly appended text normally,
remain one atomic command in the current undo group, and replay as semantic
word/line operations at their destination.

macOS marked-text/IME composition is required. An active composition is a
temporary marked range, updates visually as one composition, and commits as a
single source transaction/undo unit. Cancelled composition restores the
pre-composition projection without changing source. Native input methods and
press-and-hold accent picking are available during document text entry (Insert,
Replace, and native Selection/Vim Select) and status-line command/search entry.
Normal and Vim Visual modes, including operator/operand pending and Insert
Ctrl-O command execution, do not expose an AppKit input context or accept marked
text. Their printable keys and repeats go directly to the command machine;
already formed Unicode command operands remain supported. A mode transition
must refresh AppKit's cached input context and dismiss leftover candidates even
when there is no marked range. Late explicit native replacement callbacks in a
command mode must not execute their replacement text as commands. Native
Home/End key bindings continue to resolve through AppKit in every mode.
Escape/Ctrl-[ and AppKit's
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
return to Normal mode. This includes character, line, or block Visual selections,
and mouse selections when `noautoselect` is set and `selectmode` excludes `mouse`. With no selection, Normal-mode
Backspace retains its leftward motion and Forward Delete retains `x` behavior.

### Native Selection and Vim Select modes

`autoselect` is an application-global Boolean option, enabled by default. It
enters native **SELECTION** for mouse or Shift+navigation selection, independently
of Vim's `keymodel` (`km`) and `selectmode` (`slm`). Those Vim options default to
empty. `noautoselect` returns mouse and shifted-key entry to their Vim policy:
`startsel` starts Visual or Select with shifted special navigation, `stopsel`
ends it with unshifted navigation, and `selectmode` accepts `mouse`, `key`, `cmd`.
The latter chooses Select instead of Visual for that entry origin. Shifted
horizontal arrows without `startsel` use Vim word motions when `autoselect` is
off. Vim `v`, `V`, Ctrl-V, and explicit Select entry are independent of the native
option. Extending an active selection MUST retain its interaction policy.

Hosts distribute these options to existing and future documents in the same
profile without changing current selections, typing state, or pending commands.
`autoselect` supports enable, disable, query, invert, and reset; the Vim string
options support assignment, query, reset, append, prepend, and removal. Compound
`:set`/`:setlocal` operations validate atomically; startup uses the same grammar.

Selection shape (character, line, block) is separate from interaction policy.
Native Selection uses exact half-open insertion boundaries; Vim selections use
inclusive endpoints. Native unshifted Left/Right collapse to the corresponding
edge without another step. Other unshifted navigation ends native selection;
selections started in Insert/Replace resume that mode. Typing or Enter
replaces and enters Insert, while native Backspace/Delete deletes and enters
Insert; replacement and subsequent typing form one undo group. Vim Select
Backspace/Delete returns to Normal. Vim `stopsel` ends selection and performs
the requested movement, rather than native edge collapse. Registers, Unicode
graphemes, counts, dot-repeat, and block extents remain portable policy.

`gh`, `gH`, and `gCtrl-H` explicitly enter Vim Select Character, Line, and Block.
`Ctrl-G` toggles Vim Select/Visual; in native Selection it switches explicitly
to Visual. `Ctrl-O` runs one complete Visual command before returning to the
originating policy when a selection remains; native Copy preserves that policy
and exact extent. Native Selection does not execute Vim Visual/Select mappings.
Character/line IME stays an overlay until commit; cancellation restores the
selection. The frontend labels native Selection **SELECTION**, and preserves
Vim's distinct SELECT/VISUAL labels. Current platform differences are recorded
in `docs/native-selection.md`.

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
- Native Copy (`Command-C` on macOS, `Control-C` on Windows, menus, and platform
  Copy selectors) preserves the directed selection, active endpoint, selection
  mode, and caret. It exports the selected visible text and rich clipboard
  representation through a dedicated portable Copy intention. It does not
  synthesize a Vim yank or temporary mode transition, execute a pending mapping,
  or consume a pending register/count or temporary Visual command. Vim `y`, including explicit
  clipboard-register yanks, retains its normal selection-ending behavior.
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
- Ctrl-G u closes the current unit without leaving Insert/Replace or discarding
  its repeat recipe. Ctrl-G U preserves the unit and repeat recipe through the
  immediately following Left/Right movement within the same hard line. Any
  intervening input cancels the join; other movements retain their
  usual undo boundaries. Dot/count replay stays one undo unit even
  when the recorded program contains explicit undo breaks.
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
  change number. `:earlier`/`:ea` and `:later`/`:lat` move through chronological
  changes across branches, defaulting to one change. Counts accept `s`, `m`,
  `h`, and `d` for elapsed time, or `f` for successful full-buffer writes.
  Navigation clamps to retained history boundaries. `:undolist`/`:undol`
  reports each retained leaf's change number, depth, age in seconds, and write
  number. Selecting a node is atomic and updates the same preferred-child
  links as stepwise navigation. Timestamps and write annotations belong to
  the portable history model; printing the list creates no undo entry.
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

Saving to another name or location MUST preserve the previously bound source
file byte-for-byte and write the selected destination separately. A successful
Save As adopts that destination; a failed write retains the original binding.
No format change, Save, Save As, or native document action may rename or delete
the original path. Ordinary Save retains the existing filename and extension,
including Code files, unknown extensions, dotfiles, and extensionless files.
Format selection is an in-memory operation until an explicit write. When the
serialization family differs from the last-loaded/saved format, native Save
requests Save As and every write entry point rejects the original destination.
Text/Code, Markdown/Markdown Source, and HTML/HTML Source each share one
serialization family, so those presentation changes do not require a new file.
A successful Save As establishes the destination's new format baseline. Native
rename/move entry points use the same write-and-adopt semantics as Save As, and
the File menu exposes Save As rather than destructive Rename/Move commands.
Atomic replacement of bytes at an explicitly selected save destination and
cleanup of Viem-owned temporary/recovery files remain permitted.
Different destination names resolving through symbolic links or case-only
aliases to the original file are rejected: overwriting an alias would still
change the original's bytes. An ordinary save to the unchanged bound name is
permitted. Distinct hard-link names use Save As replacement so the original
name and bytes remain intact.

History retention has configurable node and retained-byte budgets. The default
byte policy allows 256 MiB of additional retained history above the current
document state's unavoidable storage; a large live document must not by itself
erase every undo step. Explicit callers may instead choose a combined retained
byte target. Diagnostics report the total retained estimate, current-state
estimate, and additional history cost separately. All estimates include retained
source, derived projections and their materialized caches, position maps, and
applicable bookkeeping; moving live state outside the default history allowance
must not hide its memory from total diagnostics. A conservative heap estimate
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
read-only text and an explicit close button at the left of the status line.
A new `:` replaces that output with an editable prompt; ordinary editor input
dismisses the output and is processed normally. Output also dismisses after
30 seconds. Selecting, navigating, and copying output do not dismiss it or
extend the timeout; output cannot be cut, deleted, or edited.

Ctrl-R selects a register for insertion into Ex or search prompts. Ctrl-R
Ctrl-R and Ctrl-R Ctrl-O insert literally; the ordinary form interprets supported
prompt-editing controls while keeping prompt terminators and Tab literal.
Register expansion never submits a command or edits the underlying document.
Ctrl-R Ctrl-W/A/L insert the word, WORD, or hard line at the document caret.
The `/` and `:` selectors insert the last search and last submitted Ex command;
existing text, clipboard, last-insert, and filename registers remain available.
Semantic register hard breaks become literal CR in the prompt; literal LF
content retains its identity. Esc/Ctrl-C during register selection cancel that
selection and retain the prompt. Recursive expansion is bounded. Unsupported
expression, alternate-file, and filename/path-object selectors report an error.

Filename arguments to `:edit`/`:E`, `:write`, `:saveas`, `:wq`, `:xit`,
`:split`/`:vsplit`, `:read`, `:source`, `:file`, and `:cd`/`:chdir` support
Rust-backed prefix completion.
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
`'<,'>` for the selected logical hard-line range. Visual endpoints are retained
as marks, so manually typed ranges and recalled Ex history use the latest
selection. The initial prompt range is bound to that document
revision; an intervening edit in another view makes execution stale rather
than silently applying a stale selection. Escape changes no source,
and `gv` can restore the remembered selection.

Ex addresses support local marks (`'a`), Visual marks (`'<`, `'>`), and forward
or backward Regex v2 patterns (`/pattern/`, `?pattern?`) alongside numeric
addresses, `.`, `$`, and signed offsets. Empty patterns reuse the preceding
search. Pattern addresses search beyond the current hard line and obey the
case and wrapping options. A comma keeps the original current line for both
addresses; a semicolon resolves the second relative to the first. Copy and move
destinations accept the same address grammar. Missing marks, invalid patterns,
and failed addresses leave source and search state unchanged.

`:e`/`:edit` replaces the active pane after the usual unsaved-change review.
Replacing a pane with a different document requires a clean buffer or `!` only
when that pane is the buffer's last view. Reloading the current file with
`:edit` still requires a clean buffer or `!`, because it changes the shared
buffer itself.
The case-sensitive `:E` opens a document in a new native window. `:pwd` displays
the working directory; `:cd`/`:chdir` changes the application's directory for
relative file requests. `:update` writes only a modified buffer. `:s` remains
substitute; filename-like misuse reports `:w` and `:saveas` as the save commands.
Open file-backed documents watch the file and its containing directory for
external writes, atomic replacement, deletion, and recreation. Filesystem
notifications are coalesced; unchanged documents are not repeatedly read by a
polling timer. `:checktime`, window activation, and notifications compare the
bound artifact with the exact last-loaded/saved fingerprint off the main thread.
Changed or replaced files present a prompt offering Keep Buffer (the default)
or Load File. The dialog is the only change notification; successful review,
deferred or acknowledged review, and reload do not also publish status-bar
messages. An explicit `:checktime` may report that an unchanged file is unchanged.
Load File explicitly discards unsaved changes in every view of
that buffer, and the prompt warns when such changes exist. Deleted or unreadable
files offer Keep Buffer and preserve the editable text. Prompts for inactive
documents wait until the app and document window can present them.
One disk state produces at most one acknowledged prompt per buffer, including
across multiple views. Keep Buffer acknowledges only the notification, never
the load/save baseline. A further disk change requires another choice. Reload
consent becomes stale if the buffer, bound path, or observed disk state changes
while the prompt is open. A reload installs the exact verified bytes and retains
the current editing format; a failed replacement preserves the existing buffer
and views. Returning to the saved baseline ends the previous notification's
acknowledgement. The review/acknowledgement policy lives in the Rust
document layer; native file watching and alert presentation live in `src/mac`.
Retargeting a document moves its watch; closing it stops monitoring. Viem's own
saves refresh the baseline before pending file notifications are reviewed.
Every explicit save path,
including native Save/Save As, close-review saves, `:write`, `:saveas`, `:wq`,
`:xit`, `:update`, `:wall`, and forced `!` variants, checks the destination before
writing. Changes since the last load/save require a modal error dialog offering
Cancel (the default) or Save Anyway. `!` does not bypass this external-change
confirmation. A successful reload/save refreshes the baseline. Checks never
reload without a choice; Load File and `:e!` explicitly reload. A queued write rechecks its
accepted fingerprint before publishing; a further change needs another choice.
This narrows races with other writers but is not a filesystem compare-and-swap.

Normal-mode `ZZ` requests one full save followed by closing the active view.
It ignores a count and register prefix, changes no registers or undo history,
and has no operator-pending form. Escape cancels the uppercase `Z` prefix.
Read-only rejection, save cancellation, stale snapshot acknowledgement, or any
write failure reports the failure and MUST keep the document/view open.
Successful `ZZ`, `:quit`, `:qall`, `:wq`, and `:xit` commands (including their
abbreviations and supported force/range/path variants) terminate the application
when their close leaves no document windows or other ordinary application
windows open. Closing only a split pane does not terminate. Stoplight and other
non-command window closes do not request application termination; a previous
command close must not change that policy for a later ordinary close.

The editor provides a native contextual Cut/Copy/Paste menu through right-click,
Control-click, and keyboard contextual-menu access. Wheel deltas use AppKit's
already preference-adjusted direction exactly once.

Required Ex commands and common unambiguous abbreviations are:

- files/views: `:edit`, `:enew`, `:split`/`:sp`, `:vsplit`/`:vs`, `:only`/`:on`,
  `:close`/`:clo`, `:write`, `:saveas`, `:read`/`:r`, `:file`/`:f`, `:source`/`:so`,
  `:quit`, `:qall`, `:wq`,
  `:xit`, `:wall`, `:next`/`:n`, `:Next`/`:N`, `:previous`/`:prev`,
  `:wnext`/`:wn`, `:wNext`/`:wN`, `:wprevious`/`:wp`, `:first`,
  `:rewind`, `:last`, `:argument`/`:argu`, and force `!` variants where meaningful;
- editing: `:undo`, `:redo`, `:earlier`, `:later`, `:delete`, `:yank`, `:put`,
  `:join`, `:copy`, `:move`, `:sort`, `:>`, `:<`, `:retab`, `:left`, `:right`,
  `:center`, and `:normal` for the supported Normal command subset;
- search/change: `:global`/`:g`, `:vglobal`/`:v`, `:substitute` with ranges,
  confirmation and repeat flags, `:&`, `:~`, and
  `:nohlsearch`/`:noh`;
- navigation/info: line addresses, `:goto`, `:marks`, `:delmarks`, `:registers`,
  `:jumps`, `:undolist`, `:print`/`:p`, `:number`/`:nu`/`:#`, `:list`/`:l`, and
  `:pwd`; and
- options: `:set`, `:setlocal`, `:set wrap`, `:set nowrap`, `:set fileformat?`, and
  `:set fileformat=unix|dos|mac` (where one value is supplied), including the
  `ff` abbreviation and corresponding `:setlocal` forms. The global
  `fileformats` open-policy option supports query and ordered assignment even
  though changing it does not reinterpret an already open buffer.

Search switches `ignorecase` (`ic`), `smartcase` (`sc`), `wrapscan` (`ws`),
`hlsearch` (`hls`), and `incsearch` (`is`) are buffer-shared Boolean policy,
defaulting to false, false, true, false, and false. Their
`:set` and `:setlocal` forms support enable/disable, toggle, query, and reset.
A compound option command validates atomically before publishing any changes.
Case switches follow Regex v2 rules for searches, repeats, and substitute;
`wrapscan` controls navigation wrapping. Highlighting and incremental-preview
behavior is specified under **Search highlighting and incremental preview**.
These options do not change persisted source.

The numeric `textwidth`/`tw` option, its buffer scope, query and inheritance
forms, and the Settings default are specified under **Hard-line reflow**. It
is part of the implemented option set.

Ranges always use hard lines. File dialogs, unsaved-change prompts, and error
presentation are frontend responsibilities driven by typed core requests and
results. Write commands serialize the authoritative source artifact, preserving
unchanged source slices exactly; they never export a newly normalized formatted
document as a substitute for the source.

`:global[!]/pattern/command` and `:vglobal[!]/pattern/command` default to all
hard lines; `vglobal` inverts matching and `!` reverses either selection.
The default command is `:print`. Matching uses Regex v2 and records the search
pattern. The first pass selects stable line identities; the second runs the
command once per surviving selected line. Insertions do not add targets,
deleted targets are skipped, and moved targets retain their identity. One
global invocation forms one undo unit. Per-line failures are reported while
remaining targets continue. Nested global commands act as current-line
predicates and cannot have a range. Host/file requests, history navigation,
and interactive substitute confirmation are unsupported inside global.
Regex work, selected targets, and recursive replay have explicit limits.

Substitute's `c` flag opens a core-owned confirmation interaction. The current
match uses **Incremental match** regardless of `hlsearch` or `incsearch`, and
the native prompt shows its expanded replacement and progress. Literal,
unmapped `y` accepts, `n` skips, `a` accepts the remainder, `l` accepts the
current match and finishes, and `q`, Escape, or Ctrl-C finish with the decisions
already made. Viem deliberately stages decisions against the original
snapshot and commits approved replacements together when confirmation ends,
preserving captures, styles, and one undo unit. Source remains unchanged while
choosing. An intervening source revision invalidates the pending interaction
and discards its staged decisions with a diagnostic. Ctrl-E/Ctrl-Y scrolling
during confirmation is not implemented.

Standalone `:print`, `:number`, and `:list` use the selectable command-output
surface. They default to the current hard line; a trailing count starts at the
last addressed line. Their `p`, `#`, and `l` output flags also apply to Ex shifts.
List output renders tabs and control characters visibly and appends `$` to
each hard line; printable Unicode remains intact.
Printing changes no source, registers, or undo state.
`:>` and `:<` shift addressed hard lines using `shiftwidth`, `tabstop`, and
`expandtab`; repeating the symbol multiplies the shift, while a trailing count
selects how many lines to shift from the last address. Empty lines remain empty,
outdenting stops at column zero, and the batch is one undo unit.

`:retab[!] [-indentonly] [new-tabstop]` defaults to all hard lines. It measures
existing whitespace with the old tabstop and writes equivalent logical-column
whitespace using the new tabstop and `expandtab`. Without `!` it changes only
runs containing a tab; `!` also allows compression of space runs. `-indentonly`
restricts conversion to leading whitespace. Omitted or zero tabstop retains
the current value; a valid nonzero value also sets the buffer's tabstop.
Invalid arguments do not partially edit source or options.

In HTML and RTF formatted views, `:left`, `:right`, and `:center` set sparse
direct paragraph alignment on the addressed paragraphs, retaining their text
and other styles. Alignment follows paragraph start/end direction. Numeric
indent/width arguments are rejected in these views. Formatted Markdown reports
unsupported alignment. In plain text, Code, and source views, the commands
adjust leading whitespace using logical columns and the indentation options.
`:left [indent]` defaults to zero; `:right [width]` and `:center [width]` default
to `textwidth`, or 80 when it is zero. Each operation is one verified,
source-preserving undo unit.

`:[address]read [file]` inserts decoded literal text lines after the addressed
hard line, defaulting to the current line and the current filename. Address
zero inserts before the first line. The imported file's encoding and line
endings are decoded independently, then semantic hard breaks are written
through the destination format's normal verified insertion path. Source bytes
outside the insertion remain untouched. The operation changes no register,
forms one undo unit, and moves the invoking view to the first inserted line.
An empty file is a no-op. Native file I/O runs outside the core lease, and its
completion validates the originating document and revision before editing.
Shell filters (`:read !command`) and `++` overrides are unsupported.

`:source file` reads a UTF-8 command file and executes supported Ex commands
and mapping definitions in order, using the same portable command interpreter
as interactive input. Blank lines, leading `"` comments, an initial BOM, and
CRLF files are supported. Meaningful trailing payload spaces remain intact.
Host operations complete before the next command runs; nested sources share
a command budget. Limits are 1 MiB per file, 10,000 input lines per invocation
including nested files, and 16 levels of nesting. Errors identify the path and
line and stop the remaining commands, retaining successful earlier commands
with their ordinary undo units. Interactive substitute confirmation, Normal
key-script `:source!`, and a general Vimscript runtime are unsupported.

`:file [name]` reports the current filename and buffer flags, or changes the
buffer's filename without writing its contents. Renaming retains dirty state,
updates filename-dependent syntax and recovery ownership, and refuses another
live document's path. It MUST NOT treat existing bytes at a new destination as
loaded or saved content; subsequent writes retain overwrite protection.
`:only[!]` keeps the active pane. Without `!`, it validates every pane to be
closed and rejects the entire operation if it would discard a modified
buffer's last view. `!` permits that discard. Other surviving views of a
buffer retain their contents. `Ctrl-W o` shares the non-forced close policy.

`:[range]sor[t][!]` defaults to all hard lines. It supports `i` (Unicode
case-insensitive comparison), `u` (remove duplicate full lines), mutually
exclusive `n`/`x`/`o`/`b` numeric keys, and an optional Regex v2 pattern. The
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

### Startup files and argument navigation

Viem runs one editor process per user. A later executable invocation forwards
its original argument vector and working directory to the existing process,
then exits after acknowledgment. Process election must be atomic across
simultaneous launches and recover after a crash. Startup and forwarded requests
use the same argument parser and document-opening path; forwarded requests wait
until startup is ready and are processed in order, including across nested
document-recovery dialogs. A failed handoff reports an error rather than starting
a second editor.

An invocation without launch options activates the existing application and
brings an existing window forward; if no windows remain, it creates the same
blank document as a fresh startup. A file requested for immediate opening that
already appears in a window or pane keeps its existing buffer and view, including
unsaved edits; focus that pane and bring its window forward. Do not create an
additional window or pane for repeated filenames, symlinks, or hard links to that
file. Install the invocation's argument list in reused panes as well as new ones.
New files selected by `-o` share a new stacked window; represented files retain
their current windows. The first requested file retains focus, and `+line`
navigates it without inserting text or replaying pending Insert/Replace input.
Explicit extra empty panes from `-oN` remain supported.

The executable accepts multiple filename arguments, captured relative to the
launch working directory in their supplied order. By default only the first
file is opened; later files are read when selected. A nonexistent filename
opens a named, clean, empty buffer without creating a file on disk. `--` ends
option parsing so filenames beginning with `-` or `+` remain expressible.
Unknown options and unsupported startup commands report an error and usage.
The macOS bootstrap disables AppKit's automatic conversion of process arguments
into native file-open callbacks before starting the application. Each argument
must be handled only by this launch policy; later Finder file-open events retain
their normal behavior. Startup verification must exercise a fresh application
process, including default multiple files, `-o`, and `+line`; calling the delegate's
argument-list helper directly cannot validate AppKit startup dispatch.

Vim's `+123` selects hard line 123 of the first file, clamped to the file's
last line, with the cursor at the first nonblank grapheme. Bare `+` selects the
last line. This is a line number, not a file index. `-o` opens argument files
in stacked panes; `-oN` requests N panes, adding empty panes if there are fewer
files and respecting the available height. `-o0` has the same meaning as `-o`.
Startup keeps the first pane focused and applies `+line` only there.

`:next` and `:Next`/`:previous` move forward and backward by a count, default
one, without wrapping at either end of the argument list. `:first`/`:rewind`
and `:last` select its endpoints; `:argument N` and `:Nargument` select the
one-based file number, and bare `:argument` selects the current argument.
Common unambiguous abbreviations, meaningful `!`, and `+line`/bare `+`
modifiers are supported. Previous and argument commands accept Vim's trailing
count, which takes precedence over a leading count. Argument-list replacement
through `:next filenames`, wildcard expansion inside Ex, and arbitrary `+cmd`
are outside this command commitment.

Navigation is relative to the pane's current file when it belongs to the list.
Otherwise it resumes from the last argument position retained by that pane
before an unrelated file replaced it. Splits copy this position and retain
independent subsequent navigation. Reopening a represented file reuses its
buffer, including unsaved content. Only replacing the last view of a modified
buffer requires `!`; forced navigation never discards edits in another view.
An unsuccessful or stale open leaves the pane and remembered position intact.

`:wnext`, `:wNext`, and `:wprevious` write before navigating, including writing
before an end-of-list error. Write failure, cancellation, or a changed revision
prevents navigation. An alternate write filename does not rebind the buffer or
clear its modified state, so the ordinary last-view guard still applies.
File navigation changes no text, registers, or undo history by itself.
Argument parsing and index/count policy live in the portable Rust command
module; file identity, native I/O, pane lifetime, and dialogs belong to the host.

### Stacked document views

File > Open, Open Recent, and Finder opens reuse the active single-pane
window when it contains an untouched, empty, untitled document. Read the incoming
file successfully before replacement; keep the same native window and geometry
without a new-window opening animation. After any open or recovery prompt,
recheck the original document/window identity, revision, empty source, and clean
state. Failure or cancellation leaves the original window intact. Named,
nonempty, recovered, previously edited, and multi-pane documents remain open;
additional selected files open in separate windows. Already represented files
retain their existing backend and window. Eligibility uses cached state and
source-tree aggregates, never a full source copy.

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

The latest normal document-window frame is retained at `windows.documentFrame`
in config.json. The first window on the next launch restores its size and
location. If its screen geometry no longer fits, move it onto an available
screen's visible frame before shrinking only the dimensions that cannot fit.
Later new windows cascade down and right using AppKit's native title-bar
spacing. Showing an existing window preserves its frame. Minimized/fullscreen
frames are not saved as normal geometry.

#### Window commands

`CTRL-W` is a Normal and every Visual mode window prefix, including Visual Block.
Counts may occur before or after the prefix; when both occur their product is
used, with checked overflow. Escape/Ctrl-[ and Ctrl-C cancel a pending prefix
without effect. `CTRL-W :` opens the Ex prompt, retaining a Visual range when
applicable. Panes are ordered top to bottom, and
that order is the only geometry these commands address.

Focus commands, which never change pane order or content:

- `CTRL-W j`, `CTRL-W <Down>`, `CTRL-W CTRL-J`: the pane below, stopping at the
  bottom. A count repeats the step.
- `CTRL-W k`, `CTRL-W <Up>`, `CTRL-W CTRL-K`: the pane above, stopping at the top.
- `CTRL-W w`, `CTRL-W CTRL-W`: the next pane, wrapping to the top. With a count,
  the pane with that one-based index.
- `CTRL-W W`: the previous pane, wrapping to the bottom. With a count, the pane
  with that one-based index.
- `CTRL-W t`, `CTRL-W CTRL-T`: the top pane. `CTRL-W b`, `CTRL-W CTRL-B`: the
  bottom pane.
- `CTRL-W p`, `CTRL-W CTRL-P`: the previously focused pane. Each focus change
  records the pane it left, so `CTRL-W p` alternates between two panes.
- `CTRL-W h`, `CTRL-W l`, `CTRL-W <Left>`, `CTRL-W <Right>`: accepted and do
  nothing. A stacked layout never has a left or right neighbour, which is also
  what Vim does when one is absent.
  `CTRL-W CTRL-H`, `CTRL-W <BS>`, and `CTRL-W CTRL-L` are equivalent aliases.

Order commands, which move panes without changing which one is focused:

- `CTRL-W r`, `CTRL-W CTRL-R`: rotate downwards. Every pane moves down one and
  the bottom pane becomes the top. A count repeats the rotation.
- `CTRL-W R`: rotate upwards, the inverse.
- `CTRL-W x`, `CTRL-W CTRL-X`: exchange the current pane with the next one, or
  with the previous one when the current pane is last. With a count, exchange
  with the pane at that one-based index. Focus follows the moved pane.
- `CTRL-W K`: move the current pane to the top. `CTRL-W J`: move it to the
  bottom. Focus follows the moved pane.

Lifecycle commands reuse the existing Ex behavior exactly: `CTRL-W s`,
`CTRL-W S` and `CTRL-W CTRL-S` split; `CTRL-W v` and `CTRL-W CTRL-V` split the
same way, as `:vsplit` does; `CTRL-W n` and `CTRL-W CTRL-N` open a new empty
pane; `CTRL-W q` and `CTRL-W CTRL-Q` quit the pane; `CTRL-W c` closes it; and
`CTRL-W o` and `CTRL-W CTRL-O` close the other panes.
An explicit split/new-pane count sets the new pane's initial height in visual
rows, constrained by the window's available space and minimum pane sizes.
Opening a new pane does not replace or discard the active document.

Size commands change pane heights in whole visual rows of the focused pane,
taking space from or returning it to its neighbours without changing the total:

- `CTRL-W +` and `CTRL-W -`: grow or shrink the focused pane by the count,
  default one row.
- `CTRL-W _` and `CTRL-W CTRL-_`: set the focused pane to the count in rows, or as tall as the
  window allows without a count.
- `CTRL-W =`: give every pane an equal share.

A pane never shrinks below one row plus its status bar, and a size command that
cannot move any pixels leaves every pane unchanged.

Commands that require a layout this product does not have are reported as
unsupported rather than silently accepted: `CTRL-W H`, `CTRL-W L`, `CTRL-W <`,
`CTRL-W >` and `CTRL-W |` need side-by-side panes, and `CTRL-W T` needs tab
pages. `CTRL-W` followed by any other key is likewise unsupported.

Window commands are portable core command grammar. The core resolves the
prefix, the count, and the command key, then emits one typed window request;
the frontend owns pane geometry and focus. They change no document text, so
they create no undo unit, touch no register, and are not repeated by `.`.

### Status line

Each pane owns one status line along its bottom edge. Its contents are:

- a left group with the mode, the source-format popup, and any message; and
- a right-aligned caret position widget: the line-mode icon and the line and
  column, which is also the control that toggles the line mode.

The caret position widget is always present and always right-aligned. The left
group truncates before the widget moves.

While a command line is active, it replaces the whole left group rather than
covering the document with a separate band. The caret position widget remains,
separated from the command area by five points. The command area is drawn in
the status line's inverse colors: its background is the status foreground color
and its text the status background color. That inverted background extends to
the window edge on the leading side, while the command text itself obeys the
corner inset below. The command line keeps its own caret, marked-text
underline, and selection highlight, and scrolls horizontally to keep its caret
visible.

Command output uses that same status area and retains the caret position widget,
but uses the normal status foreground and background colors. Its X close button
is left of the selectable, read-only text. Long or multiline output scrolls
inside the existing status-line height; it never introduces a document overlay
or changes the size of an already-visible status line. A hidden status line
appears while a prompt or output needs it, then follows the visibility preference
again. Closing or expiring output returns focus to the editor only if the output
held focus; it never activates another window or interrupts another control.

A window with rounded corners clips the ends of a status line. Inset every
status line's contents, including the command line's text, by the width of the
region the corner clips by more than one pixel. For a corner of radius `r`,
that width is `r - sqrt(2r - 1)`, the horizontal distance at which the corner
has eaten one pixel of height. Apply the same inset to every status line,
including panes in the middle of a window whose corners are square, so that all
of them align.

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

Home and End are interpreted through AppKit's native key bindings. Native
line-beginning/end selectors and paragraph-beginning/end selectors used by
custom Home/End bindings are aliases for Viem's Home/End motions. They obey
the current Visual or Physical Source line policy and target the same line
positions as `^` and `$`, including Insert and Replace modes. Native
document-beginning/end selectors retain document-edge behavior. Do not
hardcode physical Home/End key codes over user system bindings.

The macOS frontend supports mouse placement/drag selection, scroll gestures,
standard copy/cut/paste/select-all menu items, drag selection auto-scroll, and
font selection for the active range. Native commands dispatch the same core
semantic intentions and verified source transactions as keyboard commands.
They must not maintain a second selection, source, or undo model in AppKit.
Dragging beyond the window or screen and autoscrolling MUST retain the original
selection anchor and complete logical range until an explicit selection-changing
action. Moving selection endpoints outside materialized layout MUST NOT discard
the selection export or its native selected-text range. Character/Line exports
retain their full logical segments and provide rectangles only for materialized
selected content; an entirely offscreen selection may have zero rectangles.
Mouse-up stops autoscrolling and retains the selected range.
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
  - Revert To
    - Last Saved Version
    - Browse All Versions…
  - separator
  - Convert to
    - Text
    - Markdown
    - HTML
  - Reinterpret as
    - Text
    - Code
    - Markdown
    - HTML
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
  - Superscript
  - Subscript
  - Ligatures
    - Use Default Ligatures
    - Use All Ligatures
    - Use No Ligatures
  - OpenType Features (available features of the current resolved font)
  - Show Colors
  - Text Color…
  - Highlight Color…
  - separator
  - Style
    - Edit Styles…
    - Save as default <format> style
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
  - Default Paragraph (clear named character styling)
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

The Format font and color commands open persistent native choosers. On macOS
these are the system Fonts and Colors panels; Windows uses modeless windows
with native WinUI font and color controls. Text Color and Highlight Color
initialize from the current foreground and background, including transparent
backgrounds. While a chooser remains open, caret/selection changes in its
invoking view refresh its values and editing target after a 150ms coalescing
delay. Invoking the command from another view explicitly retargets the panel;
unrelated document or pane activation alone does not retarget it.
Programmatic refreshes do not mutate source or create undo entries. A gesture
resolves the latest target before committing, so it cannot act on the previous
caret during a pending refresh. Unsupported formats or modes remain visible
but cannot apply direct formatting. All exposed Format actions have handlers;
availability follows format capabilities and current selection/mode.

The Windows frontend uses `Control-0` through `Control-5` for the same
paragraph/heading assignments. Heading 6 is menu-only because `Control-6`
also spells Vim's `Control-^` and must reach the command interpreter.
Menu validation follows the focused pane and
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

`Convert to` and `Reinterpret as` dispatch the distinct portable format
operations above. Both disable the current source family, including its Source
display variant, and all targets are disabled without an attached document.
Read-only buffers retain these in-memory operations; their external write
restriction remains unchanged.

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
Rich-formatting actions are disabled for Text and Code; `Convert to` offers an
explicit conversion into a format that supports them. Style and
formatting items show a checkmark, mixed state, or no mark as appropriate.
Character and Paragraph menus reserve the same mark column for every item,
so labels align whether or not the item is checked. Active named styles remain
checked when menu validation refreshes their command state.
Choosing a character style clears direct character declarations and inline
traits on the selected range before assigning the style, in one undoable source
transaction. Rechoosing the same style also clears overrides. With only a caret,
the choice replaces pending direct formatting for future typing without changing
existing text. Default Paragraph clears the named assignment and direct traits.
Unspecified properties continue to inherit paragraph and semantic Link defaults;
explicit named declarations override them. Link targets and unselected source
remain intact.
In Code mode, check the named syntax style at the caret, or the single style
shared by the selection, using the currently displayed syntax runs. Check the
specific assigned style rather than its linked ancestors; unstyled text and
unresolved syntax names use the current paragraph appearance. A selection spanning different
named styles has no single checked character style. Code's paragraph menu
checks the current paragraph style. These checks describe the document, not
the style currently selected in the style editor, and never trigger parsing.
Code queries use intersecting cached syntax spans and indexed text boundaries;
a large selection must not enumerate all hard lines just to open a style menu.
The generic Edit Styles command remains unmarked even when its initial target
is the current base style.
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
formatting area, bordered live preview, and bottom
action row. It is a composition reference rather than a behavioral or
pixel-exact template. Use native AppKit controls, metrics, typography, focus
rings, accessibility behavior, and current macOS window appearance. Do not
copy Word's `Format` section picker, template/Quick Style/automatic-update
checkboxes, or `Cancel` and `OK` buttons.

#### Window behavior and ownership

These document-target rules also apply to the explicit global Code stylesheet
target with the ownership and persistence exceptions in "Code format and
syntax highlighting". Global Code style editing never retargets implicitly to
a document stylesheet and never creates a document source transaction. Following
a Code view selects definitions within the global sheet; it does not change
their ownership or move their edits into document history.

- Although it is colloquially a dialog box, the style editor is a modeless
  auxiliary window or panel. It is never an application-modal dialog or a
  document-modal sheet. The user can focus and edit any document while it is
  open.
- Its title is **Styles**, with the compact utility-panel title bar and window
  buttons used by the native font picker. Formatting controls and preview have
  no section headings. There is no textual resolved-attribute summary. Omit the inherited-formatting
  instruction, live-apply footer text, and separator above the Close button;
  relevant availability and error messages remain visible when needed.
- **F8** opens the style editor through the same **Edit Styles…** action,
  without Command, Control, Option, or Shift. It works in Normal and Insert
  modes without inserting text or changing the editing mode.
- Exactly one style-editor window exists application-wide. Invoking any
  `Edit Styles…` action while it is closed creates it. Invoking one while it is
  open brings the existing window forward, retargets it to the invoking
  document or explicitly requested global Code sheet, and selects the requested
  style by stable style ID.
- Every general **Edit Styles…** action, including **F8**, **Paragraph**, and
  **Character** menu actions, immediately selects the style at the invoking
  view's cursor using the same rules as subsequent caret following below.
  Reopening an existing editor also reselects that current style. Choosing an
  individual style definition from a menu or another explicit Edit Style action
  selects that requested style.
- An editor opened from a document view follows subsequent logical caret or
  selection changes in that invoking view. Select its current non-default
  character style when there is one single such style; otherwise select its
  current paragraph style. For a selection spanning multiple character styles,
  use the paragraph result; if paragraphs are mixed as well, use Base Paragraph.
  Never choose an arbitrary first style from a mixed selection. Merely focusing
  or moving in another document does not change the editor's target or following
  context. The target-closure policy below remains applicable.
- Both native frontends coalesce caret following until half a second has elapsed
  without a logical caret or selection change. Schedule a one-shot timer only
  after an actual change and restart it on subsequent changes. Defer both the
  current-style query and the control reload, so rapid dragging does neither.
  An unchanged presentation does not schedule or extend the timer. Opening or
  explicitly reopening the inspector selects immediately. Explicit style
  navigation, retargeting and closure cancel pending following work; idle
  inspectors do not poll for styles.
- An explicit Style picker or hierarchy-navigation choice remains selected
  until the followed view's caret or selection actually changes. Scrolling,
  repainting, syntax publication, style edits, and document or stylesheet
  revisions or layout generations alone MUST NOT override that choice. Following
  uses the core's current named-style query and snapshot validation, including
  existing Code syntax runs; it MUST NOT trigger parsing, wait for syntax, scan the whole
  document, or infer named assignments from displayed font attributes. Following
  a style changes presentation only and uses the same pending-edit validation
  as explicit style navigation.
- The global Code editor is opened through an explicit edit action from a Code
  view. It follows that view without changing global stylesheet ownership.
- The window has a **Close** button at the bottom trailing edge. It has no
  **Apply**, **Cancel**, or **OK** button because valid changes are already
  applied. The standard window close command and `Command-W` have the same
  effect when the style editor is key. Closing never rolls changes back.
- The window coordinator, current document/global target identity, and selected
  stable style ID belong to `src/mac`. Style definitions and mutations remain
  owned by core. The frontend never keeps a private editable copy of a style sheet.

#### Layout and common controls

From top to bottom, the content is:

1. a properties section containing:
   - **Style**, a pop-up that selects a style in the target document and groups
     Paragraph, Character, and Internal styles, in that order. Internal styles,
     including Incremental match, appear only in the final Internal group even
     when their underlying style type is Character;
   - **Name**, an editable text field;
   - **Style type**, a read-only value showing Paragraph or Character; and
   - **Based on**, a pop-up for the style's parent with a trailing **↗** button;
   - **Next paragraph**, a pop-up for paragraph styles with a trailing **↗**
     button;
2. a native macOS tab row immediately below the name/base-style section, with
   **Character** and **Paragraph** tabs;
3. the controls for the selected tab;
4. a bordered, live preview using the real core style resolver and Core Text
   shaping path;
5. a bottom action row containing the **Close** button.

Style type is immutable after style creation. The Based on picker contains only
parents allowed by the selected style's namespace and role and excludes the
style itself and its transitive descendants. It cannot create an inheritance
cycle. Base Paragraph has no parent and cannot be deleted or have its role
changed. Character styles offer Default Paragraph as their default parent;
this clears the parent link and has no editable definition to navigate to.

Each **↗** button selects the referenced style in this same editor by stable
ID, allowing the user to traverse the hierarchy. Navigation commits any valid
pending property edit through the normal editing path, but does not change the
relationship or create a source/settings edit merely by selecting a style. A
fixed parent can still be visited. Disable the button when there is no distinct
applicable target, including Next paragraph's Same Style choice. Give the
buttons accessible action names and tooltips identifying their destinations.
Top-section labels MUST be vertically centered with their fields and pop-ups,
including rows with auxiliary buttons, at supported window sizes and in light
and dark appearances.

Each property has an unlabeled checkbox immediately to its left, with tooltip
**Override inherited**. Unchecked means no declaration at this layer. Its native
controls are disabled and entry fields are empty, including font family and
size. Clicking a disabled property control checks its override box and then
performs that same original click, such as opening the font list or selecting
Bold. The initial activation and action form one undo gesture. Checking a box
starts from the resolved value; unchecking removes the declaration and restores
the inherited presentation. Explicit normal weight, no decoration, zero spacing,
and transparent color remain distinct from inheritance. Unsupported properties
remain disabled and cannot activate through a click. Clicking a property caption,
unit label, or icon only enables the property; it MUST NOT forward a native
control action or open a field, menu, or color panel. A missing inherited
background activates as explicitly transparent.

Base Paragraph supplies every effective property. Its override checkboxes are
always checked and disabled, while its supported value controls remain editable.
Its Next paragraph is always Same Style, with the popup and navigation button
disabled. Opening this editor does not materialize sparse source declarations.

#### Character tab

The Character tab edits Character declarations. It is enabled for Character
and Paragraph styles. For a Paragraph style, these controls edit the
paragraph's default character declarations rather than assigning a separate
Character style.

Expose controls for these Character properties (language remains a core/source
property without an editor control):

- ordered font-family and fallback requests;
- font size in layout units, aligned with the font-family and face controls
  without a visible **Size** label above it; retain its accessible control name;
- a native font-face picker (Regular, Light, Bold, Italic, etc.) and separate
  Bold and Italic toggles, with no generic numeric weight/slant fields;
- native foreground/background color wells, including Default/Inherited;
  clicking either well opens the full native macOS **Colors** window directly,
  initialized to the current effective color, including custom colors and
  transparency. The native current-color preview, controls, and saved palettes
  remain available. Merely opening the picker must not change an already
  enabled property or create an undo entry;
- underline and strike decoration;
- writing-direction override;
- an original SVG feature button opening the selected font’s supported OpenType
  feature menu, with checkmarks and the same catalog as Format > OpenType Features;
- letter spacing; and
- exclusive Superscript and Subscript buttons labeled **x²** and **x₂**, with
  one shared **Override inherited** checkbox. Both off explicitly means Normal;
  clearing the checkbox inherits script position.

Font family/fallback and native face/base weight have independent override
checkboxes. An explicit face weight remains visible and editable when its font
family is inherited; clearing the family does not remove that weight override.

Character controls form compact grouped rows, with original consistent SVG
icons and separate B/I/U actions. Paragraph controls use corresponding alignment,
indentation, and spacing groups. The reference images guide density and grouping;
the interface must not be a tall generic attribute list. **Text Color**,
**Background Color**, and **OpenType** captions appear above their controls,
like Tracking and Script. The B/I/U/strike buttons reserve the same caption
space so their override checkboxes align vertically with the color checkboxes.
Separate adjacent property sections on a row with one em before each subsequent
checkbox; keep each checkbox close to its own control.

Font face establishes the base weight/slant. Bold is a separate portable semantic
property: add 300 to the base weight (capped at 1000), then choose the next
available face at least that bold, falling back to the strongest available face.
Italic selects an intrinsic italic face when possible. Unsupported traits use
native synthesis. Applying or removing Bold must retain the chosen base face.
Source adapters preserve this distinction through their owned style metadata,
with conventional interoperable bold fallback where necessary.

Changing the primary font family retains the current face's style name when
that exact style exists in the new family. Otherwise select Regular or an
obvious regular equivalent such as Normal, Roman, or Book; if none exists,
select the family's first available face. Commit the chosen face identity and
its base weight/slant together as one undoable choice, preserving the ordered
fallback tail and the separate Bold setting. The face picker, committed style,
live preview, and document rendering must agree. Repeated native selection and
editing-completion notifications must not reset a newly chosen face. Font
discovery must not mistake Core Text's substitute for an unavailable name as
that requested family's catalogue, and resolving an explicit face must retain
its identity when weight/slant are unchanged, including width variants that
share the same weight and slant.

Font-family fallback order requires an ordered editor rather than a single-font
field. Primary and fallback font-name dropdowns request 20 visible font rows;
native AppKit may constrain their height to available screen space. Native font
and color panels remain modeless and update the same selected style while owned
by the inspector. They must not become alternate persistence or undo authorities;
opening a direct-formatting panel transfers ownership from the inspector.

#### Paragraph tab

The Paragraph tab is enabled only for Paragraph-role styles. It is visibly
disabled for Character styles and cannot retain keyboard focus
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
applicable value form the committed declaration. In the editor, `normal` is the
canonical presentation of a 1× multiplier: its numeric field remains enabled,
shows `1`, has the trailing `×` unit, and steps by 0.1. A relative value other
than 1 is presented as `multiplier`; changing the field or stepper away from 1
selects `multiplier`, and returning it to exactly 1 selects `normal`. Explicitly
choosing `multiplier` while `normal` is selected starts at 1.1. Merely displaying
an existing source declaration does not rewrite it. Every line-spacing value is
rounded to the nearest 0.1 at the control boundary and displayed without a
fractional part when that fraction is zero. `at-least` and `exact` show `pt` and
their native up/down click changes the value by 1 pt. These controls fit the
existing fixed inspector width; make the adjacent space-before and space-after
fields narrower instead of widening the window.

Numeric style fields have native up/down steppers on their right edge. A
step uses the displayed resolved value and creates an explicit declaration;
the override checkbox can restore inheritance. Invalid drafts disable the stepper without
committing. Indents, paragraph spacing, and tracking retain
their supported signed ranges. Held autorepeat is one continuous undo gesture.

#### Live application, preview, and undo

- Every valid control change immediately issues a typed style-definition
  intention against the exact current core snapshot. Successful reverse
  projection commits through the normal verified source transaction path and
  updates every view of the document without waiting for the window to close.
  For the global Code target, the intention instead updates the application
  stylesheet authority and all Code views through a checked global-style operation.
- The preview is not a private draft. It renders the currently committed style
  after cascade resolution. A Character style preview shows the style in
  representative surrounding text. A Paragraph style preview shows preceding,
  current, and following paragraphs so font, indentation, alignment, line
  spacing, and before/after spacing are visible together.
- A pop-up or button choice is one undo unit. The native Colors window commits
  each completed color gesture as one undo unit. Continuous gestures such as
  stepping or scrubbing apply live but coalesce into one undo unit per gesture.
  Contiguous typing in a text field applies each valid change live
  while coalescing according to the core's text-input undo grouping rules.
- An incomplete or invalid intermediate text-field value remains visibly local
  to that control and does not mutate core. Show an inline validation state and
  retain the last committed style value until the entry becomes valid.
- If an adapter rejects a style edit as unsupported, ambiguous, stale, or
  policy-dependent, commit nothing, restore the affected control from current
  core state, and present the structured diagnostic. Other controls and the
  document remain usable.
- Undo, redo, source reprojection, or another frontend action may change the
  selected style's definition while the window is open. The editor observes
  style-sheet revision changes and refreshes its fields, inheritance state,
  and preview from core without manufacturing another edit.

#### Selection validity and deletion

Deleting any non-base style that is in use reassigns its content to the
Base Paragraph assignment or no character assignment (Default Paragraph),
rather than to the deleted style's parent. Definitions, assignments, and supporting source metadata change
atomically and undo restores all of them. Generated heading/list definitions
must not silently reappear after reparsing; an adapter may persist an explicit
deletion marker in its owned schema. Base Paragraph remains undeletable. Required tests cover local edits and reopen after deletion, source
locality, and deeper generated list levels.

The global Code target instead follows the name-reference and persisted
suppression rules in its section: no source assignments or document history
are rewritten. All target identity checks below use the global sheet identity
when appropriate, and closing a document does not close or retarget that editor.
If its followed Code view closes, detach that following context and retain the
selected global definition; do not start following another document implicitly.

The selected style is tracked by document-or-global target identity and stable
style ID, never by menu index, name, or stale array position. On every style-sheet
update and before sending an edit, the window revalidates that identity against
the latest snapshot.

If the selected style was deleted from another action, reverse projection, or
history navigation, the editor must not apply a pending callback to the deleted
ID or to whichever style reused its former list position. It immediately
selects **Base Paragraph** in the same target, switches the style type and tab
enablement accordingly, and reloads every control and preview from that style.
Base Paragraph is the guaranteed default paragraph style and is undeletable.

If a document target closes, the editor retargets to Base Paragraph in the
current key document when one exists; otherwise it remains open in a disabled
no-document state or closes according to normal macOS auxiliary-window policy.
It must never continue editing a closed document through retained UI callbacks.
An editor targeting the global Code sheet remains valid when a document closes.

Required macOS integration tests cover single-window reuse and retargeting,
continued document editing while the window is open, live application and undo
grouping, Character/Paragraph tab enablement, inherited versus explicit values,
base-style parent restrictions, external undo/redo refresh, deletion fallback
to Base Paragraph, stale callback rejection, and target-document closure. Also
cover role-specific initial selection, following a non-default character style
and falling back to paragraph styles, mixed selections, explicit picker and
hierarchy choices surviving unrelated refreshes, no following of unrelated
documents, and Code-view launches retaining global ownership and the correct
following context. Large-document following queries must remain bounded and
must not request new syntax work.

### User key mappings

`map {lhs} {rhs}` defines recursive mappings for Normal, Visual, Select, and
operator-pending input; `noremap` defines the same modes without remapping the
replacement keys. Full command names with `n`, `v`, `x`, `s`, `o`, `i`, or `c` prefixes
select Normal, Visual plus Select, Visual only, Select only, operator-pending,
Insert/Replace, or command-line input.
`map!` and `noremap!` select Insert/Replace and command-line input. `unmap` and
`mapclear`, including these mode prefixes and bang forms, remove mappings.
Interactive definitions apply to the current buffer and its views; startup
definitions initialize every buffer. Mapping listing, command abbreviations,
mapping attributes such as `<expr>`/`<buffer>`, and user abbreviations remain
unsupported and report diagnostics.

Key notation accepts ordinary Unicode characters, `<Esc>`, `<CR>`, `<Tab>`,
`<S-Tab>`, `<BS>`, `<Del>`, navigation keys, control characters, and `<F1>` through
`<F35>`. Function and navigation keys accept combined `S-`, `C-`, `A-`/`M-`, and `D-` modifiers
for Shift, Control, Alt/Option, and Command. `<Space>`, `<lt>`, `<Bar>`, and
`<Bslash>` express literal separator characters; `<Nop>` is an empty replacement.
Trailing replacement spaces are significant. Unsupported notation is diagnosed
instead of installing an unusable mapping. Native bare F8 and Shift-F10 retain
their existing Styles and context-menu shortcuts.

For example, `map Y y$` uses the existing line-mode-aware `$` yank semantics,
preserving explicit counts and registers. `map <C-F2> :sp` enters the Ex prompt with `sp` ready to
edit; `map <C-F2> :sp<CR>` executes the split. Mapping expansion uses the ordinary
command and layout pipeline, with fresh layout when edits require it. Literal
command operands and quoted input bypass mapping. Recursive expansion is bounded
and reports an error on exhaustion; nonrecursive replacements bypass mappings.
Multi-key prefixes wait for further input, with a one-second native timeout that
selects a shorter complete mapping or releases unmatched keys. Escape cancels a
pending prefix. Mapping-driven edits are grouped for undo, and their normal
register, dot-repeat, and macro behavior remains testable in the portable core.

### Explicitly deferred compatibility

The following are outside the initial command commitment unless a later change
adds them here: Vimscript/Vim9script, user abbreviations, plugins,
terminal jobs, shell filters and `:!`, tags, quickfix, diff mode, folding,
spellchecking, Vim tab pages and side-by-side splits,
sessions/viminfo, remote server commands, and full Vim option/regex parity.
Architecture must not gratuitously prevent these, but do not build speculative
subsystems for them now.

The Code syntax-provider system, nine bundled Tree-sitter language families,
versioned query compatibility, and supported Vim syntax-loading/detection
profiles specified above are required exceptions. They do not imply general
Vimscript/Vim9script execution, Neovim Lua plugins, arbitrary runtime
autocommands, syntax-driven folding/concealment, or a change to Viem Regex v2.

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
        Code syntax input, providers, language regions, and highlight snapshots
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

Projection, syntax analysis/queries, segmentation, shaping, wrapping, and height
refinement may run on a bounded worker pool. A job captures only immutable inputs,
including:

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

- A projection, syntax coverage, or positioned layout result is installed only
  when its complete source/projection revision and relevant configuration
  generations match the current target.
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

Windows pre-layout extends beyond the installed viewport's overscan in the
direction of scrolling, initially forward. Its target is three times the
observed number of hard lines on screen, capped at 128 lines and half the
regional cache's line budget. The coordinator captures at most 32 hard lines
and 16 KiB of text per job; individual paragraphs beyond that byte limit retain
their existing on-demand bounded layout path. A single application-wide worker
uses an independent DirectWrite/Win2D shaper, sharing only immutable leased glyph
resources with the UI provider. Low-priority UI callbacks capture and install
chunks; they do not shape text. No cache budgets are increased. Cache-only
installation MUST NOT change the visible snapshot, caret or viewport origin.
Edits, configuration/metrics changes, direction reversal, distant jumps and
view closure cancel obsolete work. Current dependency identities and the scroll
band are checked again before installation. Once the band is ready, no timer or
polling task remains; caret-only motion does not schedule it again. A viewport
generation admits at most 32 chunks, also bounding retries under cache pressure.

Scrolling inside an already materialized viewport/overscan region MUST retain
its exact layout snapshot and pending background work when their dependencies
are unchanged. It updates the presentation origin without rebuilding geometry;
sparse horizontal coverage and newly exposed regions still require exact demand
layout. Windows caches native drawing commands for the visible area plus one
screen in the scroll direction and translates their replay on covered vertical
scrolls. Viewport-specific whitespace markers retain their separate invalidation
requirement. Snapshot, size/DPI, paint/theme and relevant option changes still
invalidate drawing commands.

Initial layout MUST use the application's final canvas padding. View creation
accepts that padding before measuring text; reapplying unchanged padding to a
current snapshot is a no-op. An unmeasured document starts with a small paragraph
band and extends until the first viewport has exact coverage, rather than using
one-row height estimates to shape several prose screens upfront. Windows opens
command-line files without first attaching an empty editor, parses a newly opened
document on a worker before attaching its native view, and starts with the
horizontal scrollbar collapsed so wrapped documents keep their first-frame size.
Adjacent compatible LTR glyph clusters share native drawing calls; direction,
font, paint, baseline, color-font and decoration boundaries preserve rendering
order and positioning. Temporary drawing batches remain bounded.
Page-refill estimates convert missing visual rows to hard lines using observed
wrapping density, then retry against exact coverage if needed. Missing visual
rows MUST NOT each schedule an entire wrapped paragraph. Immutable snapshot
copies used by commands share glyph/caret geometry; revision rebinding or
geometry changes detach shared storage without changing older snapshots.

**TODO(macOS): Hook up bounded background pre-layout.** The shared Rust planner,
cache-only installer and C ABI are implemented, but `EVCoreViewSession` does not
schedule them. Connect the Mac view lifecycle and viewport updates to a bounded
worker scheduler, with independent Core Text response storage and compatible
shared render-resource ownership. See the implementation checklist in
[the background layout notes](docs/windows-background-layout.md#todo-macos-connect-the-native-scheduler)
and the Windows reference in `src/win/Core/BackgroundLayout.cs`. This remains
unfinished until native Mac cancellation, rendering, memory and paging tests pass.

Cancellation is checked at bounded parse checkpoints, projected leaves,
shaping fragments, and hard-line/wrap units. Obsolete jobs must release retained
snapshots promptly enough that continuous typing cannot keep an unbounded chain
of old source revisions alive. Cancellation does not make a partially produced
result observable.

The coordinator retains separate cooperative cancellation tokens for one current
visible-layout job and one cache-only pre-layout job per attached view. Ordinary
foreground layout publication MUST NOT cancel pre-layout with unchanged
dependencies, and cache-only completion MUST NOT supersede a visible-layout job.
A replacement of either kind becomes current, and only then
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
only once. Paging replaces the visible snapshot with a bounded band around the
viewport and required command endpoints. Reusable paragraph geometry remains
in the separately bounded regional cache, subject to its eviction limits.
The low-level command API continues to expose typed demands to callers
that schedule layout themselves.

Regional layout requests reuse exact cached hard-line geometry when document,
view configuration, measurement environment, metrics generation, and render
resource policy still match. Requests retain only immutable cache hits inside
their requested band, never the whole cache or live height index. Reused caret
geometry receives the new layout revision; paint is resolved for the current
request. The existing line, row, and byte budgets also govern these entries.
Newly exposed lines still use bounded on-demand layout; paging must not trigger
eager whole-document shaping or expand cache limits.

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
- **Syntax package/execution provider**: locates and retains compatible native,
  imported Vim, or optional Wasm resources and supplies declared worker
  mechanisms; core owns language selection, analysis, budgets, coverage, and
  named-style output. No platform UI dependency enters portable syntax policy.

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
  explicit owner, thread rule, and metrics-generation validity. A shaped
  fragment holds a shared native-resource lease; cache/snapshot eviction releases
  that lease, and the last lease releases its native resources. Measurement
  provider ABI v3
  retain/release callbacks provide this ownership without native pointers in
  document or command APIs. Lease release may originate on any thread and after
  view removal; the frontend schedules native destruction on its required
  executor. Borrowed drawing exports remain tied to their exact layout.
- The provider announces a new metrics generation when font availability,
  fallback, feature resolution, or scale makes previous measurements stale.
- Core does not assume calls must run on the UI thread. The macOS
  implementation documents any stricter rule and schedules accordingly.

Unicode line-break opportunity detection belongs in portable core code so all
frontends make the same wrapping choices. Shaping supplies the actual advances
and legal cluster boundaries. Locale-sensitive hyphenation is deferred unless
added with a portable policy and provider capability.

The portable line breaker implements the default Unicode 15.0 UAX #14 rules
directly, using compact character-property tables generated from the published
Unicode data. It does not use an external line-breaking implementation or a
copied implementation table. The optional numeric-expression tailoring in
UAX #14 section 8.2 is not enabled. Unicode version changes require deliberate
data regeneration and conformance review. Streaming state must remain bounded;
partial layout resumes at a verified break checkpoint rather than guessing
context at an arbitrary text slice.

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
  spacing, or script-position changes invalidate affected shaping and downstream wrap
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
- The portable layout layer owns vertical scroll bounds and row visibility.
  Keyboard paging, wheel/drag scrolling, and native scrollbars use that same
  margin-inclusive document extent. Scrollbars consume the core's exported
  maximum and exactness; estimated heights never become authoritative clamps.
  Repeated paging at either endpoint is stationary: the top is zero and the
  bottom includes final-row ink, paragraph spacing, and bottom padding.
  Explicit viewport commands finalize their own scroll position; a subsequent
  generic caret reveal must not override it, even when the cursor moved.
  If trailing spacing exceeds the viewport, explicit scrolling can place the
  caret offscreen, just as scrollbar movement can. Caret movement and editing
  continue to use the minimum-reveal and oversized-row policies below.
- Cursor reveal scrolls the minimum needed subject to configured context.
- Typing and asynchronous layout/syntax completion preserve the visible editing
  row's screen baseline; configured motion context does not force a new scroll
  while that row is fully visible. Fill missing local layout coverage before
  restoring the anchor, so a regional cache boundary cannot bottom-align the
  row or masquerade as a document edge. Actual clipping permits only the minimum
  reveal, with the oversized-row and offscreen-caret policies defined for Code.
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
  trigger repeated reflow. The legacy vertical scrollbar reserves a stable right
  gutter; horizontal controls overlay the canvas in both native styles and never
  reserve a bottom strip. Layout, painting, pointer hits and input-method geometry
  use the same full-height canvas up to the status line.
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

### macOS style inspector

The style inspector presents its metadata fields without a redundant
"Properties" heading. Keep the field labels and Character/Paragraph tabs.

### macOS pointer selection performance

The shared Rust core reuses current visible layout during pointer selection,
bounds resize work using previously visible hard lines, and repairs retained
position state during native Undo/Redo. The Swift frontend additionally:

- Keeps one bounded immutable geometry/decorations and paint export per view,
  validated against the full layout identity: view, document, document revision,
  layout revision, configuration generation, measurement environment, and
  metrics generation. Live extent metadata is refreshed separately. Whitespace
  exports also validate the current viewport; stale explicit requests still
  fail even when their former export is cached.
- Reuses visible formatted text slices against their layout and formatted
  snapshot identities, and composition slices against their overlay identity.
  Native selected-text queries use those same formatted slices when covered;
  larger or offscreen text queries retain the checked on-demand path. Selection
  and caret exports update independently on every presentation refresh.
- Validates the current core identity and viewport before pointer hits,
  scrolling, and drag autoscroll. Missing or retired font/metrics geometry is
  rebuilt, and presentation is refreshed only when the retained snapshot no
  longer matches. This validity check does not advance search or replay input.
- Invalidates changed selection rectangles and old/new custom caret cells when
  text drawing inputs are unchanged. Text ink, selection, whitespace, and caret
  drawing respect damage clips while preserving their drawing order. Layout,
  viewport, theme, appearance, backing-scale, and composition changes force a
  full redraw. The frontend retains overlay geometry, not a second pixel cache.

`EVPresentationCacheIntegrationTests` measures core pointer work, presentation
refreshes, immutable export copies, and Core Text shaping separately using
`AGENTS.md` and a 20,000-line fixture. Repeated visible selection MUST NOT trigger
new shaping, layout identities, or immutable export/text copies.
`EVPointerDrawingPerformanceTests` compares partial selection frames with fresh
full rendering pixel-for-pixel and measures drawing separately. Cache and
invalidation coverage includes edits, Undo/Redo, resize/zoom, font metrics,
paint styles, viewport changes, theme/appearance, backing properties, whitespace,
and composition. The million-line bounded-projection test remains required.
Run native tests through `scripts/test-mac.sh` to rebuild the matching Rust core
and isolate the test profile from user preferences. Native macOS measurements
establish behavior on that host; Windows timings are not a macOS latency claim.
Recorded validation and representative timings are in
[`docs/mac-pointer-performance.md`](docs/mac-pointer-performance.md).

Mac menu handling uses native menu-validation callbacks rather than an explicit
all-menu validation pass after each editor input. The Windows menu optimization
does not require a direct Mac counterpart.

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
  first responder in the key window of the active application and the applicable
  mode uses a vertical caret. Set it to
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
the portable requirements in "Modes and caret". The Windows frontend uses
the same portable appearance and centralized color preference but chooses its
own native or custom implementation for the thin vertical caret.

## Windows frontend requirements

`src/win/Viem.Windows.csproj` builds the unpackaged x64 C# / WinUI 3 frontend.
`scripts/build-win.ps1` builds the matching Rust DLL and Windows executable;
`scripts/run-win.ps1` only launches the existing Release build and reports
the build command if it is missing. Use `scripts/build-win.ps1 -Configuration
Release` to rebuild; add `-Offline` for already-restored dependencies.
Release packaging runs `dotnet publish` with ReadyToRun compilation, including
the managed framework projections, at the existing executable location. A plain
`dotnet build -c Release` compiles optimized IL but does not perform that publish
step. Debug builds retain their normal JIT/debugging behavior.
Python 3 verifies and packages the shared Vim runtime on every build and
publish, including direct MSBuild builds and publishing without rebuilding.
`scripts/test-win-vim-runtime.ps1` verifies packaging, relocation, legacy
settings migration, native highlighting, and missing-resource fallback.
`scripts/test-win.ps1` checks generated ABI declarations and runs the native
integration harness with an isolated profile. Development instructions and
ownership boundaries are in `src/win/README.md`. The screenshots under
`docs/mac_references` remain the visual reference for editor chrome, stacked
panes, the bottom command prompt, and the modeless style inspector.

The window has a Windows menu bar below its title bar. A native toggle button
immediately to the left of the caption controls shows or hides the menu and
persists the preference. Panes stack vertically, with native resize dividers,
independent scrollbars, status bars, cursors, selections, and view options.
The menu bar matches the title-bar background. Its visibility toggle blends
into that background when off and uses WinUI's neutral default control fill
when on, with native hover, pressed, and keyboard-focus feedback.
Native menus, dialogs, color pickers and font controls use WinUI compact sizing
through the application-wide `DensityStyles/Compact.xaml` resource dictionary.
This is a keyboard-and-mouse writing application; all application windows and
dialogs MUST inherit these resources rather than override them with touch-sized
controls. Native system file pickers retain their platform-owned presentation.
Document typography is independent of control density.

Document windows retain the last normal size and position in
`windows.documentFrame`, matching the macOS lifecycle contract. On Windows
these are physical desktop coordinates; monitor-relative work areas must be
translated to desktop coordinates before fitting. Restore before activation,
move onto the best available monitor before shrinking oversized dimensions,
and cascade later new windows by the native caption height. Reactivation does
not reposition an existing window. Minimized, maximized and fullscreen bounds
do not replace normal geometry. Coalesce persistence until move/resize becomes
idle, flush the final normal frame on close, and avoid preference-change
notifications that would invalidate editor layout while saving placement.

Settings is a modeless window with View, Theme and Editing categories in
a fixed left sidebar. Theme follows `docs/mac_references/settings_theme.png`:
Paper/Midnight presets, a live writing preview with caret and selection,
grouped Editor and Status bar colors, status font/size, and Restore Defaults.
Validated edits persist through the shared portable profile schema;
Windows-specific settings stay in `windows`.

Unmodified `F8` opens or raises the modeless style inspector outside literal-next
input. Menu and keyboard entry must leave focus in the inspector, without an
always-on-top flag. The inspector is not resizable or maximizable. Its compact,
centered Character/Paragraph tabs share one fixed-height formatting area; the
window fits the form, preview, and bottom buttons without spare bottom space.
Complete the populated inspector's measure/arrange layout while hidden before
fitting its native window. Opening must not expose intermediate sizes, retain
an initial template measurement's excess height, or repeatedly resize in
response to layout events.
Opening, reopening and idle caret following use the macOS current-style rules
above, including mixed selections and displayed Code syntax styles. When opened
from a Code view, the inspector targets the global Code stylesheet while
following that view's caret context. Windows color wells and their
popups resolve an undeclared emergency foreground through the active editor
theme, while retaining explicit and inherited authored colors and alpha.
Enabling such a foreground override copies the theme color. Opening or closing
an unchanged picker creates no edit. Color changes update the document, swatch,
and committed inspector preview live. Rapid changes coalesce at a 33 ms cadence;
idle pickers schedule no work. One popup session forms one document undo unit.
Global Code color changes update all Code views and persist live. Successful
changes do not rebuild the inspector, resize its popup, or write rounded RGB
values back into the active picker. Closing or retargeting flushes the latest
color to the original target and disconnects pending work. External document
edits or caret commands dismiss the gesture without replaying queued colors.
Opening a color picker cancels pending caret following so it cannot retarget
or rebuild the edited style during a color gesture.
The picker uses the horizontal layout, compact input controls, and a 256-pixel
square spectrum with numeric and alpha inputs. Its native flyout can extend
outside the inspector's bounds without clipping to the dialog.
Top labels are close to their fields and vertically centered. Based on and
Next paragraph have accessible ↗ buttons that navigate by stable style ID
without changing the relationship. Parent choices exclude inheritance cycles.
Inherited entry fields are empty; enabling an override starts with its resolved
value. Base Paragraph's override boxes stay checked and disabled. Character
styles disable the Paragraph tab. The paragraph pane uses alignment icon buttons
and aligned columns for indents and spacing. The font ellipsis edits the ordered
fallback family list. Code Styles includes Restore Defaults, which replaces and
persists the shared defaults; a write failure restores the previous global
styles. Document Styles does not offer this global action.
Font families are sorted using the current
culture. Both style and direct-font pickers expose installed font variants.
Variants retain their PostScript name, weight and slant, preserving a named
style's fallback families and grouping a face change into one document undo.
Family changes preserve a matching face name where possible, otherwise choose
a regular face. Unavailable/custom faces remain unresolved instead of silently
selecting the first variant. DirectWrite resolves persisted face names back to
their installed family and width; weight and slant use the stored declarations.

Startup MUST NOT enumerate every installed font face or load font-picker lists
before the first editor draw. Resolve document fonts through indexed DirectWrite
family/PostScript-name queries, cache matches and misses, and load sorted picker
families only when a picker needs them. Windows generated text styles and
emergency shaping defaults request Segoe UI; `system-ui` resolves to Segoe UI.
SF Pro is a macOS default, not a Windows alias or built-in font choice: Windows
offers and resolves it only when installed. Explicit authored font requests
remain preserved, with unavailable families following normal fallback rules.
Missing/custom document fonts must not trigger a
whole-system face scan. Native tests cover this startup constraint and preserve
variant selection, fallback, and layout invalidation coverage.
The read-only system font index is shared across family lookups and independent
shapers, and may be prepared on a worker while the shell and document load.
Creating a new index for each font family is unnecessary startup work.
`scripts/test-win-startup.ps1` measures Release launches with isolated profiles,
optional copied settings and an optional reference executable. Opt-in tracing
starts at the managed entry point, records startup phases, JIT CPU time, first
draw and font discovery counts, and checks initial document geometry. Separate
first launches of newly built/relocated output from repeated launches; these
measurements do not measure compositor presentation or cold-boot disk latency.
Fixed startup JSON records use generated serialization metadata. Updating recent
files refreshes their menus without reapplying editor settings, while settings
edits merged from disk must still notify the editor.
Initial preferences are read and validated on a worker during native WinUI
initialization, then handed to the application before any window is created.
The worker must not construct UI controls or retain a core validation document.

The Windows status line follows the shared command-entry/output contract above:
an inverse-color prompt replaces the left group, while selectable read-only
output uses normal status colors and a close button. Both preserve the right
location widget and temporarily reveal a hidden status line. Prompt drawing
must refresh when Win2D resources or its measured width first become available.
Scrolling the caret outside regional layout must not abort status updates or
force distant layout; retain its location only while the document, cursor and
measurement configuration are unchanged. Native regression checks cover the
first colon, subsequent typing, command results, hidden status bars, output
scrolling/focus, and command entry in a scrolled large document.

Windows clipboard shortcuts are an intentional exception to Vim compatibility:
`Control-C`, `Control-X`, and `Control-V` always mean Copy, Cut, and Paste,
including during literal-next input. `Control-Shift-V` pastes plain text.
`Control-Q` remains the core's alternate Visual Block/literal-next binding.
`Control-S` and `Control-Shift-S` save/save as outside literal-next input.
`Control-Z` and `Control-Shift-Z` invoke the core's native Undo and Redo actions
outside literal-next input. They work during Insert mode as well as Normal mode
and use the same document history as the Edit menu; they never undo the hidden
input TextBox. These bindings apply only to Windows. The vi `u` and `Control-R`
commands and macOS shortcuts remain unchanged.
`Control-0` through `Control-5` assign Base Paragraph and Headings 1–5;
Heading 6 remains in the menu to preserve `Control-6` / `Control-^`.
Other vi control keys MUST reach the Rust interpreter: Windows MUST NOT claim
`Control-B/F` for formatting/find, `Control-I/U` for italic/underline,
`Control-N/O` for new/open, or `Control-A/W` for native editor commands.
Those native actions remain available through menus. AltGr remains text input;
function-key modifiers are preserved. Windows owns its system shortcuts.

The Rust core owns document state, editing, mappings, command prompts,
completion, formatting, history, wrapping, geometry and hit testing. The C#
bridge is generated from `include/viem_core.h` and `include/viem_startup.h`;
`src/win/tools/generate_bindings.py --check` must pass after ABI changes.
Win2D's DirectWrite layout shapes bounded unwrapped contextual fragments and
retains glyph resources through the core's leases. Drawing uses viewport
exports and never maintains a second editable document string. The small
native input TextBox is an input-method host, not the document authority.
IME marked text uses the core's composition overlay and explicit commit/cancel
protocol. Default font names resolve to installed Windows families; actual
glyph shapes and font fallback naturally differ from Core Text.
Localized fallback-font names, color-font classification, and face metrics are
shared across equivalent DirectWrite font faces, using native face-reference
equality rather than transient Win2D wrapper identity. Each shaper retains at
most 32 face descriptions and 1,024 exact single-glyph ink queries, in addition
to its leased render resources. Ink keys include size, glyph, advances, offsets
and bidi direction. Metrics/device changes clear these caches, and disposal
releases their retained faces. Workers own independent caches. Native tests
cover large-document reuse, invalidation, size changes and rendered pixels.
Keyboard regressions MUST also exercise the native WinUI input host, including
text entry, command keys and pane focus. Calling the Rust input wrappers alone
does not verify Windows event routing.

Pointer selection over already materialized visible rows MUST reuse exact
layout geometry instead of running visible reflow for each pointer event.
Windows caches exported geometry and text drawing commands against the full
layout identity. Selection and caret drawing remain independent; viewport,
device/DPI, theme and whitespace changes invalidate affected drawing commands.
Rebuilding text drawing commands shares native brushes by color within the
drawing pass instead of allocating a brush for every glyph cluster.
Adjacent compatible LTR glyphs share native drawing calls. Ordinary whole LTR
clusters use the captured native run's advances for their origin; split-run and
bidi clusters retain DirectWrite's region query. Native checks compare captured
origins with DirectWrite and compare batched drawing with individual glyphs.
Menu validation runs when menus are opened, not on each editor input event.
Regression checks MUST cover cache invalidation, large-document selection and
pixel equivalence between cached and freshly rebuilt selection frames.

Native save preserves the core's source bytes through synchronized temporary
files and atomic replacement. External-file checks compare saved content
hashes; a conflicting overwrite is reviewed. Named documents claim their own
recovery slots, write source snapshots after idle edits, and remove only owned
slots on document close. Recovery uses the Mac-compatible recovery envelope;
foreign slots remain untouched. One process per Windows user/profile receives
bounded same-user named-pipe launch requests, including the caller's working
directory. Relative paths, first-file/deferred arguments, `-o` and `+line` use
the portable launch parser.

### Known gaps from the macOS frontend

These are explicit limitations of the current Windows frontend, not changes to
the portable document or vi command contract:

- AppKit Services, the system menu search, Dictionary/Look Up, spelling and
  grammar panels, application-managed dictation, and macOS document Versions
  have no corresponding integration in this frontend. Their menu commands are
  omitted. Revert to Last Saved and Viem recovery are available.
- Page Setup and Print are omitted pending a Windows printing adapter; the
  AppKit printing implementation cannot be reused.
- Native controls have automation labels, but the custom document surface does
  not yet expose a complete Windows UI Automation TextPattern. Narrator text
  ranges, accessible editing, and macOS-equivalent text-service integration are
  not claimed. IME protocol checks do not substitute for testing every installed
  Windows IME or speech-input service.
- The modeless style inspector exposes inheritance, names, fonts, colors,
  decoration, tracking, script position, paragraph direction/alignment/indents and
  spacing. Font and color commands open persistent modeless WinUI control
  windows. Per-font OpenType feature discovery and the full Mac typography
  menus are omitted. Existing source OpenType features still participate in
  DirectWrite shaping. The inspector preview currently demonstrates font,
  decoration, script position, tracking, and alignment in surrounding text
  rather than the complete paragraph layout. Mac's click-through activation of
  disabled inherited controls is not yet implemented; use the override checkbox.
- Per-span explicit bidi overrides are retained in source but are not realized
  by the Win2D adapter; their inspector control is omitted. Unicode bidi and
  explicit paragraph direction are supported.
- Windows clipboard interchange writes Unicode text, HTML, and the lossless
  Viem private fragment, and reads text, private fragments, HTML and RTF.
  An outgoing RTF representation is omitted; applications supporting only RTF
  receive the plain-text fallback. HTML cannot exactly express RTF's
  minimum-line-height policy; Viem-to-Viem transfer retains the original data.

Changes to these gaps MUST update this list and the Windows integration tests
when the relevant mechanism becomes available.

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

The source/transform suite additionally includes Text, Code, Markdown, HTML,
and RTF samples as adapters are implemented, with alternate equivalent syntax,
comments/trivia, malformed and unknown constructs, legacy encodings, and
characters that cannot be represented in the original encoding.

Performance tests should instrument source bytes decoded and parsed, formatted
bytes projected, bytes segmented and shaped, hard lines wrapped, cache hits,
height-tree operations, and discarded stale tasks. Assert bounded work
structurally; avoid brittle wall-clock-only tests.

HTML replacement context uses a persistent, revision-bound source scope index.
Lexical inline scopes and anchor destinations remain distinct from recovered
semantic character and paragraph context. Ordinary replacement and continued
typing MUST NOT scan unrelated source or the untouched prefix of a long
paragraph to recover these contexts. Local index updates share untouched
branches and validate their exit context before reusing a suffix; grammar
changes without a verified regional boundary retain full projection validation.
The index participates in snapshot history and retained-memory accounting.

Replacement work measurements MUST include context capture before preparation,
scratch transactions, IME preparation, commit, and continued typing. Track actual
source materialization, decoding, tokenization, index traversal and maintenance,
and broader-parse causes separately from final candidate projection counters.
The implementation and measured limits are recorded in
[`docs/html-replacement-work-results.md`](docs/html-replacement-work-results.md).

### TODO: Compact document projections and measure large-file memory

**Open follow-up; the compact Text/Code implementation is measured below.**
Ordinary literal mappings now scale with bounded chunks and actual conversion
exceptions. Remaining work includes progressive cold construction, dense
exceptions, rich projections, and full native memory/latency validation.
Passing syntax-worker responsiveness tests alone does not close this TODO or
establish complete-editor memory bounds. Retain the portable ownership
boundaries and lossless behavior of every format while addressing the remaining
requirements.

**Implementation update (September 2026):** The compact literal pipeline,
regional Text/Code edits, sparse backing allocations, streaming hash/search,
shared and budgeted layout, render-resource leases, and byte-budgeted idle
syntax sessions are implemented. See `docs/large-file-memory-results.md` for
repeated allocation/process measurements and remaining limits. The original
baseline below remains historical evidence. This TODO remains open for
progressive first display, dense-exception/rich-projection bounds, and broader
native memory/latency profiling; compact eager opening does not satisfy the
progressive-display requirement by itself.

#### Recorded evidence and measurement limits

The baseline in [code-pipeline-performance.json](docs/code-pipeline-performance.json)
uses fixture revision 1, Rust 1.98.1 release builds, an Apple M1 Ultra
(Mac13,2), and 128 GiB physical RAM. See also
[the validation notes](docs/code-syntax-validation.md) and
`pinned_code_pipeline_performance` in
`src/core/coordinator/syntax/tests.rs`.

The benchmark reads `history_status().retained_memory_bytes` immediately after
document construction, before creating views, running syntax providers, or
accumulating edits. The recorded `history_budget_bytes` adds 64 MiB of explicit
headroom to that value. Subtract that headroom to recover the original estimate:

| Source fixture | Estimated retained document/history state at open |
| --- | ---: |
| 100 MiB UTF-8, mixed line lengths/endings | 15,672,729,263 bytes (15.67 GB / 14.60 GiB) |
| 100 MiB UTF-16LE, mixed line lengths/endings | 7,925,602,485 bytes (7.93 GB / 7.38 GiB) |
| 100 MiB Latin-1, mixed line lengths/endings | 15,746,207,923 bytes (15.75 GB / 14.66 GiB) |
| 1,000,000 short UTF-8 lines, 2,000,000 source bytes | 700,429,255 bytes (about 668 MiB) |

These are **conservative retained-heap accounting charges, not measured process
RAM, resident-set size, peak allocation, or OS physical footprint**. The
estimate includes source storage, the formatted projection, source hard-line
indexes, history metadata, and the accounting ledger itself. The ledger uses
allocation capacities and conservative allocator/alignment/hash-table
allowances; it deduplicates shared Arc/Vec allocations by identity. The audit
found no obvious large shared-subtree double count, but the estimate still
needs independent allocation and process-memory validation. It excludes
subsequently created view/layout and native syntax state, and it does not
measure transient allocation peaks during opening. Do not present the table as
an observed 8–16 GB resident-memory reading.

The large estimates occur with syntax both disabled and enabled. Both syntax
workers are deliberately suspended throughout this fixture. UTF-16LE has fewer
decoded characters than the mostly ASCII UTF-8/Latin-1 fixtures at the same
physical byte size; its lower estimate is not evidence of an inherently more
efficient UTF-16 projection implementation.

First display in these core/mock-shaper runs takes about 4.3–10.2 seconds for
the 100 MiB fixtures, and about 0.60–0.75 seconds for the million-short-line
fixture. The measured typing/newline/undo and distant two-view scroll paths are
regional, but cold construction remains eager. These timings are neither
native AppKit frame measurements nor proof of bounded peak memory.

#### Original sources of amplification (before the compact literal path)

- `encoding.rs` builds `DecodedText` with a `DecodedSpan` for each decoded
  Unicode scalar, including each ordinary ASCII character in valid UTF-8.
  `push_valid_utf8` therefore stores individual source/decoded ranges even when
  a whole source range has an identity mapping.
- `line_endings.rs` builds a normalized text representation and `LogicalUnit`
  records. `projection.rs::project_plain`, also used by Code's literal format
  projection, creates a `ProvenanceSpan` for each unit. The source-to-display
  relationship is thus represented at character granularity even for long
  unchanged text runs.
- `projection.rs::source_text_boundaries` creates forward/backward-affinity
  boundary records from each provenance span for reverse lookup. Together with
  the provenance index, these retain multiple range/position records per
  character. On the current 64-bit target a `ProvenanceSpan` alone contains two
  16-byte ranges, before reverse indexes, tree nodes, and bookkeeping. A mostly
  ASCII 100 MiB file can contain roughly 100 million characters.
- Hard-line/block records, paragraph/style metadata, persistent range-tree
  nodes, and retained-allocation ledger entries add further overhead. The
  million-short-line result warrants measuring these separately even after
  character-granularity mappings are removed.
- Opening constructs decoded and normalized strings, temporary per-character
  vectors, provenance and reverse-index collections, and final persistent
  structures. Some coexist during construction. Lazy compatibility accessors
  can also materialize flat copies of persistent text or range stores. Audit
  these lifetimes and callers; a small final retained representation alone
  would not prove a small opening peak.

The estimate's components have not yet been independently attributed with a
complete allocation/RSS profile. Treat the items above as inspected storage
mechanisms, not a measured percentage breakdown or proof that one optimization
alone accounts for the entire amplification.

#### Required design and implementation work

1. **Measure before changing the baseline.** Add a reproducible per-component
   allocation breakdown for source buffers, decoded/normalized text, provenance,
   reverse indexes, hard-line/block/style records, compatibility copies,
   history nodes/maps, ledger overhead, view/layout caches, and syntax state.
   Record current and peak owned bytes, allocation counts, and independently
   sampled process footprint/RSS where available. Label each metric and its
   exclusions; account for allocator caching and macOS memory compression when
   comparing it with the history estimate. Use a separate process per memory
   fixture so an earlier case's allocator high-water mark cannot contaminate
   later RSS/peak measurements. Preserve the old report for comparison rather
   than replacing its numbers without explanation.
2. **Compress common mappings.** Represent valid unchanged UTF-8 runs with
   identity/constant-offset mappings over immutable source pieces or shared
   text leaves. Use compact stride/run encodings or bounded checkpoints for
   UTF-16 and legacy conversion where appropriate. Record exceptions for BOMs,
   CRLF/CR interpretation, differing encoded widths, invalid-byte diagnostics,
   and other actual transformations. Ordinary ASCII, valid UTF-8, and regular
   UTF-16 text MUST NOT require persistent heap records for every scalar.
   Source pieces, transformation exceptions, and compact line metadata should
   determine common-case mapping size. Dense exceptional input still needs
   finite chunk sizes and measured worst-case bounds.
3. **Support both mapping directions without expanding runs.** Provide indexed
   source-to-text and text-to-source queries over the compact representation.
   Resolve local encoding/Unicode details within bounded chunks/checkpoints.
   Do not replace the forward per-character table with an equally large reverse
   table, or expand a run to individual records during lookup, editing, or undo.
   Adjacent source/text boundaries retain their explicit insertion association
   and boundary affinity; numeric offset coincidence is not identity.
4. **Construct and retain only necessary projection data.** Stream or fuse
   decoding, line-ending interpretation, and literal projection while retaining
   their observable contracts. Avoid simultaneous whole-file intermediate
   strings/vectors and reuse immutable buffers when their encoding permits it.
   Make expensive derived metadata lazy or incrementally materialized; preserve
   logarithmic navigation with compact aggregates/checkpoints. Define partial
   materialization and snapshot validity explicitly, so missing work cannot be
   mistaken for an exact mapping. Initial display/input MUST NOT wait for
   unrelated offscreen projection work. Do not substitute a whole-line
   temporary buffer for a whole-file buffer: huge lines are required fixtures.
5. **Keep edits and retained snapshots compact.** Local edits split/coalesce
   mapping runs and update persistent paths, preserving unchanged identities.
   Undo, redo, branches, multiple views, background jobs, save, and recovery
   share unchanged storage. Audit paragraph/style IDs and per-line allocations
   for opportunities to pack or share metadata. Compatibility flat copies must
   be explicitly bounded/evictable or removed from large-file hot paths; they
   must not silently double retained storage after a read API is called.
6. **Preserve all source and editing guarantees.** Keep byte-exact no-op saves,
   patch locality, original encodings/BOMs/mixed endings, malformed-byte
   preservation, checked snapshot/domain identities, grapheme-safe edits,
   anchor remapping, and exact reverse-edit translation. Rich formats retain
   relational provenance, hidden syntax, indivisible entities, and structured
   ambiguous/synthetic/unresolvable results. An identity fast path for Code/Text
   must not incorrectly assume those properties for Markdown, HTML, or RTF.
7. **Resolve the undo-budget consequence explicitly.** The original 256 MiB
   combined history policy charged the live document state as well as retained
   history. The implemented default now allows 256 MiB of additional history
   above live-state cost, while reporting total, live, and additional estimates;
   explicitly constructed combined targets remain available. With the original
   combined target, ordinary edits lost undo history as retention attempted to
   meet a budget smaller than the live state. The earlier Code benchmark used
   128 history nodes and live-state estimate plus 64 MiB; this was a disclosed
   test override. The repeated literal-memory fixtures now use the shipped
   default policy and verify that completed edits retain undo. Verify
   useful undo under the shipped policy after compaction, and decide explicitly
   how unavoidable live-state cost relates to prunable history cost. Raising
   the benchmark budget, undercounting mappings, or disabling retention is not
   a fix for the underlying storage amplification. Excluding current projection
   or cache costs from the history budget would change the existing retention
   specification and requires an explicit product-policy decision; do not make
   that change implicitly as an accounting optimization.
8. **Avoid wide undo-summary copies.** Ordinary local undo now uses persistent
   source/text differences. A history unit with distant disjoint edits still
   produces one conservative source replacement hull, whose construction can
   copy all intervening bytes. Replace that summary path with sparse changes
   or a lazy retained replacement representation while preserving the exact
   independent history/anchor map. Include large macros or grouped edits near
   opposite ends of a file; local-undo tests do not cover this case.

#### Completion criteria and regression fixtures

Keep this TODO open until measured evidence and structural tests establish the
new bounds. Choose and commit explicit retained/peak byte and metadata-count
ceilings for each fixture after validating the measurement method. They must
demonstrate that ordinary text no longer has tens of bytes of persistent
mapping overhead per character, and that opening does not recreate the old
per-character tables transiently. Do not invent an unmeasured universal memory
multiplier for arbitrary encodings or rich documents.

- Run the same 100 MiB UTF-8, UTF-16LE, and Latin-1 and million-short-line
  fixtures with syntax disabled and with both providers suspended. Include
  UTF-16BE, mixed endings, BOMs, ASCII/non-ASCII runs, supplementary scalars,
  combining sequences, and invalid/truncated encoded input.
- Add a multi-megabyte single line and cases dense with conversion exceptions.
  Assert bounded temporary buffers and distinguish expected exceptional-data
  cost from ordinary identity/stride-run cost. Measure at several file sizes
  so a favorable small-file result cannot hide per-character heap growth.
- Compare every compact mapping/reverse edit against a simple trusted oracle
  on small randomized documents. Exercise exact boundaries, interior queries
  requiring rejection, line endings split across chunks, edits at mapping-run
  edges, same-text replacements, deletion, and old-snapshot/anchor resolution.
- Measure current and peak allocation during cold construction, after first
  display, after distant scrolling/wrap/zoom, after local and disjoint edits,
  after undo/redo/branch retention, and after view/job/snapshot release. Verify
  reclamation as well as sharing. Repeated operations must not grow hidden flat
  caches or the allocation ledger without bound. Updating allocation accounting
  after a local edit must visit newly retained paths, not rescan unchanged
  subtrees or every retained snapshot.
- Re-run large-file editing with the normal history policy and report actual
  undo availability alongside memory. Any separate stress-test override must
  remain visible in its report. Test color-only Code style changes and multiple
  views without duplicating document mappings or altering saved source.
- Record cold first-interaction latency and p50/p95/p99 input/scroll latency
  together with allocation statistics on identified hardware/builds. Keep the
  existing no-full-projection, bounded-chunk, stale-result, and source-fidelity
  tests. Confirm the real frontend remains responsive while background metadata
  construction proceeds; a mock-shaper result alone does not establish that.

This TODO records unfinished work and does not relax the existing performance,
source-preservation, position, or history requirements.

### Code syntax performance and regression gates

Both Vim and Tree-sitter MUST have performance suites, with pinned engine,
grammar/rule, query, compiler, and fixture revisions. Test the complete Code
editing/display path as well as isolated providers: a fast highlighter does not
excuse full-document decoding, projection, styling, or layout on ordinary edits.

Instrument executor identity; detector bytes; syntax input read callbacks,
bytes supplied and copied; Vim instructions, evaluated lines, checkpoints and
dependency operations; Tree-sitter progress and reuse metrics where available;
query traversal, predicates, matches/captures and limit overflow; injection
jobs; named-style resolution; shaped bytes; queue depth; publication rejections;
and retained bytes by cache/job/snapshot class. Input bytes supplied are not
reported as bytes parsed: a parser may consume part of a chunk or reread it.

Required automated fixtures and assertions are:

1. **Cold open:** one million short lines and 100 MiB mixed-length inputs,
   including UTF-8, UTF-16, legacy encoding, and mixed physical endings. Suspend
   all providers and assert that initial text display, typing, and scrolling
   still work. First display must not wait for grammar compilation or a whole
   Tree-sitter tree. Assert bounded chunk reads without an extra flattened
   whole-document copy for syntax. Detection reads no more than its 64 KiB
   allowance plus logarithmic index work, including on a huge first/last line.
2. **Warm local edits:** generated independent-function/statement fixtures at
   10,000, 100,000, and 1,000,000 lines for each bundled language, plus pinned
   representative Vim files. Insert/delete characters and hard-line boundaries
   at the start, middle, and end. With unchanged downstream context, provider
   and query work stays under the same explicit per-fixture ceilings as
   unrelated suffixes grow; tree lookups may grow logarithmically. Commit
   numeric ceilings with each fixture, justify baseline changes, and reject
   full projection or eager suffix-offset/cache rewrites.
3. **Scrolling:** jump from the first viewport to near EOF and repeatedly
   alternate distant regions, with wrapping on/off and two views. Foreground
   work includes no provider call/wait or scanning of the intervening prefix.
   Once a Tree-sitter tree exists, scrolling does not reparse it. Queries and
   Vim recovery stay budgeted; returning to retained valid coverage performs
   zero provider evaluation. Repeated scrolling keeps cache/queue memory bounded.
4. **Long dependency:** remove/restore a comment terminator, multiline string,
   or heredoc/raw-string delimiter so repair can reach EOF. Lazy invalidation
   must not enumerate all affected spans/checkpoints. Yield repeatedly while
   processing input between jobs. Compare completed results to fresh analysis;
   capped results remain missing/provisional and cannot validate exact state.
5. **Huge line:** a multi-megabyte line, giant token/delimiter, deep nesting,
   regex backtracking/lookaround, and dense captures, wrapped and unwrapped.
   Assert bounded temporary chunks, VM/predicate work and emitted-span budgets;
   no full-line flattening merely to highlight the visible fragment. Test a
   controllably blocked native scanner on a worker while foreground operations
   continue, then release it. Hard-termination providers additionally test
   worker termination/replacement; do not leave an infinite native test running.
6. **Dependency correctness:** same-length identifier renames affecting text
   predicates, previously failed matches becoming successful, ancestor/sibling
   dependencies, locals, and injection-language changes. Include changes outside
   the viewport that affect its highlights. Incremental output equals a fresh
   parse/query; structural changed ranges alone must not pass these tests.
7. **Cancellation and supersession:** deterministically interleave edits,
   undo/redo, reload, format/language changes, query/package reload, view close,
   and suspended parse/query completion. Resume only the captured input; reset
   abandoned Tree-sitter continuations. Reject stale coverage and bound queued
   replacement requests and retained revisions. An unrelated edit must not
   restart identical previously capped work.
8. **Fallback and limits:** remove a grammar, mismatch its ABI, reject a query
   handler, exceed regex/query/memory budgets, exhaust captures, and make the
   Vim directory unavailable. Assert Tree-sitter to ready Vim to default-style
   behavior without blocking input. Empty completed primary captures clear
   fallback colors. Simulate input beyond Tree-sitter's 32-bit limits without
   allocating gigabytes; make zero invalid Tree-sitter calls and no truncating
   casts. Loading other languages exercises the same bounded provider contract.
9. **Styles and geometry:** change/add/delete a referenced Code style while
   several Code buffers are open, including a formerly missing name. Preserve
   provider runs/trees/queries. Color-only changes make zero shaping calls;
   font/size/weight changes and Base Paragraph line-spacing changes invalidate
   the correct layout layers and preserve viewport anchors. Missing styles use
   the default, never another provider. Cache invalidation remains lazy on the
   million-line fixture. No source/dirty/document-undo changes occur.
   Exercise the Character menu with defined and missing provider names: listing
   is read-only, editing targets the shared Code definition, and only explicit
   Define creates a missing definition. Test asynchronous publication of mixed
   font sizes, weight, and slant in newly exposed rows; repaint and reflow use
   current identities and preserve text anchors across multiple Code buffers.
   Paint-only publications preserve unrelated exact heights even when another
   syntax style has metric declarations. Estimated scrollbar height remains
   usable without styling the unvisited prefix or the gap between distant views.
   Type at an off-center caret in a distant viewport with syntax enabled and
   unavailable; compare its screen baseline and scroll before the edit,
   immediately afterward, and after deferred completion. Change a delimiter so
   a larger font arrives asynchronously, including a caret near the top edge;
   preserve the baseline or assert the minimum movement required for visibility.
   Include ordinary newline/wrap advancement and committing marked text through
   an input method, without recentering their previous visible content.
   Include wrapped long lines crossing layout-chunk boundaries and native Vim
   syntax, not only unwrapped Tree-sitter documents. Place the caret away from
   the viewport center and type rapidly before, during, and after delayed syntax
   publication. Assert unchanged screen placement when the row still fits,
   complete local coverage of the preserved viewport, and the minimum scroll
   only when the final row actually clips. A million-line fixture must retain
   these guarantees without full-document layout or anchor walks.
   Suspend providers across insertion, deletion, discontiguous edits, undo, and
   redo: existing mapped colors remain until accepted replacement coverage,
   while empty current coverage clears them and stale results change nothing.

Use deterministic clocks, fuel counters, bounded fake providers, and allocation
accounting as hard release gates. Assert zero syntax-provider execution/waits
on the UI and edit-commit paths, at most one active job per mutable session and
one coalesced replacement, and all configured hard byte limits. Native backend
allocations that cannot be hard-limited in process need measured accounting and
declared soft-limit/fallback behavior, or an executor that enforces hard limits.
Measured soft-limit overruns must trigger that declared cancellation/fallback
policy; they cannot be ignored because the allocation occurred in native code.

Additionally benchmark p50/p95/p99 first-interaction, input, scroll, highlight
repair, and fallback latency on recorded reference hardware, both with syntax
disabled and enabled. Target roughly 2-4 ms cooperative syntax slices and
publication of ready visible results within one display frame, without
delaying required input/layout. Record actual maxima and native-scanner
exceptions. These are measured tuning/regression targets, not a promise that
arbitrary native callbacks are preemptible; timing tests do not replace the
structural gates. All cache and worker budgets must have tested finite defaults.

## Correctness test strategy

- **C ABI declaration checks**: `cargo run --locked --example check_c_abi`
  validates the public C header's sizes, alignment, field offsets, constants,
  and selected function declarations against the current Rust definitions.
  This explicit native validation runs in `scripts/test-mac.sh` and is also
  available as `make check-abi`; it stays outside ordinary Rust test execution.
  Windows MSVC validation runs from a matching Visual Studio developer shell.
  See `docs/abi-validation.md` for platform setup and CI usage.
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
  detected, forced, and defaulted mode. Text, Code, Markdown, and HTML use the
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
- **Code conformance tests**: byte-exact no-op saves, local edits, reinterpret
  round-trips, literal markup/entities/whitespace, and all input paths with
  Smart Quotes both on and off. Syntax styles never affect clipboard/register
  text, editing boundaries, source serialization, or undo. Test marker/filename
  precedence, bounded samples, aliases, ambiguous extensions, unavailable
  languages, and no marker rescanning during typing. Test global stylesheet
  persistence/live updates independently of document history and default
  rendering for unresolved names. For supported Vim rules, compare completed
  exact results to pinned Vim 9.2 and record deliberate heuristic/profile
  differences; the local 9.1.1887 corpus is an additional fixture. For all nine
  bundled Tree-sitter families and separately loaded packages, compare
  incremental trees/captures to fresh evaluation, including errors, injections,
  Unicode/encoding coordinates, query precedence, and fallback coverage.
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

Primary Code syntax references:

- [Vim 9.2 syntax rules and synchronization](https://github.com/vim/vim/blob/v9.2.0000/runtime/doc/syntax.txt)
- [Vim 9.2 syntax-state cache](https://github.com/vim/vim/blob/v9.2.0000/src/syntax.c)
- [Tree-sitter rope input and coordinates](https://tree-sitter.github.io/tree-sitter/using-parsers/2-basic-parsing.html)
- [Tree-sitter incremental parsing and language regions](https://tree-sitter.github.io/tree-sitter/using-parsers/3-advanced-parsing.html)
- [Tree-sitter predicate and directive contract](https://tree-sitter.github.io/tree-sitter/using-parsers/queries/3-predicates-and-directives.html)
- [Tree-sitter query limits](https://docs.rs/tree-sitter/latest/tree_sitter/struct.QueryCursor.html)
- [Neovim queries, captures, and injections](https://neovim.io/doc/user/treesitter/)

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

- which additional format adapters beyond those explicitly required here ship;
- the canonical syntax of any future adapter not specified above;
- user policy for Unicode edits not representable in the source encoding;
- exact Unicode word/sentence segmentation tailoring;
- whether rich system clipboard formats are required for the first release;
- additional style defaults and visual design choices not fixed above;
- hyphenation and justification; and
- hardware-specific latency thresholds and numerical cache defaults not fixed
  by the bounded-work and Code syntax regression requirements above.

## Format controls, Markdown authoring, and lists

The status bar exposes a native popup for source format, with a small vertical
triangle and hover highlight. Encoding and line endings appear only in their
File submenus, not in the status bar. A choice is a checked core transaction
shared by the buffer's views and reversible with undo. Completing a format
selection returns keyboard focus to the document as soon as the popup closes. Format
selection within a format family or to Text/Code changes interpretation while
preserving source bytes. An explicit HTML-to-Markdown or Markdown-to-HTML
conversion instead translates the formatted text and representable styling to
new source syntax as one undoable transaction, including source-visible variants.
This explicitly requested conversion may replace the entire source and reports
lost unsupported information through command output in the status line. It is distinct
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
reporting the count in the status line; undo restores the exact original bytes.
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

Opening a Markdown file defaults to the source-visible Markdown view. The
status popup can select the WYSIWYG view. New bold uses `**`, italic uses `*`,
and headings use one through six `#` characters followed by one space.
Untouched alternative delimiters and physical line endings remain exact.

Switching Markdown Source and WYSIWYG must preserve source bytes and anchors
without quadratic work in document length and formatting-change count.
Reprojection maps use ordered batch traversal of provenance and changes;
the frontend refreshes each affected view once and exports only its viewport
text. Large-document regressions must cover the first switch in both
directions, rather than relying only on a warmed projection cache.

The rich-format Code character style and Code Block paragraph style are
independent of the Code format and its global stylesheet. These rich styles
use the system monospace family and dark green (`#006400`). HTML `<code>` and
`<pre>` and Markdown inline/fenced backticks project to these roles; code whitespace remains
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
inside the same paragraph and item. In preformatted Markdown code, Text, and
Code, the source line ending itself expresses the break. This is distinct from
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

The two HTML views similarly share one physical HTML serialization. Opening an
HTML file defaults to HTML Source; the status popup can select HTML WYSIWYG:

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
document source merely by being toggled. All document text input uses the same
policy: Insert and Replace typing, Normal and Visual `r`, clipboard and register
puts, and committed input-method or accessibility replacements transform
straight quotes in prose. Uncommitted input-method overlays and command prompts
remain literal. The entire Code format always suppresses quote conversion,
regardless of this setting, language detection, syntax coverage, or style name.
In other formats, Code character spans and code paragraphs always suppress quote
conversion, including pending Code typing styles and code in pasted rich text.
Source input also preserves syntax-required quotes in HTML attributes and
Markdown code/link/tag constructs, including constructs inside an input batch.
Existing curly quotes are preserved as supplied. Quote conversion and its text
edit form one transaction; undo restores the original source exactly.
If a generated quote cannot be represented in the current encoding or through
a supported format escape, preserve the entered straight quote.
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
At any caret position inside a list-item paragraph in Insert mode, Tab and
Shift-Tab invoke the same verified Indent and Unindent actions for the complete
item when the format can express them. This includes a continuation paragraph
inside the item. An unavailable nesting change leaves the item unchanged.
Source views recognize every caret position associated with the semantic list
paragraph; Markdown Source also recognizes its literal marker boundaries.
Backspace at the visible WYSIWYG beginning first unindents a nested item by one
level. At the top level it removes list treatment, regardless of whether the
caret arrived by typing, navigation, or pointer placement; hidden tags and
insertion history do not redefine the beginning.
The beginning of a continuation paragraph within the same item is not a new
item-label boundary: Backspace joins it to the preceding visible paragraph using
the ordinary first-paragraph style rule, including when a nested child intervenes.
Removing a Markdown item label also removes the item's hidden continuation
indentation without consuming its visible paragraph boundaries or following items.
Numbered continuation and repeat calculate the next ordinal from current
structure. One list action and its supporting source patches form one undo unit.
Indenting an ordered item creates a nested numbering run beginning at one, so
its first generated label is `a.`. Generated ordered marker styles by zero-based
depth are decimal, lower-alpha, lower-roman, and decimal; generated bullet
styles are disc, circle, square, and disc. Deeper imported levels use the fourth
style, matching the fourth generated list paragraph style and its additional
structural inset. Alphabetic numbering continues bijectively after `z` (`aa`,
`ab`, and so on), and Roman numbering falls back to decimal outside its
supported positive range. Marker spelling in source-visible modes remains
literal and editable rather than being replaced by this generated furniture.
For a structural indent, Markdown canonicalizes only the moved ordered root
markers to `1.`, `2.`, and so on; untouched marker bytes remain unchanged.
HTML gives a newly created nested container the matching `type` value and no
inherited `start`, so saved HTML has the same marker family and restart.
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
markup. Leaving an HTML list converts the current item into a paragraph in
place, reusing existing paragraph children instead of creating another empty
placeholder. Intentional continuation paragraphs and nested lists remain. An
attribute-free item wrapper is removed when its children already supply the
paragraph owners; a container remains when needed to retain attributes or mixed
block content. After leaving an empty item, another Backspace deletes its
preceding paragraph separator and merges normally.
Canonical RTF list paragraphs use scoped groups with `\ls0\li400\fi-200`,
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

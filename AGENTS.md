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

- Put portable document, command, editing, and layout logic under `src/core`.
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
- **Text position**: a boundary in formatted text, with affinity where a single
  logical position has two visual sides. Public editing APIs do not use pixels.
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
  adjacent transformation stages, including affinity and synthetic content.
- **Metrics generation**: an identity for the current font resolver and text
  measurement environment. It changes when results may differ.

The formatted document is expressed in valid UTF-8. User-visible positions,
selections, deletions, and caret movement MUST NOT split an extended grapheme
cluster.
Shaping clusters and bidirectional affinity must be preserved where they are
stricter than grapheme boundaries. Source byte positions and formatted text
positions are distinct types and must never be confused. Raw numeric offsets
MUST NOT be retained across edits as persistent marks, selections, or cache
keys; use source anchors, stable projected identities, and explicit affinity.

## Source authority and transformation pipeline

The ground truth of every document is its source artifact. Formatted content
and layout are disposable projections that can be regenerated on demand. The
required conceptual pipeline is:

```text
SourceArtifact (original bytes or package parts)
    -> EncodingProjection (valid UTF-8 plus byte provenance)
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

### Semantic edit intentions and reverse projection

The UI and Vim command engine issue typed intentions against a projection
snapshot, for example replacing text, setting or clearing bold, changing a
block kind, inserting a link, or deleting a formatted range. They MUST NOT
directly persist mutations to derived style spans.

Committed edits follow this path:

1. capture the semantic intention, selection/cursor affinity, and exact
   projection revision on which the command operated;
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
- composable provenance with affinity;
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
- Background projection is cancellable and revision checked. Results from a
  stale source revision or transform configuration are discarded.
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
meaningful with proportional fonts. Its left and right edges are layout x
coordinates and its vertical extent is a sequence of visual rows. Each row is
hit-tested independently to produce a set of logical text ranges. Block edits
are one atomic source transaction after reverse projection. When wrapping or
width changes during an active block selection, recompute row intersections
from stable projected identities/source anchors and the stored x edges.

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

The `=` operator initially performs deterministic indentation defined by core
configuration. It must not invoke a language-specific formatter implicitly.

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

One Insert/Replace session is normally one undo unit, matching Vim's useful
behavior. Explicit cursor moves, `Ctrl-O`, paste boundaries, and IME commit
boundaries may split units where Vim or native text input expectations require
it; these cases need tests.

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
- Required commands: `u`, `Ctrl-R`, `U`, `.`, `q{a-z}`/`q`, `@{a-z}`, and
  `@@`.
- Undo history is a branching transaction tree, not only two linear stacks.
  Its committed states are source snapshots. Text changes, style changes, and
  compound block edits participate in the same history, and undo restores the
  exact previous source bytes before regenerating projections.
- Dot repeat records a semantic change action with its inserted payload and
  count, not a replay of frontend-specific key codes.
- Macros record normalized core command/text events. Replaying a macro is
  deterministic and guarded against unbounded recursion.

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
  `:set nolinebreak`, and queries for implemented options.

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
- Persistent marks, selections, jumps, and viewport anchors retain a projected
  identity/affinity plus recoverable source provenance. They are remapped after
  every committed source transaction and reprojection.
- A committed source transaction reports exact byte patches, affected source
  identities, before/after source revisions, projection changes, and any
  supporting format/encoding changes.

### Buffer, view, and command state

Keep document-global and view-local state separate. The source artifact,
pipeline configuration, projection caches, registers, marks, search history,
undo history, and file identity belong to the document/buffer/session as
appropriate. Cursor, selection, desired x, wrap width, scroll anchor, viewport,
and layout caches belong to a view. Two views of one buffer may have different
widths, wrapping settings, and layout caches while sharing source snapshots,
formatted projections, and width-independent shaping results.

The command engine is a state machine inspired by Vim's separation of Normal
command parsing and operator execution:

1. normalize platform input into key, text, composition, pointer, or command
   events;
2. parse counts, prefixes, registers, operators, and motions without mutating;
3. resolve motions/text objects against a formatted projection snapshot and,
   when needed, a view layout snapshot;
4. produce a typed semantic edit intention or non-mutating action;
5. reverse-project an edit intention to a minimal source patch set, tentatively
   apply it, reproject, and verify its semantic result;
6. atomically commit the source transaction, undo record, mode,
   cursor/selection, registers, repeat state, and invalidations; and
7. return state changes, diagnostics/policy requests, and redraw/layout
   requests to the frontend.

Commands that require visual rows explicitly receive a valid layout snapshot.
Most edits, searches, word motions, and linewise operations must remain usable
in headless core tests with fake format, encoding, and measurement providers.

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
- legal caret stops with affinity;
- per-cluster or per-caret advances;
- ascent, descent, leading, ink bounds, and typographic bounds;
- resolved fallback font identities; and
- direction and any diagnostics for unsupported/missing glyphs.

Requirements for the provider contract:

- Results are deterministic for the same request and metrics generation.
- It never returns a caret stop inside an indivisible shaping cluster.
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
- hit-test mappings in both directions;
- caret geometry for every legal caret stop requested;
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

On a source-backed style change, use the new projection's change summary to
invalidate only affected shaping. A global view default-font or
metrics-generation change may make all shaping logically stale, but
invalidation is generation-based and `O(1)`; replacement remains
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
- The primary scroll anchor is a stable projected identity/position with source
  provenance, its affinity, and an offset from the viewport edge, not only an
  absolute y value.
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
  provenance, and affinity invariants.
- **Round-trip tests**: no-op saves are byte-identical; changed saves alter
  only declared patches; forward projection after a reverse edit satisfies the
  semantic intention.
- **Encoding tests**: legacy and invalid byte input survives no-op saves, maps
  correctly to valid UTF-8, and never silently substitutes unrepresentable
  edits.
- **Pipeline tests**: composed provenance and reverse edits match an equivalent
  unfused pipeline; stale, generated, ambiguous, and unsupported edits return
  the required structured result.
- **Undo tests**: any transaction sequence round-trips exact source bytes,
  projected content/styles, registers where applicable, marks, and cursor state
  through undo/redo and alternate branches.
- **Command table tests**: every supported command covers counts, registers,
  mode transitions, operator composition, cancellation, dot repeat, and macro
  replay.
- **Vim differential tests**: for supported behavior that this file does not
  deliberately change, run the plain-text identity adapter, execute equivalent
  keystrokes against a pinned Vim version, and compare resulting formatted
  text, cursor, registers, and mode when feasible.
- **Layout equivalence tests**: incremental results equal a from-scratch layout
  for the same projection/configuration using a deterministic fake shaper.
- **Cache tests**: source edits, projection changes, font-generation changes,
  wrapping toggles, and resize invalidate exactly the required layers. Stale
  async results are rejected.
- **Geometry tests**: round-trip text position -> caret point -> hit-tested
  position, including affinity, bidi, ligatures, mixed sizes, empty lines, and
  visual-block rectangles.
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

- implementation language and build/package layout;
- which format adapters beyond plain text ship initially;
- each adapter's default authoring policy, such as Markdown delimiter and HTML
  element/style choices;
- user policy for Unicode edits not representable in the source encoding;
- exact Unicode word/sentence segmentation tailoring;
- exact regular-expression syntax supported by `/` and `:substitute`;
- whether rich system clipboard formats are required for the first release;
- default font, page insets, colors, and other visual design choices;
- hyphenation and justification; and
- concrete latency and memory budgets for supported hardware.

# Per-view presentation: plan

Status: **plan only; nothing is implemented.** This document is the study and
the staged plan for making Markdown Preview and Text/Code properties of a view
instead of the buffer, so a diff can show literal source while another view of
the same document shows Preview. The syntax language stays a buffer property,
but moves into its own styling layer.

Read [AGENTS.md](AGENTS.md) and [docs/diff_mode.md](docs/diff_mode.md) first.
Line numbers below are from `main` at `1fc266e`.

## Contents

1. [The two problems and their cause](#1-the-two-problems-and-their-cause)
2. [What the code does today](#2-what-the-code-does-today)
3. [Target model](#3-target-model)
4. [Design decisions](#4-design-decisions)
5. [Mechanisms](#5-mechanisms)
6. [Phases](#6-phases)
7. [C ABI and frontends](#7-c-abi-and-frontends)
8. [Tests and validation](#8-tests-and-validation)
9. [Risks](#9-risks)
10. [Decisions](#10-decisions)
11. [Requirement and document updates](#11-requirement-and-document-updates)

## 1. The two problems and their cause

**Problem 1: opening a `.md` file from the diff file popup shows Preview
first.** Confirmed, on both frontends:

- macOS: `EVDiffModeController.selectFile`
  (`src/mac/AppShell/EVDiffMode.swift:1288`) calls
  `EVDocumentWindowController.openDiffFile`
  (`EVDocumentWindowController.swift:2088`), which opens the file through the
  ordinary argument-list path. The core is created with the Markdown format,
  so `viem_core_create` builds the complete Preview projection. The pane then
  shows that document. Only afterwards does `present` load the base (on a
  worker unless cached) and `install` call `beginDiff`, whose
  `EVCoreDiffSession.init` (`src/mac/Editor/Sources/EVCoreDiffMode.swift:101`)
  switches the buffer to Code. Preview is on screen for as long as the base
  load takes.
- Windows: `CoreDocument.EnterDiff` (`src/win/Core/DiffComparison.cs:43`)
  does the same switch. The previous pair stays visible while loading, so it
  flashes less, but the work is the same.

The wasted work per file is one complete Markdown projection (roughly 30
times the source in retained memory, per the comment in
`view_format_prefetch.rs`), one complete Code reprojection, a whole-document
diff of the two projections to map positions, and a `SourceMetadata` history
node. Leaving diff mode does it all again in reverse and adds a second
history node.

**Problem 2: turning Preview on in another window breaks the diff.**
"Open in New Editor Window" calls `EVDocument.showAdditionalWindow`, which
adds a view to the same `Core`. `ModelRequest::SetViewFormat` replaces the
buffer's only projection, so the diff's live view becomes Preview too. The
diff session compares hard lines of the live projection with the Code base,
so its line pairing no longer describes what is drawn.

**Cause.** Three things that should be independent are one field:

| Concept | Where it lives today | Where it belongs |
| --- | --- | --- |
| What the file is (Text, Code, Markdown) | `DocumentState.format` | Buffer default |
| Which projection is shown (literal or Preview) | `DocumentState.format` + `DocumentState.projection` | View |
| Whether a literal view is styled as Text or as Code | `DocumentState.format` | View |
| Which language Code uses | One `SyntaxService` per `Core` | Buffer (unchanged), in its own layer |

`DocumentState` (`src/core/document/mod.rs:634`) is the undo tree's payload,
so the choice of projection is also part of history.

## 2. What the code does today

### 2.1 Ownership

```
Core<P>                      src/core/coordinator.rs:955   one per buffer
├─ document: Document        src/core/document/mod.rs:733
│   ├─ history: History<DocumentState, PositionMap>
│   │     DocumentState = { revision, source: SourceSnapshot,
│   │                       projection: FormattedDocument, format,
│   │                       source_hard_lines, encoding, file_format, … }
│   ├─ code_presentation     styled clone of the projection (syntax spans)
│   ├─ configuration(_state) theme style overlay, keyed by one Format
│   └─ view_format_prefetch  background preparation of the next switch
├─ syntax: CoreSyntax        coordinator/syntax.rs:68
│   ├─ service: SyntaxService      one language for the whole buffer
│   ├─ selection, detection, automatic_mode
│   └─ markdown: MarkdownCodeSyntax   per-code-block services in Preview
├─ buffer_commands: BufferCommandState   command/mod.rs:1242
│   └─ marks: BTreeMap<char, usize>      raw formatted offsets
└─ views: BTreeMap<ViewId, View<P>>      cursor, layout, search, diff overlay
```

### 2.2 Facts the design depends on

1. **The source is already separate and shared.** `SourceSnapshot`
   (`document/source.rs:52`) is a persistent piece tree over `Arc<[u8]>`.
   History, `saved_source` and candidates share its buffers.
2. **A projection is a second, larger structure.** `FormattedDocument`
   (`document/projection.rs:1550`) owns a `FormattedTextTree` (a UTF-8 rope
   with its own `Arc<str>` buffers), blocks, hard lines, style spans,
   provenance and a style sheet. Every part is persistent, so a clone is O(1)
   and shares everything.
3. **Text and Code are the same projection.** `project_with_start`
   (`projection.rs:4878`) builds Code as `project_plain` followed by
   `install_code_styles`. Only the sheet, the document style and the spans
   differ. A Code projection is one paragraph block per hard line.
4. **Syntax colours are installed into the document.** `poll_syntax`
   (`coordinator/syntax.rs:310`) feeds the service's runs to
   `Document::install_code_presentation_delta`, which keeps a styled clone of
   the projection that `Document::projection()` (`mod.rs:1133`) returns in
   place of the history state's. So there is already a "structure plus
   disposable styled clone" split, but with room for exactly one styling.
5. **A syntax service is not tied to a view.** `SyntaxService` takes an
   immutable `SyntaxInputSnapshot` (identity plus a text-tree clone) and a
   `SyntaxConfiguration` (language, filename, Vim directory). Its provider
   session lives in the shared worker pool. `for_export()` and
   `PreparedCodeSyntax` already run a second service over the same buffer.
6. **Switching Preview is a history transaction.**
   `prepare_view_format_change` (`document/transaction.rs:4830`) reprojects
   the whole source, diffs the two projections
   (`source_backed_reprojection_edits`), builds a
   `PositionMap::for_projection_switch`, reconciles block identities and
   publishes a new revision. The coordinator maps every view through that one
   map (`coordinator.rs:4826`).
7. **Edits already know their source patches.** Every prepared transaction
   carries `summary.source_patches()` and can attach a source-domain
   `PositionMap` (`attach_prepared_source_map`, `transaction.rs:1493`).
8. **A projection can already be advanced from source patches alone.**
   `prepare_reprojected_source_patches` (`transaction.rs:4681`) tries the
   bounded Markdown strategies and falls back to complete reprojection, with
   shadow verification.
9. **Positions are formatted offsets in one implied domain.** `TextPoint`
   and `TextAnchor` carry a document and a revision, not a projection.
   `PositionDomain` has `Source(part)` and a single `FormattedText`.
   `CommandInterpreter.cursor` and `BufferCommandState.marks` are raw `usize`
   offsets rebased through each transaction's map.
10. **Anchors can recover through source.** `TextAnchor`'s backing holds a
    `SourceAnchorProvenance` (source boundary plus neighbouring text
    fingerprints), and `FormattedDocument::map_source_boundary` maps a source
    boundary into a projection in logarithmic time.
11. **The diff is view-local presentation.** `DiffSession`
    (`coordinator/diff.rs`) installs per-view diff lines and filler in
    `ViewLayout`, like search matches. It needs only that the live view shows
    one block per hard line.
12. **Scale of the single-projection assumption.** `.projection()` is called
    1,271 times inside `src/core/document`, 174 in `command`, 175 in the
    coordinator, 109 in `layout` and 41 in `ffi`. `&Document` or
    `&mut Document` appears in 553 command signatures. `tests/all` constructs
    documents with `Document::…` 1,320 times. Any design that changes the
    type or signature of `Document` for those callers is impractical.

## 3. Target model

Four layers. Each refers to the one below and never copies it.

```
Buffer (one per file; the serial owner is still Core)
│
├─ 1. Source            SourceSnapshot + encoding, BOM, fileformat,
│                       line-ending evidence, source hard-line index.
│                       The undo tree is over this layer only.
│
├─ 2. Projections       at most one lineage per kind, built on first use:
│     ├─ Literal        decoded text, one block per hard line
│     └─ Preview        the Markdown formatted projection
│
├─ 3. Style layers      built on first use, shared by every view using them:
│     ├─ on Literal:    Text | Code
│     │                 Code owns the buffer's one SyntaxService and one
│     │                 styled clone of the Literal projection
│     └─ on Preview:    Preview (theme defaults, code-block syntax)
│
└─ 4. Views             presentation = (projection kind, style layer)
                        + own overlays: diff, search, composition, whitespace
                        + own cursor, selection, layout, viewport, wrapping
```

How the user-facing modes compose:

| A view showing… | Projection | Style layer | Overlay |
| --- | --- | --- | --- |
| Text | Literal | Text | |
| Code (the buffer's language) | Literal | Code | |
| Markdown with Preview off | Literal | Code (Markdown) | |
| Markdown Preview | Preview | Preview | |
| Diff, live side | Literal | Code | diff |

**One language per buffer.** The language stays a buffer property, chosen by
detection, the language menu or `:set filetype`, exactly as today. Every
Code view of a buffer uses it, so there is one Code layer and one syntax
service, not a map of them. The layer is still a separate type with a
narrow interface (D7), so a second language per buffer, or another kind of
styling, can be added later by changing how layers are held, not how they
work.

What is shared, by construction:

- Every view of a file shares the source, history, registers and edits.
- Every literal view (Text, Code, diff) shares one text tree, block list and
  provenance index.
- Every Code view, including a diff's live side, shares the one syntax
  service, run store and styled projection. Analysis happens once for the
  union of their visible ranges. The diff adds only its per-view line
  overlay.
- Every Preview view shares the Preview projection, its code-block syntax,
  reference cache and image snapshots.
- A kind or style layer no view uses costs nothing: it is never built, and
  it is released when its last view leaves.

Memory per buffer becomes: one source, at most one Literal projection, at
most one Preview projection, one syntax run store, and each view's layout. Today's switch transiently holds both projections and keeps
both in history; the new model holds both only while views of both kinds are
open.

## 4. Design decisions

### D1. History holds source only; projections are derivable caches

The undo tree becomes `History<SourceState, SourceTransition>`, where
`SourceState` is `DocumentState` without `projection` and `format`.
AGENTS.md already says formatted content is a disposable projection and that
undo "restores recorded source artifacts and metadata". This makes it true in
the types.

Each projection lineage keeps a bounded side table from history node to the
exact projection it had there, filled when a node is committed or visited
while the lineage is live. Undo to a cached node restores the recorded
projection exactly as today (block identities included). Undo to an uncached
node derives the projection with the [follow](#51-follow-moving-a-lineage-across-a-source-change)
mechanism. Evicting a cache entry never affects correctness.

Rejected: keeping `DocumentState` as the payload with an optional second
projection. Finalized history states are immutable, so a projection built
later could not be attached, and one could never be dropped when its last
view closes.

### D2. `Document` keeps its API and gains a focus

`Document` stays one owned type with the same methods. It holds the lineages
and a `focus: Presentation`; `projection()`, `format()`, `state()` and every
editing method address the focused lineage and style layer. The coordinator
sets the focus at each entry point that acts for a view.

This is ambient state, chosen deliberately because of fact 12. It is safe
because a buffer has one serial owner, and it is made *checkable* by D3.

Internally `DocumentState` survives as the working pair "source plus one
projection" that all transaction code builds and reads. Each lineage's
current state holds a clone of the source handle (O(1), same buffers), so
`self.state()` keeps working for the 70 `impl Document` blocks.

Rejected: a borrowed `Document<'_>` facade over a new `Buffer` (breaks 1,320
test constructions and every stored `&Document`); two full `Document`s
sharing a source (two undo trees, save points, edit groups and command
checkpoints to keep isomorphic).

### D3. Each projection kind is its own coordinate domain

`PositionDomain::FormattedText` becomes `FormattedText(ProjectionKind)`.
`TextPoint`, `TextAnchor`, `TextRange`, `RangeSet`, `PositionMap`, layout
snapshot identities and `ViemFormattedSnapshotIdentity` carry the kind, and
every existing identity check also compares it. A literal offset used
against Preview fails with `PositionError::WrongDomain` or `STALE_REVISION`
instead of landing on the wrong character. This is the AGENTS.md rule
"reject mismatched identities … never clamp" extended to the new domain, and
it is what turns a missed focus into a test failure.

### D4. Preview on or off is not an edit

Changing a view's presentation creates no history node, no new revision, no
dirty state and no register effect, like `modifiable`. It changes one view.
`ModelRequest::SetViewFormat`, `prepare_view_format_change` and the
`SourceMetadata` nodes it creates are deleted.

**Decided:** undo never reverses a view toggle of any kind (Preview, Text or
Code, wrap, line numbers, line meaning). This changes documented behaviour
(AGENTS.md: "Toggling Preview is an undoable buffer transaction"). See
[section 11](#11-requirement-and-document-updates).

### D5. What is per view and what is per buffer

| Choice | Scope | Set by |
| --- | --- | --- |
| Preview on or off | View, while the buffer is Markdown | The Preview toggle; choosing the Markdown language (turns it off) |
| Text or Code styling of literal views | Buffer | Detection, the mode and language menu, `:set filetype` |
| Language | Buffer | The same |
| Diff | Window; its live views are always Code | Diff mode |

- **Decided:** Preview is per view. Toggling it changes the active view
  only; every other view keeps what it shows.
- **Decided:** the language menu, `:set filetype` and redetection stay
  buffer-wide, as today and as in Vim. The language belongs to Code only; a
  Preview view has no language. A language change recolours every Code view
  and never invalidates a Preview view.
- **Decided:** choosing a language on a Markdown file sets the buffer's
  language to that selection and its mode to Code, as today. Preview exists
  only for a Markdown buffer, so:
  - choosing a language other than Markdown makes the buffer Code in that
    language, and every view of it, including views that were showing
    Preview, shows Code. Preview is unavailable until the buffer is set back
    to Markdown from the mode menu;
  - choosing Markdown as the language turns Preview off in the active view
    only; other Preview views stay in Preview.
- **Decided:** a split copies its source view's Preview state. A new window
  of a Markdown buffer uses the saved setting (D8).
- **Decided:** a diff's live view is created as Code and stays Code for its
  whole life (D9). That is not a Preview toggle and never writes the saved
  setting.

A view therefore stores one thing: whether it shows Preview. Whether a
literal view is drawn as Text or Code comes from the buffer, except that a
diff view is always Code. There is no "follow or override" state to keep
consistent.

### D8. The Preview toggle is remembered

A new setting in `config.json`, `markdown.preview`, holds the last state the
user explicitly set with the Preview toggle, in any view of any document. It
is on until first set. It follows the pattern of `code.wordWrap`.

- It is the last state *set*, not the state of the last view used. Focusing
  or closing a view never writes it.
- A view created without a source view to copy (opening a Markdown
  document, a new window) reads it: Preview when on, Code when off.
- Views that already exist never change when it changes.
- Only the explicit toggle writes it. Diff views, splits, reloads and a
  language choice that turns Preview off do not.

Settings changes create no source edit and no document undo, as for every
other preference.

### D9. Diff mode has its own window

**On.** Turning Diff Mode on from an ordinary window leaves that window and
its views exactly as they are and opens the comparison in a new window,
using the diff window frame defaults (`windows.diffFrame`, cascading as
today). The new window shows the same buffer, with unsaved edits, through
new views created as Code.

**Off.** Turning Diff Mode off in a diff window opens the current file in an
ordinary window, exactly as **Open in New Editor Window** does, and then
closes the diff window. The buffer stays open throughout, so there is no
save review and unsaved edits carry over. If the file is already shown in an
ordinary window, that window is brought forward instead of opening another.

Consequences:

- Entering or leaving diff mode never changes an existing view's
  presentation, wrap, line numbers or scroll position, so nothing has to be
  restored.
- The "restore the format when the last pair ends" logic disappears from
  both frontends rather than being rewritten per view.
- A window is a diff window from creation to close. Folder diffs and
  `viem --diff` already work this way.

### D6. Followers catch up inside the same transaction

**Decided:** the simple form. When an edit commits, every *other* live
lineage is advanced to the new source before publication, so
`Document::revision()` stays the single revision of source and all live
projections. A lineage with no views is not advanced. The known cost and the
recorded improvement are in
[section 5.1](#51-follow-moving-a-lineage-across-a-source-change).

### D7. Styling is a layer with its own interface, held singly for now

A style layer is the only thing that knows how a projection gets its sheet
and automatic spans. Its interface is small:

```
input:   the projection's text tree at a revision, an exact transition map,
         the visible ranges of the views using it
output:  a styled clone of that projection, and the lines whose paint or
         metrics changed
```

Nothing above it (views, layout, diff, commands) and nothing below it
(lineages, history, follow) knows what is inside. `Document::projection()`
asks the focused view's layer for its styled clone and falls back to the
unstyled structure.

With one language per buffer the Literal lineage holds its Code layer as a
single optional slot, and Preview holds its own. That keeps the first
version free of keyed maps, eviction and per-layer bookkeeping. Because
views name a layer by *kind* (Text, Code, Preview) and reach it only through
the interface, replacing the slot with a keyed collection later touches the
slot's owner and the view's presentation type, not the layer, layout or the
syntax service.

## 5. Mechanisms

### 5.1 Follow: moving a lineage across a source change

One operation serves every case where a lineage must reach a source state it
has no cached projection for:

```
follow(lineage, old source state, new source state, source patches)
    → new projection, formatted PositionMap, LocalFormattedChange?
```

Callers: an edit made through another lineage; undo, redo or `:undo N` to an
uncached node; a lineage rejoining after its views were closed; reload and
encoding or `fileformat` conversion.

- **Literal.** The literal projection is a function of decoded source with a
  divisible mapping, so source patches convert exactly to literal text
  edits. Reuse the literal regional machinery (`build_line_local_candidate`,
  `DisjointLiteral`) against the *given* new source rather than building one.
  The proof obligation is that the result's provenance covers the new source
  exactly.
- **Preview.** Generalize `prepare_reprojected_source_patches`
  (`transaction.rs:4681`) to take the target source instead of applying the
  patches itself. It keeps its strategies (TableLocal, IndependentParagraph)
  and its complete fallback. Add a line-local strategy driven by patches if
  measurements show declines on ordinary typing.
- **Patches without a transaction** (history navigation, rejoin) come from
  the persistent source tree diff (`SourceSnapshot` already reports
  `SourceDiffStats`), which visits only changed subtrees.
- **Outcome discipline.** Follow uses the existing Verified / Declined /
  Rejected outcomes and candidate tracing. A follower can never reject the
  user's edit: its last resort is complete projection of the new source,
  which cannot fail. Cargo-run tests shadow-verify every bounded follow
  against complete reprojection.
- **Block identities** are reconciled with
  `install_reconciled_source_block_ids`, as the switch does today. Each
  lineage owns its `next_projected_block_id`.
- **Position map.** A bounded follow yields one splice over its formatted
  hull, refined by provenance. Views in that lineage rebase their anchors
  through it; their anchors' source provenance resolves anything the hull
  collapsed.

Cost. With one presentation open (the common case) follow never runs. With a
Code view and a Preview view of one Markdown file open, each keystroke pays
one bounded Preview follow, or a complete Preview projection when the
strategies decline. That fallback is the cost editing in Preview already has
when its own strategies decline, but it is new for typing in the Code view.
This is accepted for the first version.

**Known improvement, not planned.** If typing is slow with both kinds of
view open on a large Markdown file, the fix is a *lagging* follower: the
edit publishes without the other projection, a worker reprojects it from the
immutable source snapshot and installs it when still current, and input to a
lagging view forces catch-up first. It needs a projection revision distinct
from the source revision throughout layout, which is why it is left out.
Phase 4 records this in three places so it is found again: a comment at the
synchronous follow call, an entry under open work in
[docs/performance.md](docs/performance.md#open-work) with the measured
dual-view typing numbers, and a work counter for complete follower
reprojections so a slow session can be diagnosed.

### 5.2 Translate: one position between projections

```
translate(from kind, to kind, offset, affinity) → MappingOutcome<offset>
```

Formatted offset → source boundary through the origin projection's
provenance → `map_source_boundary` in the target. Empty Markdown paragraphs
have no grapheme to carry provenance; they use the per-point form of the
`empty_paragraph_source_points` pairing that `prepare_view_format_change`
builds today. Hidden syntax has no caret stop, so a literal position inside
it maps to the nearest visible boundary by affinity and reports `Moved`.

Uses: a view changing presentation (cursor, Visual anchor, selection, jump
list, viewport anchor), buffer-shared positions (5.3), and undo restoration
snapshots recorded in another domain (5.4).

This replaces the whole-document projection diff for a switch with a few
logarithmic queries. `PositionMap::for_projection_switch` stays as the test
oracle: for every caret stop of the corpus documents, translate must agree
with it.

### 5.3 Focus and buffer-shared positions

`Core::focus_view(view)` runs first in every entry point that takes a
`ViewId`. It sets `document.focus` to the view's presentation and, when the
domain changed since the last command, translates the buffer-shared
positional state: `BufferCommandState.marks` and any other raw offset found
by the phase 2 audit (candidates: staged `:s///c` targets, `:global` line
identities, repeat state, file-mark positions). Those values carry a domain
tag so a missed translation fails loudly.

Code that walks several views (rebasing after a commit, `poll_syntax`,
prelayout, search presentation) addresses lineages explicitly through
`document.projection_of(kind)` and never through the focus.

### 5.4 History and restoration

- `SourceTransition` holds the source-domain map, the source patches summary
  and the semantic kind. `History` no longer sees formatted maps.
- Each lineage keeps `edge_maps` for edges committed while it was live.
  `map_between` composes them when every edge is present and otherwise
  follows (5.1).
- `HistoryRestorationSnapshot` (cursor and marks as `TextAnchor`) stays in
  history but its anchors carry their domain. Restoring into a view of the
  other kind translates through the anchors' source provenance.
- Command checkpoints and edit-group suspension capture and restore the
  lineage tables together with the history position. They are `Arc` clones.
- Lineage caches get their own byte budget beside the history retention
  budget and report into the same memory diagnostics. When history prunes a
  node, lineages drop that node's entries.

### 5.5 Style layers and syntax sharing

`CoreSyntax` splits in two, with no new collections:

- **Buffer policy** (unchanged): `selection`, `automatic_mode`, detection,
  filename, associations. It resolves the buffer's one language.
- **`CodeLayer`**: the `SyntaxService`, the publication state (`published`,
  `referenced_names`, `complete_progress`) and the styled clone of the
  Literal projection with its transition map (today's `CodePresentation`,
  which moves here from `Document`). It implements the D7 interface. The
  Literal lineage holds an `Option<CodeLayer>`.

Each poll, when at least one view shows Code: rebase the layer's input to
the current Literal text tree, request the union of those views' visible
ranges, and publish the delta into the styled clone. Layout invalidation
after a publication touches only Code views; a Text or Preview view of the
same buffer is not invalidated. When the last Code view leaves, the layer is
cancelled and dropped; the worker pool's idle-session limits already bound
what its provider retained. A language change reconfigures the one layer, as
`set_code_language` does today.

`Document::projection()` under a Code focus returns the layer's styled clone
when it is current, else the plain Literal structure with the Code sheet,
exactly as `code_presentation` falls back today. Under Text it returns the
Literal structure with the Text sheet and theme defaults.

`Format::Code` stops being stored: `initialize_code_format`,
`install_code_styles` inside `project`, and `Configuration.format` become
style-layer selection. `Document::format()` is computed from the focus, so
the roughly 400 call sites that ask `is_code()`, `is_literal()` or
`is_markdown()` keep their meaning per view.

Preview's `MarkdownCodeSyntax` moves onto the Preview lineage unchanged.
`PreparedCodeSyntax` and `view_format_prefetch` are replaced by one idle
task: when a Markdown buffer has only one kind live and editing has paused,
prepare the other lineage and its visible syntax so the first toggle is
instant. It holds the same single-job, size-capped, cancel-on-edit rules.

### 5.6 Opening

`viem_core_create` decodes, detects line endings and builds the source state
only. `ViemDocumentOptions.format` becomes the buffer default. The first
`viem_core_view_add` builds the lineage its presentation needs. The decoded
text is handed from create to that first build and then dropped, so the
single-decode guarantee (`DocumentOpenWorkStatistics`) holds.

Anything that needs a projection before a view exists builds the default
lineage on demand. A work counter per kind lets tests assert that opening a
Markdown file into a diff pane builds one Literal projection and no Preview
projection.

`initialize_style_defaults` stores the defaults on the document and applies
them when a matching lineage is built.

## 6. Phases

Each phase leaves the suites green and is separately reviewable. Phases 1 to
3 change no user-visible behaviour except where stated.

### Phase 0. Baselines

- Record memory (`large_file` and history limits) and the switch, open and
  typing timings for a 100 KiB and a 1 MiB Markdown file, on `main`.
- Classify the `format()` call sites: asks about the projection kind, about
  Text-versus-Code styling, or about the file. The third group moves to the
  buffer default; list it.
- List every C ABI function that reads the projection without a view
  argument.

### Phase 1. Style layers on the Literal projection

Core:

- Introduce `Presentation` and `Document.focus` with only the style layer
  (Text or Code) varying. A wrong focus here can only pick the wrong
  colours.
- Store Literal structure without Code styles; build the Text and Code
  styled clones through the D7 interface. Remove `Format::Code` from
  `DocumentState`.
- Split `CoreSyntax` into buffer policy and the single `CodeLayer`.
- Per-view presentation in `View` (Preview flag, diff forces Code);
  `Core::focus_view`.

Delivers: a Text view beside a Code view of one file, sharing one text tree.
A diff of a Text or Code file no longer touches buffer state. Markdown diffs
still use the old switch. The language stays buffer-wide.

### Phase 2. Domain tags

- `ProjectionKind`; `PositionDomain::FormattedText(kind)`; tag points,
  anchors, ranges, maps, layout and snapshot identities.
- Audit and tag buffer-shared raw offsets.
- No behaviour change: one kind is live at a time, so every check passes.

### Phase 3. Source-only history with one lineage

- `SourceState`, `SourceTransition`, lineage side tables, memory accounting,
  checkpoint and edit-group capture.
- `follow` for history navigation to uncached nodes, exercised by a test
  mode that evicts the cache.
- Replace `SetViewFormat` with "swap the live lineage": still buffer-wide,
  but no history node and no revision bump (D4). Position mapping moves to
  `translate`, with the old map as oracle.

Delivers: Preview toggles and diff entry no longer appear in undo history.

### Phase 4. Concurrent lineages

- Both lineages live; follower catch-up in `prepare`, atomic publication of
  source, every projection and every map.
- Per-lineage view rebasing, layout change records and
  `layout_change_chain`.
- Focus-change translation of buffer-shared positions; restoration across
  domains.
- Per-view presentation for Preview; release of a lineage with no views.
- `VIEM_SHADOW_LINEAGE` test mode (section 8).
- Measure dual-presentation typing and record it, with the lagging-follower
  note, in `docs/performance.md` and at the follow call site.

Delivers: problem 2 in the core.

### Phase 5. ABI and frontends

Section 7. Delivers problems 1 and 2 to users on macOS and Windows.

### Phase 6. Cleanup and optional sharing

- Delete `view_format_prefetch.rs`, `PreparedCodeSyntax`,
  `capture_format_position_anchors`, `follow_format_line_mode`'s switch
  path, the frontends' restore-mode bookkeeping.
- Update AGENTS.md and the docs (section 11).
- Optional, each only with numbers:
  - **Zero-copy literal text.** For UTF-8 with LF endings the literal text
    tree's bytes equal the source's. Leaves could slice the source pieces'
    allocations instead of owning `Arc<str>` copies. This saves about one
    source size out of the roughly twelve a Code projection retains, so
    measure before starting.
  - **Shared style resolution and shaping** between views with the same
    presentation, zoom and metrics. Wrapping and geometry stay per view.

If problem 1 must ship before phase 5, a stopgap exists: have the frontends
open a file destined for a diff pane with `VIEM_FORMAT_CODE`. It removes the
flash and the first switch but still records the restore in history and does
nothing for problem 2, so it is not part of this plan.

## 7. C ABI and frontends

The ABI has no compatibility promise; change the core, header, Swift, C#,
generated bindings and the ABI check together and bump the version. The
header and `ffi.rs` use CRLF.

### 7.1 ABI

| Today | After |
| --- | --- |
| `ViemDocumentOptions.format` selects the projection built at create | Buffer default only; create builds no projection |
| `ViemViewOptions` has no mode | Adds presentation flags: Preview, and diff (always Code). No language |
| `viem_core_view_set_document_mode_with_effects(core, view, …)` changes the buffer | Keeps its buffer-wide meaning, including the language; a new `viem_core_view_set_preview…` changes one view and is rejected unless the buffer is Markdown |
| `viem_core_copy_document_mode_json(core)` | Adds a view argument; reports default and the view's effective presentation |
| `ViemDocumentState.format`, `VIEM_DOCUMENT_STATE_MARKDOWN_SOURCE` | Describe the buffer default; a per-view state query reports the effective format |
| `ViemFormattedSnapshotIdentity { document, revision }` | Adds the projection kind |
| Formatted-snapshot and other projection reads taking only `core` | Take a view, or an identity that names the kind (phase 0 list) |
| `viem_diff_session_create(base_core, new_core, …)` | Takes the live view; rejects a view that is not literal |

### 7.2 macOS

- `EVCoreDocumentBackend`: `sourceFormat` and the mode queries move to
  `EVEditorSurfaceController` (per view). The backend keeps the default.
- `EVDiffModeController.toggle` / `enable`: from an ordinary window, open a
  new window on the same `EVDocument` (as `showAdditionalWindow` does) sized
  from `windows.diffFrame`, and enable diff mode there. The originating
  window is not touched. The pristine-untitled-window reuse stays for
  folder diffs only.
- A pane in a diff window creates every view with the Code override, so a
  file chosen from the popup is literal in its first frame, before the base
  has loaded.
- `EVCoreDiffSession`: delete `restoreMode`, the mode switch in `init`, the
  heir hand-off and the restore in `end()`. Nothing is restored.
- The Preview toggle saves `markdown.preview` through
  `EVConfigurationStore`; opening a Markdown document reads it.
- "Open in New Editor Window" needs no change: the new window's view reads
  the saved Preview setting.
- **Diff Mode** off in a diff window: show the document in an ordinary
  window (reusing one that already shows it, else `showAdditionalWindow`),
  then close the diff window. No save review, since the buffer stays open.
- Choosing a language keeps its buffer-wide call. Surfaces showing Preview
  refresh to Code when the buffer stops being Markdown.
- Menus, toolbar and status bar validate against the active view: Preview
  checkmark, language menu, formatting toolbar, the line-mode eye.
- Saved `code.wordWrap` and `code.lineNumbers` apply when a *view* starts
  showing Code.
- Reload replaces the core under existing surfaces: each surface re-adds its
  view with the presentation it had.

### 7.3 Windows

- `CoreDocument`: delete `EnterDiff`, `LeaveDiff`, `InheritDiffMode`,
  `priorDiffMode`, `diffViews` and `SwitchingDiffProjection`
  (`src/win/Core/DiffComparison.cs:31-70`).
- `DiffMode.cs`: turning diff mode on from an ordinary window opens a new
  window on the same `CoreDocument` with the diff frame and creates its live
  `CoreView` as Code. The originating window is not touched.
- The Preview toggle saves `markdown.preview`; a view created without a
  source view reads it.
- Diff mode off opens the document in an ordinary window and closes the diff
  window. A language choice that makes the buffer non-Markdown shows Code in every view.
- `State.format` consumers (menus, toolbar, Ctrl-0…5 accelerators, status
  bar) read the active view's effective format.
- The Windows changes cannot be built on the Mac; they need
  `scripts/test-win.ps1 -Suite DiffMode,DiffCollapsedContext` on Windows.

## 8. Tests and validation

Tests go in `tests/all/` (registered in `main.rs`) or beside the code, and
are deterministic. Each asserts what its name says.

**Document**

- Follow equals fresh projection for both kinds, across the corpus, every
  encoding, mixed line endings and BOM cases (source round trip, patch
  locality, incremental versus fresh).
- An edit through each kind advances the other; both equal fresh projections
  of the identical source snapshot.
- Undo and redo with the lineage cache hit, missed and evicted reach the
  same projection; branch navigation; exact source bytes throughout.
- Translate agrees with `for_projection_switch` at every caret stop;
  empty paragraphs, hidden syntax, front matter, tables, images.
- Changing presentation leaves revision, dirty state, history node count,
  registers and `.` repeat unchanged.
- Work counters: one kind open performs zero follower work; opening into a
  literal view builds no Preview projection and decodes once; closing the
  last Preview view releases its lineage.
- A failed or rolled-back command restores every lineage table.
- Domain tags: a point, anchor or map from one kind is rejected by the
  other.

**Coordinator**

- Two views, Preview and Code: typing, operators, paste, `:s`, `:g`, undo
  and redo from each; the other view's cursor, selection and viewport anchor
  stay on the same text.
- Marks set in one presentation and used in the other.
- Two Code views and a diff view of one buffer: one service, each visible
  range analyzed once. A Text view of the same buffer requests no analysis
  and is not invalidated by a syntax publication.
- Changing the buffer's language recolours every Code view and leaves Text
  and Preview views untouched.
- The layer boundary holds: layout, diff and command code compile without
  access to `CodeLayer`'s fields (kept private to its module, as the
  architecture dependency checks do for other layers).
- A diff session with a Preview view open on the same buffer: toggling,
  typing and undo in the Preview view leave the diff's pairing equal to a
  fresh diff.
- Closing a view cancels its layer's work only when it was the last user.
- Every command entry path sets the focus (a debug assertion on the domain
  tag, exercised by the command matrix).

**Syntax and layout**

- The existing syntax list in AGENTS.md (cold display, local edits,
  injections, cancellation, fallback) per layer.
- Layout cache invalidation when a view's presentation changes; two views of
  different widths and presentations; rapid presentation changes followed by
  editing; stale job rejection across a presentation change.

**Shadow lineage.** `VIEM_SHADOW_LINEAGE=1` keeps the other projection kind
live for every Markdown-capable document and verifies it against fresh
projection after each transaction. Running the existing suites and the
consistency probe with it gives the follower path the whole existing corpus
of edits without writing each case twice.

**Native**

- macOS `EVDiffModeTests`: choosing a `.md` from the file popup never
  presents a Preview frame and the buffer's history has one node; "Open in
  New Editor Window" then Preview on leaves the diff view literal with
  correct pairing; leaving diff mode adds no undo entry.
- Windows `DiffMode` suite: the same, plus its existing first-frame check.
- Cached-versus-fresh pixel comparisons for two views of one buffer in
  different presentations.

**Validation level.** This is a large, risky refactor of shared editing
code, so from phase 3 on run focused tests, both suites, the full
consistency probe, backend fuzzing (with the shadow lineage on) and the
memory measurements, plus `check_c_abi`, the bindings check and native tests
when the ABI or a frontend changes. Phases 1 and 2 need the suites, the
quick probe and memory measurements.

## 9. Risks

| Risk | Mitigation |
| --- | --- |
| A code path reads `document.projection()` under the wrong focus | Domain tags (D3) make it an error; debug assertion in `Core` entry points; phase 1 introduces focus where a mistake is cosmetic |
| Buffer-shared raw offsets missed by the audit | Tag the containers, not the call sites; shadow-lineage runs alternate focus |
| Follower regional strategy wrong | Existing Verified/Declined/Rejected discipline and shadow verification against complete reprojection |
| Typing with both kinds open scales with the document when the bounded follow declines | Accepted; measured and recorded in phase 4 with the lagging-follower improvement noted in code and `docs/performance.md` |
| Undo to an uncached node changes block identities or caret recovery | Cache hit path is today's behaviour; miss path uses the switch's reconciliation; anchors fall back to provenance; tests cover both |
| History memory accounting drifts | Lineage tables have their own visitor and budget; keep `assert_memory_matches_full_recount` for history and add the equivalent for lineages |
| Command checkpoint rollback leaves a lineage ahead | Checkpoint captures lineage tables; rollback test per kind |
| Frontend state still per document somewhere | Remove the document-level accessors so the compiler finds every use |
| Accessibility and text-service queries cross domains | Snapshot identity carries the kind; they already pass identities |
| Windows side cannot be verified on the Mac | Land the C# with the ABI change but gate the phase on a Windows run |

## 10. Decisions

AGENTS.md asks that product questions not be settled silently. All of the
questions this plan raised are answered; record them in AGENTS.md and the
docs as section 11 lists.

| Question | Decision | Where |
| --- | --- | --- |
| Scope of the language | Buffer-wide, one language per buffer; `FileType` and `&filetype` keep their meaning | D5 |
| Language and Preview | The language applies to Code only; Preview views are untouched by a language change | D5 |
| Choosing a language on a Markdown file | Sets the buffer language and Code mode; Preview applies only while the buffer is Markdown | D5 |
| Scope of Preview | Per view | D5 |
| Splits and new windows | A split copies its source view; a new window uses the saved setting | D5, D8 |
| Remembering the toggle | One application-wide setting holding the last state explicitly set | D8 |
| Undo and view toggles | Undo never reverses a view toggle | D4 |
| Diff Mode on | New window with the diff frame; the existing window is unchanged | D9 |
| Diff Mode off | Open the file in an ordinary window, then close the diff window | D9 |
| Follower cost | Synchronous catch-up; lagging followers recorded as a possible improvement | D6, 5.1 |

Small points decided here by inference; say if any is wrong:

- A language choice that turns Preview off does not write
  `markdown.preview`; only the Preview toggle does.
- Choosing a non-Markdown language makes the whole buffer Code, so its
  other Preview views switch to Code as well, since Preview has no meaning
  for a buffer that is not Markdown. The views keep their own Preview flag,
  so setting the buffer back to Markdown shows Preview again where it was.
- Diff Mode off reuses an ordinary window that already shows the file
  instead of opening a second one.

## 11. Requirement and document updates

Update together with the code that changes the behaviour:

- **AGENTS.md, Product:** "Changing format, Preview or view mode reprojects
  the same source" stays true; add that Preview, and Text versus Code, are
  per view over one shared source, while the language is per buffer.
- **AGENTS.md, Core architecture:** state that the undo tree holds source
  only; that each projection kind is a coordinate domain; that follower
  projections are regional candidates under the same proof rules.
- **AGENTS.md, Editing in Preview:** replace "Toggling Preview is an
  undoable buffer transaction…" with the per-view rule and its guarantees
  (no bytes, revision, dirty state or history change; caret and viewport
  recovered; not quadratic; idle preparation).
- **AGENTS.md, Code and syntax highlighting:** a buffer has one language and
  one highlighting layer shared by all of its Code views; styling is a
  separate layer over the projection.
- **AGENTS.md, Files:** reload keeps each view's presentation.
- **docs/diff_mode.md:** remove "a format switched to Code for the
  comparison is restored when the last pair … ends" and the Preview-switch
  notes in the macOS and Windows status; "Showing the new side in Code" is
  now a view presentation. Rewrite the Diff menu section: **Diff Mode**
  on opens a new diff window and leaves the current one unchanged; off
  opens the file in an ordinary window and closes the diff window (D9).
- **AGENTS.md, Configuration:** `markdown.preview` and its rules (D8).
- **docs/window-panes.md:** a split copies the presentation; views of one
  buffer may differ.
- **docs/performance.md:** per-buffer memory model, the dual-presentation
  typing measurement, and under open work the lagging-follower improvement
  (5.1) with the symptom that should prompt it.

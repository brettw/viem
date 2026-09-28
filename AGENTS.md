# Agent guide for Viem

This file records product requirements, deliberate compatibility choices, and
engineering constraints that are not safely inferred from the current code.
Implementation inventories and mechanics belong in code and focused documentation.
Keep this guide current when requirements change; do not add API schemas,
supported-language lists, UI catalogues, or implementation walkthroughs.
Current development guides and examples are indexed in [docs/README.md](docs/README.md).

## Product and repository boundaries

Viem is a modal, keyboard-first editor for human-language writing: gVim's
composable editing model with word-processor typography, mixed styles,
proportional fonts, optional soft wrapping, visual-row navigation, and responsive
large-document editing. macOS and Windows are native frontends; Windows follows
the macOS presentation and behavior with platform controls. A terminal frontend,
a Vimscript runtime, and pixel-for-pixel gVim emulation are outside scope.

Text, Code, and Markdown are the source formats. Code is literal text with
optional syntax highlighting; HTML files use Code. Only Markdown has Source and
WYSIWYG views. Switching those views preserves source bytes, encoding, BOM, line
endings, anchors, and undo restoration. There is no general format conversion or
reinterpretation and no HTML editing mode. Every opening path must admit
unrecognized files, including extensionless files and dotfiles, through the
lossless Text fallback; native file filters must not reject them first.
Recognized and explicitly selected formats retain their precedence.

- Keep portable document policy/state in `src/core/document`, Vim interpretation
  in `src/core/command`, and layout in `src/core/layout`. The crate root is a
  composition boundary and public facade, not a competing implementation home.
- Keep platform input, drawing, clipboard, accessibility, files, and lifecycle
  mechanisms in the native frontends. The Rust core cannot import platform UI
  frameworks. macOS uses Swift/AppKit, not SwiftUI; Windows uses C#/WinUI 3 and
  Win2D/DirectWrite. Features must not become macOS-only by default.
- Use a narrow C ABI with explicit ownership and batch exchanges on hot paths.
  Internal Rust/API/ABI backward compatibility is unnecessary: update every
  in-repository consumer, provider, declaration, and test together, remove old
  adapters, and rebuild both sides. This does not relax pointer, snapshot,
  threading, atomicity, persisted-format, or user-behavior guarantees.
- Use format-family predicates rather than duplicating variant sets at callers.
- Follow Vim 9.2 semantics unless this guide specifies a deliberate difference.
  Do not claim a command is supported until counts, registers, operator-pending
  forms, undo grouping, and relevant edge cases have tests.
- Integration tests belong in `tests/all/` and are registered in
  `tests/all/main.rs`; top-level `tests/*.rs` creates another full-core binary.
  Use a module filter, e.g. `cargo test --test all ex_sort::`.
- New layout work requires cache-invalidation and large-document tests. Correct
  full-document relayout on every edit is unacceptable.
- See [macOS development](docs/macos-development.md) for build/test commands.
  After moving a checkout, clean path-dependent Swift/Clang module caches.
  Builds must use the verified bundled Vim runtime without requiring installed
  Vim or downloads; preserve its original bytes, attribution, and license.

## Architecture and concurrency constraints

- `document` owns state, invariants and semantic mutations; `command` interprets
  Vim input through immutable public queries; `layout` consumes document
  snapshots. Document code must not depend on command grammar. Commands must not
  access private storage, patch source directly, or mutate state while resolving.
  The core facade coordinates these services without duplicating their policy.
  Native actions use the same model transactions without manufacturing Vim keys.
- A failed, unsupported, cancelled or stale source transaction leaves source and all
  success-dependent state unchanged, including registers, marks, selections,
  mode, repeat state and history. Prepare and validate before atomic publication;
  no provider callbacks or external I/O belong inside that publication. Compound
  commands retain completed transactions under their explicit partial-progress
  rules; failure of a later action does not undo earlier completion acceptance.
- Each buffer has one serial owner of mutable state. Workers receive immutable
  snapshots and return candidates; they never mutate buffers/views or call UI
  code. Do not place a whole-document reader/writer lock around the model.
- Background results must match all relevant document, projection, view,
  configuration, metrics and resource identities at installation. Stale results
  cannot become current; reusing an unchanged subresult requires validating its
  complete dependencies. Identifiers must not wrap or be reused after teardown.
- Work, queues, caches and retained revisions need finite budgets and
  cancellation. Prioritize exact visible interaction over overscan and offscreen
  work; coalesce superseded edits, widths and viewports. Foreground editing must
  not wait for obsolete offscreen work. Cache-only pre-layout must not change the
  visible snapshot, caret or viewport, or cancel useful visible work.
- Never hold locks while parsing, projecting, shaping, wrapping, drawing, doing
  I/O, waiting, invoking callbacks or crossing into providers. Do not nest cache
  or registry locks. Native render resources need explicit ownership and thread
  rules, including final release from workers after a view closes.
- View closure cancels its work and composition overlay and finishes its
  committed insertion undo group without disturbing other views. Shared buffer
  state includes source/history/registers; each view has its own cursor,
  selection, mode, pending command, desired x, wrapping and viewport.

## Source preservation and editing

The source artifact is the only document persistence authority; formatted
content, resolved styles, and layout are disposable projections. An artifact may contain multiple
parts rather than one flat string. Every editable pipeline must preserve:

1. **Identity:** opening and saving without a source edit reproduces the exact
   physical artifact, including container metadata.
2. **Patch locality:** original bytes outside the declared source patches remain
   identical, including alternate delimiter/entity spellings and whitespace.
3. **Semantic correctness:** projecting the patched source produces the edit
   requested by the user.

Retain malformed, unknown, and unsupported input losslessly. Source serialization
must not depend on the formatted projection. Detection is read-only and cannot
rewrite ambiguous input. A small edit must not regenerate the entire document.
Supporting syntax repairs may extend beyond the selection, but must be minimal,
explicit patches in the same verified atomic transaction.

Commands and UI issue semantic intentions against an exact projection revision.
Tentatively apply/reproject and verify before atomically publishing source,
projection, history, registers, marks, cursor, and invalidations. Do not persist
mutations to derived spans. Editing requires an unambiguous reverse rule through
every transform; arbitrary generated/lossy output is read-only. IME commit uses
this same path; cancellation leaves no committed state.

Reverse mapping is relational, not necessarily one source interval. Never expand
an edit to a convenient source hull, drop synthetic/unmapped content, or guess
from stale identities. Rewrite shared indivisible contributors once, preserving
unselected parts without expanding logical selections/change maps. User errors
describe unsupported edits/format constraints, not internal “unambiguous source
range” failures.

A valid caret names the visible location within its paragraph, including an
empty body. Hidden delimiters must not make it unusable. Line ownership selects
context at line edges; boundary affinity selects inline context. Nonempty edits
consume the smallest contributing source runs while retaining intervening hidden
syntax. Text, formatted payloads, clipboard replacement, and formatting share
these rules; structural changes use the format's semantic operations.

Ordinary valid Markdown deletion/replacement must succeed, including last code
span characters, cross-row selections, native selections, Visual Line, and
Delete/Backspace at structural or document boundaries. Repair delimiters,
newly active punctuation, code-span joins, and retained folded spaces locally.
An emptied implicit paragraph needs supporting syntax if its unselected boundary
would disappear on reopen; an unselected empty paragraph at a half-open edge
remains outside the edit. Removing a fully selected object/owner includes its
fallback body and opaque contents, not unrelated metadata. Do not bypass
verification; a candidate mismatch on supported visible-text deletion is an
editor defect. Test command entry paths, reopened source, and exact undo/redo,
not just prepared document edits.

### Encoding and line endings

Raw bytes remain authoritative. Preserve BOMs/declarations and copy untouched
encoded bytes rather than re-encoding them. Invalid bytes project to visible
diagnostics tied to their original bytes; a no-op save must not substitute U+FFFD.
An insertion immediately after a dangling UTF-16 byte may locally encode U+FFFD
to restore code-unit alignment, as part of that insertion; no modal prompt is
needed, unrelated malformed bytes stay untouched, and undo restores exact bytes.

Use the original encoding for new text when representable. Otherwise use a
semantically exact format escape or return an explicit encoding policy/rejection;
never silently substitute or change encoding during ordinary input. Markdown
prose may use numeric references; code spans/fences and literal/source views
must retain literal semantics. Explicit encoding conversion is separately
undoable and verifies the new projection. Conversion to Latin-1 may replace
unrepresentable scalars with `?`, reporting the count; ordinary Latin-1 input
remains strict, and undo restores exact original bytes.

Text, Code, and Markdown share line-ending interpretation and conversion.
`unix` recognizes LF, `dos` CRLF and bare LF, and `mac` CR; unrecognized CR/LF,
other Unicode separators, and trailing DOS Ctrl-Z remain content unless a later
format explicitly interprets them. Follow Vim's ordered `fileformats` opening
policy; macOS defaults to `unix,dos`, with `unix` fallback. Detection is read-only
and buffer-local. Preserve each existing delimiter and final-terminator presence;
new breaks use current `fileformat`.

Changing `fileformat` converts existing logical break spellings in one
cancellable, undoable transaction. It must not reinterpret literal CR/LF, change
formatted text/hard-line identities, or add/remove a final terminator. Verify
candidate decoding on reopen and reject conversions that would change the
logical sequence. This explicitly requested conversion may touch every break.
Plain Text gives each logical break a paragraph boundary, preserves consecutive
empty lines, and has no source-backed rich styles or hidden style sidecar.

### Position identity and incremental work

Source, decoded, formatted-text, and layout coordinates are distinct. Public
editing uses logical boundaries, not pixels. Snapshot points are valid only in
their document/domain/revision. Reject incompatible identities, invalid Unicode
boundaries/ranges, and stale layout; never clamp or substitute the current
revision. State surviving edits—cursors, selections, marks, jumps, viewports,
history, caches—requires stable anchors, not naked document offsets. Temporary
snapshot-local offsets are allowed.

Logical edits/selections/search/registers obey extended-grapheme boundaries;
fonts and wrapping cannot change affected text. Visual placement also obeys
shaping caret stops. Missing geometry may expand drawing to its containing
cluster, never the logical edit. Hard breaks contribute one LF to flat logical
APIs, with no implicit final newline; retain hard-break/paragraph distinctions.
Objects are atomic with explicit plain-text representations. Empty lines supply
real caret geometry, not fictitious characters.

Insertion association controls movement across inserted text; affinity
independently selects adjacent semantic context and visual side. Typing follows
its insertion; selection edges associate inward so unrelated boundary insertions
are excluded. Anchors follow moved identities; deletion collapses to its boundary,
preferring following then preceding content. Report recovery/ambiguity/failure,
and never guess for source edits. Undo restores recorded states, not an assumed
inverse map. Rebasing is explicit and must not require updating every later
anchor per edit or traversing unbounded map chains.

Ranges are ordered, half-open, snapshot-compatible, and may be empty. Preserve
selection direction and character/line/block semantics. Convert Vim inclusive
endpoints once before operators consume them. Block edits are one atomic
transaction; moved/discontiguous content may require range sets rather than
mapped endpoints. Source patch ranges stay within one artifact part.

Invalidate changed dependencies only. Global syntax dependencies may widen work,
but must explain why; a first interactive viewport cannot require unrelated
formatting without such a dependency. Layout-only changes cannot invalidate
source/semantic projections. Background work is cancellable and revision checked;
stale results never replace current state. Cache eviction cannot affect
correctness. Resolve positions/line boundaries logarithmically apart from local
Unicode work, and preserve forward/reverse semantics when changing transforms.

## Styles and rich-text behavior

Styles are portable configuration, with stable identities independent of names
or array indexes. Paragraph/container and character roles remain distinct;
character styles cannot change paragraph geometry. Use sparse declarations:
absence inherits, while explicit normal/none/zero/transparent values override.
Clearing removes the declaration at the requested layer, not a guessed ancestor
value. Unknown source declarations remain preserved even without layout support.

Base Paragraph is the undeletable paragraph root and default assignment.
Default Paragraph is the character-menu choice for **no named character style**,
not a stored/editable definition. Character styles inherit the current paragraph
appearance, then their sparse parent declarations; they must not reapply Base
Paragraph over a heading. At most one named character style applies at a point;
direct properties may overlap independently and never mutate the named style.
Inheritance is single-parent and acyclic; diagnose invalid parents/roles and
fall back deterministically without discarding source. Style deletion must
atomically reassign all children/content or reject dangling references.

Resolve paragraph/context appearance before sparse character styles and direct
formatting; source-authored defaults outrank generated ones. Clearing a built-in
style's declarations must expose inheritance, not hidden copies of its original
defaults. Keep relative sizes relative in saved configuration: paragraph
percentages use the parent's effective size; character percentages use the
underlying paragraph/text size before the character chain and apply once.
Base Paragraph permits only absolute sizes. Bold is distinct from base face
weight. Unspecified colors resolve through the active theme at paint time;
explicit black/white are not “Default.”

Container nesting retains paragraph styles. Lists are structure with synthetic
markers, not bullet text or paragraph flags; preserve independent adjacent lists
and deeper imported levels. Container text defaults cascade, box properties do
not. Source-root canvas context is separate from named styles/view margins.
Reserved features (justification, pagination, custom tab stops) remain unsupported.

Document formatting may write only Markdown's representable block/inline
vocabulary. Displaying imported/theme properties does not grant editing support
for arbitrary font, color, OpenType, or paragraph properties. Style-definition
editing is separate configuration and retains its full normalized properties;
synthetic read-only definitions remain read-only.

- Insertion inherits character context from caret affinity, using the only
  interior side at paragraph edges. Empty paragraphs retain explicit paragraph
  and typing context. Hidden syntax adds no insertion/deletion stops.
- Replacement inherits the first selected text character's pre-edit character
  context, independent of direction, skipping paragraph separators but not
  spaces. Separator-only selections use pre-edit insertion context and must not
  inherit a neighboring link. Fully consumed interior styling cannot leak into
  a replacement beginning in ordinary text; a replacement beginning in styled
  text keeps that context. Explicit pending typing choices take precedence.
- A whole-paragraph replacement retains the first paragraph's style/direct
  declarations, including whole-document replacement. Joins keep the first
  paragraph's style while retained text keeps its inline formatting.
- Pending character choices are view-local until text commits, without source,
  dirty, or undo changes. Named-style choices in Normal mode carry into the next
  Insert/Replace session; failures preserve pending choices. Movement/selection
  retires them. Do not create empty source scopes for pending formatting.
- Selected formatting remains inside paragraph/heading/list/code owners and
  preserves unselected content and spelling.
- Paragraph splits copy style/direct declarations except terminal Enter uses
  the following style. Rich `o` and `O` use the origin's following-style rule;
  counts/repeat evaluate it for each new paragraph. Opening/styling/typing share
  one Insert undo unit; literal views retain source-line behavior.
- Backspace at a code paragraph's visible start removes code treatment. Structural
  resets must remove enclosing layers that would retain that treatment. List
  boundaries follow the structural rules below; ordinary paragraph-start
  Backspace joins preceding content or does nothing at the start of the
  document. Forward Delete removes the following grapheme/boundary except
  at EOF; selection deletion joins crossed boundaries into the first paragraph.
- At a nonempty ordinary paragraph's end, choosing Block quote opens a blank
  quote after it, reusing an empty final paragraph where possible, atomically.
  Enter continues nonempty quotes and removes treatment from empty quotes.
  Backspace at quote start follows structural joining; quote removal preserves
  inner treatments. Whole-document deletion leaves an empty ordinary paragraph
  and preserves document metadata.

Typography supports proportional advances, ligatures, combining marks, emoji,
fallback, bidi, and mixed sizes. Kerning is always enabled, including previews;
zero tracking/authored `kern` cannot disable it. Preserve authored syntax but do
not expose a kerning setting. Edits/search/registers use logical order; visual
movement uses visual order/affinity. Row metrics account for all fragments.

## Markdown compatibility and passive HTML

Target GitHub's rendering of repository Markdown, using the
[GFM specification](https://github.github.com/gfm/) and GitHub's documented
rendering features. Source and WYSIWYG share interpretation; Source exposes the
markup. Missing constructs are compatibility gaps, not alternative rules. Keep
[MARKDOWN_GAPS.md](MARKDOWN_GAPS.md) and
[the feature demo](docs/markdown_demo.md) current. The gap inventory is the
reference for deferred constructs and known incompatibilities.

Deliberate presentation exceptions:

- Images, reference links, and reference definitions retain literal bracket
  syntax in WYSIWYG with the light-purple `Markdown reference` style (`#A673D1`);
  image resources are never loaded.
- Comments remain visible with `Comment` styling. Surplus separator lines retain
  editable empty paragraphs rather than collapsing to GitHub's presentation.
- Thematic rules are non-text furniture with an editable paragraph boundary.
  Quote depth/list ownership are independent of a paragraph's heading/code style.
- Embedded HTML is passive semantic formatting. Markdown within HTML blocks
  stays literal; authored CSS is ignored. Unsupported literal syntax is retained.

HTML utilities serve Markdown, clipboard import, entity decoding, and export.
They must never execute active content, fetch resources, install authored
stylesheets, or use a browser to determine rendering. Clipboard import may map
supported inline CSS into resolved styles with plain-text backing; it does not
create an editable HTML document. Unknown/hidden content remains anchored to
its source. Nearby text edits cannot discard or move it; edits that cannot
preserve it at an equivalent boundary must fail explicitly. Canonicalization
may replace only the smallest formatting construct required by the intention,
never an unrelated ancestor or whole document. Test passive behavior, escaping,
semantic styling, and the absence of resource loading.

Only explicit Open link interaction launches a target. Resolve it from current
source and relative to the document URL; reject stale targets, control characters,
and executable schemes. HTTP, HTTPS, and file targets use native URL APIs, never
a shell. Preserve existing percent escapes and encode invalid bytes once.

## Markdown authoring and structural editing

Markdown opens in Source view. Source and WYSIWYG share one serialization;
switching is an undoable buffer transaction that preserves bytes and recovers
each view's caret and viewport through provenance. It must not reset scrolling
or become quadratic; test first switches in both directions on large documents.
Insert/Replace affinity follows the adjacent content when closing syntax appears.

- WYSIWYG prose folds ordinary source endings to spaces. Pairs of separator
  endings create paragraphs; surplus pairs retain editable empty paragraphs,
  and an unmatched terminal ending is trivia. Source exposes markup and ordinary
  continuation endings but represents paragraph separators without extra raw
  blank rows. Continuations share paragraph style and spacing. Code endings and
  whitespace remain literal in both views.
- Enter in prose creates a paragraph with two current-fileformat endings.
  Shift-Enter creates an internal hard break in WYSIWYG (Markdown backslash
  break, or inline `<br>` where a physical ending would alter structure).
  Source Shift-Enter and preformatted/Text/Code input use literal line endings.
  Keep breaks within their quote/list owner.
- New emphasis uses `**`/`*`; new headings use ATX `#` plus a space. Preserve
  untouched alternative spellings. Removing heading, quote, list or Code Block
  treatment must preserve paragraph boundaries with minimal supporting syntax.
  Preserve body-leading WYSIWYG spaces/tabs with numeric references when literal
  whitespace would become structural indentation. Escape typed reference syntax;
  code and source views retain their literal spelling.
- Code Block assignment fences each selected paragraph with a sufficiently
  long delimiter. Clearing fences removes their language annotation; clearing
  indented code removes its code indentation. Preserve literal body text and
  boundaries through supporting escapes/breaks. An indented block may become a
  fence when needed to express empty or leading/trailing blank code lines.
- A code block is one container with one literal paragraph, internal hard breaks
  and spacing around the whole block. Rich Code/Code Block styles are independent
  of the Code format's global syntax sheet. List/quote ownership survives a
  contained paragraph's heading or Code Block style.
- WYSIWYG labels and their gaps are layout decorations with no text, register,
  selection or caret positions. The first body grapheme is first editable text.
  Label gutters are independent of body indentation; hanging labels must not
  shift body alignment when an ordinal gains a digit. Tight/loose spacing follows
  structure. Source markers remain literal and editable.
- List actions affect whole items and their descendants. Indent requires a
  preceding sibling and cannot create nesting beyond four levels; unindent
  requires a parent, but imported deeper lists can be unindented. Enter and
  `o`/`O` continue semantic items, including Source continuations; Enter on an
  empty item exits to an independent ordinary paragraph. Splitting inside a
  visible prefix edits that syntax rather than duplicating it.
- Insert-mode Tab/Shift-Tab anywhere in an item, including continuation
  paragraphs and Source marker boundaries, invoke structural indent/unindent.
  An unavailable action leaves it unchanged. Backspace at a WYSIWYG item start
  unindents nested items or removes top-level list treatment. A continuation
  paragraph start joins the previous visible paragraph, even across a child
  list; it is not another label boundary.
- Semantic continuation, whole-item deletion, toggling and nesting renumber
  affected ordered runs in source in the same transaction. Preserve the first
  surviving run's starting ordinal; a paragraph splitting a run starts the lower
  run at one, and rejoining continues the first run. New nested ordered runs
  start at one. Adjust owned indentation when number width changes. Preserve
  delimiters, spacing, encoding and unrelated lists. No-op saves and ordinary
  literal typing never renumber.
- Generated ordered labels use decimal/lower-alpha/lower-roman/decimal by depth;
  bullets use disc/circle/square/disc, with the last style for deeper imports.
  Alphabetic labels continue after `z`; unsupported Roman values use decimal.
  Newly authored source lists use `- ` or decimal markers.
- Linewise deletion of complete items removes their structure and selected
  boundaries. Characterwise deletion or replacement of the full body retains an
  empty item; partial visual-row deletion retains its owner. Splits, opens, joins and
  removals retain unselected whitespace, hard breaks, empty edge paragraphs and
  following items. All supporting patches share verification, position maps
  and undo with the initiating action.

### Authored input and caret formatting

Smart quotes are off initially and convert straight quotes consistently in
committed prose input, including replacement, paste/register puts, IME and
accessibility. Existing curly quotes remain unchanged. They never
alter Code, rich code spans/blocks, command prompts, uncommitted IME text or
syntax-required quotes, including within a single input batch. Hidden syntax
does not contribute prose context. Preserve entered straight quotes if context
is unavailable within a bounded query or conversion cannot be encoded. Quote
conversion and insertion are one undoable edit.

With no selection in Insert/Replace, character-formatting actions set a sparse
view-local typing override at the exact caret without changing source or history.
The next insertion applies text and formatting atomically; repeated typing keeps
it, explicit movement/mode exit/projection change retires it. UI reports inherited
style plus pending overrides, excluding automatic syntax colors. Turning off a
property at its closing boundary exits that context (crossing closing markup in
Source); doing so inside text splits subsequent formatting without skipping text
or losing unrelated nested styles. IME commit includes the override.

Replace Backspace restores original local source patches, including formatting
and delimiters, rather than reconstructing plain text or saving a whole document
per keystroke. macOS Option-I intentionally overrides its dead key to toggle
typing Italic in Insert/Replace with no active composition; other native fields
retain normal keyboard behavior.

## Code and syntax highlighting

Code always displays decoded source literally, including markup, comments,
indentation and empty lines. It has no syntax concealment, folding, paragraph
flow, source-backed formatting, manual style assignment or pending typing style.
Each source-line break remains a hard line and paragraph; no final newline is
invented. Rich paste uses plain text. All input paths preserve supplied quotes,
regardless of Smart Quotes or highlighting availability. Markdown escaping and
prose/list continuation never run. Syntax decorations never enter saved source,
clipboard payloads, registers, dirty state or document undo.

Explicit format selection wins on open; existing Markdown defaults remain.
Otherwise recognized code filenames or load-time markers may select Code instead
of Text. Language and format are independent: detecting Markdown inside Code
must not switch to WYSIWYG. Automatic detection uses safe modelines, user then
bundled filename rules, shebangs, then bounded content signatures; an explicit
language override (including None) wins. Unknown explicit markers remain
unavailable selections instead of falling through. Detection runs on load,
explicit redetection and relevant filename changes, never on ordinary edits,
scrolling or highlighting completion. Markers are data, not executable Vimscript
or arbitrary option settings. See the [detection profile](src/core/document/syntax/detection/PROFILE.md)
for bounded sampling, supported marker syntax and registered rules.

Use the pinned bundled Vim runtime, with confined includes and no installed-Vim,
network or user-selectable-directory dependency. Native packages verify it and
retain original bytes/licenses/provenance. Missing resources cannot prevent
editing. Keep pinned upstream Tree-sitter queries unmodified; add required host
support rather than change their semantics. Consult existing provider/profile
contracts instead of duplicating package and implementation inventories:
[Tree-sitter](src/core/document/syntax/treesitter/PROFILE.md),
[Vim](src/core/document/syntax/vim/PROFILE.md),
[Windows packaging](src/win/README.md).

Vim script uses bundled Vim syntax. Other languages prefer a compatible
Tree-sitter package, with ready Vim coverage or default styling when the primary
provider is missing, incompatible or over budget. Fallback fills unavailable
coverage, never gaps between captures or undefined style names. A completed
query with no captures clears previous fallback colors. Exact, provisional and
missing coverage are distinct; errors in parsed source do not themselves make a
completed parse provisional. Unsupported query handlers or Vim atoms must report
compatibility failures, not silently succeed with different semantics. Vim syntax
patterns, Neovim query predicates and Viem Regex v2 are separate languages.

Opening, drawing, editing and scrolling never wait for syntax work. Analysis,
retries and retained state must be bounded, cancellable and shared across views;
no document/giant-line flattening or whole-document highlighting for an exact
scrollbar. Exact repair may reach EOF but must yield. Never truncate backend
coordinates or split arbitrary code into false independent parses. Native
callbacks do not promise hard in-process preemption.

Publish only current results. Invalidation includes text predicates, failed
matches, context and injections beyond structural changes. Unknown injected
languages leave parent highlighting intact; available children overlay parent
runs, retaining parent runs in gaps. Test stale results, empty coverage, fallback,
dependency repair, long lines and large-document locality.

### Global Code styles

Code uses one live application-wide stylesheet in the selected theme,
respecting the configuration directory override. Style edits have their own undo
session and affect every Code buffer without entering document history.
Document style/default-saving and manual content-formatting actions remain
unavailable. F8 and Edit Styles open this global target and follow the originating
view's selected style without parsing unvisited text or waiting for syntax.

Code defaults to system monospace at 14 layout units, single spacing and zero
paragraph spacing/indents. Providers supply character style names, never
paragraph geometry. Names are case-sensitive; Tree-sitter removes `@` and
uppercases only the first letter. Canonical names share definitions across
providers; dotted built-ins inherit from their immediate dotted parent.
Resolve capture precedence deterministically before style lookup.

Missing names inherit the nearest defined dotted ancestor or default appearance,
never a losing capture. Accepted syntax generates bounded empty implicit
definitions/ancestry without changing appearance. They appear in Styles, survive
reload in memory with stable identity, and persist with necessary ancestry only
when edited. Saved definitions survive restart before their names are emitted.
Deleting an emitted definition permits empty regeneration; renaming ends its
old syntax association. Suppress removed/renamed built-in declarations across
reload. Preserve migration/user overrides without rewriting files on load.

Read themes on selection/startup and explicit Reload, without polling. Reload
republishes to open buffers even if the file appears unchanged. Missing files
select Default; malformed or oversized files report an error without overwriting
them. An in-app edit detecting an external change refuses its write and reloads.

Paint-only changes must reuse shaping/wrapping; metrics changes invalidate only
affected dependencies. Implicit definitions and unrelated paint coverage cannot
discard valid height estimates. Retain surviving accepted colors through exact
edit/history maps while replacement work is pending, including discontiguous
edits. Inserted/uncached text may use defaults; ordinary typing must not flash
existing text unstyled. Clear incompatible language/provider state and reject
stale results/layout.

Preserve a visible editing row's screen baseline through local edits, composition
commit and asynchronous font changes; reveal only as needed, never recenter or
jump to the bottom. A taller-than-viewport row prioritizes a stable visible
baseline. An offscreen caret preserves the viewport anchor. Missing regional
layout is not a document edge: obtain local coverage before clamping, without
laying out gaps to an offscreen caret. Test wrapped/unwrapped and long-line cases.

## Canvas, wrapping and line meaning

The interactive canvas is continuous and unpaginated. Pages, columns,
headers/footers, footnotes and widow/orphan rules are outside interactive layout.
Document padding, view chrome and zoom determine usable width. Paragraph start
and end follow writing direction; first-line indent is relative to start indent,
including hanging indents. Indents and spacing never manufacture source breaks.

Normal-flow boxes support automatic width, solid borders and CSS vertical margin
collapse. Floats, positioning, explicit sizes, border radii, formatting contexts
and justification remain unsupported. Paint nested backgrounds/borders parent
before child with source-over alpha, no double-painted slices/corners, and zoom
applied once. Negative margins are exact unless they reverse row order or precede
the canvas origin: use a one-layout-unit forward advance, scaled by zoom, with a
range diagnostic and unchanged source. Empty paragraphs retain caret geometry.
Exact line spacing retains unclipped ink bounds.

Wrapping is per view and never changes source. It uses Unicode word/line-break
opportunities, with no separate character-wrap/`linebreak` mode. An unbreakable
segment overflows through its next legal break or hard-line end; neither a
grapheme nor an internal cache boundary creates an extra break. With wrapping
off, retain horizontal scrolling. `textwidth` controls explicit reflow only.

Usable-width changes reflow visible rows exactly for the next frame, reusing
width-independent shaping; offscreen heights may remain estimated. Preserve the
viewport text anchor and caret visibility. Live resize stays bounded to visible
work even after estimates reset. Test fresh-layout equivalence, invalidation and
large documents.

Each view independently selects Visual or Physical Source line meaning,
initially Physical Source in Code and Visual elsewhere. The status location
toggles the choice with eye/file icons. Visual counts displayed rows; Physical
Source counts actual source-line tokens, including hidden syntax. This policy
applies to ordinary line motions/operators, line-oriented insertion and Visual
Line, including counts/replay. Explicit `g` visual motions retain their visual
meaning; Ex addresses retain formatted hard-line meaning. `gq`/`gw` implicit line
motions count complete hard lines. Native Select All always selects the entire
formatted document through its terminal boundary.

Visual motion retains desired x; physical motion retains checked source column,
including invisible destinations. Visual-row deletion invents no newline;
complete paragraph/list deletion is structural, and a label-only row clears its
list. Physical deletion/registers use authoritative source spelling. Partial
visual rows remain characterwise. Location reporting must not lay out preceding
text: show exact `hard line · visual row` if a global visual ordinal is unknown.

### Carets and display-space selections

Normal and Visual modes use a filled block; Insert, Select and native Selection
use a thin caret; Replace uses an underline; command input has its own thin
caret. Escape cancels pending command state without partial edits. Native
Selection is distinct from Vim Select/Visual. Core determines whether the cursor
occupies an actual grapheme or an insertion boundary; frontends must not infer
this from offset and affinity. Affinity chooses the visual side of a boundary,
not which character a block covers (especially after `$`/End).

Use exact associated-item geometry, including indivisible shaping clusters,
without expanding logical ranges or assuming fixed monospace cells. Empty
lines/documents and EOF use resolved typing-font geometry with minimum half-em
width. Leaving Insert on a final empty paragraph keeps that boundary.

All caret forms use the theme caret color. Monochrome blocks redraw overlapping
ink in the higher-contrast black/white by relative luminance; color glyphs/objects
retain their appearance beneath translucent fill and solid outline. Blinking
must not repaint entire long paragraphs. Inactive carets are nonblinking hollow
outlines at 75% opacity; active blinking follows accessibility preferences. Keys
and motion reveal the caret and reset blinking.

Visual Block uses a display-space rectangle with stable row anchors and x edges.
Recompute exact row intersections after width/wrap changes; never retain row
ordinals or flatten to one character range. The full range set edits atomically.

## Vim commands, search and text editing

Use Vim 9.2 for supported command grammar, counts, registers, operator/motion
composition, cancellation and observable semantics, except the deliberate
requirements here. Find supported-command/option inventories in the
[command implementation](src/core/command) and
[command tests](tests/all/vim_command_matrix.rs). Unsupported commands fail
non-destructively. Do not claim a command without
counts, registers, operator forms, undo and edge-case tests. Sentence/paragraph
motions use portable Unicode-aware human-language rules with Vim blank-line
behavior, pinned in fixtures rather than delegated to frontend segmentation.
Control-Home/End always target document boundaries, preserving input mode or
extending Visual selection; in a prompt they target prompt boundaries.

### Search and Viem Regex v2

Search, operator searches, substitute and sort use the stable **Viem Regex v2**
contract, not full Vim regex or whatever a library happens to accept. Preserve
pattern spelling and dialect identity in history/repeat/cache behavior. The
accepted grammar and rejection cases are pinned by
[search regex tests](tests/all/search_regex.rs) and
[command tests](tests/all/regex_commands.rs). Changes to those tests must not
silently widen or change the dialect.

Patterns are Unicode scalar based with mandatory Unicode mode, conventional
unescaped groups/alternation/quantifiers, Unicode classes/properties, named
captures, and inline/scoped `i`, `s`, `x` flags only. Reject unsupported escapes
and Vim-only magic modes, grouping/quantifier spellings, backreferences,
lookaround, match-boundary controls, editor-position assertions and class
meanings even if the engine would accept them differently. Report the first
unsupported atom; do not rely on a substring blacklist. `~` is literal.

Key compatibility distinctions:

- `^`/`$` always assert formatted hard-line boundaries; `\A`/`\z` assert logical
  document boundaries. Soft wraps and style boundaries contribute nothing.
  Literal U+000A content and semantic hard breaks both match `\n`, but only the
  latter establishes a hard-line boundary. Dot excludes U+000A unless `s` is set.
- `\b`/`\B` are Unicode regex word boundaries; `\<`/`\>` are their directional
  start/end forms, independent of Vim `iskeyword` and word motions. Their word
  class is Alphabetic, Mark, Decimal_Number, Connector_Punctuation and
  Join_Control. Other `\b{...}`/`\B{...}` forms are unsupported; directional
  assertions inside classes are malformed.
- Keyword extraction for `*`, `#`, register-word insertion and completion is
  grapheme based: its first scalar is alphanumeric or `_`. Consequently `*`/`#`
  preserve generated `\b{escaped keyword}\b`, not `\<...\>`; `g*`/`g#` use
  the escaped literal without boundaries.
- Matching defaults to case-sensitive. Option-derived smart case, when enabled,
  examines unescaped uppercase literals outside classes. Substitute `i`/`I`
  override the option default; scoped flags override locally. Use Unicode simple
  folding, without locale-specific casing or normalization.

Search sees logical formatted text, never hidden markup, and crosses style and
source-piece boundaries. A substitute match must fit wholly within its addressed
hard-line span. Without `g`, select the first non-overlapping match starting on
each addressed line; with `g`, select all contained matches. A multiline match
belongs only to its start line. Raw matches may split a grapheme for decoration,
but navigation skips illegal starts. Any selected edit match with an illegal
start/end, including an empty match, rejects the entire command without changing
source, cursor, selection, histories or registers. Never round edit endpoints.
Empty-match iteration must advance to a legal boundary and make progress.

Replacement syntax differs from patterns: `&`/`\0`, `\1`–`\9` and `\g{name}`
insert captures; `\\`, `\&`, `\t` insert their literals. `\n` inserts literal
U+000A content; `\r` inserts a semantic hard break. Captures retain typed breaks,
styles and provenance rather than reconstructing them from bytes. Missing
optional captures insert nothing; undeclared captures fail. Dollar captures,
previous-replacement `~`, case-conversion/expression replacements and unlisted
escapes are unsupported. Preserve these distinctions in dot/macro replay.

Matching must remain linear for a fixed pattern, with bounded compilation,
captures and work, cooperative cancellation and non-destructive resource errors.
Results belong to exact snapshots. Ordinary edits must not synchronously rescan
the whole document because highlighting is enabled. Verify dialect/rejection
coverage, Unicode/decomposed graphemes, typed newlines, multiline replacement,
empty matches, stale work and atomic reverse-edit failures.

#### Search presentation

`hlsearch` and `incsearch` are buffer-shared and default off. Incremental search
also supports counted, Visual and operator searches; substitute previews matches
within addressed hard lines, never replacement text. Preview reveals candidates
without changing authoritative cursor/selection, pending origin, registers or
history. Ctrl-G/T navigate candidates; prompt changes reset that navigation.
Enter accepts; Escape restores cursor, selection and viewport. Invalid, empty,
unmatched or resource-limited preview commits nothing. `:nohlsearch` suppresses
accepted-pattern highlighting until a later search or explicit option change,
without disabling preview or altering the saved pattern.

The internal **Incremental match** style defaults to translucent yellow
background and overlays only explicitly declared properties. It is editable in
the style editor's Internal group but cannot be assigned, renamed, removed or
reparented. It never enters authored content, source or undo. Display may expand
to containing grapheme/shaping clusters while logical matches stay exact.
Metrics declarations invalidate affected layout; paint-only changes reuse
shaping. Matching/preview/invalidation remain portable, cooperative,
snapshot-bound and viewport-budgeted, including multiline context. Test stale
work, style invalidation and bounded large-document behavior.

### Literal input, indentation and whitespace

Keep literal text-input CR/LF distinct from a normalized Enter event through
replacement and replay. Normal `r<Enter>` produces a semantic break; Visual
Character/Line retain Vim's literal CR replacement behavior; Visual Block inserts
one semantic break per nonempty selected row, regardless of width. Unrepresentable
literal control content fails atomically rather than changing hard-line meaning.

Ctrl-V/Q literal-next input follows Vim grammar in Insert, Replace and command
input, before native shortcuts. Normal/Visual retain their block-selection
meaning. Quoted input bypasses indentation, comment continuation and smart
quotes. Quoted Tab stays a tab; quoted control keys do not execute commands.
Quoted Return is literal CR (LF content with Mac line endings); quoted LF/NUL
store actual U+0000, not Vim's internal LF-as-NUL convention. Named special-key
notation consumes only one original grapheme in Replace. Reject invalid Unicode
scalars. Undo, repeat, macros and Replace Backspace retain literal intent.

Text, Code and Markdown Source use logical indentation columns with defaults
`autoindent`, `tabstop=2`, `shiftwidth=2`, `softtabstop=2`, `expandtab`, `smarttab`.
Options inherit application defaults independently per buffer field; explicit
overrides are shared across views and never enter document undo. WYSIWYG breaks
retain format-aware structure. `=` copies preceding nonblank indentation; it
never invokes a language formatter, `cindent`, `indentexpr` or external process.
Vim tab/soft-stop, Ctrl-T/D and temporary-indent behavior must preserve counts,
repeat and restoration semantics. Replace Tab is one restoration unit; Replace
Enter inserts rather than overwrites the following body.

Enter and `o`/`O` preserve logical indentation under the configured tabs/spaces
policy. Generate indentation only at a resulting hard-line start, including the
existing interior-wrap Visual `O` exception. Remove unchanged generated
indentation when leaving an otherwise empty inserted line, while preserving
authored blanks/manual changes. Paste and IME commits bypass per-keystroke
indentation/comment assistance; the shared batch smart-quote policy still applies.

Code comment continuation defaults on separately for Enter and `o`/`O`. It shares
reflow's declarative language profiles and full-line context, independent of
syntax highlighting. Preserve distinct line leaders and established starred or
unstarred block bodies. A new conventional block generates ` * `; typing `/`
immediately after that generated prefix closes it as ` */`. After a closer,
resume opener indentation. A bare leading `*` or delimiter inside code/string
is not a comment context. Context lookup is bounded to 512 preceding hard lines;
outside that bound continuation uses ordinary indentation and `=` uses zero.

Leading ASCII spaces/tabs may use actual space advances or half the default
Paragraph font size per logical column (paragraph en), excluding character
styles and tracking. Code defaults to paragraph en; other formats to spaces.
Nonleading spaces shape normally. Tabs advance to stops, never add a constant
width. Font, zoom and window width do not change logical indentation/reflow
columns. Presentation changes invalidate affected geometry, not text.

Code wrapped continuation rows start at original leading-whitespace advance
plus a configurable margin, initially four units independent of tab/shift width.
Zero retains original indentation. The margin neither accumulates across rows
nor synthesizes text, stops or markers, and is not clamped to fit narrow views.
In all formats, the original ASCII indentation prefix supplies no soft break,
including at its end; keep its first word on the first row even if it overflows.
Code additionally allows breaks after ASCII punctuation runs before nonpunctuation,
excluding quotes; `;` and `(` allow breaks even before punctuation. Respect
legal grapheme/shaping boundaries, independently of highlighting. Full/regional/
long-line layout must agree and avoid rescanning arbitrarily long prefixes.

Visible whitespace follows Vim 9.2 `listchars` semantics and validation, with
view-local `list`/`listchars` overrides of live application defaults. Initially
markers are enabled with `tab:>-,trail:*,extends:>,precedes:<`. The menu toggle
uses the same option. Markers appear only in Text, Code and source views, never
WYSIWYG (including rich Code blocks). They use the application-owned **Visible
whitespace** style, initially dark blue `#00008B`, with application-settings undo.
It cannot become authored/typing style. Marker ink fits existing geometry and
changes no text, caret stops, wrapping, height, search, clipboard, accessibility
or serialization. Exact marker exports must use the same presentation text as
layout, including IME/completion previews. Test options/atomic validation,
inheritance, mixed-font geometry, composition, multiple views, cache invalidation
and bounded large-document work.

### Hard-line reflow

`gq` and `gw` are internal portable reflow operators for Text and Code only.
Markdown Source/WYSIWYG return non-destructive unsupported-format errors until
format-aware semantics exist. No implicit external formatter or Vimscript runs.
`textwidth` defaults to 80 positive Unicode columns and inherits per buffer from
application settings; zero/window-width fallback is unsupported. Changing it
neither reformats text nor enables automatic hard wrapping. Soft wrap and source
Paragraph Flow remain independent.

Resolve normal checked Vim motion/selection endpoints, then expand only to their
intersected hard lines. Visual Block formats each touched hard line once. Doubled
forms and implicit line motions count hard lines regardless of view line mode;
explicit visual motions retain their domains before expansion. Paragraph
recognition must not extend the edit outside the selected lines. `gq` leaves the
cursor at first nonblank of the last formatted line; `gw` restores its original
anchored location. Successful Visual forms leave Visual; reveal only as needed.

Width includes indentation and leaders, uses portable Unicode columns and tab
stops, and is independent of fonts/zoom/window. Preserve graphemes and unbreakable
words/URLs even if they overflow. Join paragraph words with single spaces while
preserving order, punctuation, indentation, blank separators, recognized list
markers/numbering and hanging indent. Do not join adjacent list items or
paragraphs with differing indentation except recognized list continuations.
An overwide prefix must not generate empty lines or repeated splits.

Code comment profiles recognize only full-line leaders. Preserve `//`, `///`,
`//!`, block openers/closers and established interior-star conventions; standalone
delimiter and blank comment lines remain separate. Comment-kind/leader changes
separate paragraphs. Lists inside comments retain hanging indent after repeated
prefixes. Mixed code plus trailing-comment lines remain byte-identical boundaries.
Nested block comments and languages without a declared profile remain gaps;
the former ends at its first closer, and the latter's leaders are ordinary words.
Comment context and reflow cannot depend on asynchronous highlighting or styles.

One reflow is one atomic transaction/undo unit, leaving registers unchanged.
Dot repeat retains extent/count semantics and uses the target's effective width;
undo/redo restore recorded source/cursor state. Source-identical results create
no dirty/history change. Preserve unchanged bytes, mixed line endings, encoding
and final-terminator presence; new breaks use `fileformat`. Do not run generated
text through typing assistance. Failure/cancellation leaves source, registers
and prior repeat recipe intact. Small reflows remain local without flattening,
highlighting or laying out the whole document. Verify Vim counts/composition,
cursor policies, Visual/replay/undo, Unicode widths, comment/list boundaries,
partial selections, serialization locality and bounded large-document work.

## Editing, selection, and command behavior

Text objects and logical operations act on formatted content, independent of
wrapping and font metrics, and retain provenance for source edits.

### Insert, Replace, completion, and native input

- Ctrl-C cancels pending grammar or exits editing/selection/prompt input without
  undoing completed edits or applying unfinished Insert count expansion. During
  deferred block insertion it keeps only the first row's insertion; a preceding
  block change still deletes the entire selected area. During Insert Ctrl-O it
  ends the suspended insertion in Normal, retaining completed text and repeat.
- Preserve native modifiers and original key identity until literal-next and
  pending register operands have consumed their input; resolve portable
  control-key aliases afterward. Ctrl-[ cancels like Escape in every mode.
- Insert Ctrl-E/Y copies a whole grapheme from the neighbouring hard line at
  the caret's tab-expanded logical column. Fonts and wrapping cannot affect
  the copied item; columns inside tabs/wide graphemes select the entire item.
  Missing items are no-ops, tabs remain hard tabs, and copying bypasses typing
  assistance. Dot/count replay retains the copied value; macros rerun the command.
- Replace Backspace and Ctrl-W/U restore overwritten source through replacement
  history and remove appended text normally. They are semantic operations on
  replay, not recorded byte-offset deletions.

Insert Ctrl-N/P completes case-sensitive word prefixes from the current visible
formatted document in forward/backward document order, wrapping once. Use the
same word classes as word motions, deduplicate insertion strings, exclude the
occurrence being completed, allow an empty prefix, and include the original
prefix in the cycle. Completion does not copy candidate styles. Only Ctrl-N/P
navigate: **every other key accepts the displayed candidate and then performs
its ordinary action**, including Escape, Ctrl-E/Y, arrows, Enter, and Tab. This
is a deliberate Vim difference. Replace and deferred block completion remain
unsupported.

Completion preview must not modify source, dirty state, or history. Acceptance
inserts the missing suffix using destination typing style in the current Insert
undo unit; repeat, the last-insert register, and macros retain accepted text,
not navigation or search timing. A failure of the following key must still
report a completion already committed. Search and previews must remain bounded
on huge lines/clusters; publish explicit truncation and never reorder discovered
results. Discard stale search work. Native pointer/menu/IME actions accept through
core before capturing new revision-bound targets. The popup stays anchored to
the word's beginning, follows actual placement changes, and respects bidi direction.

IME/marked text is available in Insert, Replace, native Selection/Vim Select,
and command/search prompts. Composition is a temporary overlay; commit is one
source transaction and undo unit, cancellation restores the prior source and
selection. Normal, Visual, pending command grammar, and Insert Ctrl-O command
execution must not expose a native input context or consume late replacement
callbacks as command text. Mode changes refresh native input context and dismiss
candidates even without a marked range. Escape/native cancel discards an active
composition without changing its previous mode; dismissing an accent picker
must not delete its already committed base letter. Never send partially decoded
input to core.

### Selection and clipboard

Selection shape and interaction policy are independent; preserve policy when
extending. `autoselect` defaults on, entering native **SELECTION** for pointer/
Shift navigation independently of Vim options. These options are profile-wide,
without reclassifying existing selections or pending commands. Native selections
are half-open; Vim selections are inclusive. Native replacement typing/deletion
enters Insert and groups with following typing, while Visual/Select deletion
returns to Normal. Visual Delete/Backspace shares Visual `d` semantics. Retain
the detailed native/Vim entry, collapse, resume, mapping, IME, and platform
exceptions in [`docs/native-selection.md`](docs/native-selection.md).

Native Copy is a dedicated portable intention that preserves selection extent,
direction, active endpoint, mode, and caret. It must not synthesize a yank,
consume pending mappings/registers/counts, or finish a temporary Visual command.
Vim yanks retain their selection-ending behavior. The deliberate `"*c` shortcut
copies to `*`: in Visual it yanks, and in Normal it takes yank's motion/count
grammar; other `c` commands retain change semantics.

Pointer selection and autoscrolling preserve the original anchor and complete
logical range, including outside the window and materialized layout. Offscreen
selection exports remain available even when they have no drawable rectangles.
Mouse-up stops scrolling without dropping the range. Native editing and menu
actions use core selection and editing intentions, never a second frontend
selection or undo authority.

### Registers, repeat, and history

Registers distinguish semantic hard breaks, literal LF, and block row separators
through append and cross-format put. Macro events are separate from inspection
text: never parse/insert displayed `<Left>` notation. Putting non-text macro
keys fails atomically with `MacroContainsNonTextKeys`; representable controls
stay literal (Enter is CR, not a semantic break). Text overwrite removes a
macro program; append preserves it and appends normalized text events.

Dot records semantic changes with accepted payloads/counts. Macros record
normalized core events, preserve key identity, and have bounded recursion/work.
Normal `U` is deliberately unsupported; Visual `U` and `gU` still uppercase.

Undo/redo restores exact previously committed source artifacts and metadata;
it never reruns edits or inverse commands. History branches when editing after
undo and retains the abandoned future. Redo follows the preferred child, with
navigation/creation updating that preference; unavailable targets fail without
partial installation. Source transactions are atomic, undo units may group
transactions, and finalized history states are immutable. Derived layout and
native objects are not history authority.

Required grouping:

- One ordinary change, its count, and all patches of a block/multi-range change
  form one unit. Change-operator deletion plus following Insert input, and the
  line creation of `o`/`O` plus typing, stay together.
- Ordinary Insert/Replace typing, deletion, Enter, and Tab share the current
  unit. Explicit cursor movement closes it. Ctrl-G u closes without losing the
  repeat recipe; Ctrl-G U joins only the immediately following Left/Right within
  the same hard line. Insert Ctrl-O closes before its separate Normal command.
- Paste, register insertion, and each committed IME composition are separate
  units with breaks on both sides; cancelled/updated composition is not history.
- A dot, macro replay, or `:normal` invocation is one unit even with internal
  undo breaks. A later replay failure stops while retaining completed changes
  in that unit. Multi-range substitute/style changes are atomic transactions.

Finalize open units before history navigation, save, buffer close, ending the
editing mode, or an unrelated change. Cancel IME before undo/redo. Failed
transactions and successful source no-ops create no history entry. Source-
affecting metadata changes retain their own documented undo policy.

Undo/redo restores buffer marks and puts the invoking view in Normal with no
selection at the recorded before/after restoration anchor. Other views retain
mode/presentation while remapping cursor, selection, and viewport anchors.
Registers, repeat/macro state, search and prompt history, jumps, view options,
scroll offsets, and filename are not restored; redo does not repeat side
effects. Ordinary edit side effects still commit atomically with that edit.

Dirty state compares exact persisted source identity, not serialized text or
hashes on every query. Saving creates no undo entry; failed writes do not move
the save point. Returning to the saved source makes the buffer clean, including
when the saved history node has been pruned. Configuration-only changes do not
make source dirty. Asynchronous saves acknowledge the captured source identity,
not whichever revision is current at completion.

Retention charges additional history separately from unavoidable live state so
large documents retain useful undo. Total diagnostics still include caches/maps
and count sharing once, without walking unchanged subtrees per edit. Prune
noncurrent leaves before old ancestry, never active/open-unit states. Undo is
memory-only; future persistence must reject source/configuration mismatches.

## Clipboard and HTML export

Internal registers preserve portable rich structure where the destination can
translate it. WYSIWYG Copy publishes plain text, native rich text, and a versioned
private fragment retaining selected source, pipeline metadata, and resolved
styles. Compatible contiguous private paste uses verified local transactions.
Rectangular copies retain each selected source segment separately, exclude
intervening unselected text, and keep plain-text rectangular paste semantics.
Source/Code copy literal selected source only; automatic syntax styles never
enter Code registers or clipboard payloads. Copy Source publishes source markup
as plain text; Paste and Match Style ignores private data. Malformed optional
private data must not break ordinary commands/plain paste. Windows `+` and `*`
share its one clipboard; retry transient ownership contention and fall back to
plain text when optional rich formats are unavailable.

External plain-text paste normalizes CRLF/bare CR to semantic breaks. Valid
private data supplies character/line/block shape; otherwise only text ending
in CR/LF is linewise, per Vim. Linewise clipboard writes end in a break even
when the original final source line did not. Do not rewrite the clipboard or
ask modal questions. Internal exact registers, literal-input/command prompts,
and representable NUL retain their own semantics.

Export writes a separate standalone UTF-8 styled HTML document from any format,
without changing source, format, path, dirty state, history, or saved baseline,
including on cancellation/failure. Never overwrite any open document's source;
HTML source gets a distinct suggested export name. Markdown Source and WYSIWYG
export the same interpreted structure and presentation exceptions. Code exports
escaped literal whitespace/text with whole-snapshot syntax coverage, or default
Code styling when highlighting is unavailable/disabled.

Reuse the normalized style cascade and CSS serializers. Export resolved content
styling and structure, exclude editor furniture, escape source/CSS so authored
text cannot become active markup, and require no network resources. Native
frontends only choose the destination and write the portable core's bytes.

## File lifecycle, Ex, and views

### Persistence and external changes

Save As writes a separate destination and adopts it only after success; it
must preserve the previous file's name and bytes. No format switch or native
rename/move action may rename/delete the original. Ordinary Save preserves its
filename/extension. Format changes remain in memory until a write; changing
serialization family requires Save As and forbids every write entry point from
overwriting the original. Text/Code share a family, as do Markdown/Markdown
Source. Successful Save As establishes the new format baseline.

Reject symbolic-link and case-only destination aliases that would overwrite
the original; distinct hard-link names require replacement preserving the
original's bytes. Ordinary Save to the unchanged bound name remains allowed.
`:file` changes the filename without writing and must not pretend existing
bytes at its new destination were loaded or saved; preserve overwrite checks.
Failed/stale writes retain bindings and dirty state.

Watch file-backed documents for writes, replacements, deletion, and recreation.
Coalesce event-driven checks off the main thread; do not repeatedly reread
unchanged documents with a polling timer. Each changed disk state gets at most
one acknowledged review per buffer across all views. Keep Buffer is the default
and acknowledges only that notification, never the load/save baseline. Further
changes require new choices. Deleted/unreadable files preserve editable text;
inactive-document prompts wait until they can be presented.

Load File explicitly discards unsaved changes in every view after warning.
Consent becomes stale when the buffer, bound path, or disk state changes. Reload
uses the exact verified bytes, retains editing format, and installs atomically.
The dialog alone reports a change; do not add duplicate status messages.
Viem's saves refresh the baseline before queued notifications are reviewed.
Every save path, including forced Ex writes and close-review saves, checks for
external destination changes and asks Cancel (default) or Save Anyway. **`!`
does not bypass external-change confirmation.** Recheck accepted fingerprints
before publication; further changes require new consent. These checks do not
promise filesystem compare-and-swap.

### Ex and prompt gotchas

Ex ranges always address logical hard lines. A Visual `:` range retains the
selected revision and becomes stale after an intervening edit in another view;
never reinterpret its old coordinates. Prompt selection, clipboard, and register
insertion target validated prompt identity and must never edit the underlying
document or submit a command through expanded register text. Command output is
selectable, read-only status-line content, closable/expiring without resizing the
status line or stealing another control's focus. Ordinary editor input dismisses
output and then executes normally; a new `:` replaces it with a prompt.

Keep these deliberate restrictions and differences:

- `:global` selects stable line identities before execution: new lines are not
  targets, deleted targets disappear, moved targets keep identity. One invocation
  is one undo unit; per-line failures report and continue. Nested global is a
  current-line predicate without a range. Host/file requests, history navigation,
  and interactive substitution are unsupported inside it; work is bounded.
- Confirming substitute stages decisions against the original revision and
  commits approved replacements together when confirmation finishes. Source
  stays unchanged while choosing; an intervening edit cancels staged decisions
  with a diagnostic. Its highlight is visible regardless of search options.
  Ctrl-E/Y scrolling during confirmation remains unimplemented.
- Combined text/option changes validate atomically. Formatted Markdown alignment
  is unsupported; literal/source alignment uses logical columns. `:read` decodes
  independently and inserts through the destination adapter; revalidate its
  document/revision after I/O. Shell filters and `++` overrides are unsupported.
- `:source` executes supported UTF-8 Ex commands/mappings in order, preserving
  meaningful trailing spaces and waiting for host operations. Bound file, line,
  nesting, and execution work. Errors identify file/line and stop, retaining
  successful earlier commands with ordinary undo units. Interactive confirmation,
  `:source!`, and general Vimscript remain unsupported.
- Sort is a source-preserving permutation, retaining encoding, final terminator,
  and delimiters. WYSIWYG supports complete single-hard-line Markdown paragraphs
  with their syntax/styles; split or ambiguous owners fail atomically. Literal
  views sort visible source rows and reparse. Float/locale sorting and command
  chaining remain unsupported.

### Opening, launching, and closing

Run one process per user: forward later invocations' original arguments/cwd in
order after initialization, acknowledge them, and report failed handoffs instead
of starting duplicates. Election survives concurrent launches/crashes. Empty
invocations activate an existing window or create a blank one when none remains.
Repeated filenames/symlinks/hard links reuse the same buffer/history and existing
pane, preserving edits; never fork buffers for aliases.

Startup captures filenames relative to launch cwd; only the first opens by
default, later files load on navigation. Nonexistent files start named, clean,
empty without disk creation. `+line` addresses the first file and `-o` opens
stacked panes with it focused. Arbitrary `+cmd` and Ex wildcards are unsupported.
Test fresh-process macOS startup: AppKit must not duplicate arguments as file
opens. Argument position is per pane, nonwrapping, and survives unrelated opens;
splits copy then independently evolve it. Failed/stale opens leave it intact.
`:wnext` writes before even end-of-list errors; failed/cancelled/stale writes
prevent navigation. Alternate writes do not rebind or clear dirty state.

`:edit` replaces the active pane; case-sensitive `:E` opens a native window.
Only replacing a modified buffer's last view needs unsaved review/`!`; reloading
that shared buffer always does. `:only` validates every planned closure before
closing any, unless forced; surviving views preserve shared text. Each distinct
closing document receives one native unsaved-changes review, not duplicate
prompts after Save/Don't Save. Cancellation keeps the document available.
`ZZ` requires successful full save before closing, regardless of count/register;
any rejection, cancellation, stale acknowledgment, or write failure keeps it open.
Successful Ex/ZZ closes terminate the application only when no ordinary windows
remain; ordinary window closes never inherit that termination policy.

Native Open may reuse only an untouched, empty, untitled single-pane window,
retaining its frame. Recovered/previously edited documents are not eligible.
Finder drops replace the receiving clean pane; dirty targets/additional files
open new windows. Drops never insert paths or write files. Read successfully
and revalidate pane/document/revision/dirty state after dialogs or asynchronous
work before replacement; failures preserve the original.

Splits, including `:vsplit`, are **stacked only**. Views share document/history
but own cursor, selection, viewport, wrapping, line mode, status, and scrollbar.
Focus controls save/style/menu routing; final-view closure reviews unsaved changes.
Window commands are core grammar with frontend geometry/focus, create no undo or
register effects, and are not dot-repeatable. Side-by-side/tab-page-dependent
commands are unsupported. Retain normal window geometry across launches,
recover offscreen frames, cascade new windows, and preserve existing frames.

### Editing locks and recovery

Claim an exclusive recovery slot for named documents, with a writable recovery
location fallback. Existing Viem/Vim swap files offer read-only, edit-anyway, or
cancel; offer Recover only for valid Viem snapshots. Never interpret or rewrite
foreign swap bytes, nor replace/remove another session's slot.

Read-only still permits editing, registers, and history; it guards writing.
Unforced writes report E45, explicit forced writes permit it, and native Save
asks Save Anyway/Cancel. External-change consent is still required. Recovery
restores full source and interpretation metadata under the original filename
and starts dirty even at history root. Only successful current Save/Save As
clears recovered dirty state; failed/alternate writes do not.

After idle editing, write complete immutable source plus interpretation metadata
to the owned recovery slot off the UI thread; never autosave over the original.
Superseded jobs cannot overwrite newer backups. Use exclusive private temporary
files, synchronize, and replace atomically. Save As rebinding retains the last
valid backup until a current backup commits for the new target. Failed opens or
rebinding cannot delete a valid backup. Document close removes only owned slots;
closing one shared pane is not document close. Crash leftovers remain recoverable.

## Configuration and themes

Application preferences have one versioned JSON authority, normally
`~/.viem/config.json`. Resolve the profile centrally: explicitly injected path,
then `VIEM_CONFIG_DIR`, then `~/.viem`; all consumers share it. Validate complete
candidates, write atomically, preserve unknown keys (including nested keys), and
leave invalid/unsupported files untouched with diagnostics. Settings update open
views and future defaults without source edits or document undo changes.

Themes combine appearance and format styles as application-wide presentation
configuration, with settings undo independent of document undo. Named-theme
edits save automatically. Default is editable in memory with no save path;
New theme copies current values, including unsaved Default edits. Names must be
unique, portable, and filename-safe; retain filename identity when display names
collide. Missing selected files use Default. Preserve legacy files when importing
them into a named theme; missing resources must not prevent loading.
**When changing `assets/themes/Midnight.json`, update the built-in defaults too;
tests must compare the complete preset against code defaults.**

Defaults remain sparse and source assignments/direct declarations take precedence.
Source/WYSIWYG share defaults. Loading or changing themes never writes inherited
values into source, changes dirty state, or adds document history. Reject the
smallest independently invalid saved declaration/definition with diagnostics,
retaining valid settings; invalid JSON/unsupported versions retain existing
settings. Loading must not rewrite files or revive obsolete style definitions.
Color changes need repaint only; font/paragraph changes invalidate affected layout.

View margins are independent of themes and source canvas padding, and add to
that padding. Changes preserve viewport anchors and invalidate only affected
geometry. If the active row already fits between top/bottom margins, typing and
reveal must leave displayed rows stationary; otherwise scroll only enough to
reveal it. A cached region boundary is not a document edge. Margins are not paint
clips; scrolled content continues through them. Include the last row's full ink
and natural height before bottom scroll padding, even under exact line spacing.
If row plus margins cannot fit, prioritize making the row visible.

`startup.viem` is optional UTF-8 configuration, loaded once per profile/startup
after JSON preferences and never rewritten by Settings. Missing files are silent;
invalid lines report path/line and do not block later valid commands. Accept BOM
and CRLF; bound input to 1 MiB. It supports configuration Ex commands and mappings,
not edits, file/window actions, or Vimscript. Reject `fileformat` assignments
because they change source and `fileformats` assignments until startup can affect
pre-open decoding. See [startup guidance](docs/macos-development.md#startup-commands).

Native and Ex operations share one persisted recent-file list. Successful
opens/saves/reopens promote canonical file identity without symlink/hard-link
duplicates; failures, recovery autosaves, and unopened-copy writes do not.
Missing files remain until cleared/displaced. Preserve full paths as targets and
merge updates without overwriting unrelated settings.

## Native controls and style inspector

Native controls share core intentions/history; frontend undo adapters only
proxy it. Standard edit selectors follow native focus, including settings fields.
Preserve AppKit Home/End bindings with Viem's line policy and distinct document-
edge selectors. Consume preference-adjusted wheel direction once. Provide native
contextual clipboard access; never make bare Vim keys global menu equivalents.

There is no Format menu. Supported inline formatting lives in the toolbar;
Style retains named styles, lists, and independent definition editing.
Document-formatting actions are capability-limited, not an arbitrary property-
editing API. Menus and toolbar describe the focused document's actual
named/structural/inline state, including mixed values; Code style inspection
uses current cached syntax runs without parsing or whole-document scans.
Native menu tracking must retain item identity while the user holds a button.
Windows Heading 6 remains menu-only because Control-6 belongs to Vim Control-^.

The toolbar is absent in Text/Code and remembers visibility per Markdown format.
Its Formatted view toggle uses source-preserving Markdown switching, stays
reachable in narrow windows, and follows shared changes/history. The status line
has no format popup/label; its caret widget remains right-aligned beside prompts/
output. Flow Source Paragraphs is initially-off Markdown Source layout only,
never a text-coordinate, serialization, or history change.

Choosing/rechoosing a named character style clears direct declarations and
inline traits in the same undoable transaction; a caret changes future typing
only. Default Paragraph clears named character assignment and direct traits.
Inheritance and semantic Link defaults still apply to unspecified properties;
links and unselected source remain intact. Toolbar refresh must not construct
candidate edits, reparse, or scan unrelated content; retain bounded local queries.

### Inspector ownership and interaction

The inspector edits independent theme definitions with its own undo, never
document source; Code retains global ownership. Use native role-sensitive
controls and real resolved/shaped preview, without a private editable stylesheet.
[`docs/Word style.png`](<docs/Word style.png>) is a density/composition reference,
not a template/automatic-update/Apply/OK/Cancel workflow.

One modeless inspector exists application-wide; F8 opens/reuses and retargets
it without changing document mode. Select the current style immediately, then
follow actual caret/selection changes only in the originating view: single
nondefault character style, otherwise paragraph style, otherwise Base Paragraph.
Never infer style from fonts or choose an arbitrary first mixed style. Explicit
choices survive unrelated refreshes. Coalesce following; never poll, parse, or
scan large documents just to follow the caret.

Track target and stable style identity; revalidate before every mutation and
callback. Deleted selections fall back to Base Paragraph, never a reused menu
index. Cancel pending work on retarget/closure. Closing a followed Code view
only detaches following, retaining the global target. Closed document targets
must not accept retained callbacks.

Valid changes apply live; invalid intermediate text remains local with validation
and the last committed value intact. Choices are one undo unit, continuous
gestures coalesce, and refreshes/merely opening native pickers create no edits.
Closing never rolls back. Native font/color panels stay modeless and retain the
same target and undo authority. External changes refresh controls/preview without
manufacturing another edit.

Overrides distinguish inheritance from explicit normal/zero/transparent values.
Activation starts from resolved values; clicking a disabled supported control
also performs its original action as one undo gesture, while captions only
enable. Base Paragraph stays complete, parentless, and undeletable; types and
acyclic inheritance remain valid. Deleting an in-use configuration style falls
back to Base Paragraph/Default Paragraph, not its parent, atomically; Code keeps
its name-reference/suppression policy.

Keep family/fallback, explicit face/base weight, and semantic Bold independent;
removing Bold retains the base face. Family changes preserve matching face style
when available, fallback order, and Bold. Native substitution must not masquerade
as an unavailable family's catalogue or lose an explicit same-metrics face.
Relative-size unit changes preserve appearance; show transparency distinctly.
Displaying controls never rewrites source declarations.

## Mappings and explicit scope limits

Interactive mappings are buffer-wide; startup mappings initialize every buffer.
Preserve literal-operand bypass, nonrecursive replacement, trailing spaces, and
bounded recursion/prefix waiting. F8/Shift-F10 retain native actions. Listing,
mapping-command abbreviations, attributes (`<expr>`/`<buffer>`), and user
abbreviations remain unsupported.

Do not build speculative Vimscript/Vim9script, plugin/runtime execution,
terminal jobs, shell filters/`:!`, tags, quickfix, diff, folding, spellchecking,
Vim tab pages, side-by-side splits, sessions/viminfo, remote server commands, or
full Vim option/regex parity. Adopted syntax providers and supported Vim syntax
loading are exceptions only for syntax highlighting; they do not authorize a
general runtime, Neovim Lua plugins, arbitrary autocommands, syntax-driven
folding/concealment, or changing the Viem Regex dialect.

## Layout and responsiveness

Core owns layout, wrapping, hit testing and scroll policy; native shapers provide
authoritative font resolution, clusters, caret stops and metrics. A string-width
API is insufficient. Fragmentation must preserve shaping context, bidi, fallback
and ligatures. Layout geometry is valid only for its exact snapshot and metrics
generation. Missing visual stops never enlarge a logical edit's grapheme range.

- Line breaking uses the default Unicode 15.0 UAX #14 rules in portable,
  first-party code, without the optional numeric-expression tailoring or an
  external/copied implementation. Unicode updates require deliberate data
  regeneration and conformance review. Hyphenation remains deferred. See
  [Unicode data and conformance policy](data/unicode/15.0.0/README.md).
- Edits invalidate only affected content and required context. Width changes
  preserve valid shaping; paint-only changes do not reshape or rewrap.
  Global invalidation must not walk the document. Caches are bounded and optional
  for correctness; unchanged geometry should be shared across snapshots/views.
- Shape and wrap huge hard lines in bounded work, with exact continuation
  context. Partial capture edges must not become invented line breaks. Never
  assume a whole line is small enough to flatten or shape as one object.
- Layout-dependent input must obtain missing exact coverage and finish the
  same uncommitted command without swallowing or replaying its keystroke.
  Retrying disposable layout must not repeat edits, register effects or undo
  grouping. Missing coverage is distinct from a fully materialized document edge.
- Scrolling uses continuous layout units and one core-owned, margin-inclusive
  extent for wheel, keyboard and scrollbars. Estimates are not authoritative
  clamps. Repeated endpoint paging is stationary; explicit scrolling must not
  be undone by a subsequent generic caret reveal.
- Preserve a text anchor and fractional row offset through reflow, projection
  changes and height refinement. Typing and asynchronous syntax/layout completion
  preserve the editing row's screen baseline while it fits; clipping permits
  only the minimum reveal. Fill missing local coverage before restoring anchors.
- Scrollbar dragging leaves the caret in place. Horizontal range comes from
  overflowing rows in the current viewport, not overscan or wider distant rows,
  and is clamped after vertical scrolling. Scrollbar updates must not scan or
  shape the document or cause visibility/reflow oscillation.
- Pointer selection and scrolling within unchanged materialized coverage reuse
  exact geometry. Selection/caret changes alone must not reshape text or recopy
  immutable viewport exports. Validate cached drawing against all relevant
  snapshot, viewport, metrics, theme, device and composition changes.

Complexity and bounded work are release requirements:

- Local editing is logarithmic plus changed bytes, transformation dependencies,
  local Unicode/shaping work and visible reflow. Typing in a short visible line
  must not scale with total bytes, lines, anchors or visual rows.
- Opening may scan for detection/indexing, but first interaction must not wait
  for unrelated full-document semantic projection, shaping or wrapping unless a
  declared global dependency requires it. Syntax availability must not block it.
- Resize and distant scrolling perform bounded visible work, never eager
  whole-document invalidation or shaping the intervening prefix.
- Test a million short lines, 100 MiB mixed-encoding documents, multi-megabyte
  single lines wrapped/unwrapped, mixed fonts/bidi/emoji, two different-width
  views and rapid resize followed by editing. Check actual materialization,
  context capture, scratch/IME preparation and commit, not only the final edit.
- Assert work counts, cache/queue bounds and stale-result rejection. Keep
  explicit measured fixture ceilings; do not hide regressions by raising budgets
  or omitting native allocations. Distinguish retained-heap estimates, actual
  allocations, peak memory, RSS and complete native-app latency. Timing alone is
  not a structural bound, and a mock shaper is not evidence of native latency.

## Native frontend requirements

Native controls are input/rendering adapters, never another document, selection
or undo authority. IME marked text remains an overlay until explicit commit;
native accessibility edits use the same checked semantic transactions.
Validate actual native event routing, geometry and rendering, not just calls to
the Rust wrappers.

### macOS

- Use AppKit, Core Text and the native `NSTextInsertionIndicator` for thin Insert
  and command-line carets. Preserve its system blinking, accessibility and input
  accessories; draw other modes with exact core geometry. Only the appropriate
  native/custom caret is visible. Both use the same theme-resolved caret color;
  system colors follow appearance, accent and contrast changes dynamically.
- Keep AppKit UI on the main thread. Expose text, ranges, selection and geometry
  to VoiceOver and native text services without introducing native text storage
  as a second source of truth.
- Follow the system scrollbar preference live. Legacy vertical bars reserve a
  stable gutter; horizontal controls overlay the full-height canvas. Painting,
  layout, hit testing and IME geometry must agree on that canvas.

### Windows

Follow macOS presentation and portable behavior with WinUI controls and these
deliberate platform differences. Build/test and packaging instructions live in
[src/win/README.md](src/win/README.md); the screenshots in `docs/mac_references`
are the visual reference. Use the shared compact control density in all app
windows/dialogs; document typography and native system pickers are independent.

- Ctrl-C/X/V always mean Copy/Cut/Paste, even during literal-next input;
  Ctrl-Shift-V pastes plain text. Ctrl-Q remains the Visual Block/literal-next
  alternative. Ctrl-S/Shift-S and Ctrl-Z/Shift-Z invoke Save/Save As and core
  Undo/Redo outside literal-next, including Insert mode. Never undo the input
  host's private text. Ctrl-0 through Ctrl-5 assign paragraph/headings; preserve
  Ctrl-6/Ctrl-^ for Vim. Other vi control keys must reach core, including
  Ctrl-B/F/I/U/N/O/A/W. Preserve function-key modifiers and AltGr text entry.
- Use Windows system font defaults; SF Pro is available only if installed.
  Preserve unavailable authored families/faces and normal fallback. Startup and
  missing-font lookup must not enumerate all font faces or load picker lists
  before first draw. Font changes still preserve ordered fallback families.
- Restore normal window geometry before activation, fit it onto an available
  monitor in desktop coordinates, and preserve normal bounds across
  minimization/maximization/fullscreen. Persist placement without invalidating
  document layout.
- Style/color editing follows the shared inspector transaction and stale-target
  rules. A popup gesture is one settings undo; closing/retargeting flushes to its
  original target, while external edits dismiss it without replaying queued
  values. Do not rebuild or resize active pickers on every color update.
- Save core bytes through atomic replacement; review external-content conflicts.
  Recovery slots belong to their document/process and use the shared envelope;
  never remove foreign slots. Same-user launch forwarding must be bounded and
  retain the caller's working directory for relative paths.

Current Windows gaps remain explicit: no full UI Automation TextPattern/Narrator
text editing; no printing adapter; no macOS-specific Services, Dictionary,
spelling/grammar panels, app dictation or Versions. Inspector OpenType discovery,
full paragraph preview and click-through activation of inherited controls are
missing. Per-span explicit bidi overrides are preserved but not rendered;
Unicode bidi and paragraph direction are supported. IME protocol tests do not
establish compatibility with every installed IME/speech service. Update these
gaps and native tests when their mechanisms ship.

## Validation and open work

Use deterministic tests at the document, command, layout and coordinator
boundaries. Cover source-byte round trips and minimal patch locality;
incremental-versus-fresh projection/layout/syntax; checked identities and range
algebra; encoding and mixed endings; sparse style inheritance; exact undo/redo
and branch retention; all command entry paths; and cancellation/stale publication.
Keep architecture dependency checks. Test failure rollback and reentrant
providers, not just successful edits. Native rendering/cache changes need real
provider geometry, input-routing and cached-versus-fresh pixel checks.

For syntax changes, cover cold display with providers suspended, bounded local
edits as unrelated suffixes grow, distant scrolling, EOF-reaching dependencies,
huge tokens/lines, text-sensitive queries/injections, cancellation, fallback and
limits. Compare completed incremental output with fresh analysis (and pinned Vim
for its supported profile). Test asynchronous metric/paint changes at an
off-center caret and mapped color continuity while providers are suspended.
Target roughly 2–4 ms cooperative slices and ready visible publication within a
frame, but document nonpreemptible native callbacks; timings do not replace hard
work/memory gates. See the provider profiles and
[measurement guide](docs/performance.md).

Keep these follow-ups visible; linked guides describe current next steps and
reproducible checks. Keep generated measurements outside committed documentation:

- **macOS background pre-layout is not connected.** Reuse portable policy and
  prove native cancellation, render-resource ownership, memory and paging before
  closing the [scheduler checklist](docs/windows-background-layout.md#todomacos-connect-the-native-scheduler).
- **Large-file memory/first display remains incomplete.** Compact literal
  storage does not satisfy progressive cold display. Remaining work includes
  dense conversion exceptions, rich projections, remaining flat-text consumers,
  per-component attribution and complete native memory/latency validation.
  Measure opening, local and disjoint edits, history branches, multiple views,
  jobs and release in fresh processes. Ordinary text must not recreate per-scalar
  heap mappings, even transiently; retain useful undo under shipped policy.
  Budget increases or accounting exclusions cannot substitute for compaction.
  See the [measurement guide and remaining work](docs/performance.md).
- **Markdown compatibility** is tracked in [MARKDOWN_GAPS.md](MARKDOWN_GAPS.md);
  keep the [feature demo](docs/markdown_demo.md) current. The broader command/port
  [audit](gaps.md) is dated evidence, not an automatically current inventory.

Do not silently settle these unrelated open product decisions: additional format
adapters and their canonical syntax; general policy for unrepresentable Unicode
edits; Unicode word/sentence tailoring; first-release rich clipboard obligations
beyond the explicit requirements above;
unspecified style/design defaults; hyphenation/justification; and hardware-specific
latency/cache defaults beyond established regression bounds. Record decisions
here or in an architecture decision record.

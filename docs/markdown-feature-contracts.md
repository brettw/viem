# Markdown feature contracts

A new syntax owner changes how editing interprets source, even when the visible
request is small. Before implementing it, identify the affected rows below and
choose a small representative for each. Write down any deliberate unsupported
operation. Finish this integration review before calling the candidate frozen.

| Contract | Representative | Verification |
| --- | --- | --- |
| Recognition | Leading, nonleading, incomplete, quoted and fenced spellings | Intended owner kind; unchanged bytes on open/save |
| Literal versus formatted body | Punctuation, comments, nested Markdown lookalikes | Text and semantic styles, not text alone |
| Empty and complete owners | Insert into empty body; delete the complete owner | Usable caret, retained surrounding source, exact undo |
| Syntax activated by another edit | Introduce or expose a delimiter elsewhere | Reopened appearance; retained rules/containers |
| Source transfer | Move/copy an owner to the document start | Source identity and semantic structure |
| Clipboard | Whole, partial and cross-owner selections | Destination representation; donor unchanged |
| Encoding | One BOM case first, then UTF-8/UTF-16/Latin-1 and endings as applicable | Exact bytes, typed U+FEFF distinct from BOM |
| Navigation and view switching | Heading-like text in literal owners; switch both ways | Same targets, source and anchors |
| Authoring capabilities | Language, style removal, default inheritance and creation | Allowed actions succeed; disabled actions are atomic |
| Incremental work | Small edit beside a large unrelated suffix | Work counters and fresh/incremental equivalence |
| Native interaction | Actual hover/click and popup lifecycle | Observable readiness, completion, preserved caret |

`python3 scripts/validate.py contract --contract front-matter` runs the existing
representative tests selected by `scripts/fixtures/markdown-contracts.json`.
Add a feature matrix there when useful, selecting existing tests where possible.
The contract stage complements the focused suites; it does not replace required
consistency, full-suite, or native checks.

## Comment authoring

Style → Character → Comment wraps a selection in a visible `<!-- … -->`
span. At a caret it immediately inserts `<!--  -->` and enters Insert mode
between the two spaces. This insertion is undoable before any body text is typed.
The body is literal, and continued Insert or Replace input stays
before the closing delimiter. Creating a comment preserves surrounding paragraph,
list, quote and table context; supporting passive inline markup may be needed to
avoid turning a line's unselected suffix into an HTML block. Code blocks, atomic
objects and selections containing comment delimiters cannot be wrapped.

`tests/all/comment_style.rs` covers recognition, literal bodies, immediate caret
creation,
selection and table actions, Insert-mode entry, exact history, encoding and
ending preservation, and bounded work beside an unrelated suffix. Existing HTML
editing regressions cover editing already-authored comments. Native character
menu tests exercise capability, dispatch, immediate delimiters, caret placement
and reopened styles.

## Shared fixtures and diagnostics

`tests/all/markdown_test_support.rs` supplies encoded fixtures, reopen/history
assertions and explicit Insert-boundary placement. A Normal-mode character
position can differ from an Insert boundary at the end of a paragraph. Assert
the actual mode and cursor before applying an operation; do not assume a
successful cursor-setting call preserved the requested offset. Failures should
include the encoding, ending, BOM, source/selection and requested operation.
Check owner kinds or container identities when two structures have the same
visible text, such as a rule and an empty paragraph.

Code-row deletion must keep following paragraphs outside retained code, including
when the final body row is empty. Test fenced, indented, nested and passive HTML
code through Vim and native line selections; keep explicit `J` joining distinct.

Keep shared demo fixtures structurally stable when tests consume them. New
syntax that must appear at document start belongs in a linked standalone demo.

## Line deletion caret policy

Preview linewise deletion follows the next surviving character, even if paragraph
reflow puts it on an earlier displayed row. At a shared wrap boundary, choose the
start of the following row (downstream affinity). Never skip surviving text just
to preserve the old screen row. If no following character remains, use normal
backward recovery.

Deleting the final wrapped row of ordinary prose can leave its paragraph
separator intact. Step across that separator to the next paragraph instead of
normalizing backward to the preceding character. A following empty paragraph is
an editable destination and is not skipped. Apply code/quote containment after
choosing this forward destination; end-of-document still falls back backward.

There is one containment exception: when all selected rows are inside a code
block or quote and that owner survives, do not implicitly leave it. If ordinary
recovery would leave, use the last surviving editable position before the
deletion, preferring the innermost surviving protected owner. A whole inner code
block can disappear while its enclosing quote survives. Compare owner identities,
not style names, and use selected-row ownership rather than the source patch hull
or the exclusive range end. The separator after a selected row does not itself
select the following paragraph. Lists are not protected owners.

Apply this to `dd`, linewise operator motions, counts, Visual Line deletion and
equivalent native line selection deletion/Cut, including replay. Explicitly
selecting outside content (`dj`, `d` + Down, counts or selections crossing the
boundary) permits leaving that owner. Deleting an entire owner permits normal
recovery; never recreate an empty owner to retain a caret. Thus repeated `dd` or
`.` at the bottom works backward through the owner, while one counted deletion
can select outside rows. Undo/redo restores the recorded caret without rerunning
the policy. Other views and general anchor deletion recovery are unchanged.

Change commands, characterwise deletion, `J`/`gJ`, Insert boundary joins, explicit
container exits, ordinary navigation and Ex address navigation retain their own
contracts. Physical source selections that include hidden syntax retain their
explicit source-edit semantics.

The comments and tests in `tests/all/markdown_deletion_caret.rs` protect these
deliberate distinctions. Before changing this behavior, warn the user about any
conflict and its consequences as required by AGENTS.md; update the requirement,
this contract, code comments and tests together when the policy changes.

## Preview source reflow

`gq` and `gw` in Preview tidy the underlying Markdown source while preserving
exact visible text, styles, hard breaks, paragraph/container ownership, links
and objects. Source line wrapping is the intended change; it must never change
what the reader sees. Compare semantic structure as well as text, including
empty paragraphs and different owner kinds with identical text.

Resolve the motion in Preview before mapping its extent to source. Implicit
line motions and doubled/count forms use displayed rows even if the view's
other commands use Physical Source line meaning: `gqq` selects one row and
`gqj` selects two. Text and Code retain hard-line counts even when soft-wrapped.
Explicit physical addresses retain their source-address meaning; existing
Visual selections retain their selected domain. Visual Block reflow covers
complete touched displayed rows in Preview, and complete hard lines in literal
formats. Replay resolves the command again against current content and layout.

The selected visible fragment is authoritative. Two Preview rows may occupy
part of one long source line; never round that selection outward to complete
source lines or paragraphs. Account for the fragment's original source column
and preserve bytes outside the selected fragment, apart from minimal required
continuation prefixes. `gqip` is the whole-paragraph cleanup operation. Output
width uses `textwidth` in Unicode source columns, including syntax and prefixes;
changing the window width changes the rows selected, not that output width.

Reflow eligible prose independently within paragraphs, quotes and list items.
Preserve paragraph boundaries, editable empty paragraphs, explicit hard breaks,
list nesting and tight/loose structure. Leave code blocks, front matter,
HTML blocks (including preformatted HTML), headings, tables and opaque content
fixed. Mixed selections
format eligible prose and skip fixed structures; selections containing only
fixed content succeed without a source edit. Never split protected inline
syntax or activate new Markdown at a line start merely to meet `textwidth`;
indivisible constructs may exceed it. Do not normalize unrelated delimiter,
link, bullet or heading spellings. Preserve encoding, untouched line endings,
and final-terminator state.

`gq` finishes at the first nonblank of the original last selected displayed row;
`gw` preserves its original visible caret and affinity. A selected empty final
row remains the destination even though it contributes no text character.
Invisible new source breaks never determine the Preview caret. Other views,
marks and viewport anchors retain their visible locations. The source edit is
one verified transaction and undo unit, leaves registers unchanged, and exact
undo/redo restores its source and view state. Already-formatted content creates
no history entry. Failed verification publishes no partial edits or view state.

Before changing the source-only guarantee or mode-specific selection/caret
rules, warn the user about the conflict and consequences as required by
AGENTS.md. Update that requirement, this contract, explanatory comments and
regression tests together when the user chooses a different policy.

The representative matrix is available through
`python3 scripts/validate.py contract --contract preview-reflow`.

## Architecture boundaries

`markdown_syntax::ParseContext` makes input origin explicit. `Document` admits
document-leading syntax; `source_region` chooses the appropriate context from
mapped source coordinates, including a BOM's content offset. `Interior` and
`ContainerFragment` cannot acquire document metadata. Container-stripped input
must not be passed as a new whole document. Inline body parsing uses the fragment
context; full-source grammar callers state their document context explicitly.

`CodeBlockKind` owns language editability, treatment removal (including empty
owners), the blank-line threshold for Enter to exit, default-language eligibility,
creation eligibility, fixed labels and clipboard representation.
Layout and authoring consume these queries. Low-level delimiter parsers still
inspect the concrete owner kind because source spelling is their responsibility.
Do not infer authoring capabilities from a presentation label or style name.

Whole encoded clipboard artifacts enter `prepare_artifact_replacement`, which
fully decodes BOM metadata and verifies the projected text and break structure
before preparing publication. Ordinary text enters the text-edit path, where a
leading U+FEFF remains content. Clipboard-specific style verification remains at
the clipboard boundary. Neither path bypasses atomic publication or exact undo.

## Reference labels

Resolved reference uses hide their brackets and definition identifier, render
inline label formatting, and retain explicit source and label ownership.
Definitions remain visible. Editing shortcut or collapsed labels promotes the
use to a full reference to retain its destination. Removing a definition through
ordinary editing preserves surviving visible uses as inline links. Toolbar
insertion preserves selected formatting and adds any new shared definition in
one transaction. See [reference controls](markdown-references.md).

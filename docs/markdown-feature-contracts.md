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

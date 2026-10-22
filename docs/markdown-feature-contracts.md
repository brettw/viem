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

Keep shared demo fixtures structurally stable when tests consume them. New
syntax that must appear at document start belongs in a linked standalone demo.

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

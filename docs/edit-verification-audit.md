# Formatted-edit verification audit

Historical validation: standalone HTML modes and their dedicated editing tests
were subsequently removed. Markdown and RTF retain the shared verification rules;
HTML-mode results below describe the earlier implementation.

Markdown WYSIWYG deletion now repairs source syntax when removing visible text changes how the remaining source parses. The shared translation path serves Visual `x`/`d`, ordinary delete and Backspace, replacements, payloads, and batched edits. It still verifies the candidate projection before publishing one undoable transaction.

## Reproduced failures and fixes

| Trigger | Previous failure | Repair |
| --- | --- | --- |
| Delete the last grapheme in inline code, or delete a larger selection containing the complete span | Empty backtick delimiters became visible literal characters | Delete only the consumed span's delimiters and optional padding as supporting patches. |
| Delete between two surviving code spans, including across physical lines or paragraphs | Adjacent closing/opening backticks formed a longer delimiter run | Join the surviving code bodies by removing the interior delimiters; retain or grow the outer markers as required. |
| Delete at the edges of a padded code body | Padding could become visible content, or remaining authored spaces could be trimmed | Preserve the original delimiter spelling whenever it still represents the exact body; adjust padding only when needed. |
| Replace a cross-style selection beginning inside code with a literal backtick | Prose escaping inserted an unwanted backslash inside code | Insert literal code text and grow its markers; prose segments continue to use prose escaping. |
| Delete a physical-line body while retaining an adjacent folded space | The source newline became a paragraph boundary or disappeared | Reparse the affected source band and represent only the lost retained space with `&#32;`. |
| Delete a physical break inside a quote or list continuation | The following hidden quote/list prefix became visible mid-line | Remove only the now-consumed container prefix. |
| Delete the entire remaining body of a list continuation | Its previously hidden indentation became visible on an empty row | Remove the unmapped indentation when its complete body is consumed. |
| Delete between a retained inline hard break and a following paragraph boundary | Source whitespace could combine the two retained boundaries | Give the affected inline break an explicit `<br>` spelling while retaining its physical newline. |

These are explicit source patches. Untouched bytes, source encoding, BOM, line-ending spelling, visible grapheme selection, registers, and undo/redo remain governed by their existing guarantees. The fixes do not serialize the whole document, expand a selection to a source hull, suppress verification, or turn a failed candidate into a source-only commit. Code-delimiter lookup now reads the adjacent encoded marker units rather than decoding the complete document.

## Other routes to the same message

The C bridge maps four families to `VerificationFailed`: `DocumentError::VerificationFailed`, `FormattedPayloadCannotReproject`, `HardLineTransferProjectionMismatch`, and formatted-storage `ResultTextMismatch`.

| Route inspected | What the check protects | Outcome |
| --- | --- | --- |
| Ordinary text, payload replacement, recovered HTML contributors, and structural edit batches | Exact requested text and source provenance after parsing; atomic composition of multiple edits | Fixed the Markdown translation defects above. HTML/RTF deletion matrices and the existing rich-edit/paragraph-merge suites cover cross-paragraph, inline-style, entity, and object deletion. |
| Incremental literal, Markdown, HTML, and source-view projection builders | Agreement between local projection, persistent text tree, source-line index, and untouched surrounding content | Retained all checks. A failure here indicates a translation/cache invariant defect, not permission to discard source or edit a stale snapshot. |
| Empty-document clearing, paragraph/list operations, quote insertion, indentation, and block-style changes | Retained text, paragraph identity, hard-break identity, and unaffected styles | Existing structural-edit suites remain required. The newly exposed prefix and boundary repairs run through the common text path. |
| Named/direct character and paragraph styles, HTML/RTF style definitions, and materialized contributors | Requested style assignment without unrelated text/style changes | Fixed two integration defects: HTML character verification now resolves automatic Link defaults instead of rejecting the interval; HTML direct-format clearing recognizes `<b>`/`<strong>` as relative bold, separately from numeric base weight. Candidate and unaffected-style checks remain. |
| Encoding/metadata updates | Exact text after re-encoding and unchanged projection for metadata-only changes | HTML insertion of literal U+000D was a known, reproducible representability limitation reported as a generic verification failure. It now returns the existing structured `UnrepresentableFormattedCharacter` policy result before mutation, as NUL already does. HTML preprocessing/whitespace rules cannot preserve exact U+000D even through a numeric reference. |
| Formatted payloads and hard-line transfers/reordering | Payload hard-break metadata and format/container structure can actually be represented by the destination source | These checks remain necessary. For example, a payload requesting literal LF content without a hard-line boundary cannot be serialized in Unix plain text; its existing atomic rejection test remains. This is distinct from deleting an ordinary valid visible selection. |
| Checked position/storage adapters | Impossible ranges, exhausted identities/revisions, and disagreement between a supplied target text and tree edits | Preserved invariant checks. Public stale-document/revision, invalid-grapheme, ambiguity, and unsupported-operation paths already use their structured statuses. |

Link styling can also be deleted without deleting the link itself. Its content-derived interval stays available for source-authoritative destination queries, while an explicitly deleted generated Link definition resolves to empty appearance declarations. Reprojection, text edits, undo, and redo preserve that choice; unknown references to other styles still fail validation. Link defaults participate in character-edit verification at the same priority as rendering, below authored character declarations.

The invariant error is intentionally still available for defects and unrepresentable structured payloads. It is not an accepted outcome for the supported Markdown deletions exercised here. A static audit and regression corpus cannot prove that every future parser construct is free of defects.

## Validation

`tests/all/markdown_deletion_audit.rs` exercises every legal deletion range across 30 Markdown fixtures and every range with ordinary text, space, and backtick replacements across seven fixtures. It also covers:

- Visual `v` selection followed by named-register `x` across wrapped physical lines, exact undo/redo, and reopened source;
- final inline-code grapheme deletion through Normal `x` and Insert Backspace, including emoji and combining-safe text positions;
- disjoint edits that jointly empty one inline code span in one history transaction;
- exact patch locality and reopened projection across UTF-8, Latin-1, UTF-16LE, UTF-16BE, LF, CRLF, and CR files;
- every legal deletion range across HTML/RTF inline scopes, paragraphs, entities, and atomic objects.

`tests/all/link_edit_verification.rs` additionally covers HTML set/clear formatting with Link defaults present or deleted, destination retention, undo, and source reprojection in all four link formats. It pins the independence of numeric base weight and relative bold when clearing HTML declarations.

Validation passed across the 19 focused editing targets (123 tests at the initial run), followed by the final affected-target rerun including all seven new deletion tests. The Link integration changes also run with the existing `rich_direct_properties` and `typography` targets. Reproduce the new coverage with:

```sh
cargo test --offline --test all -- markdown_deletion_audit:: link_edit_verification::
cargo test --offline --test all -- rich_direct_properties:: typography:: paragraph_merge_adapters:: paragraph_editing_keys:: markdown_inline_breaks:: markdown_block_quotes:: fuzz_markdown_edits::
```

## Integrated validation

The complete Rust run passed 2,331 tests across 157 targets (four existing ignored tests). The final six affected targets passed 67 tests after integration, including the 20 link tests and three link-formatting tests. The native suite passed 573 tests; the final save/fingerprint subset passed 36 tests, including three new streaming-fingerprint regressions. The native app was rebuilt after those checks.

All 18 existing large-file memory fixtures passed their checked ceilings. Live and peak requested heap values were byte-for-byte identical to the measurements after the style-branch merge: the 100 MiB plain-text open retained 317.09 MiB with a 656.94 MiB peak, and the 4 MiB text/code two-view cases retained 29.79 MiB with a 35.62 MiB peak after scrolling and wrap changes. Final raw results are `/tmp/viem-editing-memory-final.json` and `/tmp/viem-editing-memory-supplemental-final.json`. Native external-change checks additionally use streaming SHA-256 with one reusable 1 MiB buffer, keeping the save check's input storage bounded.

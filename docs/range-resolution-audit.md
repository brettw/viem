# Editable source range audit

> Historical audit: references to HTML/RTF editing and general format conversion
> describe retired implementations. Current editable formats are Text, Code,
> and Markdown; only Markdown Source/WYSIWYG switching remains.


Historical audit: standalone HTML editing modes and their named helpers were
removed in September 2026. HTML-specific examples below describe the former
implementation; the shared boundary rules continue to apply to Markdown.

This audit covers the `DocumentError::AmbiguousProjection` producers in the
portable core and their macOS presentation. Function names and line numbers in
review notes refer to the code at the start of the
September 2026 list/boundary fix. The later rich-editing pass applies the explicit
paragraph-ownership rules below; formatting and explicit move limitations remain
separately identified.

## Shared rule

A visible edit should resolve the smallest **complete** source patch set that
implements its semantic intention. “Smallest” does not mean the shortest single
byte interval, collapsing a nonempty selection, or silently dropping contributors.

1. Resolve a caret against its logical paragraph/item first. The first and last
   visible caret locations belong to that paragraph, including its empty body.
   Hidden opening/closing syntax must not turn an insertion into a reversed
   range. Use boundary affinity only for legitimate adjacent inline context.
2. For selected text, collect all contributing source runs in logical order,
   preserving unselected syntax between them. Empty anchor records are boundary
   metadata, not selected characters and not holes in coverage.
3. Pass selected hard breaks, paragraph/list boundaries, and atomic objects to
   their format adapter with their identity intact. They need semantic operations,
   not a zero-byte text deletion.
4. Verify that the complete candidate source reproduces the requested text and
   structure before committing. Preserve invalid-boundary, stale-snapshot,
   malformed-byte, and patch-conflict checks; none is a range tie to guess away.

The implementation now puts ordinary rich-text translation in
`source_edit::rich_text_patches`. Its `insertion_point` chooses visible boundary
ownership, `visible_runs` enumerates complete discontiguous text contributors,
and `complete_contributors` rewrites an indivisible entity while carrying through
unselected prefix/suffix text. The original logical edit and its position maps
remain unchanged by that source-only expansion. The contiguous-hull helper is
appropriate for bounded source inspection, not a general editing contract.
When several logical edits share one entity, `rich_text_batch_patches` partitions
and combines their source translations once while retaining the original logical
edits/maps. Structural adapters still translate paragraph/list/object intentions
and emit supporting patches in the same transaction.

Character styling uses `style_contributors::prepare_with_materialized_style_boundaries`
for semantic styling, direct property set/clear, and named character assignment.
It materializes only indivisible contributors crossed by the selection endpoints,
verifies that this leaves all existing text and styling unchanged, and prepares
the original logical style intention on that speculative snapshot. Its composed
publication retains an identity text map and one atomic source patch set. An
already-satisfied semantic style is a byte-exact no-op.

Paragraph ownership is centralized in `edit_boundary::{paragraph_at, merged_paragraphs,
is_code_paragraph}`. `edit_translation::structural_text_patches` is the shared
adapter dispatch for flat edits and formatted payloads. Removing a boundary
keeps the preceding paragraph's assignment and paragraph declarations, including
across different containers; retained explicit character declarations remain
attached to their text. The same rule handles Backspace, Forward Delete,
selection deletion/replacement, and joins. Backspace at a list/code start is the
one structural reset: it assigns the normal paragraph style without removing
text. Other paragraph starts join into their predecessor.

`html_merge` replaces the former two-sibling restriction with a structural-context
splice: retain the left owner, keep the right text's inline and whitespace
context, and restore the untouched following paragraph's original containers.
Empty merged paragraphs retain an explicit editable owner. Markdown owns the
crossed prefixes/fences and retains literal code text; RTF scopes the preceding
paragraph context over the retained tail while preserving subsequent controls
and shared numbering resources. `prepare_structural_text_batch` composes shared
source delimiters before one publication, rebasing every intermediate range
through named position maps. Supporting whitespace is prepared before joins.
Rich clipboard replacement uses the same deletion plan before inserting its
balanced source fragment. If the destination paragraph changes the copied
character style through inheritance, the existing character-formatting adapter
isolates the copied formatting locally, and the complete paste is verified again.

Paragraph opening carries both the origin and the requested side to the shared
following-style computation. Empty paragraphs are verified before any text is
typed. In RTF, assigning the style of an empty paragraph includes its terminating
`\par` in the scope, so the parser does not restore the preceding style before
emitting that paragraph. Paragraph-style assignment checks the result as well as
text preservation. Character-style verification partitions at hard boundaries,
so a paragraph separator cannot masquerade as a character-style interval.

Opaque objects now retain their complete original source extent as one atomic
contributor, with usable before/after insertion positions. Their interiors remain
uneditable. `source_edit::overlapping_text_plan` subtracts unselected visible
contributors and their required inline syntax when HTML recovery moves text out
of an atomic source owner (for example, fostered table text). Text and rich
clipboard edits share the logical replacement-placement rule. Before structural edits through recovered HTML, `prepare_with_recovered_source`
materializes only the affected relation in a verified scratch snapshot. It moves
recovered text and its inline scopes into visible order, supplies an explicit
paragraph owner where needed, and then invokes the original text/payload/clipboard
operation. Paragraph assignments, character styles, and hard lines must remain
identical during materialization. The final transaction retains the original
logical edits and position maps. A styled `div` supplies a flow-capable owner when
HTML's content model would otherwise close a `p` around a retained table.

An unclosed object
receives only the missing closing syntax needed to insert after it. Complete
malformed decoding units likewise retain their source extent; partial-unit and
stale-position protections remain.

HTML hard-line deletion now distinguishes a selected paragraph from selected
lines inside it. It removes native intra-paragraph breaks while retaining the
surviving paragraph boundary. A partially selected list owner is retained for its
continuation paragraphs and nested items; selecting parent text never implicitly
deletes children. Source-backed structural separators, such as a nested list's
opening tag, are retained unless their owner is removed.

## Emitter inventory and centralization points

The original producers and their replacements fall into these families. Files are under
`src/core/document` unless otherwise stated.

| Family | Producers | Assessment |
| --- | --- | --- |
| Text endpoint/run resolution | `source_edit::{rich_text_patches, rich_text_batch_patches, insertion_point, visible_runs, complete_contributors, contiguous_range}` and `rich_text::{text_source_range, editable_source_range, text_source_runs, block_source_point, list_item_source_point}`; callers in `transaction::{prepare_text_edits_with_patches, prepare_formatted_payload_edits, prepare_rich_list_enter, prepare_html_paragraph_enter, prepare_rtf_next_paragraph_enter}` | Caret ownership and complete contributor enumeration now use the shared source-edit policy. The earlier strict contiguous mapping rejected editable text split by harmless inline syntax; remaining contiguous helpers are limited to callers that actually require one source interval. |
| Source-visible semantic translation | `html_source::map_range`, `list_indent::prepare_list_indent`, `transaction::{prepare_html_source_patches, prepare_physical_source}` | Use the same boundary rule after mapping through the explicit source/cooked domains. Do not swap inverted endpoints or collapse a nonempty selection merely because hidden syntax has no visible interior. |
| Structural HTML | `html::{list_enter_patch, list_patches}`, `html_paragraph::{enter_patches, deletion_patches}`, `html_merge::{patches, anonymous_break_patches}`, `html_quotes::{in_native_pre, remove_patches}`, `list_indent::html_patches` | Text boundary joins use the shared owner rule across containers. Styling and explicit tree moves remain distinct intentions; they cannot safely be fixed by a hull. |
| Structural Markdown | `markdown_list_edit`, `markdown_list_structure`, `markdown_quote_edit`, `markdown_quotes`, `markdown_code`, `markdown_split`, `markdown_block_styles`, `paragraph_keys`, `structural_style`, `list_indent::{markdown_patches, block_source_at}`, `transaction::{prepare_markdown_paragraph_style_raw, prepare_list_style_raw}` | Most emissions are failed source-boundary/line lookups and should disappear with consistent ownership. Missing certified list/quote/fence delimiters indicate unsupported syntax or a projection invariant, not competing editable ranges. Keep supporting delimiter edits and semantic verification. |
| Character and paragraph styling | `named_character`, `typing`, `html_typing`, `markdown_typing`, `html_direct::clear_character_patches`, `rtf_direct::clear_character_patches`, `rtf_styles::character_assignment_patches`, `transaction::{prepare_html_named_style_raw, prepare_rtf_named_style, prepare_rich_block_properties, rtf_paragraph_property_patches, prepare_rich_list_style, rich_character_source_patches, semantic_style_source_patches, markdown_style_removal_patches}` | Reuse contributor enumeration for character runs and paragraph ownership for block styles. Exact delimiter removal, misnested scopes, and partial indivisible contributors remain distinct checks. Styling can ignore semantic paragraph separators by explicit policy; ordinary text deletion cannot. |
| Whitespace and RTF encoding context | `html_whitespace::{collapsed_space_tail, exposed_whitespace, html_boundary_space_edits, normalize_typing_payload}`, `rich_text::{escape_html_source_edit_with_context, html_preserves_whitespace_at_source}`, `rtf::{escape_insertion, advance_past_fallback_scope}`, `transaction::escape_markdown_source_text` | Required contributors and encoding context must remain complete. Centralize basic boundary resolution while retaining format escaping and whitespace normalization. Some emissions are arithmetic/storage failures, not ambiguity. |
| Rich clipboard | `clipboard_fragment::{clipboard_fragment_with_source, prepare_clipboard_fragment, source_hull, html_separator_source_hull, selected_source}` | Separate export of owned syntax from editable text runs. A copied fragment can require balanced wrappers and resources beyond visible text; candidate verification must remain. Serialization failure should have a clipboard/format-specific result. |
| Reordering and RTF structural deletion | `reorder::{rich_patches, html_rows, map_owners, markdown_rows}`, `rtf_structure::deletion_patches`, `list_indent::rtf_patches` | Preserve content identity and parent/list ownership. Cross-parent moves, partial list owners, and shared numbering resources require structural policy, not endpoint selection. |
| Snapshot, storage, and composition invariants | `source_lines::{physical_line, physical_line_at_text}`; `replacement::{record_formatted, source_patches, formatted_edits, prepare_recorded_replacement_with_typing_style, map_after}`; `fragments::prepare_fragment_edits`; `structural_style::publish`; `paragraph_keys::verify_hard_break_paragraph`; `typing::prepare_insertion_with_typing_style`; `transaction::{line_local_projection_region, validate_source_patches, apply_source_patches}`; `src/core/command/mod.rs::{replace_characters, prepared_cursor, prepared_break_cursor}` | Failed checked arithmetic, missing storage slices, patch overlap, and unresolved history maps are not ambiguity. Preserve rejection, classify accurately, and fix the violated invariant. Never recover these by editing a nearby range. |
| Layout precondition | `src/core/command/line_mode.rs::visual_indent_edits` | Missing exact line layout is a layout-precondition failure, unrelated to source ranges. |
| User-facing presentation | `mod.rs::DocumentError::fmt`, `src/core/ffi.rs::document_status`, `src/mac/Editor/Sources/EVCoreDocument.swift::errorDescription` | The low-level “unambiguous source range” text should not reach the user. Safely resolvable ordinary edits should succeed; remaining failures should identify the unsupported operation or violated format constraint. |

## Cases needing explicit product decisions

These are the places where choosing a minimal raw range is not sufficient:

- **Explicit moves across different structural parents.**
  `reorder::map_owners` still requires nonoverlapping owners and common ancestry.
  The paragraph-boundary deletion rule now explicitly chooses the preceding
  owner for joins, but a move must separately specify where list, quote, or
  container ownership should travel.
- **Malformed/misnested HTML and uncertified Markdown delimiters.**
  `html_direct::clear_character_patches`, `reorder::html_rows`, and
  `markdown_code::patches` require native source ownership for structural changes.
  Ordinary text editing now materializes the required local delimiters and
  recovered source order while preserving literal text; arbitrary formatting or reordering still requires a
  separately verified rewrite.
- **Partial indivisible content and invalid coordinates.** Synthetic paragraph
  separators and complete atomic objects have explicit deletion rules. An edit
  must still identify a complete valid logical item; invalid grapheme boundaries,
  stale snapshots, or partial decoding units cannot be repaired by taking a
  minimum byte slice. Multi-character entities remain editable by materializing
  the complete contributor and preserving its unselected visible text.
- **Appending after a truncated encoding unit.** A UTF-16 source ending in one
  unmatched byte cannot accept bytes after that unit while retaining both the
  original byte and the same decoded text. `OpaqueDecodingConflict` remains for
  that operation; inserting before the diagnostic or explicitly replacing the
  complete malformed unit succeeds. Repairing the byte or changing the encoding
  would require an explicit policy, not a different source-range tie break.
- **Markup-only source selections for semantic formatting.**
  `html_source::map_range` can map both ends of tag-only syntax to different
  neighboring visible locations. Targeting the containing paragraph is sensible
  for paragraph formatting; character formatting requires an explicit containing
  or adjacent context policy instead of silently broadening the selection.

## Remaining adapter limitations

- Clearing inherited conventional HTML bold is an existing style-translation
  limitation independent of source range ambiguity. On HTML
  `<p><b>fj</b></p><!--keep-->`, `ModelRequest::EditDirectProperty` with formatted
  `range: 1..2`, `property: CharacterBold`, and `value: None` returns
  `Document(VerificationFailed)` without changing source. The same failure occurs
  after materializing `&fjlig;` to `fj`; choosing another source range would not
  fix it. The clear adapter and `character_clear_verified` need an agreed meaning
  for removing conventional inherited emphasis from only part of its scope.
  Setting semantic Strong to `false` for that selection succeeds and preserves
  the unselected character's bold style. Explicit direct properties such as
  font size can be cleared on a partially selected entity through the shared
  materialization path.

## Regression coverage to retain or add

- At each paragraph/list boundary, compare typing, Backspace, Enter, and `dd`
  after direct insertion, arrow navigation, pointer hit testing, and mode changes;
  include empty first, middle, and terminal paragraphs and both affinities.
- Exercise inline formatting boundaries, nested empty tags, adjacent empty
  blocks, HTML comments/scripts, Markdown delimiters, UTF-16/BOM/CRLF sources,
  combining graphemes, and synthetic paragraph separators. Assert byte locality,
  exact formatted intent, and undo/redo restoration.
- Existing tests cover opaque-byte rejection in `document/mod.rs`, HTML collapsed
  whitespace handling, rich clipboard separator round-trips in
  `clipboard_fragment.rs`, and relational source mapping in `projection.rs`.
  Keep these guarantees while relaxing harmless boundary ambiguity.
- Add policy-specific regressions for each structural decision above when
  implemented; they should never be hidden by a generic nearest-range fallback.
- Keep ordinary caret/run resolution local. Use indexed adjacent provenance and
  block lookup; do not scan all blocks, paragraphs, or anchors on each keypress.

## Earlier range-refactor validation

The counts in this section and the following audit section are historical.
The Markdown Source fence-policy failure has since been resolved by
`markdown_source_fence_edit_reparses_immediately_through_either_path`, which
verifies immediate source-view reprojection through both edit APIs. Its full
12-test regression binary passes. The subsequent upstream Markdown list-caret
failure is also fixed: ordinary payload insertion now uses the shared
visible-line resolver, with unchanged original regression coverage and added
exact-patch/undo/redo checks.

- The complete Rust suite (`cargo test --all-targets --no-fail-fast`) has 1,892
  passing tests and one preexisting failure:
  `projection_fuzz_regressions::markdown_source_fence_edit_requires_explicit_source_intent_when_breaks_reinterpret`.
  The same test fails on clean HEAD
  `87f4508728300c4b56adb0ff8ddaf5a11d99ac99`; its rejection expectation predates
  the intentional Markdown Source reprojection fallback. It is unchanged here.
- The complete native suite passes all 365 XCTest tests and all 26 Swift Testing
  tests. It was run with an isolated temporary `VIEM_CONFIG_DIR` and access to
  macOS services such as the pasteboard. The restricted sandbox cannot support
  several existing configuration, pasteboard, and appearance tests.
- Focused coverage includes partial and shared HTML entities, rich paste and
  character styling, list keys after navigation, terminal empty-paragraph caret
  geometry, structural hard-line deletion, and flowed HTML source blocks.
  Layout tests cover composition, cache invalidation, stable identities, and
  bounded work when editing a large document with many same-line source blocks.

## Rich-editing audit validation

- Final `cargo test --all-targets --no-fail-fast`: 1,945 passing tests and the
  same preexisting Markdown Source fence expectation listed above. No new Rust
  failures remain. Full output: `/tmp/viem-rich-verified-rust.log`.
- Final `scripts/test-mac.sh`: all 366 XCTest tests and all 26 Swift Testing
  tests pass with isolated temporary settings. The script rebuilt and signed
  `.build/Viem.app`. Full output: `/tmp/viem-rich-verified-native.log`.
- Regressions cover immediate empty-paragraph following styles for `o`/`O`,
  count/repeat/history, nested structural resets after arrow navigation,
  inline-grapheme deletion, every legal selection through mixed blocks and
  recovered objects, payload snapshot rebinding and no-ops, rich paste, exact
  reopen/undo behavior, and source-flow ownership of empty styled containers.
- The 10,000-paragraph checks retain unrelated block identities and cached
  layout, limit ordinary styled-container typing to regional projection with
  less than 256 decoded source bytes and at most one shaping request, and bound
  character-verification queries while detecting a changed distant style.
- Both working-tree and staged `git diff --check` pass.

# Editable source range audit

This audit covers the `DocumentError::AmbiguousProjection` producers in the
portable core and their macOS presentation. Function names below are stable
references; line numbers in review notes refer to the code at the start of the
September 2026 list/boundary fix. This is an inventory and policy assessment,
not a claim that every listed adapter limitation has been implemented.

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
| Structural HTML | `html::{list_enter_patch, list_patches}`, `html_paragraph::{enter_patches, join_patches, deletion_patches}`, `html_quotes::{in_native_pre, remove_patches}`, `list_indent::html_patches` | Source ownership should use the shared resolver. Unsupported tree restructurings and malformed owner relationships need specific structural failures; they cannot safely be fixed by a hull. |
| Structural Markdown | `markdown_list_edit`, `markdown_list_structure`, `markdown_quote_edit`, `markdown_quotes`, `markdown_code`, `markdown_split`, `markdown_block_styles`, `paragraph_keys`, `structural_style`, `list_indent::{markdown_patches, block_source_at}`, `transaction::{markdown_empty_code_patches, prepare_markdown_paragraph_style_raw, prepare_list_style_raw}` | Most emissions are failed source-boundary/line lookups and should disappear with consistent ownership. Missing certified list/quote/fence delimiters indicate unsupported syntax or a projection invariant, not competing editable ranges. Keep supporting delimiter edits and semantic verification. |
| Character and paragraph styling | `named_character`, `typing`, `html_typing`, `markdown_typing`, `html_direct::clear_character_patches`, `rtf_direct::clear_character_patches`, `rtf_styles::character_assignment_patches`, `transaction::{prepare_html_named_style_raw, prepare_rtf_named_style, prepare_rich_block_properties, rtf_paragraph_property_patches, prepare_rich_list_style, rich_character_source_patches, semantic_style_source_patches, markdown_style_removal_patches}` | Reuse contributor enumeration for character runs and paragraph ownership for block styles. Exact delimiter removal, misnested scopes, and partial indivisible contributors remain distinct checks. Styling can ignore semantic paragraph separators by explicit policy; ordinary text deletion cannot. |
| Whitespace and RTF encoding context | `html_whitespace::{collapsed_space_tail, exposed_whitespace, html_boundary_space_edits, normalize_typing_payload}`, `rich_text::{escape_html_source_edit_with_context, html_preserves_whitespace_at_source}`, `rtf::{escape_insertion, advance_past_fallback_scope}`, `transaction::escape_markdown_source_text` | Required contributors and encoding context must remain complete. Centralize basic boundary resolution while retaining format escaping and whitespace normalization. Some emissions are arithmetic/storage failures, not ambiguity. |
| Rich clipboard | `clipboard_fragment::{clipboard_fragment_with_source, prepare_clipboard_fragment, source_hull, html_separator_source_hull, selected_source}` | Separate export of owned syntax from editable text runs. A copied fragment can require balanced wrappers and resources beyond visible text; candidate verification must remain. Serialization failure should have a clipboard/format-specific result. |
| Reordering and RTF structural deletion | `reorder::{rich_patches, html_rows, map_owners, markdown_rows}`, `rtf_structure::deletion_patches`, `list_indent::rtf_patches` | Preserve content identity and parent/list ownership. Cross-parent moves, partial list owners, and shared numbering resources require structural policy, not endpoint selection. |
| Snapshot, storage, and composition invariants | `source_lines::{physical_line, physical_line_at_text}`; `replacement::{record_formatted, source_patches, formatted_edits, prepare_recorded_replacement_with_typing_style, map_after}`; `fragments::prepare_fragment_edits`; `structural_style::publish`; `paragraph_keys::verify_hard_break_paragraph`; `typing::prepare_insertion_with_typing_style`; `transaction::{line_local_projection_region, validate_source_patches, apply_source_patches}`; `src/core/command/mod.rs::{replace_characters, prepared_cursor, prepared_break_cursor}` | Failed checked arithmetic, missing storage slices, patch overlap, and unresolved history maps are not ambiguity. Preserve rejection, classify accurately, and fix the violated invariant. Never recover these by editing a nearby range. |
| Layout precondition | `src/core/command/line_mode.rs::visual_indent_edits` | Missing exact line layout is a layout-precondition failure, unrelated to source ranges. |
| User-facing presentation | `mod.rs::DocumentError::fmt`, `src/core/ffi.rs::document_status`, `src/mac/Editor/Sources/EVCoreDocument.swift::errorDescription` | The low-level “unambiguous source range” text should not reach the user. Safely resolvable ordinary edits should succeed; remaining failures should identify the unsupported operation or violated format constraint. |

## Cases needing explicit product decisions

These are the places where choosing a minimal raw range is not sufficient:

- **Partially selected RTF list owners.** `rtf_structure::deletion_patches`
  still rejects partial multiparagraph items. The recommended policy matches the
  implemented HTML behavior: retain the owner of surviving content and remove
  only the selected paragraph. Removing or promoting descendants implicitly would
  exceed the selected hard-line extent.
- **Joining or moving across different structural parents.**
  `html_paragraph::join_patches` checks matching parent ancestry and intervening
  structure; `reorder::map_owners` checks nonoverlapping owners and common ancestry.
  Minimal text patches alone do not specify which list, quote, table cell, or
  block container should own the result.
- **Shared RTF numbering definitions.** `rtf_structure::deletion_patches` refuses
  removal of a numbering destination still referenced by surviving items.
  Preserve the resource and add a local override when the adapter can verify it;
  do not erase shared bytes because they are the nearest source representation.
- **Malformed/misnested HTML and uncertified Markdown delimiters.**
  `html_direct::clear_character_patches`, `reorder::html_rows`, and
  `markdown_code::patches` cannot always establish a native syntax owner.
  Preserving imported syntax should remain the default until an explicit
  structural rewrite policy exists.
- **Synthetic or indivisible content.** Synthetic breaks need paragraph/list
  operations. Atomic objects need object deletion/replacement rules. Partial
  malformed decoding units and overlapping/reordered provenance need an exact
  adapter relation. A minimum byte slice must not split or discard them. Ordinary
  multi-character entities are handled by `complete_contributors`: preserve the
  unselected visible portion while rewriting the complete source entity.
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

## Validation of this change

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

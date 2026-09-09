# Code quality review — September 2026

Reviewed against `7537b4f7e85f0d7705356e65886ff27dc3058125`. This pass implements
behavior-preserving reductions. All proposals below that alter product behavior,
source preservation, or compatibility remain unimplemented.

## Scope and size

The review covered the document and format adapters, command interpretation,
coordinator, layout and measurement, C ABI, native editor and style UI, AppKit
shell, build scripts, and test/fuzz infrastructure. Deeper review and independent
review concentrated on repeated implementations and the changes selected here;
this is not a claim of exhaustive correctness verification of every line.

Baseline physical line counts include comments, blank lines, and declarations:

| Area | Lines before this pass |
| --- | ---: |
| Rust document, including inline tests | 67,008 |
| Rust command, including inline tests | 40,664 |
| Rust coordinator, including inline tests | 12,198 |
| Rust layout, including inline tests | 14,862 |
| C ABI implementation and header, including inline tests | 16,659 |
| Native implementation | 19,159 |
| Separate test files | 59,907 |
| Build scripts and fuzz harness | 3,350 |

These counts should not be read as production implementation sizes. In addition
to the separate tests, the command interpreter alone contained about 7,500 lines
of inline tests. The ABI includes explicit records and safety contracts, and the
HTML adapter contained a large duplicate generated table. Nevertheless, the
review found substantial avoidable implementation duplication.

## Implemented reductions

The source diff removes **4,185 net lines across 23 files**, including the two
new entity regressions. This report is excluded from those totals. The duplicate
entity table accounts for 2,235 lines; the remaining reduction is **1,950 lines**.

| Owner | Net reduction | Change and preserved contracts |
| --- | ---: | --- |
| Command | 866 | Shared Insert/Replace recording, deletion ranges, replacement targets and journals, and edit-plan assembly. Removed duplicated fallback Normal/Visual/prompt dispatch already handled by the shared controller dispatch. Counts, pending state, registers, repeat, and transaction timing remain intact. |
| Document, excluding entity data | 337 | One sparse-property registry supplies set/clear/compare/merge operations. Text and payload edits share adapter dispatch, mapping, escaping, and encoding. Shared scratch-document construction and block snapshots; removed repeated RTF assignments. Payload break identities, affinity, adapter order, and verification remain distinct where necessary. |
| Duplicate HTML entity data | 2,235 | Reused the table already provided by html5ever. All 2,231 mappings were compared before replacement. A frozen digest regression checks the complete mapping, with separate prefix and attribute-decoding regressions. No dependency was added. |
| Coordinator | 59 | Shared preparation of mapped command images and postcommit viewport/composition refresh across three native transaction paths. Format-specific anchor capture, edit-group boundaries, and failure/publication order remain unchanged. |
| C ABI | 149 | Shared pairwise region validation and copying of nonempty output slices. Capacity checks still precede every atomic batch copy; empty/null buffers, overlap rejection, error order, and public ABI stay unchanged. |
| Native editor | 539 | Shared ordinary-operation and host-effect lifecycles, plus typed snapshot-slice validation. Removed the unreachable legacy style-property editor and its factories/state; the active compact controls remain. Effect ownership, request lifetimes, and successful-state publication remain unchanged. |

Key implementation locations:

- `src/core/command/mod.rs`: `InsertSession`, controller dispatch, `edited_plan`,
  `normal_delete_range`, and `replacement_payload_targets`.
- `src/core/document/style.rs`: `sparse_property_operations` and property overlays.
- `src/core/document/edit_translation.rs`: `translate_source_edits` and
  `preserve_markdown_edit_boundaries`.
- `src/core/coordinator.rs`: `prepare_mapped_commands` and
  `refresh_views_after_native_change`.
- `src/core/ffi.rs`: `validate_disjoint_regions` and `copy_output`.
- `src/mac/Editor/Sources/EVCoreDocument.swift`,
  `EVCorePresentationExports.swift`, and `EVStyleEditor.swift`.

## Architectural assessment

The largest avoidable source of complexity was historical overlap: two command
dispatch paths, an unused style-editor implementation, independent sparse-style
field lists, and parallel text/payload translation loops. This pass removes or
consolidates those proven-equivalent paths within their existing owners.

The remaining size is partly a consequence of explicit requirements: lossless
format editing, malformed-source recovery, multiple source and presentation
domains, modal repeat/register/history behavior, and bounded incremental layout.
These requirements need distinct policies even when some surrounding mechanics
can be shared. A generic rich-format serializer would reduce code by changing
source-preservation behavior; it is not an equivalent refactor.

The layout review found repeated regional/long-line assembly, but the two paths
have different capture, checkpoint, cancellation, and coverage contracts. They
were not merged speculatively. Likewise, the mock provider's visual-order
calculation remains independent of the production validator; sharing it would
make some tests less effective. Existing cache-invalidation and large-document
tests were retained and exercised by the full Rust run.

The AppKit shell and Core Text bridge retain explicit ownership and lifecycle
boundaries. Similar-looking snapshot and buffer APIs have different identities,
release rules, or publication order; only the identical slice validation and
operation lifecycles were shared. Build scripts are already small and focused;
introducing a common shell framework would not improve them. Test fixtures and
fuzz oracles were retained rather than shrinking coverage to improve line counts.

Two larger behavior-preserving opportunities remain architectural follow-ups:

- Complete the migration from compatibility execution to immutable command
  plans. Layout-dependent and compound commands still need explicit parity work
  for counts, macro prefixes, partial success, undo grouping, and repeat. Removing
  their remaining executors is not yet a proven-equivalent deletion.
- Consider a narrow schema for repetitive ABI style/value marshalling, and a
  maintained SHA-256 implementation in place of `document/source.rs`'s handwritten
  implementation. The former needs ABI/validation-order parity; the latter adds
  a dependency and build footprint. Neither is required for the consolidations
  implemented here. Existing digest padding-edge tests would support replacement.

## Prioritized proposals requiring approval

Priority balances useful simplification against user-visible cost. None of these
changes was implemented, and `AGENTS.md` requirements remain unchanged.

| Priority | Proposal | Benefit | Exact behavior or requirement change |
| --- | --- | --- | --- |
| 1 | Commit typed style values on Enter or focus loss | Avoids transactions for every valid intermediate number and simplifies text-field undo/reprojection coordination. Buttons, menus, steppers, and color gestures can remain live. | Typing `1`, `12`, `120` updates the document and preview only when the field is committed. Requires changing immediate live application in `AGENTS.md`; continuous-gesture grouping still exists. |
| 2 | Support only the current host ABI after an explicit compatibility cutoff | Removes old input-context/provider adaptation and potentially the independent document-only handle API/registry. The native frontend already uses the newer paths. | Older external callers/providers must upgrade. Existing native editing need not change. Confirm external consumers before selecting the exact exports to retire; the `V1` provider struct name alone does not indicate an old provider, since the native table advertises ABI v2. |
| 3 | Migrate old Viem HTML stylesheet v1 to v2 on an explicit upgrade/save | Removes old stylesheet authoring/update/version-routing paths and eventually a parallel private grammar. | Existing v1 style metadata and class spelling change once during an authorized migration. Keeping backward-compatible import requires retaining the v1 reader; deleting that parser also requires ending runtime v1 support or supplying an external converter. |
| 4 | Retire Normal `U`, retaining `u` and Ctrl-R | Removes separate exact-source line baselines, identity remapping, line restoration, and coordinator bookkeeping. | The dedicated restore/swap-last-changed-hard-line command disappears. Ordinary branching undo/redo remains unchanged. This is an explicit reduction in the supported Vim command set. |
| 5 | Make recorded macro registers executable only | Removes dual text/event representations, macro-to-text control-character conversion, and mixed macro/text append rules. | `p`, `:put`, and Insert Ctrl-R reject recorded macros, including text-only recordings. Appending yanked text into an existing macro program stops being supported; macro execution remains. |
| 6 | Honor semantic bold in document-default character updates | Makes sparse-style updates consistent and removes one special merge policy. Mainly a correctness improvement, with tiny code savings. | `SetDocumentDefaultCharacter` currently ignores incoming `bold`; after this change those requests would affect the default. The pass deliberately preserves the current behavior. |
| 7 | Permit canonical serialization of an affected rich-text paragraph, by explicit policy | Could replace a substantial amount of recovered-tree and whitespace repair with serialization of a normalized paragraph. | Untouched syntax *inside that paragraph* may be rewritten: entity spelling, whitespace, tags, and attributes can change despite equivalent rendering. This relaxes a central preservation guarantee; do not adopt it as an automatic fallback without an explicit product decision. |

Proposal entry points:

1. `EVCompactStyleControls.swift` field callbacks and `EVStyleEditor.swift` edit
   grouping; `AGENTS.md` “Live application, preview, and undo.”
2. `ffi.rs`: `CTextMeasurementProvider`, both host-context input versions,
   `viem_document_*`, and their declarations in `include/viem_core.h`.
3. `document/html_styles.rs`: `parse_rule`, `write_rule`, `parse_rule_v2`, and
   `write_rule_v2`.
4. `command/mod.rs` line-undo tracking/execution and the corresponding source-line
   image translation in `document/mod.rs` and `document/transaction.rs`.
5. `command/registers.rs` macro conversion and append handling.
6. `document/transaction.rs`: `SetDocumentDefaultCharacter` and
   `merge_character_properties`.
7. `document/html_merge.rs`, `html_whitespace.rs`, and recovered-source preparation
   in `edit_translation.rs`/`transaction.rs`.

Two lower-value alternatives are not recommended ahead of this list: dropping
remembered line-spacing drafts saves little while making the controls less
pleasant; restricting Visual Block to monospaced source/plain-text views could
save more, but removes a substantial proportional/bidi rich-text feature.

## Verification and remaining failure

- `cargo test --all-targets --no-fail-fast`: **1,947 passed, 1 failed, 1 ignored**
  across 147 test binaries. The added entity tests account for the two additional
  passing tests relative to the baseline.
- After the final replay-allocation adjustment, all **406 command unit tests**
  passed again; the agent also reran the 12 input-assistance tests.
- `./scripts/test-mac.sh`: **392 passed** (366 XCTest and 26 Swift Testing). The
  script rebuilt the current Rust archive and `.build/Viem.app`, with isolated
  test settings. No compiler warnings were reported in that final run.
- `python3 -m unittest discover -s scripts/tests`: **11 passed**.
- `git diff --check`: passed.
- Independent review found no behavior-preservation issues in the FFI,
  coordinator, or command changes.

The remaining Rust failure is
`projection_fuzz_regressions::markdown_source_fence_edit_requires_explicit_source_intent_when_breaks_reinterpret`.
At `tests/projection_fuzz_regressions.rs:541`, the test expects rejection but
receives a prepared transaction. The same failure was reproduced offline from
an isolated archive of clean `7537b4f`, using a separate target directory. It was
not changed or hidden during this refactor; choosing the intended source-edit
policy is a separate behavior decision.

Local validation logs: `/tmp/viem-quality-rust-all.log`,
`/tmp/viem-quality-command-final.log`, `/tmp/viem-quality-native-all.log`,
`/tmp/viem-quality-python.log`, and the clean-baseline proof at
`/tmp/viem-quality-baseline-pgIoh8/baseline-test.log`.

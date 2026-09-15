# Code quality review — September 2026

The initial behavior-preserving pass was reviewed against
`7537b4f7e85f0d7705356e65886ff27dc3058125` and committed as `bf9ca47`. The user
subsequently approved proposals 2–4: retire obsolete APIs, migrate legacy HTML
style authoring, and remove Normal-mode `U`. Proposals 1 and 5–7 remain pending.

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

## Initial implemented reductions

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

## Approved follow-up: proposals 2–4

The API policy now explicitly permits breaking Rust APIs and the C ABI because
all consumers live in this repository. Callers, providers, declarations, and
tests must change together; both native and core artifacts must be rebuilt.
This permission does not change document-preservation requirements or authorize
other editing behavior changes.

This follow-up removes **1,480 net lines of code and tests**: 1,467 added and
2,947 removed, including both new migration regression files and excluding
documentation. The total reduction across the initial pass and this follow-up
is **5,665 lines**. The API and `U` deletions account for the reduction; robust
HTML migration adds code and coverage.

- **2 — Current APIs only.** Removed the independent `viem_document_*` API and
  registry, host-context v1 input, context-free input entry points,
  effect-discarding format/encoding setters, provider ABI v1 behavior, and unused
  statuses. Core ABI is now 4 and provider ABI 2 is the only supported provider
  version. Existing current request names remain; a `V1` suffix on a retained
  record does not imply an obsolete execution path. Meaningful validation and
  lifecycle coverage moved to the retained APIs.
- **3 — One HTML stylesheet authoring format.** Version 1 remains a strict
  import format; its canonical spelling function serves only import validation.
  Every persisted definition uses the shared version-two writer. The next
  explicit stylesheet operation migrates recognized legacy rules in one
  verified transaction and undo unit. Ordinary typing and no-edit open/save
  do not migrate source. Rules keep their original order; mixed sheets split
  in place around opaque CSS. Legacy classes retain their body elements and
  assignments, with selector-aware CSS and matching native rules for later
  assignments. Generated syntax follows the document's line-ending spelling.
  Deleted headings retain the effective Paragraph CSS, including explicit
  resets; the earlier minimal version-two deletion spelling remains readable
  and is updated on subsequent stylesheet authoring.
  The tests check style graphs, assignments, source preservation, reopen, and
  undo/redo; they do not claim browser-computed-style equivalence for arbitrary
  unsupported CSS. Safe migration adds some production stylesheet code; the
  benefit is one authoring policy, not an immediate line-count reduction in
  this file.
- **4 — Normal-mode `U` retired.** Removed saved-line images, baseline tracking,
  remapping, restoration transactions, repeat support, and history presentation
  dedicated to `U`. Unsupported `U` consumes its count/register prefix without
  changing source, history, registers, or the previous dot command. Ordinary
  `u`/Ctrl-R and Visual `U`/`gU` remain covered. Their regressions replace the
  obsolete saved-line-specific tests.

## Prioritized proposals and status

Priority balances useful simplification against user-visible cost. These were
separate from the initial behavior-preserving pass; proposals 2–4 were later
approved. `AGENTS.md` now records that all API consumers are in this repository,
API/ABI compatibility is unnecessary, and Normal-mode `U` is unsupported.

| Priority | Proposal | Benefit | Exact behavior or requirement change |
| --- | --- | --- | --- |
| 1 | Commit typed style values on Enter or focus loss | Avoids transactions for every valid intermediate number and simplifies text-field undo/reprojection coordination. Buttons, menus, steppers, and color gestures can remain live. | Typing `1`, `12`, `120` updates the document and preview only when the field is committed. Requires changing immediate live application in `AGENTS.md`; continuous-gesture grouping still exists. |
| 2 | Current host APIs only — implemented | Removes old input-context/provider adaptation and the independent document-only handle API/registry. | All repository callers are updated together; no API/ABI backward compatibility is required. See the follow-up above for the retired paths. |
| 3 | Author HTML styles as v2 — implemented | Removes parallel authoring policies; the strict v1 grammar remains for import. Safe migration adds some code. | The next explicit stylesheet operation migrates recognized rules to v2, preserving class assignments and rule order. No-edit open/save remains byte-exact. |
| 4 | Retire Normal `U` — implemented | Removes separate exact-source line baselines, identity remapping, line restoration, and coordinator bookkeeping. | The dedicated restore/swap-last-changed-hard-line command disappears. Ordinary branching undo/redo and uppercase commands remain. |
| 5 | Make recorded macro registers executable only | Removes macro-to-paste control-character conversion and mixed text/macro append reconstruction. Savings are modest: inspection formatting and executing ordinary text registers still need conversion. | `p`, `:put`, and Insert Ctrl-R reject recorded macros, including text-only recordings. Appending yanked/deleted text into an existing macro program stops being supported; macro execution, recording append, and ordinary text append remain. |
| 6 | Honor semantic bold in document-default character updates | Makes sparse-style updates consistent and removes one special merge policy. Mainly a correctness improvement, with tiny code savings. | `SetDocumentDefaultCharacter` currently ignores incoming `bold`; after this change those requests would affect the default. The pass deliberately preserves the current behavior. |
| 7 | Permit canonical serialization of an affected rich-text paragraph, by explicit policy | Could replace a substantial amount of recovered-tree and whitespace repair with serialization of a normalized paragraph. | Untouched syntax *inside that paragraph* may be rewritten: entity spelling, whitespace, tags, and attributes can change despite equivalent rendering. This relaxes a central preservation guarantee; do not adopt it as an automatic fallback without an explicit product decision. |

Proposal entry points (retired paths below refer to the initial review):

1. `EVCompactStyleControls.swift` field callbacks and `EVStyleEditor.swift` edit
   grouping; `AGENTS.md` “Live application, preview, and undo.”
2. `ffi.rs`: `CTextMeasurementProvider`, both host-context input versions,
   `viem_document_*`, and their declarations in `include/viem_core.h`.
3. `document/html_styles.rs`: the import readers, shared `write_rule` /
   `write_rule_for_selector`, and `migrate_legacy_sheet`.
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

### What proposals 5 and 6 mean

**5 — Recorded macros as programs.** A register is a named slot used for copied
text or a recorded command sequence. Today, recording a macro containing `x`
in register `a` gives that slot two uses: `@a` executes `x` and deletes a
character, while `"ap` inserts the literal letter `x`. More complex recordings
can be pasted as command letters plus control characters. Recordings containing
arrows already reject pasting because those keys have no literal-text meaning.

This proposal would consistently reject pasting any recorded macro through
`p`, `:put`, or Insert-mode Ctrl-R. It would also reject mixing copied text into
a recorded macro through uppercase-register append, such as `"Ayy`. Executing
and inspecting macros would remain. Recording more commands with `qA`, appending
ordinary text to text registers, and executing an ordinary text register with
`@a` can all remain supported. Overwriting a macro slot with ordinary text would
still replace the macro.

The tradeoff is losing the ability to paste a recorded macro into the document,
edit its command text, and copy it back for execution. This is a real Vim
workflow. The original proposal overstated the storage simplification: macro
inspection formatting and ordinary text-to-command conversion still remain.
The main saving is a smaller boundary between recorded programs and pasteable
text, not removal of all register conversions.

**6 — Default bold is currently ignored by one configuration setter.** The
specific `SetDocumentDefaultCharacter` operation merges supplied character
defaults but explicitly preserves the old bold flag. For example, supplying
`size: 18` and `bold: true` changes the size but does not enable bold; supplying
`bold: false` does not disable it either. The proposal would use the common
property merge and honor that field along with the others.

The exact setter currently has no production caller in this repository. Its
callers are tests, including a call inside `ffi.rs`'s test module. Ordinary
selection and typing Bold actions follow different paths and are unaffected.
This is a small internal consistency fix, with very little code reduction;
it is not evidence that the current Bold UI is broken. It remains unimplemented.

## Follow-up verification

- The full `cargo test --all-targets --no-fail-fast` run passed **1,940 tests**,
  with **1 known failure and 1 ignored**, across 147 binaries. This run preceded
  the final native-block deletion CSS correction and its new regressions.
- After that correction, all **10 stylesheet unit tests and 66 affected
  integration tests** passed. These cover the style graph, native/class
  assignments, CSS ordering and fallback declarations, opaque rules, both HTML
  views, encoding/line-ending preservation, reopen, and exact undo/redo.
- `cargo test --all-targets --no-run` compiled all final Rust test targets.
- The final `./scripts/test-mac.sh` rebuilt the app and passed **392 tests**:
  366 XCTest and 26 Swift Testing. Existing deprecated AppKit scroller-arrow
  warnings remain in the unchanged scrollbar implementation/tests.
- `git diff --check` passed. Independent reviews covered retired API contracts,
  command/history behavior, in-place CSS migration, and deletion fallback CSS.

The Rust failure is the same baseline Markdown fence-policy failure documented
below. No additional failures appeared. Local follow-up logs:
`/tmp/viem-retire-compat-rust-all.log`,
`/tmp/viem-html-style-migration-unit.log`,
`/tmp/viem-html-style-migration-final-integration.log`,
`/tmp/viem-retire-compat-compile-final.log`, and
`/tmp/viem-retire-compat-native-final.log`.

## Initial pass verification and remaining failure

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

### Subsequent failure follow-up

The Markdown Source fence-policy failure recorded above has since been
resolved: `markdown_source_fence_edit_reparses_immediately_through_either_path`
now verifies the intended source-view behavior, including exact patches,
reprojection, position maps, and history. All 12 `projection_fuzz_regressions`
tests passed when this historical failure was rechecked.

The later rich-caret failure recorded in `code-syntax-validation.md` was still
reproducible and is now fixed by using the shared visible-line insertion
resolver for ordinary Markdown payloads. Its original regression passes
unchanged; additional coverage checks source locality and undo/redo.

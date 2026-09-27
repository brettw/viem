# Test-suite audit — 19 September 2026

Historical measurements: standalone HTML modes and their dedicated test modules
were subsequently removed. References below record the suite at the audit date.

The initial audit implemented speedups without removing or newly ignoring tests,
or reducing fixture sizes or iteration counts.

**Approved follow-up:** all 14 tests originally proposed below have now been
removed at the user's request: 12 Rust tests and 2 macOS tests. This includes the
eight conditional command matrices. Their unused matrix helpers and the empty
`EVEditorCompositionTests.swift` file were also removed. The retained tables
record the coverage lost and the surviving related tests. Measurements below
predate these removals; they measure the earlier speedups.

Follow-up validation passed:

- `cargo test --locked --lib` and `cargo test --locked --test all -- vim_command_matrix:: ffi_surface:: core_acceptance:: projection_open_work:: vim_exact_conformance::`:
  1,497 passed, 0 failed, 3 existing ignored tests.
- `scripts/test-mac.sh --filter 'EVFunctionKeyMappingTests|EVLaunchArgumentsTests'`:
  23 passed, 0 failed; ABI validation, runtime verification, app packaging, and
  the Swift build also passed.
- The source test-name inventory confirms exactly the 14 approved removals.

## Measurement method

Measurements were taken on this checkout on arm64 macOS 26.6.2, with 20 logical
CPUs, Rust 1.98.1 and Swift 6.3.3. Builds and test runs were serialized so they
did not compete with another suite. These are local observations, not CI latency
budgets. Per-test times include contention with other tests in the same binary;
do not add those durations to estimate suite wall time.

Rust test executables were discovered using
`cargo test --locked --no-run --message-format=json`. Each pass launched every
executable in the same order with its normal test-thread count. The existing
stable compiler built both versions. For measurement only,
`RUSTC_BOOTSTRAP=1` was set on the already-built executables to expose libtest's
`--format json -Z unstable-options --report-time`; it was not set on Cargo or
rustc. All complete passes ran **2,733 passing tests and 4 existing ignored tests
across 168 binaries**.

The comparison below uses warmed executable launches in both profiles, with no
concurrent discovery/build/test work. The unoptimized unit-test binary already
includes the two small helper improvements, while its optimization is explicitly
set to zero; the unoptimized integration binaries are the original ones. This is
a conservative comparison for the combined changes, rather than attributing the
helper savings to compiler optimization. Separate unchanged-profile helper
measurements are recorded below.

The initial audit pass took 242.758 seconds, but overlapped the tail of a
`--help` discovery pass and included startup overhead. It is retained as an
observation, not used as the headline baseline. The original pre-measurement
build took 41.15 seconds with existing artifacts; that was not a clean-build
benchmark.

Native tests use `scripts/test-mac.sh`, including ABI validation, matching Rust
archive, app packaging, profile isolation, and Swift tests. AppKit measurements
require access to the desktop and pasteboard; an initial sandboxed run produced
unavailable-screen/pasteboard/file-I/O failures and is excluded from comparisons.
The Python tooling tests use unittest discovery under `scripts/tests`. Windows
WinUI diagnostics were inspected but cannot be executed or timed on this host.

## Measured results

| Measurement | Before | After | Scope |
| --- | ---: | ---: | --- |
| Rust wall time, previously launched executables | 205.987 s | 33.888 s | Same 168 binaries and 2,733 passing tests; **83.5% less wall time**. |
| Rust time reported inside libtest suites | 178.911 s | 32.738 s | Excludes time outside the test harness; **81.7% less execution time**. |
| Complete macOS runner | 194.14 s | 178.64 s | Build/ABI/package plus 758 XCTest and 38 Swift Testing cases; **8.0% less wall time**. Same four preexisting failed cases. |
| XCTest execution | 159.824 s | 156.708 s | All 758 cases retained. |
| Python tooling tests | 3.639 s | Unchanged | All 15 pass; no Python changes. |

The first build of the optimized Rust profile took **119.96 seconds**, including
new optimized dependency artifacts. Its first full execution took **258.781
seconds**, of which libtest reported only **34.687 seconds**: about **224 seconds
were outside the test harness**. In particular, newly built tiny test binaries
frequently took 1–2 seconds per process despite milliseconds of test work.
The subsequent identical full execution took 33.888 seconds. This host's
first-launch overhead across 168 executables remains a major cost; warmed
numbers are not a promise for a newly linked suite. The next potential structural
improvement is consolidating integration-test executables after auditing shared
state and fixture isolation. This change retains the current process boundaries.

These observations also make build costs important: the optimized profile speeds
execution while increasing compilation work. A no-change `cargo test --locked --doc` validation reused that profile in
0.05 seconds and passed (there are no current doctests). No matched clean-build comparison was performed.

### Slowest Rust tests

Times below are from the two complete, previously launched-binary passes. Tests
still run concurrently within each binary, so individual durations include
contention and should not be summed.

| Test | Unoptimized | Optimized |
| --- | ---: | ---: |
| `viem_core::command::layout_motion::tests::document_home_insert_and_replace_keep_large_literal_documents_unflattened` | 28.736 s | 7.963 s |
| `pending_typing_style::rtf_scalar_typing_keeps_group_depth_bounded_and_undo_exact` | 26.569 s | 3.412 s |
| `viem_core::coordinator::viewport::tests::newline_near_bottom_in_markdown_reveals_only_the_clipped_row` | 16.703 s | 4.855 s |
| `viem_core::coordinator::viewport::tests::typing_near_bottom_preserves_visible_rows_in_every_text_format` | 16.108 s | 5.130 s |
| `html_source_layout::large_html_source_flow_captures_only_visible_semantic_blocks_and_rejects_stale_jobs` | 13.967 s | 2.669 s |
| `viem_core::coordinator::tests::composition_in_long_plain_and_flowed_paragraphs_uses_bounded_slices` | 13.578 s | 2.585 s |
| `html_code_lists::large_pre_edits_preserve_paragraph_identity_and_bounded_invalidation_in_both_views` | 12.187 s | 2.558 s |
| `viem_core::layout::engine::adjacent_regions_tests::adjacent_regions_preserve_sparse_hit_testing_and_logical_selection_boundaries` | 6.655 s | 1.416 s |
| `viem_core::command::indentation::tests::million_lines_indent_and_comment_enter_do_not_flatten` | 5.868 s | 7.470 s |
| `markdown_source_layout::ten_thousand_source_groups_keep_capture_edits_and_flow_invalidation_bounded` | 5.714 s | 0.892 s |
| `viem_core::layout::engine::tests::shaped_payload_budgets_bound_repeated_large_document_regions` | 5.568 s | 1.072 s |
| `viem_core::command::reflow::tests::small_reflow_in_a_million_line_document_stays_local_and_never_flattens` | 5.342 s | 7.431 s |
| `rich_incremental::ordinary_rich_edits_reparse_one_line_and_share_unaffected_indexes` | 5.009 s | 0.848 s |
| `paragraph_flow::long_source_flow_retains_first_paragraph_geometry_after_checkpoints_and_resize` | 4.950 s | 0.825 s |

The two million-line indentation/reflow tests appear slower inside the optimized
parallel run. A targeted check running each sequentially showed improvements
from **2.723 → 0.420 seconds** and **2.160 → 0.345 seconds**, respectively.
Their parallel timings reflect different contention as other tests finish
sooner. No fixture, iteration count, cache/work bound, or undo assertion changed.
The slow RTF test still performs all 2,000 scalar insertions in its primary case.

### Slow native tests retained

| Test | Before | After |
| --- | ---: | ---: |
| `EVLayoutPaintIntegrationTests.testLongFlowedParagraphDrawsOnlyVisibleInkAfterResizeAndMetricsInvalidation` | 22.670 s | 22.856 s |
| `EVPointerSelectionTests.testOffscreenDragKeepsItsAnchorAndVisibleSelectionAcrossLongDocuments` | 16.008 s | 15.482 s |
| `EVMarkdownSwitchIntegrationTests.testLargeMarkdownModeSwitchPublishesOneViewportAndPreservesSource` | 8.414 s | 8.292 s |

These test native culling after metrics/width changes, offscreen selection while
scrolling long documents, and large source/view switching. They remain valuable;
small differences here are run-to-run variation, not claimed improvements.


## Implemented speedups

- `Cargo.toml`: build the test profile at optimization level 1, retaining debug
  assertions, overflow checks, and debug information. This targets the actual
  multi-megabyte and million-line regression workloads; it does not shrink their
  fixtures or relax their bounds. The ordinary dev/native profile is unchanged.
  The tradeoff is a separate optimized test build and slower recompilation.
  For a tight edit/run loop involving only a tiny test, an explicit
  `CARGO_PROFILE_TEST_OPT_LEVEL=0 cargo test <filter>` override remains available;
  switching profiles itself can require rebuilding artifacts.
- `src/core/layout/mock.rs`: only look for a mock ligature when the first
  grapheme is an uncombined `f`, and only compare styles after the text matches
  one of the four supported ligatures. Previously every grapheme incurred
  repeated style lookups/comparisons. Candidate order, feature handling, style
  boundaries, shaping context, and output remain the same. Existing ligature,
  cache-invalidation, and large-document tests exercise this shared helper.
- `src/core/document/formatted_text.rs`:
  `utf8_and_utf16_boundaries_map_through_tree_aggregates` now computes expected
  UTF-16 offsets cumulatively instead of re-encoding every preceding prefix.
  Both conversions remain asserted at every scalar boundary and at EOF, and
  invalid-boundary checks remain. The independent oracle becomes linear.

A separate two-test check kept optimization disabled in both builds to isolate
helper changes from the profile change. The shape-payload stress test decreased
from 5.521 to 5.074 seconds; the UTF-16 boundary test decreased from 0.472 to
0.345 seconds. These single-run measurements indicate modest gains; the complete
suite comparison includes both the helper changes and compiler optimization.

- `scripts/test-mac.sh`: export `MACOSX_DEPLOYMENT_TARGET=26.0` before
  the ABI check, matching app packaging. The old runner recompiled the bundled
  C syntax parsers when it changed this value partway through the run. Native
  ABI/app Cargo phases fell from 7.38 + 5.87 seconds to 0.41 + 0.04 seconds,
  without bypassing ABI validation, packaging, or signing.
- `EVCodeScrollingTests`: remove 40 unconditional 25 ms sleeps before scrolling.
  Every iteration still waits, with a deadline, for observable syntax paint in
  the visible range. The viewport sequence fell from 1.600 to 1.028 seconds and
  wheel sequence from 3.036 to 2.242 seconds in the full native runs. All
  destinations, input iterations, geometry checks, and persistence assertions
  remain. The complete three-test scrolling class passed twice more with the
  new waits, using `swift test --skip-build --filter EVCodeScrollingTests` and
  isolated profiles.

## Existing native failures

The desktop-capable baseline reports five failed assertions across four tests:

- `EVCodeStyleMenuTests.testCodeMenuListsEveryCharacterDefinitionAndEditsTheGlobalTarget`
  expects the internal `* Incremental match` style in the menu; the menu omits it.
- `EVCoreStateMenuIntegrationTests.testJumpToSelectionRevealsCoreCursorAfterSelectionLeavesLayoutCoverage`
  expects a nil offscreen selection export (and a related false state), while
  the current export retains logical segments with no rectangles.
- `EVCoreStateMenuIntegrationTests.testMarkdownSemanticStyleMenuUsesLogicalSelectionWhenGeometryIsOffscreen`
  also expects a nil offscreen export instead of logical segments with no
  rectangles.
- `EVCoreStateMenuIntegrationTests.testNativeCopyUsesTheCoreSelectionWhenItsGeometryIsOffscreen`
  has the same nil-export expectation.

The full after-run reproduced exactly the same four failed test cases, with
no new failures. These four tests were retained. Their menu, selection, and
clipboard behavior coverage remains useful; reconcile the expectations with the intended API
contract separately. No assertions were relaxed to make the speedups pass.

## Removed tests — duplicate and weak checks

These approved removals reduce duplication and weak checks, with very small
runtime savings. They do not address the suite's primary bottlenecks. Test names
below include the integration binary, module, or XCTest class.

| Removed test | Reason | Coverage lost | Surviving coverage |
| --- | --- | --- | --- |
| `vim_command_matrix::visual_block_join_uses_touched_hard_lines_and_is_one_undo_unit` | Duplicates the next test's source, narrow layout, Visual Block join, result, and undo assertions. | A separate invocation using the default Visual line mode; no distinct edit or undo expectation. Baseline: 0.018 s. | `visual_block_join_is_line_mode_independent_and_dot_replays_its_block_extent` repeats this case, adds Physical Source mode, `gJ`, and dot replay. |
| `ffi_surface::explicit_format_constants_remain_disjoint` | Checks only three integer inequalities, without checking every pair, header values, or decoding. | Direct pins that PlainText differs from Markdown, Unix from DOS, and DOS from Mac. Baseline: below 1 ms. | `ffi_surface::latin1_utf16_and_markdown_sources_round_trip_exactly` exercises these discriminators with different observable decoded text; ABI validation separately checks declarations. |
| `document::tests::grapheme_clusters_are_indivisible` | Repeats the same public deletion rejection and successful combining-cluster deletion as the acceptance suite. | The minimal `a` + combining accent + `b` fixture; no unique boundary rule or API path. Baseline: below 1 ms. | `core_acceptance::grapheme_clusters_are_atomic_for_document_and_controller_edits` repeats the invalid and valid deletion boundaries, checks the next boundary, and adds a ZWJ/controller case. |
| `document::tests::no_op_open_save_is_byte_exact_for_every_encoding` | A small identity-only matrix overlapped by broader identity-and-edit matrices. | Four exact fixtures: BOM UTF-8 `hello\r\n`, Latin-1 `Hé\r\n`, and BOM UTF-16 LE/BE `Hé\n`. Those precise fixture combinations were removed; the encodings and identity property remain covered. Baseline: below 1 ms. | `core_acceptance::text_pipelines_preserve_no_op_bytes_and_patch_locally_in_every_encoding` and `projection_open_work::opening_decodes_each_encoding_once_and_preserves_exact_source`. Slightly lower confidence because the fixtures differ. |
| `EVEditorCompositionTests.testFrontendCompositionInstalls` | Calls `EVEditorComposition.install()` with no assertions. | A stand-alone no-crash smoke invocation of global installation. Baseline: below the native timer’s 1 ms resolution. | Launch fixtures and richer frontend/AppShell integration exercise installation and its observable behavior. |
| `EVFunctionKeyMappingTests.testUnmodifiedFunctionKeyIsMappedInsteadOfInsertedAsPrivateUseText` | The broader modifier test repeats F2 → `l`, expected cursor 1, unchanged text/dirty state, and no command error. | Isolation with no other mappings installed, and immediate text/dirty-state checks after F2; the broader test checks those states after all six inputs. Baseline: 0.050 s. | `testFunctionModifiersAndHighFunctionNumbersReachTheirDistinctMappings` includes the same input/output case, plus modifier variants and F35. |

Sources: `tests/all/vim_command_matrix.rs`, `tests/all/ffi_surface.rs`,
`src/core/document/mod.rs`, `src/mac/Editor/Tests/EVEditorCompositionTests.swift`,
and `src/mac/Editor/Tests/EVFunctionKeyMappingTests.swift`.

## Removed tests — status-only command matrices

The removed matrix helper in `tests/all/vim_command_matrix.rs` checked that dispatch
produced no error and finished as either `Complete` **or `Cancelled`**. It did
not check final text, cursor, selections, registers, or mode. An implementation
that returned `Complete` without doing the command could pass.

These tests covered real grammar combinations and were originally conditional
removal candidates. The user approved removing all eight; the combinations below
are accepted coverage losses, not claims of complete duplication. Related
behavioral tests remain. The entire binary originally took about a quarter
second, so this is primarily a test-quality decision.

| Removed test | Coverage lost | Stronger overlapping coverage |
| --- | --- | --- |
| `vim_command_matrix::normal_motion_and_search_matrix` | Dispatch acceptance for 71 basic/word/row/line/find/document/viewport/mark/jump/search spellings. | `vim_exact_conformance::normal_motion_counts_land_on_exact_grapheme_and_line_boundaries`; specific viewport, row-edge, page, and search tests. |
| `vim_command_matrix::operator_shorthand_and_text_object_matrix` | Operator, shorthand, put, insert-entry, Unicode, and text-object spellings, including simple counts/registers. Some aliases may lack exact-result counterparts. | Exact operator endpoints, shorthand ranges/registers/cursors, put cursor semantics, and pair-text-object alias tests. |
| `vim_command_matrix::pending_prefix_cancellation_matrix` | Escape dispatch after count, register, `d`, `g`, `z`, and `f` prefixes. The removed matrix did not assert exact cancellation or non-destructive state. | Pending-operator cancellation, source/register preservation, and exact Visual cancellation tests; not every prefix is necessarily duplicated. |
| `vim_command_matrix::insert_and_replace_input_matrix` | Both modes crossed with 14 controls: escape aliases, breaks, tab, deletion, navigation, register insertion, and Ctrl-O under narrow layout. | Exact Insert controls, Replace restoration, line-mode edge keys, and undo-group navigation tests. Unique mode/key pairs are no longer covered by this matrix. |
| `vim_command_matrix::command_line_editing_matrix` | Five exact prompt sequences involving movement, Backspace/Delete, Home/End, history, and Escape. | Grapheme-aware prompt editing/history and prompt-selection tests assert actual contents/carets/state. |
| `vim_command_matrix::ex_command_surface_matrix` | Acceptance of 35 Ex spellings for host/file requests, edits, information, navigation, and options. It does not verify host effects. | Typed Ex result tests and `ex_file_commands`, `ex_goto`, and `ex_misc` integration tests. |
| `vim_command_matrix::visual_mode_by_motion_matrix` | Seventy motions crossed with Character, Line, and Block Visual modes at a narrow width. This is substantial smoke coverage despite the weak oracle. | Specific directional Visual jumps, marks/text objects, selection geometry, and viewport tests. Not a complete replacement for the removed cross-product. |
| `vim_command_matrix::visual_mode_command_matrix` | Four Visual entries plus 23 commands across three Visual modes; no resulting extents/edits/registers are asserted. | Exact Visual mode/count/edit/case/undo tests. Some removed cross-product combinations may have been unique. |

## Tests and checks to keep

- Million-line, multi-megabyte, and large rich-document tests guard explicit
  structural performance requirements. Their slow runtime is not evidence of
  low quality. Preserve fixture sizes, work counters, cache invalidation,
  history/undo, and independent fresh-result checks.
- Native pointer/presentation tests using `AGENTS.md`, 20,000 lines, and the
  million-line fixture are explicitly required by the specification. Retain
  pixel-equivalence and bounded-export/shaping assertions.
- Architecture source scans implement an explicit repository boundary
  requirement; do not remove them merely because they inspect source text.
- Literal serialized HTML/RTF/Markdown expectations protect source-byte
  preservation and patch locality, which are public product guarantees.
- The regex dialect/version pin is a deliberate product contract.
- Python runtime-integrity tests cover distinct corruption, inventory, and
  preservation cases. No worthwhile removals identified.
- Windows diagnostics contain fixed waits of roughly 80–650 ms. Replacing
  these with bounded waits for observable state is worth investigating on a
  Windows host; no unvalidated WinUI changes were made here.

# macOS pointer presentation validation

Validated on 2026-09-19, macOS 26.6.2 (25G83), arm64, using the debug Rust and
Swift builds. These measurements cover synchronous core/API work and native
bitmap drawing; compositor presentation and end-to-end drag latency were not
measured. Timings are observations, not test thresholds.

## Presentation exports

`EVPresentationCacheIntegrationTests` alternates 80 visible cursor/selection
placements after warming the initial presentation. The fixtures are `AGENTS.md`
in Markdown Source format and 20,000 plain-text lines. Each placement is followed
by a presentation refresh and direct geometry/paint export requests.

| Fixture | Core placements | Presentation refreshes | Cached export requests |
| --- | ---: | ---: | ---: |
| AGENTS.md | 8.364 ms | 21.405 ms | 1.249 ms |
| 20,000 lines | 10.832 ms | 25.185 ms | 1.208 ms |

Both fixtures retained the same full layout identity throughout the loop, with
zero additional Core Text shaping batches, geometry/decorations copies, paint
copies, whitespace copies, or formatted text range copies. Selection geometry
continued to update. Native selected-text requests reused the visible text
slices. Forced cache eviction produced exports identical to the retained ones.

## Drawing

`EVPointerDrawingPerformanceTests` applies 20 selection updates to an existing
bitmap, drawing the accumulated damage after each update. It also renders every
frame from scratch and compares all pixel bytes. Its fixtures are `AGENTS.md`
and 20,000 plain-text lines containing tabs and trailing spaces. Both use an
active native test window, including the custom block/Visual caret.

| Fixture | Full drawing | Partial drawing | Clusters submitted, full / partial |
| --- | ---: | ---: | ---: |
| AGENTS.md | 60.719 ms | 27.819 ms | 14,280 / 169 |
| 20,000 lines | 89.407 ms | 26.454 ms | 17,860 / 338 |

Every partial frame matched fresh rendering exactly. Forty native pointer events
also verified one presentation refresh per event, unchanged layout identity,
zero new geometry copies or shaping, and bounded selection damage.

The regression tests cover edit/Undo/Redo, resize/zoom, metrics retirement,
whitespace settings and viewport validation, paint styles/list decorations,
composition updates/cancellation, theme, appearance, and backing-property
notifications. Existing million-line, scrolling/autoscroll, caret-ink,
accessibility, clipboard, and style-inspector tests were included in the full run.

## Reproduction and suite result

Run `scripts/test-mac.sh` to check the C ABI, rebuild/package the matching core
and frontend, and run native tests with an isolated temporary profile. To run
only the new tests, append
`--filter 'EVPresentationCacheIntegrationTests|EVPointerDrawingPerformanceTests'`.

The full native run passed 756 of 757 XCTest tests and all 38 Swift Testing tests.
All 11 new tests passed. The sole failure was the pre-existing
`EVCodeStyleMenuTests.testCodeMenuListsEveryCharacterDefinitionAndEditsTheGlobalTarget`
assertion: its expected character-style set includes the internal
`* Incremental match` style that the menu intentionally hides.

The restricted agent sandbox denied native AppKit safe-save/file-coordination
and clipboard operations. Repeating the same built suite outside that sandbox,
still with an isolated temporary profile, cleared those environmental failures.

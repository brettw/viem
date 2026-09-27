# TODO3 and GAPS validation

Historical validation: HTML editing modes referenced below have since been
removed. HTML files now open in Code.


Validation used the isolated `com.viem.todo-validation` application bundle,
`VIEM_CONFIG_DIR=/private/tmp/viem-todo3-config`, and temporary source fixtures.
The user's running editor and personal configuration were left intact.

## Automated checks

- `cargo test --offline --all-targets`: 1,316 tests passed across 62 targets.
- `scripts/build-mac-app.sh`: Rust/Swift build and ad-hoc application signing passed.
- `scripts/test-mac.sh`: 280 native tests passed (256 XCTest + 24 Swift Testing).
  The helper builds the current archive/app and isolates configuration for every run.
- `cargo fmt --all -- --check`, `git diff --check`, compatibility JSON parsing,
  and every referenced compatibility-test path are checked before delivery.

## Computer-use checks

- `:e second.txt` replaced the active window's document after `:cd`; `:E`
  created another window, with both documents present in the Window menu.
- `:pwd` displayed the current directory in selectable read-only output.
  Its visible X dismissed the bar, and the next `:` replaced it with a prompt.
- Shift+arrow and mouse-drag selection worked in the prompt. Native
  Cmd+C/Cmd+V and Cmd+A copied, pasted, and replaced prompt text without editing
  the document. Shift+F10 opened the native Cut/Copy/Paste context menu.
- A Theme change wrote `config.json`; relaunch preserved that theme.
  Format > Style > Save as default markdown style wrote the expected JSON.
- A newly opened HTML file inherited Georgia 22 from `html_style.json`.
  Making its first word bold and saving produced only `<b>Inherited</b>`;
  no inherited font or size was serialized.
- HTML code rendered in real monospace and dark green. HTML-to-Markdown kept
  visible text, emphasis and code blocks, reported information loss, and wrote
  valid Markdown. Undo restored HTML interpretation and source.
- Explicit Latin-1 conversion changed an unrepresentable heart to `?` and
  displayed a warning counting one replacement. Undo restored the Unicode text.
- `:w alternate.html` wrote exact source bytes without changing the current
  binding. `:2,3w range.txt` wrote exactly `a\nb\n` from the selected lines.
- `:set ic sc` followed by `%s/cat/dog/g` replaced all three case variants.
  A selected match ending inside a decomposed grapheme returned
  `RegexMatchSplitsGraphemeCluster` without changing the buffer. One undo still
  undid the preceding successful substitution. Explicit multiline substitution
  consumed a semantic break and reported one replacement.
- After an external fixture edit, `:checktime` warned and preserved the buffer.
  Ordinary `:w` preserved the external file; explicit `:e!` reloaded it.
- `:s saved.txt` displayed guidance to use `:w` or `:saveas`.
- Inserting a literal backtick inside inline code changed its serialization from
  one backtick delimiter to two, preserved visible `c` + backtick + `ode`, and
  saved successfully.

Wheel direction is additionally covered by native event tests for horizontal
and vertical, discrete and precise deltas, boundary clamping, wrapping and
AppKit's already-adjusted direction. The system-wide scrolling preference was
not changed during validation.

## Scope

All TODO3 items were implemented. The bottom of GAPS.md records the compatibility
candidates not adopted for this prose-focused editor. `docs/compatibility.json`
links the supported and intentionally different capabilities to their tests.

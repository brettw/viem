# Paragraph, list, style-control, and zoom validation

Historical validation: standalone HTML modes and their dedicated tests were
subsequently removed. HTML-mode results below describe the earlier implementation.

Validated on macOS 26 with the isolated `com.viem.todo-validation` app and
temporary fixtures/configuration. The user's normal editor and settings were
not used for test edits.

## Implemented behavior

- Markdown and HTML paragraphs use 7pt before and 7pt after spacing. Under
  Viem's additive spacing rule, adjacent paragraphs have a 14pt gap at the
  default 14pt font size. This approximates the common one-em paragraph gap in
  [HTML rendering defaults](https://html.spec.whatwg.org/multipage/rendering.html#flow-content-3).
- Fenced Markdown and HTML preformatted code share one paragraph per block,
  retaining indentation, explicit internal breaks, and blank rows. Markdown
  Code Block has a 32pt start indent. Source views retain literal syntax.
- Markdown list continuations flow within items; ordered labels count from
  the first ordinal and unordered labels render as bullets. Source marker
  spelling remains exact. Labels hang outside the item body, with a 32pt
  default body inset per level. Signed first-line body indents remain active.
- Nine numeric style controls have native steppers, inherited reset behavior,
  invalid-draft handling, and continuous-gesture undo grouping.
- Zoom controls use all 17 requested stops from 25% through 500%, saturate at
  the endpoints, and use Option-Hyphen/Option-Equals.

## Computer-use checks

- Screenshots at 100% and 175% confirmed paragraph separation, code spacing
  only around the block, preserved code blank rows, and aligned bodies for
  list items 9 and 10 and their wrapped continuations.
- Repeated `$`, followed by `A` and typing at the first visual row's end in a
  numbered Markdown item, inserted at the expected boundary. Undo restored it.
- Enter and typing inside HTML code produced an internal line with the same
  code style and no extra paragraph gap.
- Repeated mode switching after that edit reproduced the reported backend
  reflow failure before the fix and rendered successfully afterward. Undo
  restored the source and the preceding WYSIWYG presentation.
- Edit Styles showed the 32pt Markdown code indent, inherited 7pt spacing,
  and up/down arrows. Clicking the size arrows changed 14 to 15 and back.
  Normal line spacing disabled its numeric control and stepper.
- Zoom reached the upper bound with Zoom In disabled, and the lower bound
  with Zoom Out disabled. A 500% screenshot also showed intact block-caret
  glyph rendering.
- The automation tool's `alt+equal` emits Option-Shift-Equals. Temporary key
  tracing established that distinction; the unshifted keypad-equals alias
  exercised the intended command. Native tests cover physical key code 24
  with Option alone. Diagnostic tracing was removed.

## Quality regressions covered

- HTML code Enter at empty, start, middle, and end boundaries, including
  nested code and code inside lists; exact source undo/redo.
- HTML list child blocks retaining their item identity without phantom labels.
- Canonical Markdown labels, nested/lazy continuations, list exit, nested
  code indentation, and newly introduced list levels after incremental edits.
- Negative first-line indents, inherited list-style inset duplication, and
  natural RTL items beginning with digits or directional isolates.
- Source projection with blank code rows, including switching modes after
  an edit and resizing. The native pass exposed an empty style-span failure
  at consecutive source line endings.
- Character-by-character HTML code indentation remains literal whitespace
  rather than accumulating nested preservation spans. Explicit HTML
  whitespace overrides still use the existing preservation policy.
- Regional projection/layout work in 10K-item and 10K/20K-line code documents,
  cache invalidation after edit/resize/zoom, and bounded long-line capture.

## Final verification

- `cargo test --offline --no-fail-fast`: 1,406 passed, zero failures.
- `scripts/test-mac.sh` with the required AppKit/file-coordination access:
  274 XCTest and 26 Swift Testing tests passed, zero failures.
- The rebuilt and signed `.build/Viem.app` passed the final computer-use
  reproduction: code indentation serialized as `<br>    added_line()`, HTML
  Source rendered its blank code row without a reflow error, and two undos
  restored the original source and WYSIWYG presentation.

Logs: `/private/tmp/viem-paragraphs-rust-complete.log` and
`/private/tmp/viem-final-native-whitespace-approved.log`.

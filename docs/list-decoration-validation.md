# WYSIWYG list decoration validation

Historical validation: standalone HTML modes and their dedicated tests were
subsequently removed. HTML-mode results below describe the earlier implementation.

Validated September 6, 2026.

## Behavior

Markdown, HTML, and RTF WYSIWYG projections contain list body text and paragraph
boundaries. Labels and their following gap are shaped and painted separately by
layout. They do not occupy editable text, register, selection, or caret positions.
Source-visible modes retain literal syntax. Clicking a label resolves to the body.

Wrapped list `$`, `A`, `y$`, and `d$` share an endpoint before the wrap separator.
The separator stays in logical text. Whole-item linewise deletion carries explicit
model intent; deleting/changing the only word retains the list item. Enter,
empty-item exit, paste, numbering, conversion, and undo preserve source structure.

The implementation also fixes empty nested HTML/RTF insertion anchors, repeated
RTF numbering, first-word loss in cross-format list conversion, RTL decimal label
direction, and marker shaping evicting the body shaping cache.

## Automated checks

- Full Rust suite: 1,440 passed; one manual performance benchmark ignored.
- Full native suite: 320 passed (294 XCTest and 26 Swift Testing tests).
- Native tests cover real Core Text marker pixels and color, empty items, body-only
  selection and hit testing, source switching, zoom, stale layout exports, and
  atomic buffer-capacity failures.
- Existing native list-editing assertions now inspect body text and decoration
  labels separately; their 12-test suite passes.
- Layout checks include cache invalidation, independent marker caching, 10,000-item
  regional work bounds, and continued long-line layout emitting each marker once.
- Command checks include bullet/decimal fixtures, counts, registers, operators,
  dot repeat, Unicode graphemes, empty items, partial rows, and exact source undo.

Final logs are in `/private/tmp/viem-list-final-core-tests.log`,
`/private/tmp/viem-list-native-complete-final.log`, and
`/private/tmp/viem-list-native-editing-final.log`.

## Computer use

Used the separately signed `com.viem.todo-validation` app with disposable Markdown
and HTML fixtures and an isolated configuration directory. The user's running
editor and configuration were left alone. Screenshots were inspected in-session.

At a narrow 560pt window, both formats were checked for:

- Hanging bullet and decimal labels, including 9-to-10 digit transitions and
  empty items; body text aligns across wrapped rows.
- Clicking a bullet/number followed by `x` deletes the first body letter.
- `$x` deletes the last visible letter on a wrapped row, and undo restores it.
- `A` inserts before the wrap space, without changing list labels.
- `ciw` leaves an empty decorated item; typing and undo restore the body.
- Enter exits an empty HTML list item.
- Source mode displays original markup; settled Markdown source mode permits
  deleting its literal bullet.

The final release package also passed `$x` and undo at 200% zoom. The screenshot
showed complete labels and glyphs at this zoom. The validation app was closed
without saving fixture changes.

## Existing limitation

Paragraph inset distances remain fixed while zoom scales glyphs. Unusually wide
labels at extreme zoom can exceed the available gutter/canvas margin. This change
preserves that existing inset policy; standard one- and two-digit labels were
visually checked at 100% and 200%.

# HTML end-of-document editing

## Reproduced failures and changes

Typing a hyphen after an invisible trailing element, such as
`<p>first</p><span></span>`, returned `VerificationFailed`. The same state is
created by exiting a final list item and adding another blank row. The source
then contains a final paragraph followed by an empty list container.

The HTML projection retained an insertion anchor inside that invisible element,
across a deferred paragraph boundary. Inserting there materialized an unexpected
newline, so the semantic verifier correctly rejected the candidate. The
projection now retains empty-element anchors only within the current visible
paragraph. Existing source syntax remains untouched.

The quality pass also found that splitting an HTML paragraph beside a visible
collapsible space could fail verification: HTML dropped the space at the new
paragraph edge. The split now explicitly preserves that adjacent spacing using
the existing reversible whitespace representation.

## Computer-use checks

The original release app reproduced the list-ending failure with:

1. Open `<ul><li>first</li></ul>` in HTML WYSIWYG.
2. Type `GA`, press Return three times, then type `-`.
3. Observe “This edit cannot preserve the format's text and structure.”

The rebuilt app passed the same sequence and displayed `first\n\n-` without an
error. Each Up moved to the immediately preceding displayed row; Down, continued
typing, and Backspace also worked. Saving a disposable copy produced exactly
`<ul><li>first</li></ul><p><br>-</p><ul></ul>`. A second check appended a hyphen to
`<p>Last paragraph.</p><!--keep--><span data-keep="yes"></span>` successfully.
Splitting `Second paragraph.` immediately after its space preserved that space,
accepted further typing, and undid back to the original paragraph. Screenshots
confirmed caret movement and the absence of the previous error panel. The
isolated validation app was closed afterward.

Separate checks reproduced the reported two-Up behavior in Normal mode with
Physical Source Lines selected. Blank source lines between HTML paragraphs are
invisible in WYSIWYG, but remain destinations in this explicitly selected mode.
The status bar's file icon identifies Physical Source Lines; clicking it changes
to the eye icon and Visual Lines. Each Up then moves one displayed row. Insert
arrows also use displayed rows. This specified distinction was preserved.

Tests and computer use run with the isolated validation app and
`VIEM_CONFIG_DIR=/private/tmp/viem-quality-config`. Disposable fixtures are under
`/private/tmp/viem-html-eof-validation`; user documents are not edited.

## Regression coverage

`html_eof_editing` covers EOF insertion, source-byte preservation, encoding,
paragraph splits, undo/redo, and local work in a large HTML document.
`html_vertical_navigation` checks 64 terminal-edit cases and each displayed
Up/Down destination, plus the physical/visual navigation distinction.
`EVHTMLEOFEditingTests` covers native text input, list exit, hyphen insertion,
reopen, undo, and visible caret movement at ordinary and doubled zoom. Both new
native tests failed against the original release core before the fix; the red
log is `/private/tmp/viem-html-eof-native-red.log`.

Final validation passed: 1,465 Rust tests (one manual benchmark ignored),
299 XCTest tests, and 26 Swift Testing tests. The native geometry regression
uses the editor's existing affinity fallback when EOF has only an upstream
shaping caret. The release build and signing succeeded, and
`git diff --check` is clean.

Final logs:

- `/private/tmp/viem-html-eof-final-core.log`
- `/private/tmp/viem-html-eof-final-native2.log`
- `/private/tmp/viem-html-eof-final-build.log`

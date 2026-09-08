# Markdown Source paragraphs and accent cancellation

## Changes

Markdown Source preserves literal syntax and ordinary source-line breaks, but
maps blank source separator lines to styled paragraph boundaries. Soft source
continuations share a paragraph; Flow Source Paragraphs controls whether their
internal lines flow together. Code-block whitespace stays literal. Source bytes
outside declared edits remain exact.

Enter on an empty source list marker writes the additional physical separator
needed to keep later prose outside the preceding list. Empty ordinary source
line Enter also distinguishes completing a separator from authoring an additional
empty paragraph. Physical-source navigation continues to use original source
line coordinates despite the normalized display.

Native Escape, Ctrl-[, `doCommand(cancelOperation:)`, and direct AppKit
`cancelOperation(_:)` use one cancellation path. This discards native candidate
state even when there is no marked range, as can occur with the press-and-hold
accent picker. Active marked text is cancelled without committing or deleting
the original base letter.

## Validation evidence

The original list bug was reproduced through computer use in the isolated editor:
after deleting an empty `3. ` with Enter, typed prose remained indented as part of
the previous numbered item. The fixed build was then checked with the same
sequence: new prose uses the ordinary paragraph margin in both Source and
WYSIWYG. Saving a disposable result confirmed the required blank source line.
The source fixture with normal paragraphs, a continuation line, a list, and
fenced code showed styled paragraph gaps and retained the code's internal blank
row. Toggling Flow Source Paragraphs joined only the ordinary continuation.
Disposable UTF-8 fixtures live under `/private/tmp/viem-source-ime-validation`.

The original IME code failed three new native regressions. Direct
`cancelOperation` raised an unrecognized-selector exception, and Escape without
a marked range did not discard the input context. After the fix, all 25 focused
text-input and command-prompt tests passed, including source/revision preservation,
mode handling, replacement of the original `o`, undo, and redo.

Computer use verified native dead-key composition with Escape and Ctrl-[, and
confirmed that ordinary Escape preserves an already typed base letter. The
isolated app was closed after undoing the temporary input edits. The exact
press-and-hold popup cannot be automated by the available computer-use API,
which exposes key presses but no held-key gesture; that cancellation path is
covered by native responder/input-context tests.

Final full-suite validation passed: 1,459 Rust tests (one manual benchmark
ignored), 297 XCTest tests, and 26 Swift Testing tests. The release app build
and signing also succeeded. `git diff --check` is clean.

Relevant tests include `markdown_source_paragraphs`, `markdown_source_navigation`,
`markdown_source_layout`, `markdown_source_transfers`, and
`EVTextInputClientTests`. The source regressions cover original separator bytes,
UTF-8/Latin-1/UTF-16, list exit, source and display navigation, copy/move/sort,
undo, paragraph spacing, cache invalidation, and bounded large-document work.

Final logs are `/private/tmp/viem-source-ime-final-core.log`,
`/private/tmp/viem-source-ime-final-native.log`, and
`/private/tmp/viem-source-ime-final-build.log`. The targeted IME red/green logs
are under `/private/tmp/viem-ime-escape-*`.

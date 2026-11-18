# Bugs

Reviewed November 15, 2026 against current code, focused Rust regressions and
the three macOS test classes named in the previous audit.

## Open issues

- **Code-block language selection.** Native menu selection of None fails in
  `EVCodeBlockLanguageTests.testNativeCodeLanguageMenuEscapeAndNoneSelectionPreserveCaretAndUndo`:
  the language remains Rust, the caret and mode change, and undo does not
  restore the expected source.
- **Uppercase `</PRE>` end tag.** pulldown-cmark ends a `<pre>` HTML block
  only at a lowercase `</pre>`; CommonMark and GitHub match it
  case-insensitively. `<PRE>x</PRE>` followed by a blank line and `## Next`
  keeps the heading inside the HTML block as literal text.

## Open questions

### Q11. Leaving a trailing table

Code-block exit is implemented: Enter after two trailing blank lines creates
a following paragraph (one blank line for front matter). What gesture should
create the paragraph after a trailing table? Repeated Enter and `o` still add
in-cell hard breaks.

### Q13. Composition in Normal mode

IME composition is the temporary marked text used by input methods such as
Japanese conversion. `Begin` captures a replacement range, `Update` changes
the overlay, and `Commit` writes its text as one edit. The core validates the
target's document, revision and Unicode boundaries, but does not consistently
validate that its mode and active selection permit that replacement.

Two direct coordinator reproductions still succeed:

- Open `abc` in Normal mode at offset 0; begin composition with target `0..0`,
  update to `か`, then commit. The source becomes `かabc` without entering
  Insert mode. This bypasses the modal input contract.
- Open `abc`, enter Insert, select all with a native selection (caret at 3),
  then begin with target `3..3` instead of the selected `0..3`. Commit `か`.
  The source becomes `abcか`, retaining the text the user expected to replace.

The macOS input adapter admits document composition only in Insert, Replace,
Select and native-selection modes. Without an explicit IME replacement range,
it supplies the active selection or caret as the target, so ordinary macOS
input avoids these reproductions. Direct core/ABI callers can still reach
them; the protection belongs in the core as well as its adapters.

**Recommendation:** reject document composition in Normal and Vim Visual
modes. Admit it in Insert/Replace and editable Select/native selections.
When a selection is active, require the target to equal its exact semantic
replacement extent; reject a conflicting target rather than silently
inserting elsewhere. With no selection, keep explicit, current IME replacement
ranges for reconversion, including nonempty ranges. Command-line composition
remains a separate input path. Validate before opening a session, retiring
completion or closing edit groups, and revalidate relevant state at commit.
Regression tests should cover both reproductions, accepted reconversion and
selection replacement, stale targets, cancellation and unchanged history,
registers, source and selection on refusal. This remains a policy proposal;
no composition behavior has been changed yet.

### Q16. Footnotes

Footnotes remain unsupported. Should definitions be treated as opaque until
support is added?

### Q17. Literal CRs in mixed-ending files

A file with mostly LF and some CRLF lines opens as `unix`, following Vim's
`fileformats=unix,dos`. The CRs become literal content: Preview shows `ab\r
c`, the caret can sit between CR and LF, and Backspace at a line start can
delete an invisible CR instead of joining lines. This matches Vim's `^M`
behavior. Should Preview show or protect these CRs, for example with a visible
marker or by refusing to put the caret between CR and LF?

### Q19. List continuation in Plain Text

In Plain Text, Enter at the end of `- item`, `* item` or `1. one` continues
the list (`- `, `* `, `2. `), and Enter on the empty continued item ends it.
`o` doesn't continue. AGENTS.md describes list continuation for Markdown and
says Code never runs it, but doesn't mention Plain Text. Should Plain Text
continue lists, and if so, should `o` match Enter?

### Q22. Joining soft-wrapped rows

Under Visual line meaning, `J`, `gJ` and Visual `J` on rows within one hard
line have no separator to remove. Should these remain no-ops, report a
message, or advance to the next hard line?

### Q23. Pasting body text into a table header

A nonrectangular selection spanning bold header text and ordinary body text
carries their resolved appearances. Pasting it into a body cell works. In a
header, preserving the body text's non-bold appearance would require
overriding the header's bold default, which Markdown's editable inline
vocabulary cannot express. Should copied paragraph defaults inherit from the
destination header, or should a paste that cannot preserve its resolved
appearance be refused? The original reproduction is `| **bold** | _it_ |\n|
--- | --- |\n| a | b |`, copying logical range 0..9 and pasting at 0. This
paste currently returns an unsupported-formatting result atomically.

### Q25. Shifting a wrapped row in Text

Under Visual line meaning, `>>`, `<<` and `V>` on the second or later display
row of a soft-wrapped line insert indentation in the middle of the sentence
(`jumps   over the lazy dog`) in Text. Should Text row-based shifts apply to
the row's hard line, or be refused on continuation rows? Preview is resolved:
shifts use the indent/unindent buttons' list operations and do nothing where
those operations are unavailable.

### Q26. `yyp` on a partial row

This concerns Text and Markdown Preview with Visual line meaning, when one
hard line wraps onto several display rows. `yy` correctly copies the current
display row, but an interior row has no actual line ending. Its register is
therefore characterwise. `p` follows that register kind and inserts immediately
after the caret's character, rather than after the row that was copied.

For example, suppose a narrow view wraps one source line like this (the shown
breaks are soft wraps, not source newlines):

```text
The quick brown
fox jumps over
the lazy dog
```

With the caret on the `f` in the second row, `yy` captures `fox jumps over `
(including its trailing space). The caret stays on `f`; `p` inserts the captured
text after that character. The source becomes:

```text
The quick brown ffox jumps over ox jumps over the lazy dog
```

The copied text is intact and undo restores the original source, but the put
splits the original word `fox`. The mismatch is between the user's row-based
yank and the characterwise placement of the subsequent put. It does not arise
for an ordinary complete hard-line yank, whose linewise register already puts
after the line. Code always uses Physical Source line meaning.

**Recommendation:** retain that a register was yanked as a complete display
row, separately from the text and its real hard breaks. Use that intent to put
after the current display row (`p`) or before it (`P`), preserving the copied
text and styles without inventing a source newline. Simply making the register
linewise would add a hard line or paragraph break that the yank never captured.
This would also need defined behavior for counts, partial first/last hard rows,
rich Markdown fragments, clipboard round trips and a view whose wrapping or
font changed between yank and put. Placement should use current, validated
layout; undo/redo should replay the resolved edit without depending on later
wrapping. This is a proposed behavior change only; Q26 is still open.

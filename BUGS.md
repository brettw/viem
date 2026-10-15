# Bugs

## Open issues

- **Caret hover and plain clicks.** The plain-click comparison in
  `EVCaretHoverTests` fails.
- **Code-block language selection.** The demo None-to-C selection in
  `EVCodeBlockLanguageTests` fails.
- **Code-style persistence.** Rename/deletion persistence fails in
  `EVCodeStylesTests`.

## Open questions

### Q4. Autoformatting in an existing list item

Typing `- ` at the start of `- a` autoformats to a nested bullet (`- - a`).
Typing `1. ` writes `- 1.&#32;a` instead. Should ordered markers nest the same
way, or should neither autoformat inside an existing item's first line?

### Q5. Line commands on table rows

What should `J`, `gJ`, `2dd`, `gUU`, `g~~`, `>>`, `:sort` and `:g` do in
Markdown Preview tables? They currently fail with a mix of "Editing across table cell
boundaries…", `TextDoesNotMatchLayout`, `AmbiguousProjection` and
`UnsupportedFormatting`. Possible rules: act on the current cell's text, act
on whole rows, or refuse with one consistent message.

### Q6. Boundary Backspace and Delete in cells

These are no-ops by design. Should they at least move the caret to the
previous or next cell, as Word does, so the key doesn't feel dead?

### Q9. Formatting availability vs. refusal

Increase/Decrease Indent outside lists, Heading/Code Block on thematic breaks
and HTML blocks, and Bold on selections the core can't wrap are presented as
available and then rejected. Should `selection_semantic_style_presentation`
and the paragraph-style presentation run the same capability check as the
edit, or should these always succeed through a fallback spelling (for example
`<strong>`)?

### Q11. Leaving a trailing code block or table

What gesture should create the paragraph after the block: Down at the last
line, Enter on an empty last line, or a triple Enter?

### Q13. Composition in Normal mode

The core accepts `CompositionEvent::Begin` and commits marked text while in
Normal mode. The fuzzer committed `か` without entering Insert. The frontends
never send this, so should the core refuse it? The core also accepts a
composition whose target is the caret while a native selection is active, and
then inserts at the caret without replacing the selection. The macOS frontend
always passes the selection as the target, so this only matters for other
callers.

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

### Q25. Shifting a wrapped row

Under Visual line meaning, `>>`, `<<` and `V>` on the second or later display
row of a soft-wrapped line insert indentation in the middle of the sentence
(`jumps   over the lazy dog`), in Plain and Markdown Preview. Should row-based shifts
apply to the row's hard line, or be refused on continuation rows?

### Q26. `yyp` on a partial row

`yyp` on an interior display row puts the row's text characterwise after the
cursor, splitting a word. Should a row yank remember that it was a whole row
and put it after the row's end?

### Q28. Paragraphs in Markdown views

What should count as a paragraph for `}`, `{`, `ap` and `ip` in Markdown
Preview: each block (heading, list item, table, code block), or runs of
ordinary paragraphs as separated in the source? Vim users will expect `dap` on
a list item to delete that item, or the whole list.

### Q29. Deleting a gutter row selection

Should Backspace on a gutter-selected heading row remove the row's paragraph
(so `# T\n\nend` becomes `end`), as a linewise delete does, rather than
joining the next paragraph into the heading? On a gutter-selected table row,
should Delete clear the cells (the table spec's selection rule) or delete the
row?

### Q30. Reindent around braces and block comments

`==` copies the preceding nonblank line's indentation, so a closing `}` keeps
the body's indentation and a line after ` */` gets one space. Keep the literal
rule, or skip comment-continuation lines and dedent closing brackets?

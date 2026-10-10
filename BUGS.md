# Markdown parser bugs

No unresolved numbered issues.

The October 9, 2026 audits at commits `cc0d454` and `93a7c52` produced
reports 32–59. All 28 reports were independently reproduced and resolved,
with focused regression coverage for opening, editing, source preservation,
and undo where applicable.

Missing GitHub features and deliberate presentation differences are tracked
in [MARKDOWN_GAPS.md](MARKDOWN_GAPS.md).

## How the audit was run

- **Reference corpus:** 927 examples, compared with their expected HTML. They
  are the CommonMark spec, the GFM table, strikethrough and task-list examples,
  and pulldown-cmark's regression suite, taken from the pulldown-cmark 0.13.4
  test sources.
- **Mutation fuzzing:** every example with one character deleted, or one
  Markdown token inserted, at every position. That is about 2.4 million
  sources, each opened as Markdown and as Markdown Source.
- **Edit fuzzing:** at every boundary of every example, insert `x`, a space,
  `*` or a line ending, or delete one grapheme. Source view also inserted
  `` ` ``, `>`, `- `, `#`, `|` and `<`. After each committed edit, the
  projection was compared with a fresh `Document::from_bytes` of
  `source_bytes()`.

After each committed edit, compare the projection's text, block attributes
and resolved character styles with a fresh parse of `source_bytes()`.
Compare semantic results rather than document identities or equivalent
style-span partitions. Check exact source bytes through undo/redo and the
locality of supporting patches too.

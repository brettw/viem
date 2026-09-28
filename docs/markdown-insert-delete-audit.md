# Markdown insertion and deletion audit

> Historical audit: references to HTML/RTF editing and general format conversion
> describe retired implementations. Current editable formats are Text, Code,
> and Markdown; only Markdown Source/WYSIWYG switching remains.


This pass follows the source-mode bulleted-list consistency error and the
list/code toolbar and numbering fixes. It checks command execution, saved-source
reopening, and exact undo/redo, including failures previously tolerated by tests.

## Repairs

| Affected edit | Repair |
| --- | --- |
| Enter at visible quote/list prefixes or after quoted continuations | Split source syntax at the caret; continue from the parsed item owner, including across quote-only blank rows. |
| Split the final source line | Keep its stable identity while assigning its newly parsed hard-break metadata. The next keystroke must see the new line. |
| Source Enter and line opening whose source separators reparse differently | Derive the effective formatted change from the authored syntax and retain the correct typing caret. Candidate verification remains enabled. |
| Edits next to nested fences or quote-only separators | Include their dependent context in the regional parse or use a complete parse. Ordinary large-document typing and Enter retain the bounded regional path. |
| Open a line within quoted code | Preserve the quote prefix and code line ending. |
| WYSIWYG Enter, open-line and multiline insertion in quoted lists or continuation paragraphs | Use the containing item's marker and preserve quote prefixes, empty paragraphs, folded spaces and inline hard breaks. |
| Type spaces at list-body boundaries or beside a Markdown hard break | Protect authored spaces with character references where raw spaces would become syntax. |
| Remove list treatment from a wrapped item | Remove its hidden continuation indentation as well as its label. |
| Join an empty bare list item to its successor | Supply marker padding so the following text remains list content. |
| Join ordinary prose to a multi-line quote | Remove the consumed quote prefixes throughout the joined paragraph. |
| Open paragraphs beside a code fence | Supply the required separator endings; preserve list ownership and the configured following paragraph style. |
| Type punctuation at EOF inside inline code | Insert literal code text inside the closing delimiter. |
| Backspace to unindent an item before remaining nested siblings | Adjust the following indentation when the lifted item's marker is wider than the former parent's marker. |

All supporting source changes are part of the initiating transaction. Source
encoding, line-ending spelling, exact source undo/redo and unrelated content
remain covered by the existing checks.

## Coverage

`tests/all/markdown_edit_audit.rs` adds:

- 6,530 source-mode caret/action cases across 34 fixtures;
- 24,825 source-mode range deletion/replacement cases;
- 4,130 formatted Markdown caret/action cases;
- end-of-list Enter followed by typing across UTF-8, Latin-1, UTF-16LE/BE,
  LF/CRLF/CR, and visual/physical-source line modes.

Actions include ordinary text, Markdown punctuation, spaces, multiline input,
Enter, Backspace, Delete, `o`, and `O`. Reopening checks text, paragraph metadata,
inline styles and hard breaks. Each source-mode command is also followed by
ordinary typing to check the resulting caret and line metadata. The source typing differential tests now reject
all Enter errors rather than silently accepting some refusals. The full suite
also includes the existing HTML/RTF replacement, native selection, structural
caret deletion, Unicode and large-document work-limit audits.

Optional reports can be written by setting `VIEM_EDIT_AUDIT_REPORT_DIR` to an
existing directory. Run the new matrix with:

```sh
cargo test --test all markdown_edit_audit::
```

Initial audit validation on September 26, 2026:

- Rust library tests: 1,549 passed, 3 ignored.
- Rust integration tests: 1,519 passed, 5 ignored, including the 35,485 matrix
  cases above. One then-ignored test was the indented-code policy reproducer,
  now enabled by the follow-up below; the other four were already ignored.
- Native editing reliability and formatting toolbar tests: 26 passed.
- `make debug` rebuilt and signed `.build/Viem.app` successfully.
- `git diff --check` passed.

## Indented-code follow-up

The user selected GitHub-compatible interpretation. Four-column/tab-indented
Markdown now projects as code. The previously ignored reproducer is enabled,
and the ordinary WYSIWYG matrix includes the indented fixture. Additional
regressions cover code within quotes and lists, tabs, blank lines, toolbar
removal, and source preservation. An edit that requires leading/trailing blank
code lines or an empty code body can convert the affected block to fences in
the same undo transaction.

The earlier counts above record the first audit pass. Current fixture counts
are printed by the tests; code indentation no longer contributes visible prose
characters. Remaining rendering differences from GitHub are prioritized in
[MARKDOWN_GAPS.md](../MARKDOWN_GAPS.md).

Follow-up validation passed 1,549 library tests, 1,524 integration tests and
27 native editing/toolbar tests. The remaining 3 library and 4 integration
ignores predate this work. The debug app was rebuilt and signed successfully.

The pre-existing format/encoding limits described in
[the editing policy audit](wysiwyg-editing-audit.md#input-recovery-policies)
remain explicit errors: for example, emoji cannot be inserted losslessly into
Latin-1 source text without changing encoding. Stale revisions, invalid grapheme
boundaries and inconsistent structured payloads also remain rejected. This
fixture-based audit does not establish that every possible document is error-free.

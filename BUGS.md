# Markdown parser bugs

Found in an audit on October 9, 2026, at commit `cc0d454`. Each entry gives a
minimal source, the action and the result. Sources use Rust string escapes
(`\n` is a line ending, `\\` one backslash). Line references are to that
commit; the comparison audit after documentation commit `93a7c52` adds
entries 49–59 and further reproductions below without changing parser code.
Missing GitHub features and deliberate presentation differences are
tracked in [MARKDOWN_GAPS.md](MARKDOWN_GAPS.md), not here.

**(keys)** marks bugs reproduced by sending key events through `Core`, which is
what a user sees. **(model API)** marks bugs reproduced only with
`ModelRequest::ApplyTextEdits`. Typing sometimes takes a different path, so a
model-API failure is not always reachable from the keyboard. It still breaks
the contract for native actions that issue the same request.

**(document API)** marks edits reproduced with public `Document::replace`,
without establishing their keyboard entry path.

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

Results:

- **WYSIWYG (99,253 edits):** 3,236 projections differed from a fresh parse.
  829 of those showed different visible text, and 1,785 differed only in
  Markdown reference styling. Separately, 11,185 edits failed with
  `VerificationFailed` and 79 with `AmbiguousProjection`.
- **Source view (297,883 edits):** 134 panics, 1,877 divergences from a fresh
  parse and 10 `VerificationFailed`.
- **Other checks:** opening never panicked except in bug 2. LF, CRLF and CR
  copies of every example projected identically. The adversarial lines
  exercised in that run, up to 400 KB, projected in linear time; the URL
  family in bug 49 instead takes quadratic time.

The comparison audit also opened 672 available GFM examples and 6,000
generated sources in both views, then checked targeted public document and
Core-key reproductions. All 352 existing Markdown-related tests passed;
these findings require additional regression cases.

The cheapest regression guard is the same comparison. After each committed
edit in `tests/all/fuzz_markdown_edits.rs`, assert that the projection's
text, blocks and style spans equal those of a fresh parse of
`source_bytes()`.

## Crashes

1. **Deleting a table pipe in Source view panics (keys).** Open
   `"| a  |\n| -- |\n"` as Markdown Source, put the cursor on the final `|` of
   the first row and press `x`. It panics at the
   `.expect("private projectors emit a valid current-adapter hard-line partition")`
   in `FormattedDocument::from_parts`
   ([projection.rs:1791](src/core/document/projection.rs#L1791)). The path is
   `Document::build_table_row_candidate` then
   `FormattedDocument::extract_table_row`. The FFI boundary returns
   `ViemStatus::Panic` and the edit is lost. All 134 Source-view fuzz panics
   took this path, including deleting the closing pipe of
   `"| f\\|oo  |\n| ------ |\n..."`. The candidate builder should reject the
   invalid local candidate and fall back to structural reprojection of the
   ordinary Source edit instead of panicking.
2. **Opening a table with a literal CR in Source view underflows.** Open
   `"| a |\r\n| - |\n"` or `"| abc | def |\n| --- | -\r-- |\n"` as Markdown
   Source. These have mixed endings, so `unix` detection makes `\r` content.
   `next_start - range.end` in `install_flow_ranges`
   ([projection.rs:1839](src/core/document/projection.rs#L1839)) subtracts
   with overflow. Release builds wrap and record a huge `separator_length`.
   The comparison audit also reproduced the panic with
   `"|a|\r|-|\na\r# b"`. The same line-ending disagreement hides unrelated
   text: default-Unix `"a\r# b"` displays only `b` in WYSIWYG, while
   `"|a|\r|-|"` changes Source projection text to `"|a|\n|-|"`.
   [`grammar_text`](src/core/document/markdown_syntax.rs#L283) masks literal
   LF under Mac policy but leaves literal CR exposed to grammar line parsing.

## Edits that commit but reopen differently

After an edit, the regional reprojection
([`splice_line_local_projection`](src/core/document/projection.rs#L4303))
re-parses only the touched lines. Sometimes an edit changes how a
neighbouring line parses. The candidate projection is then wrong, but
verification compares the request with it and accepts. The committed
document no longer matches a fresh parse of its own bytes, so the screen
shows one thing and save-and-reopen shows another. This breaks the
semantic-correctness requirement in AGENTS.md.

### Reproduced with keys

3. **A setext underline on the following line is ignored.**
   - In `"\n====\n"`, type `x` on the empty first line. The screen shows
     paragraphs `x` and `====`; reopening gives an H1 `x`.
   - (model API) In `"# Heading\n    foo\nHeading\n------\n    foo\n----\n"`,
     type `x` before the last `    foo`. On reopen `----` becomes a setext
     underline.
   - (model API) In Source view, ```` "> ```\n> aaa\n\nbbb\n" ```` with
     `- ` typed on a new final line shows a list item. Reopening gives an
     H2 `bbb`.
4. **Fenced code with an indented opener loses typed indentation.** In ```` " ```\n aaa\naaa\n```\n" ````, type a space before the second `aaa`.
   The screen shows ` aaa`. The file gets ` aaa`, which the opener's
   one-space indent strips, so reopening shows `aaa`. (model API) Two- and
   three-space openers behave the same, for example ```` "  ```\naaa\n  aaa\naaa\n  ```\n" ````. The regional parse does not
   see the opener's indentation.
5. **An edit can turn a code line into a fence closer.**
   - In ```` "```\naaa\n    ```\n" ````, delete one space before the
     indented ```` ``` ````. The line becomes a valid closer. The screen
     still shows it as code, and reopening shows only `aaa`.
   - (model API) `" ~~~\n    ~~~\n"` with its first code space deleted
     behaves the same.
6. **Typing into an empty item with a bare marker destroys the marker.** In
   `"1.\n  Text after.\n"` or `"-\n  foo\n"`, type `x` in the empty item. The
   file gets `1.x` or `-x` with no space, which is not a list marker.
   Reopening shows `1.x Text after.` as a plain paragraph. (model API)
   `"*\n      <div>\n     <div>\n"` behaves the same.

   - (document API) `"-\nparagraph"` with `replace(0..0, "X")` commits
     live text `"X paragraph"` but saves `"-X\nparagraph"`, which reopens
     as `"-X paragraph"`.
   - (document API) `"text\n1.\nnext"` with `replace(5..5, "X")` commits
     `"text\nX next"` but saves `"text\n1.X\nnext"`, which reopens as
     `"text 1.X next"`.
   The structured local candidate path
   ([transaction.rs:4777](src/core/document/transaction.rs#L4777)) can
   inherit the existing incorrect list context; verification must establish
   the meaning of the patched source, including the marker boundary.
7. **Typing into an empty ATX heading at end of file writes after it.**
   - In `"## \n#\n### ###\n"`, type `x` in the last heading. The file
     becomes `"...### ###\nx"`. Reopening shows the empty heading followed
     by a paragraph `x`.
   - (model API) `"### ###\n"` and `"### \n"` alone behave the same.
   - `"### \n\nNext\n"` works.
8. **Typing into a blank separator joins it to the following paragraph.**
   - In ```` "> ```\n> aaa\n\nbbb\n" ````, type `x` on the blank line. The
     file becomes `"...> aaa\nx\nbbb\n"`, which reopens as one paragraph
     `x bbb`.
   - (model API) Typing on the first line of `">\n> foo\n>  \n"` reopens as
     `x foo`.
   - (model API) Typing on the empty first line of `"\n<foo>\n"` (or
     `"\n <foo>\n"`, `"\n   <foo>\n"`) does the same. `<foo>` stops being an HTML block and
     joins: `x <foo>`.
9. **Deleting a line ending after a table in Source view is not shown.**
   - In `"| a | b |\n| - | - |\n| c | d |\n> q\n"`, press Backspace at the
     start of `> q`. The file now contains `| c | d |> q`, but the screen
     still shows the row and `> q` on separate lines.
   - (model API) Deleting the final line ending after a table at end of
     file also leaves the line ending on screen.

### Reproduced with the model API

10. **Reference definitions and their dependents go stale.**
    - **Lost styling:** typing next to a reference link drops its Markdown
      reference styling until reopen, for example inserting `x` before or
      after `[foo]` in `"[foo]\n\n[foo]: /bar\n"`.
    - **Broken definition:** inserting `x` at the start of a definition (so
      it reads `x[ref]: /uri`) stops it being a definition. Its dependents
      keep their reference presentation.
      ``[link *foo **bar** `#`*][ref]`` still shows literal syntax instead
      of formatting, and `![foo][bar]` stays an image object instead of
      literal text.
    - **Multi-line definitions:** `"[foo]:\n/url\n'title'\n"` and
      `"[Foo\n  bar]: /url\n"` flow differently after any edit than on open.

    MARKDOWN_GAPS.md says reference-sensitive edits are meant to use a
    complete parse; these cases show that some do not.
11. **Breaking an HTML comment terminator does not reproject the rest of the
    file.**
    - In `"<!-- <dl> -->\n- **foo** (u8, u8)\n"`, delete a `-` from `-->`. On reopen, the following lines are HTML-block
      content and `- **foo**` is literal. The screen keeps the formatted
      list.
    - `"- foo\n- bar\n\n<!-- -->\n\n- baz\n- bim\n"` behaves the same.
12. **Editing literal unknown-tag text can create a real tag.** In
    `"* <foo>\n\t<bar>\n"`, delete the `a` of the literal `<bar>`. The file
    then contains `<br>`, which reopens as a hard break, while the screen
    shows literal `<br>` text. Typed changes inside literal HTML should be
    escaped or rejected.
13. **Indentation edits change block structure on reopen.**
    - In `" -    one\n\n     two\n"`, type a space before the indented code
      ` two`. On reopen `two` becomes a paragraph of the list item.
    - In `"- # Foo\n- Bar\n"`, type a space before `Foo`. The ATX heading
      strips it on reopen, so the typed space is lost. A quote inside a
      list item (`"* > foo"`) and `"- _t\n  # test\n  t_\n"` behave the
      same.
14. **Emptying a paragraph leaves a phantom.** In
    ``"`Foo\n----\n`\n\n<a title=\"a lot\n---\nof dashes\"/>\n"``, delete
    the lone `` ` `` paragraph. The screen keeps an empty paragraph that
    disappears on reopen.
15. **Code styling goes stale.**
    - Typing `*` at the start of an indented list continuation paragraph
      styles the paragraph as Code until reopen. Examples:
      `"  - foo\n\n    bar\n"`, `"  - foo\n\n\tbar\n"` and
      `"   > > 1.  one\n>>\n>>     two\n"`.
    - Typing at the start of a line inside a fenced code block or
      `<pre><code>` leaves the new character outside the Code span until
      reopen. For example, type at the start of `  return 3` in ```` "```ruby\ndef foo(x)\n  return 3\nend\n```\n" ````.
16. **Table cell whitespace and lone pipes diverge.**
    - In `"| Col 1 | Col 2 |\n|-------|-------|\n"`, delete the `1`. The
      screen shows `Col ` with a trailing space; reopening trims it to
      `Col`.
    - Typing before the lone `|` row in
      `"| Table | Header |\n|-------|--------|\n|\n"` reopens with a
      different cell structure.
17. **Source-view table structure changes are not reprojected.**
    - In `"Hello World\n| abc | def |\n| --- | --- |\n| bar | baz |\n"`,
      delete the leading `|` of the header row. The screen keeps the table;
      reopening shows a paragraph.
    - In `"foo|bar  \n---|---\nfoo|bar\n"`, delete a trailing space after
      `bar`. The header and delimiter rows merge into one block.

## Ordinary edits that are rejected

### Reproduced with keys

18. **Enter fails inside any multi-line fenced code block.** In ```` "```\nab\ncd\n```\n" ```` or ```` "```ruby\ndef foo(x)\n  return 3\nend\n```\n" ````, Enter in the
    body fails with `FormattedPayloadCannotReproject`. The model API
    reports `AmbiguousProjection` or `VerificationFailed`. Single-line
    blocks work.
19. **Typing after combined emphasis, strikethrough and strong fails.**
    `"*~~__a__~~* b\n"` with typing after `a` fails. (model API) So does
    typing at the end of the inner runs of
    `"*~~__emphasis strike strong__~~* ~~*__strike emphasis strong__*~~\n"`
    and of its variants that use code spans.
20. **Typing next to an unmatched delimiter fails.** In `"**foo*\n"`, shown
    as `*foo`, typing at the start fails. Through the model API it also
    fails after the literal `*` of `"*foo**"`, and in `"__foo_"`,
    `"____foo_"`, `"_foo____"`, `"****foo*bar*baz****"` and
    `"**a.*.**a*.**."`.
21. **Typing on a blank line inside an indented code block fails.** Example:
    `"    a\n\n\n    b\n"`. (model API) Likewise
    `"    chunk1\n\n    chunk2\n  \n \n \n    chunk3\n"`.
22. **Enter at the start of some headings fails.** This applies to ATX
    headings with a closing sequence (`"## foo ##\n"`) and to setext
    headings (`"Foo *bar*\n=========\n"`).

### Reproduced with the model API

These are examples from the 11,185 failures. Some are structurally
ambiguous and may be acceptable rejections. Even then, they should report
a user-facing reason rather than `VerificationFailed`.

23. **Joining some adjacent ATX headings fails.** Deleting the break
    between headings with closing sequences fails, as in
    `"## foo ##\n  ###   bar    ###\n"` and
    `"# foo ##################################\n##### foo ##\n"`. So does
    joining indented headings, as in `" ### foo\n  ## foo\n   # foo\n"`.
    Joining `"# a\n# b\n"` works.
24. **Enter fails in lists with tab-indented content.** Examples: at the
    start of `foo` in `"- foo\n\n\t\tbar\n"`, and at the start of `bar` in
    `" - foo\n   - bar\n\t - baz\n"`.
25. **Enter fails in and after indented code.** Examples: at the end of
    `chunk1` in `"    chunk1\n\n    chunk2\n"`, and at the start of `bar`
    in `"    foo\nbar\n"`.
26. **Typing at the start of a list with an empty item fails.** Typing
    into the empty item of `"-   \n  foo\n"` fails, as does typing at the
    start of `foo` in `"- foo\n-   \n- bar\n"`. The latter works through
    keys.
27. **Typing at the start of a multi-line code span fails.** Example: ``` "``\nfoo\n``\n" ```. The same span inside a sentence
    (``` "a ``\nfoo\n`` b\n" ```) accepts typing through keys.
28. **Typing after a backslash in a table cell fails.** Examples are cells
    holding `` `\|` `` or ending in `\`, as in pulldown-cmark's table
    tests.
29. **Enter fails at some positions in Source view.** Examples: at the end
    of `aaa` in `"  \n\naaa\n  \n\n# aaa\n\n  \n"`, and inside the
    indentation of `  bar` in `"- foo\n\n\n  bar\n"`.
30. **Typing before HTML blocks and comments, and inside reference
    definitions, fails.** Examples: typing at the start of
    `"<!-- foo -->*bar*\n"` or `"<table><tr><td>\n..."`, and typing just
    before the colon of `"[Foo bar]:\n<my url>\n'title'\n\n[Foo bar]\n"`.

    - (document API) In `"<ul><li>a</li><li>b</li></ul>"`, whose visible
      text is `"a\nb"`, `replace(0..3, "X")` returns `VerificationFailed`,
      and deleting `1..2` returns `AmbiguousProjection`. The HTML boundary
      repair handles paragraph/heading/div owners but not these list
      boundaries; an unsupported structural edit still needs an explicit
      format-policy error rather than failed verification.
    - (document API) In `"<div><!--x--></div>"`, inserting `X` at visible
      offset 6 or 7 returns `VerificationFailed`; the same insertions in
      standalone `"<!--x-->"` succeed. Breaking the comment closer changes
      the visibility of the enclosing syntax, which the candidate does not
      preserve.

## Projection mistakes on open

Most of these come from three hand-written line classifiers that run beside
pulldown-cmark and disagree with it:
[`markdown_blocks::classify`](src/core/document/markdown_blocks.rs),
[`markdown_quotes::classify`](src/core/document/markdown_quotes.rs) and the
soft-break detection in
[`paragraph_flow`](src/core/document/paragraph_flow.rs).

### Hidden or lost text

31. **A list item's first line vanishes before an indented block start.**
    - `"- item one\n  <div>block</div>\n"` shows only `block`.
    - `"- item one\n  ***\n- two\n"` shows only a rule and `two`.
    - `"1. Step one\n   ## Sub heading\n"` shows only the heading.
    - `"- _t\n  # test\n  t_\n"` and
      `"* A Heading:\n  # inside a list item\n"` lose their first lines.

    Soft-break detection checks the continuation with
    `markdown_block_prefix` on the raw line
    ([paragraph_flow.rs:54](src/core/document/paragraph_flow.rs#L54)),
    which ignores indented block starts. The lines are folded together, and
    the heading, HTML or rule branch then replaces the folded line.
32. **A trailing backslash at the end of a block is hidden and becomes a
    line break.** Examples:
    - `"foo\\\n"` at end of file, `"### foo\\\n"` and
      `"# foo \\\nbar \\\n"`.
    - `"Foo\\\n----\n"` and `"a\\\n==\n"`.
    - `"- a\\\n- b\n"`, `"a\\\n* b\n"`, `"a\\\n> b\n"` and
      `"a\\\n# b\n"`.

    GitHub shows the backslash.
    [paragraph_flow.rs:199-228](src/core/document/paragraph_flow.rs#L199)
    treats a trailing `\` or two spaces as a hard break even when the line
    ending is a block boundary.
33. **An escaped backslash at the end of a line becomes a hard break.**
    `"path C:\\\\\nnext\n"`, whose source line ends in `\\`, shows `C:\\`
    and a line break. GitHub shows `C:\ next`. Neither the hard-break code
    nor the soft-break check
    ([paragraph_flow.rs:84](src/core/document/paragraph_flow.rs#L84))
    checks whether the backslash is itself escaped.
34. **The last line of an indented code block loses trailing spaces or a
    backslash.** `"    foo  \n"`, `"    a\\\n"` and
    `"para\n\n    code\\\n"` drop them and gain an extra empty paragraph.
    The final line ending is outside the code's `body_end`, so the
    hard-break branch above applies to it.
35. **Empty list items interrupt paragraphs, hiding the marker.** An empty
    list item cannot interrupt a paragraph, so these are plain text on
    GitHub: `"foo\n*\n\nfoo\n1.\n"`, `"*foo bar\n*\n"`, `";\n*\n%\n"` and
    `";\n* \n%\n"`.
    [markdown_blocks.rs:249](src/core/document/markdown_blocks.rs#L249)
    only applies the ordered-list start rule.
36. **Over-indented markers become list items.**
    - In `"- a\n - b\n  - c\n   - d\n    - e\n"`, `- e` is continuation
      text.
    - In `"1. a\n\n  2. b\n\n    3. c\n"`, `3. c` is indented code.
    - In `"-\n\n\t-\n\n\n- x\n\n\t-\n"`, the first `\t-` is indented code.

    [markdown_blocks.rs:256](src/core/document/markdown_blocks.rs#L256)
    allows a marker up to `content_indent + 3` columns.
37. **A paragraph after an empty item is pulled into the item.**
    `"1.\n  Text after.\n"` and `"-\n  foo\n"` show the paragraph as the
    item's text, with a leading space. GitHub shows an empty item followed
    by a paragraph.
38. **A lazy `===` line in a quote moves out of the quote.** For
    `"> foo\nbar\n===\n"`, GitHub shows one quoted paragraph,
    `foo bar ===`. Viem moves `===` outside the quote, because
    [markdown_quotes.rs:193](src/core/document/markdown_quotes.rs#L193)
    treats `=` lines as non-prose. Edits on that line also diverge from a
    fresh parse.

### Wrong structure

39. **An unclosed fence in a list item continues past the item.** ```` "- ```\n  code\n- next\n" ```` shows `- next` as literal text in
    the first item. The list classifier keeps its fence state until a
    closing fence, ignoring the end of the item
   ([markdown_blocks.rs:168](src/core/document/markdown_blocks.rs#L168)).
   In ```` "- ```\na\n```\ntail" ````, unindented `a` and `tail` also
   acquire `ListItem` kinds after the empty item's fence has ended.
40. **A fence-like line inside an HTML block corrupts later lists.** ```` "<div>\n```\n</div>\n\n- a\n- b\n" ```` shows `- a` and `- b` as
    literal paragraphs. Both classifiers open a fence on any ```` ``` ```` line
    ([markdown_blocks.rs:235](src/core/document/markdown_blocks.rs#L235),
    [markdown_quotes.rs:177](src/core/document/markdown_quotes.rs#L177)).
    A related case is the closing fence of ```` "- ```\n  code\n  ```\n\n> quote\n" ````.
    `markdown_quotes::classify` never saw the opener after the list marker,
    so it treats the closer as a new fence. Later quote lines then get depth
    0 in the quote context used by editing. Rendering is still correct
    because the parser's containers take over.
41. **A single-dash setext heading becomes a list.** For
    `"Foo\n-\nbar\n"`, GitHub shows an H2 `Foo` and a paragraph `bar`.
    Viem shows a paragraph, an empty bullet, and `bar` as list
    continuation. The list-context filter on headings at
    [projection.rs:5700-5705](src/core/document/projection.rs#L5700)
    suppresses the heading.
    (document API) In `"text\n-\nnext"`, `replace(5..5, "X")` commits
    live `"text\nX\nnext"`, but saved `"text\n- X\nnext"` reopens as
    `"text\nX next"`.
42. **Several container markers on one line are not nested.**
    `"- - foo\n"`, `"1. - 2. foo\n"`, `"- *foo\n  - - \n  baz*\n"` and
    `" - >*\n"` show literal `- foo`, `- 2. foo`, `- ` and `*` inside one
    item. The list classifier recognizes one marker per line.
43. **A list in a quote in a list gets the outer level.** For
    `"- a\n  > - b\n  >   c\n"`, Viem gives `b c` the same label level as
    `a` (level 0).

### Inline

44. **Inline syntax cannot cross a hard line break.** Examples:
    `"**bold across  \nbreak**"`, `"*foo\\\nbar*"`, `"[link  \ntext](u)"`,
    `` "`code  \nspan`" `` and ``` "``\nfoo\nbar  \nbaz\n``" ``` all show raw
    delimiters. `paragraph_flow` produces hard breaks before inline parsing,
    which then runs one hard line at a time. Inside a code span, the two
    spaces or backslash should not be a break at all.
45. **Collapsed reference images leave a literal `[]`.**
    `"![foo][]\n\n[foo]: /url\n"` shows the image followed by `[]`. So do
    `![*foo* bar][]`, `![Foo][]` and `First ![^1][] Second`.
46. **Inline links ignore precedence and line endings.** These sources do
    not contain the outer inline link that Viem constructs.
    - In `"[a <b x=\"](c)\">"`, raw HTML takes precedence.
    - In `"[foo<https://example.com/?search=](uri)>"`, the autolink takes
      precedence. Viem instead resolves the outer construct to `uri`,
      rather than the angle autolink's
      `https://example.com/?search=](uri)` destination.
    - In `"[a[ref]](url)\n\n[ref]: inner"`, the resolved inner reference
      prevents an outer inline link. Viem nevertheless resolves it to `url`.
    - In `"[link](<foo\nbar>)"`, a pointy-bracket destination cannot
      contain a line ending.

    [links.rs:89](src/core/document/links.rs#L89) scans brackets without
    raw-HTML or autolink precedence, on text where line endings are already
    folded into spaces.
47. **Over-long numeric character references decode.** `&#87654321;`,
    `&#00000000;` and `&#x0000000;` become U+FFFD. CommonMark allows at
    most 7 decimal or 6 hexadecimal digits, so GitHub keeps them literal
    ([html.rs:1086](src/core/document/html.rs#L1086)).
48. **Multi-line reference definitions get the wrong block boundaries.**
    - `"[foo]:\n/url\n'title'\n"` and `"[foo]: /url '\ntitle\nline1\n'"`
      show one paragraph per physical line.
    - `"   [foo]: \n      /url  \n           'the title'  \n"` keeps raw
      indentation and trailing spaces.
    - `"[\nfoo\n]: /url\nbar\n"`, `"[foo]: /url\n\"title\" ok\n"` and
      `"[foo]: /url\n===\n[foo]\n"` merge the definition with the
      following paragraph.

    Reference presentation is deliberately literal, but block boundaries
    should still follow the definition.

## Additional findings from the comparison audit

### Performance

49. **Autolinks with unmatched trailing parentheses take quadratic time.**
    Open `format!("http://example.com/{}", ")".repeat(n))` in either
    Markdown view. The trimming loop
    ([markdown_syntax.rs:98](src/core/document/markdown_syntax.rs#L98))
    recounts every opening and closing parenthesis for each removed `)`.
    Doubling the suffix approximately quadruples opening time; a 128,000-byte
    suffix takes seconds even in an optimized test build. Compute the
    balance once and update it while trimming; a small document must not
    require quadratic synchronous parsing work.

### Block and HTML projection

50. **Indented code after some block starts is parsed as prose.**
    `"---\n    *code*"` projects as `"\n    code"`, with a Paragraph
    rather than Code Block. The literal asterisks disappear and emphasis
    applies. The same problem follows Setext headings, empty ATX headings,
    indented ATX headings and tab-delimited headings. In
    `"Title\n===\n    &amp;"`, the code entity incorrectly decodes.
    [markdown_indented_code.rs:156](src/core/document/markdown_indented_code.rs#L156)
    decides whether the preceding line is prose using the incomplete
    `markdown_block_prefix`, rather than grammar-owned block boundaries.

    - (document API) `replace(6..7, "")` on the first source rejects an
      ordinary visible-character deletion with `VerificationFailed`.
    - (document API) `replace(10..11, "")` on the second source succeeds
      with live `"Title\n    "`, while saved `"Title\n===\n    "`
      reopens as `"Title"`.
51. **Markdown prose cooking changes literal HTML `<pre>` content.**
    - `"<pre>a\n\nb</pre>"` displays `"a\nb"`, losing a blank code line.
      Source view also shows `"<pre>a\nb</pre>"`.
    - `"<pre>a\\\nb</pre>"` displays `"a\nb"`, losing the backslash.
    - `"<pre>a  \nb</pre>"` displays `"a\nb"`, losing both spaces.

    Original bytes remain intact, but visible code and editing coordinates
    are wrong. [paragraph_flow.rs:150](src/core/document/paragraph_flow.rs#L150)
    exempts Markdown code scopes without exempting HTML blocks; its
    separator folding and hard-break cleanup then transform literal content.
52. **The ignored initial HTML `<pre>` newline remains visible.**
    `"<pre>\nfoo</pre>"` and `"<pre>&#10;foo</pre>"` display `"\nfoo"`
    instead of `"foo"`. [html.rs:955](src/core/document/html.rs#L955)
    recognizes the ignored LF only when advancing an empty caret boundary;
    the text-emission path still emits it. The HTML clipboard parser
    produces the correct text for both sources. The
    [HTML standard](https://html.spec.whatwg.org/dev/syntax.html#restrictions-on-content-models)
    specifies ignoring one initial newline.
53. **Allowed inline HTML styles disappear inside HTML blocks.**
    `"a<kbd>b</kbd>c"` gives `b` the Code style, but
    `"<div>a<kbd>b</kbd>c</div>"` gives it none. `<samp>` and `<tt>` behave
    the same; wrapping `<ins>` in `<div>` removes its underline.
    [markdown_html.rs:106–109](src/core/document/markdown_html.rs#L106)
    supports these treatments, while the
    [block HTML handler](src/core/document/html.rs#L833) omits them.

### Inline recognition and destinations

54. **Semicolon-free references silently change Markdown link destinations.**
    `"[a](foo&copy)"` resolves to `foo©`, and `"[a](foo&#123)"` resolves to
    `foo{`, instead of retaining the literal destinations. This is observable
    through `Document::link_at` in both views. Source bytes are unchanged,
    but explicit navigation targets a different URL or file.
    [links.rs:33](src/core/document/links.rs#L33) uses permissive HTML entity
    recovery instead of Markdown's semicolon and digit-count rules. Prose
    already checks for a semicolon, although it has bug 47's length defect.
55. **Malformed parenthesized link titles become links.**
    `"[a](url (tit(le))"` should stay literal, but Viem displays linked `a`
    with destination `url`. Adding a final `)` displays `a)` instead of the
    literal source. [links.rs:203](src/core/document/links.rs#L203) scans
    until the first closing parenthesis without rejecting an unescaped
    opening parenthesis inside the title.
56. **Code spans containing non-space whitespace keep unwanted edge spaces.**
    `` "` \t `" `` displays space–tab–space instead of one tab; NBSP and
    other Unicode whitespace have the same problem.
    [projection.rs:6401](src/core/document/projection.rs#L6401) uses Unicode
    `.trim()` for the code-span exception that applies only to content made
    entirely of ASCII spaces.
57. **Valid extended email autolinks are rejected.** `a@b.c1` is not a link,
    and domains with internal underscores are rejected.
    [markdown_syntax.rs:117](src/core/document/markdown_syntax.rs#L117)
    requires a final alphabetic character and bans every underscore; GFM
    permits a final digit and internal underscores, provided the final
    character is neither `-` nor `_`.

### Container boundaries and furniture

58. **Lazy quote context leaks into following top-level blocks.**
    `"> a\n#\tfoo"` gives the following top-level heading quote depth 1
    instead of 0. The lazy-continuation check
    ([markdown_quotes.rs:160](src/core/document/markdown_quotes.rs#L160))
    relies on the incomplete block recognizer. Conversely, `"> a\n2. b"`
    keeps `2. b` as literal prose instead of starting the top-level ordered
    list: quote stripping loses the boundary before list classification.
    (document API) `replace(2..2, "X")` on the latter commits live
    `"a\nX2. b"`, while saved `"> a\nX2. b"` reopens as `"a X2. b"`.
59. **Allowed HTML `<hr>` is stripped without a rule or boundary.**
    `"<div>a<hr>b</div>"` becomes `"ab"`, and `"two<hr>three"` becomes
    `"twothree"`. `<hr>` belongs to the admitted passive HTML vocabulary,
    but neither the [inline handler](src/core/document/markdown_html.rs#L67)
    nor the [block handler](src/core/document/html.rs#L833) creates thematic
    furniture or a paragraph boundary. This is a projection defect, rather
    than an unsupported-tag fallback.

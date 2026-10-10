# Markdown compatibility with GitHub

Updated October 9, 2026. The target is GitHub's rendering of repository
Markdown files, using the [GFM specification](https://github.github.com/gfm/)
for syntax. This inventory distinguishes deliberate Viem presentation choices
from deferred work. It does not claim complete GFM conformance. Parsing and
editing defects, where Viem misreads syntax it claims to support, are tracked
separately in [BUGS.md](BUGS.md).

## Implemented from the P1/P2 audit

- **Emphasis delimiter rules:** CommonMark opening/closing, intraword
  underscores, nesting and delimiter runs. The grammar recognizer supplies
  source ranges; Viem retains the original bytes and editable provenance.
  Bold and italic can be authored together or nested in either direction.
- **Underline and scripts:** passive `<ins>`/`<u>`, `<sup>` and `<sub>` project
  underline, superscript and subscript. Native character actions author these
  effects through verified source edits and pending typing.
- **Automatic links:** angle URLs and email addresses, and GFM bare HTTP(S),
  `www.` and email links, with punctuation and balanced-parenthesis handling.
- **Inline link authoring and navigation:** the native toolbar inserts Markdown
  links from Text and Destination fields, with selected text prefilled. Caret
  popups open, copy, edit or remove inline links and automatic links; editing
  an automatic link converts it to explicit inline syntax. Source treats the
  complete `[text](destination)` notation as the link. Heading fragments navigate the
  current document, local document links focus an existing view or pane when
  available and otherwise open a new Viem window, and HTTP(S) links open the
  default browser. Local document fragments target headings in the chosen view.
  Heading names use lowercase text with punctuation removed, hyphenated spaces
  and duplicate suffixes. The destination combobox lists those headings; a
  caret without a link or selection starts an empty link draft. Compact popups
  fit the link label within their previous maximum width.
- **Inline images:** Markdown images, including resolved references, and passive
  HTML `<img src="…">` tags are atomic
  objects in WYSIWYG; Source retains full notation. Native Insert Image controls
  and location popups insert, edit, copy, open or remove images with ordinary
  undo. Local previews retain intrinsic aspect ratio and fit the content width
  without upscaling, with a maximum displayed width or height of 1024
  device-independent pixels even when document zoom increases. HTML `width`
  and `height` accept positive integer pixel dimensions and may enlarge a
  preview; one dimension preserves intrinsic proportions, while both specify
  the displayed proportions. Content width and the same display caps still
  apply. Location edits preserve dimensions and other untouched attributes;
  dimension controls are not exposed. Remote URLs are
  text placeholders and are never fetched.
  Explicitly opening a remote location launches the default browser.
  The editable Image paragraph style supplies margins, padding and borders for
  image-only ordinary paragraphs. Its font and color style image labels and
  complete Source notation, including inline images within prose. Prose,
  heading, list and table paragraphs retain their own geometry. Unavailable
  local previews show their location with a broken-image indicator and can be
  refreshed after the local resource changes without fetching remote images.
- **Reference links:** full, collapsed and defined shortcut links and reference
  definitions keep their literal brackets in WYSIWYG, styled with the light-purple
  **Markdown reference** character style. Definition edits invalidate dependent
  image metadata as well as reference styling.
- **Composed containers:** headings in lists, quotes inside lists, nested quote
  depth, and Code Block/heading presentation inside quotes. Quotes, code blocks,
  lists, and list items have explicit owners; contained paragraphs retain
  their own styles. Each owner has its own background, margins, padding, and
  per-side borders, with normal-flow vertical margin collapsing. Removing one
  treatment preserves the surrounding containers. Ordered children starting
  above one need a blank separator before they can interrupt prose; otherwise
  the marker remains visible continuation text through structural editing.
- **Code fences:** opener-relative indentation is hidden, and closers permit
  at most three spaces relative to their container. Four-space-indented fence
  lookalikes stay in the code body. Fences end with their containing quote;
  a later unquoted fence starts an independent code block. Live edits and
  reopened files keep those owners separate. Indented code remains supported.
  WYSIWYG blocks highlight specified languages with the Code syntax system and
  offer a native language selector. None retains the Code Block style. New
  blocks inherit the preceding language or, without a preceding block, the next.
- **Headings and thematic breaks:** Setext underlines, optional closing ATX
  markers and horizontal rules, including the relevant block precedence.
- **Strikethrough:** GFM single/double-tilde spans use the generated
  **Strikethrough** character style.
- **Passive HTML and character references:** supported inline/block HTML uses
  the shared HTML projection; scripts, resource loading and author CSS are not
  enabled. Markdown inside HTML blocks stays literal. Named and numeric
  character references decode, including invalid-code-point replacements.
  Comments remain visible in the new **Comment** character style.
- **Prose continuation whitespace:** indentation and trailing whitespace at a
  soft source break fold to one space. Explicit hard breaks remain distinct.
- **Loose lists:** items receive paragraph spacing; tight lists retain their
  compact presentation. Tightness is structure, not direct formatting.
- **GFM pipe tables:** header/body structure, alignment, optional pipes,
  short and excess rows, inline formatting and explicit cell breaks. WYSIWYG
  uses content-sized cells; Source aligns the original syntax through geometry
  without adding source characters. Native insertion, row/column actions and
  rectangular cell editing follow the [table specification](docs/markdown-tables.md).
  Nested tables honor enclosing list/quote insets and continuous container
  borders/backgrounds in full, regional and progressive large-cell layout.
  Counted structured matrix puts are explicitly rejected; single puts retain
  the matrix instead of flattening it to text.

Supporting edits retain the requested visible text. Depending on the edit,
this can require numeric whitespace references, switching an affected Setext
heading to ATX, or replacing an affected emphasis delimiter pair with passive
HTML tags. Such repairs are explicit source patches in the same undo transaction.
Unedited source is not regenerated.

The loadable feature tour is [docs/markdown_demo.md](docs/markdown_demo.md).
Regression coverage includes `tests/all/markdown_gfm.rs` and the existing
Markdown source, caret, deletion, replacement, formatting and layout audits.

## P1 — deferred structure

1. **HTML tables.** Raw HTML tables retain literal source. Pipe-table support
   does not enable editing HTML tables, merged cells, or row/column spans.

## P2 — deliberate presentation choices and deferred features

2. **Task-list checkboxes.** `[ ]` and `[x]` stay literal bullet-item text;
   there are no checkbox controls.
4. **Image/reference presentation.** Remote images never load: Viem shows their
   URLs in boxes. Missing, unsupported or invalid local files use placeholders.
   Local previews are limited to 10 MB (10,000,000 bytes) and 5000 source pixels
   in each dimension; exactly those limits are allowed. Larger resources show
   their location without the broken-image indicator and remain placeholders
   until explicitly reloaded or their document resource identity changes.
   Native decoders support local raster formats; SVG previews,
   animated playback and resizing handles
   remain unsupported. Reference links and definitions remain visible source
   notation with Markdown reference styling. Inline formatting inside a
   reference link's label also stays literal: `[*foo* bar][ref]` shows its
   asterisks, while GitHub renders the emphasis.
5. **Comments.** GitHub hides comments; Viem deliberately displays and styles
   them so they remain directly editable.
6. **Surplus blank separators.** Viem deliberately retains editable empty
   paragraphs from repeated separator pairs. Do not collapse them to GitHub's
   presentation.
7. **Unsupported HTML and filtered content.** Tags outside the passive
   vocabulary keep their literal source syntax. This differs from GitHub's
   supported passive HTML and its subsequent filtering and sanitization.
   - `<picture>` and `<source>` remain literal, including the common README
     pattern for light and dark logos; only the inner `<img>` renders.
     Theme-dependent source selection is unsupported, including image URL fragments
     `#gh-dark-mode-only` and `#gh-light-mode-only`. GitHub documents
     [theme-aware README images](https://docs.github.com/en/get-started/writing-on-github/getting-started-with-writing-and-formatting-on-github/quickstart-for-writing-on-github#adding-an-image-to-suit-your-visitors)
     and the
     [image-fragment form](https://github.blog/changelog/2021-11-24-specify-theme-context-for-images-in-markdown/).
   - Custom elements such as `<Warning>` or `<foo>` remain literal.
   - Processing instructions (`<?php … ?>`), CDATA sections and
     `<!DOCTYPE …>` display as literal text; their GitHub sanitizer
     presentation requires separate comparison.

   Do not treat every filtered tag as hidden content. GFM's
   [disallowed raw HTML extension](https://github.github.com/gfm/#disallowed-raw-html-extension-)
   escapes the opening `<` in tags such as `<script>`, `<style>` and
   `<textarea>`, and the
   [GitHub markup implementation](https://github.com/github/markup/blob/master/lib/github/markup/markdown.rb)
   enables that filter. Their literal appearance alone is not a missing
   rendering feature. Supported `<pre>`, inline styles and `<hr>` retain
   their passive semantics, including literal preformatted whitespace and
   thematic-rule paragraph boundaries.
8. **Character references in the C1 range.** Numeric references from 0x80
   to 0x9F decode through Windows-1252, as in HTML5, so `&#x80;` shows `€`.
   GitHub's cmark-gfm decodes them as the C1 control code points. The current
   behavior is covered by a test, so this is a deliberate choice to confirm
   or revisit.

## P3 — unchanged and not implemented in this pass

9. **Footnotes and alerts.** Footnote references/backlinks and GitHub alert
   titles, icons and treatments remain unsupported.
10. **Math and diagrams.** Math expressions stay literal. Mermaid, GeoJSON,
   TopoJSON and ASCII STL fences remain code rather than rendered diagrams,
   maps or 3D models. GitHub supports
   [all four diagram formats](https://docs.github.com/en/get-started/writing-on-github/working-with-advanced-formatting/creating-diagrams)
   in repository Markdown.
11. **Emoji and GitHub navigation.** Emoji shortcodes remain literal. Explicit
   HTML anchors and a generated table of contents remain unsupported;
   Markdown heading links are supported. Mentions and conversation-specific
   issue/PR reference autolinks are separate GitHub application behavior.
   GitHub itself
   [does not create issue/PR reference autolinks in repository files](https://docs.github.com/en/get-started/writing-on-github/working-with-advanced-formatting/autolinked-references-and-urls),
   so those are outside this repository-Markdown compatibility target.
12. **YAML front matter.** GitHub renders valid YAML front matter as a metadata
   table. Viem has no front-matter recognition: for example,
   `---\ntitle: Example\n---` becomes a thematic break followed by a Setext
   H2. More complex metadata follows ordinary Markdown block parsing.
13. **Collapsible sections.** `<details>` and `<summary>` render as flat
   text, with the summary and body always visible and no disclosure control.
14. **HTML alignment attributes.** `align` on passive HTML is ignored, for
   example `<p align="center">`, `<div align="center">`, `<h1 align="center">`
   and `<img align="right">`. Centered README headers and logos render
   left-aligned.

## Remaining conformance audit

The selected audit fixtures are covered; the full GFM example corpus and
GitHub's HTML sanitizer are broader than those fixtures. An October 9, 2026
run of the CommonMark spec examples, the GFM examples and pulldown-cmark's
regression suite against both views found parsing and editing defects recorded
in [BUGS.md](BUGS.md). The numbered parser reports now have focused regression
coverage; that does not establish full-corpus conformance. Continue differential
coverage of malformed/container nesting, tabs at every depth, Unicode
punctuation, autolink edge cases, HTML recovery and allowed-tag presentation.
Native typography and controls need not reproduce GitHub's CSS pixel for pixel.

Link authoring is limited to a single paragraph outside code and passive HTML.
Passive HTML anchors expose their destination but retain read-only editing
controls; reference-link presentation remains literal. Email launching is
unsupported. Link and image controls are implemented on both native frontends; Windows
runtime validation remains outstanding on a Windows host.

Table event, rendering, and accessibility adapters are implemented on both native
frontends. macOS has automated native coverage and app interaction checks;
Windows runtime validation remains outstanding on a Windows development host.

The shared bounded shaper does not yet carry distant explicit Unicode bidi
embedding, override, or isolate controls into later text slices. A control such
as U+202E followed by thousands of characters can therefore lose its directional
effect in later slices, in ordinary unwrapped text as well as large table cells.
Natural paragraph direction and ordinary mixed-direction table content retain
their direction across slices; carrying explicit control stacks is separate
typography work.

Reference-sensitive source edits use a complete parse so
changes to remote definitions cannot leave stale styling. Ordinary prose/list
edits retain regional projection. A cached definition dependency index is a
future performance improvement for very large reference-heavy files.

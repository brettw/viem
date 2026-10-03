# Markdown compatibility with GitHub

Updated October 3, 2026. The target is GitHub's rendering of repository
Markdown files, using the [GFM specification](https://github.github.com/gfm/)
for syntax. This inventory distinguishes deliberate Viem presentation choices
from deferred work. It does not claim complete GFM conformance.

## Implemented from the P1/P2 audit

- **Emphasis delimiter rules:** CommonMark opening/closing, intraword
  underscores, nesting and delimiter runs. The grammar recognizer supplies
  source ranges; Viem retains the original bytes and editable provenance.
- **Automatic links:** angle URLs and email addresses, and GFM bare HTTP(S),
  `www.` and email links, with punctuation and balanced-parenthesis handling.
- **Images and reference links:** full, collapsed and defined shortcut
  references, images and definitions keep their literal brackets in WYSIWYG,
  styled with the new light-purple **Markdown reference** character style.
  No image resources are loaded. Definition edits invalidate dependent styling.
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
3. **Fenced-code syntax highlighting.** Info strings remain preserved in source;
   code bodies use Code Block styling without language highlighting.
4. **Image/reference presentation.** GitHub shows images or linked labels and
   hides definitions. Viem deliberately displays their source notation with
   Markdown reference styling instead.
5. **Comments.** GitHub hides comments; Viem deliberately displays and styles
   them so they remain directly editable.
6. **Surplus blank separators.** Viem deliberately retains editable empty
   paragraphs from repeated separator pairs. Do not collapse them to GitHub's
   presentation.
7. **Unsupported inline HTML.** `<sub>` and `<sup>` retain literal source syntax.

## P3 — unchanged and not implemented in this pass

8. **Footnotes and alerts.** Footnote references/backlinks and GitHub alert
   titles, icons and treatments remain unsupported.
9. **Math and diagrams.** Math expressions stay literal; Mermaid and other
   diagram fences remain code.
10. **Emoji and GitHub navigation.** Emoji shortcodes remain literal. Heading
   anchors/table-of-contents navigation, repository mentions, issue links and
   commit links are separate future work.

## Remaining conformance audit

The selected audit fixtures are covered; the full GFM example corpus and
GitHub's HTML sanitizer are broader than those fixtures. Continue differential
coverage of malformed/container nesting, tabs at every depth, Unicode
punctuation, autolink edge cases, HTML recovery and allowed-tag presentation.
Native typography and controls need not reproduce GitHub's CSS pixel for pixel.

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

Reference-sensitive source edits currently use a complete parse so changes to
remote definitions cannot leave stale styling. Ordinary prose/list edits retain
regional projection. A cached definition dependency index is a future
performance improvement for very large reference-heavy files.

# Remaining Markdown work

Audited November 15, 2026 against the implementation and regression tests.
The compatibility target is GitHub's rendering of repository Markdown, using
[GFM](https://github.github.com/gfm/) and GitHub's documented extensions.
This file contains only remaining feature, performance and validation work.
Deliberate presentation choices live in [AGENTS.md](AGENTS.md), supported
features in the [demo](docs/markdown_demo.md), and defects in supported behavior
in [BUGS.md](BUGS.md).

## Rendering and navigation

- **HTML tables.** Render and edit raw HTML tables, including merged cells and
  row/column spans. They currently retain literal source.
- **Footnotes.** Add references, note presentation and backlinks.
- **Emoji shortcodes.** Expand shortcodes, which currently remain literal.
- **Explicit HTML anchors.** Resolve fragment links to HTML `id`/`name` targets.
- **Table of contents.** Add a generated table of contents.
- **Collapsible sections.** Add disclosure controls for `<details>` and
  `<summary>`; their contents currently remain visible.
- **HTML alignment.** Honor passive HTML `align` attributes, including on
  paragraphs, divisions, headings and images.

## Authoring and performance

- **Image resizing handles.** Add drag handles for resizing previews.
- **Passive HTML links.** Add editing controls for `<a>` links; their
  destinations are exposed, but the link controls remain read-only.
- **Link labels with hard breaks.** Support authoring labels containing an
  explicit hard break within one paragraph.
- **Email links.** Support explicitly opening `mailto:` destinations.
- **Reference-sensitive edits.** Avoid complete reprojection when edits affect
  reference definitions or their uses. The revision-cached reference index
  supports inspection and navigation, but reference-sensitive changes still
  fall back to a complete parse. Track dependencies so large reference-heavy
  files can update only affected content.
- **Explicit bidi controls across shaping slices.** Carry embedding, override
  and isolate context into distant text slices. A control such as U+202E
  followed by thousands of characters can lose its effect in later slices,
  including ordinary unwrapped text and large table cells.
# Viem: A more native gvim implementation

The goal is to have an editor that is a cimbination of gvim and Microsoft Word. The editing model is based on gvim with modes and ex commands. But it supports richer layout including proportional fonts, different font sizes, etc. These are exposed in a “style” system more like a traditional word processor.

The name is “vi” + “em” (like an em dash or em space, indicating the proportional font support).

### Flexible text handling

With an emphasis more on traditional text editing, it can automatically wrap long lines in the viewport separately from the physical newlines in the file. A line can either by a “physical” line as by counting newlines in the source file (default for code), or a “visual” line as counting line in the current view (default for text views). This controls the line display in the status and line\-based commands like `dd`. You can toggle this by clicking on the line number display.

It has several presentation modes.

- Text (for plain text files)
- Code (for source code)
- Markdown source and WYSIWYG
- HTML source and WYSIWYG
- RTF

### Code editing

Viem uses treesitter for the most popular languages for dynamic syntax highlighting. This happens on demand asynchronously to remain responsive, with the disadvantage being that you can get flashes of unstyled content when scrolling quickly. It falls back to vim syntax definitions for other formats.

Code mode supports optional word wrapping for long lines which are indented by one extra tab stop from the first one.

In code mode, leading spaces are always treated as 1en wide. This allows the use of proportional fonts while maintaining reasonable indenting.

### Markdown and HTML modes

These modes are more experimental.

The “source” modes show the literal source of the file while styling the formatting, at least to some extent.

The “WYSIWYG” modes attempt to display a version of the final rendering. HTML is treated very simply, showing basic headings, paragraphs, and things like bold, but it is not a full HTML renderer.
# Viem: A more native gvim implementation

The goal is to have an editor that is a cimbination of gvim and Microsoft Word. The editing model is based on gvim with modes and ex commands. But it supports richer layout including proportional fonts, different font sizes, etc. These are exposed in a “style” system more like a traditional word processor.

The name is “vi” + “em” (like an em dash or em space, indicating the proportional font support).

### Flexible text handling

With an emphasis more on traditional text editing, it can automatically wrap long lines in the viewport separately from the physical newlines in the file. A line can either by a “physical” line as by counting newlines in the source file (default for code), or a “visual” line as counting line in the current view (default for text views). This controls the line display in the status and line\-based commands like `dd`. You can toggle this by clicking on the line number display.

It has several presentation modes.

- Text (for plain text files)
- Code (for source code)
- Markdown source and WYSIWYG

### Code editing

Viem uses treesitter for the most popular languages for dynamic syntax highlighting. This happens on demand asynchronously to remain responsive, with the disadvantage being that you can get flashes of unstyled content when scrolling quickly. It falls back to vim syntax definitions for other formats.

Code mode supports optional word wrapping for long lines which are indented by one extra tab stop from the first one.

In code mode, leading spaces are always treated as 1en wide. This allows the use of proportional fonts while maintaining reasonable indenting.

### Markdown modes

The toolbar’s Formatted view toggle switches between Markdown Source, which keeps markup visible while applying its styles, and WYSIWYG, which shows the interpreted document. New Markdown documents remember your last choice across launches. With no previous choice, Formatted view is off.

HTML files open as literal Code, with syntax highlighting and ordinary code editing.

### HTML export

File > Export… writes a standalone HTML copy with the document’s styles translated to CSS. Markdown exports its formatted content from either view. Code exports its literal text with syntax highlighting and preserved whitespace. Export does not change the open document’s format, filename, source bytes, or saved state.

Style > Theme selects an application-wide theme for all windows. Each file in `~/.viem/themes` contains the appearance settings and format style sheets; Settings > Theme and the Styles editor edit that active theme. New profiles include Paper and Midnight and select Midnight. Default uses the built-in Midnight values and is editable for the current session. Choose New theme… to save a copy of your current settings, including changes made to Default.

# Split views

`:split` (`:sp`, Control-W s) opens and focuses another view above the current one.
`:vsplit` (`:vs`, Control-W v) opens and focuses one to its left. Both views share the
buffer, edits, and history, with independent cursors, selections, scrolling,
wrapping, and line modes. Horizontal and vertical splits can nest.
Each view also shows the buffer its own way: a split shows what its source
view shows, so views of one Markdown buffer may differ, one in Preview and
one showing the source. Turning Preview on or off changes only the active
view. A new window of a Markdown buffer shows the remembered Preview choice
(see [AGENTS.md](../AGENTS.md#configuration-and-themes)).

`:new` and `:vnew` open an empty buffer in a horizontal or vertical split.
`:enew` replaces the current buffer. `:vertical split` and `:vertical new`
are equivalent to their vertical forms. A count sets the new view's height
or width. Startup `-o[N]` and `-O[N]` request horizontal or vertical views;
the last orientation option wins.

Each view remembers its alternate file: the file it showed before its current
buffer was replaced, for example by `:edit` or argument navigation. Control-^
(Control-6) edits it, with the same modified-buffer check as `:edit`. A split
starts with its source view's alternate file. The native frontends own this
per-view history and supply it to the core for the `#` register.
See [registers and marks](vim-registers-and-marks.md) for filename reads and
application-wide uppercase marks.

| Command | Action |
| --- | --- |
| Control-W h/j/k/l, Left/Down/Up/Right | Focus a neighbouring view; a count repeats the movement |
| Control-W w/W | Next/previous view, or a numbered view with a count |
| Control-W t/b/p | First, last, or previously accessed view |
| Control-W r/R/x | Rotate or exchange views within the current row or column |
| Control-W H/J/K/L | Move the current view to the far left/bottom/top/right |
| Control-W + / - / _ | Grow, shrink, or set height |
| Control-W > / < / \| | Grow, shrink, or set width |
| Control-W = | Equalize heights and widths |
| Control-W c/q/o | Close the view, close the view, or close other views |
| `:wincmd {key}` | Execute the corresponding Control-W command |
| `:resize [N/+N/-N]` | Set, grow, or shrink height; an omitted size maximizes it |
| `:vertical resize [N/+N/-N]` | Set, grow, or shrink width |
| `:{window}resize ...` | Resize a numbered view without changing focus |
| `:vertical wincmd =` / `:horizontal wincmd =` | Equalize heights / widths only |

Height counts use the Base Paragraph font size times its line spacing and view
zoom. Width counts use the Base Paragraph's zoomed advance of `0`. Mixed text
sizes and current visible rows do not change these units.

`:set number` (`:set nu`) or **View > Show line numbers** shows physical source
line numbers in the current view; `:set nonumber` hides them. The menu checkmark
reflects that view's setting. A split copies this option, then each view
can change it independently. Soft wrapping does not add line numbers. In
Markdown Preview, hidden syntax keeps its source ordinal without a separate
number. When several source lines share a rendered row, only its first source
ordinal is displayed. Current-line styling follows that displayed number.

Absolute addresses such as `23G`, `23gg`, `:23` and `:23,25d` always use
physical source lines. Counts beyond the file stop at its last numbered line for
navigation; an invalid Ex address produces an error. Contextual Ex addresses
follow line mode: in Visual mode, `:.,.+2d` selects three displayed rows.
Offsets inherit their base: `:23+2` means physical line 25. Mixed ranges preserve
the exact boundaries of both endpoints.

The builtin block styles **Line number** and **Current line number** control
the gutter. Their builtin identity distinguishes them from content or syntax
styles with the same names. Current line number inherits the ordinary style
without default overrides. The gutter reserves enough width for either style
and the document's largest line number. A single-document window without a
vertical split grows or shrinks to preserve content width when the monitor has
room, preferring to keep the content's left edge in the same screen position.
Other windows take gutter space from the corresponding content view.

Click a status bar to focus the buffer above it. Clicking the eye also toggles
line mode in Text and Markdown Preview; Code always uses physical source lines
and shows no eye. On macOS, the click that activates a window does not also
reach its status bars. Dragging consumes the click and preserves focus. A
dragged bar keeps the pointer's offset within it, even after the pointer leaves
the window and returns. Status bars use an
up/down cursor during dragging. Drag a column's bottom bar upward past its top
to open a new pane of the same document below it; the dragged bar keeps
following the pointer, and the new pane's bar stays at the window edge. The
rest of the gesture is an ordinary status drag, so dragging back down closes the
new view again as described below. Focus stays put, and dragging the bottom bar
downward does nothing. Each gesture creates at most one split and needs room
for another status bar.

Dragging a bar down stacks the bars below it at the bottom of its column. When
every pane from there to the bottom of that stack shows the same buffer as the
dragged pane, dragging further closes them one at a time, bottom first. The
bar still stops against the next one, and a view closes only once the pointer
has moved the bar's full height past that stop, so the bar would cover the next
one completely; the dragged pane can reach the bottom. Closing is a preview until release: dragging
back restores the same views, and only release closes them. Such a drag never
closes a document.

Each status bar's context menu has **Close pane**, which closes only that view,
reviewing unsaved changes when it is the buffer's last view, as **Close** does.
It closes the window when it is the window's last pane. The filename menu lists
it after its own commands.

The filename extends to within 10 DIPs of the position. Without room, leading
directories are dropped first, then the filename is clipped at its end; a
clipped filename shows the whole path in a tooltip.

Status bars
use the same height and centered text regardless of split width. Empty
background in any status bar also shows the up/down cursor on hover;
text and controls keep their usual hover cursors. Editor areas
can collapse completely, but status bars may only touch, never overlap. A
horizontal split needs room for another status bar in the current view.

The internal **Status bar** container style controls the shared status font,
colors, borders, and padding across document formats. Margin and alignment do
not affect the bar. **Status bar command** inherits it and controls command
text and its background within the existing command area. Its font is centered
and clipped without changing status bar height.

Vertical dividers are five DIPs wide and use the status background and matching
separator borders. They show a left/right cursor on hover and while dragging.
Every view keeps at least 100 DIPs of width, so a vertical split requires 205
DIPs in the current view. A split without room reports an error.

Dragging either kind of divider pushes any neighbours it reaches, collecting
more dividers until it reaches the edge. Vertical groups retain their minimum
widths. Reversing moves only the grabbed divider until it reaches a neighbour
in the new direction. A vertical divider reverses immediately, even after a drag
has reached the edge; a status bar resumes once the pointer returns to it.

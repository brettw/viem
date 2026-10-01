# Split views

`:split` (`:sp`, Control-W s) opens another view below the current one.
`:vsplit` (`:vs`, Control-W v) opens one to its right. Both views share the
buffer, edits, and history, with independent cursors, selections, scrolling,
wrapping, and line modes. Horizontal and vertical splits can nest.

`:new` and `:vnew` open an empty buffer in a horizontal or vertical split.
`:enew` replaces the current buffer. `:vertical split` and `:vertical new`
are equivalent to their vertical forms. A count sets the new view's height
or width. Startup `-o[N]` and `-O[N]` request horizontal or vertical views;
the last orientation option wins.

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

Click a status bar to focus the buffer above it. Clicking the eye also toggles
line mode. Dragging consumes the click and preserves focus. Status bars use an
up/down cursor during dragging; each column's bottom bar is fixed. Editor areas
can collapse completely, but status bars may only touch, never overlap. A
horizontal split needs room for another status bar in the current view.

Vertical dividers are five DIPs wide and use the status background and matching
separator borders. They show a left/right cursor on hover and while dragging.
Every view keeps at least 100 DIPs of width, so a vertical split requires 205
DIPs in the current view. A split without room reports an error.

Dragging either kind of divider pushes any neighbours it reaches, collecting
more dividers until it reaches the edge. Vertical groups retain their minimum
widths. Reversing moves only the grabbed divider until it reaches a neighbour
in the new direction, even after a drag has reached the edge.

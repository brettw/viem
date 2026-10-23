# Viewport stability

Ordinary edits and layout changes preserve the viewport's text anchor and its
screen offset. Changed content and the caret move naturally with reflow. Scroll
only as required by navigation, minimal caret reveal or the valid document
extent. This policy applies regardless of whether an action comes from a key,
a Vim command, a toolbar or another native control.

Preserving the viewport does not mean freezing its numeric scroll offset.
Edits or refined height estimates above the viewport can change the document
coordinate of its anchor. Adjusting the offset to compensate keeps the visible
text stationary. Geometry estimates must not introduce premature clamps, and
cache-only pre-layout must not move displayed content.

## Why the caret baseline is not the default anchor

A structural edit can intentionally change the space above the caret. Removing
an empty code block, for example, removes its padding. Holding the caret's old
screen baseline compensates for that change by moving the entire viewport,
including otherwise unchanged content. Creating a block can cause the reverse
movement. Neither action calls for scrolling when the edited location already
fits on screen.

The previous blanket baseline-preservation requirement conflicted with viewport
stability whenever the distance between the viewport anchor and the editing row
changed. Prefer the viewport for ordinary edits, explicit style changes, width
changes and composition. Preview and committed IME content should have matching
geometry when their content and formatting match; baseline compensation must
not conceal a mismatch.

## Exception: delayed syntax highlighting

Asynchronously arriving syntax highlighting can change fonts or metrics after
the user has already seen and started editing a row. That delayed baseline
movement was distracting, so this case deliberately prioritizes the visible
editing row's baseline over the viewport anchor. Surrounding text may move as a
consequence; this is the limited tradeoff we accept.

Use the exception only when current syntax results change geometry and the
editing row was visible before their application. Preserve its screen baseline
while the resulting row fits, subject to minimal reveal and document bounds.
Do not bring an offscreen caret into view merely because syntax arrived.
Stale results must still be rejected. Color-only syntax updates need no reflow
or scroll adjustment. Other asynchronous layout or font updates retain the
ordinary viewport policy.

## Regression checks

Measure unchanged text in screen coordinates as well as the caret and scroll
offset. A stable caret-to-block distance cannot detect the whole viewport
moving. Cover block creation/removal, ordinary typing and undo/redo at scrolled
positions, minimal reveal near viewport edges, and shrinkage near document end.
Separately cover delayed syntax metric changes, color-only updates, an
offscreen caret, stale publication and refinement of heights above the viewport.
Ordinary edits should allow natural caret movement; delayed syntax geometry
should retain the visible baseline whenever it fits.

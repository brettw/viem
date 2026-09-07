# Document scrollbars

The macOS editor uses native `NSScroller` controls while the portable core
continues to own the viewport. Thumb, page, and line actions request checked
core scroll positions and do not move the editing caret or change source.
`NSScroller.preferredScrollerStyle` and its change notification track the system
Show scroll bars setting. Legacy controls reserve stable gutters; overlay
controls fade after idle time. Horizontal availability also fades instead of
changing layout dimensions as wide lines enter and leave view.

Standalone overlay `NSScroller` controls rely on an `NSScrollView` coordinator
for native knob opacity. A noninteractive sibling paints the overlay thumb from
the native knob geometry, and the container routes visible native parts when
AppKit's private opacity would reject a hit. Native controls retain dragging,
tracking, page actions, and accessibility. Legacy painting remains native.

Horizontal extents come from rows intersecting the current vertical viewport.
The core caches each row's extent with its immutable layout, then queries only
visible rows using binary search. Overscan and wider offscreen rows do not
enlarge the range. Vertical scrolling clamps the horizontal origin when the
newly visible rows have a narrower range. Wrapped content can still scroll
horizontally if an indivisible cluster overflows.

Vertical thumb size uses the existing total-height estimate, refining as layout
becomes exact. Neither scrollbar requests full-document layout. A system change
between overlay and legacy style updates the usable viewport once; alpha and
availability changes do not reflow text.

Dragging the vertical thumb to its bottom endpoint requests the actual document
end, so refining an estimated height cannot leave the last rows out of reach.
The final coverage grows backward to fill the viewport. An unmeasured, long
wrapped paragraph discovers its wrap checkpoints in bounded chunks, retaining
only the visible tail; repeat visits reuse those checkpoints. Trailing empty or
short lines do not force the preceding paragraph into a full layout export.
Scrollbar appearance follows the document canvas for contrast, including a light
document in a dark system appearance.

Regression coverage includes visible-row boundaries, distant widest lines,
edit/undo/resize cache invalidation, large-document bounded layout, real native
thumb/page actions, system style notifications, fade timing, and timer cleanup.
Native tests use `EVIM_CONFIG_DIR=/private/tmp/evim-quality-config`. Disposable
computer-use fixtures are under `/private/tmp/evim-scrollbars-validation`.

The 250-line computer-use fixture contains a medium-width line at line 61 and
a much wider line at line 221. With the system's Always setting, native dragging
confirmed that the horizontal scrollbar appears for the medium line, scrolls
its text, disappears and resets the horizontal origin on short lines, and has a
smaller thumb when the much wider line enters view. Scrollbar dragging leaves
the editing caret in place. The scrolling-only preference is exercised with an
isolated application process argument; the user's global preference is unchanged.

The final overlay computer-use pass confirmed both native thumb drags, including
horizontal movement without entering Visual mode. Moving away from the controls
and waiting 1.5 seconds left both controls fully faded out, including their
accessibility elements. Returning to short lines restored the horizontal origin.
The final legacy pass confirmed readable gray thumbs on the Paper canvas and a
vertical value of 1 at the true document end, with the editing caret unchanged.

Final validation: **1,474 Rust tests passed (1 ignored)** and **337 native tests
passed**, with no failures. The release app was rebuilt and signed at
`.build/eVim.app`. Logs are `/private/tmp/evim-scrollbars-core-complete.log`,
`/private/tmp/evim-scrollbars-native-complete.log`, and
`/private/tmp/evim-scrollbars-complete-build.log`.

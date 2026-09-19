# Style inspector following and color picking

Windows now shares macOS's caret-style selection policy: use a single named
character style, otherwise a uniform paragraph style, otherwise Base Paragraph.
The core query supplies validated named assignments and retained Code syntax
runs; font appearance is not used to guess the style. Opening and reopening
select immediately. A standalone Code settings inspector has no followed view.

Both frontends wait for 0.5 seconds of logical selection inactivity before
querying the style or updating the dialog. There is one pending one-shot timer,
restarted only by actual caret/selection changes. Scrolling, unchanged reports,
layout generations and stylesheet revisions do not change the followed
selection. Explicit picker/hierarchy choices cancel pending following, and
retargeting or closure detaches the previous context. Windows also defers
document-notification reloads while caret following is pending.

Windows measures and arranges the populated inspector while hidden, then
remeasures after template bindings settle before fitting its native window.
Using only the initial measure produced a screen-height window with empty space
below the form. It no longer resizes from successive `SizeChanged` events during
opening. Later loads fit
new guidance/error rows only when the required client size changes. Both tabs
retain the same formatting area, and the preview's initial clear color matches
its theme background.

Windows color controls previously displayed the core's emergency black rather
than resolving default foreground through the editor theme. The well, popup,
override initialization and preview now share that resolution. Explicit colors
retain their RGBA values. Opening a popup does not write source, reopening does
not round-trip colors, and choosing a different color commits once when it closes.
Dialog refreshes preserve popup drafts, and retargeting disconnects them.

## Live color preview and compact native popup

`ColorChanged` now updates the inspector preview and swatch immediately.
`CanvasControl.Invalidate` coalesces preview draws into the next frame. Changes
remain a popup draft until dismissal, when the selected color commits once.
Previewing does not edit source, add undo entries, reload the inspector or write
global Code settings. Programmatic initialization is guarded, and retargeting
or closing clears the draft. Document notifications defer control reloads until
popup closure so rebuilding focusable controls cannot dismiss the native flyout.
Opening a picker also cancels already queued caret following: the property edit
keeps its selected style instead of allowing an earlier caret movement to
retarget the inspector midway through picking.

WinUI offers `ColorChanged` and binding to `Color` for immediate visual updates.
The event also fires for programmatic changes, so initialization needs a guard.
Microsoft's flyout guidance supports committing on dismissal or using explicit
confirmation. These APIs allow a live preview without requiring every sample to
be a saved edit. [ColorChanged API](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.colorpicker.colorchanged)
and [color picker guidance](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/color-picker).

The horizontal picker retains a 256-pixel square spectrum, value/alpha sliders,
RGB/HSV fields and hex/alpha inputs. Its duplicate preview strip is hidden.
The application already loads WinUI's `Compact.xaml` for text and combo controls;
ColorPicker's minimum height and label column also need local resource overrides.
The picker is 538 by 264 layout units including built-in padding, with 12-unit
flyout padding. The flyout presenter has a 600-unit width limit so its default
cap does not clip the horizontal picker or introduce a horizontal scrollbar.
No replacement template or custom color control is used.
[Microsoft's picker template](https://github.com/microsoft/microsoft-ui-xaml/blob/main/controls/dev/ColorPicker/ColorPicker.xaml)
and [dimension resources](https://github.com/microsoft/microsoft-ui-xaml/blob/main/controls/dev/ColorPicker/ColorPicker_themeresources.xaml).

`ShouldConstrainToRootBounds = false` uses native popup hosting so the flyout can
extend beyond the style window. The project's Windows App SDK supports this;
no custom window, focus handling or dismissal implementation is needed.
[WinUI flyout bounds API](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.primitives.flyoutbase.shouldconstraintorootbounds?view=windows-app-sdk-2.0).

## Further alternatives

Live document updates would need a captured view/style/property target, a guard
against initialization callbacks, and a gesture-level undo group. Coalescing
to at most one pending update per display frame would bound formatting work.
Persistence for the global Code sheet should happen once per completed gesture,
with rollback on failure. These are implementation recommendations, not extra
behavior supplied automatically by WinUI.

| Presentation option | Tradeoff |
| --- | --- |
| Simplified flyout with a ring spectrum and slider | Hide channel/hex fields and optional preview to reduce height; fewer controls for exact numeric entry. |
| Text-first popup | Hide the spectrum; retain RGB/HSV, hex and optional alpha controls. Useful for exact colors in a smaller area. |
| Horizontal picker (implemented) | Moves controls beside the spectrum, trading height for width. |
| Palette plus an advanced picker | A custom palette handles common colors; a full picker remains available for arbitrary values. |

The built-in visibility properties cover the spectrum, preview, channel input,
hex input, color slider and alpha controls. Microsoft recommends a square of
at least 256×256 pixels for precise visual picking, or text inputs for refinement.
[Configuration and accuracy guidance](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/color-picker).

`ColorPicker` also supports custom styles/templates, while the lower-level
`ColorSpectrum` can be used to compose a custom interface. Those approaches
require more accessibility, keyboard, focus and maintenance work than hiding
built-in sections. A palette with an Advanced button is a Viem design option,
not a separate stock palette mode of `ColorPicker`.
[ColorPicker API](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.colorpicker),
[ColorSpectrum API](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.primitives.colorspectrum).

## Validation

Windows coverage lives in `StyleInspectorBehaviorTests`: prepared opening size
and initial rendered content fit for document and Code inspectors,
immediate selection, rapid drag suppression, the real half-second timer,
unchanged state, manual choices, mixed selections, view isolation, Code styles,
compact picker dimensions, native popup hosting, live preview, persistence, undo
and retargeting. Tests capture the picker and preview for visual review.
Run `scripts/test-win.ps1`.
Add `-PointerInput` to exercise real spectrum dragging with live preview enabled:
rapid diagonal movement, a held pause and reversal must track the pointer while
the popup geometry and inspector controls remain stable. This briefly moves the
mouse in the isolated test app; the helper checks window ownership and restores
its original position. It explicitly maps XAML coordinates through the owner's
client origin and DPI, since the windowed popup's automation bounds omit that
screen offset. These tests pass at 150% scaling; the reported random movement
has not been reproduced in them.
For just these checks, set `VIEM_TEST_STYLES_ONLY=1` in the test process's
environment before running the script. The color persistence fixtures explicitly
enable HTML style export; generated defaults otherwise remain presentation-only.
macOS policy tests explicitly settle pending work; a separate real-timer test
checks rapid movement, idle scheduling, explicit choices and cancellation.
Run `swift test --filter EVStyleEditorTrackingTests` on macOS.

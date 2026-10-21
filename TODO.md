# TODO

Make rewrapping in markdown mode more format-aware so it doesn’t mess up lists, etc. Make rewapping `gq` work in markdown preview mode.

Style Dialog box (Windows only): Clicking on disabled unchecked controls should check the box and then perform the click action (if applicable). For example, if I click on a color swatch, it should check the box to enable it and then open the color picker. The drop-downs should check the box and open the drop-down. Text entry fields should check the box and then select the full text.

Plain `:global` (without confirmation) still revisits a line that a substitution joined into the line above with an empty replacement. For example, with lines `x1`, `x2`, `x3`, `:g/x/s/x1\n//` moves `x2` into the first line; Vim then skips the `x2` target, but Viem runs the body on that line once more. From the source edit alone this looks the same as deleting the line above, so target rebasing can't tell them apart. Confirmed `:s///c` already drops joined targets using its approved matches (`joined_targets` in `src/core/command/global_confirmation.rs`); plain `:global` needs the substitute's matches passed through its replay the same way.

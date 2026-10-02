#if DEBUG
using Viem.Windows.Shell;
using Viem.Windows.Input;
using Windows.System;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class TablePickerTests
{
    internal static void Run()
    {
        void Check(bool condition, string message) {
            if (!condition) throw new InvalidOperationException(message);
            FrontendSmokeTests.UiChecks.Add(message);
        }
        var state = new TablePickerState();
        Check(KeyPolicy.Route(VirtualKey.F10, false, true, false).Action == NativeAction.ContextMenu
            && KeyPolicy.Route(VirtualKey.Application, false, false, false).Action == NativeAction.ContextMenu,
            "Windows context menu keys expose table actions without moving the caret");
        Check(KeyPolicy.Route(VirtualKey.F10, false, true, false, true).Kind == VIEM_KEY_FUNCTION,
            "literal-next retains Shift+F10 for vi input");
        state.Point(2, 3, false);
        Check(state.SelectedColumns == 3 && state.SelectedRows == 4, "picker counts body rows separately from its header");
        state.Point(15, 20, false);
        Check(!state.HasSelection && state.Columns == 10 && state.Rows == 10, "picker hover cannot grow the grid");
        state.Point(200, 14, true);
        Check(state.SelectedColumns == 20 && state.SelectedRows == 15, "picker clamps the horizontal limit independently");
        state.Point(2, 200, true);
        Check(state.SelectedColumns == 3 && state.SelectedRows == 50, "picker clamps the vertical limit independently");
        state.Point(-1, 200, true);
        Check(!state.HasSelection, "picker release left of grid cancels even below bottom limit");
        state.Point(200, -1, true);
        Check(!state.HasSelection, "picker release above grid cancels even beyond right limit");
        state.Move(0, 0);
        for (int i = 0; i < 100; i++) state.Move(1, 1);
        Check(state.SelectedColumns == 20 && state.SelectedRows == 50, "picker keyboard reaches all supported dimensions");
        for (int i = 0; i < 100; i++) state.Move(-1, -1);
        Check(state.SelectedColumns == 1 && state.SelectedRows == 1 && state.Columns == 20 && state.Rows == 50, "picker retains grown extent when selection shrinks");
    }
}
#endif

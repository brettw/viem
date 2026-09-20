#if DEBUG
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Dispatching;
using Viem.Windows.Core;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class GlobalSelectionOptionTests
{
    internal static void Run(CanvasDevice device, DispatcherQueue dispatcher, Action<bool, string> check)
    {
        string profile = Path.Combine(Path.GetTempPath(), "viem-selection-" + Guid.NewGuid().ToString("N"));
        using var first = new CoreDocument("first"u8.ToArray());
        first.InitializeStartup("set noautoselect keymodel= selectmode=cmd"u8.ToArray());
        GlobalSelectionOptions.Attach(first, profile);
        using var second = new CoreDocument("second"u8.ToArray());
        GlobalSelectionOptions.Attach(second, profile);
        using var other = new CoreDocument("other"u8.ToArray());
        other.InitializeStartup("set keymodel=startsel selectmode=mouse"u8.ToArray());
        GlobalSelectionOptions.Attach(other, profile + "-other");
        check(first.SelectionOption(VIEM_EX_OPTION_KEYMODEL) == "" && second.SelectionOption(VIEM_EX_OPTION_SELECTMODE) == "cmd" && second.SelectionOption(VIEM_EX_OPTION_AUTOSELECT) == "0",
            "global selection options initially inherit startup.viem and reach later document cores");
        using var firstView = new CoreView(first, device, dispatcher, 600, 300);
        using var secondView = new CoreView(second, device, dispatcher, 600, 300);
        using var secondSplit = new CoreView(second, device, dispatcher, 600, 300);
        secondView.Command("v"); secondSplit.Command("i");
        var selected = secondView.LogicalSelection();
        var effects = firstView.SourceLine("set autoselect keymodel=startsel,stopsel selectmode=mouse,key", 1)!;
        GlobalSelectionOptions.Observe(first, effects.SelectionOptions);
        check(second.SelectionOption(VIEM_EX_OPTION_KEYMODEL) == "startsel,stopsel"
            && second.SelectionOption(VIEM_EX_OPTION_SELECTMODE) == "mouse,key" && second.SelectionOption(VIEM_EX_OPTION_AUTOSELECT) == "1", "runtime selection option changes propagate across document cores");
        check(secondView.IsTextSelection && secondView.LogicalSelection().Equals(selected) && secondSplit.Presentation.mode == VIEM_MODE_INSERT,
            "global option propagation preserves inactive selections and Insert mode in split views");
        check(other.SelectionOption(VIEM_EX_OPTION_KEYMODEL) == "startsel" && other.SelectionOption(VIEM_EX_OPTION_SELECTMODE) == "mouse",
            "selection options remain isolated between configuration directories");
        int notifications = 0; second.Changed += () => notifications++;
        GlobalSelectionOptions.Observe(first, effects.SelectionOptions);
        check(notifications == 0, "unchanged global option effects do not recurse or refresh unrelated views");
        using var future = new CoreDocument("future"u8.ToArray());
        future.InitializeStartup("set keymodel= selectmode=cmd"u8.ToArray());
        GlobalSelectionOptions.Attach(future, profile);
        check(future.SelectionOption(VIEM_EX_OPTION_KEYMODEL) == "startsel,stopsel" && future.SelectionOption(VIEM_EX_OPTION_SELECTMODE) == "mouse,key" && future.SelectionOption(VIEM_EX_OPTION_AUTOSELECT) == "1",
            "future documents inherit live global choices over startup defaults");
        using var futureView = new CoreView(future, device, dispatcher, 600, 300);
        futureView.Key(VIEM_KEY_RIGHT, modifiers: VIEM_KEY_MODIFIER_SHIFT);
        check(futureView.IsTextSelection && future.FormattedText() == "future" && second.FormattedText() == "second",
            "propagated options govern actual navigation without source edits");
    }
}
#endif

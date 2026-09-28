#if DEBUG
using System.Globalization;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation.Peers;
using Viem.Windows.Editor;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class StatusFilePathTests
{
    internal static async Task Run(EditorPane pane, EditorWindow window, Preferences preferences, Action<bool, string> check)
    {
        foreach (var (file, cwd, expected) in new (string, string, string)[] {
            (@"C:\work\notes.md", @"C:\work", "notes.md"),
            (@"c:\WORK\drafts\notes.md", @"C:\work\project", @"..\drafts\notes.md"),
            (@"C:\work\drafts\..\café.md", @"C:\work\", "café.md"),
            (@"C:\other\notes.md", @"C:\work", @"C:\other\notes.md"),
            (@"C:\work\notes.md", @"C:\", @"C:\work\notes.md"),
            (@"D:\work\notes.md", @"C:\work", @"D:\work\notes.md"),
            (@"\\server\share\work\notes.md", @"\\SERVER\SHARE\work\drafts", @"..\notes.md"),
            (@"\\server\share\other\notes.md", @"\\server\share\work", @"\\server\share\other\notes.md"),
            (@"\\server\other\notes.md", @"\\server\share\work", @"\\server\other\notes.md"),
            (@"\\server\share\notes.md", @"\\server\share\", @"\\server\share\notes.md"),
        }) check(StatusFilePath.Display(file, cwd) == expected, $"status path relative/root policy: {file} from {cwd}");
        check(StatusFilePath.Display(null, @"C:\work") == "Untitled", "unnamed status path is Untitled");
        check(StatusFilePath.Relative(@"C:\other\notes.md", @"C:\work") == @"..\other\notes.md"
            && StatusFilePath.Relative(@"C:\work\notes.md", @"C:\") == @"work\notes.md",
            "clipboard relative paths can traverse root independently of the status display");
        check(StatusFilePath.Relative(@"D:\notes.md", @"C:\work") == null
            && StatusFilePath.Relative(@"\\server\other\notes.md", @"\\server\share\work") == null,
            "relative clipboard paths are unavailable across drives or UNC shares");
        double Width(string text) => new StringInfo(text).LengthInTextElements;
        check(StatusFilePath.TrimLeft("folder/👩‍💻é.md", 6, Width) == "…👩‍💻é.md", "left trimming preserves Unicode graphemes and filename suffix");
        check(StatusFilePath.TrimLeft("notes.md", 8, Width) == "notes.md"
            && StatusFilePath.TrimLeft("notes.md", 1, Width) == "…"
            && StatusFilePath.TrimLeft("notes.md", 0, Width) == "", "status path fits exact and minimal widths");

        string? originalPath = pane.Document.FilePath;
        bool showStatus = preferences.ShowStatus;
        double originalWidth = pane.FilePathHost.Width;
        var originalAlignment = pane.FilePathHost.HorizontalAlignment;
        var view = pane.View!;
        try
        {
            preferences.Set("appearance", "showStatusBar", true);
            pane.DismissCommandOutput();
            pane.Document.FilePath = Path.Combine(Environment.CurrentDirectory, new string('x', 180), "café-notes.md");
            pane.FilePathHost.Width = 160;
            pane.FilePathHost.HorizontalAlignment = HorizontalAlignment.Left;
            pane.Refresh();
            await Task.Delay(80);
            double x = pane.FilePathControl.TransformToVisual(pane.StatusControl).TransformPoint(new(0, 0)).X;
            check(pane.FilePathControl.Text.StartsWith('…') && pane.FilePathControl.Text.EndsWith("café-notes.md"),
                "narrow native status field trims the left and retains the filename");
            await WindowCapture.Save(window.Hwnd, pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".status-file-path.png");
            await CheckCopyMenu(pane, check);
            foreach (string command in new[] { "v", "V", "i", "R" })
            {
                view.Command(command);
                await Task.Delay(30);
                pane.StatusControl.UpdateLayout();
                double changedX = pane.FilePathControl.TransformToVisual(pane.StatusControl).TransformPoint(new(0, 0)).X;
                check(Math.Abs(x - changedX) < 0.5, $"status filename stays aligned in {pane.ModeControl.Text} mode");
                var measure = new TextBlock { Text = pane.ModeControl.Text, FontFamily = pane.ModeControl.FontFamily, FontSize = pane.ModeControl.FontSize };
                measure.Measure(new(double.PositiveInfinity, double.PositiveInfinity));
                check(pane.ModeSlotWidth >= measure.DesiredSize.Width,
                    $"status mode fits {pane.ModeControl.Text}: slot={pane.ModeSlotWidth}, needed={measure.DesiredSize.Width}, reserved={pane.ModeControl.Width}");
                view.Key(VIEM_KEY_ESCAPE);
            }
        }
        finally
        {
            view.Key(VIEM_KEY_ESCAPE);
            pane.Document.FilePath = originalPath;
            pane.FilePathHost.Width = originalWidth;
            pane.FilePathHost.HorizontalAlignment = originalAlignment;
            preferences.Set("appearance", "showStatusBar", showStatus);
            pane.Refresh();
        }
    }

    private static async Task CheckCopyMenu(EditorPane pane, Action<bool, string> check)
    {
        var menu = pane.FilePathControl.ContextFlyout as MenuFlyout
            ?? throw new InvalidOperationException("Filename has no context menu.");
        var items = menu.Items.OfType<MenuFlyoutItem>().ToArray();
        check(items.Select(item => item.Text).SequenceEqual(new[] { "Copy full path", "Copy relative path" }),
            "filename context menu exposes full and relative path actions");
        string? original = pane.Document.FilePath;
        string? copied = null;
        var before = pane.View!.Presentation;
        pane.FilePathClipboardWriterForTesting = value => copied = value;
        async Task Open()
        {
            var ready = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            void Opened(object? sender, object args) => ready.TrySetResult();
            menu.Opened += Opened;
            try { menu.ShowAt(pane.FilePathControl); await ready.Task.WaitAsync(TimeSpan.FromSeconds(5)); }
            finally { menu.Opened -= Opened; }
        }
        async Task Invoke(int index)
        {
            await Open();
            check(items[index].IsEnabled, "named file path action is enabled");
            await Close(() => new MenuFlyoutItemAutomationPeer(items[index]).Invoke());
        }
        async Task Close(Action? invoke = null)
        {
            var ready = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            void Closed(object? sender, object args) => ready.TrySetResult();
            menu.Closed += Closed;
            try
            {
                invoke?.Invoke();
                menu.Hide();
                await ready.Task.WaitAsync(TimeSpan.FromSeconds(5));
            }
            finally { menu.Closed -= Closed; }
        }
        try
        {
            await Invoke(0);
            check(copied == Path.GetFullPath(original!), "full path action copies the entire untruncated filename");
            await Invoke(1);
            check(copied == Path.GetRelativePath(Environment.CurrentDirectory, original!),
                "relative path action copies the entire cwd-relative filename");
            await Open();
            pane.Document.FilePath = Path.Combine(Environment.CurrentDirectory, "renamed café.md");
            await Close(() => new MenuFlyoutItemAutomationPeer(items[1]).Invoke());
            check(copied == "renamed café.md", "path action resolves the current binding when invoked");
            pane.Document.FilePath = null;
            await Open();
            check(items.All(item => !item.IsEnabled), "unnamed documents disable both path actions");
            await Close();
            string drive = Path.GetPathRoot(Environment.CurrentDirectory)!.StartsWith("C:", StringComparison.OrdinalIgnoreCase) ? "D:" : "C:";
            pane.Document.FilePath = drive + @"\notes.md";
            await Open();
            check(items[0].IsEnabled && !items[1].IsEnabled, "different-drive files allow only full path copying");
            var after = pane.View.Presentation;
            check(after.document_revision == before.document_revision && after.mode == before.mode
                && after.cursor_utf8_offset == before.cursor_utf8_offset, "copying paths leaves document and cursor state unchanged");
        }
        finally
        {
            menu.Hide(); pane.FilePathClipboardWriterForTesting = null;
            pane.Document.FilePath = original; pane.Refresh(); pane.FocusEditor();
        }
    }
}
#endif

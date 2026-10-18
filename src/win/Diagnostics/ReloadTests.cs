#if DEBUG
using System.Text;
using Viem.Windows.Editor;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private static unsafe ViemViewRestorationV1 CaptureReloadPosition(EditorPane pane)
    {
        var state = New<ViemViewRestorationV1>();
        Check(viem_core_view_capture_restoration(pane.Document.Handle, pane.View!.Id, &state), "Capture reload test");
        return state;
    }
    private async Task TestReloadPositions()
    {
        // Full diagnostics already have split views. Use two fixture panes so
        // the empty row and padding fit, making a zero scroll origin meaningful.
        var window = NewWindow(null);
        try
        {
            await window.ActivePane!.Ready;
            await window.RunReloadPositionChecks();
        }
        finally { window.closing = true; window.Close(); Activate(); ActivePane?.FocusEditor(); }
    }
    private async Task RunReloadPositionChecks()
    {
        string path = Path.Combine(preferences.DirectoryPath, "reload-positions.txt");
        string source = string.Join("\n", Enumerable.Range(0, 140).Select(i => $"line {i:000} has some content"));
        await File.WriteAllTextAsync(path, source);
        var document = NewDocument(Encoding.UTF8.GetBytes(source), path, VIEM_FORMAT_PLAIN_TEXT);
        var blank = ActivePane!;
        var first = AddPane(document); await first.Ready;
        RemovePane(blank);
        var second = AddPane(document); await second.Ready;
        paneGrid.UpdateLayout();
        first.View!.GoToLine(100); first.View.Command("llll");
        second.View!.GoToLine(110); second.View.Command("llllll");
        first.View.Scroll(0, 307); second.View.Scroll(0, 609);
        SetActive(first); first.FocusEditor();
        var before = new[] { CaptureReloadPosition(first), CaptureReloadPosition(second) };
        var queuedQuit = first.View.ExecuteCommandLine("q")
            ?? throw new InvalidOperationException("Could not capture the pre-reload quit command.");
        if (!queuedQuit.Requests.Any(request => request.Value.kind == VIEM_EX_FRONTEND_QUIT))
            throw new InvalidOperationException("The captured command did not contain a quit request.");
        await File.WriteAllTextAsync(path, source.Replace("line 000", "the first line has grown"));
        await Reload(first, true);
        if (ActivePane != first || first.Document != second.Document || first.Document == document
            || await first.Ready != first.View) throw new InvalidOperationException("Reload lost pane ownership or focus.");
        await ApplyEffects(first, queuedQuit);
        if (!Panes.Contains(first) || !Panes.Contains(second) || first.Document != second.Document)
            throw new InvalidOperationException("A pre-reload quit command closed a replacement document's pane.");
        Diagnostics.FrontendSmokeTests.UiChecks.Add("reload rejects queued commands belonging to the previous document");
        foreach (var (pane, expected) in new[] { (first, before[0]), (second, before[1]) })
        {
            var after = CaptureReloadPosition(pane);
            if (after.cursor_line != expected.cursor_line || after.cursor_column != expected.cursor_column
                || after.viewport_line != expected.viewport_line || Math.Abs(after.row_fraction - expected.row_fraction) > 0.001)
                throw new InvalidOperationException("Reload did not preserve independent cursor/scroll positions.");
        }
        await File.WriteAllTextAsync(path, "é👩‍💻");
        await Reload(first, true);
        foreach (var pane in new[] { first, second })
            if (pane.View!.Presentation.cursor_utf8_offset != (ulong)Encoding.UTF8.GetByteCount("é"))
                throw new InvalidOperationException("Reload did not clamp to the final Normal-mode grapheme.");
        await File.WriteAllTextAsync(path, "");
        await Reload(first, true);
        foreach (var pane in new[] { first, second })
        {
            var layout = pane.View!.LayoutInfo();
            if (layout.total_height > layout.viewport_height)
                throw new InvalidOperationException($"Reload fixture must fit its empty row and padding: content={layout.total_height:R}, viewport={layout.viewport_height:R}.");
            if (pane.View!.Presentation.cursor_utf8_offset != 0 || pane.View.Viewport.top != 0)
            {
                var viewport = pane.View.Viewport;
                var restored = CaptureReloadPosition(pane);
                throw new InvalidOperationException($"Empty reload did not clamp cursor/scroll positions: pane={(pane == first ? "first" : "second")}, cursor={pane.View.Presentation.cursor_utf8_offset}, top={viewport.top:R}, maximumTop={viewport.maximum_top:R}, canvasHeight={pane.Canvas.ActualHeight:R}, viewportHeight={layout.viewport_height:R}, totalHeight={layout.total_height:R}, insets={layout.content_insets.top:R}/{layout.content_insets.bottom:R}, restoredLine={restored.viewport_line}, rowFraction={restored.row_fraction:R}, sourceBytes={pane.Document.State.source_byte_count}, error={pane.LastError}.");
            }
        }
        Diagnostics.FrontendSmokeTests.UiChecks.Add("file reload preserves each pane's cursor and scroll and clamps shortened/empty content");
    }
}
#endif

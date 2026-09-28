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
        string path = Path.Combine(preferences.DirectoryPath, "reload-positions.txt");
        string source = string.Join("\n", Enumerable.Range(0, 140).Select(i => $"line {i:000} has some content"));
        await File.WriteAllTextAsync(path, source);
        var document = NewDocument(Encoding.UTF8.GetBytes(source), path, VIEM_FORMAT_PLAIN_TEXT);
        var first = AddPane(document); await first.Ready;
        var second = AddPane(document); await second.Ready;
        first.View!.GoToLine(100); first.View.Command("llll");
        second.View!.GoToLine(110); second.View.Command("llllll");
        first.View.Scroll(0, 307); second.View.Scroll(0, 609);
        SetActive(first); first.FocusEditor();
        var before = new[] { CaptureReloadPosition(first), CaptureReloadPosition(second) };
        var queuedQuit = first.View.SourceLine("q", 1)
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
            if (pane.View!.Presentation.cursor_utf8_offset != 0 || pane.View.Viewport.top != 0)
                throw new InvalidOperationException("Empty reload did not clamp cursor/scroll positions.");
        RemovePane(second); RemovePane(first);
        Diagnostics.FrontendSmokeTests.UiChecks.Add("file reload preserves each pane's cursor and scroll and clamps shortened/empty content");
    }
}
#endif

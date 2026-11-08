#if DEBUG
using System.Diagnostics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Diagnostics;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private static async Task TestDiffMode(Preferences preferences)
    {
        static void Check(bool condition, string name) {
            if (!condition) throw new InvalidOperationException(name);
            FrontendSmokeTests.UiChecks.Add(name);
        }
        static async Task WaitFor(Func<bool> predicate) {
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            while (!predicate()) await Task.Delay(25, timeout.Token);
        }
        string directory = Path.Combine(preferences.DirectoryPath, "diff " + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        preferences = new Preferences(Path.Combine(directory, "profile"));
        async Task Git(params string[] arguments) {
            var start = new ProcessStartInfo("git") { WorkingDirectory = directory, UseShellExecute = false,
                CreateNoWindow = true, RedirectStandardError = true };
            foreach (var argument in arguments) start.ArgumentList.Add(argument);
            using var process = Process.Start(start)!;
            string error = await process.StandardError.ReadToEndAsync();
            await process.WaitForExitAsync();
            if (process.ExitCode != 0) throw new InvalidOperationException(error);
        }
        try {
            string original = string.Join("\n", Enumerable.Range(1, 110).Select(n => $"Line {n}")) + "\n";
            await using (var editor = await TestEditor.Open(preferences, original)) {
                var window = editor.Window;
                var menu = window.Menu.Items.Single(m => m.Title == "Diff");
                var mode = (ToggleMenuFlyoutItem)menu.Items[0];
                Check(mode.Text == "Diff Mode" && window.Menu.Items[window.Menu.Items.IndexOf(menu) + 1].Title == "View", "Diff Mode heads the Diff menu immediately before View");
                var peer = new MenuBarItemAutomationPeer(menu);
                await MenuTestHelpers.Transition(true, () => { peer.Expand(); return Task.CompletedTask; }, mode);
                mode.Focus(FocusState.Programmatic);
                await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
                await WaitFor(() => window.comparison != null);
                var left = window.diffLeft!;
                Check(window.diffBase.Kind == VIEM_DIFF_BASE_LAST_SAVED && editor.Document.State.format == VIEM_FORMAT_CODE
                    && left.Document.Source(left.Document.State.document_revision).AsSpan().SequenceEqual(System.Text.Encoding.UTF8.GetBytes(original)),
                    "Diff menu opens an untitled Markdown document beside its saved source in Code");
                Check(preferences.MarkdownFormattedView, "Comparing Markdown preserves the opening preference for other files");
                var source = left.Document.Source(left.Document.State.document_revision);
                try { left.View!.Ex("1delete"); } catch (InvalidOperationException) { }
                Check(left.Document.Source(left.Document.State.document_revision).SequenceEqual(source) && !left.Document.IsDirty,
                    "The diff base rejects a line deletion and stays clean");
                editor.View.Command("i"); editor.View.Text("changed "); editor.View.Key(VIEM_KEY_ESCAPE);
                Check(window.comparison!.Marks(true).Marks.Length > 0 && window.comparison.Marks(false).Marks.Length > 0,
                    "Typing updates both sides' change marks");
                window.NavigateChange(true);
                Check(editor.View.Layout().Decorations.Any(d => (d.flags & VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND) != 0),
                    "The live layout paints changed lines after navigating");
                var toolbar = window.diffToolbar!;
                var align = toolbar.Align;
                var markdownToggle = window.Toolbar.FormattedView;
                Check(toolbar.ActualHeight == window.Toolbar.Height && align.ActualWidth == markdownToggle.Width
                    && align.ActualHeight == markdownToggle.Height && align.Content is Canvas
                    && AutomationProperties.GetName(align) == "Align Sides" && Equals(ToolTipService.GetToolTip(align), "Align Sides"),
                    "Diff uses Markdown toolbar dimensions and exposes the alignment icon as a named toggle");
                window.SetDiffAlignment(true);
                editor.View.Scroll(0, 300);
                Check(left.View!.Viewport.top > 0, "Scrolling the aligned live side moves the base");
                Check(align.IsChecked == true && align.Focus(FocusState.Programmatic), "Align Sides reflects the menu state and takes keyboard focus");
                await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
                await WaitFor(() => align.IsChecked == false && !window.diffAligned);
                Check(!preferences.Get("diff", "alignSides", true), "The alignment icon toggles and persists the setting from the keyboard");
                float top = left.View.Viewport.top;
                editor.View.Scroll(0, 500);
                Check(left.View.Viewport.top == top, "Unaligned diff panes scroll independently");
                window.SetLineNumbers(false);
                Check(!left.View.NumberEnabled && !editor.View.NumberEnabled, "The diff line-number choice reaches both views");
                editor.View.Undo();
                Check(editor.Source == original && window.comparison.Marks(true).Marks.Length == 0, "Undo restores the exact source and clears diff marks");
                await window.SetDiffMode(false);
                Check(editor.Document.State.format == VIEM_FORMAT_MARKDOWN && editor.Source == original && window.diffLeft == null,
                    "Leaving the comparison restores Markdown Preview and removes the base");
            }

            await Git("init", "-q", "-b", "main");
            string scoped = Path.Combine(directory, "sub");
            Directory.CreateDirectory(scoped);
            string a = Path.Combine(scoped, "a"), b = Path.Combine(scoped, "b");
            await File.WriteAllTextAsync(a, "old a"); await File.WriteAllTextAsync(b, "old b");
            await File.WriteAllTextAsync(Path.Combine(directory, "outside"), "old");
            await Git("add", "sub", "outside");
            await Git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false", "commit", "-qm", "base");
            await File.WriteAllTextAsync(a, "index a"); await Git("add", "sub/a");
            await File.WriteAllTextAsync(a, "live a"); File.Delete(b);
            await File.WriteAllTextAsync(Path.Combine(directory, "outside"), "modified");
            await using var empty = await TestEditor.Open(preferences, "", VIEM_FORMAT_PLAIN_TEXT);
            var folder = empty.Window;
            await folder.DiffFolder(scoped);
            Check(folder.diffFiles.Length == 2 && SamePath(folder.ActivePane!.Document.FilePath!, a)
                && folder.diffLeft?.Document.FormattedText() == "index a", "Folder diff reuses a pristine window, scopes the file list, and reads the Git index");
            await WindowCapture.SaveElement(folder.diffToolbar!, folder.ActivePane!.Canvas.Device, FrontendSmokeTests.ReportPath + ".diff-toolbar.png");
            await folder.ChooseDiffBase(new(VIEM_DIFF_BASE_HEAD));
            Check(folder.diffLeft?.Document.FormattedText() == "old a" && folder.diffFiles.Length == 2,
                "HEAD uses committed bytes and retains the folder scope");
            await folder.ChooseDiffBase(new(VIEM_DIFF_BASE_BRANCH, "main"));
            Check(folder.diffLeft?.Document.FormattedText() == "old a", "A branch base reads its tip");
            var first = folder.ActivePane!;
            first.View!.Command("i"); first.View.Text("edit "); first.View.Key(VIEM_KEY_ESCAPE);
            folder.CloseReviewDecisionForTesting = _ => Task.FromResult(ContentDialogResult.None);
            await folder.MoveDiffFile(1);
            Check(folder.ActivePane == first && first.Document.IsDirty, "Cancelling review keeps the unsaved file and comparison");
            folder.CloseReviewDecisionForTesting = _ => Task.FromResult(ContentDialogResult.Secondary);
            await folder.MoveDiffFile(1);
            await WaitFor(() => folder.comparison != null);
            Check(SamePath(folder.ActivePane!.Document.FilePath!, b) && folder.deletedSide && folder.diffLeft!.Document.FormattedText() == "old b",
                "Next File opens a deleted file with a protected empty live side");
            await folder.MoveDiffFile(1);
            await WaitFor(() => folder.comparison != null);
            Check(SamePath(folder.ActivePane!.Document.FilePath!, a), "Next File wraps from the last file to the first");
            string chosen = Path.Combine(directory, ".basis");
            await File.WriteAllTextAsync(chosen, "chosen");
            await folder.ChooseDiffBase(new(VIEM_DIFF_BASE_FILE, chosen));
            Check(folder.diffFiles.Length == 0 && folder.diffLeft!.Document.FormattedText() == "chosen", "An explicit base accepts a dotfile and clears file navigation");
            await File.WriteAllTextAsync(chosen, "refreshed");
            await folder.RefreshDiff();
            Check(folder.diffLeft!.Document.FormattedText() == "refreshed", "Refresh reloads the chosen file from disk");
            try { await folder.ChooseDiffBase(new(VIEM_DIFF_BASE_FILE, scoped)); }
            catch (IOException) { }
            Check(folder.diffBase.Argument == chosen && folder.comparison != null && folder.diffLeft.Document.FormattedText() == "refreshed",
                "A failed base load retains the current file and comparison");
            var pending = folder.ChooseDiffBase(new(VIEM_DIFF_BASE_HEAD));
            await folder.SetDiffMode(false);
            await pending;
            Check(folder.diffLeft == null && !folder.diffEnabled, "A pending Git load cannot reopen a closed comparison");
        } finally {
            foreach (var file in Directory.EnumerateFiles(directory, "*", SearchOption.AllDirectories)) File.SetAttributes(file, FileAttributes.Normal);
            Directory.Delete(directory, recursive: true);
        }
    }
}
#endif

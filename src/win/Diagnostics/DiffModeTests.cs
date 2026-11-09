#if DEBUG
using System.Diagnostics;
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Viem.Windows.Diagnostics;
using Viem.Windows.Editor;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private static void CheckDiff(bool condition, string name)
    {
        if (!condition) throw new InvalidOperationException(name);
        FrontendSmokeTests.UiChecks.Add(name);
    }

    private static async Task TestCollapsedDiff(Preferences preferences)
    {
        var profile = new Preferences(Path.Combine(preferences.DirectoryPath, "collapsed-diff"));
        var lines = Enumerable.Range(0, 40).Select(n => $"line {n}").ToList();
        string basePath = Path.Combine(profile.DirectoryPath, "base.txt");
        await File.WriteAllTextAsync(basePath, string.Join("\n", lines));
        lines[30] = "replaced";
        lines.Insert(10, "inserted");
        string source = string.Join("\n", lines);
        await using var editor = await TestEditor.Open(profile, source, VIEM_FORMAT_CODE);
        var window = editor.Window;
        await window.SetDiffMode(true);
        await window.ChooseDiffBase(new(VIEM_DIFF_BASE_FILE, basePath));
        var left = window.diffLeft!;
        var live = editor.Pane;
        var button = window.diffToolbar!.Collapse;
        const uint band = VIEM_LAYOUT_DECORATION_COLLAPSED_CONTEXT | VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND;
        static bool Shown(EditorPane pane, ulong line) => pane.View!.Layout().Rows.Any(r => r.hard_line_index == line);
        static bool HasDivider(EditorPane pane) => pane.View!.Layout().Decorations.Any(d => (d.flags & band) == band);
        async Task ClickMiddleDivider(EditorPane pane, ulong ownerLine) {
            var layout = pane.View!.Layout();
            var row = layout.Rows.Single(r => r.hard_line_index == ownerLine);
            var box = layout.Decorations.Single(d => d.row_index == row.row_index && (d.flags & band) == band).typographic_bounds;
            var viewport = pane.View.Viewport;
            var point = new global::Windows.Foundation.Point((box.x + box.width / 2 - viewport.left) / pane.Canvas.ActualWidth,
                (box.y + box.height / 2 - viewport.top) / pane.Canvas.ActualHeight);
            await InputRoutingTests.Drag(window, pane.Canvas, new[] { point }, _ => { }, focusTarget: false);
        }

        CheckDiff(button.IsChecked == true && AutomationProperties.GetName(button) == "Collapse Context"
            && Equals(ToolTipService.GetToolTip(button), "Collapse Context"), "Collapse Context starts checked and names its toolbar icon");
        var face = (ContentPresenter)VisualTreeHelper.GetChild(button, 0);
        var checkedFill = ((SolidColorBrush)face.Background).Color;
        var glyph = (Microsoft.UI.Xaml.Shapes.Path)((Canvas)button.Content).Children.Single();
        CheckDiff(glyph.Stroke == face.Foreground && checkedFill.A == 255
            && checkedFill != ((SolidColorBrush)face.Foreground).Color, "Checked Collapse Context has a solid fill and contrasting icon");
        await WindowCapture.SaveElement(button, live.Canvas.Device, FrontendSmokeTests.ReportPath + ".collapse-on.png");
        CheckDiff(Shown(left, 7) && Shown(live, 7) && !Shown(left, 0) && !Shown(live, 0)
            && Shown(left, 12) && Shown(live, 13) && !Shown(left, 14) && !Shown(live, 15),
            "The initial diff hides leading and middle context on both sides of an insertion");
        foreach (var (pane, line) in new[] { (left, 12UL), (live, 13UL) }) {
            if (pane == live) pane.View!.Command("i");
            ulong caret = pane.View!.Presentation.cursor_utf8_offset;
            uint mode = pane.View.Presentation.mode;
            await ClickMiddleDivider(pane, line);
            CheckDiff(Shown(left, 14) && Shown(live, 15) && !Shown(left, 0) && !Shown(live, 0)
                && pane.View.Presentation.cursor_utf8_offset == caret && pane.View.Presentation.mode == mode && editor.Source == source,
                $"Clicking the {(pane == left ? "base" : "live")} divider opens only its region on both sides and preserves the caret, mode and source");
            if (pane == live) pane.View.Key(VIEM_KEY_ESCAPE);
            await window.RefreshDiff();
            CheckDiff(!Shown(left, 14) && !Shown(live, 15), "Refreshing the base closes manually opened context");
        }
        window.SetDiffAlignment(false);
        left.View!.Ex("16");
        CheckDiff(Shown(left, 15) && Shown(live, 16), "A base caret entering hidden context opens the live side while unaligned");
        live.View!.Command("gg");
        CheckDiff(Shown(left, 0) && Shown(live, 0), "A live caret entering hidden context opens the base side while unaligned");
        var menu = window.Menu.Items.Single(m => m.Title == "Diff");
        var item = menu.Items.OfType<ToggleMenuFlyoutItem>().Single(i => i.Text == "Collapse Context");
        var peer = new MenuBarItemAutomationPeer(menu);
        await MenuTestHelpers.Transition(true, () => { peer.Expand(); return Task.CompletedTask; }, item);
        CheckDiff(item.IsChecked && item.Focus(FocusState.Programmatic), "The Diff menu reflects the collapse toggle");
        await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
        CheckDiff(button.IsChecked == false && !HasDivider(left) && !HasDivider(live)
            && !new Preferences(profile.DirectoryPath).Get("diff", "collapseContext", true), "The menu expands both sides and saves the choice");
        CheckDiff(((SolidColorBrush)face.Background).Color != checkedFill && glyph.Stroke == face.Foreground
            && ((SolidColorBrush)glyph.Stroke).Color.A != 0, "Turning collapse off keeps the icon visible on the neutral background");
        await WindowCapture.SaveElement(button, live.Canvas.Device, FrontendSmokeTests.ReportPath + ".collapse-off.png");
        await window.SetDiffMode(false);
        await window.SetDiffMode(true);
        await window.ChooseDiffBase(new(VIEM_DIFF_BASE_FILE, basePath));
        CheckDiff(button.IsChecked == false && !HasDivider(window.diffLeft!) && Shown(live, 15),
            "A new comparison starts expanded after saving that choice");
        button.Focus(FocusState.Programmatic);
        await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
        CheckDiff(button.IsChecked == true && HasDivider(window.diffLeft!) && HasDivider(live)
            && Shown(live, 0) && !Shown(live, 15), "The toolbar recollapses context while keeping the live caret visible");
    }

    private static async Task TestAlignedDiffScroll(Preferences preferences)
    {
        var profile = new Preferences(Path.Combine(preferences.DirectoryPath, "aligned-scroll"));
        string basePath = Path.Combine(profile.DirectoryPath, "short.md");
        await File.WriteAllTextAsync(basePath, "common\nshort");
        string live = "common\n" + string.Join("\n", Enumerable.Range(1, 96)
            .Select(n => $"{n} " + string.Concat(Enumerable.Repeat("a long wrapped addition ", 12))));
        await using var editor = await TestEditor.Open(profile, live, VIEM_FORMAT_CODE);
        var window = editor.Window;
        await window.SetDiffMode(true);
        await window.ChooseDiffBase(new(VIEM_DIFF_BASE_FILE, basePath));
        window.SetDiffAlignment(true);
        var left = window.diffLeft!;
        var right = editor.Pane;
        right.SetWordWrap(true);
        window.root.UpdateLayout();
        await InputRoutingTests.Settle();
        void CheckPair(double fraction, EditorPane driver) {
            double maximum = driver.VerticalScrollControl.Maximum;
            CheckDiff(maximum > driver.Canvas.ActualHeight, "The aligned track includes the long addition");
            driver.ScrollVerticallyFromScrollbar(fraction * maximum);
            var a = left.View!.Viewport;
            var b = right.View!.Viewport;
            double expected = fraction == 1 ? a.maximum_top : Math.Min(fraction * maximum, a.maximum_top);
            CheckDiff(Math.Abs(a.top - expected) < .1 && a.top == b.top && a.maximum_top == b.maximum_top
                && left.VerticalScrollControl.Maximum == a.maximum_top && right.VerticalScrollControl.Maximum == b.maximum_top
                && left.VerticalScrollControl.Value == a.top && right.VerticalScrollControl.Value == b.top,
                $"Dragging the {(driver == left ? "base" : "live")} thumb to {fraction:P0} synchronizes both ranges, origins and thumbs"
                + $" (expected {expected:F1}, base {a.top:F1}/{a.maximum_top:F1}, live {b.top:F1}/{b.maximum_top:F1},"
                + $" thumbs {left.VerticalScrollControl.Value:F1}/{left.VerticalScrollControl.Maximum:F1}"
                + $" and {right.VerticalScrollControl.Value:F1}/{right.VerticalScrollControl.Maximum:F1})");
        }
        CheckPair(.55, left);
        CheckPair(.72, right);
        CheckPair(1, left);
        CheckPair(.95, left);
        CheckPair(.4, right);
        CheckPair(0, left);
        window.SetDiffAlignment(false);
        CheckDiff(left.VerticalScrollControl.Maximum == 0 && right.VerticalScrollControl.Maximum > right.Canvas.ActualHeight,
            "Disabling alignment restores the short and long files' independent scroll ranges");
    }

    private static async Task TestDiffMode(Preferences preferences)
    {
        static void Check(bool condition, string name) => CheckDiff(condition, name);
        static async Task WaitFor(Func<bool> predicate) {
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            while (!predicate()) await Task.Delay(25, timeout.Token);
        }
        string directory = Path.Combine(preferences.DirectoryPath, "diff " + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        preferences = new Preferences(Path.Combine(directory, "profile"));
        var placement = new DocumentWindowPlacement(preferences);
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
            await using (var editor = await TestEditor.Open(preferences, original, placement: placement)) {
                var window = editor.Window;
                var menu = window.Menu.Items.Single(m => m.Title == "Diff");
                var mode = (ToggleMenuFlyoutItem)menu.Items[0];
                Check(mode.Text == "Diff Mode" && window.Menu.Items[window.Menu.Items.IndexOf(menu) - 1].Title == "File", "Diff Mode heads the Diff menu immediately after File");
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
                await WaitFor(() => left.ActualWidth > 0 && window.diffSplitter?.ActualWidth > 0);
                var splitter = window.diffSplitter!;
                double available = window.root.ActualWidth - DiffSplitterWidth;
                bool Near(double actual, double expected) => Math.Abs(actual - expected) < 1;
                Check(Grid.GetColumn(splitter) == 1 && Near(splitter.ActualWidth, DiffSplitterWidth) && Near(left.ActualWidth, available / 2)
                    && AutomationProperties.GetName(splitter) == "Resize views horizontally",
                    "The pane splitter divides the two sides of a diff evenly");
                window.DragDiffSplitter(60); window.root.UpdateLayout();
                Check(Near(left.ActualWidth, available / 2 + 60), "Dragging the diff splitter moves the boundary between the sides");
                window.DragDiffSplitter(-available); window.root.UpdateLayout();
                Check(Near(left.ActualWidth, DiffMinimumWidth), "The diff splitter stops at the base side's minimum width");
                window.DragDiffSplitter(40); window.root.UpdateLayout();
                Check(Near(left.ActualWidth, DiffMinimumWidth + 40), "Reversing a blocked diff splitter drag moves the boundary at once");
                window.diffSplitFraction = .5; window.ApplyDiffSplit(); window.root.UpdateLayout();
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
                Check(toolbar.AddedLines.Text == "+1" && toolbar.RemovedLines.Text == "–1"
                    && ((UIElement)toolbar.FileCount.Parent).Visibility == Visibility.Collapsed,
                    "A live single-file edit shows one addition and one removal without a file count");
                var align = toolbar.Align;
                var markdownToggle = window.Toolbar.FormattedView;
                Check(toolbar.ActualHeight == window.Toolbar.Height && align.ActualWidth == markdownToggle.Width
                    && align.ActualHeight == markdownToggle.Height && align.Content is Canvas
                    && AutomationProperties.GetName(align) == "Align Sides" && Equals(ToolTipService.GetToolTip(align), "Align Sides"),
                    "Diff uses Markdown toolbar dimensions and exposes the alignment icon as a named toggle");
                window.SetDiffAlignment(true);
                // The scroll tests need the unchanged lines to extend beyond the viewport.
                window.SetDiffCollapsed(false);
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
                Check(toolbar.AddedLines.Text == "+0" && toolbar.RemovedLines.Text == "–0", "Undo restores the live line totals");
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
            await using var empty = await TestEditor.Open(preferences, "", VIEM_FORMAT_PLAIN_TEXT, placement);
            var folder = empty.Window;
            await folder.OpenArgumentsForTesting(new(["--diff"], scoped));
            Check(folder.diffFiles.Length == 2 && SamePath(folder.ActivePane!.Document.FilePath!, a)
                && folder.diffLeft?.Document.FormattedText() == "index a", "Folder diff reuses a pristine window, scopes the file list, and reads the Git index");
            var folderToolbar = folder.diffToolbar!;
            Check(folderToolbar.FileCount.Text == "2" && ((UIElement)folderToolbar.FileCount.Parent).Visibility == Visibility.Visible
                && folderToolbar.AddedLines.Text == "+1" && folderToolbar.RemovedLines.Text == "–2",
                "Bare --diff shows totals for all changes under the launch working directory");
            folder.SetLineNumbers(true);
            foreach (var (pane, underlay, styleId) in new[] {
                (folder.diffLeft!, VIEM_LINE_NUMBER_DIFF_REMOVED, "Builtin diff removed line"),
                (folder.ActivePane!, VIEM_LINE_NUMBER_DIFF_ADDED, "Builtin diff added line") }) {
                await WaitFor(() => pane.DrawnLineNumbers.Any(n => n.Line == 1));
                var lineStyle = ChromeStyle.Resolve(pane.View!.Styles(), styleId,
                    ChromeStyle.Fallback(preferences.Theme.Foreground, preferences.Theme.Background));
                var band = pane.LineNumbersForTesting!.Regions[0];
                int y = (int)((band.top + band.bottom) / 2 - pane.LineNumbersForTesting.Info.viewport.y);
                using var target = new CanvasRenderTarget(pane.Canvas.Device, (float)pane.GutterWidth, y + 2, 96);
                using (var drawing = target.CreateDrawingSession()) pane.DrawLineNumbersForTesting(drawing);
                byte[] pixel = target.GetPixelBytes(1, y, 1, 1);
                var color = lineStyle.Background;
                Check((band.flags & underlay) != 0 && pixel[3] == color.A && color.A > 0
                    && Math.Abs(pixel[2] - color.R * color.A / 255f) <= 1 && Math.Abs(pixel[1] - color.G * color.A / 255f) <= 1
                    && Math.Abs(pixel[0] - color.B * color.A / 255f) <= 1,
                    $"{styleId} colors the displayed number on its side of the comparison");
            }
            var priorWindows = App.Instance.Windows.ToHashSet();
            try {
                string? launchError = null;
                try { await folder.OpenArgumentsForTesting(new(["--diff", ".git", "outside", "sub/a", "sub", "."], directory)); }
                catch (IOException error) { launchError = error.Message; }
                Check(launchError?.Contains(Path.Combine(directory, ".git")) == true,
                    "A directory outside the working tree reports its path while the other diff arguments open");
                var opened = App.Instance.Windows.Where(w => !priorWindows.Contains(w)).ToArray();
                var fileWindows = opened.Where(w => w.singleFileDiff).ToArray();
                Check(opened.Length == 4 && opened.All(w => w.comparison != null)
                    && fileWindows.Length == 2 && fileWindows.All(w => w.diffFiles.Length == 1
                        && ((UIElement)w.diffToolbar!.FileCount.Parent).Visibility == Visibility.Collapsed
                        && w.diffToolbar.AddedLines.Text == "+1" && w.diffToolbar.RemovedLines.Text == "–1")
                    && fileWindows.Any(w => SamePath(w.diffFiles[0].Path, a))
                    && fileWindows.Any(w => SamePath(w.diffFiles[0].Path, Path.Combine(directory, "outside")))
                    && opened.Any(w => w.diffScope != null && SamePath(w.diffScope, scoped) && w.diffFiles.Length == 2)
                    && opened.Any(w => w.diffScope != null && SamePath(w.diffScope, directory) && w.diffFiles.Length == 3),
                    "Mixed --diff arguments open two single-file windows and two independently scoped folder windows");
            } finally {
                foreach (var w in App.Instance.Windows.Where(w => !priorWindows.Contains(w)).ToArray())
                    await w.ClosePane(w.ActivePane!, force: true);
                folder.Activate();
            }
            await WindowCapture.SaveElement(folder.diffToolbar!, folder.ActivePane!.Canvas.Device, FrontendSmokeTests.ReportPath + ".diff-toolbar.png");
            await folder.ChooseDiffBase(new(VIEM_DIFF_BASE_HEAD));
            Check(folder.diffLeft?.Document.FormattedText() == "old a" && folder.diffFiles.Length == 2,
                "HEAD uses committed bytes and retains the folder scope");
            await folder.ChooseDiffBase(new(VIEM_DIFF_BASE_BRANCH, "main"));
            Check(folder.diffLeft?.Document.FormattedText() == "old a", "A branch base reads its tip");
            var bases = FormattingToolbarTests.Descendants<ComboBox>(folderToolbar).Single(box => AutomationProperties.GetName(box) == "Diff base");
            var files = FormattingToolbarTests.Descendants<ComboBox>(folderToolbar).Single(box => AutomationProperties.GetName(box) == "Files in this diff");
            var arrows = FormattingToolbarTests.Descendants<Button>(folderToolbar)
                .Where(button => AutomationProperties.GetName(button) is "Previous File" or "Next File")
                .ToDictionary(AutomationProperties.GetName);
            double Left(FrameworkElement control) => control.TransformToVisual(folderToolbar).TransformPoint(default).X;
            var previousFile = arrows["Previous File"];
            var nextFile = arrows["Next File"];
            var fileMenu = folder.Menu.Items.Single(m => m.Title == "Diff").Items.OfType<MenuFlyoutItem>();
            foreach (var (button, name, command) in new[] { (previousFile, "Previous File", ":N"), (nextFile, "Next File", ":n") }) {
                var menuItem = fileMenu.Single(item => item.Text == name);
                Check(Equals(ToolTipService.GetToolTip(button), $"{name} ({command})")
                    && menuItem.KeyboardAcceleratorTextOverride == command && menuItem.KeyboardAccelerators.Count == 0,
                    $"{name} displays {command} as a hint without registering a keyboard accelerator");
            }
            folder.root.UpdateLayout();
            double previousX = Left(previousFile), nextX = Left(nextFile), pickerWidth = files.ActualWidth;
            files.Width = pickerWidth < files.MaxWidth - 50 ? pickerWidth + 50 : pickerWidth - 50;
            folder.root.UpdateLayout();
            Check(previousX + previousFile.ActualWidth <= nextX && nextX + nextFile.ActualWidth <= Left(files)
                && Math.Abs(files.ActualWidth - pickerWidth) > 45 && Left(previousFile) == previousX && Left(nextFile) == nextX,
                "Both file arrows stay in place before the picker when its width changes");
            files.ClearValue(FrameworkElement.WidthProperty);
            var recent = await Task.Run(() => DiffSources.Commits(scoped, null, CancellationToken.None));
            var commit = recent.Single();
            Check(commit.Title == "base" && (string)bases.Items[^2] == "Commit…" && (string)bases.Items[^1] == "Choose File…",
                "Choose File… follows Commit… at the bottom of the base picker");
            ContentDialog? CommitDialog() => VisualTreeHelper.GetOpenPopupsForXamlRoot(folder.root.XamlRoot)
                .Where(p => p.Child != null).SelectMany(p => FormattingToolbarTests.Descendants<ContentDialog>(p.Child)).FirstOrDefault();
            for (int pass = 0; pass < 2; pass++) {
                bases.SelectedIndex = bases.Items.IndexOf("Commit…");
                await WaitFor(() => CommitDialog() != null);
                var dialog = CommitDialog()!;
                var content = (StackPanel)dialog.Content;
                var list = content.Children.OfType<ListBox>().Single();
                var input = content.Children.OfType<TextBox>().Single();
                Button? CompareButton() => FormattingToolbarTests.Descendants<Button>(dialog).FirstOrDefault(button => button.Name == "PrimaryButton");
                await WaitFor(() => CompareButton()?.IsLoaded == true);
                var compare = CompareButton()!;
                if (pass == 0) {
                    list.SelectedIndex = 0;
                    Check((string)list.Items[0] == $"{commit.Short}  base" && input.Text == commit.Hash, "Selecting a commit title fills in its full hash");
                } else {
                    input.Text = "--all";
                    await WaitFor(() => compare.IsEnabled);
                    new ButtonAutomationPeer(compare).Invoke();
                    await WaitFor(() => FormattingToolbarTests.Descendants<TextBlock>(dialog).Any(block => block.Text.Contains("hexadecimal")));
                    Check(folder.diffBase.Kind == VIEM_DIFF_BASE_HEAD, "An invalid pasted commit keeps the previous comparison and picker");
                    input.Text = commit.Short;
                }
                await WaitFor(() => compare.IsEnabled);
                new ButtonAutomationPeer(compare).Invoke();
                await WaitFor(() => folder.diffBase.Kind == VIEM_DIFF_BASE_COMMIT && CommitDialog() == null);
                Check(folder.diffBase.Argument == commit.Hash && Equals(bases.SelectedItem, $"Commit {commit.Short}")
                    && folder.diffLeft!.Document.FormattedText() == "old a" && folder.diffFiles.Length == 2,
                    $"The {(pass == 0 ? "listed" : "pasted")} commit compares against the pinned tree under the folder");
                if (pass == 0) await folder.ChooseDiffBase(new(VIEM_DIFF_BASE_HEAD));
            }
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
            Check(folderToolbar.AddedLines.Text == "+1" && folderToolbar.RemovedLines.Text == "–2",
                "Navigating to a deletion preserves the full patch totals");
            await folder.MoveDiffFile(1);
            await WaitFor(() => folder.comparison != null);
            Check(SamePath(folder.ActivePane!.Document.FilePath!, a), "Next File wraps from the last file to the first");
            folder.SetLineNumbers(true);
            string rust = string.Join("\n", Enumerable.Range(0, 650).Select(n => $"pub fn value_{n}() -> usize {{ {n} }}"));
            string[] names = ["c.rs", "d.rs", "e.rs"];
            foreach (string name in names) {
                string path = Path.Combine(scoped, name);
                await File.WriteAllTextAsync(path, rust);
                await Git("add", "sub/" + name);
                await File.WriteAllTextAsync(path, "// changed " + name + "\n" + rust);
            }
            await folder.ChooseDiffBase(new(VIEM_DIFF_BASE_UNSTAGED));
            folder.SetDiffCollapsed(true);
            var times = new List<string>();
            for (int pass = 0; pass < 2; ++pass) {
                foreach (string name in names) {
                    int index = Array.FindIndex(folder.diffFiles, file => Path.GetFileName(file.Path) == name);
                    var timer = Stopwatch.StartNew();
                    var livePane = folder.ActivePane!;
                    var basePane = folder.diffLeft!;
                    bool complete = false;
                    string? partialFrame = null;
                    void Frame(object? sender, object args) {
                        if (!SamePath(livePane.Document.FilePath!, folder.diffFiles[index].Path)) return;
                        ulong liveId = livePane.Document.State.document_id, baseId = basePane.Document.State.document_id;
                        bool text = livePane.PaintedDocumentForTesting == liveId, numbers = livePane.PaintedNumberDocumentForTesting == liveId;
                        bool baseText = basePane.PaintedDocumentForTesting == baseId, baseNumbers = basePane.PaintedNumberDocumentForTesting == baseId;
                        complete = text && numbers && baseText && baseNumbers;
                        if (!complete && (text || numbers || baseText || baseNumbers))
                            partialFrame ??= $"live text={text}, numbers={numbers}, base text={baseText}, numbers={baseNumbers}";
                    }
                    Microsoft.UI.Xaml.Media.CompositionTarget.Rendered += Frame;
                    try {
                        await folder.SelectDiffFile(index);
                        Check(livePane == folder.ActivePane && basePane == folder.diffLeft && folder.comparison != null
                            && livePane.LineNumbersForTesting?.Enabled == true && basePane.LineNumbersForTesting?.Enabled == true,
                            $"{name} installs both numbered views and the comparison before file navigation returns");
                        Check(livePane.View!.Layout().Decorations.Any(d => (d.flags & VIEM_LAYOUT_DECORATION_COLLAPSED_CONTEXT) != 0)
                            && basePane.View!.Layout().Decorations.Any(d => (d.flags & VIEM_LAYOUT_DECORATION_COLLAPSED_CONTEXT) != 0),
                            $"{name} opens with unchanged context collapsed on both sides");
                        await WaitFor(() => complete);
                        times.Add($"{pass + 1} {name} {timer.Elapsed.TotalMilliseconds:F1}");
                        Check(partialFrame == null, $"{name} first appears with both text surfaces and both gutters in the same frame {partialFrame}");
                    } finally { Microsoft.UI.Xaml.Media.CompositionTarget.Rendered -= Frame; }
                }
            }
            File.WriteAllLines(FrontendSmokeTests.ReportPath + ".diff-switch-ms.txt", times);
            // Hold an open just before publication. The old pair must remain
            // usable, and a later selection must win even if the old load finishes last.
            var paused = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            var resume = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            var previous = folder.ActivePane!.Document;
            var previousBase = folder.diffLeft!.Document;
            folder.BeforePublishOpenForTesting = async () => {
                folder.BeforePublishOpenForTesting = null;
                paused.SetResult(); await resume.Task;
            };
            var superseded = folder.SelectDiffFile(0);
            try {
                await paused.Task.WaitAsync(TimeSpan.FromSeconds(10));
                Check(folder.ActivePane.Document == previous && folder.diffLeft.Document == previousBase
                    && folder.comparison != null, "A pending file keeps the previous comparison visible and editable");
                await folder.SelectDiffFile(1);
            } finally { resume.TrySetResult(); }
            try { await superseded; } catch (OperationCanceledException) { }
            Check(SamePath(folder.ActivePane.Document.FilePath!, b) && folder.deletedSide
                && folder.diffLeft.Document.FormattedText() == "old b", "An older file switch cannot replace the newer comparison");
            string chosen = Path.Combine(directory, ".basis");
            int extra = 256 - folder.diffFiles.Length;
            for (int i = 0; i < extra; i++) File.WriteAllText(Path.Combine(scoped, $"z{i:D3}"), "new");
            await Git("add", "--", "sub/z*");
            await folder.ChooseDiffBase(new(VIEM_DIFF_BASE_HEAD));
            Check(files.Items.Count == 256 && folder.diffFiles.Length == 256 && files.Items[^1] is string, "Exactly 256 files fit without a clipping notice");
            File.WriteAllText(Path.Combine(scoped, "zzz"), "last");
            await Git("add", "sub/zzz");
            await folder.RefreshDiff();
            Check(files.Items.Count == 257 && files.Items[^1] is ComboBoxItem { IsEnabled: false, Content: "1 files clipped" }
                && folderToolbar.FileCount.Text == "257", "The disabled footer counts omitted files while totals cover the set");
            await folder.SelectDiffFile(255);
            await folder.MoveDiffFile(1);
            Check(files.SelectedIndex == 255 && Equals(files.SelectedItem, "zzz (added)")
                && SamePath(folder.ActivePane!.Document.FilePath!, Path.Combine(scoped, "zzz")),
                "Next File reaches the clipped file and the capped popup names it");
            await File.WriteAllTextAsync(chosen, "chosen");
            var comparedDocument = folder.ActivePane!.Document;
            int picks = 0;
            folder.PickDiffBaseFileForTesting = () => Task.FromResult(++picks == 1 ? null : chosen);
            bases.SelectedIndex = bases.Items.Count - 1;
            await WaitFor(() => picks == 1 && Equals(bases.SelectedItem, "HEAD"));
            Check(folder.diffFiles.Length == 257 && folder.diffScope == scoped && folder.ActivePane.Document == comparedDocument,
                "Canceling Choose File… preserves the folder comparison and current file");
            bases.SelectedIndex = bases.Items.Count - 1;
            await WaitFor(() => folder.diffBase.Argument == chosen);
            folder.PickDiffBaseFileForTesting = null;
            Check(folder.diffScope == null && folder.diffFiles.Length == 0 && !files.IsEnabled && files.PlaceholderText == comparedDocument.Name
                && folder.ActivePane.Document == comparedDocument && folder.diffLeft!.Document.FormattedText() == "chosen",
                "Choosing a dotfile as base compares it with the current document and clears folder navigation");
            await File.WriteAllTextAsync(chosen, "refreshed");
            await folder.RefreshDiff();
            Check(folder.diffLeft!.Document.FormattedText() == "refreshed", "Refresh reloads the chosen file from disk");
            try { await folder.ChooseDiffBase(new(VIEM_DIFF_BASE_FILE, scoped)); }
            catch (IOException) { }
            Check(folder.diffBase.Argument == chosen && folder.comparison != null && folder.diffLeft.Document.FormattedText() == "refreshed",
                "A failed base load retains the current file and comparison");
            await folder.ChooseDiffBase(new(VIEM_DIFF_BASE_HEAD));
            Check(folder.diffFiles.Length == 1 && SamePath(folder.diffFiles[0].Path, comparedDocument.FilePath),
                "Returning to a Git base keeps the comparison scoped to the current file");
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

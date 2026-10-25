#if DEBUG
using System.Text;
using Viem.Windows.Diagnostics;
using Viem.Windows.Editor;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    /// Runs the native self-test stages in order. `VIEM_TEST_SUITES` selects
    /// stages by name (comma separated); setup stages always run.
    private async Task RunSelfTests(EditorPane pane)
    {
        FrontendSmokeTests.WriteProgress("initialization");
        if (Environment.GetEnvironmentVariable("VIEM_PERF_DOCUMENT") is string performancePath)
        {
            var performancePane = AddPane(NewDocument(File.ReadAllBytes(performancePath), format: VIEM_FORMAT_MARKDOWN));
            await ClosePane(pane);
            await InputPerformance.Run(performancePane, FrontendSmokeTests.ReportPath!);
            Environment.Exit(0);
            return;
        }
        StyleWindow.ManualCaretFollowForTesting = true;
        var device = pane.Canvas.Device;
        string profile = preferences.DirectoryPath;
        var initial = pane;
        EditorPane? second = null;
        async Task WithPane(byte[] source, uint format, Func<EditorPane, Task> run)
        {
            var fixture = AddPane(NewDocument(source, format: format));
            try {
                await run(fixture);
            }
            finally {
                if (Panes.Contains(fixture)) await ClosePane(fixture, true);
            }
        }
        SelfTestRunner.Stage[] stages = [
            new("StartupFonts", () => {
                StyleAndSettingsTests.StartupFontChecks();
                return Task.CompletedTask;
            }),
            new("InputRouting", () => InputRoutingTests.Run(initial)),
            new("Zoom", () => InputRoutingTests.RunZoom(initial)),
            new("PointerSelection", () => {
                initial.View!.Wrap(true);
                return SelectionInputTests.RunPointerPlacement(initial);
            }),
            new("KeyboardSelection", () => SelectionInputTests.Run(initial, pointerPlacement: false)),
            new("MenuAccessKeys", () => MenuAccessKeyTests.Run(preferences)),
            new("CommandStatus", () => CommandStatusTests.Run(initial, this, preferences)),
            new("Chrome", () => ChromeStyleTests.Run(this, preferences)),
            new("CloseReview", () => DocumentCloseReviewTests.Run(preferences)),
            new("FileChangeMonitor", () => FileChangeMonitorTests.Run(profile)),
            new("ExternalFileChange", () => ExternalFileChangeTests.Run(preferences)),
            new("BlockingEdit", () => BlockingEditTests.Run(preferences)),
            new("DirectoryOpen", () => DirectoryOpenTests.Run(preferences)),
            new("HomePath", () => HomePathTests.Run(preferences)),
            new("ScrollDrawing", () => WithPane(ScrollDrawingTests.Fixture, VIEM_FORMAT_MARKDOWN, async scrolled => {
                await CommandStatusTests.RunScrolled(scrolled);
                await ScrollDrawingTests.Run(scrolled);
            })),
            new("HorizontalScroll", () => WithPane(HorizontalScrollTests.Fixture, VIEM_FORMAT_PLAIN_TEXT, HorizontalScrollTests.Run)),
            new("MarkdownDocument", async () => {
                pane = AddPane(NewDocument("# Viem for Windows\n\nA modal editor for writing.\n\nThe same Rust core, with native Windows controls.\n\nUnicode: café · 日本語 · مرحبا · 👩‍💻\n"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN));
                await pane.Ready;
                await ClosePane(initial, true);
                await InputRoutingTests.Settle();
                if (pane.LastError != null) throw pane.LastError;
                FrontendSmokeTests.UiChecks.Add("native editor draws without presentation errors");
            }, Setup: true),
            new("Presentation", () => PresentationTests.Run(pane, Menu, preferences, this)),
            PaneLayoutTests.Group(preferences),
            new("SplitInput", async () => {
                second = AddPane(pane.Document);
                await second.Ready;
                await InputRoutingTests.FocusPane(second, "Split input ");
                second.View!.Command(":");
                second.View.Text("%s/writing/composing/g");
                if (second.LastError != null) throw second.LastError;
                FrontendSmokeTests.UiChecks.Add("stacked panes and editable command prompt");
                second.View.Key(VIEM_KEY_ESCAPE);
                double previous = pane.Canvas.ActualHeight;
                preferences.Set("windows", "showMenu", false);
                await InputRoutingTests.Settle();
                if (Menu.Visibility != Microsoft.UI.Xaml.Visibility.Collapsed || pane.Canvas.ActualHeight <= previous) throw new InvalidOperationException("Menu toggle did not reclaim editor space.");
                preferences.Set("windows", "showMenu", true);
                FrontendSmokeTests.UiChecks.Add("titlebar menu visibility toggle reflows panes");
            }),
            new("VariableFonts", () => VariableFontTests.Run(pane, preferences)),
            new("FontCatalog", () => StyleAndSettingsTests.FontChecks(pane, preferences)),
            new("InspectorWindow", () => StyleAndSettingsTests.Run(pane, this, preferences)),
            new("InspectorRetargeting", () => StyleAndSettingsTests.DocumentSwitchingChecks(preferences)),
            new("StyleFontSizes", () => StyleFontSizeTests.Run(pane, preferences)),
            new("StyleInheritedActivation", () => StyleInspectorBehaviorTests.InheritedActivation(pane, preferences)),
            new("SelectionFind", () => SelectionFindTests.Run(preferences)),
            new("DocumentModeMenu", () => DocumentModeMenuTests.Run(preferences)),
            ListInteractionTests.Group(preferences),
            new("CodeStyles", () => StyleAndSettingsTests.CodeStyleChecks(pane, preferences)),
            new("StylePreview", async () => {
                StyleInspectorBehaviorTests.BlockPreview(pane, preferences);
                await StyleInspectorBehaviorTests.CharacterScriptPreview(pane, preferences);
            }),
            new("StyleBlockControls", () => {
                StyleInspectorBehaviorTests.LinkedBlockControls(pane, preferences);
                return Task.CompletedTask;
            }),
            new("StyleSessionRecovery", () => {
                StyleSessionRecoveryTests.Run(pane, preferences);
                return Task.CompletedTask;
            }),
            new("StyleFollowing", () => StyleInspectorBehaviorTests.Following(pane, preferences)),
            new("StyleColors", () => StyleInspectorBehaviorTests.Colors(pane, preferences)),
            new("Settings", () => StyleAndSettingsTests.RunSettings(pane, this, preferences)),
            new("VisibleWhitespaceStyle", () => VisibleWhitespaceStyleTests.Run(preferences)),
            new("ToolbarShortcuts", () => FormattingToolbarTests.RunShortcuts(preferences)),
            new("TablePicker", () => {
                TablePickerTests.Run();
                return Task.CompletedTask;
            }),
            new("TableInteraction", () => TableInteractionTests.Run(preferences)),
            LinkInteractionTests.Group(preferences),
            new("ImageInteraction", () => ImageInteractionTests.Run(preferences)),
            new("ImageLocationPicker", () => ImageInteractionTests.LocationPicker(preferences)),
            new("ImagePopups", () => ImageInteractionTests.NestedPopups(preferences)),
            new("ImageKeyboardSelection", () => ImageInteractionTests.KeyboardSelection(preferences)),
            new("ImageLocalPreview", () => ImageInteractionTests.LocalPreview(preferences)),
            new("ImageBackgroundResources", () => ImageInteractionTests.BackgroundImageResources(preferences)),
            new("ImageHtmlDimensions", () => ImageInteractionTests.HtmlImageDimensions(preferences)),
            new("ImageSizeEditor", () => ImageInteractionTests.SizeEditor(preferences)),
            new("ImagePreviewLimits", () => ImageInteractionTests.PreviewLimits(preferences)),
            new("ImageScrollReload", () => ImageInteractionTests.ScrollDuringImageReload(preferences)),
            new("ImageStyle", () => ImageInteractionTests.ImageStyle(preferences)),
            new("ImageCacheBudgets", () => ImageInteractionTests.CacheBudgets(preferences)),
            FormattingToolbarTests.Group(preferences),
            new("MarkdownViewPreference", () => MarkdownViewPreferenceTests.Run(profile)),
            new("WindowPlacement", () => WindowPlacementTests.Run(this, profile)),
            new("FileChecks", () => FrontendSmokeTests.FileChecks(device, DispatcherQueue, profile)),
            new("VimRuntime", () => VimRuntimeTests.Run(device, DispatcherQueue, profile)),
            new("BackgroundLayout", () => BackgroundLayoutTests.Run(device, DispatcherQueue)),
            new("SaveLifecycle", () => SaveLifecycle(pane)),
            new("Reload", TestReloadPositions),
            new("HtmlExport", () => HtmlExportTests.Run(this, pane, profile)),
            new("RenameAndLaunch", () => RenameAndLaunch(pane)),
            new("Core", () => {
                FrontendSmokeTests.Run(device, DispatcherQueue);
                return Task.CompletedTask;
            }),
        ];
        int CloseStrayWindows()
        {
            var stray = App.Instance.Windows.Where(w => w != this).ToArray();
            foreach (var w in stray) {
                w.closing = true;
                App.Instance.Windows.Remove(w);
                w.Close();
            }
            return stray.Length;
        }
        bool passed = await SelfTestRunner.Run(stages, CloseStrayWindows);
        foreach (var w in App.Instance.Windows.ToArray()) {
            w.closing = true;
            w.Close();
        }
        Environment.Exit(passed ? 0 : 1);
    }

    private async Task SaveLifecycle(EditorPane pane)
    {
        string saved = Path.Combine(preferences.DirectoryPath, "saved.md");
        await Save(pane, explicitPath: saved);
        var savedView = pane.View ?? throw new InvalidOperationException("The saved document has no editor view.");
        savedView.Command("ggi");
        savedView.Text("Saved ");
        savedView.Key(VIEM_KEY_ESCAPE);
        await Save(pane);
        if (pane.Document.IsDirty || !File.ReadAllBytes(saved).AsSpan().SequenceEqual(pane.Document.Source(pane.Document.State.document_revision))) throw new InvalidOperationException("Native save did not preserve source bytes or clear dirty state.");
        FrontendSmokeTests.UiChecks.Add("native save and atomic replacement preserve source bytes");
    }

    private async Task RenameAndLaunch(EditorPane pane)
    {
        if (pane.Document.FilePath == null) await SaveLifecycle(pane);
        var savedView = pane.View!;
        string renamed = Path.Combine(preferences.DirectoryPath, "renamed.md");
        string priorSlot = recoveries[pane.Document].Slot;
        savedView.Ex("file " + renamed);
        await effectQueue;
        if (pane.Document.FilePath != renamed || File.Exists(priorSlot) || DocumentRecovery.Read(recoveries[pane.Document].Slot)?.targetPath != renamed || File.Exists(renamed)) throw new InvalidOperationException(":file did not move its owned recovery slot without writing the source.");
        FrontendSmokeTests.UiChecks.Add("filename changes transfer recovery ownership without writing source");
        await Save(pane);
        string rangePath = Path.Combine(preferences.DirectoryPath, "range.txt");
        var rangePane = AddPane(NewDocument(Encoding.UTF8.GetBytes("first\nsecond\nthird"), format: VIEM_FORMAT_PLAIN_TEXT));
        await rangePane.Ready;
        rangePane.View!.Ex("1,2wq! " + rangePath);
        await effectQueue;
        if (Panes.Contains(rangePane) || File.ReadAllText(rangePath) != "first\nsecond\n") throw new InvalidOperationException("Ranged :wq did not write only its source lines and close its pane.");
        FrontendSmokeTests.UiChecks.Add("ranged write-and-quit honors source extent");
        string forwarded = Path.Combine(preferences.DirectoryPath, "forwarded.txt");
        File.WriteAllText(forwarded, "first\nsecond\n");
        var launch = new System.Diagnostics.ProcessStartInfo(Environment.ProcessPath!) { UseShellExecute = false, WindowStyle = System.Diagnostics.ProcessWindowStyle.Hidden, WorkingDirectory = preferences.DirectoryPath };
        launch.ArgumentList.Add("+2");
        launch.ArgumentList.Add("forwarded.txt");
        launch.ArgumentList.Add("deferred.txt");
        using (var child = System.Diagnostics.Process.Start(launch)!)
        {
            FrontendSmokeTests.ChildProcess = child;
            try {
            await child.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(10));
        } catch {
            if (!child.HasExited) child.Kill();
            throw;
        } finally {
            FrontendSmokeTests.ChildProcess = null;
        }
        }
        await invocationQueue;
        var opened = App.Instance.Windows.SelectMany(w => w.Panes).FirstOrDefault(p => p.Document.FilePath == forwarded);
        if (opened?.View?.Location.hard_line != 2 || App.Instance.Windows.SelectMany(w => w.Panes).Any(p => p.Document.FilePath?.EndsWith("deferred.txt") == true)) throw new InvalidOperationException("CLI handoff did not retain caller directory, first-file policy and +line.");
        FrontendSmokeTests.UiChecks.Add("second-process CLI handoff, relative paths, +line and lazy file arguments");
        var owner = App.Instance.Windows.Single(w => w.Panes.Contains(opened));
        if (owner == this) throw new InvalidOperationException("The forwarded file did not open in a new window.");
        await InputRoutingTests.CheckForegroundEditor(owner, opened, "second-process CLI launch focuses the new document window");
    }
}
#endif

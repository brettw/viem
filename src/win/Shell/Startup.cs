using System.Text;
using System.Text.Json;
using Viem.Windows.Editor;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private bool startupHandled;
    private static bool HasLaunchFiles()
    {
#if DEBUG
        if (Diagnostics.FrontendSmokeTests.ReportPath != null) return false;
#endif
        var arguments = ParseArguments(App.Instance.LaunchInvocation.Arguments);
        return arguments.Error == null && arguments.Filenames.Length > 0;
    }
    private void BeginFileLaunch()
    {
        if (startupHandled) return;
        startupHandled = true;
        invocationQueue = InitializeInvocation();
    }
    private async Task InitializeInvocation()
    {
        try
        {
            await OpenInvocation(App.Instance.LaunchInvocation);
            if (Panes.Count == 0 && !closed) AddPane(NewDocument());
            if (preferences.Error != null) ActivePane?.SetMessage(preferences.Error);
        }
        catch (Exception error)
        {
            if (closed) return;
            if (Panes.Count == 0) AddPane(NewDocument());
            ActivePane?.Report(error);
        }
    }
    partial void OnPaneReady(EditorPane pane)
    {
        if (startupHandled) return; startupHandled = true;
#if DEBUG
        if (Diagnostics.FrontendSmokeTests.ReportPath != null)
        {
            if (Diagnostics.FrontendSmokeTests.Started) return;
            Diagnostics.FrontendSmokeTests.Started = true;
            DispatcherQueue.TryEnqueue(async () => {
                try {
                    async Task RunTestStage(string name, Func<Task> run)
                    {
                        Diagnostics.FrontendSmokeTests.WriteProgress(name);
                        await run();
                    }
                    Diagnostics.FrontendSmokeTests.WriteProgress("initialization");
                    if (Environment.GetEnvironmentVariable("VIEM_TEST_BLOCKING_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_STARTUP_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_HOME_PATH_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_DIRECTORY_OPEN_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_SYNTAX_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_SCROLL_ONLY") is "1" or "horizontal" or "status"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_SETTINGS_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_LINE_SPACING_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_LISTS_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_IMAGES_ONLY") == "resources"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_TOOLBAR_ONLY") is "1" or "startup" or "main"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_POINTER_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_SELECTION_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_CLIPBOARD_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_ZOOM_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_MENU_KEYS_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_STATUS_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_CHROME_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_PANES_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_RELOAD_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_CLOSE_ONLY") == "1"
                        || Environment.GetEnvironmentVariable("VIEM_TEST_STYLES_ONLY") is "1" or "all" or "variable" or "switching" or "sizes" or "selection" or "code-follow" or "following" or "colors" or "modes" or "code-style")
                    {
                        if (Environment.GetEnvironmentVariable("VIEM_TEST_STARTUP_ONLY") == "1")
                        {
                            Diagnostics.StyleAndSettingsTests.StartupFontChecks();
                            await RunTestStage("FrontendSmokeTests.FileChecks", () => Diagnostics.FrontendSmokeTests.FileChecks(pane.Canvas.Device, DispatcherQueue, preferences.DirectoryPath));
                            await RunTestStage("BackgroundLayoutTests.Run", () => Diagnostics.BackgroundLayoutTests.Run(pane.Canvas.Device, DispatcherQueue));
                            Diagnostics.FrontendSmokeTests.WriteProgress("FrontendSmokeTests.Run");
                            Diagnostics.FrontendSmokeTests.Run(pane.Canvas.Device, DispatcherQueue);
                            Environment.Exit(0);
                            return;
                        }
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_HOME_PATH_ONLY") == "1")
                            await RunTestStage("HomePathTests.Run", () => Diagnostics.HomePathTests.Run(preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_DIRECTORY_OPEN_ONLY") == "1")
                            await RunTestStage("DirectoryOpenTests.Run", () => Diagnostics.DirectoryOpenTests.Run(preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_POINTER_ONLY") == "1")
                            await RunTestStage("SelectionInputTests.RunPointerPlacement", () => Diagnostics.SelectionInputTests.RunPointerPlacement(pane));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_SELECTION_ONLY") == "1")
                            await RunTestStage("SelectionInputTests.Run", () => Diagnostics.SelectionInputTests.Run(pane, pointerPlacement: false));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_CLIPBOARD_ONLY") == "1")
                            await RunTestStage("ClipboardSerializationTests.Run", Diagnostics.ClipboardSerializationTests.Run);
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_ZOOM_ONLY") == "1")
                            await RunTestStage("InputRoutingTests.RunZoom", () => Diagnostics.InputRoutingTests.RunZoom(pane));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_MENU_KEYS_ONLY") == "1")
                            await RunTestStage("MenuAccessKeyTests.Run", () => Diagnostics.MenuAccessKeyTests.Run(preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_STATUS_ONLY") == "1")
                            await RunTestStage("CommandStatusTests.Run", () => Diagnostics.CommandStatusTests.Run(pane, this, preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_CHROME_ONLY") == "1")
                            await RunTestStage("ChromeStyleTests.Run", () => Diagnostics.ChromeStyleTests.Run(this, preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_PANES_ONLY") == "1")
                            await RunTestStage("PaneLayoutTests.Run", () => Diagnostics.PaneLayoutTests.Run(preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_RELOAD_ONLY") == "1")
                            await RunTestStage("Reload positions", TestReloadPositions);
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_BLOCKING_ONLY") == "1")
                            await RunTestStage("BlockingEditTests.Run", () => Diagnostics.BlockingEditTests.Run(preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_CLOSE_ONLY") == "1")
                            await RunTestStage("DocumentCloseReviewTests.Run", () => Diagnostics.DocumentCloseReviewTests.Run(preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_TOOLBAR_ONLY") == "startup")
                            await RunTestStage("FormattingToolbarTests.RunStartup", () => Diagnostics.FormattingToolbarTests.RunStartup(preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_TOOLBAR_ONLY") == "1")
                            await RunTestStage("FormattingToolbarTests.Run", () => Diagnostics.FormattingToolbarTests.Run(preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_TOOLBAR_ONLY") == "main")
                            await RunTestStage("FormattingToolbarTests.Run", () => Diagnostics.FormattingToolbarTests.Run(preferences, startup: false));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_STYLES_ONLY") == "variable")
                        {
                            Diagnostics.StyleAndSettingsTests.StartupFontChecks();
                            await RunTestStage("VariableFontTests.Run", () => Diagnostics.VariableFontTests.Run(pane, preferences));
                        }
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_STYLES_ONLY") == "all")
                        {
                            Diagnostics.StyleAndSettingsTests.StartupFontChecks();
                            await RunTestStage("VariableFontTests.Run", () => Diagnostics.VariableFontTests.Run(pane, preferences));
                            await RunTestStage("StyleAndSettingsTests.Run", () => Diagnostics.StyleAndSettingsTests.Run(pane, this, preferences));
                        }
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_STYLES_ONLY") == "1")
                            await RunTestStage("StyleInspectorBehaviorTests.Run", () => Diagnostics.StyleInspectorBehaviorTests.Run(pane, preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_STYLES_ONLY") == "switching")
                            await RunTestStage("StyleAndSettingsTests.DocumentSwitchingChecks", () => Diagnostics.StyleAndSettingsTests.DocumentSwitchingChecks(preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_STYLES_ONLY") == "sizes")
                            await RunTestStage("StyleFontSizeTests.Run", () => Diagnostics.StyleFontSizeTests.Run(pane, preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_STYLES_ONLY") == "selection")
                            await RunTestStage("SelectionFindTests.Run", () => Diagnostics.SelectionFindTests.Run(preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_STYLES_ONLY") == "code-follow")
                            await RunTestStage("StyleInspectorBehaviorTests.MarkdownCodeFollowing", () => Diagnostics.StyleInspectorBehaviorTests.MarkdownCodeFollowing(pane, preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_STYLES_ONLY") == "following")
                            await RunTestStage("StyleInspectorBehaviorTests.Following", () => Diagnostics.StyleInspectorBehaviorTests.Following(pane, preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_STYLES_ONLY") == "colors")
                            await RunTestStage("StyleInspectorBehaviorTests.Colors", () => Diagnostics.StyleInspectorBehaviorTests.Colors(pane, preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_STYLES_ONLY") == "modes")
                            await RunTestStage("DocumentModeMenuTests.Run", () => Diagnostics.DocumentModeMenuTests.Run(preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_STYLES_ONLY") == "code-style")
                            await RunTestStage("StyleAndSettingsTests.CodeStyleChecks", () => Diagnostics.StyleAndSettingsTests.CodeStyleChecks(pane, preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_LINE_SPACING_ONLY") == "1")
                            await RunTestStage("StyleAndSettingsTests.RunLineSpacing", () => Diagnostics.StyleAndSettingsTests.RunLineSpacing(pane, preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_LISTS_ONLY") == "1")
                            await RunTestStage("ListInteractionTests.Run", () => Diagnostics.ListInteractionTests.Run(preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_IMAGES_ONLY") == "resources")
                            await RunTestStage("ImageInteractionTests.RunResources", () => Diagnostics.ImageInteractionTests.RunResources(preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_SETTINGS_ONLY") == "1")
                            await RunTestStage("StyleAndSettingsTests.RunSettings", () => Diagnostics.StyleAndSettingsTests.RunSettings(pane, this, preferences));
                        else if (Environment.GetEnvironmentVariable("VIEM_TEST_SCROLL_ONLY") is "1" or "horizontal" or "status")
                        {
                            bool horizontal = Environment.GetEnvironmentVariable("VIEM_TEST_SCROLL_ONLY") == "horizontal";
                            bool statusOnly = Environment.GetEnvironmentVariable("VIEM_TEST_SCROLL_ONLY") == "status";
                            var scrollPane = AddPane(NewDocument(
                                horizontal ? Diagnostics.HorizontalScrollTests.Fixture : Diagnostics.ScrollDrawingTests.Fixture,
                                format: horizontal ? VIEM_FORMAT_PLAIN_TEXT : VIEM_FORMAT_MARKDOWN));
                            if (!statusOnly) await ClosePane(pane);
                            if (statusOnly) await RunTestStage("CommandStatusTests.RunScrolled", () => Diagnostics.CommandStatusTests.RunScrolled(scrollPane));
                            else if (horizontal) await Diagnostics.HorizontalScrollTests.Run(scrollPane);
                            else await Diagnostics.ScrollDrawingTests.Run(scrollPane);
                        }
                        else
                            await RunTestStage("VimRuntimeTests.Run", () => Diagnostics.VimRuntimeTests.Run(pane.Canvas.Device, DispatcherQueue, preferences.DirectoryPath));
                        var checks = Diagnostics.FrontendSmokeTests.UiChecks;
                        Diagnostics.FrontendSmokeTests.WriteSuccess(checks);
                        Environment.Exit(0);
                        return;
                    }
                    if (Environment.GetEnvironmentVariable("VIEM_PERF_DOCUMENT") is string performancePath)
                    {
                        var performancePane = AddPane(NewDocument(File.ReadAllBytes(performancePath), format: VIEM_FORMAT_MARKDOWN));
                        await ClosePane(pane);
                        await RunTestStage("InputPerformance.Run", () => Diagnostics.InputPerformance.Run(performancePane, Diagnostics.FrontendSmokeTests.ReportPath));
                        Environment.Exit(0);
                        return;
                    }
                    Diagnostics.StyleAndSettingsTests.StartupFontChecks();
                    await RunTestStage("InputRoutingTests.Run", () => Diagnostics.InputRoutingTests.Run(pane));
                    await RunTestStage("MenuAccessKeyTests.Run", () => Diagnostics.MenuAccessKeyTests.Run(preferences));
                    await RunTestStage("CommandStatusTests.Run", () => Diagnostics.CommandStatusTests.Run(pane, this, preferences));
                    await RunTestStage("ChromeStyleTests.Run", () => Diagnostics.ChromeStyleTests.Run(this, preferences));
                    await RunTestStage("DocumentCloseReviewTests.Run", () => Diagnostics.DocumentCloseReviewTests.Run(preferences));
                    await RunTestStage("BlockingEditTests.Run", () => Diagnostics.BlockingEditTests.Run(preferences));
                    await RunTestStage("DirectoryOpenTests.Run", () => Diagnostics.DirectoryOpenTests.Run(preferences));
                    await RunTestStage("HomePathTests.Run", () => Diagnostics.HomePathTests.Run(preferences));
                    var scrolled = AddPane(NewDocument(Diagnostics.ScrollDrawingTests.Fixture, format: VIEM_FORMAT_MARKDOWN));
                    await RunTestStage("CommandStatusTests.RunScrolled", () => Diagnostics.CommandStatusTests.RunScrolled(scrolled));
                    await RunTestStage("ScrollDrawingTests.Run", () => Diagnostics.ScrollDrawingTests.Run(scrolled));
                    await ClosePane(scrolled);
                    var horizontalScroll = AddPane(NewDocument(Diagnostics.HorizontalScrollTests.Fixture, format: VIEM_FORMAT_PLAIN_TEXT));
                    await RunTestStage("HorizontalScrollTests.Run", () => Diagnostics.HorizontalScrollTests.Run(horizontalScroll));
                    await ClosePane(horizontalScroll);
                    var priorPane = pane;
                    pane = AddPane(NewDocument("# Viem for Windows\n\nA modal editor for writing.\n\nThe same Rust core, with native Windows controls.\n\nUnicode: café · 日本語 · مرحبا · 👩‍💻\n"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN));
                    await pane.Ready;
                    await ClosePane(priorPane, true);
                    await Task.Delay(400);
                    if (pane.LastError != null) throw pane.LastError;
                    Diagnostics.FrontendSmokeTests.UiChecks.Add("native editor draws without presentation errors");
                    await RunTestStage("WindowCapture.Save", () => Diagnostics.WindowCapture.Save(Hwnd, pane.Canvas.Device, Diagnostics.FrontendSmokeTests.ReportPath + ".png"));
                    await RunTestStage("PresentationTests.Run", () => Diagnostics.PresentationTests.Run(pane, Menu, preferences, this));
                    await RunTestStage("PaneLayoutTests.Run", () => Diagnostics.PaneLayoutTests.Run(preferences));
                    var second = AddPane(pane.Document);
                    await Task.Delay(250);
                    await RunTestStage("InputRoutingTests.FocusPane", () => Diagnostics.InputRoutingTests.FocusPane(second, "Split input "));
                    second.View!.Command(":"); second.View.Text("%s/writing/composing/g");
                    await Task.Delay(250);
                    if (second.LastError != null) throw second.LastError;
                    Diagnostics.FrontendSmokeTests.UiChecks.Add("stacked panes and editable command prompt");
                    await RunTestStage("WindowCapture.Save", () => Diagnostics.WindowCapture.Save(Hwnd, pane.Canvas.Device, Diagnostics.FrontendSmokeTests.ReportPath + ".panes.png"));
                    second.View.Key(VIEM_KEY_ESCAPE);
                    double previous = pane.Canvas.ActualHeight;
                    preferences.Set("windows", "showMenu", false); await Task.Delay(100);
                    if (Menu.Visibility != Microsoft.UI.Xaml.Visibility.Collapsed || pane.Canvas.ActualHeight <= previous) throw new InvalidOperationException("Menu toggle did not reclaim editor space.");
                    preferences.Set("windows", "showMenu", true);
                    Diagnostics.FrontendSmokeTests.UiChecks.Add("titlebar menu visibility toggle reflows panes");
                    await RunTestStage("VariableFontTests.Run", () => Diagnostics.VariableFontTests.Run(pane, preferences));
                            await RunTestStage("StyleAndSettingsTests.Run", () => Diagnostics.StyleAndSettingsTests.Run(pane, this, preferences));
                    await RunTestStage("FormattingToolbarTests.Run", () => Diagnostics.FormattingToolbarTests.Run(preferences));
                    await RunTestStage("WindowPlacementTests.Run", () => Diagnostics.WindowPlacementTests.Run(this, preferences.DirectoryPath));
                    await RunTestStage("FrontendSmokeTests.FileChecks", () => Diagnostics.FrontendSmokeTests.FileChecks(pane.Canvas.Device, DispatcherQueue, preferences.DirectoryPath));
                    await RunTestStage("VimRuntimeTests.Run", () => Diagnostics.VimRuntimeTests.Run(pane.Canvas.Device, DispatcherQueue, preferences.DirectoryPath));
                    await RunTestStage("BackgroundLayoutTests.Run", () => Diagnostics.BackgroundLayoutTests.Run(pane.Canvas.Device, DispatcherQueue));
                    Diagnostics.FrontendSmokeTests.WriteProgress("Native save and launch lifecycle");
                    string saved = Path.Combine(preferences.DirectoryPath, "saved.md");
                    await Save(pane, explicitPath: saved);
                    var savedView = pane.View ?? throw new InvalidOperationException("The saved document has no editor view.");
                    savedView.Command("ggi"); savedView.Text("Saved "); savedView.Key(VIEM_KEY_ESCAPE); await Save(pane);
                    if (pane.Document.IsDirty || !File.ReadAllBytes(saved).AsSpan().SequenceEqual(pane.Document.Source(pane.Document.State.document_revision))) throw new InvalidOperationException("Native save did not preserve source bytes or clear dirty state.");
                    Diagnostics.FrontendSmokeTests.UiChecks.Add("native save and atomic replacement preserve source bytes");
                    await RunTestStage("Reload positions", TestReloadPositions);
                    await RunTestStage("HtmlExportTests.Run", () => Diagnostics.HtmlExportTests.Run(this, pane, preferences.DirectoryPath));
                    string renamed = Path.Combine(preferences.DirectoryPath, "renamed.md");
                    string priorSlot = recoveries[pane.Document].Slot;
                    savedView.Ex("file " + renamed); await effectQueue;
                    if (pane.Document.FilePath != renamed || File.Exists(priorSlot) || DocumentRecovery.Read(recoveries[pane.Document].Slot)?.targetPath != renamed || File.Exists(renamed)) throw new InvalidOperationException(":file did not move its owned recovery slot without writing the source.");
                    Diagnostics.FrontendSmokeTests.UiChecks.Add("filename changes transfer recovery ownership without writing source");
                    await Save(pane);
                    string rangePath = Path.Combine(preferences.DirectoryPath, "range.txt");
                    var rangePane = AddPane(NewDocument(Encoding.UTF8.GetBytes("first\nsecond\nthird"), format: VIEM_FORMAT_PLAIN_TEXT)); await rangePane.Ready;
                    rangePane.View!.Ex("1,2wq! " + rangePath); await effectQueue;
                    if (Panes.Contains(rangePane) || File.ReadAllText(rangePath) != "first\nsecond\n") throw new InvalidOperationException("Ranged :wq did not write only its source lines and close its pane.");
                    Diagnostics.FrontendSmokeTests.UiChecks.Add("ranged write-and-quit honors source extent");
                    string forwarded = Path.Combine(preferences.DirectoryPath, "forwarded.txt"); File.WriteAllText(forwarded, "first\nsecond\n");
                    var launch = new System.Diagnostics.ProcessStartInfo(Environment.ProcessPath!) { UseShellExecute = false, WindowStyle = System.Diagnostics.ProcessWindowStyle.Hidden, WorkingDirectory = preferences.DirectoryPath };
                    launch.ArgumentList.Add("+2"); launch.ArgumentList.Add("forwarded.txt"); launch.ArgumentList.Add("deferred.txt");
                    using (var child = System.Diagnostics.Process.Start(launch)!)
                    { Diagnostics.FrontendSmokeTests.ChildProcess = child; try { await child.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(10)); } catch { if (!child.HasExited) child.Kill(); throw; } finally { Diagnostics.FrontendSmokeTests.ChildProcess = null; } }
                    await invocationQueue;
                    var opened = App.Instance.Windows.SelectMany(w => w.Panes).FirstOrDefault(p => p.Document.FilePath == forwarded);
                    if (opened?.View?.Location.hard_line != 2 || App.Instance.Windows.SelectMany(w => w.Panes).Any(p => p.Document.FilePath?.EndsWith("deferred.txt") == true)) throw new InvalidOperationException("CLI handoff did not retain caller directory, first-file policy and +line.");
                    Diagnostics.FrontendSmokeTests.UiChecks.Add("second-process CLI handoff, relative paths, +line and lazy file arguments");
                    Diagnostics.FrontendSmokeTests.WriteProgress("FrontendSmokeTests.Run");
                    Diagnostics.FrontendSmokeTests.Run(pane.Canvas.Device, DispatcherQueue);
                    foreach (var w in App.Instance.Windows.ToArray()) { w.closing = true; w.Close(); }
                    Environment.Exit(0);
                }
                catch (Exception e) { Diagnostics.FrontendSmokeTests.WriteFailure(e); Environment.Exit(1); }
            });
            return;
        }
#endif
        if (App.Instance.Windows.Count <= 1)
        {
            invocationQueue = InitializeInvocation();
        }
    }
    private Task invocationQueue = Task.CompletedTask;
#if DEBUG
    internal Task OpenArgumentsForTesting(OpenInvocation request) => OpenArguments(request);
#endif
    internal void ReceiveInvocation(OpenInvocation request)
    {
        async Task Next(Task previous) { await previous; try { Activate(); await OpenInvocation(request); ActivePane?.FocusEditor(); } catch (Exception error) { ActivePane?.Report(error); } }
        invocationQueue = Next(invocationQueue);
    }
    private async Task OpenInvocation(OpenInvocation request)
    {
        if (request.CompletionPipe == null) { await OpenArguments(request); return; }
        BlockingEditSessions.Completion? completion = await BlockingEditSessions.Connect(request.CompletionPipe);
        try
        {
            if (request.Arguments.Length != 1) throw new InvalidDataException("Blocking editing requires one file.");
            string path = ResolvePath(request.Arguments[0], request.Directory);
            if (Directory.Exists(path)) throw new IOException("Blocking editing requires a file, not a directory.");
            await OpenArguments(request);
            var document = App.Instance.Windows.SelectMany(w => w.Panes).Select(p => p.Document)
                .FirstOrDefault(d => d.FilePath is string file && FileIdentity.Same(file, path));
            if (document == null) throw new IOException("The requested file was not opened.");
            App.Instance.BlockingEdits.Attach(document, completion);
            completion = null;
#if DEBUG
            var owner = App.Instance.Windows.First(w => w.Panes.Any(p => p.Document == document));
            await Diagnostics.BlockingEditTests.OnBlockingLaunch(owner);
#endif
        }
        finally { completion?.Dispose(); }
    }

    private async Task OpenArguments(OpenInvocation request)
    {
        var args = ParseArguments(request.Arguments);
        if (args.Error != null) throw new InvalidOperationException(args.Error);
        if (request.Arguments.Length == 0) return;
        string[] paths = args.Filenames.Select(p => ResolvePath(p, request.Directory)).ToArray();
        bool represented = paths.Length > 0 && App.Instance.Windows.SelectMany(w => w.Panes).Any(p => p.Document.FilePath is string file && FileIdentity.Same(file, paths[0]));
        bool blank = Panes.Count == 0 || (Panes.Count == 1 && ActivePane is { Document: var current } && current.FilePath == null && current.State.source_byte_count == 0 && !current.IsDirty && (current.State.flags & (VIEM_DOCUMENT_STATE_CAN_UNDO | VIEM_DOCUMENT_STATE_CAN_REDO)) == 0);
        if (paths.Length > 0 && !represented && !blank)
        {
            if (File.Exists(paths[0])) _ = await File.ReadAllBytesAsync(paths[0]);
            var window = NewWindow(null); await window.OpenArguments(request); return;
        }
        App.Instance.Arguments.Clear(); App.Instance.Arguments.AddRange(paths);
        int split = args.SplitCount == 0 ? Math.Max(1, paths.Length) : args.SplitCount ?? 1;
        split = Math.Clamp(split, 1, Math.Max(1, args.VerticalSplits ? (int)((paneGrid.ActualWidth + 5) / 105) : (int)(paneGrid.ActualHeight / preferences.StatusHeight)));
        EditorPane? first = paths.Length == 0 ? ActivePane : null;
        foreach (string path in paths.Take(split))
        {
            if (first != null && ActivePane != null) paneGrid.Action(ActivePane, 5, args.VerticalSplits ? 1u : 0u, flags: 2);
            await OpenPath(path, first != null, vertical: args.VerticalSplits);
            paneGrid.Equalize();
            if (closed) return;
            first ??= App.Instance.Windows.SelectMany(w => w.Panes).FirstOrDefault(p => p.Document.FilePath is string file && FileIdentity.Same(file, path));
        }
        while (Panes.Count < split) { paneGrid.Action(ActivePane!, 5, args.VerticalSplits ? 1u : 0u, flags: 2); RequireSplitRoom(vertical: args.VerticalSplits); var added = SplitPane(ActivePane!, NewDocument(), vertical: args.VerticalSplits); paneGrid.Equalize(); first ??= added; }
        if (first != null)
        {
            foreach (var pane in Panes) await pane.Ready.WaitAsync(TimeSpan.FromSeconds(15));
            var view = await first.Ready.WaitAsync(TimeSpan.FromSeconds(15));
            if (args.InitialLine != null) view.GoToLine(args.InitialLine.Value);
            var owner = App.Instance.Windows.First(w => w.Panes.Contains(first)); owner.Activate(); owner.SetActive(first); first.FocusEditor();
        }
    }
    private sealed record LaunchArguments(string[] Filenames, int? SplitCount, ulong? InitialLine, bool VerticalSplits, string? Error);
    private static unsafe LaunchArguments ParseArguments(string[] args)
    {
        using var timing = Diagnostics.StartupPerformance.Measure("arguments.parse");
        byte[] input = JsonSerializer.SerializeToUtf8Bytes(args, FrontendJsonContext.Default.StringArray);
        byte[] output = Abi.Copy((p, n, r) => { fixed (byte* data = input) return viem_parse_launch_arguments(data, (ulong)input.Length, p, n, r); });
        using var document = JsonDocument.Parse(output);
        var root = document.RootElement;
        string? error = root.GetProperty("error").ValueKind == JsonValueKind.Null ? null : root.GetProperty("error").GetString();
        if (error != null) return new([], null, null, false, error);
        var arguments = root.GetProperty("arguments");
        return new(arguments.GetProperty("filenames").EnumerateArray().Select(p => p.GetString()!).ToArray(),
            arguments.GetProperty("splitCount").ValueKind == JsonValueKind.Null ? null : arguments.GetProperty("splitCount").GetInt32(),
            arguments.GetProperty("initialLine").ValueKind == JsonValueKind.Null ? null : arguments.GetProperty("initialLine").GetUInt64(), arguments.GetProperty("verticalSplits").GetBoolean(), null);
    }
}

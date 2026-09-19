using System.Text;
using System.Text.Json;
using Viem.Windows.Editor;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private bool startupHandled;
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
                    if (Environment.GetEnvironmentVariable("VIEM_TEST_SYNTAX_ONLY") == "1")
                    {
                        await Diagnostics.VimRuntimeTests.Run(pane.Canvas.Device, DispatcherQueue, preferences.DirectoryPath);
                        var checks = Diagnostics.FrontendSmokeTests.UiChecks;
                        File.WriteAllText(Diagnostics.FrontendSmokeTests.ReportPath, JsonSerializer.Serialize(new { passed = true, count = checks.Count, checks }));
                        Environment.Exit(0);
                        return;
                    }
                    if (Environment.GetEnvironmentVariable("VIEM_PERF_DOCUMENT") is string performancePath)
                    {
                        var performancePane = AddPane(NewDocument(File.ReadAllBytes(performancePath), format: VIEM_FORMAT_MARKDOWN));
                        await ClosePane(pane);
                        await Diagnostics.InputPerformance.Run(performancePane, Diagnostics.FrontendSmokeTests.ReportPath);
                        Environment.Exit(0);
                        return;
                    }
                    Diagnostics.StyleAndSettingsTests.StartupFontChecks();
                    await Diagnostics.InputRoutingTests.Run(pane);
                    await Diagnostics.CommandStatusTests.Run(pane, this, preferences);
                    var scrolled = AddPane(NewDocument(Encoding.UTF8.GetBytes(string.Concat(Enumerable.Repeat("A paragraph in a large document.\n", 5000)))));
                    await Diagnostics.CommandStatusTests.RunScrolled(scrolled);
                    await ClosePane(scrolled);
                    pane.View!.Command("i"); pane.View.Text("# Viem for Windows\n\nA modal editor for writing.\n\nThe same Rust core, with native Windows controls.\n\nUnicode: café · 日本語 · مرحبا · 👩‍💻\n"); pane.View.Key(VIEM_KEY_ESCAPE);
                    pane.View.Format(VIEM_FORMAT_MARKDOWN);
                    await Task.Delay(400);
                    if (pane.LastError != null) throw pane.LastError;
                    Diagnostics.FrontendSmokeTests.UiChecks.Add("native editor draws without presentation errors");
                    await Diagnostics.WindowCapture.Save(Hwnd, pane.Canvas.Device, Diagnostics.FrontendSmokeTests.ReportPath + ".png");
                    await Diagnostics.PresentationTests.Run(pane, Menu, preferences, this);
                    var second = AddPane(pane.Document);
                    await Task.Delay(250);
                    await Diagnostics.InputRoutingTests.FocusPane(second, "Split input ");
                    second.View!.Command(":"); second.View.Text("%s/writing/composing/g");
                    await Task.Delay(250);
                    if (second.LastError != null) throw second.LastError;
                    Diagnostics.FrontendSmokeTests.UiChecks.Add("stacked panes and editable command prompt");
                    await Diagnostics.WindowCapture.Save(Hwnd, pane.Canvas.Device, Diagnostics.FrontendSmokeTests.ReportPath + ".panes.png");
                    second.View.Key(VIEM_KEY_ESCAPE);
                    double previous = pane.Canvas.ActualHeight;
                    preferences.Set("windows", "showMenu", false); await Task.Delay(100);
                    if (Menu.Visibility != Microsoft.UI.Xaml.Visibility.Collapsed || pane.Canvas.ActualHeight <= previous) throw new InvalidOperationException("Menu toggle did not reclaim editor space.");
                    preferences.Set("windows", "showMenu", true);
                    Diagnostics.FrontendSmokeTests.UiChecks.Add("titlebar menu visibility toggle reflows panes");
                    await Diagnostics.StyleAndSettingsTests.Run(pane, this, preferences);
                    await Diagnostics.WindowPlacementTests.Run(this, preferences.DirectoryPath);
                    await Diagnostics.FrontendSmokeTests.FileChecks(pane.Canvas.Device, DispatcherQueue, preferences.DirectoryPath);
                    await Diagnostics.VimRuntimeTests.Run(pane.Canvas.Device, DispatcherQueue, preferences.DirectoryPath);
                    string saved = Path.Combine(preferences.DirectoryPath, "saved.md");
                    await Save(pane, explicitPath: saved);
                    pane.View.Command("ggi"); pane.View.Text("Saved "); pane.View.Key(VIEM_KEY_ESCAPE); await Save(pane);
                    if (pane.Document.IsDirty || !File.ReadAllBytes(saved).AsSpan().SequenceEqual(pane.Document.Source(pane.Document.State.document_revision))) throw new InvalidOperationException("Native save did not preserve source bytes or clear dirty state.");
                    Diagnostics.FrontendSmokeTests.UiChecks.Add("native save and atomic replacement preserve source bytes");
                    string renamed = Path.Combine(preferences.DirectoryPath, "renamed.md");
                    string priorSlot = recoveries[pane.Document].Slot;
                    pane.View.Ex("file " + renamed); await effectQueue;
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
            async Task Initialize()
            { try { LoadCodeStyles(); await OpenArguments(new(Environment.GetCommandLineArgs().Skip(1).ToArray(), Environment.CurrentDirectory)); if (preferences.Error != null) ActivePane?.SetMessage(preferences.Error); } catch (Exception error) { ActivePane?.Report(error); } }
            invocationQueue = Initialize();
        }
    }
    private Task invocationQueue = Task.CompletedTask;
    internal void ReceiveInvocation(OpenInvocation request)
    {
        async Task Next(Task previous) { await previous; try { Activate(); await OpenArguments(request); ActivePane?.FocusEditor(); } catch (Exception error) { ActivePane?.Report(error); } }
        invocationQueue = Next(invocationQueue);
    }
    private async Task OpenArguments(OpenInvocation request)
    {
        var args = ParseArguments(request.Arguments);
        if (args.Error != null) throw new InvalidOperationException(args.Error);
        if (request.Arguments.Length == 0) return;
        string[] paths = args.Filenames.Select(p => Path.GetFullPath(p, request.Directory)).ToArray();
        bool represented = paths.Length > 0 && App.Instance.Windows.SelectMany(w => w.Panes).Any(p => p.Document.FilePath is string file && FileIdentity.Same(file, paths[0]));
        bool blank = Panes.Count == 1 && ActivePane is { Document: var current } && current.FilePath == null && current.State.source_byte_count == 0 && !current.IsDirty && (current.State.flags & (VIEM_DOCUMENT_STATE_CAN_UNDO | VIEM_DOCUMENT_STATE_CAN_REDO)) == 0;
        if (paths.Length > 0 && !represented && !blank)
        {
            if (File.Exists(paths[0])) _ = await File.ReadAllBytesAsync(paths[0]);
            var window = NewWindow(null); await window.OpenArguments(request); return;
        }
        App.Instance.Arguments.Clear(); App.Instance.Arguments.AddRange(paths);
        int split = args.SplitCount == 0 ? Math.Max(1, paths.Length) : args.SplitCount ?? 1;
        split = Math.Clamp(split, 1, Math.Max(1, (int)paneGrid.ActualHeight / 60));
        EditorPane? first = paths.Length == 0 ? ActivePane : null;
        foreach (string path in paths.Take(split))
        {
            await OpenPath(path, first != null);
            first ??= App.Instance.Windows.SelectMany(w => w.Panes).FirstOrDefault(p => p.Document.FilePath is string file && FileIdentity.Same(file, path));
        }
        while (Panes.Count < split) { var added = AddPane(NewDocument()); first ??= added; }
        if (first != null)
        {
            foreach (var pane in Panes) await pane.Ready.WaitAsync(TimeSpan.FromSeconds(15));
            var view = await first.Ready.WaitAsync(TimeSpan.FromSeconds(15));
            if (args.InitialLine != null) view.GoToLine(args.InitialLine.Value);
            var owner = App.Instance.Windows.First(w => w.Panes.Contains(first)); owner.Activate(); owner.SetActive(first); first.FocusEditor();
        }
    }
    private sealed record LaunchArguments(string[] Filenames, int? SplitCount, ulong? InitialLine, string? Error);
    private static unsafe LaunchArguments ParseArguments(string[] args)
    {
        byte[] input = JsonSerializer.SerializeToUtf8Bytes(args);
        byte[] output = Abi.Copy((p, n, r) => { fixed (byte* data = input) return viem_parse_launch_arguments(data, (ulong)input.Length, p, n, r); });
        using var document = JsonDocument.Parse(output);
        var root = document.RootElement;
        string? error = root.GetProperty("error").ValueKind == JsonValueKind.Null ? null : root.GetProperty("error").GetString();
        if (error != null) return new([], null, null, error);
        var arguments = root.GetProperty("arguments");
        return new(arguments.GetProperty("filenames").EnumerateArray().Select(p => p.GetString()!).ToArray(),
            arguments.GetProperty("splitCount").ValueKind == JsonValueKind.Null ? null : arguments.GetProperty("splitCount").GetInt32(),
            arguments.GetProperty("initialLine").ValueKind == JsonValueKind.Null ? null : arguments.GetProperty("initialLine").GetUInt64(), null);
    }
}

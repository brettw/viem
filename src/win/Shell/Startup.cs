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
                try { await RunSelfTests(pane); }
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
            await OpenPath(path, first != null, vertical: args.VerticalSplits, splitBefore: false);
            paneGrid.Equalize();
            if (closed) return;
            first ??= App.Instance.Windows.SelectMany(w => w.Panes).FirstOrDefault(p => p.Document.FilePath is string file && FileIdentity.Same(file, path));
        }
        while (Panes.Count < split) { paneGrid.Action(ActivePane!, 5, args.VerticalSplits ? 1u : 0u, flags: 2); RequireSplitRoom(vertical: args.VerticalSplits); var added = SplitPane(ActivePane!, NewDocument(), vertical: args.VerticalSplits, before: false); paneGrid.Equalize(); first ??= added; }
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

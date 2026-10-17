#if DEBUG
using System.Threading.Channels;
using Viem.Windows.Shell;

namespace Viem.Windows.Diagnostics;

internal static class FileChangeMonitorTests
{
    private static void Check(bool value, string name)
    { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }

    internal static async Task Run(string profile)
    {
        string directory = Path.Combine(profile, "file-monitor-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        try
        {
            await Changes(directory);
            await ParentRecreation(directory);
            await Disposal(directory);
        }
        finally { Directory.Delete(directory, recursive: true); }
    }

    private static async Task Changes(string directory)
    {
        string path = Path.Combine(directory, "document.txt");
        await File.WriteAllTextAsync(path, "initial");
        var events = Channel.CreateUnbounded<int>();
        int ownerThread = Environment.CurrentManagedThreadId;
        using var monitor = new FileChangeMonitor(path, () => events.Writer.TryWrite(Environment.CurrentManagedThreadId));
        int callbackThread = await Next(events);
        Check(callbackThread != ownerThread, "file monitoring installs and notifies off the window thread");

        await Observe(events, () => File.WriteAllText(path, "modified"));
        Check(true, "file monitoring detects an in-place write without polling or activation");

        string temporary = Path.Combine(directory, "replacement.txt");
        await File.WriteAllTextAsync(temporary, "replacement");
        await Observe(events, () => File.Move(temporary, path, overwrite: true));
        Check(true, "file monitoring detects an atomic replacement");

        await Observe(events, () => File.Delete(path));
        await Observe(events, () => File.WriteAllText(path, "recreated"));
        Check(true, "file monitoring continues through deletion and recreation");

        await Quiet(events);
        await File.WriteAllTextAsync(Path.Combine(directory, "unrelated.txt"), "unrelated");
        await Task.Delay(200);
        Check(!events.Reader.TryRead(out _), "unrelated sibling writes do not request a document content check");

        await Quiet(events);
        using var stop = new CancellationTokenSource();
        var writes = Task.Run(async () =>
        {
            while (!stop.IsCancellationRequested)
            {
                await File.AppendAllTextAsync(path, ".");
                await Task.Delay(10);
            }
        });
        try
        {
            await Next(events);
            Check(!writes.IsCompleted, "continuous writes cannot postpone file monitoring until the write burst ends");
        }
        finally { stop.Cancel(); await writes; }
    }

    private static async Task ParentRecreation(string directory)
    {
        string parent = Path.Combine(directory, "nested", "child");
        string path = Path.Combine(parent, "document.txt");
        Directory.CreateDirectory(parent);
        await File.WriteAllTextAsync(path, "initial");
        var events = Channel.CreateUnbounded<int>();
        using var monitor = new FileChangeMonitor(path, () => events.Writer.TryWrite(0));
        await Next(events);
        await Observe(events, () => Directory.Delete(Path.GetDirectoryName(parent)!, recursive: true));
        await Observe(events, () => { Directory.CreateDirectory(parent); File.WriteAllText(path, "recreated"); });
        await Observe(events, () => File.WriteAllText(path, "changed again"));
        Check(true, "file monitoring rearms after missing ancestor directories are recreated");

        string missingPath = Path.Combine(directory, "not-yet-created", "deeper", "document.txt");
        var missingEvents = Channel.CreateUnbounded<int>();
        using var missing = new FileChangeMonitor(missingPath, () => missingEvents.Writer.TryWrite(0));
        await Next(missingEvents);
        await Observe(missingEvents, () => { Directory.CreateDirectory(Path.GetDirectoryName(missingPath)!); File.WriteAllText(missingPath, "created"); });
        Check(true, "file monitoring observes creation when the initial parent directory is absent");
    }

    private static async Task Disposal(string directory)
    {
        string path = Path.Combine(directory, "disposed.txt");
        await File.WriteAllTextAsync(path, "initial");
        var events = Channel.CreateUnbounded<int>();
        var monitor = new FileChangeMonitor(path, () => events.Writer.TryWrite(0));
        await Next(events);
        File.WriteAllText(path, "queued change");
        monitor.Dispose();
        await Task.Delay(200);
        Check(!events.Reader.TryRead(out _), "disposing a file monitor cancels its pending coalesced notification");
        await File.WriteAllTextAsync(path, "after disposal");
        await Task.Delay(200);
        Check(!events.Reader.TryRead(out _), "disposing a file monitor cancels queued work and releases its old path");
    }

    private static async Task Observe(Channel<int> events, Action change)
    {
        await Quiet(events);
        change();
        await Next(events);
    }

    private static async Task Quiet(Channel<int> events)
    {
        await Task.Delay(200);
        while (events.Reader.TryRead(out _)) { }
    }

    private static async Task<int> Next(Channel<int> events) =>
        await events.Reader.ReadAsync().AsTask().WaitAsync(TimeSpan.FromSeconds(5));
}
#endif

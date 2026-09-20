using System.Diagnostics;
using System.Collections.Concurrent;
using System.Text.Json;
using Microsoft.UI.Dispatching;

namespace Viem.Windows.Diagnostics;

// Opt-in tracing also works in Release, so startup measurements include the
// actual shipped code path. Ordinary launches do not collect or write traces.
internal static class StartupPerformance
{
    private static readonly string? report = Environment.GetEnvironmentVariable("VIEM_STARTUP_REPORT");
    private static readonly long origin = Stopwatch.GetTimestamp();
    private static readonly ConcurrentQueue<object> events = new();
    private static volatile bool completed;
    private static readonly List<object> frames = [];
    private static bool firstDraw;
    internal static void Mark(string name)
    {
        if (report == null || completed) return;
        events.Enqueue(new { name, milliseconds = Stopwatch.GetElapsedTime(origin).TotalMilliseconds });
    }
    internal static Measurement Measure(string name) => new(name, report != null && !completed ? Stopwatch.GetTimestamp() : 0);
    internal readonly struct Measurement(string name, long start) : IDisposable
    {
        public void Dispose()
        {
            if (start == 0 || completed) return;
            events.Enqueue(new { name, milliseconds = Stopwatch.GetElapsedTime(origin).TotalMilliseconds, duration = Stopwatch.GetElapsedTime(start).TotalMilliseconds });
        }
    }
    internal static void FirstDraw(DispatcherQueue dispatcher, string? path, object geometry)
    {
        if (report == null || completed) return;
        if (Environment.GetEnvironmentVariable("VIEM_STARTUP_DOCUMENT") is string target
            && !string.Equals(path, target, StringComparison.OrdinalIgnoreCase)) return;
        if (frames.Count < 32) frames.Add(new { milliseconds = Stopwatch.GetElapsedTime(origin).TotalMilliseconds, geometry });
        if (firstDraw) return;
        firstDraw = true;
        Mark("editor.firstDraw");
        using var process = Process.GetCurrentProcess();
        double processMilliseconds = (DateTime.Now - process.StartTime).TotalMilliseconds;
        bool fontPickerListLoaded = Rendering.FontCatalog.FamilyListLoaded;
        int fontFaceDescriptionsRead = Rendering.FontCatalog.FaceDescriptionsRead;
        dispatcher.TryEnqueue(DispatcherQueuePriority.Low, async () => {
            await Task.Delay(1000);
            completed = true;
            File.WriteAllText(report, JsonSerializer.Serialize(new { processMilliseconds, fontPickerListLoaded, fontFaceDescriptionsRead, events, frames }, new JsonSerializerOptions { WriteIndented = true }));
            if (Environment.GetEnvironmentVariable("VIEM_STARTUP_EXIT") == "1") App.Instance.Exit();
        });
    }
    internal static bool Enabled => report != null && !completed;
}

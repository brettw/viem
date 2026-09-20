using System.Diagnostics;
using System.Collections.Concurrent;
using System.Text.Json;
using System.Runtime;
using System.Runtime.CompilerServices;
using System.Text.Json.Serialization;
using Microsoft.UI.Dispatching;

namespace Viem.Windows.Diagnostics;

// Opt-in tracing also works in Release, so startup measurements include the
// actual shipped code path. Ordinary launches do not collect or write traces.
internal static class StartupPerformance
{
    private static readonly string? report = Environment.GetEnvironmentVariable("VIEM_STARTUP_REPORT");
    private static readonly long origin = Stopwatch.GetTimestamp();
    private static readonly ConcurrentQueue<TraceEvent> events = new();
    private static volatile bool completed;
    private static readonly List<Frame> frames = [];
    private static bool firstDraw;
    internal static void Mark(string name)
    {
        if (report == null || completed) return;
        events.Enqueue(new(name, Stopwatch.GetElapsedTime(origin).TotalMilliseconds, null,
            Environment.CurrentManagedThreadId, JitInfo.GetCompilationTime().TotalMilliseconds,
            JitInfo.GetCompiledMethodCount()));
    }
    internal static Measurement Measure(string name) => new(name, report != null && !completed ? Stopwatch.GetTimestamp() : 0);
    internal readonly struct Measurement(string name, long start) : IDisposable
    {
        public void Dispose()
        {
            if (start == 0 || completed) return;
            events.Enqueue(new(name, Stopwatch.GetElapsedTime(origin).TotalMilliseconds, Stopwatch.GetElapsedTime(start).TotalMilliseconds,
                Environment.CurrentManagedThreadId));
        }
    }
    internal static void FirstDraw(DispatcherQueue dispatcher, string? path, Geometry geometry)
    {
        if (report == null || completed) return;
        if (Environment.GetEnvironmentVariable("VIEM_STARTUP_DOCUMENT") is string target
            && !string.Equals(path, target, StringComparison.OrdinalIgnoreCase)) return;
        if (frames.Count < 32) frames.Add(new(Stopwatch.GetElapsedTime(origin).TotalMilliseconds, geometry));
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
            File.WriteAllText(report, JsonSerializer.Serialize(new TraceReport(processMilliseconds, fontPickerListLoaded, fontFaceDescriptionsRead,
                !RuntimeFeature.IsDynamicCodeSupported, events, frames), StartupJsonContext.Default.TraceReport));
            if (Environment.GetEnvironmentVariable("VIEM_STARTUP_EXIT") == "1") App.Instance.Exit();
        });
    }
    internal static bool Enabled => report != null && !completed;
    internal static void Failed(Exception error)
    {
        if (!Enabled) return;
        completed = true;
        File.WriteAllText(report!, JsonSerializer.Serialize(new Failure(error.ToString(), events), StartupJsonContext.Default.Failure));
        if (Environment.GetEnvironmentVariable("VIEM_STARTUP_EXIT") == "1") Environment.Exit(1);
    }
    internal sealed record TraceEvent(string name, double milliseconds, double? duration, int thread, double? jitMilliseconds = null, long? jitMethods = null);
    internal readonly record struct Geometry(double width, double height, double top, double firstBaseline, double canvasY,
        ulong revision, ulong configuration, long shaped, long fontMetadataReads, long glyphBoundsQueries, long glyphBoundsHits);
    internal sealed record Frame(double milliseconds, Geometry geometry);
    internal sealed record TraceReport(double processMilliseconds, bool fontPickerListLoaded, int fontFaceDescriptionsRead,
        bool nativeAot, ConcurrentQueue<TraceEvent> events, List<Frame> frames);
    internal sealed record Failure(string error, ConcurrentQueue<TraceEvent> events);
}

[JsonSourceGenerationOptions(WriteIndented = true, DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull)]
[JsonSerializable(typeof(StartupPerformance.TraceReport))]
[JsonSerializable(typeof(StartupPerformance.Failure))]
internal partial class StartupJsonContext : JsonSerializerContext;

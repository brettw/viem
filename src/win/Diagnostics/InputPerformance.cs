using System.Diagnostics;
using System.Text.Json;

namespace Viem.Windows.Diagnostics;

// Disabled outside the opt-in UI benchmark; no allocations on ordinary input.
internal static class InputPerformance
{
    private static bool Enabled = false;
    private static readonly Dictionary<string, List<double>> samples = [];
    internal static Measurement Measure(string name) => new(name, Enabled ? Stopwatch.GetTimestamp() : 0);
    internal readonly struct Measurement(string name, long start) : IDisposable
    {
        public void Dispose()
        {
            if (start == 0) return;
            if (!samples.TryGetValue(name, out var values)) samples[name] = values = [];
            values.Add(Stopwatch.GetElapsedTime(start).TotalMilliseconds);
        }
    }
#if DEBUG
    internal static async Task Run(Editor.EditorPane pane, string report)
    {
        var view = await pane.Ready;
        await Task.Delay(300);
        bool resize = Environment.GetEnvironmentVariable("VIEM_PERF_SCENARIO") == "resize";
        bool page = Environment.GetEnvironmentVariable("VIEM_PERF_SCENARIO") == "page";
        string scenario = page ? "page" : resize ? "resize" : "drag";
        float width = (float)pane.Canvas.ActualWidth, height = (float)pane.Canvas.ActualHeight;
        using var surface = new Microsoft.Graphics.Canvas.CanvasRenderTarget(pane.Canvas.Device, (float)pane.Canvas.ActualWidth, (float)pane.Canvas.ActualHeight, pane.Canvas.Dpi);
        view.Place(40f, 40f);
        for (int i = 0; i < 8; i++)
        {
            if (resize) view.Resize(width - i * 12, height);
            else view.Place(50f + i * 10, 60f, true);
            using var drawing = surface.CreateDrawingSession(); pane.Draw(drawing);
        }
        if (page) view.Key(Interop.Native.VIEM_KEY_ESCAPE);
        samples.Clear(); Enabled = true;
        long shaped = view.Provider.ShapedCharacters;
        long metadata = view.Provider.FontMetadataReads;
        int builds = pane.DrawingCacheBuilds;
        int peakRenderResources = view.Provider.LiveResourceCount;
        try
        {
            var random = new Random(42);
            for (int i = 0; i < 100; i++)
            {
                using (Measure(scenario + ".frame.cpu"))
                {
                    using (Measure(scenario + ".turn"))
                    {
                        if (page) view.Key(i < 75 ? Interop.Native.VIEM_KEY_PAGE_DOWN : Interop.Native.VIEM_KEY_PAGE_UP);
                        else if (resize) view.Resize(width - (i < 50 ? i : 99 - i) * 5, height);
                        else view.Place((float)(30 + random.NextDouble() * (pane.Canvas.ActualWidth - 60)), (float)(30 + random.NextDouble() * (pane.Canvas.ActualHeight - 60)), true);
                    }
                    using var drawing = surface.CreateDrawingSession(); pane.Draw(drawing);
                }
                peakRenderResources = Math.Max(peakRenderResources, view.Provider.LiveResourceCount);
                await Task.Delay(16);
            }
        }
        finally { Enabled = false; }
        if (pane.LastError != null) throw pane.LastError;
        var layout = view.Layout();
        var results = samples.ToDictionary(pair => pair.Key, pair => {
            var sorted = pair.Value.Order().ToArray();
            return new { count = sorted.Length, mean = sorted.Average(), p50 = sorted[sorted.Length / 2], p95 = sorted[(int)(sorted.Length * .95)], max = sorted[^1] };
        });
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed = true, scenario, count = 100, shapedCharacters = view.Provider.ShapedCharacters - shaped, fontMetadataReads = view.Provider.FontMetadataReads - metadata, drawingCacheBuilds = pane.DrawingCacheBuilds - builds,
            width, height, dpi = pane.Canvas.Dpi, peakRenderResources,
            rows = layout.Rows.Length, clusters = layout.Clusters.Length, hardLines = layout.Info.coverage_hard_line_end - layout.Info.coverage_hard_line_start,
            milliseconds = results }, new JsonSerializerOptions { WriteIndented = true }));
    }
#endif
}

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
        bool roundtrip = Environment.GetEnvironmentVariable("VIEM_PERF_SCENARIO") == "roundtrip";
        bool scroll = Environment.GetEnvironmentVariable("VIEM_PERF_SCENARIO") == "scroll";
        string scenario = roundtrip ? "roundtrip" : scroll ? "scroll" : page ? "page" : resize ? "resize" : "drag";
        int count = roundtrip ? 120 : 100;
        var steps = new List<object>();
        float width = (float)pane.Canvas.ActualWidth, height = (float)pane.Canvas.ActualHeight;
        using var surface = new Microsoft.Graphics.Canvas.CanvasRenderTarget(pane.Canvas.Device, (float)pane.Canvas.ActualWidth, (float)pane.Canvas.ActualHeight, pane.Canvas.Dpi);
        view.Place(40f, 40f);
        for (int i = 0; i < 8; i++)
        {
            if (resize) view.Resize(width - i * 12, height);
            else view.Place(50f + i * 10, 60f, true);
            using var drawing = surface.CreateDrawingSession(); pane.Draw(drawing);
        }
        if (page || scroll || roundtrip) view.Key(Interop.Native.VIEM_KEY_ESCAPE);
        if (page || scroll || roundtrip) await BackgroundLayoutTests.Idle(view);
        int interval = int.TryParse(Environment.GetEnvironmentVariable("VIEM_PERF_INTERVAL_MS"), out var delay) ? delay : 16;
        samples.Clear(); Enabled = true;
        long shaped = view.Provider.ShapedCharacters;
        long backgroundShaped = view.Provider.BackgroundShapedCharacters;
        long metadata = view.Provider.FontMetadataReads;
        long boundsQueries = view.Provider.GlyphBoundsQueries, boundsHits = view.Provider.GlyphBoundsHits;
        int builds = pane.DrawingCacheBuilds;
        int peakRenderResources = view.Provider.LiveResourceCount;
        try
        {
            var random = new Random(42);
            for (int i = 0; i < count; i++)
            {
                string phase = roundtrip ? new[] { "down.cold", "up.cached", "down.revisit", "up.revisit" }[i / 30] : (i < 75 ? "down" : "up");
                long started = Stopwatch.GetTimestamp(), beforeShape = view.Provider.ShapedCharacters;
                var beforePause = GC.GetTotalPauseDuration();
                long beforeAllocation = GC.GetAllocatedBytesForCurrentThread();
                int beforeBuild = pane.DrawingCacheBuilds;
                using (Measure(scenario + ".frame.cpu"))
                {
                    using (Measure(scenario + ".turn"))
                    {
                        if (scroll) { var origin = view.Viewport; view.Scroll(origin.left, Math.Max(0, origin.top + (i < 75 ? 60 : -60))); }
                        else if (page || roundtrip) view.Key((roundtrip ? (i / 30) % 2 == 0 : i < 75) ? Interop.Native.VIEM_KEY_PAGE_DOWN : Interop.Native.VIEM_KEY_PAGE_UP);
                        else if (resize) view.Resize(width - (i < 50 ? i : 99 - i) * 5, height);
                        else view.Place((float)(30 + random.NextDouble() * (pane.Canvas.ActualWidth - 60)), (float)(30 + random.NextDouble() * (pane.Canvas.ActualHeight - 60)), true);
                    }
                    using var drawing = surface.CreateDrawingSession(); pane.Draw(drawing);
                }
                steps.Add(new { index = i, phase, milliseconds = Stopwatch.GetElapsedTime(started).TotalMilliseconds,
                    gcPauseMilliseconds = (GC.GetTotalPauseDuration() - beforePause).TotalMilliseconds,
                    allocatedBytes = GC.GetAllocatedBytesForCurrentThread() - beforeAllocation,
                    shapedCharacters = view.Provider.ShapedCharacters - beforeShape, drawingBuilds = pane.DrawingCacheBuilds - beforeBuild,
                    top = view.Viewport.top, revision = view.Viewport.layout_revision });
                peakRenderResources = Math.Max(peakRenderResources, view.Provider.LiveResourceCount);
                await Task.Delay(interval);
            }
        }
        finally { Enabled = false; }
        if (pane.LastError != null) throw pane.LastError;
        if (view.BackgroundLayout.LastError != null) throw new InvalidOperationException(view.BackgroundLayout.LastError);
        var layout = view.Layout();
        var results = samples.ToDictionary(pair => pair.Key, pair => {
            var sorted = pair.Value.Order().ToArray();
            return new { count = sorted.Length, mean = sorted.Average(), p50 = sorted[sorted.Length / 2], p95 = sorted[(int)(sorted.Length * .95)], max = sorted[^1] };
        });
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed = true, scenario, count, steps, shapedCharacters = view.Provider.ShapedCharacters - shaped, fontMetadataReads = view.Provider.FontMetadataReads - metadata, drawingCacheBuilds = pane.DrawingCacheBuilds - builds,
            width, height, dpi = pane.Canvas.Dpi, peakRenderResources, interval,
            glyphBoundsQueries = view.Provider.GlyphBoundsQueries - boundsQueries, glyphBoundsHits = view.Provider.GlyphBoundsHits - boundsHits,
            backgroundLayout = view.BackgroundLayout.Enabled, backgroundShapedCharacters = view.Provider.BackgroundShapedCharacters - backgroundShaped,
            backgroundJobs = new { view.BackgroundLayout.Started, view.BackgroundLayout.Installed, view.BackgroundLayout.Discarded },
            rows = layout.Rows.Length, clusters = layout.Clusters.Length, hardLines = layout.Info.coverage_hard_line_end - layout.Info.coverage_hard_line_start,
            milliseconds = results }, new JsonSerializerOptions { WriteIndented = true }));
    }
#endif
}

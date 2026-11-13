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
    private static async Task WaitForPaint(Editor.EditorPane pane, ulong revision)
    {
        var rendered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        void Frame(object? sender, object args) {
            if (pane.PaintedRevisionForTesting == revision) rendered.TrySetResult();
        }
        Microsoft.UI.Xaml.Media.CompositionTarget.Rendered += Frame;
        try { await rendered.Task.WaitAsync(TimeSpan.FromSeconds(5)); }
        finally { Microsoft.UI.Xaml.Media.CompositionTarget.Rendered -= Frame; }
    }

    internal static async Task Run(Editor.EditorPane pane, string report)
    {
        var view = await pane.Ready;
        await Task.Delay(300);
        string scenario = Environment.GetEnvironmentVariable("VIEM_PERF_SCENARIO") ?? "drag";
        bool resize = scenario == "resize", page = scenario == "page", roundtrip = scenario == "roundtrip";
        bool scroll = scenario == "scroll", typing = scenario == "typing";
        // Markdown Preview on/off from a caret in the middle of the document,
        // as the toolbar and title-bar buttons do.
        bool preview = scenario == "preview";
        if (typing && Environment.GetEnvironmentVariable("VIEM_PERF_BASE") != null) scenario = "diff-typing";
        int count = preview ? 20 : roundtrip ? 120 : 100;
        var steps = new List<InputPerformanceStep>();
        float width = (float)pane.Canvas.ActualWidth, height = (float)pane.Canvas.ActualHeight;
        using var surface = new Microsoft.Graphics.Canvas.CanvasRenderTarget(pane.Canvas.Device, (float)pane.Canvas.ActualWidth, (float)pane.Canvas.ActualHeight, pane.Canvas.Dpi);
        view.Place(40f, 40f);
        for (int i = 0; i < 8; i++)
        {
            if (resize) view.Resize(width - i * 12, height);
            else view.Place(50f + i * 10, 60f, true);
            using var drawing = surface.CreateDrawingSession(); pane.Draw(drawing);
        }
        if (page || scroll || roundtrip || preview || typing) view.Key(Interop.Native.VIEM_KEY_ESCAPE);
        if (preview || typing) view.Command("50%");
        if (typing) {
            view.Number(true);
            view.Command("zzA");
            view.Key(Interop.Native.VIEM_KEY_ENTER);
            view.Text("warmup");
            view.Key(Interop.Native.VIEM_KEY_ESCAPE);
            view.Command("u50%zzA");
            if (view.Document.IsDirty) throw new InvalidOperationException("Typing warmup did not restore the input file.");
        }
        if (page || scroll || roundtrip || preview || typing) await BackgroundLayoutTests.Idle(view);
        if (typing) {
            width = (float)pane.Canvas.ActualWidth;
            height = (float)pane.Canvas.ActualHeight;
        }
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
                string phase = scenario switch {
                    "typing" or "diff-typing" => i % 10 == 9 ? "enter" : "character",
                    "preview" => i % 2 == 0 ? "code" : "preview",
                    "roundtrip" => new[] { "down.cold", "up.cached", "down.revisit", "up.revisit" }[i / 30],
                    _ => i < 75 ? "down" : "up"
                };
                long started = Stopwatch.GetTimestamp(), beforeShape = view.Provider.ShapedCharacters;
                var beforePause = GC.GetTotalPauseDuration();
                long beforeAllocation = GC.GetAllocatedBytesForCurrentThread();
                int beforeBuild = pane.DrawingCacheBuilds;
                using (Measure(scenario + (typing ? ".toPaint" : ".frame.cpu")))
                {
                    using (Measure(typing ? scenario + "." + phase + ".turn" : scenario + ".turn"))
                    {
                        if (typing) {
                            if (phase == "enter") view.Key(Interop.Native.VIEM_KEY_ENTER);
                            else view.Text("x");
                        }
                        else if (preview) view.SetMarkdownPreview(i % 2 != 0);
                        else if (scroll) { var origin = view.Viewport; view.Scroll(origin.left, Math.Max(0, origin.top + (i < 75 ? 60 : -60))); }
                        else if (page || roundtrip) view.Key((roundtrip ? (i / 30) % 2 == 0 : i < 75) ? Interop.Native.VIEM_KEY_PAGE_DOWN : Interop.Native.VIEM_KEY_PAGE_UP);
                        else if (resize) view.Resize(width - (i < 50 ? i : 99 - i) * 5, height);
                        else view.Place((float)(30 + random.NextDouble() * (pane.Canvas.ActualWidth - 60)), (float)(30 + random.NextDouble() * (pane.Canvas.ActualHeight - 60)), true);
                    }
                    if (typing) await WaitForPaint(pane, view.State.document_revision);
                    else { using var drawing = surface.CreateDrawingSession(); pane.Draw(drawing); }
                }
                steps.Add(new(i, phase, Stopwatch.GetElapsedTime(started).TotalMilliseconds,
                    (GC.GetTotalPauseDuration() - beforePause).TotalMilliseconds,
                    GC.GetAllocatedBytesForCurrentThread() - beforeAllocation,
                    view.Provider.ShapedCharacters - beforeShape, pane.DrawingCacheBuilds - beforeBuild,
                    view.Viewport.top, view.Viewport.layout_revision));
                peakRenderResources = Math.Max(peakRenderResources, view.Provider.LiveResourceCount);
                // Let speculation settle between toggles so each is measured
                // from the state a reader would toggle from.
                if (preview) { Enabled = false; await BackgroundLayoutTests.Idle(view); await Task.Delay(200); Enabled = true; }
                await Task.Delay(interval);
            }
        }
        finally { Enabled = false; }
        if (pane.LastError != null) throw pane.LastError;
        if (view.BackgroundLayout.LastError != null) throw new InvalidOperationException(view.BackgroundLayout.LastError);
        var layout = view.Layout();
        var results = samples.ToDictionary(pair => pair.Key, pair => {
            var sorted = pair.Value.Order().ToArray();
            return new InputPerformanceStatistics(sorted.Length, sorted.Average(), sorted[sorted.Length / 2], sorted[(int)(sorted.Length * .95)], sorted[^1]);
        });
        File.WriteAllText(report, JsonSerializer.Serialize(new InputPerformanceReport(true, scenario, count, steps,
            view.Provider.ShapedCharacters - shaped, view.Provider.FontMetadataReads - metadata, pane.DrawingCacheBuilds - builds,
            width, height, pane.Canvas.Dpi, peakRenderResources, interval,
            view.Provider.GlyphBoundsQueries - boundsQueries, view.Provider.GlyphBoundsHits - boundsHits,
            view.BackgroundLayout.Enabled, view.Provider.BackgroundShapedCharacters - backgroundShaped,
            new(view.BackgroundLayout.Started, view.BackgroundLayout.Installed, view.BackgroundLayout.Discarded),
            layout.Rows.Length, layout.Clusters.Length, layout.Info.coverage_hard_line_end - layout.Info.coverage_hard_line_start,
            results), NativeTestJsonContext.Default.InputPerformanceReport));
    }
#endif
}

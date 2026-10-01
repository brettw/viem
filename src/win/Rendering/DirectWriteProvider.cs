using System.Globalization;
using System.Collections.Concurrent;
using System.Numerics;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Text;
using Microsoft.Graphics.Canvas.Brushes;
using Microsoft.UI.Dispatching;
using Viem.Windows.Interop;
using Windows.UI;
using Windows.UI.Text;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Rendering;

/// <summary>
/// DirectWrite shapes bounded, contextual fragments. Rust owns wrapping, positions,
/// cache invalidation and hit testing. Only immutable glyph resources stay here.
/// Each core fragment lease pins its resources independently of response storage.
/// </summary>
internal sealed unsafe partial class DirectWriteProvider : IDisposable
{
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)] private delegate ulong GenerationCallback(nint context);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)] private delegate uint ShapeCallback(nint context, ViemShapeRequestV1* requests, ulong count, ViemShapeResponseV1* responses, ulong capacity);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)] private delegate nint RetainCallback(nint context, ViemRenderRunHandleV1* handles, ulong count);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)] private delegate void ReleaseCallback(nint lease);
    private static long nextOwner;
    private readonly SharedResources shared;
    private ulong owner => shared.Owner;
    private readonly ulong? frozenGeneration;
    private bool IsWorker => frozenGeneration.HasValue;
    private CanvasDevice device;
    private CanvasRenderTarget measurement;
    private readonly DispatcherQueue dispatcher;
    private readonly GenerationCallback generationCallback;
    private readonly ShapeCallback shapeCallback;
    private readonly RetainCallback retainCallback;
    // A lease may be released after its provider has been detached.
    private static readonly ReleaseCallback releaseCallback = Release;
    private ConcurrentDictionary<ulong, Resource> resources => shared.Resources;
    private readonly List<ulong> responseResources = [];
    private NativeArena arena = new();
    private readonly Dictionary<GlyphInkKey, global::Windows.Foundation.Rect> inkBounds = new();
    private readonly Dictionary<nint, GlyphFontMetadata> fontMetadata = new();
    private ulong inkGeneration;
    internal int CachedGlyphBounds => inkBounds.Count;
    public ulong Generation => frozenGeneration ?? (ulong)Volatile.Read(ref shared.Generation);
    public long ShapedCharacters { get; private set; }
    public long BackgroundShapedCharacters => Interlocked.Read(ref shared.BackgroundCharacters);
    public int BackgroundShapingThread => Volatile.Read(ref shared.BackgroundThread);
    public long FontMetadataReads { get; private set; }
    public long GlyphBoundsQueries { get; private set; }
    public long GlyphBoundsHits { get; private set; }
    public string? LastError { get; private set; }
    public int LiveResourceCount => resources.Count;
#if DEBUG
    internal int FailShapingBatchesForTest;
    private static readonly bool verifyGlyphOrigins = Diagnostics.FrontendSmokeTests.ReportPath != null
        && Environment.GetEnvironmentVariable("VIEM_PERF_DOCUMENT") == null;
    internal string[] RenderedFontNames(ulong handle) => resources.TryGetValue(handle, out var resource)
        ? resource.Parts.SelectMany(p => p.Font.GetInformationalStrings(CanvasFontInformation.PostscriptName).Values).Distinct().ToArray() : [];
#endif

    public DirectWriteProvider(CanvasDevice device, DispatcherQueue dispatcher) : this(device, dispatcher, new(), null) { }
    private DirectWriteProvider(CanvasDevice device, DispatcherQueue dispatcher, SharedResources shared, ulong? generation)
    {
        this.shared = shared; frozenGeneration = generation;
        this.device = device;
        measurement = new CanvasRenderTarget(device, 1, 1, 96);
        this.dispatcher = dispatcher;
        generationCallback = _ => Generation;
        shapeCallback = ShapeBatch;
        retainCallback = Retain;
    }
    // Capture on the UI thread, construct on the worker. Each shaper owns its
    // response arena and measuring surface; only leased immutable glyph data
    // is shared. Win2D objects are agile and synchronize their native access.
    public Func<DirectWriteProvider> CaptureWorkerFactory()
    {
        var capturedDevice = device;
        ulong generation = Generation;
        return () => new(capturedDevice, dispatcher, shared, generation);
    }
    public ViemTextMeasurementProviderV1 Table => new()
    {
        struct_size = (uint)sizeof(ViemTextMeasurementProviderV1), abi_version = 3,
        measurement_environment_id = owner, threading = VIEM_PROVIDER_THREADING_ANY_WORKER,
        has_render_run_policy = 1, render_run_owner = owner,
        render_run_threading = VIEM_RENDER_THREADING_ANY,
        metrics_generation = Marshal.GetFunctionPointerForDelegate(generationCallback),
        shape_batch = Marshal.GetFunctionPointerForDelegate(shapeCallback),
        retain_render_runs = Marshal.GetFunctionPointerForDelegate(retainCallback),
        release_render_runs = Marshal.GetFunctionPointerForDelegate(releaseCallback)
    };

    public void InvalidateMetrics() => Interlocked.Increment(ref shared.Generation);
    public void ResetDevice(CanvasDevice replacement)
    {
        var next = new CanvasRenderTarget(replacement, 1, 1, 96);
        var previous = measurement;
        measurement = next; device = replacement; InvalidateMetrics();
        previous.Dispose();
    }
    public bool IsColorGlyph(ViemRenderRunHandleV1 handle) => resources.TryGetValue(handle.identifier, out var resource) && resource.ColorGlyph;
    public void Draw(CanvasDrawingSession drawing, ViemRenderRunHandleV1 handle, Vector2 baseline, Color color)
    {
        using var batch = BeginDrawing(drawing);
        batch.Draw(handle, baseline, color);
    }

    public GlyphDrawingBatch BeginDrawing(CanvasDrawingSession drawing, bool combine = true) => new(this, drawing, combine);

    // Reflow changes glyph positions, so text commands must be recorded again.
    // Share immutable brushes across that recording instead of creating and
    // releasing a native COM brush for every individual shaped cluster.
    public sealed class GlyphDrawingBatch(DirectWriteProvider provider, CanvasDrawingSession drawing, bool combine) : IDisposable
    {
        private readonly Dictionary<Color, CanvasSolidColorBrush> brushes = [];
        private readonly List<CanvasGlyph> glyphs = [];
        private GlyphFontMetadata? font;
        private float size, advance;
        private Vector2 origin;
        private CanvasSolidColorBrush? runBrush;
        public int DrawCalls { get; private set; }
        public void Draw(ViemRenderRunHandleV1 handle, Vector2 baseline, Color color)
        {
            if (!brushes.TryGetValue(color, out var brush)) brushes[color] = brush = new(drawing, color);
            if (handle.owner != provider.owner || handle.metrics_generation != provider.Generation
                || !provider.resources.TryGetValue(handle.identifier, out var resource)) return;
            if (!combine || resource.ColorGlyph)
            {
                Flush(); provider.Draw(drawing, handle, baseline, color, brush); DrawCalls++;
                return;
            }
            foreach (var part in resource.Parts)
            {
                Vector2 position = baseline + part.Offset;
                // Keep RTL and exceptional positioning on the original path.
                // Ordinary adjacent LTR clusters share one native glyph call.
                if (part.BidiLevel != 0)
                {
                    Flush(); drawing.DrawGlyphRun(position, part.Font, part.Size, part.Glyphs, false, part.BidiLevel, brush); DrawCalls++;
                    continue;
                }
                if (font != part.Metadata || size != part.Size || runBrush != brush
                    || position.Y != origin.Y || Math.Abs(position.X - (origin.X + advance)) > .01f || glyphs.Count >= 1024)
                    Flush();
                if (glyphs.Count == 0) { font = part.Metadata; size = part.Size; origin = position; runBrush = brush; }
                float correction = position.X - (origin.X + advance);
                foreach (var source in part.Glyphs)
                {
                    var glyph = source;
                    glyph.AdvanceOffset += correction;
                    glyphs.Add(glyph); advance += glyph.Advance;
                }
            }
        }
        public void Flush()
        {
            if (glyphs.Count == 0) return;
            drawing.DrawGlyphRun(origin, font!.Font, size, glyphs.ToArray(), false, 0, runBrush); DrawCalls++;
            glyphs.Clear(); advance = 0; font = null;
        }
        public void Dispose() { try { Flush(); } finally { foreach (var brush in brushes.Values) brush.Dispose(); } }
    }

    private void Draw(CanvasDrawingSession drawing, ViemRenderRunHandleV1 handle, Vector2 baseline, Color color, CanvasSolidColorBrush brush)
    {
        if (handle.owner != owner || handle.metrics_generation != Generation || !resources.TryGetValue(handle.identifier, out var resource)) return;
        if (resource.ColorGlyph)
        {
            // DrawTextLayout performs DirectWrite's color-font translation.
            // Restrict this exceptional pass to the leased color cluster.
            var bounds = resource.Bounds;
            using var clip = drawing.CreateLayer(1, new global::Windows.Foundation.Rect(baseline.X + bounds.x - 1, baseline.Y + bounds.y - 1, bounds.width + 2, bounds.height + 2));
            drawing.DrawTextLayout(resource.Fragment.Layout, baseline.X - resource.Left, baseline.Y - resource.Baseline, color);
            return;
        }
        foreach (var part in resource.Parts)
            drawing.DrawGlyphRun(baseline + part.Offset, part.Font, part.Size, part.Glyphs, false, part.BidiLevel, brush);
    }

    private uint ShapeBatch(nint context, ViemShapeRequestV1* requests, ulong count, ViemShapeResponseV1* responses, ulong capacity)
    {
        using var startup = Diagnostics.StartupPerformance.Measure(IsWorker ? "shape.worker" : "shape.foreground");
        using var timing = IsWorker ? default(Diagnostics.InputPerformance.Measurement) : Diagnostics.InputPerformance.Measure("shape.batch");
        try
        {
            if (capacity < count) return VIEM_STATUS_BUFFER_TOO_SMALL;
            arena.Dispose();
            arena = new();
            foreach (var id in responseResources) ReleaseResource(id);
            responseResources.Clear();
#if DEBUG
            if (FailShapingBatchesForTest > 0) { FailShapingBatchesForTest--; throw new InvalidOperationException("Injected native shaping failure."); }
#endif
            for (ulong i = 0; i < count; i++) responses[i] = Shape(requests[i]);
            LastError = null;
            return VIEM_STATUS_OK;
        }
        catch (Exception error)
        {
            LastError = error.ToString();
            // No response from a failed batch is accepted or leased by core.
            // Dispose partial native output before the default-font retry.
            foreach (var id in responseResources) ReleaseResource(id);
            responseResources.Clear();
            arena.Dispose(); arena = new();
            return VIEM_STATUS_PROVIDER_FAILURE; // Managed exceptions never cross the ABI.
        }
    }

    private ViemShapeResponseV1 Shape(ViemShapeRequestV1 request)
    {
        if (inkGeneration != Generation) { inkBounds.Clear(); fontMetadata.Clear(); inkGeneration = Generation; }
        string before = Text(request.context_before), interior = Text(request.text), after = Text(request.context_after);
        string text = before + interior + after;
        var map = new Utf8IndexMap(text);
        long contextStart = checked((long)request.text_start - (long)request.context_before.length);
        var fontTiming = Diagnostics.StartupPerformance.Measure("shape.fontSetup");
        var fontCpu = IsWorker ? default : Diagnostics.InputPerformance.Measure("shape.fontSetup");
        var defaultFont = ResolveFont(request.default_style);
        using var format = new CanvasTextFormat
        {
            FontFamily = FontCatalog.RenderingFamily(defaultFont.Family, request.default_style.weight, Slant(request.default_style.slant), defaultFont.Stretch),
            FontStretch = defaultFont.Stretch, FontSize = request.default_style.size * request.scale,
            FontWeight = new FontWeight { Weight = (ushort)Math.Clamp(request.default_style.weight, 1, 999) },
            FontStyle = Slant(request.default_style.slant), WordWrapping = CanvasWordWrapping.NoWrap,
            Direction = request.paragraph_base_direction == VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT
                ? CanvasTextDirection.RightToLeftThenTopToBottom : CanvasTextDirection.LeftToRightThenTopToBottom
        };
        // NoWrap permits overflow. A small width keeps RTL coordinates near
        // the text and avoids losing fractional precision at a huge right edge.
        // Rust owns the real wrap width and final paragraph placement.
        var layout = new CanvasTextLayout(device, text.Length == 0 ? " " : text, format, 1, 16_777_216);
        var fragment = new Fragment(layout);
        using var unleasedLayout = new UnleasedLayout(fragment);
        layout.Options = CanvasDrawTextOptions.EnableColorFont;
        using var measuring = measurement.CreateDrawingSession();
        ApplyStyle(layout, 0, Math.Max(text.Length, 1), request.default_style, request.scale);
        for (ulong i = 0; i < request.style_run_count; i++)
        {
            var run = request.style_runs[i];
            int first = map.Utf16((int)Math.Max(0, (long)run.text_start - contextStart));
            int last = map.Utf16((int)Math.Min(map.ByteLength, (long)run.text_end - contextStart));
            if (last > first) ApplyStyle(layout, first, last - first, run.style, request.scale);
        }
        var capture = new GlyphCapture(GetFontMetadata);
        fontTiming.Dispose();
        fontCpu.Dispose();
        using (Diagnostics.StartupPerformance.Measure("shape.nativeCapture"))
        using (IsWorker ? default : Diagnostics.InputPerformance.Measure("shape.nativeCapture")) layout.DrawToTextRenderer(capture, Vector2.Zero);
        using var clusterTiming = Diagnostics.StartupPerformance.Measure("shape.clusters");
        using var clusterCpu = IsWorker ? default : Diagnostics.InputPerformance.Measure("shape.clusters");
        var line = layout.LineMetrics[0];
        var defaultMetrics = new ViemTextMetricsV1 {
            ascent = Math.Max(0, line.Baseline), descent = Math.Max(0, line.Height - line.Baseline), leading = 0
        };
        var clusters = new List<ViemShapedClusterV1>();
        // The same glyph occurs on many lines. Keep exact native face/size/run
        // arguments across fragments, bounded to 1,024 entries per shaper and
        // scoped to its device/metrics generation. Workers own separate caches.
        var fontNames = new Dictionary<string, ViemUtf8Slice>(StringComparer.Ordinal);
        var markerFonts = new Dictionary<int, MarkerFont>();
        var positions = new List<float>();
        int start = 0;
        var boundaries = new HashSet<int>(StringInfo.ParseCombiningCharacters(text)) { text.Length };
        var nativeClusters = layout.ClusterMetrics;
        var caretStops = new ViemClusterCaretStopV1[nativeClusters.Length * 2];
        for (int index = 0; index < nativeClusters.Length && start < text.Length; index++)
        {
            int end = start + nativeClusters[index].CharacterCount;
            float advance = nativeClusters[index].Width;
            while (end < text.Length && !boundaries.Contains(end) && index + 1 < nativeClusters.Length)
            { index++; end += nativeClusters[index].CharacterCount; advance += nativeClusters[index].Width; }
            int byteStart = map.Utf8(start), byteEnd = map.Utf8(end);
            long globalStart = contextStart + byteStart;
            if (globalStart < (long)request.text_start || globalStart >= (long)request.text_end) { start = end; continue; }
            // The captured native run already contains exact cluster advances.
            // Avoid a COM hit-test/array allocation per ordinary LTR cluster.
            // Split runs and bidi retain DirectWrite's region query.
            bool capturedLeft = capture.TryGetClusterLeft(start, end, out float left);
            if (!capturedLeft)
            {
                var regions = layout.GetCharacterRegions(start, end - start);
                left = regions.Length == 0 ? layout.GetCaretPosition(start, false).X : (float)regions.Min(r => r.LayoutBounds.X);
            }
#if DEBUG
            if (capturedLeft && verifyGlyphOrigins)
            {
                var regions = layout.GetCharacterRegions(start, end - start);
                float expected = regions.Length == 0 ? layout.GetCaretPosition(start, false).X : (float)regions.Min(r => r.LayoutBounds.X);
                if (Math.Abs(expected - left) > .005f) throw new InvalidOperationException($"Captured glyph origin differs from DirectWrite: {left} != {expected} at {start}.");
            }
#endif
            var parts = capture.Extract(start, end, left, line.Baseline);
            uint bidi = parts.Count == 0 ? 0 : parts[0].BidiLevel;
            var style = request.default_style;
            int styleIndex = -1;
            for (ulong r = 0; r < request.style_run_count; r++)
                if (request.style_runs[r].text_start <= (ulong)globalStart && request.style_runs[r].text_end > (ulong)globalStart) { style = request.style_runs[r].style; styleIndex = (int)r; }
            float ascent = parts.Count == 0 ? defaultMetrics.ascent : parts.Max(p => p.Metadata.Ascent * p.Size);
            float descent = parts.Count == 0 ? defaultMetrics.descent : parts.Max(p => p.Metadata.Descent * p.Size);
            float leading = parts.Count == 0 ? 0 : parts.Max(p => Math.Max(0, p.Metadata.LineGap * p.Size));
            var cluster = New<ViemShapedClusterV1>();
            cluster.text_start = (ulong)globalStart;
            cluster.text_end = checked((ulong)(contextStart + byteEnd));
            cluster.advance = Math.Max(0, advance);
            cluster.metrics = new() { ascent = Math.Max(0, ascent), descent = Math.Max(0, descent), leading = leading };
            cluster.typographic_bounds = new() { y = -ascent, width = cluster.advance, height = Math.Max(0, ascent + descent) };
            cluster.ink_bounds = cluster.typographic_bounds;
            foreach (var part in parts)
            {
                GlyphInkKey? key = part.Glyphs.Length == 1 ? new(part.Metadata, part.Size, part.Glyphs[0].Index,
                    part.Glyphs[0].Advance, part.Glyphs[0].AdvanceOffset, part.Glyphs[0].AscenderOffset, part.BidiLevel, part.Offset) : null;
                global::Windows.Foundation.Rect ink;
                if (key is { } existing && inkBounds.TryGetValue(existing, out ink)) GlyphBoundsHits++;
                else
                {
                    ink = part.Font.GetGlyphRunBounds(measuring, part.Offset, part.Size, part.Glyphs, false, part.BidiLevel);
                    GlyphBoundsQueries++;
                    if (key is { } fresh && inkBounds.Count < 1024) inkBounds[fresh] = ink;
                }
                var previous = cluster.ink_bounds;
                float x = Math.Min(previous.x, (float)ink.X), y = Math.Min(previous.y, (float)ink.Y);
                cluster.ink_bounds = new() { x = x, y = y, width = Math.Max(previous.x + previous.width, (float)ink.Right) - x, height = Math.Max(previous.y + previous.height, (float)ink.Bottom) - y };
            }
            cluster.bidi_level = bidi;
            string family = (parts.Count == 0 ? null : parts[0].Metadata.Family) ?? ResolveFont(style).Family;
            if (!fontNames.TryGetValue(family, out var nativeName)) fontNames[family] = nativeName = arena.Utf8(family);
            cluster.fallback_font = nativeName;
            caretStops[clusters.Count * 2] = new() { text_offset = cluster.text_start, inline_offset = (bidi & 1) == 0 ? 0 : cluster.advance, affinity = VIEM_BOUNDARY_AFFINITY_DOWNSTREAM };
            caretStops[clusters.Count * 2 + 1] = new() { text_offset = cluster.text_end, inline_offset = (bidi & 1) == 0 ? cluster.advance : 0, affinity = VIEM_BOUNDARY_AFFINITY_UPSTREAM };
            cluster.caret_stop_count = 2;
            if (request.purpose == VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA)
            {
                ulong id = (ulong)Interlocked.Increment(ref shared.NextResource);
                if (!markerFonts.TryGetValue(styleIndex, out var markerFont)) markerFonts[styleIndex] = markerFont = MarkerFont.From(style, request.scale);
                if (!resources.TryAdd(id, new Resource(fragment, parts, left, line.Baseline, cluster.ink_bounds, markerFont)))
                    throw new InvalidOperationException("Duplicate glyph resource identity.");
                Interlocked.Increment(ref fragment.References);
                responseResources.Add(id);
                cluster.has_render_run = 1;
                cluster.render_run = new() { owner = owner, identifier = id, metrics_generation = Generation, threading = VIEM_RENDER_THREADING_ANY };
            }
            clusters.Add(cluster);
            positions.Add(left);
            start = end;
        }
        ShapedCharacters += interior.Length;
        if (IsWorker)
        {
            Interlocked.Add(ref shared.BackgroundCharacters, interior.Length);
            Volatile.Write(ref shared.BackgroundThread, Environment.CurrentManagedThreadId);
        }
        ulong[] order = Enumerable.Range(0, clusters.Count).OrderBy(i => positions[i]).Select(i => (ulong)i).ToArray();
        // One ABI caret allocation for the fragment, not one per character.
        var nativeStops = arena.Copy<ViemClusterCaretStopV1>(caretStops.AsSpan(0, clusters.Count * 2));
        var nativeGeometry = arena.Copy<ViemShapedClusterV1>(CollectionsMarshal.AsSpan(clusters));
        for (int i = 0; i < clusters.Count; i++) nativeGeometry[i].caret_stops = nativeStops + i * 2;
        return new()
        {
            struct_size = (uint)sizeof(ViemShapeResponseV1), document_id = request.document_id, document_revision = request.document_revision,
            measurement_environment_id = request.measurement_environment_id, metrics_generation = request.metrics_generation,
            text_start = request.text_start, text_end = request.text_end, default_metrics = defaultMetrics,
            clusters = nativeGeometry, cluster_count = (ulong)clusters.Count,
            visual_order = arena.Copy<ulong>(order), visual_order_count = (ulong)order.Length
        };
    }

    // A fragment with published resources is released by their leases. Before
    // its first resource exists, any native shaping exception must release it.
    private sealed class UnleasedLayout(Fragment fragment) : IDisposable
    {
        public void Dispose() { if (fragment.References == 0) fragment.Layout.Dispose(); }
    }

    private readonly record struct GlyphInkKey(GlyphFontMetadata Font, float Size, int Index,
        float Advance, float AdvanceOffset, float AscenderOffset, uint Bidi, Vector2 Offset);

    private static (string Family, FontStretch Stretch) ResolveFont(ViemResolvedTextStyleV1 style)
    {
        for (ulong i = 0; i < style.font_family_count; i++)
        {
            if (FontCatalog.Resolve(Text(style.font_families[i])) is { } resolved) return resolved;
        }
        return ("Segoe UI", FontStretch.Normal);
    }
    private static FontStyle Slant(uint slant) => slant switch { 1 => FontStyle.Italic, 2 => FontStyle.Oblique, _ => FontStyle.Normal };
    private static void ApplyStyle(CanvasTextLayout layout, int start, int count, ViemResolvedTextStyleV1 style, float scale)
    {
        var font = ResolveFont(style);
        layout.SetFontFamily(start, count, FontCatalog.RenderingFamily(font.Family, style.weight, Slant(style.slant), font.Stretch));
        layout.SetFontStretch(start, count, font.Stretch);
        layout.SetFontSize(start, count, style.size * scale);
        layout.SetFontWeight(start, count, new FontWeight { Weight = (ushort)Math.Clamp(style.weight, 1, 999) });
        layout.SetFontStyle(start, count, Slant(style.slant));
        layout.SetCharacterSpacing(start, count, 0, style.letter_spacing * scale, 0);
        if (style.has_language != 0) layout.SetLocaleName(start, count, Text(style.language));
        if (style.feature_count != 0)
        {
            using var typography = new CanvasTypography();
            for (ulong i = 0; i < style.feature_count; i++)
            {
                var feature = style.features[i];
                uint tag = feature.tag[0] | ((uint)feature.tag[1] << 8) | ((uint)feature.tag[2] << 16) | ((uint)feature.tag[3] << 24);
                typography.AddFeature((CanvasTypographyFeatureName)tag, feature.value);
            }
            layout.SetTypography(start, count, typography);
        }
    }

    private nint Retain(nint context, ViemRenderRunHandleV1* handles, ulong count)
    {
        try
        {
            var ids = new ulong[checked((int)count)];
            for (int i = 0; i < ids.Length; i++)
            {
                if (handles[i].owner != owner || !resources.ContainsKey(handles[i].identifier)) return 0;
                ids[i] = handles[i].identifier;
            }
            foreach (ulong id in ids) Interlocked.Increment(ref resources[id].References);
            return GCHandle.ToIntPtr(GCHandle.Alloc(new Lease(this, ids)));
        }
        catch { return 0; }
    }
    private static void Release(nint pointer)
    {
        if (pointer == 0) return;
        var handle = GCHandle.FromIntPtr(pointer);
        var lease = (Lease)handle.Target!;
        handle.Free();
        foreach (ulong id in lease.Ids) lease.Provider.ReleaseResource(id);
    }
    private void ReleaseResource(ulong id)
    {
        if (!resources.TryGetValue(id, out var resource) || Interlocked.Decrement(ref resource.References) != 0) return;
        resources.TryRemove(id, out _);
        if (Interlocked.Decrement(ref resource.Fragment.References) == 0) resource.Fragment.Layout.Dispose();
    }
    public void Dispose()
    {
        arena.Dispose();
        foreach (ulong id in responseResources) ReleaseResource(id);
        responseResources.Clear();
        inkBounds.Clear();
        fontMetadata.Clear();
        measurement.Dispose();
        GC.KeepAlive(generationCallback); GC.KeepAlive(shapeCallback); GC.KeepAlive(retainCallback);
    }
    private sealed record Lease(DirectWriteProvider Provider, ulong[] Ids);
    private GlyphFontMetadata GetFontMetadata(CanvasFontFace font)
    {
        nint reference = GlyphFontMetadata.NativeReference(font);
        try
        {
            if (reference != 0)
                foreach (var entry in fontMetadata)
                    if (GlyphFontMetadata.SameFace(reference, entry.Key)) return entry.Value;
            FontMetadataReads++;
            var metadata = new GlyphFontMetadata(font);
            // CanvasFontFace owns the native reference. DirectWrite's Equals
            // compares faces (including simulations), not wrapper addresses.
            if (reference != 0 && fontMetadata.Count < 32) fontMetadata[reference] = metadata;
            return metadata;
        }
        finally { if (reference != 0) Marshal.Release(reference); GC.KeepAlive(font); }
    }
    private sealed class SharedResources
    {
        public readonly ulong Owner = (ulong)Interlocked.Increment(ref nextOwner);
        public long Generation = 1, NextResource, BackgroundCharacters;
        public int BackgroundThread;
        public readonly ConcurrentDictionary<ulong, Resource> Resources = new();
    }
    private sealed class Fragment(CanvasTextLayout layout) { public CanvasTextLayout Layout = layout; public int References; }
    private sealed class Resource(Fragment fragment, List<GlyphPart> parts, float left, float baseline, ViemShapedBoundsV1 bounds, MarkerFont markerFont)
    {
        public MarkerFont MarkerFont = markerFont;
        public Fragment Fragment = fragment; public List<GlyphPart> Parts = parts; public int References = 1;
        public float Left = left, Baseline = baseline; public ViemShapedBoundsV1 Bounds = bounds;
        public bool ColorGlyph = parts.Any(p => p.Metadata.ColorGlyph);
    }
}

// DirectWrite's localized font names cross the COM boundary and allocate a
// dictionary. Win2D can create a new CanvasFontFace wrapper for every text run;
// its underlying DirectWrite face is the stable identity shared between lines.
internal sealed unsafe class GlyphFontMetadata
{
    internal static nint NativeReference(CanvasFontFace font)
    {
        // ICanvasResourceWrapperNative from Win2D's Microsoft.Graphics.Canvas.native.h.
        Guid wrapperId = new("5F10688D-EA55-4D55-A3B0-4DDB55C0C20A");
        nint wrapper = 0;
        try
        {
            if (Marshal.QueryInterface(((WinRT.IWinRTObject)font).NativeObject.ThisPtr, in wrapperId, out wrapper) < 0) return 0;
            Guid referenceId = new("5E7FA7CA-DDE3-424C-89F0-9FCD6FED58CD"); // IDWriteFontFaceReference, dwrite_3.h
            nint resource = 0;
            var getResource = (delegate* unmanaged[Stdcall]<nint, nint, float, Guid*, nint*, int>)(*(nint**)wrapper)[3];
            if (getResource(wrapper, 0, 0, &referenceId, &resource) >= 0) return resource; // caller releases
            if (resource != 0) Marshal.Release(resource);
            return 0;
        }
        finally
        {
            if (wrapper != 0) Marshal.Release(wrapper);
            GC.KeepAlive(font);
        }
    }
    internal static bool SameFace(nint left, nint right) => left == right
        || ((delegate* unmanaged[Stdcall]<nint, nint, int>)(*(nint**)left)[5])(left, right) != 0;
    public CanvasFontFace Font { get; }
    public string? Family { get; }
    public bool ColorGlyph { get; }
    public float Ascent { get; }
    public float Descent { get; }
    public float LineGap { get; }
    public GlyphFontMetadata(CanvasFontFace font)
    {
        Font = font;
        var names = font.FamilyNames.Values.ToArray();
        Family = names.FirstOrDefault();
        ColorGlyph = names.Any(n => n.Contains("Emoji", StringComparison.OrdinalIgnoreCase));
        Ascent = font.Ascent; Descent = font.Descent; LineGap = font.LineGap;
    }
}
internal readonly record struct GlyphPart(GlyphFontMetadata Metadata, float Size, CanvasGlyph[] Glyphs, uint BidiLevel, Vector2 Offset)
{
    public CanvasFontFace Font => Metadata.Font;
}
// Generate the WinRT callable wrapper at build time instead of discovering
// the renderer interface through reflection on the first shaped fragment.
[WinRT.GeneratedWinRTExposedType]
internal sealed partial class GlyphCapture(Func<CanvasFontFace, GlyphFontMetadata> metadataFor) : ICanvasTextRenderer
{
    private sealed record Run(Vector2 Point, GlyphFontMetadata Font, float Size, CanvasGlyph[] Glyphs, uint Bidi, int[] Map, int Start, Dictionary<int, int> Ends, float[] Advances);
    private readonly List<Run> runs = [];
    public int RunCount => runs.Count;
    public bool PixelSnappingDisabled => true;
    public Matrix3x2 Transform => Matrix3x2.Identity;
    public float Dpi => 96;
    public void DrawGlyphRun(Vector2 point, CanvasFontFace fontFace, float fontSize, CanvasGlyph[] glyphs, bool isSideways, uint bidiLevel, object brush, CanvasTextMeasuringMode measuringMode, string localeName, string textString, int[] clusterMapIndices, uint characterIndex, CanvasGlyphOrientation glyphOrientation)
    {
        var boundaries = clusterMapIndices.Distinct().Order().Append(glyphs.Length).ToArray(); var ends = new Dictionary<int, int>();
        for (int i = 0; i + 1 < boundaries.Length; i++) ends[boundaries[i]] = boundaries[i + 1];
        var advances = new float[glyphs.Length + 1]; for (int i = 0; i < glyphs.Length; i++) advances[i + 1] = advances[i] + glyphs[i].Advance;
        runs.Add(new(point, metadataFor(fontFace), fontSize, glyphs, bidiLevel, clusterMapIndices, checked((int)characterIndex), ends, advances));
    }
    public List<GlyphPart> Extract(int start, int end, float left, float baseline)
    {
        var result = new List<GlyphPart>(1);
        foreach (var run in runs)
        {
            int first = Math.Max(start, run.Start) - run.Start, last = Math.Min(end, run.Start + run.Map.Length) - run.Start;
            if (last <= first) continue;
            int glyphStart = run.Map[first], finalStart = glyphStart;
            for (int i = first + 1; i < last; i++) { glyphStart = Math.Min(glyphStart, run.Map[i]); finalStart = Math.Max(finalStart, run.Map[i]); }
            int glyphEnd = run.Ends[finalStart];
            float offset = run.Advances[glyphStart] * ((run.Bidi & 1) == 0 ? 1 : -1);
            result.Add(new(run.Font, run.Size, run.Glyphs[glyphStart..glyphEnd], run.Bidi, run.Point + new Vector2(offset - left, -baseline)));
        }
        return result;
    }
    public bool TryGetClusterLeft(int start, int end, out float left)
    {
        foreach (var run in runs)
        {
            if (run.Bidi != 0 || start < run.Start || end > run.Start + run.Map.Length) continue;
            int first = start - run.Start, last = end - run.Start;
            int glyph = run.Map[first];
            // Only whole, single native clusters have this simple origin.
            if ((first > 0 && run.Map[first - 1] == glyph) || (last < run.Map.Length && run.Map[last] == glyph)) break;
            bool oneCluster = true;
            for (int i = first + 1; i < last; i++) oneCluster &= run.Map[i] == glyph;
            if (!oneCluster) break;
            left = run.Point.X + run.Advances[glyph];
            return true;
        }
        left = 0;
        return false;
    }
    public void DrawStrikethrough(Vector2 point, float width, float thickness, float offset, CanvasTextDirection direction, object brush, CanvasTextMeasuringMode mode, string locale, CanvasGlyphOrientation orientation) { }
    public void DrawUnderline(Vector2 point, float width, float thickness, float offset, float height, CanvasTextDirection direction, object brush, CanvasTextMeasuringMode mode, string locale, CanvasGlyphOrientation orientation) { }
    public void DrawInlineObject(Vector2 point, ICanvasTextInlineObject inlineObject, bool sideways, bool rightToLeft, object brush, CanvasGlyphOrientation orientation) => throw new NotSupportedException("Unexpected inline object in a text-only shaping request.");
}

internal sealed class Utf8IndexMap
{
    private readonly int[] utf8;
    public int ByteLength => utf8[^1];
    public Utf8IndexMap(string text)
    {
        utf8 = new int[text.Length + 1];
        int index = 0, bytes = 0;
        foreach (var rune in text.EnumerateRunes())
        {
            for (int i = 0; i < rune.Utf16SequenceLength; i++) utf8[index++] = bytes;
            bytes += rune.Utf8SequenceLength;
        }
        utf8[^1] = bytes;
    }
    public int Utf8(int index) => utf8[index];
    public int Utf16(int bytes)
    {
        int index = Array.BinarySearch(utf8, bytes);
        if (index < 0) throw new ArgumentException("Not a UTF-8 scalar boundary.");
        while (index > 0 && utf8[index - 1] == bytes) index--;
        return index;
    }
}

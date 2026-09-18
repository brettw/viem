using System.Globalization;
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
    private readonly ulong owner = (ulong)Interlocked.Increment(ref nextOwner);
    private CanvasDevice device;
    private CanvasRenderTarget measurement;
    private readonly DispatcherQueue dispatcher;
    private readonly GenerationCallback generationCallback;
    private readonly ShapeCallback shapeCallback;
    private readonly RetainCallback retainCallback;
    // A lease may be released after its provider has been detached.
    private static readonly ReleaseCallback releaseCallback = Release;
    private readonly Dictionary<ulong, Resource> resources = [];
    private readonly List<ulong> responseResources = [];
    private NativeArena arena = new();
    private ulong nextResource;
    public ulong Generation { get; private set; } = 1;
    public long ShapedCharacters { get; private set; }
    public string? LastError { get; private set; }
    public int LiveResourceCount => resources.Count;

    public DirectWriteProvider(CanvasDevice device, DispatcherQueue dispatcher)
    {
        this.device = device;
        measurement = new CanvasRenderTarget(device, 1, 1, 96);
        this.dispatcher = dispatcher;
        generationCallback = _ => Generation;
        shapeCallback = ShapeBatch;
        retainCallback = Retain;
    }
    public ViemTextMeasurementProviderV1 Table => new()
    {
        struct_size = (uint)sizeof(ViemTextMeasurementProviderV1), abi_version = 3,
        measurement_environment_id = owner, threading = VIEM_PROVIDER_THREADING_FRONTEND_MAIN,
        has_render_run_policy = 1, render_run_owner = owner,
        render_run_threading = VIEM_RENDER_THREADING_FRONTEND_MAIN,
        metrics_generation = Marshal.GetFunctionPointerForDelegate(generationCallback),
        shape_batch = Marshal.GetFunctionPointerForDelegate(shapeCallback),
        retain_render_runs = Marshal.GetFunctionPointerForDelegate(retainCallback),
        release_render_runs = Marshal.GetFunctionPointerForDelegate(releaseCallback)
    };

    public void InvalidateMetrics() => Generation++;
    public void ResetDevice(CanvasDevice replacement)
    { measurement.Dispose(); device = replacement; measurement = new CanvasRenderTarget(device, 1, 1, 96); InvalidateMetrics(); }
    public bool IsColorGlyph(ViemRenderRunHandleV1 handle) => resources.TryGetValue(handle.identifier, out var resource) && resource.ColorGlyph;
    public void Draw(CanvasDrawingSession drawing, ViemRenderRunHandleV1 handle, Vector2 baseline, Color color)
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
        using var brush = new CanvasSolidColorBrush(drawing, color);
        foreach (var part in resource.Parts)
            drawing.DrawGlyphRun(baseline + part.Offset, part.Font, part.Size, part.Glyphs, false, part.BidiLevel, brush);
    }

    private uint ShapeBatch(nint context, ViemShapeRequestV1* requests, ulong count, ViemShapeResponseV1* responses, ulong capacity)
    {
        try
        {
            if (capacity < count) return VIEM_STATUS_BUFFER_TOO_SMALL;
            arena.Dispose();
            arena = new();
            foreach (var id in responseResources) ReleaseResource(id);
            responseResources.Clear();
            for (ulong i = 0; i < count; i++) responses[i] = Shape(requests[i]);
            LastError = null;
            return VIEM_STATUS_OK;
        }
        catch (Exception error)
        {
            LastError = error.ToString();
            return VIEM_STATUS_PROVIDER_FAILURE; // Managed exceptions never cross the ABI.
        }
    }

    private ViemShapeResponseV1 Shape(ViemShapeRequestV1 request)
    {
        string before = Text(request.context_before), interior = Text(request.text), after = Text(request.context_after);
        string text = before + interior + after;
        var map = new Utf8IndexMap(text);
        long contextStart = checked((long)request.text_start - (long)request.context_before.length);
        using var format = new CanvasTextFormat
        {
            FontFamily = ResolveFamily(request.default_style), FontSize = request.default_style.size * request.scale,
            FontWeight = new FontWeight { Weight = (ushort)Math.Clamp(request.default_style.weight, 1, 999) },
            FontStyle = Slant(request.default_style.slant), WordWrapping = CanvasWordWrapping.NoWrap,
            Direction = request.paragraph_base_direction == VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT
                ? CanvasTextDirection.RightToLeftThenTopToBottom : CanvasTextDirection.LeftToRightThenTopToBottom
        };
        // NoWrap permits overflow. A small width keeps RTL coordinates near
        // the text and avoids losing fractional precision at a huge right edge.
        // Rust owns the real wrap width and final paragraph placement.
        var layout = new CanvasTextLayout(device, text.Length == 0 ? " " : text, format, 1, 16_777_216);
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
        var capture = new GlyphCapture();
        layout.DrawToTextRenderer(capture, Vector2.Zero);
        var fragment = new Fragment(layout);
        var line = layout.LineMetrics[0];
        var defaultMetrics = new ViemTextMetricsV1 { ascent = line.Baseline, descent = Math.Max(0, line.Height - line.Baseline), leading = 0 };
        var clusters = new List<ViemShapedClusterV1>();
        var markerFonts = new Dictionary<int, MarkerFont>();
        var positions = new List<float>();
        int start = 0;
        var boundaries = new HashSet<int>(StringInfo.ParseCombiningCharacters(text)) { text.Length };
        var nativeClusters = layout.ClusterMetrics;
        for (int index = 0; index < nativeClusters.Length && start < text.Length; index++)
        {
            int end = start + nativeClusters[index].CharacterCount;
            float advance = nativeClusters[index].Width;
            while (end < text.Length && !boundaries.Contains(end) && index + 1 < nativeClusters.Length)
            { index++; end += nativeClusters[index].CharacterCount; advance += nativeClusters[index].Width; }
            int byteStart = map.Utf8(start), byteEnd = map.Utf8(end);
            long globalStart = contextStart + byteStart;
            if (globalStart < (long)request.text_start || globalStart >= (long)request.text_end) { start = end; continue; }
            var regions = layout.GetCharacterRegions(start, end - start);
            float left = regions.Length == 0 ? layout.GetCaretPosition(start, false).X : (float)regions.Min(r => r.LayoutBounds.X);
            var parts = capture.Extract(start, end, left, line.Baseline);
            uint bidi = parts.Count == 0 ? 0 : parts[0].BidiLevel;
            var style = request.default_style;
            int styleIndex = -1;
            for (ulong r = 0; r < request.style_run_count; r++)
                if (request.style_runs[r].text_start <= (ulong)globalStart && request.style_runs[r].text_end > (ulong)globalStart) { style = request.style_runs[r].style; styleIndex = (int)r; }
            float shift = style.baseline_shift * request.scale;
            for (int p = 0; p < parts.Count; p++) parts[p] = parts[p] with { Offset = parts[p].Offset - new Vector2(0, shift) };
            float ascent = parts.Count == 0 ? defaultMetrics.ascent : parts.Max(p => p.Font.Ascent * p.Size + shift);
            float descent = parts.Count == 0 ? defaultMetrics.descent : parts.Max(p => p.Font.Descent * p.Size - shift);
            float leading = parts.Count == 0 ? 0 : parts.Max(p => Math.Max(0, p.Font.LineGap * p.Size));
            var cluster = New<ViemShapedClusterV1>();
            cluster.text_start = (ulong)globalStart;
            cluster.text_end = checked((ulong)(contextStart + byteEnd));
            cluster.advance = Math.Max(0, advance);
            cluster.metrics = new() { ascent = Math.Max(0, ascent), descent = Math.Max(0, descent), leading = leading };
            cluster.typographic_bounds = new() { y = -ascent, width = cluster.advance, height = Math.Max(0, ascent + descent) };
            cluster.ink_bounds = cluster.typographic_bounds;
            foreach (var part in parts)
            {
                var ink = part.Font.GetGlyphRunBounds(measuring, part.Offset, part.Size, part.Glyphs, false, part.BidiLevel);
                var previous = cluster.ink_bounds;
                float x = Math.Min(previous.x, (float)ink.X), y = Math.Min(previous.y, (float)ink.Y);
                cluster.ink_bounds = new() { x = x, y = y, width = Math.Max(previous.x + previous.width, (float)ink.Right) - x, height = Math.Max(previous.y + previous.height, (float)ink.Bottom) - y };
            }
            cluster.bidi_level = bidi;
            cluster.fallback_font = arena.Utf8(parts.FirstOrDefault()?.Font.FamilyNames.Values.FirstOrDefault() ?? ResolveFamily(style));
            ViemClusterCaretStopV1[] stops = [
                new() { text_offset = cluster.text_start, inline_offset = (bidi & 1) == 0 ? 0 : cluster.advance, affinity = VIEM_BOUNDARY_AFFINITY_DOWNSTREAM },
                new() { text_offset = cluster.text_end, inline_offset = (bidi & 1) == 0 ? cluster.advance : 0, affinity = VIEM_BOUNDARY_AFFINITY_UPSTREAM }
            ];
            cluster.caret_stops = arena.Copy<ViemClusterCaretStopV1>(stops);
            cluster.caret_stop_count = 2;
            if (request.purpose == VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA)
            {
                ulong id = ++nextResource;
                fragment.References++;
                if (!markerFonts.TryGetValue(styleIndex, out var markerFont)) markerFonts[styleIndex] = markerFont = MarkerFont.From(style, request.scale);
                resources.Add(id, new Resource(fragment, parts, left, line.Baseline + shift, cluster.ink_bounds, markerFont));
                responseResources.Add(id);
                cluster.has_render_run = 1;
                cluster.render_run = new() { owner = owner, identifier = id, metrics_generation = Generation, threading = VIEM_RENDER_THREADING_FRONTEND_MAIN };
            }
            clusters.Add(cluster);
            positions.Add(left);
            start = end;
        }
        if (fragment.References == 0) layout.Dispose();
        ShapedCharacters += interior.Length;
        ulong[] order = Enumerable.Range(0, clusters.Count).OrderBy(i => positions[i]).Select(i => (ulong)i).ToArray();
        return new()
        {
            struct_size = (uint)sizeof(ViemShapeResponseV1), document_id = request.document_id, document_revision = request.document_revision,
            measurement_environment_id = request.measurement_environment_id, metrics_generation = request.metrics_generation,
            text_start = request.text_start, text_end = request.text_end, default_metrics = defaultMetrics,
            clusters = arena.Copy<ViemShapedClusterV1>(clusters.ToArray()), cluster_count = (ulong)clusters.Count,
            visual_order = arena.Copy<ulong>(order), visual_order_count = (ulong)order.Length
        };
    }

    private static readonly HashSet<string> families = new(CanvasTextFormat.GetSystemFontFamilies(), StringComparer.OrdinalIgnoreCase);
    private static string ResolveFamily(ViemResolvedTextStyleV1 style)
    {
        for (ulong i = 0; i < style.font_family_count; i++)
        {
            string family = Text(style.font_families[i]);
            string candidate = family.ToLowerInvariant() switch {
                "monospace" or "menlo" or "monaco" => families.Contains("Cascadia Mono") ? "Cascadia Mono" : "Consolas",
                "serif" or "times" => "Georgia",
                "sans-serif" or "system-ui" or "system" or "helvetica" or "helvetica neue" => "Segoe UI",
                _ => family
            };
            if (families.Contains(candidate)) return candidate;
        }
        return "Segoe UI";
    }
    private static FontStyle Slant(uint slant) => slant switch { 1 => FontStyle.Italic, 2 => FontStyle.Oblique, _ => FontStyle.Normal };
    private static void ApplyStyle(CanvasTextLayout layout, int start, int count, ViemResolvedTextStyleV1 style, float scale)
    {
        layout.SetFontFamily(start, count, ResolveFamily(style));
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
            foreach (ulong id in ids) resources[id].References++;
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
        void Finish() { foreach (ulong id in lease.Ids) lease.Provider.ReleaseResource(id); }
        if (lease.Provider.dispatcher.HasThreadAccess) Finish();
        else lease.Provider.dispatcher.TryEnqueue(Finish);
    }
    private void ReleaseResource(ulong id)
    {
        if (!resources.TryGetValue(id, out var resource) || --resource.References != 0) return;
        resources.Remove(id);
        if (--resource.Fragment.References == 0) resource.Fragment.Layout.Dispose();
    }
    public void Dispose()
    {
        arena.Dispose();
        foreach (ulong id in responseResources) ReleaseResource(id);
        responseResources.Clear();
        measurement.Dispose();
        GC.KeepAlive(generationCallback); GC.KeepAlive(shapeCallback); GC.KeepAlive(retainCallback);
    }
    private sealed record Lease(DirectWriteProvider Provider, ulong[] Ids);
    private sealed class Fragment(CanvasTextLayout layout) { public CanvasTextLayout Layout = layout; public int References; }
    private sealed class Resource(Fragment fragment, List<GlyphPart> parts, float left, float baseline, ViemShapedBoundsV1 bounds, MarkerFont markerFont)
    {
        public MarkerFont MarkerFont = markerFont;
        public Fragment Fragment = fragment; public List<GlyphPart> Parts = parts; public int References = 1;
        public float Left = left, Baseline = baseline; public ViemShapedBoundsV1 Bounds = bounds;
        public bool ColorGlyph = parts.Any(p => p.Font.FamilyNames.Values.Any(n => n.Contains("Emoji", StringComparison.OrdinalIgnoreCase)));
    }
}

internal sealed record GlyphPart(CanvasFontFace Font, float Size, CanvasGlyph[] Glyphs, uint BidiLevel, Vector2 Offset);
internal sealed class GlyphCapture : ICanvasTextRenderer
{
    private sealed record Run(Vector2 Point, CanvasFontFace Font, float Size, CanvasGlyph[] Glyphs, uint Bidi, int[] Map, int Start, Dictionary<int, int> Ends, float[] Advances);
    private readonly List<Run> runs = [];
    public bool PixelSnappingDisabled => true;
    public Matrix3x2 Transform => Matrix3x2.Identity;
    public float Dpi => 96;
    public void DrawGlyphRun(Vector2 point, CanvasFontFace fontFace, float fontSize, CanvasGlyph[] glyphs, bool isSideways, uint bidiLevel, object brush, CanvasTextMeasuringMode measuringMode, string localeName, string textString, int[] clusterMapIndices, uint characterIndex, CanvasGlyphOrientation glyphOrientation)
    {
        var boundaries = clusterMapIndices.Distinct().Order().Append(glyphs.Length).ToArray(); var ends = new Dictionary<int, int>();
        for (int i = 0; i + 1 < boundaries.Length; i++) ends[boundaries[i]] = boundaries[i + 1];
        var advances = new float[glyphs.Length + 1]; for (int i = 0; i < glyphs.Length; i++) advances[i + 1] = advances[i] + glyphs[i].Advance;
        runs.Add(new(point, fontFace, fontSize, glyphs, bidiLevel, clusterMapIndices, checked((int)characterIndex), ends, advances));
    }
    public List<GlyphPart> Extract(int start, int end, float left, float baseline)
    {
        var result = new List<GlyphPart>();
        foreach (var run in runs)
        {
            int first = Math.Max(start, run.Start) - run.Start, last = Math.Min(end, run.Start + run.Map.Length) - run.Start;
            if (last <= first) continue;
            int glyphStart = run.Map[first..last].Min(), finalStart = run.Map[first..last].Max();
            int glyphEnd = run.Ends[finalStart];
            float offset = run.Advances[glyphStart] * ((run.Bidi & 1) == 0 ? 1 : -1);
            result.Add(new(run.Font, run.Size, run.Glyphs[glyphStart..glyphEnd], run.Bidi, run.Point + new Vector2(offset - left, -baseline)));
        }
        return result;
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

using System.Buffers.Binary;
using System.Collections.Concurrent;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using Microsoft.Graphics.Canvas.Text;
using Windows.UI.Text;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Rendering;

internal sealed record FontAxis(string Tag, string Name, float Minimum, float Default, float Maximum, bool Hidden);
internal sealed record FontInstance(string Name, Dictionary<string, float> Values) { public override string ToString() => Name; }
internal sealed record FontStyleLink(float From, float To);
internal sealed record FontVariationInfo(FontAxis[] Axes, FontInstance[] Instances, Dictionary<string, List<FontStyleLink>> Links)
{
    public static readonly FontVariationInfo Empty = new([], [], []);
    public Dictionary<string, float> Defaults => Axes.ToDictionary(a => a.Tag, a => a.Default);
}

// Tables are read only for the requested native face. No filesystem font scan,
// registry scan, or native font resource is retained in the catalogue.
internal static unsafe class FontVariations
{
    private static readonly ConcurrentDictionary<FontFace, FontVariationInfo> cache = new();
    internal static FontVariationInfo For(FontFace? face) => face == null ? FontVariationInfo.Empty : cache.GetOrAdd(face, Read);
    internal static Dictionary<string, float> Decode(string json)
    {
        if (string.IsNullOrEmpty(json)) return [];
        try { return JsonSerializer.Deserialize<Dictionary<string, float>>(json) ?? []; }
        catch (JsonException) { return []; }
    }
    internal static string Encode(Dictionary<string, float> values) => JsonSerializer.Serialize(values);
    internal static uint Tag(string tag) => (uint)tag[0] | ((uint)tag[1] << 8) | ((uint)tag[2] << 16) | ((uint)tag[3] << 24);
    private static FontVariationInfo Read(FontFace face)
    {
        try { var tables = FontCatalog.VariationTables(face); return Parse(tables.Fvar, tables.Name, tables.Stat); }
        catch (Exception e) when (e is COMException or ArgumentException or IndexOutOfRangeException or OverflowException)
        { System.Diagnostics.Debug.WriteLine($"Font variations {face.Name}: {e.Message}"); return FontVariationInfo.Empty; }
    }
    internal static string Name(byte[] names, ushort id, string fallback)
    {
        string? result = null;
        if (names.Length < 6) return fallback;
        static ushort U16(byte[] b, int i) => BinaryPrimitives.ReadUInt16BigEndian(b.AsSpan(i, 2));
        int count = U16(names, 2), strings = U16(names, 4);
        for (int i = 0; i < count; i++) {
            int p = 6 + i * 12;
            if (p + 12 > names.Length) break;
            int platform = U16(names, p), language = U16(names, p + 4), length = U16(names, p + 8), offset = strings + U16(names, p + 10);
            if (U16(names, p + 6) != id || offset + length > names.Length || platform is not (0 or 3)) continue;
            string value = Encoding.BigEndianUnicode.GetString(names, offset, length);
            result ??= value;
            if (language == 0x409) return value;
        }
        return result ?? fallback;
    }
    internal static FontVariationInfo Parse(byte[] fvar, byte[] names, byte[] stat)
    {
        if (fvar.Length < 16) return FontVariationInfo.Empty;
        static ushort U16(byte[] b, int i) => BinaryPrimitives.ReadUInt16BigEndian(b.AsSpan(i, 2));
        static float Fixed(byte[] b, int i) => BinaryPrimitives.ReadInt32BigEndian(b.AsSpan(i, 4)) / 65536f;
        int axisOffset = U16(fvar, 4), axisCount = U16(fvar, 8), axisSize = U16(fvar, 10), instanceCount = U16(fvar, 12), instanceSize = U16(fvar, 14);
        if (axisCount > 64 || axisSize < 20 || axisOffset + axisCount * axisSize > fvar.Length || (instanceCount > 0 && instanceSize < 4 + axisCount * 4) || instanceCount > 4096) return FontVariationInfo.Empty;
        var axes = new List<FontAxis>();
        for (int i = 0; i < axisCount; i++) {
            int p = axisOffset + i * axisSize;
            string tag = Encoding.ASCII.GetString(fvar, p, 4);
            float min = Fixed(fvar, p + 4), def = Fixed(fvar, p + 8), max = Fixed(fvar, p + 12);
            if (min > def || def > max || axes.Any(a => a.Tag == tag)) return FontVariationInfo.Empty;
            axes.Add(new(tag, Name(names, U16(fvar, p + 18), tag), min, def, max, (U16(fvar, p + 16) & 1) != 0));
        }
        var instances = new List<FontInstance> { new("Default", axes.ToDictionary(a => a.Tag, a => a.Default)) };
        for (int i = 0; i < instanceCount; i++) {
            int p = axisOffset + axisCount * axisSize + i * instanceSize;
            if (p + instanceSize > fvar.Length) break;
            var values = axes.Select((a, j) => (a.Tag, Value: Fixed(fvar, p + 4 + j * 4))).ToDictionary(a => a.Tag, a => a.Value);
            if (axes.Any(a => values[a.Tag] < a.Minimum || values[a.Tag] > a.Maximum)) continue;
            instances.Add(new(Name(names, U16(fvar, p), $"Instance {i + 1}"), values));
        }
        var links = new Dictionary<string, List<FontStyleLink>>();
        if (stat.Length >= 18) {
            int size = U16(stat, 4), count = U16(stat, 6);
            int offset = checked((int)BinaryPrimitives.ReadUInt32BigEndian(stat.AsSpan(8, 4)));
            int valueCount = U16(stat, 12), valueOffset = checked((int)BinaryPrimitives.ReadUInt32BigEndian(stat.AsSpan(14, 4)));
            if (size >= 8 && count <= 64 && offset + count * size <= stat.Length && valueCount <= 4096 && valueOffset + valueCount * 2 <= stat.Length)
                for (int i = 0; i < valueCount; i++) {
                    int p = valueOffset + U16(stat, valueOffset + i * 2);
                    if (p + 16 > stat.Length || U16(stat, p) != 3) continue;
                    int axis = U16(stat, p + 2);
                    if (axis < count) {
                        string tag = Encoding.ASCII.GetString(stat, offset + axis * size, 4);
                        if (!links.TryGetValue(tag, out var entries)) links[tag] = entries = [];
                        entries.Add(new(Fixed(stat, p + 8), Fixed(stat, p + 12)));
                    }
                }
        }
        return new(axes.ToArray(), instances.DistinctBy(i => i.Name).ToArray(), links);
    }
    internal static Dictionary<string, float> Effective(FontVariationInfo info, Dictionary<string, float> saved, float weight, bool bold, uint slant)
    {
        var result = info.Defaults;
        foreach (var axis in info.Axes) {
            float value = saved.GetValueOrDefault(axis.Tag, axis.Default);
            if (axis.Tag == "wght") {
                value = saved.GetValueOrDefault("wght", bold ? weight - 300 : weight);
                if (bold) {
                    var link = info.Links.GetValueOrDefault("wght")?.FirstOrDefault(l => Math.Abs(value - l.From) < .001f && l.To > value);
                    value = link?.To ?? value + 300;
                }
            }
            if (slant != 0 && axis.Tag == "ital") value = 1;
            if (slant != 0 && axis.Tag == "slnt" && !info.Axes.Any(a => a.Tag == "ital")) {
                float target = info.Links.GetValueOrDefault("slnt")?.FirstOrDefault(l => Math.Abs(value - l.From) < .001f && l.To < value)?.To ?? -12;
                value = Math.Min(value, target);
            }
            result[axis.Tag] = Math.Clamp(value, axis.Minimum, axis.Maximum);
        }
        return result;
    }
#if DEBUG
    internal static Dictionary<string, float> NativeAxes(nint face)
    {
        Guid id = new("98EFF3A5-B667-479A-B145-E2FA5B9FDC29");
        nint face5 = 0;
        try {
            Marshal.ThrowExceptionForHR(Marshal.QueryInterface(face, in id, out face5));
            uint count = ((delegate* unmanaged[Stdcall]<nint, uint>)(*(nint**)face5)[53])(face5);
            var axes = new AxisValue[checked((int)count)];
            fixed (AxisValue* p = axes) Marshal.ThrowExceptionForHR(((delegate* unmanaged[Stdcall]<nint, AxisValue*, uint, int>)(*(nint**)face5)[54])(face5, p, count));
            return axes.ToDictionary(a => new string([(char)(a.Tag & 255), (char)((a.Tag >> 8) & 255), (char)((a.Tag >> 16) & 255), (char)(a.Tag >> 24)]), a => a.Value);
        } finally { if (face5 != 0) Marshal.Release(face5); }
    }
    internal static Dictionary<string, float> NativeAxes(CanvasFontFace font)
    {
        nint reference = GlyphFontMetadata.NativeReference(font), face = 0;
        if (reference == 0) throw new COMException("The glyph font reference is unavailable.");
        try {
            Marshal.ThrowExceptionForHR(((delegate* unmanaged[Stdcall]<nint, nint*, int>)(*(nint**)reference)[3])(reference, &face));
            return NativeAxes(face);
        } finally { if (face != 0) Marshal.Release(face); Marshal.Release(reference); GC.KeepAlive(font); }
    }
#endif
    [StructLayout(LayoutKind.Sequential)] private struct AxisValue { public uint Tag; public float Value; }
    [StructLayout(LayoutKind.Sequential)] private struct TextRange { public uint Start, Length; }
    internal static void Apply(CanvasTextLayout layout, int start, int count, Dictionary<string, float> values, FontFace? font)
    {
        if (values.Count == 0) return;
        Guid id = new("05A9BF42-223F-4441-B5FB-8263685F55E9"); // IDWriteTextLayout4
        nint native = NativeResource(layout, id);
        if (native == 0) throw new COMException("DirectWrite variable font layout is unavailable.");
        try {
            UseTypographicFamily(native, start, count, font);
            Marshal.ThrowExceptionForHR(((delegate* unmanaged[Stdcall]<nint, uint, int>)(*(nint**)native)[88])(native, 1u)); // Automatic opsz applies only where no explicit coordinate is set.
            var axes = values.Select(v => new AxisValue { Tag = Tag(v.Key), Value = v.Value }).ToArray();
            fixed (AxisValue* p = axes) {
                // IUnknown + TextFormat(25) + Layout(39) + Layout1(4) + Layout2(9) + Layout3(4).
                var set = (delegate* unmanaged[Stdcall]<nint, AxisValue*, uint, TextRange, int>)(*(nint**)native)[84];
                Marshal.ThrowExceptionForHR(set(native, p, (uint)axes.Length, new TextRange { Start = (uint)start, Length = (uint)count }));
            }
        } finally { Marshal.Release(native); GC.KeepAlive(layout); }
    }
    [DllImport("dwrite.dll")] private static extern int DWriteCreateFactory(uint kind, in Guid id, out nint factory);
    // Match against the complete design space, rather than the WWS projection
    // that fixes optical families such as Segoe UI Variable Text to one instance.
    // Indices are immutable/agile and retained with the catalogue for this process.
    private static readonly Lazy<CollectionHandle> systemCollection = new(() => CreateCollection(null));
    private static readonly ConcurrentDictionary<Uri, Lazy<CollectionHandle>> bundledCollections = new();
    private sealed class CollectionHandle : Microsoft.Win32.SafeHandles.SafeHandleZeroOrMinusOneIsInvalid
    {
        internal CollectionHandle(nint value) : base(true) { SetHandle(value); }
        public nint Pointer => handle;
        protected override bool ReleaseHandle() { Marshal.Release(handle); return true; }
    }
    private static CollectionHandle CreateCollection(FontFace? font)
    {
        Guid id = new("F3744D80-21F7-42EB-B35D-995BC72FC223"); nint factory = 0, collection = 0, set = 0;
        try {
            Marshal.ThrowExceptionForHR(DWriteCreateFactory(0, in id, out factory));
            if (font?.Source == null)
                Marshal.ThrowExceptionForHR(((delegate* unmanaged[Stdcall]<nint, int, uint, nint*, int>)(*(nint**)factory)[51])(factory, 0, 0, &collection));
            else {
                set = NativeResource(FontCatalog.VariationFontSet(font), new("53585141-D9F8-4095-8321-D73CF6BD116B"));
                if (set == 0) throw new COMException("The packaged variable font collection is unavailable.");
                Marshal.ThrowExceptionForHR(((delegate* unmanaged[Stdcall]<nint, nint, uint, nint*, int>)(*(nint**)factory)[52])(factory, set, 0, &collection));
            }
            var result = new CollectionHandle(collection); collection = 0;
            return result;
        } finally { if (set != 0) Marshal.Release(set); if (collection != 0) Marshal.Release(collection); if (factory != 0) Marshal.Release(factory); }
    }
    private static void UseTypographicFamily(nint layout, int start, int count, FontFace? font)
    {
        if (font == null) return;
        var collection = font.Source == null ? systemCollection.Value
            : bundledCollections.GetOrAdd(font.Source, _ => new(() => CreateCollection(font))).Value;
        var range = new TextRange { Start = (uint)start, Length = (uint)count };
        Marshal.ThrowExceptionForHR(((delegate* unmanaged[Stdcall]<nint, nint, TextRange, int>)(*(nint**)layout)[30])(layout, collection.Pointer, range));
        string family = string.IsNullOrEmpty(font.TypographicFamily) ? font.Family : font.TypographicFamily;
        fixed (char* p = family) Marshal.ThrowExceptionForHR(((delegate* unmanaged[Stdcall]<nint, char*, TextRange, int>)(*(nint**)layout)[31])(layout, p, range));
        GC.KeepAlive(collection);
    }
    internal static nint NativeResource(object wrapperObject, Guid id)
    {
        Guid wrapperId = new("5F10688D-EA55-4D55-A3B0-4DDB55C0C20A"); nint wrapper = 0, resource = 0;
        try {
            if (Marshal.QueryInterface(((WinRT.IWinRTObject)wrapperObject).NativeObject.ThisPtr, in wrapperId, out wrapper) < 0) return 0;
            var get = (delegate* unmanaged[Stdcall]<nint, nint, float, Guid*, nint*, int>)(*(nint**)wrapper)[3];
            if (get(wrapper, 0, 0, &id, &resource) >= 0) return resource;
            if (resource != 0) Marshal.Release(resource); return 0;
        } finally { if (wrapper != 0) Marshal.Release(wrapper); GC.KeepAlive(wrapperObject); }
    }
    internal static byte[] Table(CanvasFontFace font, string tag)
    {
        nint reference = GlyphFontMetadata.NativeReference(font), face = 0;
        if (reference == 0) return [];
        try {
            Marshal.ThrowExceptionForHR(((delegate* unmanaged[Stdcall]<nint, nint*, int>)(*(nint**)reference)[3])(reference, &face));
            nint data = 0, context = 0; uint size = 0; int exists = 0;
            Marshal.ThrowExceptionForHR(((delegate* unmanaged[Stdcall]<nint, uint, nint*, uint*, nint*, int*, int>)(*(nint**)face)[12])(face, Tag(tag), &data, &size, &context, &exists));
            try { return exists != 0 && size <= 4 * 1024 * 1024 ? new ReadOnlySpan<byte>((void*)data, (int)size).ToArray() : []; }
            finally { if (context != 0) ((delegate* unmanaged[Stdcall]<nint, nint, void>)(*(nint**)face)[13])(face, context); }
        } finally { if (face != 0) Marshal.Release(face); Marshal.Release(reference); GC.KeepAlive(font); }
    }
}

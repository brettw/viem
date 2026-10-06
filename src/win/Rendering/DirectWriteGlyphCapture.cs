using System.Numerics;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.Marshalling;
using Microsoft.Graphics.Canvas.Text;

namespace Viem.Windows.Rendering;

[GeneratedComInterface, Guid("EAF3A2DA-ECF4-4D24-B644-B34F6842024B")]
internal unsafe partial interface IDWritePixelSnapping
{
    [PreserveSig] int IsPixelSnappingDisabled(nint context, int* disabled);
    [PreserveSig] int GetCurrentTransform(nint context, Matrix3x2* transform);
    [PreserveSig] int GetPixelsPerDip(nint context, float* dpi);
}
[GeneratedComInterface, Guid("EF8A8135-5CC6-45FE-8825-C5A0724EB819")]
internal unsafe partial interface IDWriteTextRenderer : IDWritePixelSnapping
{
    [PreserveSig] int DrawGlyphRun(nint context, float x, float y, uint measuringMode, NativeGlyphRun* run, NativeGlyphDescription* description, nint effect);
    [PreserveSig] int DrawUnderline(nint context, float x, float y, nint underline, nint effect);
    [PreserveSig] int DrawStrikethrough(nint context, float x, float y, nint strike, nint effect);
    [PreserveSig] int DrawInlineObject(nint context, float x, float y, nint inlineObject, int sideways, int rtl, nint effect);
}
[StructLayout(LayoutKind.Sequential)] internal unsafe struct NativeGlyphRun
{
    public nint Face;
    public float Size;
    public uint Count;
    public ushort* Indices;
    public float* Advances;
    public Vector2* Offsets;
    public int Sideways;
    public uint Bidi;
}
[StructLayout(LayoutKind.Sequential)] internal unsafe struct NativeGlyphDescription
{
    public char* Locale;
    public char* Text;
    public uint Length;
    public ushort* Clusters;
    public uint Position;
}
// Win2D's text renderer rebuilds faces through a legacy named-font collection.
// Arbitrary variable instances must retain their original native face reference.
[GeneratedComClass]
internal unsafe partial class DirectWriteGlyphCapture(GlyphCapture capture) : IDWriteTextRenderer
{
    private static readonly StrategyBasedComWrappers wrappers = new();
    private static readonly Lazy<FactoryHandle> factory = new(CreateFactory);
    private Exception? failure;
    public int IsPixelSnappingDisabled(nint context, int* disabled) { *disabled = 1; return 0; }
    public int GetCurrentTransform(nint context, Matrix3x2* transform) { *transform = Matrix3x2.Identity; return 0; }
    public int GetPixelsPerDip(nint context, float* dpi) { *dpi = 1; return 0; }
    public int DrawGlyphRun(nint context, float x, float y, uint mode, NativeGlyphRun* run, NativeGlyphDescription* description, nint effect)
    {
        try {
            if (run == null) throw new InvalidOperationException("DirectWrite omitted the glyph run.");
            if (description == null) throw new InvalidOperationException("DirectWrite omitted the glyph cluster map.");
            if (run->Count != 0 && (run->Indices == null || run->Advances == null || run->Face == 0))
                throw new InvalidOperationException("DirectWrite omitted visible glyph data.");
            if (description->Length != 0 && (description->Clusters == null || description->Text == null))
                throw new InvalidOperationException("DirectWrite omitted the glyph run's text data.");
            var glyphs = new CanvasGlyph[run->Count];
            for (int i = 0; i < glyphs.Length; i++) glyphs[i] = new() {
                Index = run->Indices[i], Advance = run->Advances[i],
                AdvanceOffset = run->Offsets == null ? 0 : run->Offsets[i].X,
                AscenderOffset = run->Offsets == null ? 0 : run->Offsets[i].Y
            };
            var clusters = new int[description->Length];
            for (int i = 0; i < clusters.Length; i++) clusters[i] = description->Clusters[i];
            // Formatting controls retain text and bidi information but have no
            // glyphs. Their capture does not need a native font wrapper.
            capture.DrawGlyphRun(new(x, y), glyphs.Length == 0 ? null! : FontFace(run->Face), run->Size, glyphs, run->Sideways != 0, run->Bidi,
                null!, (CanvasTextMeasuringMode)mode, description->Locale == null ? "" : new string(description->Locale),
                new string(description->Text, 0, checked((int)description->Length)), clusters, description->Position, CanvasGlyphOrientation.Upright);
            return 0;
        } catch (Exception error) {
            failure ??= error; return Marshal.GetHRForException(error);
        }
    }
    public int DrawUnderline(nint context, float x, float y, nint underline, nint effect) => 0;
    public int DrawStrikethrough(nint context, float x, float y, nint strike, nint effect) => 0;
    public int DrawInlineObject(nint context, float x, float y, nint inlineObject, int sideways, int rtl, nint effect)
    { failure = new NotSupportedException("Unexpected inline object in a text-only shaping request."); return unchecked((int)0x80004001); }
    internal static void Draw(CanvasTextLayout layout, GlyphCapture capture)
    {
        var renderer = new DirectWriteGlyphCapture(capture);
        nint unknown = 0, callback = 0, native = 0;
        try {
            unknown = wrappers.GetOrCreateComInterfaceForObject(renderer, CreateComInterfaceFlags.None);
            native = FontVariations.NativeResource(layout, new("53737037-6D14-410B-9BFE-0B182BB70961"));
            if (native == 0) throw new COMException("DirectWrite glyph capture is unavailable.");
            Guid id = typeof(IDWriteTextRenderer).GUID;
            Marshal.ThrowExceptionForHR(Marshal.QueryInterface(unknown, in id, out callback));
            int status = ((delegate* unmanaged[Stdcall]<nint, nint, nint, float, float, int>)(*(nint**)native)[58])(native, 0, callback, 0, 0);
            if (renderer.failure != null) throw new InvalidOperationException("Capture native glyphs failed.", renderer.failure);
            Marshal.ThrowExceptionForHR(status);
        } finally { if (native != 0) Marshal.Release(native); if (callback != 0) Marshal.Release(callback); if (unknown != 0) Marshal.Release(unknown); GC.KeepAlive(renderer); GC.KeepAlive(layout); }
    }
    [DllImport("combase.dll")] private static extern int WindowsCreateString([MarshalAs(UnmanagedType.LPWStr)] string text, uint length, out nint value);
    [DllImport("combase.dll")] private static extern int WindowsDeleteString(nint value);
    [DllImport("Microsoft.Graphics.Canvas.dll")] private static extern int DllGetActivationFactory(nint name, out nint factory);
    private sealed class FactoryHandle : Microsoft.Win32.SafeHandles.SafeHandleZeroOrMinusOneIsInvalid
    {
        internal FactoryHandle(nint value) : base(true) { SetHandle(value); }
        internal nint Pointer => handle;
        protected override bool ReleaseHandle() { Marshal.Release(handle); return true; }
    }
    private static FactoryHandle CreateFactory()
    {
        nint activation = 0, native = 0, name = 0;
        try {
            const string runtimeClass = "Microsoft.Graphics.Canvas.CanvasDevice";
            Marshal.ThrowExceptionForHR(WindowsCreateString(runtimeClass, (uint)runtimeClass.Length, out name));
            Marshal.ThrowExceptionForHR(DllGetActivationFactory(name, out activation));
            Guid id = new("695C440D-04B3-4EDD-BFD9-63E51E9F7202"); // ICanvasFactoryNative
            Marshal.ThrowExceptionForHR(Marshal.QueryInterface(activation, in id, out native));
            var result = new FactoryHandle(native);
            native = 0; return result;
        } finally {
            if (native != 0) Marshal.Release(native);
            if (activation != 0) Marshal.Release(activation);
            if (name != 0) WindowsDeleteString(name);
        }
    }
    private static CanvasFontFace FontFace(nint face)
    {
        Guid id = new("D37D7598-09BE-4222-A236-2081341CC1F2"); // IDWriteFontFace3
        nint face3 = 0, reference = 0, instance = 0;
        try {
            Marshal.ThrowExceptionForHR(Marshal.QueryInterface(face, in id, out face3));
            Marshal.ThrowExceptionForHR(((delegate* unmanaged[Stdcall]<nint, nint*, int>)(*(nint**)face3)[35])(face3, &reference));
            var retainedFactory = factory.Value;
            nint nativeFactory = retainedFactory.Pointer;
            Marshal.ThrowExceptionForHR(((delegate* unmanaged[Stdcall]<nint, nint, nint, float, nint*, int>)(*(nint**)nativeFactory)[6])(nativeFactory, 0, reference, 0, &instance));
            var result = WinRT.MarshalInspectable<CanvasFontFace>.FromAbi(instance);
            GC.KeepAlive(retainedFactory);
            return result;
        } finally {
            if (instance != 0) Marshal.Release(instance);
            if (reference != 0) Marshal.Release(reference);
            if (face3 != 0) Marshal.Release(face3);
        }
    }
}

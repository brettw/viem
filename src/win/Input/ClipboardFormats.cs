using System.Net;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Viem.Windows.Rendering;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage.Streams;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Input;

internal static class ClipboardFormats
{
    public const string PrivateFormat = "com.viem.clipboard.fragment.v1";
    private const int MaximumFragmentBytes = 64 * 1024 * 1024;
    private static readonly Encoding StrictUtf8 = new UTF8Encoding(false, true);
    private static readonly Encoding StrictUtf16 = new System.Text.UnicodeEncoding(false, false, true);
#if DEBUG
    private sealed class TestPublication(DataPackage data, IRandomAccessStream? stream) : IDisposable
    {
        public DataPackage Data { get; } = data;
        public int Readers;
        private bool retired;
        public void Release() { if (--Readers == 0 && retired) Dispose(); }
        public void Retire() { retired = true; if (Readers == 0) Dispose(); }
        public void Dispose() => stream?.Dispose();
    }
    private static bool Isolated => Diagnostics.FrontendSmokeTests.ReportPath != null;
    private static TestPublication? isolatedClipboard;
    private static event EventHandler<object>? isolatedChanged;
    internal static string PrivateReadDiagnosticForTesting { get; private set; } = "";
    internal static void SetPackageForTesting(DataPackage data, IRandomAccessStream? ownedStream = null)
    {
        if (!Isolated) throw new InvalidOperationException("The test clipboard requires a self-test session.");
        PublishIsolated(new(data, ownedStream));
    }
    private static void PublishIsolated(TestPublication next)
    {
        var old = isolatedClipboard; isolatedClipboard = next; old?.Retire();
        isolatedChanged?.Invoke(null, EventArgs.Empty);
    }
#endif
    public static event EventHandler<object> ContentChanged
    {
        add {
#if DEBUG
            if (Isolated) { isolatedChanged += value; return; }
#endif
            Clipboard.ContentChanged += value;
        }
        remove {
#if DEBUG
            if (Isolated) { isolatedChanged -= value; return; }
#endif
            Clipboard.ContentChanged -= value;
        }
    }
    public static async Task<(string Text, string Fragment)> Read()
    {
        Exception? failure = null;
        for (int attempt = 0; attempt < 5; attempt++)
        {
            try { return await ReadOnce(); }
            catch (Exception error) when (Transient(error) && attempt < 4)
            {
                failure = error;
                await Task.Delay(10 * (attempt + 1));
            }
        }
        throw failure!;
    }
    private static async Task<(string Text, string Fragment)> ReadOnce()
    {
#if DEBUG
        if (Isolated) {
            if (isolatedClipboard is not { } captured) return ("", "");
            captured.Readers++;
            try { return await ReadPackage(captured.Data.GetView()); }
            finally { captured.Release(); }
        }
#endif
        return await ReadPackage(Clipboard.GetContent());
    }
    private static async Task<(string Text, string Fragment)> ReadPackage(DataPackageView data)
    {
        string text = data.Contains(StandardDataFormats.Text) ? await data.GetTextAsync() : "";
        // Rich representations are optional enhancements. A malformed or
        // unavailable private/HTML representation must not make ordinary
        // system clipboard text unusable.
        if (data.Contains(PrivateFormat))
            try
            {
                string fragment = await PrivateFragment(data);
                if (fragment.Length > 0) {
                    if (StrictUtf8.GetByteCount(fragment) > MaximumFragmentBytes) throw new InvalidDataException("Clipboard fragment is too large.");
                    using var json = JsonDocument.Parse(fragment);
                    if (json.RootElement.ValueKind != JsonValueKind.Object) throw new InvalidDataException("Clipboard fragment must be an object.");
                    return (text, fragment);
                }
            }
            catch (Exception error) when (!Transient(error)) {
#if DEBUG
                string diagnostic = PrivateReadDiagnosticForTesting + " " + error.GetType().Name + ": " + error.Message;
                PrivateReadDiagnosticForTesting = diagnostic.Length <= 1024 ? diagnostic : diagnostic[..1024];
#endif
            }
        if (data.Contains(StandardDataFormats.Html))
            try
            {
                string html = HtmlFormatHelper.GetStaticFragment(await data.GetHtmlFormatAsync());
                return Import(html, VIEM_CLIPBOARD_FORMAT_HTML, text);
            }
            catch (Exception error) when (!Transient(error)) { }
        return (text, "");
    }
    private static async Task<string> PrivateFragment(DataPackageView data)
    {
        object value = await data.GetDataAsync(PrivateFormat);
#if DEBUG
        PrivateReadDiagnosticForTesting = value.GetType().FullName ?? "unknown native value";
#endif
        if (value is string text) return text;
        if (value is IRandomAccessStreamReference reference)
        {
            using var opened = await reference.OpenReadAsync();
            return await StreamText(opened);
        }
        if (value is IRandomAccessStream stream) return await StreamText(stream);
        if (value is Stream managed)
        {
            if (!managed.CanSeek) return "";
            long length = managed.Length;
            if (length > MaximumFragmentBytes * 2L + 2) return "";
            managed.Position = 0;
            var bytes = new byte[checked((int)length)];
            await managed.ReadExactlyAsync(bytes);
            return DecodePrivateFragment(bytes);
        }
        return "";
    }
    private static async Task<string> StreamText(IRandomAccessStream stream)
    {
        ulong size = stream.Size;
        if (size > (ulong)MaximumFragmentBytes * 2 + 2) return "";
        // This independent input view belongs to this read; the publication's
        // random-access stream remains borrowed and available to other readers.
        using var input = stream.GetInputStreamAt(0);
        using var reader = new DataReader(input);
        uint length = checked((uint)size);
        uint loaded = await reader.LoadAsync(length);
        var bytes = new byte[loaded]; reader.ReadBytes(bytes);
        if (loaded != length) throw new InvalidDataException("Clipboard fragment was truncated.");
        return DecodePrivateFragment(bytes);
    }
    internal static string DecodePrivateFragment(ReadOnlySpan<byte> bytes)
    {
        if (bytes.Length > MaximumFragmentBytes * 2L + 2) throw new InvalidDataException("Clipboard fragment is too large.");
        // WinRT renders SetData(string) as UTF-16LE with a terminal code unit.
        // Only its BOM or the unambiguous compact JSON object prefix selects
        // this legacy form; other streams are strict UTF-8, never guessed text.
        bool bom = bytes.Length >= 2 && bytes[0] == 0xff && bytes[1] == 0xfe;
        if (bom || bytes.Length >= 2 && bytes[0] == (byte)'{' && bytes[1] == 0) {
            if (bom) bytes = bytes[2..];
            if (bytes.Length >= 2 && bytes[^1] == 0 && bytes[^2] == 0) bytes = bytes[..^2];
            return StrictUtf16.GetString(bytes);
        }
        return StrictUtf8.GetString(bytes);
    }
    internal static unsafe (string Text, string Fragment) Import(string source, uint format, string plain = "")
    {
        byte[] bytes = Encoding.UTF8.GetBytes(source);
        string fragment = Encoding.UTF8.GetString(Copy((p, n, r) => {
            fixed (byte* input = bytes) return viem_import_clipboard_json(format, input, (ulong)bytes.Length, p, n, r);
        }));
        using var parsed = JsonDocument.Parse(fragment);
        return (plain.Length == 0 ? PlainText(parsed.RootElement) : plain, fragment);
    }
    internal static string PlainText(JsonElement fragment) => fragment.TryGetProperty("image_plain_text", out var imageText)
        && imageText.ValueKind == JsonValueKind.String ? imageText.GetString() ?? "" : fragment.GetProperty("plain_text").GetString() ?? "";

    public static void Write(string text, string fragment)
    {
        var data = new DataPackage(); data.SetText(text);
        if (fragment.Length > 0)
        {
            data.SetData(PrivateFormat, fragment);
            data.SetHtmlFormat(HtmlFormatHelper.CreateHtmlFormat(Html(fragment)));
        }
#if DEBUG
        if (Isolated) { PublishIsolated(new(data, null)); return; }
#endif
        for (int attempt = 0; ; attempt++)
            try { Clipboard.SetContent(data); Clipboard.Flush(); return; }
            catch (Exception error) when (Transient(error) && attempt < 4)
            { Thread.Sleep(10 * (attempt + 1)); }
    }
    private static bool Transient(Exception error) => error is COMException && error.HResult is
        unchecked((int)0x800401D0) // CLIPBRD_E_CANT_OPEN
        or unchecked((int)0x80010001) // RPC_E_CALL_REJECTED
        or unchecked((int)0x80070005); // E_ACCESSDENIED while another owner renders
    internal static string Html(string fragment)
    {
        using var document = JsonDocument.Parse(fragment); var root = document.RootElement;
        // Image objects retain U+FFFC and logical indices in our private data.
        // External representations use passive Markdown text, never <img>
        // tags that could make the receiving application fetch a resource.
        if (root.TryGetProperty("image_plain_text", out var imageText) && imageText.ValueKind == JsonValueKind.String)
            return "<div style=\"white-space:pre-wrap\">" + WebUtility.HtmlEncode(imageText.GetString() ?? "") + "</div>";
        string text = root.GetProperty("plain_text").GetString() ?? ""; byte[] utf8 = Encoding.UTF8.GetBytes(text);
        var html = new StringBuilder("<div style=\"white-space:pre-wrap\">"); int cursor = 0;
        string Content(int start, int end) => WebUtility.HtmlEncode(Encoding.UTF8.GetString(utf8, start, end - start));
        var characters = root.GetProperty("character_runs").EnumerateArray().ToArray();
        void Inline(int first, int last)
        {
            int position = first;
            foreach (var run in characters)
            {
                int start = Math.Max(first, run.GetProperty("start").GetInt32()), end = Math.Min(last, run.GetProperty("end").GetInt32());
                if (end <= start) continue;
                if (start > position) html.Append(Content(position, start));
                html.Append("<span style=\"").Append(WebUtility.HtmlEncode(CharacterCss(run))).Append("\">");
                html.Append(Content(start, end));
                html.Append("</span>"); position = end;
            }
            if (position < last) html.Append(Content(position, last));
        }
        foreach (var paragraph in root.GetProperty("paragraph_runs").EnumerateArray())
        {
            int start = paragraph.GetProperty("start").GetInt32(), end = paragraph.GetProperty("end").GetInt32();
            // A paragraph element represents the inter-paragraph LF. Interior
            // hard breaks remain literal LF under white-space:pre-wrap.
            if (start > cursor && !(start == cursor + 1 && utf8[cursor] == 10)) Inline(cursor, start);
            var css = new StringBuilder("margin:0;");
            string Number(string key) => paragraph.GetProperty(key).GetDouble().ToString(System.Globalization.CultureInfo.InvariantCulture);
            css.Append("margin-block-start:").Append(Number("margin_top")).Append("pt;margin-block-end:").Append(Number("margin_bottom")).Append("pt;text-indent:").Append(Number("first_line_indent")).Append("pt;");
            bool rtl = paragraph.GetProperty("resolved_direction").GetString() == "RightToLeft";
            css.Append("direction:").Append(rtl ? "rtl" : "ltr").Append(";text-align:").Append(paragraph.GetProperty("alignment").GetString() switch { "Center" => "center", "End" => "end", _ => "start" }).Append(';');
            css.Append("margin-inline-start:").Append(Number("leading_indent")).Append("pt;margin-inline-end:").Append(Number("trailing_indent")).Append("pt;");
            var spacing = paragraph.GetProperty("line_spacing");
            if (spacing.ValueKind == JsonValueKind.Object) foreach (var entry in spacing.EnumerateObject()) if (entry.Name is "Multiplier" or "Exact") css.Append("line-height:").Append(entry.Value.GetDouble().ToString(System.Globalization.CultureInfo.InvariantCulture)).Append(entry.Name == "Exact" ? "pt;" : ";");
            html.Append("<p style=\"").Append(WebUtility.HtmlEncode(css.ToString())).Append("\">"); Inline(start, end); if (start == end) html.Append("<br>"); html.Append("</p>"); cursor = end;
        }
        if (cursor < utf8.Length) Inline(cursor, utf8.Length);
        return html.Append("</div>").ToString();
    }
    private static string CharacterCss(JsonElement run)
        {
            var css = new StringBuilder();
            string family = string.Join(",", run.GetProperty("font_families").EnumerateArray().Select(v => CssString(v.GetString()!)));
            if (family.Length > 0) css.Append("font-family:").Append(family).Append(';');
            if (run.TryGetProperty("font_face", out var face)) css.Append("--viem-font-face:").Append(CssString(face.GetString() ?? "")).Append(';');
            css.Append("font-size:").Append(run.GetProperty("size").GetDouble().ToString(System.Globalization.CultureInfo.InvariantCulture)).Append("pt;");
            css.Append("font-weight:").Append(run.GetProperty("weight").GetDouble().ToString(System.Globalization.CultureInfo.InvariantCulture)).Append(';');
            if (run.GetProperty("slant").GetString() != "Upright") css.Append("font-style:italic;");
            var decorations = new List<string>(); if (run.GetProperty("underline").GetBoolean()) decorations.Add("underline"); if (run.GetProperty("strikethrough").GetBoolean()) decorations.Add("line-through");
            if (decorations.Count > 0) css.Append("text-decoration:").Append(string.Join(' ', decorations)).Append(';');
            if (!run.GetProperty("foreground_is_default").GetBoolean()) css.Append("color:").Append(CssColor(run.GetProperty("foreground"))).Append(';');
            if (run.GetProperty("background").ValueKind != JsonValueKind.Null) css.Append("background-color:").Append(CssColor(run.GetProperty("background"))).Append(';');
            css.Append("letter-spacing:").Append(run.GetProperty("letter_spacing").GetDouble().ToString(System.Globalization.CultureInfo.InvariantCulture)).Append("pt;");
            if (run.GetProperty("direction").GetString() != "Natural") css.Append("unicode-bidi:bidi-override;direction:").Append(run.GetProperty("direction").GetString() == "RightToLeft" ? "rtl;" : "ltr;");
            var features = run.GetProperty("open_type_features").EnumerateObject().Select(p => CssString(p.Name) + " " + p.Value.GetUInt32()).ToArray();
            if (features.Length > 0) css.Append("font-feature-settings:").Append(string.Join(",", features)).Append(';');
            if (run.TryGetProperty("font_axes", out var axes) && axes.EnumerateObject().Any()) {
                var saved = FontVariations.Decode(axes.GetRawText());
                string primary = run.GetProperty("font_families").EnumerateArray().FirstOrDefault().GetString() ?? "";
                string subfamily = face.ValueKind == JsonValueKind.String ? face.GetString() ?? "" : "";
                var info = FontVariations.For(FontCatalog.Match(primary, subfamily) ?? FontCatalog.Faces(primary).FirstOrDefault());
                var rendered = FontVariations.Effective(info, saved, (float)run.GetProperty("weight").GetDouble(), run.GetProperty("bold").GetBoolean(), run.GetProperty("slant").GetString() == "Upright" ? 0u : 1u);
                if (rendered.Count == 0) rendered = saved;
                css.Append("font-variation-settings:").Append(string.Join(",", rendered.Select(p => CssString(p.Key) + " " + p.Value.ToString(System.Globalization.CultureInfo.InvariantCulture)))).Append(';');
            }
            return css.ToString();
        }
    private static string CssString(string value) => "'" + value.Replace("\\", "\\\\").Replace("'", "\\'").Replace("\r", "\\d ").Replace("\n", "\\a ") + "'";
    private static string CssColor(JsonElement value) => FormattableString.Invariant($"rgba({value.GetProperty("red").GetDouble() * 255:0},{value.GetProperty("green").GetDouble() * 255:0},{value.GetProperty("blue").GetDouble() * 255:0},{value.GetProperty("alpha").GetDouble():0.###})");
}

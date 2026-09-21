using System.Net;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage.Streams;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Input;

internal static class ClipboardFormats
{
    public const string PrivateFormat = "com.viem.clipboard.fragment.v1";
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
        var data = Clipboard.GetContent();
        string text = data.Contains(StandardDataFormats.Text) ? await data.GetTextAsync() : "";
        // Rich representations are optional enhancements. A malformed or
        // unavailable private/HTML/RTF representation must not make ordinary
        // system clipboard text unusable.
        if (data.Contains(PrivateFormat))
            try
            {
                string fragment = await PrivateFragment(data);
                if (fragment.Length > 0)
                    return (text, fragment);
            }
            catch (Exception error) when (!Transient(error)) { }
        if (data.Contains(StandardDataFormats.Html))
            try
            {
                string html = HtmlFormatHelper.GetStaticFragment(await data.GetHtmlFormatAsync());
                return Import(html, VIEM_FORMAT_HTML, text);
            }
            catch (Exception error) when (!Transient(error)) { }
        if (data.Contains(StandardDataFormats.Rtf))
            try { return Import(await data.GetRtfAsync(), VIEM_FORMAT_RTF, text); }
            catch (Exception error) when (!Transient(error)) { }
        return (text, "");
    }
    private static async Task<string> PrivateFragment(DataPackageView data)
    {
        object value = await data.GetDataAsync(PrivateFormat);
        if (value is string text) return text;
        if (value is IRandomAccessStreamReference reference)
        {
            using var opened = await reference.OpenReadAsync();
            return await StreamText(opened);
        }
        if (value is IRandomAccessStream stream) return await StreamText(stream);
        if (value is Stream managed)
        {
            using var copy = new MemoryStream();
            await managed.CopyToAsync(copy);
            return Encoding.UTF8.GetString(copy.ToArray());
        }
        return "";
    }
    private static async Task<string> StreamText(IRandomAccessStream stream)
    {
        if (stream.Size > int.MaxValue) return "";
        stream.Seek(0);
        using var reader = new DataReader(stream.GetInputStreamAt(0));
        uint length = checked((uint)stream.Size);
        uint loaded = await reader.LoadAsync(length);
        var bytes = new byte[loaded]; reader.ReadBytes(bytes);
        return Encoding.UTF8.GetString(bytes);
    }
    private static unsafe (string, string) Import(string source, uint format, string plain)
    {
        using var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: format);
        var info = New<ViemFormattedSnapshotInfoV1>(); Check(viem_core_formatted_snapshot_info(document.Handle, &info), "Read clipboard document");
        var range = New<ViemFormattedUtf8RangeV1>(); range.identity = info.identity; range.utf8_end = info.utf8_length;
        string fragment = Encoding.UTF8.GetString(Copy((p, n, r) => { var request = range; return viem_core_copy_clipboard_json(document.Handle, &request, p, n, r); }));
        return (plain.Length == 0 ? document.FormattedText() : plain, fragment);
    }
    public static void Write(string text, string fragment)
    {
        var data = new DataPackage(); data.SetText(text);
        if (fragment.Length > 0)
        {
            data.SetData(PrivateFormat, fragment);
            data.SetHtmlFormat(HtmlFormatHelper.CreateHtmlFormat(Html(fragment)));
        }
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
                string? script = run.GetProperty("script_position").GetString() switch { "Superscript" => "sup", "Subscript" => "sub", _ => null };
                html.Append("<span style=\"").Append(WebUtility.HtmlEncode(CharacterCss(run))).Append("\">");
                if (script != null) html.Append('<').Append(script).Append('>');
                html.Append(Content(start, end));
                if (script != null) html.Append("</").Append(script).Append('>');
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
            css.Append("margin-block-start:").Append(Number("spacing_before")).Append("pt;margin-block-end:").Append(Number("spacing_after")).Append("pt;text-indent:").Append(Number("first_line_indent")).Append("pt;");
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
            css.Append("font-size:").Append(run.GetProperty("size").GetDouble().ToString(System.Globalization.CultureInfo.InvariantCulture)).Append("pt;");
            css.Append("font-weight:").Append(run.GetProperty("weight").GetDouble().ToString(System.Globalization.CultureInfo.InvariantCulture)).Append(';');
            if (run.GetProperty("slant").GetString() != "Upright") css.Append("font-style:italic;");
            var decorations = new List<string>(); if (run.GetProperty("underline").GetBoolean()) decorations.Add("underline"); if (run.GetProperty("strikethrough").GetBoolean()) decorations.Add("line-through");
            if (decorations.Count > 0) css.Append("text-decoration:").Append(string.Join(' ', decorations)).Append(';');
            if (!run.GetProperty("foreground_is_default").GetBoolean()) css.Append("color:").Append(CssColor(run.GetProperty("foreground"))).Append(';');
            if (run.GetProperty("background").ValueKind != JsonValueKind.Null) css.Append("background-color:").Append(CssColor(run.GetProperty("background"))).Append(';');
            css.Append("letter-spacing:").Append(run.GetProperty("letter_spacing").GetDouble().ToString(System.Globalization.CultureInfo.InvariantCulture)).Append("pt;");
            css.Append("vertical-align:baseline;");
            if (run.GetProperty("direction").GetString() != "Natural") css.Append("unicode-bidi:bidi-override;direction:").Append(run.GetProperty("direction").GetString() == "RightToLeft" ? "rtl;" : "ltr;");
            var features = run.GetProperty("open_type_features").EnumerateObject().Select(p => CssString(p.Name) + " " + p.Value.GetUInt32()).ToArray();
            if (features.Length > 0) css.Append("font-feature-settings:").Append(string.Join(",", features)).Append(';');
            return css.ToString();
        }
    private static string CssString(string value) => "'" + value.Replace("\\", "\\\\").Replace("'", "\\'").Replace("\r", "\\d ").Replace("\n", "\\a ") + "'";
    private static string CssColor(JsonElement value) => FormattableString.Invariant($"rgba({value.GetProperty("red").GetDouble() * 255:0},{value.GetProperty("green").GetDouble() * 255:0},{value.GetProperty("blue").GetDouble() * 255:0},{value.GetProperty("alpha").GetDouble():0.###})");
}

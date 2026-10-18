using System.Globalization;
using System.Text.Json.Nodes;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class Preferences
{
    internal bool StartupApplicationOptionsApplied { get; set; }
    internal bool ScriptTimeout { get; private set; } = true;
    internal int ScriptTimeoutLength { get; private set; } = 1000;
    internal bool ScriptErrorBells { get; private set; } = true;
    internal bool ScriptVisualBell { get; private set; }
    internal string ScriptBellOff { get; private set; } = "";
    internal int? ScriptScrollOff { get; private set; }
    internal bool? ScriptShowStatus { get; private set; }
    internal bool? ScriptShowMenu { get; private set; }
    internal int? ScriptContinuationIndent { get; private set; }
    private bool scriptBreakIndent = true;
    private int scriptBreakIndentShift = 4;
    internal void ApplyScriptOption(string name, string value)
    {
        int Number(int minimum, int maximum) => int.TryParse(value, NumberStyles.Integer, CultureInfo.InvariantCulture, out int n)
            && n >= minimum && n <= maximum ? n : throw new InvalidDataException($"Invalid value for '{name}': {value}");
        switch (name)
        {
            case "timeout": ScriptTimeout = value == "1"; break;
            case "timeoutlen": ScriptTimeoutLength = Number(0, int.MaxValue); break;
            case "errorbells": ScriptErrorBells = value == "1"; break;
            case "visualbell": ScriptVisualBell = value == "1"; break;
            case "belloff": ScriptBellOff = value; break;
            case "scrolloff": ScriptScrollOff = Number(0, 1_000_000); break;
            case "laststatus": ScriptShowStatus = Number(0, 3) != 0; break;
            case "guioptions": ScriptShowMenu = value.Contains('m'); break;
            case "breakindent": scriptBreakIndent = value == "1"; ScriptContinuationIndent = scriptBreakIndent ? scriptBreakIndentShift : 0; break;
            case "breakindentopt":
                int shift;
                if (value.Length == 0) shift = 0;
                else if (value.StartsWith("shift:", StringComparison.Ordinal) && int.TryParse(value[6..], out int parsed) && parsed is >= 0 and <= 1024) shift = parsed;
                else throw new InvalidDataException("Viem supports breakindentopt=shift:0…1024.");
                scriptBreakIndentShift = shift; ScriptContinuationIndent = scriptBreakIndent ? shift : 0; break;
            case "guifont": case "linespace": ApplyScriptCodeOption(name, value); return;
            case "background": SelectTheme(value == "light" ? "Paper" : "Midnight"); return;
            default: throw new InvalidDataException($"Unsupported application option '{name}'.");
        }
        Changed?.Invoke();
    }
    private void ApplyScriptCodeOption(string name, string value)
    {
        var candidate = (JsonObject)activeTheme.DeepClone();
        var paragraph = candidate["styles"]?["code"]?["block_styles"]?.AsArray()
            .OfType<JsonObject>().FirstOrDefault(block => block["id"]?.GetValue<string>() == "Paragraph")
            ?? throw new InvalidDataException("The Code base paragraph is unavailable.");
        var character = paragraph["character"]?.AsObject() ?? new JsonObject();
        if (paragraph["character"] == null) paragraph["character"] = character;
        if (name == "guifont")
        {
            var families = new JsonArray(); double? size = null; bool bold = false, italic = false;
            foreach (string entry in (value.Length == 0 ? "monospace" : value).Split(','))
            {
                string[] fields = entry.Split(':');
                if (fields[0].Length == 0) throw new InvalidDataException("Invalid guifont.");
                families.Add(fields[0].Replace('_', ' '));
                foreach (string field in fields.Skip(1))
                    if (field == "b") bold = true;
                    else if (field == "i") italic = true;
                    else if (field.StartsWith('h') && double.TryParse(field[1..], NumberStyles.Float, CultureInfo.InvariantCulture, out double n)
                        && double.IsFinite(n) && n is >= 1 and <= 1000) size = n;
                    else throw new InvalidDataException("Viem supports guifont families with :h<size>, :b and :i.");
            }
            character["font_families"] = families; character["font_face"] = ""; character["font_axes"] = new JsonObject();
            if (size is { } height) character["size"] = height;
            character["weight"] = bold ? 700 : 400; character["bold"] = bold; character["slant"] = italic ? "Italic" : "Upright";
        }
        else
        {
            if (!double.TryParse(value, NumberStyles.Float, CultureInfo.InvariantCulture, out double spacing) || !double.IsFinite(spacing) || spacing is < 0 or > 1000)
                throw new InvalidDataException("Invalid linespace.");
            var block = paragraph["block"]?.AsObject() ?? new JsonObject();
            if (paragraph["block"] == null) paragraph["block"] = block;
            double size = character["size"]?.GetValue<double>() ?? 14;
            block["line_spacing"] = spacing == 0 ? JsonValue.Create("Normal") : new JsonObject { ["AtLeast"] = size * 1.2 + spacing };
        }
        // Publish the validated aggregate in memory. Theme/configuration files
        // remain untouched, including when startup.viem supplies these values.
        PublishTheme(candidate, SelectedTheme, () => { }, SelectedThemePath);
    }
}

internal sealed partial class EditorWindow
{
    private void ApplyScriptApplicationOption(EditorPane pane, string text)
    {
        int separator = text.IndexOf('=');
        if (separator < 0) throw new InvalidDataException("Invalid application option request.");
        string name = text[..separator], value = text[(separator + 1)..];
        if (name is "lines" or "columns")
        {
            if (!int.TryParse(value, out int cells) || cells < (name == "lines" ? 2 : 12) || cells > 10000)
                throw new InvalidDataException("Invalid window size.");
            var size = AppWindow.Size;
            double scale = Content.XamlRoot?.RasterizationScale ?? 1;
            int width = name == "columns" ? (int)Math.Ceiling((cells * (pane.View?.DefaultColumnWidth ?? 8) + pane.GutterWidth) * scale) : size.Width;
            int height = name == "lines" ? (int)Math.Ceiling((cells * (pane.View?.DefaultLineHeight ?? 20) + 100) * scale) : size.Height;
            AppWindow.Resize(new Windows.Graphics.SizeInt32(Math.Clamp(width, 480, 32767), Math.Clamp(height, 280, 32767)));
            return;
        }
        preferences.ApplyScriptOption(name, value);
        if (name == "guioptions") GlobalSelectionOptions.Observe(pane.Document,
            new Dictionary<uint, string> { [VIEM_EX_OPTION_AUTOSELECT] = value.Contains('a') ? "1" : "0" });
    }
}

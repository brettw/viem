using System.Text.Json;
using System.Text.Json.Nodes;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Windows.UI;

namespace Viem.Windows.Shell;

internal sealed partial class Preferences
{
    public string DirectoryPath { get; }
    private JsonObject root = new() { ["version"] = 1 };
    private bool writable = true;
    public string? Error { get; private set; }
    public event Action? Changed;
    public event Action? RecentChanged;
    public byte[] StartupCommands { get; private set; } = [];
    public Preferences(string? directory = null)
    {
        using var startupTiming = Diagnostics.StartupPerformance.Measure("preferences.read");
        DirectoryPath = directory ?? Environment.GetEnvironmentVariable("VIEM_CONFIG_DIR") ?? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".viem");
        string path = Path.Combine(DirectoryPath, "config.json");
        try
        {
            if (File.Exists(path)) root = Read(path);
        }
        catch (Exception e) { Error = path + ": " + e.Message; writable = false; }
        InitializeThemes(File.Exists(path));
        string startup = Path.Combine(DirectoryPath, "startup.viem");
        try { if (File.Exists(startup)) { if (new FileInfo(startup).Length > 1_048_576) throw new InvalidDataException("File is larger than 1 MiB."); StartupCommands = File.ReadAllBytes(startup); _ = new System.Text.UTF8Encoding(false, true).GetString(StartupCommands); } }
        catch (Exception e) { Error = startup + ": " + e.Message; StartupCommands = []; }
    }
    private static JsonObject Read(string path)
    {
        var result = JsonNode.Parse(File.ReadAllText(path)) as JsonObject ?? throw new InvalidDataException("Configuration must be a JSON object.");
        if (result["version"]?.GetValue<int>() != 1) throw new InvalidDataException("Unsupported config.json version. The file will not be overwritten.");
        Validate(result);
        return result;
    }
    public T Get<T>(string section, string key, T fallback)
    { try { var node = section == "theme" ? activeTheme["theme"]?[key] : root[section]?[key]; return node == null ? fallback : node.GetValue<T>(); } catch { return fallback; } }
    public bool ShowMenu => Get("windows", "showMenu", true);
    private static string ToolbarFormatKey(uint format) => format switch {
        Native.VIEM_FORMAT_MARKDOWN => "markdown", Native.VIEM_FORMAT_MARKDOWN_SOURCE => "markdownSource",

        Native.VIEM_FORMAT_CODE => "code", _ => "plainText"
    };
    public bool ShowFormattingToolbar(uint format) => Get("formattingToolbar", ToolbarFormatKey(format), true);
    public void SetFormattingToolbar(uint format, bool visible) => Set("formattingToolbar", ToolbarFormatKey(format), visible);
    public WindowFrame? DocumentWindowFrame => WindowFrame.Read(root["windows"]?["documentFrame"]);
    public void SetDocumentWindowFrame(WindowFrame frame)
    {
        if (!frame.IsValid) throw new InvalidDataException("Document window frame must have finite coordinates and positive dimensions.");
        Update(candidate => Merge(candidate, new JsonObject { ["windows"] = new JsonObject { ["documentFrame"] = frame.Json } }), notify: false);
    }
    public bool ShowStatus => Get("appearance", "showStatusBar", true);
    public bool SmartQuotes => Get("editing", "smartQuotes", false);
    public bool Midnight => Theme.Background.R * .2126 + Theme.Background.G * .7152 + Theme.Background.B * .0722 < 128;
    public Theme Theme => ReadTheme(activeTheme);
    public string StatusFontFamily => Get("theme", "statusFontFamily", "System") is "System" or "system-ui" ? "Segoe UI" : Get("theme", "statusFontFamily", "Segoe UI");
    public double StatusFontSize => Get("theme", "statusFontSize", 11d);
    public uint TextWidth => Get("editing", "textWidth", 80u);
    public byte[] Indentation => Json(root["editing"]?["indentation"], new JsonObject());
    public byte[] Whitespace => Json(root["editing"]?["whitespacePresentation"], new JsonObject());
    public byte[] Associations => Json(root["code"]?["filenameAssociations"], new JsonArray());
    private static byte[] Json(JsonNode? value, JsonNode fallback) => JsonSerializer.SerializeToUtf8Bytes(value ?? fallback, FrontendJsonContext.Default.JsonNode);
    public JsonObject Editing => root["editing"] is JsonObject value ? (JsonObject)value.DeepClone() : new();
    public float Margin(string edge) { try { return Math.Clamp(root["view"]?["margins"]?[edge]?.GetValue<float>() ?? 10, 0, 1000); } catch { return 10; } }
    public string[] Recent => root["recentDocuments"] is JsonArray a ? a.Select(v => v?.GetValue<string>() ?? "").Where(v => v.Length > 0).Take(10).ToArray() : [];
    public void Set(string section, string key, JsonNode? value)
    {
        if (section == "theme") { EditTheme(new JsonObject { [key] = value?.DeepClone() }); return; }
        Update(candidate => { if (candidate[section] is not JsonObject) candidate[section] = new JsonObject(); Merge((JsonObject)candidate[section]!, new JsonObject { [key] = value?.DeepClone() }); });
    }
    public void SetSections(JsonObject values)
    {
        var settings = (JsonObject)values.DeepClone();
        if (settings["theme"] is JsonObject theme) { EditTheme(theme); settings.Remove("theme"); }
        if (settings.Count > 0) Update(candidate => Merge(candidate, settings));
    }
    private static void Merge(JsonObject target, JsonObject source)
    { foreach (var pair in source) { if (pair.Value is JsonObject next && target[pair.Key] is JsonObject existing) Merge(existing, next); else target[pair.Key] = pair.Value?.DeepClone(); } }
    public static JsonObject ThemeJson(Theme theme) => new()
    {
        ["background"] = ColorJson(theme.Background), ["foreground"] = ColorJson(theme.Foreground),
        ["statusBackground"] = ColorJson(theme.StatusBackground), ["statusForeground"] = ColorJson(theme.StatusForeground),
        ["caret"] = ColorJson(theme.Caret), ["selection"] = ColorJson(theme.Selection)
    };
    public static JsonObject ColorJson(Color color) => new() { ["red"] = color.R / 255d, ["green"] = color.G / 255d, ["blue"] = color.B / 255d, ["alpha"] = color.A / 255d };
    private static Theme ReadTheme(JsonObject value)
    {
        var fallback = Theme.Midnight;
        Color ColorValue(string name, Color initial)
        {
            if (value["theme"]?[name] is not JsonObject color) return initial;
            byte Component(string key, double defaultValue) => (byte)Math.Round((color[key]?.GetValue<double>() ?? defaultValue) * 255);
            return Color.FromArgb(Component("alpha", 1), Component("red", 0), Component("green", 0), Component("blue", 0));
        }
        return new(ColorValue("background", fallback.Background), ColorValue("foreground", fallback.Foreground), ColorValue("statusBackground", fallback.StatusBackground), ColorValue("statusForeground", fallback.StatusForeground), ColorValue("caret", fallback.Caret), ColorValue("selection", fallback.Selection));
    }
    private static void Validate(JsonObject value)
    {
        if (value["selectedTheme"] is JsonNode selectedTheme) _ = selectedTheme.GetValue<string>();
        if (value["selectedThemeFile"] is JsonNode selectedThemeFile) _ = selectedThemeFile.GetValue<string>();
        foreach (string section in new[] { "theme", "view", "editing", "appearance", "code", "windows", "formattingToolbar" })
            if (value.ContainsKey(section) && value[section] is not JsonObject) throw new InvalidDataException(section + " must be an object.");
        void Number(JsonNode? node, double min, double max, string name, bool integral = false)
        { if (node == null) return; double n = node.GetValue<double>(); if (!double.IsFinite(n) || n < min || n > max || (integral && n != Math.Truncate(n))) throw new InvalidDataException("Invalid " + name + "."); }
        foreach (string color in new[] { "foreground", "background", "statusForeground", "statusBackground", "caret", "selection" })
            if (value["theme"]?[color] is JsonNode node)
            { if (node is not JsonObject rgb) throw new InvalidDataException("Invalid theme color."); foreach (string c in new[] { "red", "green", "blue", "alpha" }) { if (rgb[c] == null) throw new InvalidDataException("Incomplete theme color."); Number(rgb[c], 0, 1, color); } }
        Number(value["theme"]?["statusFontSize"], 8, 32, "status font size");
        if (value["theme"]?["statusFontFamily"] is JsonNode family && (family.GetValue<string>().Length is 0 or >= 256)) throw new InvalidDataException("Invalid status font family.");
        if (value["view"]?["margins"] is JsonNode margins) { if (margins is not JsonObject) throw new InvalidDataException("Invalid margins."); foreach (string edge in new[] { "top", "left", "bottom", "right" }) Number(margins[edge], 0, 1000, edge + " margin"); }
        foreach (var (section, key) in new[] { ("editing", "smartQuotes"), ("appearance", "showStatusBar"), ("windows", "showMenu") }) if (value[section]?[key] is JsonNode boolean) _ = boolean.GetValue<bool>();
        if (value["formattingToolbar"] is JsonObject toolbar)
            foreach (string key in new[] { "plainText", "code", "markdown", "markdownSource" })
                if (toolbar[key] is JsonNode visible) _ = visible.GetValue<bool>();
        if (value["windows"] is JsonObject windows && windows.ContainsKey("documentFrame") && WindowFrame.Read(windows["documentFrame"]) == null)
            throw new InvalidDataException("Document window frame must have finite coordinates and positive dimensions.");
        Number(value["editing"]?["textWidth"], 1, uint.MaxValue, "text width", true);
        if (value["recentDocuments"] is JsonNode recent)
        { if (recent is not JsonArray array) throw new InvalidDataException("Invalid recent documents."); foreach (var path in array) if (path == null || path.GetValue<string>().Length > 16384) throw new InvalidDataException("Invalid recent path."); }
        // Portable validators own indentation, whitespace and language schemas.
        using var doc = new CoreDocument([]);
        doc.ConfigureDefaults(Json(value["editing"]?["indentation"], new JsonObject()), Json(value["editing"]?["whitespacePresentation"], new JsonObject()), 80,
            Json(value["code"]?["filenameAssociations"], new JsonArray()));
    }
    public void Remember(string path)
    {
        using var startup = Diagnostics.StartupPerformance.Measure("preferences.remember");
        path = Path.GetFullPath(path);
        Update(candidate => {
            var existing = (candidate["recentDocuments"] as JsonArray)?.Select(v => v!.GetValue<string>()) ?? [];
            candidate["recentDocuments"] = new JsonArray(new[] { path }.Concat(existing.Where(v => !FileIdentity.Same(v, path))).Take(10).Select(v => (JsonNode?)JsonValue.Create(v)).ToArray());
        }, recentOnly: true);
    }
    public void ClearRecent() => Update(candidate => candidate["recentDocuments"] = new JsonArray(), recentOnly: true);
    private void Update(Action<JsonObject> edit, bool notify = true, bool recentOnly = false)
    {
        if (!writable) throw new InvalidOperationException(Error);
        string path = Path.Combine(DirectoryPath, "config.json");
        var candidate = File.Exists(path) ? Read(path) : (JsonObject)root.DeepClone();
        edit(candidate);
        // Ignore retired values of any JSON type, including externally merged
        // profiles, and remove the key only as part of a successful write.
        (candidate["code"] as JsonObject)?.Remove("vimSyntaxDirectory");
        candidate = (JsonObject)JsonNode.Parse(candidate.ToJsonString())!;
        Validate(candidate);
        AtomicWrite(path, JsonSerializer.SerializeToUtf8Bytes(candidate, FrontendJsonContext.Indented.JsonObject));
        // Opening a file changes the recent menu, not theme, margins, or layout.
        // Still publish external settings edits merged from disk on this write.
        bool settingsChanged = !recentOnly || root.Select(p => p.Key).Union(candidate.Select(p => p.Key))
            .Where(key => key != "recentDocuments").Any(key => !JsonNode.DeepEquals(root[key], candidate[key]));
        root = candidate;
        if (notify && settingsChanged) Changed?.Invoke();
        else if (notify && recentOnly) RecentChanged?.Invoke();
    }
    public static void AtomicWrite(string path, byte[] contents)
    {
        path = Path.GetFullPath(path);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        string temporary = path + ".viem-" + Guid.NewGuid().ToString("N") + ".tmp";
        try
        {
            using (var output = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None)) { output.Write(contents); output.Flush(true); }
            if (File.Exists(path)) File.Replace(temporary, path, null);
            else File.Move(temporary, path);
        }
        finally { if (File.Exists(temporary)) File.Delete(temporary); }
    }
}

internal sealed record Theme(Color Background, Color Foreground, Color StatusBackground, Color StatusForeground, Color Caret, Color Selection)
{
    public static Color Rgb(byte r, byte g, byte b, byte a = 255) => Color.FromArgb(a, r, g, b);
    public static readonly Theme Midnight = new(Rgb(9, 22, 43), Rgb(230, 237, 250), Rgb(5, 14, 31), Rgb(173, 199, 232), Rgb(194, 219, 255), Rgb(99, 166, 255, 97));
    public static readonly Theme Paper = new(Rgb(255, 255, 255), Rgb(20, 23, 28), Rgb(31, 33, 38), Rgb(224, 230, 237), Rgb(15, 61, 125), Rgb(31, 99, 186, 72));
}

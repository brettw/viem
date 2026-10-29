using System.Text.Json;
using System.Text.Json.Nodes;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Windows.UI;

namespace Viem.Windows.Shell;

internal sealed partial class Preferences
{
    public string DirectoryPath { get; }
    private JsonObject root = Normalize(new JsonObject { ["version"] = 1 });
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
        try
        {
            if (!File.Exists(startup))
            {
                Directory.CreateDirectory(DirectoryPath);
                try { using var file = new FileStream(startup, FileMode.CreateNew, FileAccess.Write, FileShare.Read); }
                catch (IOException) when (File.Exists(startup)) { } // Another process created it first.
            }
            if (new FileInfo(startup).Length > 1_048_576) throw new InvalidDataException("File is larger than 1 MiB.");
            StartupCommands = File.ReadAllBytes(startup);
            _ = new System.Text.UTF8Encoding(false, true).GetString(StartupCommands);
        }
        catch (Exception e) { Error = startup + ": " + e.Message; StartupCommands = []; }
    }
    private static JsonObject Read(string path)
    {
        var result = JsonNode.Parse(File.ReadAllText(path)) as JsonObject ?? throw new InvalidDataException("Configuration must be a JSON object.");
        return Normalize(result);
    }
    public T Get<T>(string section, string key, T fallback)
    { try { var node = section == "theme" ? activeTheme["theme"]?[key] : root[section]?[key]; return node == null ? fallback : node.GetValue<T>(); } catch { return fallback; } }
    public bool ShowMenu => ScriptShowMenu ?? Get("windows", "showMenu", true);
    private static string ToolbarFormatKey(uint format) => format switch {
        Native.VIEM_FORMAT_MARKDOWN => "markdown",
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
    public bool ShowStatus => ScriptShowStatus ?? Get("appearance", "showStatusBar", true);
    public bool MarkdownAutodetect => Get("editing", "markdownAutodetect", true);
    public bool CaretHoverEffect => Get("editing", "caretHoverEffect", true);
    public bool SmartQuotes => Get("editing", "smartQuotes", true);
    public bool Autoselect => Get("editing", "autoselect", true);
    public bool Midnight => Theme.Background.R * .2126 + Theme.Background.G * .7152 + Theme.Background.B * .0722 < 128;
    public Theme Theme {
        get { var canvas = ReadTheme(activeTheme); var status = StatusStyles.Status;
            return canvas with { StatusForeground = status.Foreground, StatusBackground = status.Background }; }
    }
    public uint TextWidth => Get("editing", "textWidth", 80u);
    public byte[] Indentation => Json(root["editing"]?["indentation"], new JsonObject());
    public byte[] Whitespace { get {
        var value = root["editing"]?["whitespacePresentation"]?.DeepClone() ?? new JsonObject();
        if (ScriptContinuationIndent is { } indent) value["codeWrappedLineIndent"] = indent;
        return Json(value, new JsonObject());
    } }
    public byte[] Associations => Json(root["code"]?["filenameAssociations"], new JsonArray());
    private static byte[] Json(JsonNode? value, JsonNode fallback) => JsonSerializer.SerializeToUtf8Bytes(value ?? fallback, FrontendJsonContext.Default.JsonNode);
    public JsonObject Editing => root["editing"] is JsonObject value ? (JsonObject)value.DeepClone() : new();
    public float Margin(string edge) { try { return Math.Clamp(root["view"]?["margins"]?[edge]?.GetValue<float>() ?? 10, 0, 1000); } catch { return 10; } }
    public float ZoomScale => Get("view", "zoomScale", 1f);
    public void SetZoomScale(float scale)
    {
        if (!float.IsFinite(scale) || scale < .25f || scale > 5f)
            throw new InvalidDataException("Editor zoom must be between 25% and 500%.");
        // This is the next view's initial zoom. Existing views retain their
        // independent magnification, and repeated/saturated actions do not
        // rewrite the configuration file.
        Update(candidate => {
            if (candidate["view"] is not JsonObject) candidate["view"] = new JsonObject();
            candidate["view"]!["zoomScale"] = scale;
        }, notify: false, skipUnchanged: true);
    }
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
        return new(ColorValue("background", fallback.Background), ColorValue("foreground", fallback.Foreground), fallback.StatusBackground, fallback.StatusForeground, ColorValue("caret", fallback.Caret), ColorValue("selection", fallback.Selection));
    }
    internal static unsafe JsonObject Normalize(JsonObject value)
    {
        var input = JsonSerializer.SerializeToUtf8Bytes(value, FrontendJsonContext.Default.JsonObject);
        using var arena = new NativeArena();
        var pointer = arena.Copy<byte>(input);
        var bytes = Abi.Copy((output, capacity, required) =>
            Native.viem_application_config(pointer, (ulong)input.Length, output, capacity, required));
        var result = JsonNode.Parse(bytes)!.AsObject();
        if (result["error"] is JsonNode error) throw new InvalidDataException(error.GetValue<string>());
        return (JsonObject)result["configuration"]!.DeepClone();
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
    private void Update(Action<JsonObject> edit, bool notify = true, bool recentOnly = false, bool skipUnchanged = false)
    {
        if (!writable) throw new InvalidOperationException(Error);
        string path = Path.Combine(DirectoryPath, "config.json");
        var candidate = File.Exists(path) ? Read(path) : (JsonObject)root.DeepClone();
        var previous = skipUnchanged ? candidate.DeepClone() : null;
        edit(candidate);
        // Ignore retired values of any JSON type, including externally merged
        // profiles, and remove the key only as part of a successful write.
        (candidate["code"] as JsonObject)?.Remove("vimSyntaxDirectory");
        candidate = (JsonObject)JsonNode.Parse(candidate.ToJsonString())!;
        candidate = Normalize(candidate);
        if (!skipUnchanged || !JsonNode.DeepEquals(previous, candidate))
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

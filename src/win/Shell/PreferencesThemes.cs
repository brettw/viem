using System.Text.Json;
using System.Text.Json.Nodes;
using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class Preferences
{
    private JsonObject activeTheme = new();
    private readonly Dictionary<CoreDocument, string> themeDocuments = [];
    private readonly Dictionary<CoreDocument, string> appliedDocumentStyles = [];
    private bool applyingTheme;
    private string? appliedCodeStyles;
    private ulong appliedCodeRevision;
    public event Action? ThemesChanged;
    public string? SelectedTheme { get; private set; }
    public string ThemeDisplayName => SelectedTheme ?? "Default";
    public string ThemesDirectory => Path.Combine(DirectoryPath, "themes");
    public string? SelectedThemePath { get; private set; }
    private static byte[] ThemeBytes(JsonNode value) => JsonSerializer.SerializeToUtf8Bytes(value, FrontendJsonContext.Indented.JsonNode);
    private static JsonObject DefaultTheme() => JsonNode.Parse(CoreThemes.Defaults())!.AsObject();
    private string ThemePath(string name) { CoreThemes.ValidateName(name); return Path.Combine(ThemesDirectory, name + ".json"); }
    internal sealed record ThemeFile(string Name, string Path) { public override string ToString() => Name; }
    public ThemeFile[] ThemeFiles => Directory.Exists(ThemesDirectory)
        ? Directory.EnumerateFiles(ThemesDirectory).Where(path => (File.GetAttributes(path) & FileAttributes.Directory) == 0)
            .Select(path => new ThemeFile(System.IO.Path.GetFileName(path).EndsWith(".json", StringComparison.OrdinalIgnoreCase) ? System.IO.Path.GetFileName(path)[..^5] : System.IO.Path.GetFileName(path), path))
            .OrderBy(file => file.Name, StringComparer.OrdinalIgnoreCase).ThenBy(file => file.Name, StringComparer.Ordinal).ThenBy(file => file.Path, StringComparer.Ordinal).ToArray()
        : [];
    public string[] ThemeNames => ThemeFiles.Select(file => file.Name).ToArray();
    private ThemeFile FindTheme(string name) => ThemeFiles.FirstOrDefault(file => file.Name == name)
        ?? ThemeFiles.FirstOrDefault(file => string.Equals(file.Name, name, StringComparison.OrdinalIgnoreCase))
        ?? throw new FileNotFoundException("Theme was not found.");
    private JsonObject ReadThemeFile(string path)
    {
        if (new FileInfo(path).Length > 20 * 1024 * 1024) throw new InvalidDataException("Theme is larger than 20 MiB.");
        byte[] bytes = File.ReadAllBytes(path); CoreThemes.Validate(bytes);
        return JsonNode.Parse(bytes)!.AsObject();
    }
    private void ThemeWarning(string message) => Error = Error == null ? message : Error + Environment.NewLine + message;
    private void InitializeThemes(bool hadConfiguration)
    {
        activeTheme = DefaultTheme();
        try
        {
            bool fresh = !hadConfiguration && ThemeNames.Length == 0;
            if (fresh && writable)
            {
                foreach (string name in new[] { "Paper", "Midnight" })
                {
                    string bundled = Path.Combine(AppContext.BaseDirectory, "Resources", "themes", name + ".json");
                    byte[] bytes = File.Exists(bundled) ? File.ReadAllBytes(bundled) : CoreThemes.Defaults(name == "Paper" ? VIEM_THEME_PRESET_PAPER : VIEM_THEME_PRESET_MIDNIGHT);
                    CoreThemes.Validate(bytes); AtomicWrite(ThemePath(name), bytes);
                }
                Update(candidate => SetThemeSelection(candidate, "Midnight", ThemePath("Midnight")), notify: false);
            }
            if (root["selectedTheme"] is JsonValue selection)
            {
                string requested = selection.GetValue<string>();
                var file = root["selectedThemeFile"] is JsonValue hint
                    ? ThemeFiles.FirstOrDefault(file => System.IO.Path.GetFileName(file.Path) == hint.GetValue<string>() && file.Name == requested)
                        ?? throw new FileNotFoundException("The selected theme file was not found.")
                    : FindTheme(requested);
                activeTheme = ReadThemeFile(file.Path); SelectedTheme = file.Name; SelectedThemePath = file.Path;
            }
        }
        catch (Exception error) { activeTheme = DefaultTheme(); SelectedTheme = null; SelectedThemePath = null; ThemeWarning(error.Message + " Using Default."); }
    }
    public static string StyleFamily(uint format) => format switch {
        VIEM_FORMAT_MARKDOWN or VIEM_FORMAT_MARKDOWN_SOURCE => "markdown", VIEM_FORMAT_CODE => "code", _ => "text"
    };
    public byte[] ThemeStyleDefaults(uint format)
    {
        string family = StyleFamily(format);
        return ThemeBytes(activeTheme["styles"]?[family] ?? DefaultTheme()["styles"]![family]!);
    }
    public string[] AttachThemeDocument(CoreDocument document, bool initialize = true)
    {
        if (themeDocuments.ContainsKey(document)) return [];
        ApplyCodeStyles(activeTheme);
        string family = StyleFamily(document.State.format);
        byte[] styles = ThemeStyleDefaults(document.State.format);
        string[] diagnostics = family == "code" ? [] : initialize ? document.InitializeStyleDefaults(styles) : document.ReplaceStyleDefaults(styles);
        themeDocuments[document] = family;
        appliedDocumentStyles[document] = family + Convert.ToBase64String(styles);
        document.Changed += () => {
            if (applyingTheme || !themeDocuments.TryGetValue(document, out string? previous)) return;
            string current = StyleFamily(document.State.format);
            if (current != previous) { ApplyDocumentStyles(document, activeTheme); document.NotifyChanged(); }
        };
        document.Disposed += () => { themeDocuments.Remove(document); appliedDocumentStyles.Remove(document); };
        return diagnostics;
    }
    private bool ApplyCodeStyles(JsonObject theme)
    {
        byte[] bytes = ThemeBytes(theme["styles"]?["code"] ?? DefaultTheme()["styles"]!["code"]!);
        string key = Convert.ToBase64String(bytes);
        if (key == appliedCodeStyles && appliedCodeRevision == CoreThemes.CodeStyleRevision) return false;
        CoreThemes.ReplaceCodeStyles(bytes); appliedCodeStyles = key; appliedCodeRevision = CoreThemes.CodeStyleRevision; return true;
    }
    private bool ApplyDocumentStyles(CoreDocument document, JsonObject theme)
    {
        string family = StyleFamily(document.State.format);
        byte[] styles = ThemeBytes(theme["styles"]?[family] ?? DefaultTheme()["styles"]![family]!);
        string key = family + Convert.ToBase64String(styles);
        bool changed = !appliedDocumentStyles.TryGetValue(document, out string? prior) || prior != key;
        if (family != "code" && changed) document.ReplaceStyleDefaults(styles);
        themeDocuments[document] = family; appliedDocumentStyles[document] = key; return changed;
    }
    private HashSet<CoreDocument> ApplyTheme(JsonObject theme)
    {
        applyingTheme = true;
        try {
            bool codeChanged = ApplyCodeStyles(theme);
            var changed = new HashSet<CoreDocument>();
            foreach (var document in themeDocuments.Keys.ToArray())
                if (ApplyDocumentStyles(document, theme) || codeChanged && document.State.format == VIEM_FORMAT_CODE) changed.Add(document);
            return changed;
        }
        finally { applyingTheme = false; }
    }
    private void PublishTheme(JsonObject candidate, string? name, Action persist, string? path = null)
    {
        CoreThemes.Validate(ThemeBytes(candidate));
        var previous = activeTheme;
        HashSet<CoreDocument> changed;
        try { changed = ApplyTheme(candidate); persist(); }
        catch { ApplyTheme(previous); throw; }
        activeTheme = candidate; SelectedTheme = name; SelectedThemePath = name == null ? null : path ?? FindTheme(name).Path;
        foreach (var document in changed) document.NotifyChanged();
        ThemesChanged?.Invoke(); Changed?.Invoke();
    }
    private static void SetThemeSelection(JsonObject configuration, string? name, string? path)
    {
        configuration["selectedTheme"] = name;
        configuration["selectedThemeFile"] = path == null ? null : System.IO.Path.GetFileName(path);
    }
    public bool EnsureCurrentThemeExists()
    {
        if (SelectedThemePath is not { } path || File.Exists(path)) return false;
        string missing = SelectedTheme!;
        PublishTheme(DefaultTheme(), null, () => {
            try { Update(root => SetThemeSelection(root, null, null), notify: false); }
            catch (Exception error) { ThemeWarning(error.Message); }
        });
        ThemeWarning($"Theme '{missing}' was removed. Using Default.");
        return true;
    }
    public void SelectTheme(string? name, string? path = null)
    {
        var file = name == null ? null : path == null ? FindTheme(name) : ThemeFiles.FirstOrDefault(file => file.Path == path) ?? throw new FileNotFoundException("Theme was not found.");
        var candidate = file == null ? DefaultTheme() : ReadThemeFile(file.Path);
        PublishTheme(candidate, file?.Name, () => Update(root => SetThemeSelection(root, file?.Name, file?.Path), notify: false), file?.Path);
    }
    public void CreateTheme(string name)
    {
        CoreThemes.ValidateName(name);
        EnsureCurrentThemeExists();
        if (ThemeNames.Contains(name, StringComparer.OrdinalIgnoreCase)) throw new IOException("A theme with that name already exists.");
        string path = ThemePath(name);
        var candidate = (JsonObject)activeTheme.DeepClone();
        if (candidate["styles"] is not JsonObject) candidate["styles"] = new JsonObject();
        var defaults = DefaultTheme()["styles"]!.AsObject();
        foreach (var family in defaults) if (candidate["styles"]![family.Key] == null) candidate["styles"]![family.Key] = family.Value!.DeepClone();
        CoreThemes.Validate(ThemeBytes(candidate));
        Directory.CreateDirectory(ThemesDirectory);
        string temporary = path + ".viem-" + Guid.NewGuid().ToString("N") + ".tmp";
        bool created = false;
        try {
            using (var output = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None)) { output.Write(ThemeBytes(candidate)); output.Flush(true); }
            // The no-overwrite move publishes a complete file atomically.
            File.Move(temporary, path); created = true;
            PublishTheme(candidate, name, () => Update(root => SetThemeSelection(root, name, path), notify: false), path);
        }
        catch { if (created) File.Delete(path); throw; }
        finally { if (File.Exists(temporary)) File.Delete(temporary); }
    }
    public void DeleteTheme()
    {
        if (EnsureCurrentThemeExists()) return;
        if (SelectedTheme == null) throw new InvalidOperationException("Default is not a saved theme.");
        string path = SelectedThemePath!;
        byte[] original = File.ReadAllBytes(path);
        PublishTheme(DefaultTheme(), null, () => {
            File.Delete(path);
            try { Update(root => SetThemeSelection(root, null, null), notify: false); }
            catch { AtomicWrite(path, original); throw; }
        });
    }
    public void EditTheme(JsonObject appearance)
    {
        var candidate = EditableTheme();
        if (candidate["theme"] is not JsonObject) candidate["theme"] = new JsonObject();
        Merge(candidate["theme"]!.AsObject(), appearance);
        SaveTheme(candidate);
    }
    public void SaveThemeStyles(uint format, byte[] styles)
    {
        var candidate = EditableTheme(); string family = StyleFamily(format);
        if (candidate["styles"] is not JsonObject) candidate["styles"] = new JsonObject();
        var replacement = JsonNode.Parse(styles)!.AsObject();
        candidate["styles"]![family] = KeepStyleExtensions(candidate["styles"]![family], replacement);
        SaveTheme(candidate);
    }
    // Fresh arrays define membership; removed definitions are never copied back.
    // Matching definitions retain unknown extension fields while new null values
    // still remove known declarations.
    private static JsonNode KeepStyleExtensions(JsonNode? previous, JsonNode current, int depth = 0)
    {
        // Only the stylesheet, definitions and property containers hold
        // extension keys. Property values (feature maps and tagged enums) replace.
        if (depth >= 4) return current.DeepClone();
        if (current is JsonObject value)
        {
            var result = previous is JsonObject old ? (JsonObject)old.DeepClone() : new JsonObject();
            var priorObject = previous as JsonObject;
            foreach (var property in value) {
                JsonNode? prior = null;
                if (priorObject != null) priorObject.TryGetPropertyValue(property.Key, out prior);
                result[property.Key] = property.Value == null ? null : KeepStyleExtensions(prior, property.Value, depth + 1);
            }
            return result;
        }
        if (current is JsonArray array)
        {
            var result = new JsonArray();
            foreach (var entry in array)
            {
                JsonNode? old = entry is JsonObject obj && obj["id"] is JsonValue id && previous is JsonArray oldArray
                    ? oldArray.FirstOrDefault(item => JsonNode.DeepEquals((item as JsonObject)?["id"], id)) : null;
                result.Add(entry == null ? null : KeepStyleExtensions(old, entry, depth + 1));
            }
            return result;
        }
        return current.DeepClone();
    }
    private JsonObject EditableTheme()
    {
        EnsureCurrentThemeExists();
        return SelectedThemePath is { } path ? ReadThemeFile(path) : (JsonObject)activeTheme.DeepClone();
    }
    private void SaveTheme(JsonObject candidate)
    {
        string? name = SelectedTheme;
        PublishTheme(candidate, name, () => { if (name != null) AtomicWrite(SelectedThemePath!, ThemeBytes(candidate)); }, SelectedThemePath);
    }
}

#if DEBUG
using System.Text;
using System.Text.Json.Nodes;
using Viem.Windows.Core;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class StyleDefaultsLoadingTests
{
    private static void Check(bool condition, string message)
    { if (!condition) throw new InvalidOperationException(message); FrontendSmokeTests.UiChecks.Add(message); }
    private static void Write(string path, JsonNode json) => File.WriteAllText(path, json.ToJsonString());
    private static JsonObject Read(string path) => JsonNode.Parse(File.ReadAllBytes(path))!.AsObject();

    internal static async Task Run(Preferences ownerPreferences)
    {
        string directory = Path.Combine(ownerPreferences.DirectoryPath, "theme-tests", Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        var preferences = new Preferences(directory);
        Check(preferences.SelectedTheme == "Midnight" && preferences.ThemeNames.SequenceEqual(new[] { "Midnight", "Paper" }),
            "a fresh profile seeds Paper and Midnight and selects Midnight");
        Check(new Preferences(directory).SelectedTheme == "Midnight", "selected theme survives preferences reload");
        byte[] midnight = File.ReadAllBytes(preferences.SelectedThemePath!);
        preferences.SelectTheme(null);
        byte[] config = File.ReadAllBytes(Path.Combine(directory, "config.json"));
        preferences.Set("theme", "statusFontSize", JsonValue.Create(17d));
        Check(preferences.StatusFontSize == 17 && new Preferences(directory).StatusFontSize == 11
            && File.ReadAllBytes(Path.Combine(directory, "config.json")).SequenceEqual(config)
            && File.ReadAllBytes(Path.Combine(preferences.ThemesDirectory, "Midnight.json")).SequenceEqual(midnight),
            "Default theme changes are memory only and leave named files and main config untouched");
        preferences.CreateTheme("Custom");
        Check(preferences.SelectedTheme == "Custom" && new Preferences(directory).StatusFontSize == 17,
            "New theme clones the current in-memory Default and selects the saved clone");
        foreach (string name in new[] { "", "Default", "../escape", "CON", "custom", new string('a', 33) }) {
            bool rejected = false; try { preferences.CreateTheme(name); } catch { rejected = true; }
            Check(rejected && preferences.SelectedTheme == "Custom", $"invalid or duplicate theme name '{name}' preserves the selection");
        }
        string customPath = preferences.SelectedThemePath!;
        var external = Read(customPath); external["future"] = new JsonObject { ["enabled"] = true };
        external["theme"]!["futureColor"] = 42; external["theme"]!["statusFontFamily"] = "Consolas"; Write(customPath, external);
        preferences.Set("theme", "statusFontSize", JsonValue.Create(18d));
        var edited = Read(customPath);
        Check(edited["future"]!["enabled"]!.GetValue<bool>() && edited["theme"]!["futureColor"]!.GetValue<int>() == 42
            && preferences.StatusFontFamily == "Consolas", "theme edits preserve extension fields and unrelated external edits");
        // Property maps replace rather than retaining a removed OpenType tag;
        // extension data on surviving definitions is retained.
        var stored = Read(customPath);
        var originalStyle = stored["styles"]!["markdown"]!["block_styles"]!.AsArray().Single(entry => entry!["id"]!.GetValue<string>() == "Paragraph")!;
        originalStyle["futureDefinition"] = 7;
        originalStyle["character"]!["futureProperty"] = "retained";
        originalStyle["character"]!["open_type_features"] = new JsonObject { ["liga"] = 0 };
        Write(customPath, stored);
        var replacement = JsonNode.Parse(preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN))!;
        replacement["block_styles"]!.AsArray().Single(entry => entry!["id"]!.GetValue<string>() == "Paragraph")!["character"]!["open_type_features"] = new JsonObject();
        preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, Encoding.UTF8.GetBytes(replacement.ToJsonString()));
        var savedStyle = Read(customPath)["styles"]!["markdown"]!["block_styles"]!.AsArray().Single(entry => entry!["id"]!.GetValue<string>() == "Paragraph")!;
        Check(savedStyle["futureDefinition"]!.GetValue<int>() == 7 && savedStyle["character"]!["futureProperty"]!.GetValue<string>() == "retained"
            && savedStyle["character"]!["open_type_features"]!.AsObject().Count == 0,
            "theme style saving retains extensions but removes cleared feature-map declarations");
        var codeStyles = JsonNode.Parse(preferences.ThemeStyleDefaults(VIEM_FORMAT_CODE))!;
        var customStyle = codeStyles["character_styles"]!.AsArray()[0]!.DeepClone();
        customStyle["id"] = "Theme test style"; customStyle["name"] = "Theme test style";
        codeStyles["character_styles"]!.AsArray().Add(customStyle);
        preferences.SaveThemeStyles(VIEM_FORMAT_CODE, Encoding.UTF8.GetBytes(codeStyles.ToJsonString()));
        codeStyles["character_styles"]!.AsArray().Remove(customStyle);
        preferences.SaveThemeStyles(VIEM_FORMAT_CODE, Encoding.UTF8.GetBytes(codeStyles.ToJsonString()));
        Check(!Read(customPath)["styles"]!["code"]!["character_styles"]!.AsArray().Any(entry => entry!["id"]!.GetValue<string>() == "Theme test style"),
            "saving a theme after deleting a style does not restore its old definition");
        string externalPath = Path.Combine(preferences.ThemesDirectory, ".External"); File.WriteAllBytes(externalPath, midnight);
        string namedDefaultPath = Path.Combine(preferences.ThemesDirectory, "Default.json"); File.WriteAllBytes(namedDefaultPath, midnight);
        Check(preferences.ThemeFiles.Any(file => file.Name == ".External" && file.Path == externalPath),
            "theme catalogue lists every regular file, including extensions other than JSON and leading dots");
        preferences.SelectTheme("Default", namedDefaultPath);
        Check(preferences.SelectedTheme == "Default" && preferences.SelectedThemePath == namedDefaultPath,
            "a file named Default is distinct from the built-in Default selection");
        string duplicateBare = Path.Combine(preferences.ThemesDirectory, "Ocean"), duplicateJson = Path.Combine(preferences.ThemesDirectory, "Ocean.json");
        File.WriteAllBytes(duplicateBare, midnight); File.WriteAllBytes(duplicateJson, midnight);
        preferences.SelectTheme("Ocean", duplicateJson);
        Check(new Preferences(directory).SelectedThemePath == duplicateJson,
            "the selected filename persists when two theme files share a display name");
        File.Delete(duplicateJson);
        Check(new Preferences(directory).SelectedTheme == null && File.Exists(duplicateBare),
            "a missing selected filename falls back to Default without selecting another file with the same name");
        preferences.SelectTheme(".External", externalPath);
        preferences.Set("theme", "statusFontSize", JsonValue.Create(19d));
        Check(Read(externalPath)["theme"]!["statusFontSize"]!.GetValue<int>() == 19,
            "external theme filenames keep their actual path when saving");
        preferences.DeleteTheme();
        Check(preferences.SelectedTheme == null && !File.Exists(externalPath) && File.Exists(customPath),
            "deleting the selected theme returns to Default and leaves other themes intact");

        preferences.CreateTheme("Disappearing");
        string disappearedPath = preferences.SelectedThemePath!; File.Delete(disappearedPath);
        preferences.Set("theme", "statusFontSize", JsonValue.Create(20d));
        Check(preferences.SelectedTheme == null && preferences.StatusFontSize == 20 && !File.Exists(disappearedPath)
            && new Preferences(directory).StatusFontSize == 11,
            "editing after an active theme file disappears uses memory-only Default and never recreates the missing file");

        string invalid = Path.Combine(preferences.ThemesDirectory, "Broken.json"); File.WriteAllText(invalid, "{");
        var root = Read(Path.Combine(directory, "config.json")); root["selectedTheme"] = "Broken"; Write(Path.Combine(directory, "config.json"), root);
        var broken = new Preferences(directory);
        Check(broken.SelectedTheme == null && broken.Error != null && File.ReadAllText(invalid) == "{",
            "an invalid selected theme falls back to Default without overwriting the file");
        root["selectedTheme"] = "Missing"; Write(Path.Combine(directory, "config.json"), root);
        Check(new Preferences(directory).SelectedTheme == null, "a missing selected theme falls back to Default");

        string legacyDirectory = Path.Combine(directory, "legacy"); Directory.CreateDirectory(legacyDirectory);
        string legacyConfigPath = Path.Combine(legacyDirectory, "config.json");
        var legacyConfig = new JsonObject {
            ["version"] = 1, ["theme"] = new JsonObject { ["statusFontSize"] = "obsolete", ["foreground"] = false },
            ["editing"] = new JsonObject { ["smartQuotes"] = true }, ["future"] = 99
        };
        Write(legacyConfigPath, legacyConfig);
        byte[] legacyConfigBytes = File.ReadAllBytes(legacyConfigPath);
        string[] retiredStyleFiles = ["text_style.json", "markdown_style.json", "code_style.json"];
        foreach (string file in retiredStyleFiles) File.WriteAllText(Path.Combine(legacyDirectory, file), "obsolete stylesheet");
        var ignored = new Preferences(legacyDirectory);
        Check(ignored.SelectedTheme == null && ignored.StatusFontSize == 11 && ignored.SmartQuotes && ignored.Error == null
            && ignored.ThemeNames.Length == 0 && !Directory.Exists(ignored.ThemesDirectory)
            && File.ReadAllBytes(legacyConfigPath).SequenceEqual(legacyConfigBytes)
            && retiredStyleFiles.All(file => File.ReadAllText(Path.Combine(legacyDirectory, file)) == "obsolete stylesheet"),
            "old appearance and top-level stylesheet files are ignored without importing, validating, or rewriting them");
        ignored.Set("editing", "smartQuotes", JsonValue.Create(false));
        Check(!new Preferences(legacyDirectory).SmartQuotes
            && JsonNode.DeepEquals(Read(legacyConfigPath)["theme"], legacyConfig["theme"])
            && Read(legacyConfigPath)["future"]!.GetValue<int>() == 99,
            "obsolete appearance data does not block preference edits and is preserved as an unknown config field");

        Directory.CreateDirectory(ignored.ThemesDirectory);
        string importedPath = Path.Combine(ignored.ThemesDirectory, "Imported.json");
        var currentTheme = JsonNode.Parse(midnight)!; currentTheme["theme"]!["statusFontSize"] = 16; Write(importedPath, currentTheme);
        ignored.SelectTheme("Imported");
        var selected = new Preferences(legacyDirectory);
        Check(selected.SelectedTheme == "Imported" && selected.SelectedThemePath == importedPath && selected.StatusFontSize == 16
            && selected.Error == null && selected.ThemeNames.SequenceEqual(new[] { "Imported" })
            && JsonNode.DeepEquals(Read(legacyConfigPath)["theme"], legacyConfig["theme"]),
            "a current-format theme named Imported loads normally despite obsolete profile data");

        string freshDirectory = Path.Combine(directory, "legacy-styles-only"); Directory.CreateDirectory(freshDirectory);
        foreach (string file in retiredStyleFiles) File.WriteAllText(Path.Combine(freshDirectory, file), "obsolete stylesheet");
        var fresh = new Preferences(freshDirectory);
        Check(fresh.SelectedTheme == "Midnight" && fresh.ThemeNames.SequenceEqual(new[] { "Midnight", "Paper" }) && fresh.Error == null
            && retiredStyleFiles.All(file => File.ReadAllText(Path.Combine(freshDirectory, file)) == "obsolete stylesheet"),
            "obsolete top-level styles do not suppress fresh-profile presets or create an Imported theme");

        preferences.SelectTheme("Custom");
        var window = new EditorWindow(preferences); App.Instance.Windows.Add(window);
        try {
            window.Activate(); await window.ActivePane!.Ready.WaitAsync(TimeSpan.FromSeconds(10));
            byte[] source = Encoding.UTF8.GetBytes("# Title\n\nBody");
            string path = Path.Combine(directory, "live.md"); File.WriteAllBytes(path, source); await window.OpenPath(path);
            var view = await window.ActivePane!.Ready.WaitAsync(TimeSpan.FromSeconds(10));
            var document = view.Document;
            view.Command("ggi"); view.Text("prefix "); view.Key(VIEM_KEY_ESCAPE);
            byte[] changed = document.Source(document.State.document_revision); ulong revision = document.State.document_revision;
            var styles = JsonNode.Parse(preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN))!.AsObject();
            styles["block_styles"]!.AsArray().Single(entry => entry!["id"]!.GetValue<string>() == "Paragraph")!["character"]!["weight"] = 650;
            preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, Encoding.UTF8.GetBytes(styles.ToJsonString()));
            Check(document.State.document_revision == revision && document.IsDirty && document.Source(revision).SequenceEqual(changed)
                && view.Styles().Styles.Single(style => style.Id == "Paragraph").Value(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).enum_value == 650,
                "theme style changes apply to open documents without source or revision changes");
            view.Undo(); Check(document.Source(document.State.document_revision).SequenceEqual(source) && !document.IsDirty,
                "theme changes preserve the existing source undo record and saved baseline");

        }
        finally {
            window.Close(); App.Instance.Windows.Remove(window);
            CoreThemes.ReplaceCodeStyles(ownerPreferences.ThemeStyleDefaults(VIEM_FORMAT_CODE));
        }
    }
}
#endif

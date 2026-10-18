using System.Text.Json.Nodes;

namespace Viem.Windows.Shell;

internal sealed partial class Preferences
{
    internal static JsonObject DefaultVisibleWhitespaceStyle() => new() {
        ["foreground"] = new JsonObject { ["red"] = 0d, ["green"] = 0d, ["blue"] = 139d / 255, ["alpha"] = 1d }
    };
    private static JsonObject VisibleWhitespaceStyleIn(JsonObject settings) =>
        settings["editing"]?["whitespacePresentation"]?["visibleWhitespace"]?["style"] is JsonObject style
            ? (JsonObject)style.DeepClone() : DefaultVisibleWhitespaceStyle();
    internal JsonObject VisibleWhitespaceStyle => VisibleWhitespaceStyleIn(root);

    internal void SaveVisibleWhitespaceStyle(JsonObject expected, JsonObject replacement)
    {
        // Update rereads disk before this comparison and validates the complete
        // candidate before its atomic write. Clearing replaces this one style
        // object, retaining every unrelated preference and unknown style field.
        Update(candidate => {
            if (!JsonNode.DeepEquals(VisibleWhitespaceStyleIn(candidate), expected))
                throw new InvalidOperationException("Visible whitespace changed in another settings owner. Review the current values and repeat the edit.");
            if (candidate["editing"] is not JsonObject) candidate["editing"] = new JsonObject();
            var editing = candidate["editing"]!.AsObject();
            if (editing["whitespacePresentation"] is not JsonObject) editing["whitespacePresentation"] = new JsonObject();
            var whitespace = editing["whitespacePresentation"]!.AsObject();
            if (whitespace["visibleWhitespace"] is not JsonObject) whitespace["visibleWhitespace"] = new JsonObject();
            whitespace["visibleWhitespace"]!["style"] = replacement.DeepClone();
        });
    }

    internal void RefreshVisibleWhitespaceStyle()
    {
        string path = Path.Combine(DirectoryPath, "config.json");
        if (!File.Exists(path)) return;
        var latest = VisibleWhitespaceStyleIn(Read(path));
        if (JsonNode.DeepEquals(latest, VisibleWhitespaceStyle)) return;
        if (root["editing"] is not JsonObject) root["editing"] = new JsonObject();
        if (root["editing"]!["whitespacePresentation"] is not JsonObject) root["editing"]!["whitespacePresentation"] = new JsonObject();
        if (root["editing"]!["whitespacePresentation"]!["visibleWhitespace"] is not JsonObject)
            root["editing"]!["whitespacePresentation"]!["visibleWhitespace"] = new JsonObject();
        root["editing"]!["whitespacePresentation"]!["visibleWhitespace"]!["style"] = latest;
        Changed?.Invoke();
    }
}

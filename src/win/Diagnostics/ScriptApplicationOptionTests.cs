#if DEBUG
using System.Text.Json.Nodes;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class ScriptApplicationOptionTests
{
    internal static void Run(Action<bool, string> check)
    {
        string directory = Path.Combine(Path.GetTempPath(), "viem-script-options-" + Guid.NewGuid().ToString("N"));
        try
        {
            var preferences = new Preferences(directory);
            string path = Path.Combine(directory, "config.json");
            byte[]? before = File.Exists(path) ? File.ReadAllBytes(path) : null;
            preferences.ApplyScriptOption("timeoutlen", "25");
            preferences.ApplyScriptOption("timeout", "0");
            preferences.ApplyScriptOption("laststatus", "0");
            preferences.ApplyScriptOption("guioptions", "a");
            preferences.ApplyScriptOption("scrolloff", "4");
            check(preferences.ScriptTimeoutLength == 25 && !preferences.ScriptTimeout && !preferences.ShowStatus
                && !preferences.ShowMenu && preferences.ScriptScrollOff == 4, "mapped Vim application options update native effective settings");
            preferences.ApplyScriptOption("breakindentopt", "shift:8");
            bool rejected = false;
            try { preferences.ApplyScriptOption("breakindentopt", "min:20"); } catch (InvalidDataException) { rejected = true; }
            check(rejected && preferences.ScriptContinuationIndent == 8, "unsupported continuation settings preserve accepted native state");
            preferences.ApplyScriptOption("guifont", "Consolas:h18:b:i");
            preferences.ApplyScriptOption("linespace", "4");
            var code = JsonNode.Parse(preferences.ThemeStyleDefaults(VIEM_FORMAT_CODE))!;
            var paragraph = code["block_styles"]!.AsArray().First(block => block?["id"]?.GetValue<string>() == "Paragraph")!;
            check(paragraph["character"]?["size"]?.GetValue<double>() == 18
                && paragraph["character"]?["font_families"]?[0]?.GetValue<string>() == "Consolas",
                "guifont changes the shared Code font in memory");
            byte[] accepted = preferences.ThemeStyleDefaults(VIEM_FORMAT_CODE); rejected = false;
            try { preferences.ApplyScriptOption("guifont", "Consolas:h18:w20"); } catch (InvalidDataException) { rejected = true; }
            check(rejected && accepted.AsSpan().SequenceEqual(preferences.ThemeStyleDefaults(VIEM_FORMAT_CODE)), "invalid guifont leaves Code style unchanged");
            check(before == null ? !File.Exists(path) : before.AsSpan().SequenceEqual(File.ReadAllBytes(path)), "script application options do not write settings files");
        }
        finally { if (Directory.Exists(directory)) Directory.Delete(directory, true); }
    }
}
#endif

#if DEBUG
using System.Text;
using System.Text.Json.Nodes;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class StyleDefaultsLoadingTests
{
    private static void Check(bool condition, string message)
    { if (!condition) throw new InvalidOperationException(message); FrontendSmokeTests.UiChecks.Add(message); }

    internal static async Task Run(Preferences ownerPreferences)
    {
        string directory = Path.Combine(ownerPreferences.DirectoryPath, "style-defaults-loading", Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        var preferences = new Preferences(directory);
        var window = new EditorWindow(preferences);
        App.Instance.Windows.Add(window);
        window.Closed += (_, _) => App.Instance.Windows.Remove(window);
        try
        {
            window.Activate();
            await window.ActivePane!.Ready.WaitAsync(TimeSpan.FromSeconds(10));
            byte[] source = Encoding.UTF8.GetBytes("> Quote\n\n```\ncode\n```\n");
            string initial = Path.Combine(directory, "initial.md");
            File.WriteAllBytes(initial, source);
            await window.OpenPath(initial);
            var initialView = await window.ActivePane!.Ready.WaitAsync(TimeSpan.FromSeconds(10));
            byte[] defaults = initialView.ExportStyleDefaults();
            uint normalWeight = initialView.Styles().Styles.Single(s => s.Id == "Paragraph" && s.Namespace == 1)
                .Value(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).enum_value;

            var unsupported = JsonNode.Parse(defaults)!.AsObject();
            unsupported["version"] = 999;
            var mixed = JsonNode.Parse(defaults)!.AsObject();
            var definitions = mixed["block_styles"]!.AsArray();
            definitions.Single(entry => entry!["id"]!.GetValue<string>() == "Block quote")!["role"] = "Paragraph";
            definitions.Single(entry => entry!["id"]!.GetValue<string>() == "Paragraph")!["character"]!["weight"] = 650;
            var cases = new (string Name, byte[] Json, bool WholeFileRejected, bool Partial)[] {
                ("malformed", "{"u8.ToArray(), true, false),
                ("unsupported", Encoding.UTF8.GetBytes(unsupported.ToJsonString()), true, false),
                ("mixed", Encoding.UTF8.GetBytes(mixed.ToJsonString()), false, true),
                ("valid", defaults, false, false),
            };
            string settings = Path.Combine(directory, "markdown_style.json");
            foreach (var test in cases)
            {
                File.WriteAllBytes(settings, test.Json);
                string path = Path.Combine(directory, test.Name + ".md");
                File.WriteAllBytes(path, source);
                await window.OpenPath(path);
                var pane = window.ActivePane!;
                var view = await pane.Ready.WaitAsync(TimeSpan.FromSeconds(10));
                Check(pane.LastError == null && pane.Document.Handle != 0 && !pane.Document.IsDirty
                    && pane.Document.Source(pane.Document.State.document_revision).AsSpan().SequenceEqual(source),
                    $"{test.Name} style settings do not prevent document creation or mutate its source");
                string message = pane.CommandOutputText ?? "";
                if (test.WholeFileRejected || test.Partial)
                    Check(message.Contains(settings, StringComparison.Ordinal), $"{test.Name} settings warning survives until the pane is ready and names the file");
                else Check(message.Length == 0, "valid style defaults open without a stale settings warning");
                if (test.WholeFileRejected)
                    Check(message.Contains("Using built-in styles.", StringComparison.Ordinal), $"{test.Name} whole-file failure explains the built-in fallback");
                if (test.Partial)
                    Check(message.Contains("Block quote", StringComparison.Ordinal), "partial style warning identifies the rejected setting");
                var styles = view.Styles().Styles;
                Check(styles.Single(s => s.Id == "Block quote" && s.Namespace == 1).Native.role == VIEM_STYLE_ROLE_QUOTE
                    && styles.Single(s => s.Id == "Code Block" && s.Namespace == 1).Native.role == VIEM_STYLE_ROLE_CODE_BLOCK,
                    $"{test.Name} settings retain valid structural container roles");
                Check(styles.Single(s => s.Id == "Paragraph" && s.Namespace == 1).Value(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).enum_value
                    == (test.Partial ? 650u : normalWeight), $"{test.Name} settings preserve usable declarations and use defaults only where needed");
                Check(File.ReadAllBytes(settings).AsSpan().SequenceEqual(test.Json), $"{test.Name} settings are not migrated or overwritten");
            }
        }
        finally { App.Instance.Windows.Remove(window); window.Close(); }
    }
}
#endif

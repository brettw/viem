#if DEBUG
using System.Text;
using System.Text.Json.Nodes;
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Dispatching;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class VimRuntimeTests
{
    private static void Check(bool condition, string name)
    { if (!condition) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }

    internal static async Task Run(CanvasDevice device, DispatcherQueue dispatcher, string profile)
    {
        bool missing = Environment.GetEnvironmentVariable("VIEM_TEST_VIM_MISSING") == "1";
        Check(Path.IsPathFullyQualified(BundledVimRuntime.SyntaxDirectory)
            && BundledVimRuntime.SyntaxDirectory.StartsWith(AppContext.BaseDirectory, StringComparison.OrdinalIgnoreCase),
            "Vim resources resolve beside the executable independently of the working directory");
        Check((BundledVimRuntime.Diagnostic != null) == missing, "bundled resource availability has a nonfatal diagnostic");

        string directory = Path.Combine(profile, "vim-settings"); Directory.CreateDirectory(directory);
        string config = Path.Combine(directory, "config.json");
        // An invalid JSON type in the retired setting must not invalidate the profile.
        foreach (JsonNode legacy in new JsonNode[] { JsonValue.Create("C:\\unused-vim")!, new JsonObject { ["obsolete"] = true } })
        {
            var original = new JsonObject { ["version"] = 1, ["future"] = 42, ["code"] = new JsonObject {
                ["vimSyntaxDirectory"] = legacy, ["future"] = "preserved", ["filenameAssociations"] = new JsonArray() } };
            File.WriteAllText(config, original.ToJsonString());
            var preferences = new Preferences(directory);
            Check(preferences.Error == null && JsonNode.DeepEquals(JsonNode.Parse(File.ReadAllText(config)), original),
                "legacy syntax directory is ignored without writing on read");
            preferences.Set("editing", "smartQuotes", true);
            var saved = JsonNode.Parse(File.ReadAllText(config))!;
            Check(!((JsonObject)saved["code"]!).ContainsKey("vimSyntaxDirectory")
                && saved["future"]!.GetValue<int>() == 42 && saved["code"]!["future"]!.GetValue<string>() == "preserved"
                && saved["code"]!["filenameAssociations"] is JsonArray
                && !File.ReadAllText(config).Contains(BundledVimRuntime.SyntaxDirectory),
                "settings writes retire only the syntax directory and never persist resource paths");
        }

        async Task Paint(string filename, string text, ulong tokenLength)
        {
            byte[] source = Encoding.UTF8.GetBytes(text);
            using var document = new CoreDocument(source, Path.Combine(directory, filename));
            var preferences = new Preferences(directory);
            document.ConfigureDefaults(preferences.Indentation, preferences.Whitespace, preferences.TextWidth, preferences.Associations);
            Check(document.State.format == VIEM_FORMAT_CODE, filename + " opens as Code");
            ulong revision = document.State.document_revision;
            using var view = new CoreView(document, device, dispatcher, 700, 400);
            bool painted = false;
            for (int attempt = 0; attempt < 500; attempt++)
            {
                document.PollSyntax();
                view.Refresh();
                LayoutSnapshot layout;
                try { layout = view.Layout(); }
                catch (CoreException error) when (error.Status == VIEM_STATUS_LAYOUT_UNAVAILABLE)
                {
                    // Match EditorPane.Refresh's deferred layout materialization.
                    view.Resize(700, 400); layout = view.Layout();
                }
                painted = layout.PaintRuns.Any(run => run.text_start == 0 && run.text_end >= tokenLength
                    && (run.paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0);
                if (painted || (missing && filename == ".vimrc" && document.SyntaxDiagnostics.Length > 0)) break;
                await Task.Delay(10);
            }
            if (missing && filename == ".vimrc")
                Check(!painted && document.SyntaxDiagnostics.Length > 0, "missing Vim runtime reports worker diagnostics and retains default styling");
            else Check(painted, filename + " receives syntax paint: " + document.SyntaxDiagnostics);
            Check(document.State.document_revision == revision && !document.IsDirty
                && document.Source(revision).AsSpan().SequenceEqual(source), filename + " highlighting preserves revision, bytes and clean state");
            view.Command("i"); view.Text("x"); view.Key(VIEM_KEY_ESCAPE);
            Check(document.IsDirty, filename + " remains editable");
            view.Undo();
            Check(document.Source(document.State.document_revision).AsSpan().SequenceEqual(source), filename + " undo restores source bytes");
        }
        var legacyProfile = JsonNode.Parse(File.ReadAllText(config))!;
        legacyProfile["code"]!["vimSyntaxDirectory"] = Path.Combine(directory, "external syntax");
        Directory.CreateDirectory(legacyProfile["code"]!["vimSyntaxDirectory"]!.GetValue<string>());
        File.WriteAllText(Path.Combine(legacyProfile["code"]!["vimSyntaxDirectory"]!.GetValue<string>(), "vim.vim"), "\" Deliberately supplies no highlighting.\n");
        File.WriteAllText(config, legacyProfile.ToJsonString());
        await Paint(".vimrc", "set number\r\nset expandtab\r\n\" editor settings\r\nlet g:example = 'value'\r\n", 3);
        if (!missing)
        {
            await Paint("Makefile", "all: input.txt\r\n\t@echo $(MESSAGE)\r\n", 3);
            // This distribution keyword is generated from shared/debversions.vim.
            await Paint("sources.list", "bookworm\n# vim: ft=debsources\n", 8);
        }
        await Paint("sample.rs", "fn main() { let value = 42; }\n", 2);
    }
}
#endif

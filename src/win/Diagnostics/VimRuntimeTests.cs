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
            if (filename == "sample.cc")
            {
                view.Command("A"); view.Text("é");
                var layout = view.Layout();
                Check(layout.PaintRuns.Any(run => run.text_start <= tokenLength && run.text_end >= tokenLength + 2
                    && (run.paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0),
                    "C++ comment typing inherits syntax paint in the first native layout");
                view.Key(VIEM_KEY_ESCAPE); view.Undo();
            }
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
        await Paint("sample.cc", "// comment\nint main() { return 0; }\n", 10);

        async Task ScrollPaint(string filename, Func<int, string> line)
        {
            byte[] source = Encoding.UTF8.GetBytes(string.Concat(Enumerable.Range(0, 1_024).Select(line)));
            var document = new CoreDocument(source, Path.Combine(directory, filename));
            var window = new EditorWindow(new Preferences(directory), document);
            App.Instance.Windows.Add(window); window.Activate();
            try
            {
                var pane = window.ActivePane!; var view = await pane.Ready;
                bool ColoredVisible()
                {
                    var layout = view.Layout(); var viewport = view.Viewport;
                    var rows = layout.Rows.Where(row => row.y + Math.Max(row.line_advance, 1) > viewport.top
                        && row.y < viewport.top + layout.Info.viewport_height).ToArray();
                    return rows.Length > 0 && layout.PaintRuns.Any(run => run.text_end > rows[0].text_start
                        && run.text_start < rows[^1].text_end
                        && (run.paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0);
                }
                // Let cold provider setup finish. The subsequent scroll must
                // publish its first frame without yielding to the syntax timer.
                for (int attempt = 0; attempt < 500 && !ColoredVisible(); attempt++)
                { document.PollSyntax(); pane.Refresh(); await Task.Delay(10); }
                Check(ColoredVisible(), filename + " completes cold syntax setup");
                var wait = System.Diagnostics.Stopwatch.StartNew();
                view.Scroll(0, 8_192);
                if (!ColoredVisible())
                {
                    // Real providers may exceed the shared 100 ms grace period.
                    Check(wait.Elapsed.TotalMilliseconds >= 80, filename + " gives pending syntax its presentation grace period");
                    for (int attempt = 0; attempt < 500 && !ColoredVisible(); attempt++)
                    { document.PollSyntax(); pane.Refresh(); await Task.Delay(10); }
                }
                Check(ColoredVisible(), filename + " installs visible syntax before or after the bounded wait");
                Check(view.Viewport.top > 0 && pane.LastError == null, filename + " scrolling retains exact native presentation");
                Check(!document.IsDirty && document.Source(document.State.document_revision).AsSpan().SequenceEqual(source),
                    filename + " bounded syntax wait preserves source and clean state");
            }
            finally { App.Instance.Windows.Remove(window); window.Close(); }
        }
        await ScrollPaint("scroll.rs", i => $"fn item_{i}() {{ let value = {i}; }}\n");
        if (!missing) await ScrollPaint("scroll.vim", i => $"set number \" row {i}\n");
    }
}
#endif

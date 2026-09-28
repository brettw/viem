#if DEBUG
using System.Text.Json.Nodes;
using Viem.Windows.Core;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class MarkdownViewPreferenceTests
{
    private static void Check(bool value, string name)
    { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }

    internal static async Task Run(string profile)
    {
        string directory = Path.Combine(profile, "markdown-view-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        var preferences = new Preferences(directory);
        Check(!preferences.MarkdownFormattedView, "Markdown without a remembered view defaults to Source");
        byte[] source = "# Heading\n\nSome **text**.\n"u8.ToArray();
        string firstPath = Path.Combine(directory, "first.md"), secondPath = Path.Combine(directory, "second.markdown");
        await File.WriteAllBytesAsync(firstPath, source); await File.WriteAllBytesAsync(secondPath, source);
        var window = new EditorWindow(preferences);
        App.Instance.Windows.Add(window); window.Activate();
        try
        {
            await window.ActivePane!.Ready;
            await window.OpenNative(firstPath);
            var first = window.ActivePane!; var firstView = await first.Ready;
            Check(first.Document.State.format == VIEM_FORMAT_MARKDOWN_SOURCE,
                "native Markdown open uses Source when no choice has been saved");
            int settingsChanges = 0;
            preferences.Changed += () => settingsChanges++;
            firstView.SetMarkdownSource(false);
            Check(preferences.MarkdownFormattedView && new Preferences(directory).MarkdownFormattedView && settingsChanges == 0,
                "a successful formatted-view change persists without reapplying document settings");

            firstView.Ex("split " + secondPath); await window.PendingEffectsForTesting;
            var second = window.ActivePane!; var secondView = await second.Ready;
            Check(second.Document.FilePath == secondPath && second.Document.State.format == VIEM_FORMAT_MARKDOWN
                && !second.Document.IsDirty
                && (second.Document.State.flags & (VIEM_DOCUMENT_STATE_CAN_UNDO | VIEM_DOCUMENT_STATE_CAN_REDO)) == 0
                && second.Document.Source(second.Document.State.document_revision).AsSpan().SequenceEqual(source),
                "Ex Markdown open uses the remembered view without changing source or adding history");

            firstView.SetMarkdownSource(true);
            second.Document.NotifyChanged(); secondView.Refresh(); window.Toolbar.Refresh();
            preferences.ObserveMarkdownView(second.Document, error => throw error);
            Check(!preferences.MarkdownFormattedView && !new Preferences(directory).MarkdownFormattedView,
                "refreshing or reattaching another document cannot overwrite the latest view choice");
            firstView.Undo();
            Check(preferences.MarkdownFormattedView && new Preferences(directory).MarkdownFormattedView,
                "undoing a Markdown view change updates the remembered choice");
            firstView.Redo();
            Check(!preferences.MarkdownFormattedView && !new Preferences(directory).MarkdownFormattedView,
                "redoing a Markdown view change updates the remembered choice");

            string newPath = Path.Combine(directory, "new.md");
            secondView.Ex("split " + newPath); await window.PendingEffectsForTesting;
            var empty = window.ActivePane!; await empty.Ready;
            Check(empty.Document.FilePath == newPath && empty.Document.State.format == VIEM_FORMAT_MARKDOWN_SOURCE
                && empty.Document.State.source_byte_count == 0,
                "new named Markdown files use the last Source choice");
            firstView.SetMarkdownSource(false);
            var reloaded = new Preferences(directory);
            using (var reopened = new CoreDocument(source, "reopened.md", markdownFormattedView: reloaded.MarkdownFormattedView))
                Check(reopened.State.format == VIEM_FORMAT_MARKDOWN,
                    "a new preferences instance restores formatted Markdown opening");

            var snapshot = new RecoverySnapshot(source, "markdownSource", 1, 1, 1, 1);
            var recovered = window.AddPane(new CoreDocument(snapshot.source, format: snapshot.Format,
                encoding: snapshot.encoding, fileFormat: snapshot.fileFormat, markdownFormattedView: preferences.MarkdownFormattedView));
            var recoveredView = await recovered.Ready; recovered.Document.MarkRecovered();
            Check(recovered.Document.State.format == VIEM_FORMAT_MARKDOWN_SOURCE && preferences.MarkdownFormattedView,
                "explicit recovery format is retained without replacing the remembered choice");
            foreach (uint format in new[] { VIEM_FORMAT_PLAIN_TEXT, VIEM_FORMAT_CODE })
            {
                var literal = window.AddPane(new CoreDocument(source, format: format, markdownFormattedView: true));
                var view = await literal.Ready; view.Command("i"); view.Text("literal"); view.Key(VIEM_KEY_ESCAPE);
                Check(literal.Document.State.format == format && preferences.MarkdownFormattedView,
                    "literal document changes do not replace the remembered Markdown view: " + format);
            }
            string config = Path.Combine(directory, "config.json");
            var external = JsonNode.Parse(File.ReadAllText(config))!;
            external["editing"]!["markdownFormattedView"] = false;
            File.WriteAllText(config, external.ToJsonString());
            recoveredView.SetMarkdownSource(false);
            Check(new Preferences(directory).MarkdownFormattedView,
                "the latest actual view choice replaces an externally changed value even when the cached preference matches");
            byte[] before = File.ReadAllBytes(config);
            foreach (JsonNode? invalid in new JsonNode?[] { null, JsonValue.Create("invalid"), JsonValue.Create(1) })
            {
                bool rejected = false;
                try { preferences.Set("editing", "markdownFormattedView", invalid); } catch { rejected = true; }
                Check(rejected && File.ReadAllBytes(config).AsSpan().SequenceEqual(before),
                    "invalid remembered-view value leaves settings untouched: " + (invalid?.ToJsonString() ?? "null"));
            }
            Check(window.Panes.All(p => p.LastError == null), "remembered Markdown view scenarios leave no presentation errors");

            string brokenDirectory = Path.Combine(directory, "failed-write");
            Directory.CreateDirectory(brokenDirectory);
            var broken = new Preferences(brokenDirectory);
            using var document = new CoreDocument(source, format: VIEM_FORMAT_MARKDOWN_SOURCE);
            using var brokenView = new CoreView(document, first.Canvas.Device, first.DispatcherQueue, 500, 200);
            Exception? failure = null;
            broken.ObserveMarkdownView(document, error => failure = error);
            File.WriteAllText(Path.Combine(brokenDirectory, "config.json"), "{\"version\":99}");
            brokenView.SetMarkdownSource(false);
            Check(failure != null && document.State.format == VIEM_FORMAT_MARKDOWN,
                "preference-write failure is reported without undoing the successful view change");
        }
        finally { App.Instance.Windows.Remove(window); window.Close(); }
    }
}
#endif

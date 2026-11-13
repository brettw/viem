#if DEBUG
using System.Text.Json.Nodes;
using Viem.Windows.Core;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class MarkdownViewPreferenceTests
{
    private static void Check(bool value, string name)
    {
        if (!value) throw new InvalidOperationException(name);
        FrontendSmokeTests.UiChecks.Add(name);
    }

    internal static async Task Run(string profile)
    {
        string directory = Path.Combine(profile, "markdown-view-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        var preferences = new Preferences(directory);
        Check(preferences.MarkdownFormattedView, "Markdown without a remembered view defaults to Preview on");
        byte[] source = "# Heading\n\nSome **text**.\n"u8.ToArray();
        string path = Path.Combine(directory, "first.md");
        await File.WriteAllBytesAsync(path, source);
        var window = new EditorWindow(preferences);
        App.Instance.Windows.Add(window); window.Activate();
        EditorWindow? other = null;
        try {
            await window.ActivePane!.Ready;
            await window.OpenNative(path);
            var first = window.ActivePane!;
            var view = await first.Ready;
            var document = first.Document;
            ulong revision = view.State.document_revision;
            Check(view.State.format == VIEM_FORMAT_MARKDOWN, "native Markdown open defaults to Preview");
            using var retainedPreview = new CoreView(document, first.Canvas.Device, first.DispatcherQueue, 300, 100, display: VIEM_VIEW_DISPLAY_PREVIEW);
            first.SetMarkdownPreview(false);
            Check(view.State.format == VIEM_FORMAT_CODE && document.IsMarkdownDocument
                && document.Source(document.State.document_revision).AsSpan().SequenceEqual(source)
                && !new Preferences(directory).MarkdownFormattedView, "Preview off changes only the view and persists its explicit choice");
            int changes = 0;
            preferences.Changed += () => changes++;
            first.SetMarkdownPreview(true);
            Check(view.State.document_revision == revision && changes == 0 && preferences.MarkdownFormattedView
                && (view.State.flags & (VIEM_DOCUMENT_STATE_CAN_UNDO | VIEM_DOCUMENT_STATE_CAN_REDO)) == 0,
                "Preview toggles preserve a live projection revision and history without reapplying settings");
            retainedPreview.Dispose();
            view.Command("i"); view.Text("X"); view.Key(VIEM_KEY_ESCAPE);
            first.SetMarkdownPreview(false);
            view.Undo();
            Check(view.State.format == VIEM_FORMAT_CODE && !preferences.MarkdownFormattedView
                && document.Source(document.State.document_revision).AsSpan().SequenceEqual(source),
                "Undo restores source while preserving Preview and its saved preference");
            view.Redo(); view.Undo();
            Check(!preferences.MarkdownFormattedView, "Redo and undo never overwrite the Preview preference");

            other = new EditorWindow(preferences, document);
            App.Instance.Windows.Add(other); other.Activate();
            var otherPane = other.ActivePane!;
            var otherView = await otherPane.Ready;
            Check(otherView.State.format == VIEM_FORMAT_CODE, "A new window of the same buffer uses the saved Preview choice");
            otherPane.SetMarkdownPreview(true);
            Check(otherView.State.format == VIEM_FORMAT_MARKDOWN && view.State.format == VIEM_FORMAT_CODE,
                "Two windows of one buffer keep independent Preview states");
            window.Activate(); window.FocusPane(first);
            var split = window.SplitPane(first, document);
            var splitView = await split.Ready;
            Check(splitView.Display == view.Display && splitView.State.format == VIEM_FORMAT_CODE && preferences.MarkdownFormattedView,
                "A split copies its source display even when the saved preference differs");
            document.NotifyChanged(); view.Refresh();
            Check(new Preferences(directory).MarkdownFormattedView, "Refreshing another view cannot replace the explicit Preview choice");
            await window.ClosePane(split, force: true);

            // Reload all views together, preserving each presentation and position.
            view.Ex("edit!");
            await window.PendingEffectsForTesting;
            view = first.View!; otherView = otherPane.View!;
            document = first.Document;
            Check(otherPane.Document == document && view.State.format == VIEM_FORMAT_CODE
                && otherView.State.format == VIEM_FORMAT_MARKDOWN && preferences.MarkdownFormattedView,
                "Reload preserves source and Preview displays across windows");
            Check(document.Source(document.State.document_revision).AsSpan().SequenceEqual(source), "Reload preserves exact source bytes");

            string newPath = Path.Combine(directory, "new.md");
            first.SetMarkdownPreview(true); first.SetMarkdownPreview(false);
            view.Ex("split " + newPath);
            await window.PendingEffectsForTesting;
            var empty = window.ActivePane!; await empty.Ready;
            Check(empty.Document.FilePath == newPath && empty.View!.State.format == VIEM_FORMAT_CODE
                && empty.Document.State.source_byte_count == 0 && !empty.Document.IsDirty,
                "Ex opens new Markdown files using the last Preview choice without edits");
            await window.ClosePane(empty, force: true);

            string config = Path.Combine(directory, "config.json");
            var external = JsonNode.Parse(File.ReadAllText(config))!;
            external["editing"]!["markdownFormattedView"] = true;
            File.WriteAllText(config, external.ToJsonString());
            first.SetMarkdownPreview(true); first.SetMarkdownPreview(false);
            Check(!new Preferences(directory).MarkdownFormattedView, "An explicit toggle replaces an externally changed preference");
            byte[] before = File.ReadAllBytes(config);
            foreach (JsonNode? invalid in new JsonNode?[] { null, JsonValue.Create("invalid"), JsonValue.Create(1) }) {
                bool rejected = false;
                try { preferences.Set("editing", "markdownFormattedView", invalid); } catch { rejected = true; }
                Check(rejected && File.ReadAllBytes(config).AsSpan().SequenceEqual(before), "Invalid Preview preference leaves settings intact");
            }
            view.SetDocumentMode(VIEM_DOCUMENT_MODE_CODE, "rust", DocumentModes.Read(view));
            Check(otherView.State.format == VIEM_FORMAT_CODE && !preferences.MarkdownFormattedView,
                "A non-Markdown language makes all views literal without saving a Preview choice");
            Check(first.LastError == null && otherPane.LastError == null, "Mixed presentations and reload leave no native presentation errors");

            File.WriteAllText(config, "{\"version\":99}");
            bool failed = false;
            try { first.SetMarkdownPreview(true); } catch { failed = true; }
            Check(failed && view.State.format == VIEM_FORMAT_MARKDOWN,
                "Preference write failure is reported without undoing a successful view change");
        } finally {
            if (other != null) { App.Instance.Windows.Remove(other); other.Close(); }
            App.Instance.Windows.Remove(window); window.Close();
        }
    }
}
#endif

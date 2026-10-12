#if DEBUG
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class StyleSessionRecoveryTests
{
    private static void Check(bool condition, string message)
    {
        if (!condition) throw new InvalidOperationException(message);
        FrontendSmokeTests.UiChecks.Add(message);
    }

    internal static void Run(EditorPane pane, Preferences preferences)
    {
        uint[] formats = [VIEM_FORMAT_PLAIN_TEXT, VIEM_FORMAT_MARKDOWN, VIEM_FORMAT_CODE];
        byte[][] original = formats.Select(preferences.ThemeStyleDefaults).ToArray();
        using var document = new CoreDocument("Body"u8.ToArray(), format: VIEM_FORMAT_PLAIN_TEXT);
        preferences.AttachThemeDocument(document);
        using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        var inspector = new StyleWindow(view, preferences, followCaret: false);
        StyleWindow? other = null;
        int publications = 0;
        void Published() => publications++;
        preferences.ThemesChanged += Published;
        try {
            float Size(StyleWindow window) => window.ThemeView.Styles().Styles.Single(s => s.Id == "Status bar")
                .Value(VIEM_STYLE_PROPERTY_CHARACTER_SIZE).number;
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == "Status bar");
            Check(inspector.EditPropertyForTesting(VIEM_STYLE_PROPERTY_CHARACTER_SIZE, 29),
                "preview recovery fixture declares its own initial style size");
            publications = 0;
            var originalView = inspector.ThemeView;
            int attempts = 0;
            Action unavailable = () => {
                attempts++;
                throw new InvalidOperationException("Injected unavailable style preview");
            };
            inspector.ThemeSessionCreationForTesting = unavailable;
            // Shared chrome updates the foreign family contexts too, requiring
            // a fresh specimen after the setting has already been published.
            Check(!inspector.EditPropertyForTesting(VIEM_STYLE_PROPERTY_CHARACTER_SIZE, 31)
                && attempts == 1 && publications == 1
                && ReferenceEquals(inspector.ThemeView, originalView) && originalView.Id != 0,
                "failed preview retains the live specimen, saves once and does not repeat its failed creation");

            inspector.ThemeSessionCreationForTesting = null;
            inspector.UndoThemeForTesting();
            Check(publications == 1 && Size(inspector) == 31 && originalView.Id == 0,
                "undo first refreshes an unavailable preview without replaying a stale history action");
            inspector.UndoThemeForTesting();
            Check(publications == 2 && Size(inspector) == 29,
                "a style save remains undoable after its optional preview failed");
            inspector.RedoThemeForTesting();
            Check(publications == 3 && Size(inspector) == 31,
                "redo restores the style save whose preview originally failed");

            publications = 0; attempts = 0; originalView = inspector.ThemeView;
            inspector.ThemeSessionCreationForTesting = unavailable;
            Check(!inspector.EditPropertyForTesting(VIEM_STYLE_PROPERTY_CHARACTER_SIZE, 33)
                && publications == 1 && attempts == 1,
                "the concurrent-editor fixture publishes once before its preview becomes unavailable");

            other = new StyleWindow(view, preferences, followCaret: false);
            other.StylePicker.SelectedItem = other.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == "Status bar");
            Check(Size(other) == 33, "a preview failure preserves the already-saved style edit");
            Check(other.EditPropertyForTesting(VIEM_STYLE_PROPERTY_CHARACTER_SIZE, 37) && publications == 2,
                "another inspector can publish while the first preview is unavailable");
            byte[][] current = formats.Select(preferences.ThemeStyleDefaults).ToArray();
            Check(!inspector.EditPropertyForTesting(VIEM_STYLE_PROPERTY_CHARACTER_SIZE, 41)
                && publications == 2
                && formats.Select((format, index) => current[index].AsSpan().SequenceEqual(preferences.ThemeStyleDefaults(format))).All(same => same),
                "unavailable stale controls cannot overwrite newer styles in any family");

            inspector.ThemeSessionCreationForTesting = null;
            Check(!inspector.EditPropertyForTesting(VIEM_STYLE_PROPERTY_CHARACTER_SIZE, 41)
                && publications == 2 && Size(inspector) == 37 && originalView.Id == 0,
                "recovery refreshes current controls and discards the edit formed from the old specimen");
            Check(inspector.EditPropertyForTesting(VIEM_STYLE_PROPERTY_CHARACTER_SIZE, 41)
                && publications == 3 && Size(other) == 41,
                "a new edit after preview recovery publishes once and updates other inspectors");
        }
        finally {
            preferences.ThemesChanged -= Published;
            inspector.ThemeSessionCreationForTesting = null;
            inspector.Close(); other?.Close();
            for (int index = 0; index < formats.Length; index++) preferences.SaveThemeStyles(formats[index], original[index]);
        }
        RetargetFailure(pane, preferences);
    }

    private static void RetargetFailure(EditorPane pane, Preferences preferences)
    {
        byte[] source = "Original target"u8.ToArray(), nextSource = "# New heading"u8.ToArray();
        using var document = new CoreDocument(source, format: VIEM_FORMAT_PLAIN_TEXT);
        using var nextDocument = new CoreDocument(nextSource, format: VIEM_FORMAT_MARKDOWN);
        preferences.AttachThemeDocument(document); preferences.AttachThemeDocument(nextDocument);
        using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        using var nextView = new CoreView(nextDocument, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        uint[] formats = [VIEM_FORMAT_PLAIN_TEXT, VIEM_FORMAT_MARKDOWN, VIEM_FORMAT_CODE];
        byte[][] styles = formats.Select(preferences.ThemeStyleDefaults).ToArray();
        var inspector = new StyleWindow(view, preferences);
        try {
            var specimen = inspector.ThemeView;
            var unavailable = new InvalidOperationException("Injected unavailable retarget preview");
            int attempts = 0;
            inspector.ThemeSessionCreationForTesting = () => { attempts++; throw unavailable; };
            bool rejected = false;
            try { inspector.FollowActiveView(nextView); }
            catch (InvalidOperationException error) when (ReferenceEquals(error, unavailable)) { rejected = true; }
            Check(rejected && attempts == 1 && ReferenceEquals(inspector.ThemeView, specimen) && specimen.Id != 0
                && specimen.Document.State.format == VIEM_FORMAT_PLAIN_TEXT,
                "failed cross-family retarget retains its current live specimen without retrying preview creation");
            view.Place(1, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            Check(inspector.CaretFollowScheduled,
                "failed cross-family retarget keeps the original source attachment and caret following");

            inspector.ThemeSessionCreationForTesting = null;
            inspector.FollowActiveView(nextView);
            Check(specimen.Id == 0 && inspector.ThemeView.Document.State.format == VIEM_FORMAT_MARKDOWN
                && inspector.StylePicker.SelectedItem is StyleDefinition { Id: "Heading1" } && !inspector.CaretFollowScheduled,
                "a later active-view retry completes the intended family and caret-style retarget");
            view.Place(2, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            Check(!inspector.CaretFollowScheduled,
                "successful retarget releases the previous source subscription");
            nextView.Place(1, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, nextDocument.State.document_revision);
            Check(inspector.CaretFollowScheduled,
                "successful retarget follows caret changes in the intended new source");
            Check(source.AsSpan().SequenceEqual(document.Source(document.State.document_revision))
                && nextSource.AsSpan().SequenceEqual(nextDocument.Source(nextDocument.State.document_revision))
                && !document.IsDirty && !nextDocument.IsDirty
                && formats.Select((format, index) => styles[index].AsSpan().SequenceEqual(preferences.ThemeStyleDefaults(format))).All(same => same),
                "failed and retried preview retargets preserve source and all saved theme families");
        }
        finally { inspector.ThemeSessionCreationForTesting = null; inspector.Close(); }
    }
}
#endif

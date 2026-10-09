#if DEBUG
using System.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class LinkInteractionTests
{
    private static void Check(bool condition, string name)
    { if (!condition) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }

    internal static async Task Run(Preferences preferences)
    {
        ResolveDestinations();
        const string source = "[alpha](https://example.test/a%20b)\n\n# Destination\n\nplain";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        var pane = window.ActivePane!; var view = await pane.Ready;
        string Source() => Encoding.UTF8.GetString(document.Source(document.State.document_revision));
        try
        {
            await Task.Delay(100);
            view.Place(1, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            Check(window.Toolbar.InsertLink.IsEnabled && window.Toolbar.InsertLink.IsChecked == true
                && AutomationProperties.GetName(window.Toolbar.InsertLink) == "Link"
                && ToolTipService.GetToolTip(window.Toolbar.InsertLink) as string == "Link",
                "Markdown has an accessible link toolbar action reflecting the Normal caret");
            Check(pane.LinkPopupVisible && !pane.LinkEditorVisible, "the caret reveals the compact link toolbar");
            window.Toolbar.InsertLink.Focus(FocusState.Programmatic); pane.Refresh();
            Check(!pane.LinkPopupVisible, "background refresh does not reopen link controls while another control has focus");
            pane.FocusEditor(); pane.Refresh();
            Check(pane.LinkPopupVisible, "returning editor focus restores the passive link toolbar");
            pane.DismissLinkPopup(); pane.Refresh();
            Check(!pane.LinkPopupVisible, "dismissing a link toolbar persists at the same caret");
            view.Place(2, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            view.Place(1, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            Check(pane.LinkPopupVisible, "moving away and returning reveals a dismissed link toolbar");
            pane.ShowInsertLink();
            Check(pane.LinkEditorVisible && pane.LinkTextValue == "alpha" && pane.LinkDestinationValue == "https://example.test/a%20b",
                "link editing uses separate native text and destination fields");
            pane.LinkDestinationValue = "#destination"; pane.ApplyLinkEditor();
            Check(Source().StartsWith("[alpha](<#destination>)", StringComparison.Ordinal), "link editing publishes the destination through the core transaction");
            view.Undo(); Check(Source() == source, "link editing undo restores exact source bytes");

            view.SetMarkdownSource(true);
            foreach (ulong offset in new ulong[] { 0, 6, 7, 12, 32 })
            {
                view.Place(offset, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
                Check(view.LinkContext().Link?.Destination == "https://example.test/a%20b" && pane.LinkPopupVisible
                    && window.Toolbar.InsertLink.IsChecked == true,
                    "Source link punctuation and destination activate the popup at offset " + offset);
            }
            view.Place(7, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            view.Place(12, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision, true);
            Check(window.Toolbar.InsertLink.IsChecked == true && !window.Toolbar.InsertLink.IsEnabled,
                "a Source destination selection retains link state and disables unsupported treatment changes");
            ulong sourceRevision = document.State.document_revision;
            uint sourceSelectionMode = view.Presentation.mode;
            window.Toolbar.ExecuteLink();
            Check(!pane.LinkEditorVisible && view.Presentation.mode == sourceSelectionMode && document.State.document_revision == sourceRevision
                && Source() == source, "an unavailable active-link action preserves selection and never opens an insertion form");
            view.Key(VIEM_KEY_ESCAPE);
            view.SetMarkdownSource(false);
            view.Place(1, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            var previous = view.LinkContext();
            view.Place(2, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            bool rejected = false;
            try { view.EditLink(previous, "wrong", "https://example.test/stale"); }
            catch (InvalidOperationException) { rejected = true; }
            Check(rejected && Source() == source, "a stale popup cannot edit a later caret selection");
            var current = view.LinkContext(); view.EditLink(current, "", "", remove: true);
            Check(Source().StartsWith("alpha\n\n", StringComparison.Ordinal), "removing a link retains its label");
            view.Undo(); Check(Source() == source, "unlink undo restores exact source spelling");

            view.Key(VIEM_KEY_ESCAPE); view.Place(2, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            ulong revision = document.State.document_revision;
            Check(window.Toolbar.InsertLink.Focus(FocusState.Programmatic), "the link toggle accepts native keyboard focus");
            await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space); await Task.Delay(40);
            Check(view.Presentation.mode == VIEM_MODE_INSERT && window.Toolbar.InsertLink.IsChecked == false
                && document.State.document_revision == revision && !pane.LinkEditorVisible,
                "Normal link toggling enters Insert with pending unlinked text without editing source");
            view.Text("X"); view.Key(VIEM_KEY_ESCAPE);
            Check(document.FormattedText().StartsWith("alXpha", StringComparison.Ordinal)
                && Source().Contains("X[", StringComparison.Ordinal), "link toggling splits the source link around newly typed plain text");
            view.Undo(); Check(Source() == source, "link split and inserted text share exact undo");

            view.Place(0, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            view.Place(5, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision, true);
            Check(window.Toolbar.InsertLink.IsChecked == true && window.Toolbar.InsertLink.IsEnabled,
                "a single selected link enables removal through the active toolbar button");
            Check(window.Toolbar.InsertLink.Focus(FocusState.Programmatic), "selected-link removal accepts native keyboard focus");
            await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space); await Task.Delay(40);
            Check(Source().StartsWith("alpha\n\n", StringComparison.Ordinal) && !pane.LinkEditorVisible,
                "clicking the selected-link button removes its treatment and retains its label");
            view.Undo(); Check(Source() == source, "selected-link toolbar removal restores original bytes on undo");
            view.Key(VIEM_KEY_ESCAPE); view.Place(2, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            view.Place(8, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision, true);
            Check(!window.Toolbar.InsertLink.IsEnabled, "complex cross-paragraph link selections disable unsupported toolbar actions");
            view.Key(VIEM_KEY_ESCAPE);

            view.Command("G0viw");
            pane.ShowInsertLink();
            Check(pane.LinkEditorVisible && pane.LinkTextValue == "plain", "selected text seeds the insert-link editor");
            pane.LinkDestinationValue = "other.md#part"; pane.ApplyLinkEditor();
            Check(Source().EndsWith("[plain](<other.md#part>)", StringComparison.Ordinal), "selected text becomes a local-document link");
            view.Undo();
            view.Key(VIEM_KEY_ESCAPE); view.Command("G0"); pane.ShowInsertLink();
            pane.LinkTextValue = ""; pane.LinkDestinationValue = "https://example.test"; pane.ApplyLinkEditor();
            Check(document.FormattedText().Contains("https://example.test", StringComparison.Ordinal), "a destination alone supplies the inserted link text");
            view.Undo();
            view.Key(VIEM_KEY_ESCAPE); view.Command("gg0"); pane.ShowInsertLink();
            view.Command("G0");
            Check(!pane.LinkEditorVisible, "moving the caret dismisses a stale link edit without applying it");
            view.GoToLinkFragment("destination");
            Check(view.Presentation.cursor_utf8_offset < (ulong)document.FormattedText().IndexOf("plain", StringComparison.Ordinal),
                "an internal fragment navigates within the same document");
            Check(pane.LastError == null, "link popup tests complete without presentation errors");
            string linkedPath = Path.Combine(Path.GetTempPath(), "viem-linked-" + Guid.NewGuid().ToString("N") + ".md");
            await File.WriteAllTextAsync(linkedPath, "# Local target");
            try
            {
                var existing = App.Instance.Windows.ToHashSet();
                await window.OpenLinkedDocument(pane, new(LinkDestinationKind.Document, linkedPath));
                var opened = App.Instance.Windows.Single(w => !existing.Contains(w));
                var linkedPane = opened.ActivePane!;
                try
                {
                    await linkedPane.Ready;
                    Check(FileIdentity.Same(linkedPane.Document.FilePath!, linkedPath) && window.Panes.Contains(pane),
                        "local document links open a new Viem window without replacing the source document");
                    var linkedView = await linkedPane.Ready;
                    linkedView.Command("G$a"); linkedView.Text(" unsaved"); linkedView.Key(VIEM_KEY_ESCAPE);
                    byte[] unsavedSource = linkedPane.Document.Source(linkedPane.Document.State.document_revision);
                    File.Delete(linkedPath);
                    var beforeReuse = App.Instance.Windows.ToHashSet();
                    window.Activate(); window.FocusPane(pane);
                    await window.OpenLinkedDocument(pane, new(LinkDestinationKind.Document, linkedPath, "local-target-unsaved"));
                    Check(beforeReuse.SetEquals(App.Instance.Windows) && opened.ActivePane == linkedPane,
                        "links focus the existing document pane without creating another window");
                    Check(linkedPane.Document.IsDirty && unsavedSource.AsSpan().SequenceEqual(linkedPane.Document.Source(linkedPane.Document.State.document_revision))
                        && linkedView.Presentation.cursor_utf8_offset == 0,
                        "existing document links preserve unsaved source and navigate fragments after the backing file is removed");
                    await window.OpenLinkedDocument(pane, new(LinkDestinationKind.Document, linkedPath, "missing-heading"));
                    Check(linkedPane.LastError?.Message.Contains("heading", StringComparison.OrdinalIgnoreCase) == true && pane.LastError == null,
                        "a missing local-document fragment reports on the existing destination window");
                }
                finally { await opened.ClosePane(linkedPane, force: true); App.Instance.Windows.Remove(opened); }
            }
            finally { File.Delete(linkedPath); }
        }
        finally { await window.ClosePane(pane, force: true); App.Instance.Windows.Remove(window); }
    }

    private static void ResolveDestinations()
    {
        string basePath = Path.Combine(Path.GetTempPath(), "viem-link-policy", "source.md");
        var local = LinkDestination.Resolve("child%20file.md#part%20two", basePath);
        Check(local.Kind == LinkDestinationKind.Document && local.Value == Path.Combine(Path.GetDirectoryName(basePath)!, "child file.md")
            && local.Fragment == "part two", "relative document links use the document directory and decode once");
        var web = LinkDestination.Resolve("https://example.test/a%20b?x=1#part", null);
        Check(web.Kind == LinkDestinationKind.Web && web.Value == "https://example.test/a%20b?x=1#part", "web links preserve existing percent escapes");
        Check(LinkDestination.Resolve("#heading", null).Kind == LinkDestinationKind.Fragment, "internal links work before saving a document");
        foreach (string value in new[] { "javascript:alert(1)", "data:text/html,test", "https://example.test/%0a", "file:///C:/test%00.md" })
        {
            bool rejected = false; try { LinkDestination.Resolve(value, basePath); } catch (InvalidOperationException) { rejected = true; }
            Check(rejected, "link actions reject executable schemes and control characters: " + value);
        }
        bool unsaved = false; try { LinkDestination.Resolve("relative.md", null); } catch (InvalidOperationException) { unsaved = true; }
        Check(unsaved, "relative document navigation requires a saved base path");
    }
}
#endif

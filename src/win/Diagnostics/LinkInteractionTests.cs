#if DEBUG
using System.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class LinkInteractionTests
{
    private static void Check(bool condition, string name, string? details = null)
    {
        if (!condition) throw new InvalidOperationException(details == null ? name : $"{name} ({details})");
        FrontendSmokeTests.UiChecks.Add(name);
    }

    private static IEnumerable<T> Children<T>(DependencyObject root) where T : DependencyObject
    {
        if (root is T match) yield return match;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(root); i++)
            foreach (var child in Children<T>(VisualTreeHelper.GetChild(root, i))) yield return child;
    }

    private const string source = "[alpha](https://example.test/a%20b)\n\n# Destination\n\nplain";

    internal static SelfTestRunner.Stage Group(Preferences preferences) =>
        TestEditor.Group("LinkInteraction", preferences, source, [
            new("ResolveDestinations", _ => {
                ResolveDestinations();
                return Task.CompletedTask;
            }, UsesEditor: false),
            new("CaretToolbar", CaretToolbar),
            new("SummarySizing", SummarySizing),
            new("PopupFocusAndDismissal", PopupFocusAndDismissal),
            new("EditDestination", EditDestination),
            new("StaleContext", StaleContext),
            new("RemoveLink", RemoveLink),
            new("TogglePendingLink", TogglePendingLink),
            new("RemoveSelectedLink", RemoveSelectedLink),
            new("RejectCrossParagraphSelection", RejectCrossParagraphSelection),
            new("LinkSelectedText", LinkSelectedText),
            new("NativeDraftTyping", NativeDraftTyping),
            new("HeadingDropdown", HeadingDropdown),
            new("DestinationSuppliesLabel", DestinationSuppliesLabel),
            new("CaretMovementAndFragment", CaretMovementAndFragment),
            new("LocalDocumentNavigation", LocalDocumentNavigation),
        ], resetCase: editor => editor.RestoreCaret(source));

    private static Task CaretToolbar(TestEditor editor)
    {
        var window = editor.Window;
        var pane = editor.Pane;
        var view = editor.View;
        var document = editor.Document;

        view.Place(1, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
        Check(window.Toolbar.InsertLink.IsEnabled && window.Toolbar.InsertLink.IsChecked == true
            && AutomationProperties.GetName(window.Toolbar.InsertLink) == "Link"
            && ToolTipService.GetToolTip(window.Toolbar.InsertLink) as string == "Link (Ctrl+Shift+K)",
            "Markdown has an accessible link toolbar action reflecting the Normal caret");
        Check(pane.LinkPopupVisible && !pane.LinkEditorVisible, "the caret reveals the compact link toolbar");
        Check(pane.LinkSummary == "alpha" && pane.LinkPopupWidth < 360, "the compact toolbar fits its link label within the previous maximum width");
        return Task.CompletedTask;
    }

    private static Task SummarySizing(TestEditor editor)
    {
        var pane = editor.Pane;
        var view = editor.View;

        editor.View.Place(1, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, editor.Document.State.document_revision);
        foreach (string summary in new[] { "alpha", "https://example.test/a%20b", new string('W', 120), "alpha" })
        {
            bool edited = summary != "alpha";
            if (edited) view.EditLink(view.LinkContext(), summary, "https://example.test/a%20b");
            pane.Refresh();
            pane.UpdateLayout();
            var button = Children<Button>(pane).Single(button => AutomationProperties.GetName(button) == "Open https://example.test/a%20b");
            var label = (TextBlock)button.Content;
            bool capped = summary.Length == 120;
            Check(pane.LinkPopupVisible && label.IsTextTrimmed == capped
                && (capped ? pane.LinkPopupWidth == 360 : pane.LinkPopupWidth < 360),
                "link summaries trim only at the popup maximum, including after a longer label",
                $"summary={summary}, popup={pane.LinkPopupWidth}, button={button.ActualWidth}, label={label.ActualWidth}, trimmed={label.IsTextTrimmed}");
            if (edited) view.Undo();
        }
        Check(editor.Source == source, "link summary sizing leaves the original source intact after undo");
        return Task.CompletedTask;
    }

    private static Task PopupFocusAndDismissal(TestEditor editor)
    {
        var window = editor.Window;
        var pane = editor.Pane;
        var view = editor.View;
        var document = editor.Document;

        editor.View.Place(1, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, editor.Document.State.document_revision);
        window.Toolbar.InsertLink.Focus(FocusState.Programmatic);
        pane.Refresh();
        Check(!pane.LinkPopupVisible, "background refresh does not reopen link controls while another control has focus");
        pane.FocusEditor();
        pane.Refresh();
        Check(pane.LinkPopupVisible, "returning editor focus restores the passive link toolbar");
        pane.DismissLinkPopup();
        pane.Refresh();
        Check(!pane.LinkPopupVisible, "dismissing a link toolbar persists at the same caret");
        view.Place(2, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
        view.Place(1, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
        Check(pane.LinkPopupVisible, "moving away and returning reveals a dismissed link toolbar");
        return Task.CompletedTask;
    }

    private static Task EditDestination(TestEditor editor)
    {
        var pane = editor.Pane;
        var view = editor.View;

        editor.View.Place(1, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, editor.Document.State.document_revision);
        pane.ShowInsertLink();
        Check(pane.LinkEditorVisible && pane.LinkTextValue == "alpha" && pane.LinkDestinationValue == "https://example.test/a%20b",
            "link editing uses separate native text and destination fields");
        pane.LinkDestinationValue = "#destination";
        pane.ApplyLinkEditor();
        Check(editor.Source.StartsWith("[alpha](<#destination>)", StringComparison.Ordinal), "link editing publishes the destination through the core transaction");
        view.Undo();
        Check(editor.Source == source, "link editing undo restores exact source bytes");
        return Task.CompletedTask;
    }

    private static Task StaleContext(TestEditor editor)
    {
        var view = editor.View;
        var document = editor.Document;

        view.Place(1, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
        var previous = view.LinkContext();
        view.Place(2, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
        bool rejected = false;
        try {
            view.EditLink(previous, "wrong", "https://example.test/stale");
        }
        catch (InvalidOperationException) {
            rejected = true;
        }
        Check(rejected && editor.Source == source, "a stale popup cannot edit a later caret selection");
        return Task.CompletedTask;
    }

    private static Task RemoveLink(TestEditor editor)
    {
        var view = editor.View;

        editor.View.Place(2, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, editor.Document.State.document_revision);
        var current = view.LinkContext();
        view.EditLink(current, "", "", remove: true);
        Check(editor.Source.StartsWith("alpha\n\n", StringComparison.Ordinal), "removing a link retains its label");
        view.Undo();
        Check(editor.Source == source, "unlink undo restores exact source spelling");
        return Task.CompletedTask;
    }

    private static async Task TogglePendingLink(TestEditor editor)
    {
        var window = editor.Window;
        var pane = editor.Pane;
        var view = editor.View;
        var document = editor.Document;

        view.Key(VIEM_KEY_ESCAPE);
        view.Place(2, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
        ulong revision = document.State.document_revision;
        Check(window.Toolbar.InsertLink.Focus(FocusState.Programmatic), "the link toggle accepts native keyboard focus");
        await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
        await InputRoutingTests.Settle();
        Check(view.Presentation.mode == VIEM_MODE_INSERT && window.Toolbar.InsertLink.IsChecked == false
            && document.State.document_revision == revision && !pane.LinkEditorVisible,
            "Normal link toggling enters Insert with pending unlinked text without editing source");
        view.Text("X");
        view.Key(VIEM_KEY_ESCAPE);
        Check(document.FormattedText().StartsWith("alXpha", StringComparison.Ordinal)
            && editor.Source.Contains("X[", StringComparison.Ordinal), "link toggling splits the source link around newly typed plain text");
        view.Undo();
        Check(editor.Source == source, "link split and inserted text share exact undo");
    }

    private static async Task RemoveSelectedLink(TestEditor editor)
    {
        var window = editor.Window;
        var pane = editor.Pane;
        var view = editor.View;
        var document = editor.Document;

        view.Place(0, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
        view.Place(5, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision, true);
        Check(window.Toolbar.InsertLink.IsChecked == true && window.Toolbar.InsertLink.IsEnabled,
            "a single selected link enables removal through the active toolbar button");
        Check(window.Toolbar.InsertLink.Focus(FocusState.Programmatic), "selected-link removal accepts native keyboard focus");
        await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
        await InputRoutingTests.Settle();
        Check(editor.Source.StartsWith("alpha\n\n", StringComparison.Ordinal) && !pane.LinkEditorVisible,
            "clicking the selected-link button removes its treatment and retains its label");
        view.Undo();
        Check(editor.Source == source, "selected-link toolbar removal restores original bytes on undo");
    }

    private static Task RejectCrossParagraphSelection(TestEditor editor)
    {
        var window = editor.Window;
        var view = editor.View;
        var document = editor.Document;

        view.Key(VIEM_KEY_ESCAPE);
        view.Place(2, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
        view.Place(8, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision, true);
        Check(!window.Toolbar.InsertLink.IsEnabled, "complex cross-paragraph link selections disable unsupported toolbar actions");
        view.Key(VIEM_KEY_ESCAPE);
        return Task.CompletedTask;
    }

    private static Task LinkSelectedText(TestEditor editor)
    {
        var pane = editor.Pane;
        var view = editor.View;
        var document = editor.Document;

        view.Command("G0viw");
        pane.ShowInsertLink();
        Check(pane.LinkEditorVisible && pane.LinkTextValue == "plain", "selected text seeds the insert-link editor");
        pane.LinkDestinationValue = "other.md#part";
        pane.ApplyLinkEditor();
        Check(editor.Source.EndsWith("[plain](<other.md#part>)", StringComparison.Ordinal), "selected text becomes a local-document link");
        view.Undo();
        return Task.CompletedTask;
    }

    private static async Task NativeDraftTyping(TestEditor editor)
    {
        var window = editor.Window;
        var pane = editor.Pane;
        var view = editor.View;

        view.Key(VIEM_KEY_ESCAPE);
        view.Command("G0");
        Check(window.Toolbar.InsertLink.Focus(FocusState.Programmatic), "empty link insertion accepts native keyboard focus");
        await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
        Check(pane.LinkEditorVisible && pane.LinkTextValue == "" && pane.LinkDestinationValue == "",
            $"a caret without a link or selection starts an empty link draft after native Space activation (visible={pane.LinkEditorVisible}, text='{pane.LinkTextValue}', location='{pane.LinkDestinationValue}', error={pane.LastError})");
        await InputRoutingTests.Text(" leading");
        Check(pane.LinkTextValue == " leading",
            $"link keyboard activation preserves subsequent native typing including a genuine leading space "
            + $"(visible={pane.LinkEditorVisible}, text='{pane.LinkTextValue}', location='{pane.LinkDestinationValue}', "
            + $"focus={FocusManager.GetFocusedElement(pane.XamlRoot)?.GetType().Name}, "
            + $"focusName={(FocusManager.GetFocusedElement(pane.XamlRoot) is DependencyObject focused ? AutomationProperties.GetName(focused) : "none")}, "
            + $"foregroundOwnsWindow={InputRoutingTests.ForegroundWindowForTesting == window.Hwnd}, "
            + $"transition={pane.PopupFocusDiagnosticForTesting}, error={pane.LastError})");
        pane.LinkTextValue = "";
        var destinationControl = pane.LinkDestinationControl!;
        await InputRoutingTests.Key(global::Windows.System.VirtualKey.Tab);
        Check(Children<DependencyObject>(destinationControl).Any(element => ReferenceEquals(element, FocusManager.GetFocusedElement(pane.XamlRoot))),
            "explicit native Tab still transfers focus into the editable link destination");
        Check(destinationControl.IsEditable, "the destination field is an editable native combobox");
    }

    private static async Task HeadingDropdown(TestEditor editor)
    {
        var pane = editor.Pane;
        var view = editor.View;
        var document = editor.Document;

        view.Command("G0");
        pane.ShowInsertLink();
        var destinationControl = pane.LinkDestinationControl!;
        var headings = view.LinkHeadings(view.LinkContext()).Headings;
        Check(headings.Count == 1 && headings[0] is { Text: "Destination", Destination: "#destination" },
            "the link draft exports the current document heading before native dropdown expansion");
        bool dropdownOpened = false;
        destinationControl.DropDownOpened += (_, _) => dropdownOpened = true;
        destinationControl.IsDropDownOpen = true;
        Check(await SelfTestRunner.Until(() => destinationControl.IsDropDownOpen
            && destinationControl.ContainerFromIndex(0) is ComboBoxItem),
            "the native heading dropdown realizes its first row");
        int sourceCount = destinationControl.ItemsSource is System.Collections.IEnumerable headingSource
            ? headingSource.Cast<object>().Count() : -1;
        Check(destinationControl.Items.Count == 1 && destinationControl.Items[0] is LinkHeading { Text: "Destination", Destination: "#destination" },
            $"expanding the destination combobox lists the document headings (opened event: {dropdownOpened}, "
                + $"open: {destinationControl.IsDropDownOpen}, items: {destinationControl.Items.Count}, editor: {pane.LinkEditorVisible}; "
                + $"source: {destinationControl.ItemsSource?.GetType().FullName}, source count: {sourceCount}, error: {pane.LinkEditorErrorForTesting}; "
                + $"visible labels: {string.Join(" | ", Children<TextBlock>(pane).Where(label => label.Visibility == Visibility.Visible).Select(label => label.Text))})");
        Check(destinationControl.DisplayMemberPath.Length == 0 && destinationControl.ItemTemplate != null
            && destinationControl.Items[0].ToString() == "#destination",
            "heading rows display their labels while native editable text uses the destination value");
        destinationControl.UpdateLayout();
        var headingRow = destinationControl.ContainerFromIndex(0) as ComboBoxItem;
        Check(headingRow != null && Children<TextBlock>(headingRow).Any(text => text.Text == "Destination"),
            "the native heading dropdown renders its bound label in managed and NativeAOT builds");
        destinationControl.SelectedIndex = 0;
        destinationControl.IsDropDownOpen = false;
        await InputRoutingTests.Settle();
        Check(pane.LinkDestinationValue == "#destination", "choosing a heading authors its exact internal fragment");
        pane.DismissLinkPopup(suppress: false);
        Check(!destinationControl.IsDropDownOpen && destinationControl.SelectedItem == null && destinationControl.Items.Count == 0,
            "dismissing a link draft closes and clears its heading dropdown");
        pane.ShowInsertLink();
        Check(pane.LinkEditorVisible && pane.LinkDestinationValue == "" && destinationControl.SelectedItem == null,
            "a new link draft starts without the previous heading selection");
        destinationControl.IsDropDownOpen = true;
        Check(await SelfTestRunner.Until(() => destinationControl.IsDropDownOpen
            && destinationControl.ContainerFromIndex(0) is ComboBoxItem),
            "the native heading dropdown realizes its first row");
        destinationControl.SelectedIndex = 0;
        destinationControl.IsDropDownOpen = false;
        await InputRoutingTests.Settle();
        Check(pane.LinkDestinationValue == "#destination", "the same heading can be selected again after cancelling a draft");
    }

    private static Task DestinationSuppliesLabel(TestEditor editor)
    {
        var pane = editor.Pane;
        var view = editor.View;
        var document = editor.Document;

        view.Command("G0");
        pane.ShowInsertLink();
        pane.LinkTextValue = "";
        pane.LinkDestinationValue = "https://example.test";
        pane.ApplyLinkEditor();
        Check(document.FormattedText().Contains("https://example.test", StringComparison.Ordinal), "a destination alone supplies the inserted link text");
        view.Undo();
        return Task.CompletedTask;
    }

    private static Task CaretMovementAndFragment(TestEditor editor)
    {
        var pane = editor.Pane;
        var view = editor.View;
        var document = editor.Document;

        view.Key(VIEM_KEY_ESCAPE);
        view.Command("gg0");
        pane.ShowInsertLink();
        view.Command("G0");
        Check(!pane.LinkEditorVisible, "moving the caret dismisses a stale link edit without applying it");
        view.GoToLinkFragment("destination");
        Check(view.Presentation.cursor_utf8_offset < (ulong)document.FormattedText().IndexOf("plain", StringComparison.Ordinal),
            "an internal fragment navigates within the same document");
        Check(pane.LastError == null, "link popup tests complete without presentation errors");
        return Task.CompletedTask;
    }

    private static async Task LocalDocumentNavigation(TestEditor editor)
    {
        var window = editor.Window;
        var pane = editor.Pane;
        var document = editor.Document;

        string linkedPath = Path.Combine(Path.GetTempPath(), "viem-linked-" + Guid.NewGuid().ToString("N") + ".md");
        await File.WriteAllTextAsync(linkedPath, "# Local target\n\n"
            + string.Join("\n\n", Enumerable.Range(1, 80).Select(line => "**line" + line + "**")));
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
                linkedView.Command("gg$a");
                linkedView.Text(" unsaved");
                linkedView.Key(VIEM_KEY_ESCAPE);
                byte[] unsavedSource = linkedPane.Document.Source(linkedPane.Document.State.document_revision);
                File.Delete(linkedPath);
                var beforeReuse = App.Instance.Windows.ToHashSet();
                window.Activate();
                window.FocusPane(pane);
                await window.OpenLinkedDocument(pane, new(LinkDestinationKind.Document, linkedPath, "local-target-unsaved"));
                Check(beforeReuse.SetEquals(App.Instance.Windows) && opened.ActivePane == linkedPane,
                    "links focus the existing document pane without creating another window");
                Check(linkedPane.Document.IsDirty && unsavedSource.AsSpan().SequenceEqual(linkedPane.Document.Source(linkedPane.Document.State.document_revision))
                    && linkedView.Presentation.cursor_utf8_offset == 0,
                    "existing document links preserve unsaved source and navigate fragments after the backing file is removed",
                    $"dirty={linkedPane.Document.IsDirty}, same={unsavedSource.AsSpan().SequenceEqual(linkedPane.Document.Source(linkedPane.Document.State.document_revision))}, cursor={linkedView.Presentation.cursor_utf8_offset}, format={linkedPane.Document.State.format}, error={linkedPane.LastError?.Message}");
                linkedView.Padding(17, 10, 29, 10);
                foreach (bool sourceView in new[] { true, false })
                {
                    linkedView.SetMarkdownPreview(!sourceView);
                    await window.OpenLinkedDocument(pane, new(LinkDestinationKind.Document, linkedPath, "L83"));
                    string targetText = sourceView ? "**line41**" : "line41";
                    ulong offset = (ulong)linkedPane.Document.FormattedText().IndexOf(targetText, StringComparison.Ordinal);
                    Check(linkedView.Presentation.cursor_utf8_offset == offset && beforeReuse.SetEquals(App.Instance.Windows),
                        "a physical source-line link focuses the unsaved existing pane in " + (sourceView ? "Code" : "Markdown Preview"));
                    var layout = linkedView.Layout();
                    var row = layout.Rows.Single(row => row.text_start == offset);
                    float middle = row.y + (row.ascent + row.descent + row.leading) / 2;
                    Check(Math.Abs(middle - linkedView.Viewport.top - (layout.Info.viewport_height + 17 - 29) / 2) < 1,
                        "source-line links center the target row between the configured margins");
                    Check(unsavedSource.AsSpan().SequenceEqual(linkedPane.Document.Source(linkedPane.Document.State.document_revision)),
                        "source-line navigation leaves unsaved bytes unchanged");
                }
                ulong beforeMissingLine = linkedView.Presentation.cursor_utf8_offset;
                float beforeMissingTop = linkedView.Viewport.top;
                await window.OpenLinkedDocument(pane, new(LinkDestinationKind.Document, linkedPath, "L999"));
                Check(linkedView.Presentation.cursor_utf8_offset == beforeMissingLine && linkedView.Viewport.top == beforeMissingTop,
                    "a missing source-line fragment leaves the destination caret and viewport unchanged");
                await window.OpenLinkedDocument(pane, new(LinkDestinationKind.Document, linkedPath, "missing-heading"));
                Check(linkedPane.LastError?.Message.Contains("heading", StringComparison.OrdinalIgnoreCase) == true && pane.LastError == null,
                    "a missing local-document fragment reports on the existing destination window");
            }
            finally {
                await opened.ClosePane(linkedPane, force: true);
                App.Instance.Windows.Remove(opened);
            }
        }
        finally {
            File.Delete(linkedPath);
        }
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
            bool rejected = false;
            try {
                LinkDestination.Resolve(value, basePath);
            } catch (InvalidOperationException) {
                rejected = true;
            }
            Check(rejected, "link actions reject executable schemes and control characters: " + value);
        }
        bool unsaved = false;
        try {
            LinkDestination.Resolve("relative.md", null);
        } catch (InvalidOperationException) {
            unsaved = true;
        }
        Check(unsaved, "relative document navigation requires a saved base path");
    }
}
#endif

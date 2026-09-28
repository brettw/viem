#if DEBUG
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class DocumentModeMenuTests
{
    private static void Check(bool value, string name)
    { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }

    internal static async Task Run(Preferences preferences)
    {
        byte[] source = "# Heading\r\n\r\n**words**\r\n"u8.ToArray();
        var document = new CoreDocument(source, "notes.rs");
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        try
        {
            var pane = window.ActivePane!; var view = await pane.Ready;
            var menu = window.Menu.Items.Single(m => m.Title == "View");
            var peer = new MenuBarItemAutomationPeer(menu);
            var plain = (ToggleMenuFlyoutItem)menu.Items[0];
            var markdown = (ToggleMenuFlyoutItem)menu.Items[1];
            var code = (MenuFlyoutSubItem)menu.Items[2];
            var auto = (ToggleMenuFlyoutItem)code.Items[0];
            var python = code.Items.OfType<ToggleMenuFlyoutItem>().Single(i => i.Text == "Python");
            async Task Open() { peer.Expand(); await Task.Delay(40); }
            async Task Choose(ToggleMenuFlyoutItem item)
            {
                await Open(); Check(item.Focus(FocusState.Programmatic), "view-mode choice takes native keyboard focus");
                await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space); await Task.Delay(40);
                peer.Collapse(); await Open();
            }
            await Open();
            Check(plain.Text == "Plain text" && markdown.Text == "Markdown" && code.Text == "Code"
                && menu.Items[3] is MenuFlyoutSeparator && code.Items[1] is MenuFlyoutSeparator,
                "View starts with the three mode choices and separated Code Auto entry");
            Check(code.Items.Skip(2).Cast<ToggleMenuFlyoutItem>().Select(i => i.Text).SequenceEqual(DocumentModes.Languages.Select(l => l.Name))
                && DocumentModes.Languages.Count > 600, "Code contains the complete shared sorted language catalogue");
            Check(!plain.IsChecked && !markdown.IsChecked && auto.IsChecked && auto.Text == "Auto (Rust)"
                && ((FontIcon)code.Icon).Glyph == "\uE73E", "initial Code Auto checks and Rust detection label");
            await Choose(plain);
            Check(document.State.format == VIEM_FORMAT_PLAIN_TEXT && plain.IsChecked && !auto.IsChecked
                && ((FontIcon)code.Icon).Glyph == "", "Plain text menu action overrides a code filename");
            peer.Collapse();
            view.SetDocumentMode(VIEM_DOCUMENT_MODE_CODE, "python", false, DocumentModes.Read(document));
            await Open();
            Check(python.IsChecked && !plain.IsChecked && !auto.IsChecked && auto.Text == "Auto (Rust)",
                "forced Python keeps the independently detected Rust label");
            await Choose(markdown);
            Check(markdown.IsChecked && document.State.format == (preferences.MarkdownFormattedView ? VIEM_FORMAT_MARKDOWN : VIEM_FORMAT_MARKDOWN_SOURCE),
                "Markdown choice uses the remembered formatted view");
            Check(!document.IsDirty && source.AsSpan().SequenceEqual(document.Source(document.State.document_revision)),
                "mode changes retain exact source bytes and clean state");
            peer.Collapse();

            foreach (var (filename, expectedFormat, label) in new[] {
                ("notes.md", VIEM_FORMAT_CODE, "Auto (Markdown)"),
                ("notes.unknown", VIEM_FORMAT_PLAIN_TEXT, "Auto (Plain Text)") })
            {
                var next = window.AddPane(new CoreDocument(source, filename)); var nextView = await next.Ready;
                nextView.SetDocumentMode(VIEM_DOCUMENT_MODE_AUTO, "", true, DocumentModes.Read(next.Document));
                await Open();
                Check(next.Document.State.format == expectedFormat && auto.Text == label && auto.IsChecked
                    && plain.IsChecked == (expectedFormat == VIEM_FORMAT_PLAIN_TEXT) && !markdown.IsChecked,
                    "Auto uses Code Markdown highlighting or Plain text fallback: " + filename);
                Check(!python.IsChecked, "changing active document refreshes the forced-language check");
                peer.Collapse();
            }
            Check(window.Panes.All(p => p.LastError == null), "view-mode scenarios leave no presentation errors");
        }
        finally { App.Instance.Windows.Remove(window); window.Close(); }
    }
}
#endif

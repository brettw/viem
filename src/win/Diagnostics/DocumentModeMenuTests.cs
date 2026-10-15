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
            var menu = window.Menu.Items.Single(m => m.Title == "Format");
            var peer = new MenuBarItemAutomationPeer(menu);
            var file = window.Menu.Items.Single(m => m.Title == "File");
            var filePeer = new MenuBarItemAutomationPeer(file);
            var plain = (ToggleMenuFlyoutItem)menu.Items[0];
            var markdown = (ToggleMenuFlyoutItem)menu.Items[1];
            var codeMode = (ToggleMenuFlyoutItem)menu.Items[2];
            var code = file.Items.OfType<MenuFlyoutSubItem>().Single(i => i.Text == "Code language");
            var auto = (ToggleMenuFlyoutItem)code.Items[0];
            var obscure = (MenuFlyoutSubItem)code.Items[1];
            Check(code.Items.Count == 3 && obscure.Items.Count == 0,
                "startup retains Code Auto and the obscure flyout without constructing language menu controls");
            Task Open() => MenuTestHelpers.Transition(true, () => { peer.Expand(); return Task.CompletedTask; }, plain, codeMode);
            Task OpenFile() => MenuTestHelpers.Transition(true, () => { filePeer.Expand(); return Task.CompletedTask; }, code);
            Task Close(Func<Task>? action = null) => MenuTestHelpers.Transition(false, async () => {
                if (action != null) await action();
                peer.Collapse(); filePeer.Collapse();
            }, plain, codeMode, code, auto, obscure, obscure.Items.FirstOrDefault() ?? obscure);
            async Task OpenCode()
            {
                await OpenFile(); Check(code.Focus(FocusState.Programmatic), "Code language flyout takes native keyboard focus");
                await MenuTestHelpers.Transition(true, () => InputRoutingTests.Key(global::Windows.System.VirtualKey.Right), auto, obscure);
            }
            async Task OpenObscure()
            {
                await OpenCode(); Check(obscure.Focus(FocusState.Programmatic), "Obscure languages flyout takes native keyboard focus");
                await MenuTestHelpers.Transition(true, () => InputRoutingTests.Key(global::Windows.System.VirtualKey.Right), obscure.Items.First());
            }
            async Task Choose(ToggleMenuFlyoutItem item)
            {
                if (obscure.Items.Contains(item)) await OpenObscure();
                else if (code.Items.Contains(item)) await OpenCode();
                else await Open();
                Check(item.Focus(FocusState.Programmatic), "view-mode choice takes native keyboard focus");
                await Close(() => InputRoutingTests.Key(global::Windows.System.VirtualKey.Space));
                await Open();
            }
            await Open();
            Check(code.Items.Count == 3 && obscure.Items.Count == 0,
                "opening Format leaves both Code language groups deferred");
            await Close(); await OpenFile();
            var python = code.Items.OfType<ToggleMenuFlyoutItem>().Single(i => i.Text == "Python");
            var languageItems = code.Items.Skip(3).ToArray();
            Check(plain.Text == "Plain text" && markdown.Text == "Markdown" && codeMode.Text == "Code"
                && menu.Items[3] is MenuFlyoutSeparator && obscure.Text == "Obscure languages" && code.Items[2] is MenuFlyoutSeparator,
                "Format starts with document types and Code language places Obscure languages after Auto");
            int codeIndex = file.Items.IndexOf(code);
            Check(file.Items[codeIndex - 1] is MenuFlyoutSeparator
                && file.Items[codeIndex + 1] is MenuFlyoutSubItem { Text: "Text Encoding" },
                "File places Code language after its divider and immediately before Text Encoding");
            Check(code.Items.Skip(3).Cast<ToggleMenuFlyoutItem>().Select(i => i.Text)
                    .SequenceEqual(DocumentModes.Languages.Where(l => l.Primary).Select(l => l.Name))
                && code.Items.OfType<ToggleMenuFlyoutItem>().Any(i => i.Text == "Objective-C") && obscure.Items.Count == 0,
                "opening File constructs the shared primary languages including Objective-C and leaves obscure controls deferred");
            await Close(); await Open();
            Check(!plain.IsChecked && !markdown.IsChecked && auto.IsChecked && auto.Text == "Auto (Rust)"
                && codeMode.IsChecked && ((FontIcon)code.Icon).Glyph == "\uE73E", "initial Code Auto checks and Rust detection label");
            await Choose(plain);
            Check(document.State.format == VIEM_FORMAT_PLAIN_TEXT && plain.IsChecked && !auto.IsChecked
                && !codeMode.IsChecked && ((FontIcon)code.Icon).Glyph == "", "Plain text menu action overrides a code filename");
            await Close();
            view.SetDocumentMode(VIEM_DOCUMENT_MODE_CODE, "python", false, DocumentModes.Read(document));
            await Open();
            Check(python.IsChecked && !plain.IsChecked && !auto.IsChecked && auto.Text == "Auto (Rust)",
                "forced Python keeps the independently detected Rust label");
            await OpenObscure();
            var obscureItems = obscure.Items.Cast<ToggleMenuFlyoutItem>().ToArray();
            Check(obscureItems.Select(i => i.Text).SequenceEqual(DocumentModes.Languages.Where(l => !l.Primary).Select(l => l.Name))
                && obscureItems.Length + languageItems.Length == DocumentModes.Languages.Count && DocumentModes.Languages.Count > 600,
                "opening Code exposes every remaining shared language in the sorted obscure flyout");
            var obscureLanguage = DocumentModes.Languages.First(l => !l.Primary);
            var obscureChoice = obscureItems.Single(i => i.Text == obscureLanguage.Name);
            await Close();
            await Choose(obscureChoice);
            Check(obscureChoice.IsChecked && !python.IsChecked && !auto.IsChecked
                && DocumentModes.Read(document).Language == obscureLanguage.Id && ((FontIcon)obscure.Icon).Glyph == "\uE73E",
                "native obscure language selection updates its check and the document mode");
            await Close();
            await Choose(markdown);
            Check(markdown.IsChecked && document.State.format == VIEM_FORMAT_MARKDOWN,
                "Markdown Preview choice selects formatted Markdown");
            await Choose(codeMode);
            Check(codeMode.IsChecked && !markdown.IsChecked && document.State.format == VIEM_FORMAT_CODE
                && DocumentModes.Read(document).Language == obscureLanguage.Id && obscureChoice.IsChecked
                && ((FontIcon)code.Icon).Glyph == "\uE73E",
                "Code document type restores Code while retaining the chosen language");
            Check(!document.IsDirty && source.AsSpan().SequenceEqual(document.Source(document.State.document_revision)),
                "mode changes retain exact source bytes and clean state");
            await Close();

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
                Check(!python.IsChecked && !obscureChoice.IsChecked,
                    "changing active document refreshes both primary and obscure forced-language checks");
                await Choose(codeMode);
                Check(next.Document.State.format == VIEM_FORMAT_CODE && codeMode.IsChecked && !plain.IsChecked && !markdown.IsChecked,
                    "Code document type stays Code independently of filename detection: " + filename);
                await Close();
            }
            Check(languageItems.SequenceEqual(code.Items.Skip(3)) && obscureItems.SequenceEqual(obscure.Items),
                "reopening Format and File retains both populated language groups without duplicates");
            Check(window.Panes.All(p => p.LastError == null), "view-mode scenarios leave no presentation errors");
        }
        finally { App.Instance.Windows.Remove(window); window.Close(); }
        await CodeBlockLanguageTests.RunMenu(preferences);
    }
}
#endif

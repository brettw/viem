#if DEBUG
using System.Text;
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class CodeBlockLanguageTests
{
    internal static async Task RunMenu(Preferences preferences)
    {
        void Check(bool value, string name)
        { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }
        const string source = "before\n\n```rust\nfn main() {}\n```\n\nafter";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        MenuFlyout? menu = null;
        try
        {
            var pane = window.ActivePane!; var view = await pane.Ready;
            await Task.Delay(80);
            var state = document.State;
            var snapshot = view.Layout();
            var decoration = snapshot.Decorations.Single(item => (item.flags & VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0);
            var row = snapshot.Rows.Single(row => row.row_index == decoration.row_index);
            var label = Encoding.UTF8.GetString(snapshot.DecorationLabels, checked((int)decoration.label_byte_start), checked((int)decoration.label_byte_length));
            bool staleRejected = false;
            void Select(string language)
            {
                try { view.SetCodeBlockLanguage(state.document_id, state.document_revision, row.text_start, language); }
                catch (InvalidOperationException) { staleRejected = true; }
            }
            menu = EditorPane.CodeBlockLanguageMenu(label, true, Select);
            var none = (ToggleMenuFlyoutItem)menu.Items[0];
            var obscure = (MenuFlyoutSubItem)menu.Items[1];
            Check(none.Text == "None" && obscure.Text == "Obscure languages" && menu.Items[2] is MenuFlyoutSeparator,
                "code block picker places Obscure languages immediately after None and before its divider");
            Check(menu.Items.Skip(3).Cast<ToggleMenuFlyoutItem>().Select(item => item.Text)
                .SequenceEqual(DocumentModes.Languages.Where(language => language.Primary).Select(language => language.Name))
                && menu.Items.OfType<ToggleMenuFlyoutItem>().Any(item => item.Text == "Objective-C") && obscure.Items.Count == 0,
                "code block picker uses the shared sorted primary group including Objective-C and defers obscure controls");
            Check(!none.IsChecked && menu.Items.OfType<ToggleMenuFlyoutItem>().Single(item => item.Text == "Rust").IsChecked,
                "the native primary code block language entry reflects the captured label");
            menu.ShowAt(pane.Canvas); await Task.Delay(40);
            Check(obscure.Items.Cast<ToggleMenuFlyoutItem>().Select(item => item.Text)
                    .SequenceEqual(DocumentModes.Languages.Where(language => !language.Primary).Select(language => language.Name))
                && obscure.Items.Count + menu.Items.Count - 3 == DocumentModes.Languages.Count,
                "opening the code block picker retains the complete catalogue in its two shared language groups");
            var obscureLanguage = DocumentModes.Languages.First(language => !language.Primary);
            var obscureChoice = obscure.Items.Cast<ToggleMenuFlyoutItem>().Single(item => item.Text == obscureLanguage.Name);
            Check(obscure.Focus(FocusState.Programmatic), "code block obscure flyout takes native keyboard focus");
            await InputRoutingTests.Key(global::Windows.System.VirtualKey.Right); await Task.Delay(40);
            Check(obscureChoice.Focus(FocusState.Programmatic), "obscure code block language takes native keyboard focus");
            await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space); await Task.Delay(40);
            menu.Hide();
            Check(!staleRejected && Encoding.UTF8.GetString(document.Source(document.State.document_revision))
                    == source.Replace("```rust", "```" + obscureLanguage.Id, StringComparison.Ordinal),
                "native obscure code block choice changes only the captured fence annotation");
            view.Undo();
            Check(document.Source(document.State.document_revision).AsSpan().SequenceEqual(Encoding.UTF8.GetBytes(source)),
                "the native obscure language choice retains exact one-step undo");
            view.Redo();

            var expected = document.State;
            menu = EditorPane.CodeBlockLanguageMenu(obscureLanguage.Name + " ▾", true, language =>
            {
                try { view.SetCodeBlockLanguage(expected.document_id, expected.document_revision, row.text_start, language); }
                catch (InvalidOperationException) { staleRejected = true; }
            });
            obscure = (MenuFlyoutSubItem)menu.Items[1];
            Check(((FontIcon)obscure.Icon).Glyph == "\uE73E", "an obscure code block language checks its containing flyout");
            menu.ShowAt(pane.Canvas); await Task.Delay(40);
            Check(obscure.Items.Cast<ToggleMenuFlyoutItem>().Single(item => item.Text == obscureLanguage.Name).IsChecked,
                "an obscure code block language retains its native child check");
            view.SetCodeBlockLanguage(expected.document_id, expected.document_revision, row.text_start, "rust");
            var python = menu.Items.OfType<ToggleMenuFlyoutItem>().Single(item => item.Text == "Python");
            Check(python.Focus(FocusState.Programmatic), "primary code block language takes native keyboard focus");
            await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space); await Task.Delay(40); menu.Hide();
            Check(staleRejected && Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                "a native language choice cannot refresh away its captured stale source revision");

            menu = EditorPane.CodeBlockLanguageMenu("Rust ▾", false, _ => throw new InvalidOperationException("read-only choice invoked"));
            menu.ShowAt(pane.Canvas); await Task.Delay(40);
            obscure = (MenuFlyoutSubItem)menu.Items[1];
            Check(!obscure.IsEnabled && menu.Items.OfType<ToggleMenuFlyoutItem>().All(item => !item.IsEnabled)
                && obscure.Items.Cast<ToggleMenuFlyoutItem>().All(item => !item.IsEnabled),
                "read-only code block menus disable primary and obscure language actions");
            Check(pane.LastError == null, "code block language menu tests leave no presentation errors");
        }
        finally { menu?.Hide(); await window.ClosePane(window.ActivePane!, force: true); App.Instance.Windows.Remove(window); }
    }

    internal static void Run(CanvasDevice device, DispatcherQueue dispatcher, Action<bool, string> check)
    {
        const string source = "before\n\n```rust\nfn main() {}\n```\n\nafter";
        using var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_MARKDOWN);
        using var view = new CoreView(document, device, dispatcher, 700, 400);
        string Source() => Encoding.UTF8.GetString(document.Source(document.State.document_revision));
        string Label()
        {
            var snapshot = view.Layout();
            var label = snapshot.Decorations.Single(item => (item.flags & VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0);
            var row = snapshot.Rows.Single(row => row.row_index == label.row_index);
            check(label.typographic_bounds.y + label.typographic_bounds.height <= row.y + .01,
                "code language furniture fits above the literal body without editable positions");
            return Encoding.UTF8.GetString(snapshot.DecorationLabels, checked((int)label.label_byte_start), checked((int)label.label_byte_length));
        }
        check(Label() == "Rust ▾" && Source() == source, "Windows exports the Markdown code language label without changing source");
        var before = view.Presentation;
        var state = document.State;
        var snapshot = view.Layout();
        var languageLabel = snapshot.Decorations.Single(item => (item.flags & VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0);
        var blockRow = snapshot.Rows.Single(row => row.row_index == languageLabel.row_index);
        view.SetCodeBlockLanguage(state.document_id, state.document_revision, blockRow.text_start, "");
        check(Source() == source.Replace("```rust", "```", StringComparison.Ordinal) && Label() == "None ▾",
            "Windows None selection changes only the source fence annotation and refreshes its label");
        check(view.Presentation.cursor_utf8_offset == before.cursor_utf8_offset && view.Presentation.mode == before.mode,
            "code language selection preserves the invoking caret and mode");
        bool staleRejected = false;
        try { view.SetCodeBlockLanguage(state.document_id, state.document_revision, blockRow.text_start, "python"); }
        catch (InvalidOperationException) { staleRejected = true; }
        check(staleRejected, "a stale Windows code language menu cannot mutate the later snapshot");
        view.Undo();
        check(Source() == source && Label() == "Rust ▾", "one Windows undo restores exact code language bytes and furniture");
        document.SetReadOnly(true);
        var readOnly = document.State;
        bool readOnlyRejected = false;
        try { view.SetCodeBlockLanguage(readOnly.document_id, readOnly.document_revision, blockRow.text_start, "python"); }
        catch (InvalidOperationException) { readOnlyRejected = true; }
        check(readOnlyRejected && Source() == source, "Windows read-only policy rejects code language authoring without changing source");
        document.SetReadOnly(false);
        view.SetMarkdownSource(true);
        check(!view.Layout().Decorations.Any(item => (item.flags & VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0),
            "Markdown Source retains literal fences without WYSIWYG language furniture");
        CheckScrolledLanguageChange(device, dispatcher, check);
    }

    private static void CheckScrolledLanguageChange(CanvasDevice device, DispatcherQueue dispatcher, Action<bool, string> check)
    {
        string source = string.Concat(Enumerable.Repeat("before\n\n", 100))
            + "```rust\n" + string.Concat(Enumerable.Repeat("fn main() {}\n", 12)) + "```\n\n"
            + string.Concat(Enumerable.Repeat("after\n\n", 100));
        using var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_MARKDOWN);
        using var view = new CoreView(document, device, dispatcher, 700, 400);
        view.GoToLine(101);
        var snapshot = view.Layout();
        var label = snapshot.Decorations.Single(item => (item.flags & VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0);
        var row = snapshot.Rows.Single(item => item.row_index == label.row_index);
        float top = label.typographic_bounds.y - 60;
        view.GoToLine(1);
        view.Scroll(0, top);
        var before = view.Viewport;
        var caret = view.Presentation;
        check(caret.cursor_utf8_offset == 0 && before.top > 100,
            "Windows language scroll fixture keeps its caret above the visible code block");
        foreach (string language in new[] { "", "rust" })
        {
            var state = document.State;
            view.SetCodeBlockLanguage(state.document_id, state.document_revision, row.text_start, language);
            check(view.Viewport.top == before.top && view.Viewport.left == before.left,
                "Windows code language selection preserves the viewport rather than revealing the offscreen caret");
            for (int attempt = 0; attempt < 30; ++attempt)
            {
                document.PollSyntax();
                view.Refresh();
                check(view.Viewport.top == before.top && view.Viewport.left == before.left
                    && view.Presentation.cursor_utf8_offset == caret.cursor_utf8_offset && view.Presentation.mode == caret.mode,
                    "Windows asynchronous code syntax refresh preserves the scrolled viewport and caret");
            }
        }
        check(Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
            "Windows language scroll checks leave all document source except the chosen fence annotation untouched");
    }
}
#endif

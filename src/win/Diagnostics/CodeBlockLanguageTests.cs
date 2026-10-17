#if DEBUG
using System.Text;
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Interop;
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
            await InputRoutingTests.Settle();
            var snapshot = view.Layout();
            var decoration = snapshot.Decorations.Single(item => (item.flags & VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0);
            var row = snapshot.Rows.Single(row => row.row_index == decoration.row_index);
            bool staleRejected = false;
            var viewport = view.Viewport;
            var bounds = new global::Windows.Foundation.Rect(decoration.typographic_bounds.x - viewport.left,
                decoration.typographic_bounds.y - viewport.top, decoration.typographic_bounds.width, decoration.typographic_bounds.height);
            var center = new global::Windows.Foundation.Point(bounds.X + bounds.Width / 2, bounds.Y + bounds.Height / 2);
            using var image = new CanvasRenderTarget(pane.Canvas.Device, (float)pane.Canvas.ActualWidth, (float)pane.Canvas.ActualHeight, pane.Canvas.Dpi);
            byte[] Draw() { using (var drawing = image.CreateDrawingSession()) pane.Draw(drawing); return image.GetPixelBytes(); }
            pane.MoveCodeBlockLanguageHover(null);
            Check(pane.CodeBlockLanguageOpacity(decoration) == .25f, "the code language overlay starts at 25% opacity");
            byte[] idle = Draw(); int builds = pane.DrawingCacheBuilds; long shaped = view.Provider.ShapedCharacters;
            pane.MoveCodeBlockLanguageHover(center);
            Check(pane.CodeBlockLanguageOpacity(decoration) == 1, "hovering the code language overlay raises opacity to 100%");
            byte[] hovered = Draw();
            Check(!hovered.AsSpan().SequenceEqual(idle) && pane.DrawingCacheBuilds == builds
                && view.Provider.ShapedCharacters == shaped && CoreView.SameLayout(view.LayoutInfo().identity, snapshot.Info.identity),
                "selector hover changes native pixels without rebuilding text, reshaping or relayout");
            pane.InvalidateDrawingCache();
            Check(Draw().AsSpan().SequenceEqual(hovered), "rebuilt code drawing retains identical selector overlay pixels");
            pane.MoveCodeBlockLanguageHover(null);
            Check(Draw().AsSpan().SequenceEqual(idle), "leaving the selector restores its idle frame");
            await InputRoutingTests.Drag(window, pane.Canvas,
                new global::Windows.Foundation.Point[] { new(center.X / pane.Canvas.ActualWidth, center.Y / pane.Canvas.ActualHeight) },
                _ => Check(pane.CodeBlockLanguageFlyout != null && pane.CodeBlockLanguageOpacity(decoration) == 1,
                    "a native selector click opens its menu at full opacity"), focusTarget: false);
            menu = pane.CodeBlockLanguageFlyout ?? throw new InvalidOperationException("The native code language picker did not open.");
            pane.MoveCodeBlockLanguageHover(null);
            Check(pane.CodeBlockLanguageOpacity(decoration) == 1, "an open native language menu keeps full opacity after pointer exit");
            var none = (ToggleMenuFlyoutItem)menu.Items[0];
            var obscure = (MenuFlyoutSubItem)menu.Items[1];
            await MenuTestHelpers.Transition(true, () => Task.CompletedTask, none, obscure);
            Check(none.Text == "None" && obscure.Text == "Obscure languages" && menu.Items[2] is MenuFlyoutSeparator,
                "code block picker places Obscure languages immediately after None and before its divider");
            Check(menu.Items.Skip(3).Cast<ToggleMenuFlyoutItem>().Select(item => item.Text)
                .SequenceEqual(DocumentModes.Languages.Where(language => language.Primary).Select(language => language.Name))
                && menu.Items.OfType<ToggleMenuFlyoutItem>().Any(item => item.Text == "Objective-C"),
                "code block picker uses the shared sorted primary group including Objective-C");
            Check(!none.IsChecked && menu.Items.OfType<ToggleMenuFlyoutItem>().Single(item => item.Text == "Rust").IsChecked,
                "the native primary code block language entry reflects the captured label");
            Check(obscure.Items.Cast<ToggleMenuFlyoutItem>().Select(item => item.Text)
                    .SequenceEqual(DocumentModes.Languages.Where(language => !language.Primary).Select(language => language.Name))
                && obscure.Items.Count + menu.Items.Count - 3 == DocumentModes.Languages.Count,
                "opening the code block picker retains the complete catalogue in its two shared language groups");
            var obscureLanguage = DocumentModes.Languages.First(language => !language.Primary);
            var obscureChoice = obscure.Items.Cast<ToggleMenuFlyoutItem>().Single(item => item.Text == obscureLanguage.Name);
            Check(obscure.Focus(FocusState.Programmatic), "code block obscure flyout takes native keyboard focus");
            await MenuTestHelpers.Transition(true, () => InputRoutingTests.Key(global::Windows.System.VirtualKey.Right), obscureChoice);
            Check(obscureChoice.Focus(FocusState.Programmatic), "obscure code block language takes native keyboard focus");
            await MenuTestHelpers.Transition(false, async () => {
                await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space); menu.Hide();
            }, none, obscure, obscureChoice);
            pane.MoveCodeBlockLanguageHover(null);
            Check(pane.CodeBlockLanguageOpacity(decoration) == .25f, "closing the menu without hover restores idle selector opacity");
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
                catch (CoreException error) when (error.Status == VIEM_STATUS_STALE_REVISION) { staleRejected = true; }
            });
            obscure = (MenuFlyoutSubItem)menu.Items[1];
            Check(((FontIcon)obscure.Icon).Glyph == "\uE73E", "an obscure code block language checks its containing flyout");
            await MenuTestHelpers.Transition(true, () => { menu.ShowAt(pane.Canvas); return Task.CompletedTask; }, menu.Items[0], obscure);
            Check(obscure.Items.Cast<ToggleMenuFlyoutItem>().Single(item => item.Text == obscureLanguage.Name).IsChecked,
                "an obscure code block language retains its native child check");
            view.SetCodeBlockLanguage(expected.document_id, expected.document_revision, row.text_start, "rust");
            var python = menu.Items.OfType<ToggleMenuFlyoutItem>().Single(item => item.Text == "Python");
            Check(python.Focus(FocusState.Programmatic), "primary code block language takes native keyboard focus");
            await MenuTestHelpers.Transition(false, async () => {
                await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space); menu.Hide();
            }, menu.Items[0], obscure, python);
            Check(staleRejected && Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                "a native language choice cannot refresh away its captured stale source revision");

            menu = EditorPane.CodeBlockLanguageMenu("Rust ▾", false, _ => throw new InvalidOperationException("read-only choice invoked"));
            obscure = (MenuFlyoutSubItem)menu.Items[1];
            await MenuTestHelpers.Transition(true, () => { menu.ShowAt(pane.Canvas); return Task.CompletedTask; }, menu.Items[0], obscure);
            Check(!obscure.IsEnabled && menu.Items.OfType<ToggleMenuFlyoutItem>().All(item => !item.IsEnabled)
                && obscure.Items.Cast<ToggleMenuFlyoutItem>().All(item => !item.IsEnabled),
                "read-only code block menus disable primary and obscure language actions");
            Check(pane.LastError == null, "code block language menu tests leave no presentation errors");
            await MenuTestHelpers.Transition(false, () => { menu.Hide(); return Task.CompletedTask; }, menu.Items[0], obscure);
            var survivor = window.SplitPane(pane, document); await survivor.Ready;
            window.FocusPane(pane); await Task.Delay(80);
            var closingLabel = view.Layout().Decorations.Single(item => (item.flags & VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0);
            viewport = view.Viewport;
            center = new(closingLabel.typographic_bounds.x - viewport.left + closingLabel.typographic_bounds.width / 2,
                closingLabel.typographic_bounds.y - viewport.top + closingLabel.typographic_bounds.height / 2);
            await InputRoutingTests.Drag(window, pane.Canvas,
                new global::Windows.Foundation.Point[] { new(center.X / pane.Canvas.ActualWidth, center.Y / pane.Canvas.ActualHeight) },
                _ => { }, focusTarget: false);
            menu = pane.CodeBlockLanguageFlyout ?? throw new InvalidOperationException("The closing pane's native language picker did not open.");
            await MenuTestHelpers.Transition(false, () => window.ClosePane(pane, force: true), menu.Items[0], menu.Items[1]);
            Check(pane.CodeBlockLanguageFlyout == null && pane.CodeBlockLanguageOpacity(closingLabel) == .25f,
                "closing a pane with its native language menu open retires selector ownership without querying detached geometry");
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
            check(label.typographic_bounds.y < row.y + row.ascent + row.descent + row.leading
                && label.typographic_bounds.y + label.typographic_bounds.height > row.y,
                "code language furniture overlays the literal body without reserving layout space or editable positions");
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
        catch (CoreException error) when (error.Status == VIEM_STATUS_STALE_REVISION) { staleRejected = true; }
        check(staleRejected, "a stale Windows code language menu cannot mutate the later snapshot");
        view.Undo();
        check(Source() == source && Label() == "Rust ▾", "one Windows undo restores exact code language bytes and furniture");
        document.SetReadOnly(true);
        var readOnly = document.State;
        bool readOnlyRejected = false;
        try { view.SetCodeBlockLanguage(readOnly.document_id, readOnly.document_revision, blockRow.text_start, "python"); }
        catch (CoreException error) when (error.Status == VIEM_STATUS_UNSUPPORTED_OPERATION) { readOnlyRejected = true; }
        check(readOnlyRejected && Source() == source, "Windows read-only policy rejects code language authoring without changing source");
        document.SetReadOnly(false);
        view.SetMarkdownPreview(false);
        check(!view.Layout().Decorations.Any(item => (item.flags & VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) != 0),
            "Markdown Preview off retains literal fences without language furniture");
        CheckScrolledLanguageChange(device, dispatcher, check);
    }

    private static void CheckScrolledLanguageChange(CanvasDevice device, DispatcherQueue dispatcher, Action<bool, string> check)
    {
        string source = string.Concat(Enumerable.Repeat("before\n\n", 100))
            + "```rust\n" + string.Concat(Enumerable.Repeat("fn main() {}\n", 12)) + "```\n\n"
            + string.Concat(Enumerable.Repeat("after\n\n", 100));
        using var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_MARKDOWN);
        using var view = new CoreView(document, device, dispatcher, 700, 400);
        view.GoToLine(202);
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

#if DEBUG
using System.Text;
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Dispatching;
using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class CodeBlockLanguageTests
{
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

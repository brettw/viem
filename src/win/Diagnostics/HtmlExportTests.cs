#if DEBUG
using System.Text;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class HtmlExportTests
{
    private static void Check(bool value, string name)
    { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }

    internal static async Task Run(EditorWindow window, EditorPane pane, string profile)
    {
        var file = window.Menu.Items.Single(item => item.Title == "File");
        Check(file.Items.OfType<MenuFlyoutItem>().Any(item => item.Text == "Export…"), "File menu includes HTML Export");
        foreach (string title in new[] { "Convert to", "Reinterpret as" })
            Check(!file.Items.OfType<MenuFlyoutSubItem>().Any(item => item.Text == title),
                title + " is absent from the File menu");

        string directory = Path.Combine(profile, "html-export");
        Directory.CreateDirectory(directory);
        var document = pane.Document;
        var view = pane.View!;
        string? originalPath = document.FilePath;
        var initial = document.State;
        byte[] original = document.Source(initial.document_revision);
        Check(await window.Export(pane, Path.Combine(directory, "clean.html")), "clean Markdown exports through the native file service");
        string html = File.ReadAllText(Path.Combine(directory, "clean.html"));
        Check(html.Contains("<!doctype html>", StringComparison.OrdinalIgnoreCase) && html.Contains("font-size:"),
            "Markdown export writes a complete UTF-8 HTML document with resolved CSS");
        Check(document.State.document_revision == initial.document_revision && document.State.flags == initial.flags
            && document.FilePath == originalPath && document.Source(initial.document_revision).AsSpan().SequenceEqual(original),
            "clean export preserves source, revision, filename, and save state");

        view.Command("ggi"); view.Text("Export & <sample> "); view.Key(VIEM_KEY_ESCAPE);
        var dirty = document.State;
        byte[] unsaved = document.Source(dirty.document_revision);
        var cursor = view.Presentation;
        Check(await window.Export(pane, Path.Combine(directory, "dirty.html")), "unsaved Markdown exports successfully");
        Check(document.IsDirty && document.State.document_revision == dirty.document_revision
            && document.FilePath == originalPath && document.Source(dirty.document_revision).AsSpan().SequenceEqual(unsaved)
            && view.Presentation.cursor_utf8_offset == cursor.cursor_utf8_offset && view.Presentation.mode == cursor.mode,
            "export preserves unsaved edits and the editing session");
        bool rejected = false;
        try { await window.Export(pane, originalPath!); } catch (IOException) { rejected = true; }
        Check(rejected && File.ReadAllBytes(originalPath!).AsSpan().SequenceEqual(original),
            "export rejects the open source path before changing its bytes");
        rejected = false;
        try { await window.Export(pane, directory); } catch (IOException) { rejected = true; } catch (UnauthorizedAccessException) { rejected = true; }
        Check(rejected && document.IsDirty && document.State.document_revision == dirty.document_revision,
            "failed export leaves unsaved document state intact");
        view.Undo();
        Check(document.Source(document.State.document_revision).AsSpan().SequenceEqual(original), "export does not add an undo entry");

        byte[] source = "<!doctype html>\n<p class=\"sample\">Literal &amp; text</p>\n"u8.ToArray();
        var code = window.AddPane(new CoreDocument(source, Path.Combine(directory, "source.HTML")));
        await code.Ready;
        try
        {
            Check(code.Document.State.format == VIEM_FORMAT_CODE && code.Document.FormattedText() == Encoding.UTF8.GetString(source),
                "HTML opens as editable literal Code");
            code.Document.SetReadOnly(true);
            Check(await window.Export(code, Path.Combine(directory, "code.html")), "read-only Code documents can export a copy");
            html = File.ReadAllText(Path.Combine(directory, "code.html"));
            Check(html.Contains("&lt;") && html.Contains("&amp;") && html.Contains("color:"),
                "Code export escapes HTML syntax and includes syntax CSS");
            Check(!code.Document.IsDirty && code.Document.Source(code.Document.State.document_revision).AsSpan().SequenceEqual(source),
                "Code export preserves original literal HTML source");
        }
        finally { await window.ClosePane(code); }

        var untitled = window.AddPane(new CoreDocument("Untitled & exported"u8.ToArray()));
        await untitled.Ready;
        try
        {
            Check(await window.Export(untitled, Path.Combine(directory, "untitled.html"))
                && untitled.Document.FilePath == null && !untitled.Document.IsDirty,
                "untitled documents export without adopting a source path");
        }
        finally { await window.ClosePane(untitled); }

        foreach (string legacy in new[] { "html", "htmlSource" })
            Check(new RecoverySnapshot(source, legacy, 1, 1, 1, 1).Format == VIEM_FORMAT_CODE,
                "legacy " + legacy + " recovery uses Code without changing source bytes");
    }
}
#endif

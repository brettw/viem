#if DEBUG
using System.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Viem.Windows.Input;
using Viem.Windows.Rendering;
using Viem.Windows.Shell;
using Windows.Foundation;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class ImageInteractionTests
{
    private static void Check(bool condition, string name)
    { if (!condition) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }

    internal static async Task Run(Preferences preferences)
    {
        Check(ImageLocation.LocalPreviewPath("https://example.invalid/a.png", null) == null
            && ImageLocation.LocalPreviewPath("file://server/share/a.png", null) == null,
            "image preview resolution rejects web and network file resources before opening them");
        Check(!LocalImagePreview.IsRaster("<svg xmlns='http://www.w3.org/2000/svg'><image href='https://example.invalid'/></svg>"u8),
            "image previews never pass SVG or embedded resource references to a decoder");
        const string source = "![remote](https://example.invalid/a.png)\u0301 tail\n\nplain";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        var pane = window.ActivePane!; var view = await pane.Ready;
        string Source() => Encoding.UTF8.GetString(document.Source(document.State.document_revision));
        try
        {
            await Task.Delay(100);
            view.Place(0, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            Check(window.Toolbar.InsertImage.IsEnabled && AutomationProperties.GetName(window.Toolbar.InsertImage) == "Insert image",
                "Markdown exposes the native insert-image toolbar action");
            var group = VisualTreeHelper.GetParent(window.Toolbar.InsertImage) as StackPanel;
            Check(group != null && group.Children.IndexOf(window.Toolbar.InsertImage) == group.Children.IndexOf(window.Toolbar.Buttons[ToolbarAction.CodeBlock]) + 1,
                "the image toolbar action immediately follows Code Block in its group");
            var cluster = view.Layout().Clusters.Single(c => view.Provider.IsInlineImage(c.render_run));
            Check(cluster.text_start == 0 && cluster.text_end == 3 && cluster.typographic_bounds.height > 0,
                "an image is an atomic layout object even beside a combining mark");
            Check(pane.ImagePopupVisible && !pane.ImageEditorVisible && pane.PendingImageLoads == 0 && pane.LocalImagePreviewCount == 0,
                "remote images display a location popup and placeholder without submitting any resource load");
            var viewport = view.Viewport;
            Check(pane.SelectInlineImage(new Point(cluster.typographic_bounds.x + cluster.typographic_bounds.width - 2 - viewport.left,
                cluster.typographic_bounds.y + cluster.typographic_bounds.height / 2 - viewport.top)) && view.Presentation.cursor_utf8_offset == 0,
                "clicking the right half of an image selects the object rather than placing the caret beyond it");
            var imageSelection = view.LogicalSelection();
            Check(view.HasSelection && imageSelection.text_start == 0 && imageSelection.text_end == 3 && pane.ImagePopupVisible,
                "an image click selects its complete logical object and retains its location popup");
            await pane.Copy(false);
            var copied = await ClipboardFormats.Read();
            using (var json = System.Text.Json.JsonDocument.Parse(copied.Fragment))
            {
                Check(copied.Text == "![remote](<https://example.invalid/a.png>)" && json.RootElement.GetProperty("plain_text").GetString() == "\uFFFC",
                    "image copy exposes passive Markdown text while retaining the private atomic object payload");
                string html = ClipboardFormats.Html(copied.Fragment);
                Check(!html.Contains("<img", StringComparison.OrdinalIgnoreCase) && !html.Contains('\uFFFC')
                    && html.Contains("&lt;https://example.invalid/a.png&gt;", StringComparison.Ordinal),
                    "image clipboard HTML contains encoded location text and no fetching resource tags");
            }
            // Exercise the core host-write publication without the native
            // Copy helper's representation override as a separate entry path.
            view.CopyOrCut(false);
            var hostCopy = await ClipboardFormats.Read();
            Check(hostCopy.Text == "![remote](<https://example.invalid/a.png>)" && hostCopy.Fragment.Length > 0 && pane.LastError == null,
                "image clipboard host effects publish fallback text without rejecting the private U+FFFC payload");
            pane.ShowInsertImage();
            Check(pane.ImageEditorVisible && pane.ImageTextValue == "remote" && pane.ImageDestinationValue == "https://example.invalid/a.png",
                "image editing expands into native alt text and location fields");
            pane.ImageDestinationValue = "javascript:alert(1)"; pane.ApplyImageEditor();
            Check(pane.ImageEditorVisible && Source() == source, "invalid image locations retain the draft without changing source");
            pane.ImageDestinationValue = "missing.png"; pane.ImageTextValue = ""; pane.ApplyImageEditor();
            Check(Source().StartsWith("![](<missing.png>)", StringComparison.Ordinal), "an empty image alt text remains empty when editing its location");
            view.Undo(); Check(Source() == source, "image edit undo restores exact original source spelling");
            view.SetMarkdownSource(true);
            foreach (ulong offset in new ulong[] { 0, 1, 8, 15, 37 })
            {
                view.Place(offset, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
                Check(view.ImageContext().Image?.Destination == "https://example.invalid/a.png" && pane.ImagePopupVisible,
                    "Source image markup activates the location popup at offset " + offset);
            }
            view.SetMarkdownSource(false);
            view.Place(0, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            var previous = view.ImageContext();
            view.Command("G0");
            bool rejected = false;
            try { view.EditImage(previous, "stale", "other.png"); } catch (InvalidOperationException) { rejected = true; }
            Check(rejected && Source() == source, "a stale image popup cannot edit a later selection");
            view.Command("gg0"); view.EditImage(view.ImageContext(), "", "", remove: true);
            Check(!Source().Contains("![", StringComparison.Ordinal) && !Source().Contains("remote", StringComparison.Ordinal),
                "deleting an image removes its source construct and alt text");
            view.Undo(); Check(Source() == source, "image deletion undo restores the exact source");
            view.Command("G0viw"); pane.ShowInsertImage();
            Check(pane.ImageTextValue == "plain", "selected text seeds the inserted image's alt text");
            pane.ImageDestinationValue = "https://example.invalid/new.png"; pane.ApplyImageEditor();
            Check(document.FormattedText().EndsWith("\uFFFC", StringComparison.Ordinal), "inserting an image replaces selected text with one atomic object");
            view.Undo();
            Check(pane.LastError == null, "image interaction diagnostics complete without presentation errors");
        }
        finally { await window.ClosePane(pane, force: true); App.Instance.Windows.Remove(window); }
        await NestedPopups(preferences);
        await KeyboardSelection(preferences);
        await LocalPreview(preferences);
        await CacheBudgets(preferences);
    }

    private static async Task NestedPopups(Preferences preferences)
    {
        const string source = "[![alt](https://example.invalid/image.png)](https://example.invalid/page)";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        var pane = window.ActivePane!; var view = await pane.Ready;
        try
        {
            await Task.Delay(100);
            view.Place(0, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            Check(pane.ImagePopupVisible && !pane.LinkPopupVisible, "a linked image shows only its image location toolbar");
            pane.ShowInsertImage(); pane.Refresh();
            Check(pane.ImageEditorVisible && !pane.LinkPopupVisible, "passive link refresh cannot dismiss a linked image's edit draft");
            pane.ShowInsertLink(); pane.Refresh();
            Check(pane.LinkEditorVisible && !pane.ImagePopupVisible, "an explicitly opened parent link editor survives passive image refresh");
            pane.DismissLinkPopup(); pane.FocusEditor(); pane.Refresh();
            Check(pane.ImagePopupVisible && !pane.LinkPopupVisible, "closing parent link editing restores the linked image's passive toolbar");
            Check(Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                "switching between nested link and image popups does not mutate source");
        }
        finally { await window.ClosePane(pane, force: true); App.Instance.Windows.Remove(window); }
    }

    private static async Task KeyboardSelection(Preferences preferences)
    {
        const string source = "A![alt](https://example.invalid/image.png)B";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        var pane = window.ActivePane!; var view = await pane.Ready;
        try
        {
            await Task.Delay(100);
            view.Command("i"); view.Key(VIEM_KEY_RIGHT);
            var selection = view.LogicalSelection();
            Check(view.HasSelection && selection.text_start == 1 && selection.text_end == 4
                && pane.CaretIsOnInlineImage() && pane.ImagePopupVisible,
                "Insert Right onto an image selects its complete object and displays an outline and location popup");
            view.Key(VIEM_KEY_RIGHT);
            Check(view.Presentation.mode == VIEM_MODE_INSERT && view.Presentation.cursor_utf8_offset == 4
                && !pane.CaretIsOnInlineImage(), "Right past a selected image restores the ordinary Insert caret");
            view.Key(VIEM_KEY_LEFT);
            selection = view.LogicalSelection();
            Check(view.HasSelection && selection.text_start == 1 && selection.text_end == 4 && pane.CaretIsOnInlineImage(),
                "Left onto an image uses its complete selection to suppress the thin caret at either active endpoint");
            view.Text("X");
            Check(document.FormattedText() == "AXB", "typing replaces the keyboard-selected image as one atomic object");
            view.Undo();
            Check(Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                "undo restores the image's original source after keyboard replacement");
        }
        finally { await window.ClosePane(pane, force: true); App.Instance.Windows.Remove(window); }
    }

    private static async Task LocalPreview(Preferences preferences)
    {
        string directory = Path.Combine(Path.GetTempPath(), "viem-image-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        string path = Path.Combine(directory, "wide.bmp");
        // A deterministic raster fixture needs no system asset or network.
        using (var file = File.Create(path)) using (var writer = new BinaryWriter(file))
        {
            const int width = 1600, height = 800, bytes = width * height * 3;
            writer.Write((ushort)0x4D42); writer.Write(54 + bytes); writer.Write(0); writer.Write(54);
            writer.Write(40); writer.Write(width); writer.Write(height); writer.Write((ushort)1); writer.Write((ushort)24);
            writer.Write(0); writer.Write(bytes); writer.Write(0); writer.Write(0); writer.Write(0); writer.Write(0);
            writer.Write(new byte[bytes]);
        }
        const string source = "before ![local](wide.bmp) after";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), Path.Combine(directory, "source.md"), VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        var pane = window.ActivePane!; var view = await pane.Ready;
        try
        {
            for (int i = 0; i < 100 && pane.LocalImagePreviewCount == 0; i++) await Task.Delay(50);
            Check(pane.LocalImagePreviewCount == 1 && pane.PendingImageLoads == 0, "a relative local raster image decodes within the bounded preview cache");
            var image = view.Layout().Clusters.Single(c => view.Provider.IsInlineImage(c.render_run));
            Check(Math.Abs(image.typographic_bounds.height / image.typographic_bounds.width - .5) < .001
                && image.typographic_bounds.width <= view.Layout().Info.viewport_width,
                "local images preserve intrinsic aspect ratio and fit the document width");
            view.Resize(400, 300);
            var narrower = view.Layout().Clusters.Single(c => view.Provider.IsInlineImage(c.render_run));
            Check(narrower.typographic_bounds.width < image.typographic_bounds.width
                && Math.Abs(narrower.typographic_bounds.height / narrower.typographic_bounds.width - .5) < .001,
                "resizing reflows the same cached image without changing its aspect ratio");
            Check(!document.IsDirty && Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                "local image decoding and layout never modify document source or dirty state");
        }
        finally
        {
            await window.ClosePane(pane, force: true); App.Instance.Windows.Remove(window);
            Directory.Delete(directory, recursive: true);
        }
        Check(pane.LocalImagePreviewCount == 0 && pane.PendingImageLoads == 0, "closing a view releases decoded images and cancels its preview work");
    }

    private static async Task CacheBudgets(Preferences preferences)
    {
        async Task Fixture(int count, int? pixelSize, long? budget, int loaded, int attempts, string label)
        {
            string directory = Path.Combine(Path.GetTempPath(), "viem-image-budget-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(directory);
            if (pixelSize is int size)
            {
                int stride = (size * 3 + 3) & ~3, bytes = stride * size;
                for (int i = 0; i < count; i++)
                {
                    using var file = File.Create(Path.Combine(directory, "image" + i + ".bmp"));
                    using var writer = new BinaryWriter(file);
                    writer.Write((ushort)0x4D42); writer.Write(54 + bytes); writer.Write(0); writer.Write(54);
                    writer.Write(40); writer.Write(size); writer.Write(size); writer.Write((ushort)1); writer.Write((ushort)24);
                    writer.Write(0); writer.Write(bytes); writer.Write(0); writer.Write(0); writer.Write(0); writer.Write(0);
                    writer.Write(new byte[bytes]);
                }
            }
            string source = string.Join(pixelSize == null ? "\n\n" : " ", Enumerable.Range(0, count).Select(i => "![](image" + i + ".bmp)"));
            var document = new CoreDocument(Encoding.UTF8.GetBytes(source), Path.Combine(directory, "source.md"), VIEM_FORMAT_MARKDOWN);
            var window = new EditorWindow(preferences, document);
            App.Instance.Windows.Add(window); window.Activate();
            var pane = window.ActivePane!; var view = await pane.Ready;
            if (budget is long limit) pane.SetImagePreviewByteBudgetForTest(limit);
            try
            {
                for (int i = 0; i < 200 && (pane.ImagePreviewLoadAttempts < attempts || pane.PendingImageLoads != 0); i++) await Task.Delay(25);
                Check(pane.ImagePreviewLoadAttempts == attempts && pane.PendingImageLoads == 0 && pane.LocalImagePreviewCount == loaded,
                    label + " settles its bounded image loading work");
                Check(pane.ImagePreviewEntries <= 48 && pane.CompletedImagePreviewEntries == pane.ImagePreviewEntries
                    && pane.ImagePreviewBytes <= (budget ?? 64 * 1024 * 1024),
                    label + " retains only admitted previews and completed placeholders within its budgets");
                ulong generation = view.Provider.Generation;
                for (int i = 0; i < 20; i++) pane.Refresh();
                await Task.Delay(100);
                Check(pane.ImagePreviewLoadAttempts == attempts && pane.PendingImageLoads == 0 && view.Provider.Generation == generation,
                    label + " repeated refreshes neither redecode nor churn image metrics");
                Check(!document.IsDirty && Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                    label + " leaves source and dirty state untouched");
            }
            finally
            {
                await window.ClosePane(pane, force: true); App.Instance.Windows.Remove(window);
                Directory.Delete(directory, recursive: true);
            }
        }
        // Tiny images progressively expose more than the entry budget within
        // one viewport; the overflow must remain a stable URL placeholder.
        await Fixture(80, 1, null, 48, 48, "an oversized visible image set");
        await Fixture(3, 20, 1600, 1, 3, "a visible set exceeding decoded memory");
        await Fixture(3, null, null, 0, 3, "failed local image requests ahead of a full queue");
    }

}
#endif

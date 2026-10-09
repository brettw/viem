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
            Check(!window.Toolbar.InsertImage.IsEnabled && window.Toolbar.InsertImage.IsChecked == true
                && AutomationProperties.GetName(window.Toolbar.InsertImage) == "Insert image",
                "the Normal image caret activates and disables the native image toolbar button");
            var group = VisualTreeHelper.GetParent(window.Toolbar.InsertImage) as StackPanel;
            Check(group != null && group.Children.IndexOf(window.Toolbar.InsertImage) == group.Children.IndexOf(window.Toolbar.Buttons[ToolbarAction.CodeBlock]) + 1,
                "the image toolbar action immediately follows Code Block in its group");
            var cluster = view.Layout().Clusters.Single(c => view.Provider.IsInlineImage(c.render_run));
            Check(cluster.text_start == 0 && cluster.text_end == 3 && cluster.typographic_bounds.height > 0,
                "an image is an atomic layout object even beside a combining mark");
            Check(pane.ImagePopupVisible && !pane.ImageEditorVisible && pane.PendingImageLoads == 0 && pane.LocalImagePreviewCount == 0,
                "remote images display a location popup and placeholder without submitting any resource load");
            int remoteLoads = pane.ImagePreviewLoadAttempts;
            pane.ReloadShownImage();
            Check(pane.ImageReloadVisible && !pane.ImageReloadEnabled && pane.ImagePreviewLoadAttempts == remoteLoads,
                "remote images expose a disabled reload action that never submits a fetch");
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
                Check(view.ImageContext().Image?.Destination == "https://example.invalid/a.png" && pane.ImagePopupVisible
                    && window.Toolbar.InsertImage.IsChecked == true && !window.Toolbar.InsertImage.IsEnabled,
                    "Source image markup activates the location popup at offset " + offset);
                Check(!pane.ImageReloadVisible, "Source image popups omit the preview reload action");
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
        await HtmlImageDimensions(preferences);
        await PreviewLimits(preferences);
        await ScrollDuringImageReload(preferences);
        await ImageStyle(preferences);
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
                && pane.CaretIsOnInlineImage() && pane.ImagePopupVisible
                && window.Toolbar.InsertImage.IsChecked == true && !window.Toolbar.InsertImage.IsEnabled,
                "Insert Right onto an image selects its complete object and displays an outline and location popup");
            view.Key(VIEM_KEY_RIGHT);
            Check(view.Presentation.mode == VIEM_MODE_INSERT && view.Presentation.cursor_utf8_offset == 4
                && !pane.CaretIsOnInlineImage() && window.Toolbar.InsertImage.IsChecked == false && window.Toolbar.InsertImage.IsEnabled,
                "Right past a selected image restores the ordinary Insert caret and insertion toolbar action");
            view.Key(VIEM_KEY_LEFT);
            selection = view.LogicalSelection();
            Check(view.HasSelection && selection.text_start == 1 && selection.text_end == 4 && pane.CaretIsOnInlineImage(),
                "Left onto an image uses its complete selection to suppress the thin caret at either active endpoint");
            view.Text("X");
            Check(document.FormattedText() == "AXB", "typing replaces the keyboard-selected image as one atomic object");
            view.Undo();
            Check(Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                "undo restores the image's original source after keyboard replacement");
            view.Key(VIEM_KEY_ESCAPE); view.Command("gg0i"); view.Key(VIEM_KEY_RIGHT); view.Key(VIEM_KEY_RIGHT);
            Check(window.Toolbar.InsertImage.Focus(FocusState.Programmatic), "the image insertion button accepts native keyboard focus");
            await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space); await Task.Delay(40);
            Check(pane.ImageEditorVisible && pane.ImageTextValue == "" && pane.ImageDestinationValue == "",
                "Insert image opens an empty insertion form beside an existing image");
            pane.ImageDestinationValue = "new.png"; pane.ApplyImageEditor();
            Check(document.FormattedText() == "A\uFFFC\uFFFCB", "Insert image creates a new object at the caret without editing the adjacent image");
            view.Key(VIEM_KEY_ESCAPE); view.Undo();
            Check(Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                "inserting beside an image restores exact original source on undo");
        }
        finally { await window.ClosePane(pane, force: true); App.Instance.Windows.Remove(window); }
    }

    private static async Task LocalPreview(Preferences preferences)
    {
        string directory = Path.Combine(Path.GetTempPath(), "viem-image-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        string path = Path.Combine(directory, "wide.bmp");
        // A deterministic raster fixture needs no system asset or network.
        WriteBitmap(path, 1600, 800);
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
            var viewport = view.Viewport;
            pane.SelectInlineImage(new Point(narrower.typographic_bounds.x + 2 - viewport.left, narrower.typographic_bounds.y + 2 - viewport.top));
            Check(pane.ImageReloadVisible && pane.ImageReloadEnabled, "a selected local image exposes Reload immediately before Delete");
            async Task WaitForLoads(int attempts)
            {
                for (int i = 0; i < 200 && (pane.ImagePreviewLoadAttempts < attempts || pane.PendingImageLoads != 0); i++) await Task.Delay(25);
                Check(pane.ImagePreviewLoadAttempts >= attempts && pane.PendingImageLoads == 0, "the requested local image reload completes");
            }
            int reloadAttempt = pane.ImagePreviewLoadAttempts + 1;
            WriteBitmap(path, 80, 120); pane.ReloadShownImage(); await WaitForLoads(reloadAttempt);
            var reloaded = view.Layout().Clusters.Single(c => view.Provider.IsInlineImage(c.render_run));
            Check(Math.Abs(reloaded.typographic_bounds.width - 80 * view.Viewport.scale) < .01
                && Math.Abs(reloaded.typographic_bounds.height - 120 * view.Viewport.scale) < .01,
                "Reload decodes changed file bytes and relayouts the image at its new intrinsic size");
            float stableHeight = view.Layout().Info.total_height, stableTop = view.Viewport.top;
            ulong stableGeneration = view.Provider.Generation;
            pane.EvictImageBitmapsForTest();
            Check(view.Provider.Generation == stableGeneration, "evicting raster textures preserves independent intrinsic dimensions");
            pane.Refresh();
            var immediate = view.Layout().Clusters.Single(c => view.Provider.IsInlineImage(c.render_run));
            Check(immediate.typographic_bounds.width == reloaded.typographic_bounds.width && immediate.typographic_bounds.height == reloaded.typographic_bounds.height
                && Math.Abs(view.Layout().Info.total_height - stableHeight) < .01 && Math.Abs(view.Viewport.top - stableTop) < .01,
                "revisiting an evicted image synchronously reuses known geometry without scrolling or extent changes");
            await WaitForLoads(reloadAttempt + 1);
            Check(Math.Abs(view.Layout().Info.total_height - stableHeight) < .01 && Math.Abs(view.Viewport.top - stableTop) < .01,
                "a bitmap-only resource refresh preserves measured document extents");
            File.Delete(path); pane.ReloadShownImage(); await WaitForLoads(reloadAttempt + 2);
            Check(pane.ImagePreviewIsBroken("wide.bmp") && pane.LocalImagePreviewCount == 0,
                "reloading a missing local image replaces its raster with the broken-image icon and URL placeholder");
            WriteBitmap(path, 40, 40); pane.ReloadShownImage(); await WaitForLoads(reloadAttempt + 3);
            Check(!pane.ImagePreviewIsBroken("wide.bmp") && pane.LocalImagePreviewCount == 1,
                "Reload recovers a repaired local image after a failed preview");
            Check(!document.IsDirty && Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                "local image decoding, reloading, and cache eviction never modify document source or dirty state");
        }
        finally
        {
            await window.ClosePane(pane, force: true); App.Instance.Windows.Remove(window);
            Directory.Delete(directory, recursive: true);
        }
        Check(pane.LocalImagePreviewCount == 0 && pane.PendingImageLoads == 0, "closing a view releases decoded images and cancels its preview work");
    }

    private static async Task HtmlImageDimensions(Preferences preferences)
    {
        string directory = Path.Combine(Path.GetTempPath(), "viem-html-image-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        WriteBitmap(Path.Combine(directory, "sized.bmp"), 120, 60);
        try
        {
            foreach (var (attributes, width, height) in new[] {
                ("width='240'", 240f, 120f),
                ("height='160'", 320f, 160f),
                ("width='80' height='180'", 80f, 180f),
                ("width='1024' height='2048'", 512f, 1024f),
            })
            {
                string source = "<img src='sized.bmp' " + attributes + " alt='preserved' title='untouched'>";
                var document = new CoreDocument(Encoding.UTF8.GetBytes(source), Path.Combine(directory, "source.md"), VIEM_FORMAT_MARKDOWN);
                var window = new EditorWindow(preferences, document);
                App.Instance.Windows.Add(window); window.Activate();
                var pane = window.ActivePane!; var view = await pane.Ready;
                try
                {
                    view.Zoom(1);
                    for (int i = 0; i < 200 && (pane.LocalImagePreviewCount == 0 || pane.PendingImageLoads != 0); i++) await Task.Delay(25);
                    Check(pane.LocalImagePreviewCount == 1 && pane.PendingImageLoads == 0,
                        "an HTML image uses the existing relative local raster loader: " + attributes);
                    view.Resize(650, 500);
                    var image = view.Layout().Clusters.Single(c => view.Provider.IsInlineImage(c.render_run));
                    Check(document.FormattedText() == "\uFFFC" && Math.Abs(image.typographic_bounds.width - width) < .01
                        && Math.Abs(image.typographic_bounds.height - height) < .01,
                        "HTML image dimensions scale, preserve one-sided proportions, and obey the display cap: " + attributes);
                    Check(view.Provider.ImageDestination(image.render_run) == "sized.bmp",
                        "HTML images expose the decoded source to existing native image controls");
                    view.SetMarkdownSource(true);
                    Check(!document.IsDirty && Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                        "HTML dimensions and unrelated attributes survive native image loading and Source view: " + attributes);
                }
                finally { await window.ClosePane(pane, force: true); App.Instance.Windows.Remove(window); }
            }
        }
        finally { Directory.Delete(directory, recursive: true); }
    }

    private static void WriteBitmap(string path, int width, int height)
    {
        int stride = (width * 3 + 3) & ~3, bytes = stride * height;
        using var file = File.Create(path); using var writer = new BinaryWriter(file);
        writer.Write((ushort)0x4D42); writer.Write(54 + bytes); writer.Write(0); writer.Write(54);
        writer.Write(40); writer.Write(width); writer.Write(height); writer.Write((ushort)1); writer.Write((ushort)24);
        writer.Write(0); writer.Write(bytes); writer.Write(0); writer.Write(0); writer.Write(0); writer.Write(0);
        writer.Write(new byte[bytes]);
    }

    private static async Task PreviewLimits(Preferences preferences)
    {
        string directory = Path.Combine(Path.GetTempPath(), "viem-image-limits-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        string path = Path.Combine(directory, "limit.bmp");
        WriteBitmap(path, 5001, 1);
        const string source = "![](limit.bmp)";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), Path.Combine(directory, "source.md"), VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        var pane = window.ActivePane!; var view = await pane.Ready;
        try
        {
            async Task WaitForLoads(int attempts)
            {
                for (int i = 0; i < 200 && (pane.ImagePreviewLoadAttempts < attempts || pane.PendingImageLoads != 0); i++) await Task.Delay(25);
                Check(pane.ImagePreviewLoadAttempts == attempts && pane.PendingImageLoads == 0, "the image admission-limit fixture finishes its requested decode");
            }
            async Task Reload()
            {
                view.Place(0, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
                int attempts = pane.ImagePreviewLoadAttempts + 1;
                pane.ReloadShownImage(); await WaitForLoads(attempts);
            }
            void CheckLimited(string name) => Check(pane.ImagePreviewIsLimited("limit.bmp")
                && !pane.ImagePreviewIsBroken("limit.bmp") && pane.LocalImagePreviewCount == 0, name);
            void CheckReady(string name) => Check(!pane.ImagePreviewIsLimited("limit.bmp")
                && !pane.ImagePreviewIsBroken("limit.bmp") && pane.LocalImagePreviewCount == 1, name);
            void PadFile(long bytes)
            {
                using var file = new FileStream(path, FileMode.Open, FileAccess.Write);
                file.SetLength(bytes); file.Position = 2;
                using var writer = new BinaryWriter(file); writer.Write(checked((int)bytes));
            }

            await WaitForLoads(1);
            CheckLimited("a 5001-pixel-wide local image displays the URL without a broken-image icon");
            int attempts = pane.ImagePreviewLoadAttempts;
            pane.EvictImageBitmapsForTest();
            for (int i = 0; i < 10; i++) pane.Refresh();
            await Task.Delay(100);
            Check(pane.ImagePreviewLoadAttempts == attempts && pane.ImagePreviewIsLimited("limit.bmp"),
                "an image rejected by size limits stays terminal after bitmap eviction and passive refresh");

            WriteBitmap(path, 5000, 1); await Reload();
            CheckReady("Reload accepts a replacement whose width is exactly 5000 pixels");
            var wide = view.Layout().Clusters.Single(c => view.Provider.IsInlineImage(c.render_run));
            Check(wide.typographic_bounds.width <= 1024.01 && wide.typographic_bounds.height <= 1024.01
                && Math.Abs(wide.typographic_bounds.height / wide.typographic_bounds.width - 1.0 / 5000) < .00001,
                "accepted wide images fit the 1024-DIP display limit while preserving aspect ratio");
            WriteBitmap(path, 1, 5001); await Reload();
            CheckLimited("a 5001-pixel-high local image displays the URL without a broken-image icon");
            WriteBitmap(path, 1, 5000); await Reload();
            CheckReady("Reload accepts a replacement whose height is exactly 5000 pixels");
            var tall = view.Layout().Clusters.Single(c => view.Provider.IsInlineImage(c.render_run));
            Check(tall.typographic_bounds.width <= 1024.01 && tall.typographic_bounds.height <= 1024.01
                && Math.Abs(tall.typographic_bounds.width / tall.typographic_bounds.height - 1.0 / 5000) < .00001,
                "accepted tall images fit the 1024-DIP display limit while preserving aspect ratio");

            WriteBitmap(path, 1, 1); PadFile(10_000_000); await Reload();
            CheckReady("a local raster of exactly 10,000,000 bytes is admitted");
            PadFile(10_000_001); await Reload();
            CheckLimited("a local file above 10,000,000 bytes displays the URL without a broken-image icon");
            Check(await LocalImagePreview.CanOpenFile(path), "preview limits do not block an explicit Open action on a local raster");
            File.WriteAllText(path, "this is not an image"); await Reload();
            Check(pane.ImagePreviewIsBroken("limit.bmp") && !pane.ImagePreviewIsLimited("limit.bmp") && pane.LocalImagePreviewCount == 0,
                "a corrupt image remains distinct from size-limited previews and displays the broken-image icon");
            WriteBitmap(path, 40, 40); await Reload();
            CheckReady("Reload clears a terminal placeholder after replacing the file with a supported local raster");
            Check(!document.IsDirty && Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                "image admission limits and explicit reloads preserve document source and dirty state");
        }
        finally
        {
            await window.ClosePane(pane, force: true); App.Instance.Windows.Remove(window);
            Directory.Delete(directory, recursive: true);
        }
    }

    private static async Task ScrollDuringImageReload(Preferences preferences)
    {
        string directory = Path.Combine(Path.GetTempPath(), "viem-image-scroll-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        string path = Path.Combine(directory, "scroll.bmp");
        WriteBitmap(path, 200, 200);
        string source = "caret stays here\n\n![scroll](scroll.bmp)\n\n"
            + string.Join("\n\n", Enumerable.Repeat("Paragraphs keep the wheel viewport away from the document end.", 100));
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), Path.Combine(directory, "source.md"), VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        var pane = window.ActivePane!; var view = await pane.Ready;
        try
        {
            for (int i = 0; i < 200 && (pane.ImagePreviewLoadAttempts == 0 || pane.PendingImageLoads != 0); i++) await Task.Delay(25);
            Check(pane.LocalImagePreviewCount == 1, "the scroll-anchor fixture loads its initial local image");
            var image = view.Layout().Clusters.Single(c => view.Provider.IsInlineImage(c.render_run));
            view.Place(image.text_start, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            WriteBitmap(path, 200, 1000);
            int attempts = pane.ImagePreviewLoadAttempts + 1;
            pane.ReloadShownImage();
            // Move and scroll synchronously before allowing the asynchronous
            // decoder to publish. Its later Resize fallback must retain the
            // wheel viewport rather than reveal this now-offscreen caret.
            view.Place(0, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            view.Scroll(0, image.typographic_bounds.y + 40);
            float top = view.Viewport.top;
            Check(top > image.typographic_bounds.y && view.Presentation.cursor_utf8_offset == 0,
                "the image reload fixture has a wheel viewport inside the image and an offscreen caret");
            for (int i = 0; i < 200 && (pane.ImagePreviewLoadAttempts < attempts || pane.PendingImageLoads != 0); i++) await Task.Delay(25);
            Check(pane.ImagePreviewLoadAttempts == attempts && pane.PendingImageLoads == 0 && pane.LastError == null,
                "the image reload publishes through native resource refresh and layout recovery");
            Check(Math.Abs(view.Viewport.top - top) < .01 && view.Presentation.cursor_utf8_offset == 0,
                "asynchronous image growth preserves the wheel viewport without revealing an offscreen caret");
        }
        finally
        {
            await window.ClosePane(pane, force: true); App.Instance.Windows.Remove(window);
            Directory.Delete(directory, recursive: true);
        }
    }

    private static async Task ImageStyle(Preferences preferences)
    {
        const string source = "![alt](https://example.invalid/style.png)";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        var pane = window.ActivePane!; var view = await pane.Ready;
        try
        {
            await Task.Delay(100);
            var style = view.Styles().Styles.Single(s => s.Id == "Image");
            view.EditStyle(style, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_SIZE, CoreView.Number(23));
            var cluster = view.Layout().Clusters.Single(c => view.Provider.IsInlineImage(c.render_run));
            Check(Math.Abs(view.Provider.ImageLocationFontSize(cluster.render_run) - 23 * view.Viewport.scale) < .01,
                "image URL labels inherit the Image style's resolved font rather than a fixed system size");
            view.SetMarkdownSource(true);
            var sourceCluster = view.Layout().Clusters.First(c => c.text_start == 0);
            Check(Math.Abs(view.Provider.ImageLocationFontSize(sourceCluster.render_run) - 23 * view.Viewport.scale) < .01,
                "Source image notation uses the same Image style font");
            Check(Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                "Image style presentation retains the authored source spelling");
        }
        finally { await window.ClosePane(pane, force: true); App.Instance.Windows.Remove(window); }
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
            var pane = window.ActivePane!;
            if (budget is long limit) pane.SetImagePreviewByteBudgetForTest(limit);
            var view = await pane.Ready;
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

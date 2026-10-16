using Microsoft.Graphics.Canvas;
using Viem.Windows.Rendering;
using Viem.Windows.Core;
using Viem.Windows.Shell;
using Windows.Foundation;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    private sealed class ImagePreviewEntry
    {
        internal LocalImagePreview? Preview;
        internal bool Complete, Loading, ReloadRequested;
        internal CancellationTokenSource? Cancellation;
        internal long LastUse;
    }
    private readonly Dictionary<string, ImagePreviewEntry> imagePreviews = new(StringComparer.Ordinal);
    private CancellationTokenSource imageCancellation = new();
    private string? imageDocumentPath;
    private long imagePreviewClock;
    private int imageLoads;
    private long imagePreviewGeneration;
    private const int MaximumImageEntries = 48;
    private const long MaximumImageBytes = 64 * 1024 * 1024;
    private long imagePreviewByteBudget = MaximumImageBytes;
    // Bitmaps resampled to the device size they are drawn at, so scrolling
    // draws them nearly 1:1 instead of resampling a large raster every frame.
    private sealed record ScaledImage(CanvasBitmap Source, uint Width, uint Height, CanvasRenderTarget Image, long LastUse);
    private readonly Dictionary<string, ScaledImage> scaledImages = new(StringComparer.Ordinal);
    private readonly Dictionary<string, (CanvasBitmap Source, uint Width, uint Height)> scalingImages = new(StringComparer.Ordinal);
    private long scaledImageBytes;
    private const int MaximumScaledImages = 16;
    private const long MaximumScaledImageBytes = 48 * 1024 * 1024;
#if DEBUG
    internal int ImagePreviewLoadAttempts { get; private set; }
    internal int ImagePreviewEntries => imagePreviews.Count;
    internal int CompletedImagePreviewEntries => imagePreviews.Values.Count(e => e.Complete);
    internal long ImagePreviewBytes => imagePreviews.Values.Sum(e => e.Preview?.DecodedBytes ?? 0);
    internal int ScaledImageRenders { get; private set; }
    internal int ScaledImageCount => scaledImages.Count;
    internal long ScaledImageBytes => scaledImageBytes;
    internal bool ImageResamplePending => scalingImages.Count != 0;
    internal void ClearScaledImagesForTest() => ClearScaledImages();
    internal bool HasScaledImageForTest(string destination) => scaledImages.ContainsKey(destination);
    internal bool ImageResamplePendingForTest(string destination) => scalingImages.ContainsKey(destination);
    internal string ImageDrawDiagnosticForTesting(string destination)
    {
        if (View == null || snapshot == null) return "no view";
        var cluster = snapshot.Clusters.FirstOrDefault(c => View.Provider.ImageDestination(c.render_run) == destination);
        if (cluster.render_run.owner == 0 && cluster.text_end == 0) return $"no cluster (clusters={snapshot.Clusters.Length})";
        var bounds = OffsetRect(cluster.typographic_bounds, viewport);
        imagePreviews.TryGetValue(destination, out var image);
        return $"bounds={bounds} canvas={Canvas.ActualWidth}x{Canvas.ActualHeight} dpi={Canvas.Dpi} layout={View.Provider.ImageLayoutSize(cluster.render_run)} "
            + $"preview={(image?.Preview is { } p ? $"{p.Width}x{p.Height}/{p.Bitmap.SizeInPixels.Width}x{p.Bitmap.SizeInPixels.Height}" : "none")} "
            + $"state={View.Provider.ImageState(destination)} scaling={scalingImages.Count} scaled={scaledImages.Count}";
    }
    internal void SetImagePreviewByteBudgetForTest(long bytes) => imagePreviewByteBudget = Math.Clamp(bytes, 0, MaximumImageBytes);
    internal void EvictImageBitmapsForTest() { ResetImagePreviews(forgetDimensions: false); Canvas.Invalidate(); }
    internal bool ImagePreviewIsBroken(string destination) => View?.Provider.ImageState(destination) == InlineImageState.Broken;
    internal bool ImagePreviewIsLimited(string destination) => View?.Provider.ImageState(destination) == InlineImageState.Limited;
#endif
    internal int LocalImagePreviewCount => imagePreviews.Values.Count(e => e.Preview != null);
    internal int PendingImageLoads => imageLoads;

    private void ResetImagePreviews(bool forgetDimensions = true)
    {
        bool pathChanged = !StringComparer.OrdinalIgnoreCase.Equals(imageDocumentPath, Document.FilePath);
        imagePreviewGeneration++;
        imageCancellation.Cancel(); imageCancellation.Dispose(); imageCancellation = new();
        foreach (var entry in imagePreviews.Values) { entry.Preview?.Dispose(); entry.Cancellation?.Dispose(); }
        imagePreviews.Clear(); imageLoads = 0;
        ClearScaledImages();
        imageDocumentPath = Document.FilePath;
        // A new base path forgets the provider's knowledge (a no-op when the
        // view was created with it). Otherwise forget only when requested.
        if (pathChanged) View?.Provider.SetImageDocumentPath(Document.FilePath);
        else if (forgetDimensions) View?.Provider.ClearImageDimensions();
    }

    private void ClearScaledImages()
    {
        foreach (var scaled in scaledImages.Values) scaled.Image.Dispose();
        scaledImages.Clear(); scalingImages.Clear(); scaledImageBytes = 0;
    }

    private void RefreshImagePreviews()
    {
        if (View == null || snapshot == null) return;
        // Admission covers the visible screen and one screen beyond each edge,
        // so nearby bitmaps are ready before scrolling reveals them.
        double height = Canvas.ActualHeight, width = Math.Max(Canvas.ActualWidth, PaintViewportWidth);
        var nearby = snapshot.Clusters
            .Where(c => c.typographic_bounds.y + c.typographic_bounds.height > viewport.top - height
                && c.typographic_bounds.y < viewport.top + 2 * height
                && c.typographic_bounds.x + c.typographic_bounds.width > viewport.left - width
                && c.typographic_bounds.x < viewport.left + 2 * width).ToArray();
        // The selected object takes priority even in a viewport containing
        // more tiny images than the bitmap cache can admit.
        string? selected = nearby.Where(c => c.text_start <= presentation.cursor_utf8_offset && c.text_end > presentation.cursor_utf8_offset
            || snapshot.SelectionSegments.Length == 1 && snapshot.SelectionSegments[0].text_start == c.text_start && snapshot.SelectionSegments[0].text_end == c.text_end)
            .Select(c => View.Provider.ImageDestination(c.render_run)).FirstOrDefault(d => d != null);
        // Visible images load before those only nearby.
        var wanted = nearby.OrderBy(c => c.typographic_bounds.y + c.typographic_bounds.height <= viewport.top
                || c.typographic_bounds.y >= viewport.top + height ? 1 : 0)
            .Select(c => View.Provider.ImageDestination(c.render_run)).OfType<string>()
            .Prepend(selected ?? "").Where(d => d.Length != 0).Distinct(StringComparer.Ordinal)
            .Take(MaximumImageEntries).ToList();
        var admitted = wanted.ToHashSet(StringComparer.Ordinal);
        foreach (var pending in imagePreviews.Where(e => e.Value.Loading && !admitted.Contains(e.Key)).ToArray())
        {
            pending.Value.ReloadRequested = false;
            pending.Value.Cancellation?.Cancel();
        }
        foreach (string destination in wanted)
        {
            if (!imagePreviews.TryGetValue(destination, out var entry))
            {
                while (imagePreviews.Count >= MaximumImageEntries)
                {
                    var oldest = imagePreviews.Where(e => e.Value.Complete && !admitted.Contains(e.Key)).MinBy(e => e.Value.LastUse);
                    if (oldest.Key == null) break;
                    oldest.Value.Preview?.Dispose(); oldest.Value.Cancellation?.Dispose(); imagePreviews.Remove(oldest.Key);
                }
                if (imagePreviews.Count >= MaximumImageEntries) continue;
                entry = new ImagePreviewEntry(); imagePreviews[destination] = entry;
            }
            entry.LastUse = ++imagePreviewClock;
            if (entry.Complete || entry.Loading) continue;
            if (ImageLocation.LocalPreviewPath(destination, Document.FilePath) == null)
            {
                entry.Complete = true;
                if (!ImageLocation.IsWeb(destination)) View.Provider.RecordImage(destination, 300, 64, InlineImageState.Broken);
                continue; // No remote destination is ever submitted to a loader.
            }
            if (!View.Provider.AdmitImage(destination)
                || View.Provider.ImageState(destination) is InlineImageState.Broken or InlineImageState.Limited && !entry.ReloadRequested)
            { entry.Complete = true; continue; }
            if (imageLoads >= 2) continue;
            entry.Loading = true; entry.ReloadRequested = false;
            entry.Cancellation = CancellationTokenSource.CreateLinkedTokenSource(imageCancellation.Token);
            imageLoads++;
#if DEBUG
            ImagePreviewLoadAttempts++;
#endif
            _ = LoadImagePreview(destination, entry, imagePreviewGeneration, View, Document.FilePath, entry.Cancellation.Token);
        }
    }

    internal void ReloadShownImage()
    {
        if (View is not { } view || Document.State.format != VIEM_FORMAT_MARKDOWN || shownImage is not { Image: { } image } context) return;
        view.ValidateImageContext(context);
        if (ImageLocation.LocalPreviewPath(image.Destination, Document.FilePath) == null) return;
        if (!view.Provider.AdmitImage(image.Destination)) { SetMessage("The image preview budget is full."); return; }
        if (imagePreviews.TryGetValue(image.Destination, out var entry))
        {
            entry.Complete = false; entry.ReloadRequested = true;
            if (entry.Loading) entry.Cancellation?.Cancel();
        }
        else
        {
            if (imagePreviews.Count >= MaximumImageEntries)
            {
                var oldest = imagePreviews.Where(e => e.Value.Complete).MinBy(e => e.Value.LastUse);
                if (oldest.Key == null) return;
                oldest.Value.Preview?.Dispose(); oldest.Value.Cancellation?.Dispose(); imagePreviews.Remove(oldest.Key);
            }
            imagePreviews[image.Destination] = new() { ReloadRequested = true };
        }
        RefreshImagePreviews(); Canvas.Invalidate();
    }

    private async Task LoadImagePreview(string destination, ImagePreviewEntry entry, long generation, CoreView owner,
        string? documentPath, CancellationToken cancellation)
    {
        var result = await LocalImagePreview.Load(Canvas.Device, destination, documentPath, cancellation);
        LocalImagePreview? decoded = result.Preview;
        if (disposed || generation != imagePreviewGeneration || View != owner
            || !StringComparer.OrdinalIgnoreCase.Equals(Document.FilePath, documentPath)
            || !imagePreviews.TryGetValue(destination, out var current) || !ReferenceEquals(entry, current))
        { decoded?.Dispose(); return; }
        imageLoads--; entry.Loading = false; entry.Cancellation?.Dispose(); entry.Cancellation = null;
        if (cancellation.IsCancellationRequested)
        {
            decoded?.Dispose();
            if (entry.ReloadRequested) entry.Complete = false;
            else { entry.Preview?.Dispose(); imagePreviews.Remove(destination); }
            RefreshImagePreviews(); return;
        }
        entry.Complete = true;
        // Record dimensions even if this image left the admitted region while
        // loading. Only geometry layout already used requires reshaping.
        ulong previousGeneration = owner.Provider.Generation;
        bool geometryChanged = owner.Provider.RecordImage(destination, decoded?.Width ?? 300, decoded?.Height ?? 64, result.Status switch {
            LocalImagePreviewStatus.Ready => InlineImageState.Ready,
            LocalImagePreviewStatus.Limited => InlineImageState.Limited,
            _ => InlineImageState.Broken });
        bool stillPresent = snapshot?.Clusters.Any(c => owner.Provider.ImageDestination(c.render_run) == destination) == true;
        entry.Preview?.Dispose(); entry.Preview = null;
        if (decoded != null && stillPresent)
        {
            long used = imagePreviews.Values.Sum(e => e.Preview?.DecodedBytes ?? 0);
            foreach (var stale in imagePreviews.Where(e => e.Value.Complete && e.Key != destination).OrderBy(e => e.Value.LastUse).ToArray())
            {
                if (used + decoded.DecodedBytes <= imagePreviewByteBudget) break;
                if (snapshot!.Clusters.Any(c => owner.Provider.ImageDestination(c.render_run) == stale.Key)) continue;
                used -= stale.Value.Preview?.DecodedBytes ?? 0;
                stale.Value.Preview?.Dispose(); stale.Value.Cancellation?.Dispose(); imagePreviews.Remove(stale.Key);
            }
            if (used + decoded.DecodedBytes <= imagePreviewByteBudget) entry.Preview = decoded;
            else decoded.Dispose(); // A completed placeholder cannot trigger a budget eviction loop.
        }
        else
        {
            decoded?.Dispose();
            if (!stillPresent) imagePreviews.Remove(destination);
        }
        if (geometryChanged)
        {
            owner.Provider.InvalidateMetrics();
            Run(() => owner.ImageResourcesChanged(previousGeneration, [destination]));
        }
        RefreshImagePreviews(); Canvas.Invalidate();
    }

    /// The bitmap to draw for `preview` at `bounds`: a copy resampled to its
    /// device size once ready, and the original until then or when it is
    /// already near that size. Resampling runs on a background thread.
    private ICanvasImage DisplayBitmap(string destination, LocalImagePreview preview, Rect bounds)
    {
        var source = preview.Bitmap; var pixels = source.SizeInPixels;
        double scale = Canvas.Dpi / 96;
        uint width = (uint)Math.Max(0, Math.Round(bounds.Width * scale)), height = (uint)Math.Max(0, Math.Round(bounds.Height * scale));
        bool differs = Math.Abs((long)width - pixels.Width) * 5 > pixels.Width || Math.Abs((long)height - pixels.Height) * 5 > pixels.Height;
        if (width == 0 || height == 0 || !differs || (long)width * height * 4 > MaximumScaledImageBytes / 4) return source;
        if (scaledImages.TryGetValue(destination, out var scaled) && ReferenceEquals(scaled.Source, source)
            && scaled.Width == width && scaled.Height == height)
        {
            scaledImages[destination] = scaled with { LastUse = ++imagePreviewClock };
            return scaled.Image;
        }
        if (scalingImages.TryGetValue(destination, out var pending) && ReferenceEquals(pending.Source, source)
            && pending.Width == width && pending.Height == height) return source;
        scalingImages[destination] = (source, width, height);
        var device = Canvas.Device; long generation = imagePreviewGeneration;
        _ = Task.Run(() => {
            try
            {
                var target = new CanvasRenderTarget(device, width, height, 96);
                using (var drawing = target.CreateDrawingSession())
                {
                    drawing.Clear(Microsoft.UI.Colors.Transparent);
                    drawing.DrawImage(source, new Rect(0, 0, width, height), new Rect(0, 0, pixels.Width, pixels.Height), 1,
                        CanvasImageInterpolation.HighQualityCubic);
                }
                return target;
            }
            catch (Exception error) when (error is ObjectDisposedException or System.Runtime.InteropServices.COMException) { return null; }
        }).ContinueWith(task => DispatcherQueue.TryEnqueue(() => FinishResample(destination, source, width, height, task.Result, generation)),
            TaskScheduler.Default);
        return source;
    }

    private void FinishResample(string destination, CanvasBitmap source, uint width, uint height, CanvasRenderTarget? image, long generation)
    {
        if (scalingImages.TryGetValue(destination, out var pending) && ReferenceEquals(pending.Source, source)
            && pending.Width == width && pending.Height == height) scalingImages.Remove(destination);
        if (image == null) return;
        if (disposed || generation != imagePreviewGeneration
            || !imagePreviews.TryGetValue(destination, out var entry) || !ReferenceEquals(entry.Preview?.Bitmap, source))
        { image.Dispose(); return; }
        long cost = (long)width * height * 4;
        if (scaledImages.Remove(destination, out var previous)) { scaledImageBytes -= (long)previous.Width * previous.Height * 4; previous.Image.Dispose(); }
        while (scaledImages.Count >= MaximumScaledImages || scaledImageBytes + cost > MaximumScaledImageBytes)
        {
            var oldest = scaledImages.MinBy(e => e.Value.LastUse);
            if (oldest.Key == null) break;
            scaledImages.Remove(oldest.Key); scaledImageBytes -= (long)oldest.Value.Width * oldest.Value.Height * 4; oldest.Value.Image.Dispose();
        }
        if (scaledImageBytes + cost > MaximumScaledImageBytes) { image.Dispose(); return; }
        scaledImages[destination] = new(source, width, height, image, ++imagePreviewClock); scaledImageBytes += cost;
#if DEBUG
        ScaledImageRenders++;
#endif
        Canvas.Invalidate();
    }

    private void DrawInlineImages(CanvasDrawingSession drawing)
    {
        if (View == null || snapshot == null) return;
        foreach (var cluster in snapshot.Clusters)
        {
            string? destination = View.Provider.ImageDestination(cluster.render_run);
            if (destination == null) continue;
            var bounds = OffsetRect(cluster.typographic_bounds, viewport);
            if (bounds.Bottom < 0 || bounds.Top > Canvas.ActualHeight || bounds.Right < 0 || bounds.Left > PaintViewportWidth) continue;
            // Draw a bitmap only while it has the dimensions layout used; a
            // changed image keeps its placeholder until layout catches up.
            if (imagePreviews.TryGetValue(destination, out var image) && image.Preview is { } preview
                && View.Provider.ImageLayoutSize(cluster.render_run) == (preview.Width, preview.Height))
            {
                var display = DisplayBitmap(destination, preview, bounds);
                var size = display is CanvasBitmap bitmap ? bitmap.Size : new Size(0, 0);
                drawing.DrawImage(display, bounds, new Rect(0, 0, size.Width, size.Height));
            }
            else
            {
                var paint = PaintFor(cluster.text_start);
                var foreground = (paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) != 0 ? preferences.Theme.Foreground : Color(paint.foreground);
                bool broken = View.Provider.ImageState(destination) == InlineImageState.Broken;
                drawing.FillRectangle(bounds, (paint.flags & VIEM_TEXT_PAINT_HAS_BACKGROUND) != 0 ? Color(paint.background) : preferences.Theme.StatusBackground);
                var outline = foreground; outline.A = (byte)Math.Round(outline.A * .5);
                drawing.DrawRectangle(new Rect(bounds.X + .5, bounds.Y + .5, Math.Max(0, bounds.Width - 1), Math.Max(0, bounds.Height - 1)), outline, 1);
                using var clip = drawing.CreateLayer(1, bounds);
                double inset = 8 * viewport.scale, iconWidth = broken ? 24 * viewport.scale : 0;
                if (broken) DrawBrokenImageIcon(drawing, new Rect(bounds.X + inset, bounds.Y + (bounds.Height - 18 * viewport.scale) / 2, 18 * viewport.scale, 18 * viewport.scale), foreground);
                View.Provider.DrawImageLocation(drawing, cluster.render_run, destination,
                    new Rect(bounds.X + inset + iconWidth, bounds.Y + 4 * viewport.scale, Math.Max(1, bounds.Width - inset * 2 - iconWidth), Math.Max(1, bounds.Height - 8 * viewport.scale)), foreground, paint.flags);
            }
            bool selected = !View.Composing && presentation.mode != VIEM_MODE_COMMAND_LINE
                && (presentation.caret_shape == VIEM_CARET_SHAPE_CELL && presentation.cursor_utf8_offset >= cluster.text_start && presentation.cursor_utf8_offset < cluster.text_end
                    || snapshot.SelectionSegments.Any(s => s.text_start <= cluster.text_start && s.text_end >= cluster.text_end));
            if (selected)
                drawing.DrawRectangle(new Rect(bounds.X + .5, bounds.Y + .5, Math.Max(0, bounds.Width - 1), Math.Max(0, bounds.Height - 1)), preferences.Theme.Caret, 1);
        }
    }

    private static void DrawBrokenImageIcon(CanvasDrawingSession drawing, Rect bounds, global::Windows.UI.Color color)
    {
        float X(double x) => (float)(bounds.X + x * bounds.Width / 18);
        float Y(double y) => (float)(bounds.Y + y * bounds.Height / 18);
        float stroke = (float)Math.Max(1, bounds.Width / 18);
        // A simple native-weight missing-photo pictogram, kept passive vector
        // furniture rather than another resource that itself needs loading.
        drawing.DrawRectangle(new Rect(X(1), Y(2), bounds.Width * 16 / 18, bounds.Height * 14 / 18), color, stroke);
        drawing.DrawCircle(X(5), Y(6), (float)(bounds.Width / 18), color, stroke);
        drawing.DrawLine(X(2), Y(13), X(7), Y(9), color, stroke);
        drawing.DrawLine(X(7), Y(9), X(10), Y(12), color, stroke);
        drawing.DrawLine(X(10), Y(12), X(14), Y(8), color, stroke);
        drawing.DrawLine(X(14), Y(8), X(16), Y(10), color, stroke);
        drawing.DrawLine(X(1), Y(17), X(17), Y(1), color, stroke);
    }

    internal bool CaretIsOnInlineImage()
    {
        if (View == null || snapshot == null) return false;
        foreach (var cluster in snapshot.Clusters)
        {
            if (!View.Provider.IsInlineImage(cluster.render_run)) continue;
            // A selected image may have either active endpoint, including its
            // trailing boundary. Its outline replaces the normal thin caret.
            if (snapshot.SelectionSegments.Length == 1 && snapshot.SelectionSegments[0].text_start == cluster.text_start
                && snapshot.SelectionSegments[0].text_end == cluster.text_end) return true;
            if (presentation.caret_shape == VIEM_CARET_SHAPE_CELL && cluster.text_start <= presentation.cursor_utf8_offset
                && cluster.text_end > presentation.cursor_utf8_offset) return true;
        }
        return false;
    }

    internal bool SelectInlineImage(Point point)
    {
        if (View == null || snapshot == null) return false;
        foreach (var cluster in snapshot.Clusters)
        {
            if (!View.Provider.IsInlineImage(cluster.render_run) || !OffsetRect(cluster.typographic_bounds, viewport).Contains(point)) continue;
            View.SelectImage(cluster.text_start, snapshot.Info.identity.document_revision);
            return true;
        }
        return false;
    }
}

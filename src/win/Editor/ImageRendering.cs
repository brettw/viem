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
    private sealed class ImageMetadata
    {
        internal uint Width = 300, Height = 64;
        internal LocalImagePreviewStatus Status;
    }
    private readonly Dictionary<string, ImagePreviewEntry> imagePreviews = new(StringComparer.Ordinal);
    // Intrinsic geometry is inexpensive document presentation state, separate
    // from the texture LRU. Never forget known sizes merely to evict a bitmap.
    private readonly Dictionary<string, ImageMetadata> imageMetadata = new(StringComparer.Ordinal);
    private long imageMetadataBytes;
    private CancellationTokenSource imageCancellation = new();
    private string? imageDocumentPath;
    private long imagePreviewClock;
    private int imageLoads;
    private long imagePreviewGeneration;
    private const int MaximumImageEntries = 48, MaximumImageMetadataEntries = 65536;
    private const long MaximumImageBytes = 64 * 1024 * 1024, MaximumImageMetadataBytes = 32 * 1024 * 1024;
    private long imagePreviewByteBudget = MaximumImageBytes;
#if DEBUG
    internal int ImagePreviewLoadAttempts { get; private set; }
    internal int ImagePreviewEntries => imagePreviews.Count;
    internal int CompletedImagePreviewEntries => imagePreviews.Values.Count(e => e.Complete);
    internal long ImagePreviewBytes => imagePreviews.Values.Sum(e => e.Preview?.DecodedBytes ?? 0);
    internal void SetImagePreviewByteBudgetForTest(long bytes) => imagePreviewByteBudget = Math.Clamp(bytes, 0, MaximumImageBytes);
    internal void EvictImageBitmapsForTest() { ResetImagePreviews(forgetDimensions: false); Canvas.Invalidate(); }
    internal bool ImagePreviewIsBroken(string destination) => imageMetadata.TryGetValue(destination, out var metadata) && metadata.Status == LocalImagePreviewStatus.Broken;
    internal bool ImagePreviewIsLimited(string destination) => imageMetadata.TryGetValue(destination, out var metadata) && metadata.Status == LocalImagePreviewStatus.Limited;
#endif
    internal int LocalImagePreviewCount => imagePreviews.Values.Count(e => e.Preview != null);
    internal int PendingImageLoads => imageLoads;

    private void ResetImagePreviews(bool forgetDimensions = true)
    {
        forgetDimensions |= !StringComparer.OrdinalIgnoreCase.Equals(imageDocumentPath, Document.FilePath);
        imagePreviewGeneration++;
        imageCancellation.Cancel(); imageCancellation.Dispose(); imageCancellation = new();
        foreach (var entry in imagePreviews.Values) { entry.Preview?.Dispose(); entry.Cancellation?.Dispose(); }
        imagePreviews.Clear(); imageLoads = 0;
        imageDocumentPath = Document.FilePath;
        if (forgetDimensions)
        {
            imageMetadata.Clear(); imageMetadataBytes = 0;
            View?.Provider.ClearImageDimensions();
        }
    }

    private ImageMetadata? AdmitImageMetadata(string destination)
    {
        if (imageMetadata.TryGetValue(destination, out var existing)) return existing;
        long bytes = (long)destination.Length * sizeof(char) + 128;
        // Stop admission instead of evicting dimensions already used by
        // document extents. This preserves stable geometry within finite work.
        if (imageMetadata.Count >= MaximumImageMetadataEntries || imageMetadataBytes + bytes > MaximumImageMetadataBytes) return null;
        var metadata = new ImageMetadata(); imageMetadata[destination] = metadata; imageMetadataBytes += bytes; return metadata;
    }

    private void RefreshImagePreviews()
    {
        if (View == null || snapshot == null) return;
        var visible = snapshot.Clusters
            .Where(c => c.typographic_bounds.y + c.typographic_bounds.height > viewport.top
                && c.typographic_bounds.y < viewport.top + Canvas.ActualHeight
                && c.typographic_bounds.x + c.typographic_bounds.width > viewport.left
                && c.typographic_bounds.x < viewport.left + Canvas.ActualWidth).ToArray();
        // The selected object takes priority even in a viewport containing
        // more tiny images than the bitmap cache can admit.
        string? selected = visible.Where(c => c.text_start <= presentation.cursor_utf8_offset && c.text_end > presentation.cursor_utf8_offset
            || snapshot.SelectionSegments.Length == 1 && snapshot.SelectionSegments[0].text_start == c.text_start && snapshot.SelectionSegments[0].text_end == c.text_end)
            .Select(c => View.Provider.ImageDestination(c.render_run)).FirstOrDefault(d => d != null);
        var wanted = visible.Select(c => View.Provider.ImageDestination(c.render_run)).OfType<string>()
            .Prepend(selected ?? "").Where(d => d.Length != 0).Distinct(StringComparer.Ordinal)
            .Take(MaximumImageEntries).ToHashSet(StringComparer.Ordinal);
        foreach (var pending in imagePreviews.Where(e => e.Value.Loading && !wanted.Contains(e.Key)).ToArray())
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
                    var oldest = imagePreviews.Where(e => e.Value.Complete && !wanted.Contains(e.Key)).MinBy(e => e.Value.LastUse);
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
                if (!ImageLocation.IsWeb(destination) && AdmitImageMetadata(destination) is { } unavailable) unavailable.Status = LocalImagePreviewStatus.Broken;
                continue; // No remote destination is ever submitted to a loader.
            }
            var metadata = AdmitImageMetadata(destination);
            if (metadata == null || (metadata.Status is LocalImagePreviewStatus.Broken or LocalImagePreviewStatus.Limited) && !entry.ReloadRequested)
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
        if (AdmitImageMetadata(image.Destination) == null) { SetMessage("The image preview budget is full."); return; }
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
        bool stillPresent = snapshot?.Clusters.Any(c => owner.Provider.ImageDestination(c.render_run) == destination) == true;
        if (!stillPresent) { decoded?.Dispose(); entry.Preview?.Dispose(); imagePreviews.Remove(destination); RefreshImagePreviews(); return; }
        var metadata = imageMetadata[destination];
        ulong previousGeneration = owner.Provider.Generation;
        uint width = decoded?.Width ?? 300, height = decoded?.Height ?? 64;
        bool dimensionsChanged = metadata.Width != width || metadata.Height != height;
        metadata.Width = width; metadata.Height = height; metadata.Status = result.Status;
        owner.Provider.SetImageDimensions(destination, width, height);
        entry.Preview?.Dispose(); entry.Preview = null;
        if (decoded != null)
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
        owner.Provider.InvalidateMetrics();
        Run(() => owner.ImageResourcesChanged(previousGeneration, dimensionsChanged ? [destination] : []));
        RefreshImagePreviews(); Canvas.Invalidate();
    }

    private void DrawInlineImages(CanvasDrawingSession drawing)
    {
        if (View == null || snapshot == null) return;
        foreach (var cluster in snapshot.Clusters)
        {
            string? destination = View.Provider.ImageDestination(cluster.render_run);
            if (destination == null) continue;
            var bounds = OffsetRect(cluster.typographic_bounds, viewport);
            if (bounds.Bottom < 0 || bounds.Top > Canvas.ActualHeight || bounds.Right < 0 || bounds.Left > Canvas.ActualWidth) continue;
            if (imagePreviews.TryGetValue(destination, out var image) && image.Preview is { } preview)
                drawing.DrawImage(preview.Bitmap, bounds, new Rect(0, 0, preview.Bitmap.Size.Width, preview.Bitmap.Size.Height));
            else
            {
                var paint = PaintFor(cluster.text_start);
                var foreground = (paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) != 0 ? preferences.Theme.Foreground : Color(paint.foreground);
                bool broken = imageMetadata.TryGetValue(destination, out var metadata) && metadata.Status == LocalImagePreviewStatus.Broken;
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

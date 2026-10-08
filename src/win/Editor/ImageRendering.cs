using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Text;
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
        internal bool Complete;
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
#if DEBUG
    internal int ImagePreviewLoadAttempts { get; private set; }
    internal int ImagePreviewEntries => imagePreviews.Count;
    internal int CompletedImagePreviewEntries => imagePreviews.Values.Count(e => e.Complete);
    internal long ImagePreviewBytes => imagePreviews.Values.Sum(e => e.Preview?.DecodedBytes ?? 0);
    internal void SetImagePreviewByteBudgetForTest(long bytes) => imagePreviewByteBudget = Math.Clamp(bytes, 0, MaximumImageBytes);
#endif
    internal int LocalImagePreviewCount => imagePreviews.Values.Count(e => e.Preview != null);
    internal int PendingImageLoads => imageLoads;

    private void ResetImagePreviews()
    {
        imagePreviewGeneration++;
        imageCancellation.Cancel(); imageCancellation.Dispose(); imageCancellation = new();
        foreach (var entry in imagePreviews.Values) entry.Preview?.Dispose();
        imagePreviews.Clear(); imageLoads = 0;
        imageDocumentPath = Document.FilePath;
        View?.Provider.ClearImageDimensions();
    }

    private void RefreshImagePreviews()
    {
        if (View == null || snapshot == null) return;
        var wanted = snapshot.Clusters
            .Where(c => c.typographic_bounds.y + c.typographic_bounds.height > viewport.top
                && c.typographic_bounds.y < viewport.top + Canvas.ActualHeight
                && c.typographic_bounds.x + c.typographic_bounds.width > viewport.left
                && c.typographic_bounds.x < viewport.left + Canvas.ActualWidth)
            .Select(c => View.Provider.ImageDestination(c.render_run)).OfType<string>().Distinct(StringComparer.Ordinal)
            .Take(MaximumImageEntries).ToHashSet(StringComparer.Ordinal);
        foreach (string destination in wanted)
        {
            if (imagePreviews.TryGetValue(destination, out var existing)) { existing.LastUse = ++imagePreviewClock; continue; }
            if (imageLoads >= 2) break;
            while (imagePreviews.Count >= MaximumImageEntries)
            {
                var oldest = imagePreviews.Where(e => e.Value.Complete && !wanted.Contains(e.Key)).MinBy(e => e.Value.LastUse);
                if (oldest.Key == null) break;
                oldest.Value.Preview?.Dispose(); imagePreviews.Remove(oldest.Key);
            }
            if (imagePreviews.Count >= MaximumImageEntries) break;
            var entry = new ImagePreviewEntry { LastUse = ++imagePreviewClock };
            imagePreviews[destination] = entry;
            // A remote destination goes straight to a completed placeholder;
            // it is never submitted to a decoder, HTTP client or file opener.
            if (ImageLocation.LocalPreviewPath(destination, Document.FilePath) == null) { entry.Complete = true; continue; }
            imageLoads++;
#if DEBUG
            ImagePreviewLoadAttempts++;
#endif
            _ = LoadImagePreview(destination, entry, imagePreviewGeneration, View, Document.FilePath, imageCancellation.Token);
        }
    }

    private async Task LoadImagePreview(string destination, ImagePreviewEntry entry, long generation, CoreView owner,
        string? documentPath, CancellationToken cancellation)
    {
        LocalImagePreview? decoded = await LocalImagePreview.Load(Canvas.Device, destination, documentPath, cancellation);
        if (disposed || cancellation.IsCancellationRequested || generation != imagePreviewGeneration || View != owner
            || !StringComparer.OrdinalIgnoreCase.Equals(Document.FilePath, documentPath)
            || !imagePreviews.TryGetValue(destination, out var current) || !ReferenceEquals(entry, current))
        { decoded?.Dispose(); return; }
        imageLoads--;
        entry.Complete = true;
        // Reuse only this exact destination/base-file dependency. An unrelated
        // text edit does not invalidate already decoded, unchanged local data.
        bool stillVisible = snapshot?.Clusters.Any(c => owner.Provider.ImageDestination(c.render_run) == destination) == true;
        if (!stillVisible) { decoded?.Dispose(); imagePreviews.Remove(destination); RefreshImagePreviews(); return; }
        if (decoded != null)
        {
            long used = imagePreviews.Values.Sum(e => e.Preview?.DecodedBytes ?? 0);
            foreach (var stale in imagePreviews.Where(e => e.Value.Complete && e.Key != destination)
                .OrderBy(e => e.Value.LastUse).ToArray())
            {
                if (used + decoded.DecodedBytes <= imagePreviewByteBudget) break;
                if (snapshot!.Clusters.Any(c => owner.Provider.ImageDestination(c.render_run) == stale.Key)) continue;
                used -= stale.Value.Preview?.DecodedBytes ?? 0;
                stale.Value.Preview?.Dispose(); imagePreviews.Remove(stale.Key);
            }
            if (used + decoded.DecodedBytes <= imagePreviewByteBudget)
            {
                entry.Preview = decoded;
                owner.Provider.UpdateImageDimensions(imagePreviews.Where(e => e.Value.Preview != null)
                    .ToDictionary(e => e.Key, e => (e.Value.Preview!.Width, e.Value.Preview!.Height), StringComparer.Ordinal));
                Run(() => owner.Resize((float)Canvas.ActualWidth, (float)Canvas.ActualHeight));
            }
            // Keep a completed placeholder when the visible working set
            // exceeds the budget. Repeated refreshes must not evict another
            // visible bitmap and decode these same candidates indefinitely.
            else decoded.Dispose();
        }
        // Successful, failed, and budget-rejected candidates all release a
        // slot and let the next visible request progress without user input.
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
                drawing.FillRectangle(bounds, preferences.Theme.StatusBackground);
                drawing.DrawRectangle(new Rect(bounds.X + .5, bounds.Y + .5, Math.Max(0, bounds.Width - 1), Math.Max(0, bounds.Height - 1)), preferences.Theme.StatusForeground, 1);
                using var format = new CanvasTextFormat { FontSize = 12 * viewport.scale, WordWrapping = CanvasWordWrapping.Wrap,
                    VerticalAlignment = CanvasVerticalAlignment.Center };
                using var clip = drawing.CreateLayer(1, bounds);
                drawing.DrawText(destination, new Rect(bounds.X + 8, bounds.Y + 4, Math.Max(1, bounds.Width - 16), Math.Max(1, bounds.Height - 8)), preferences.Theme.StatusForeground, format);
            }
            bool selected = !View.Composing && presentation.mode != VIEM_MODE_COMMAND_LINE
                && (presentation.caret_shape == VIEM_CARET_SHAPE_CELL && presentation.cursor_utf8_offset >= cluster.text_start && presentation.cursor_utf8_offset < cluster.text_end
                    || snapshot.SelectionSegments.Any(s => s.text_start <= cluster.text_start && s.text_end >= cluster.text_end));
            if (selected)
                drawing.DrawRectangle(new Rect(bounds.X + .5, bounds.Y + .5, Math.Max(0, bounds.Width - 1), Math.Max(0, bounds.Height - 1)), preferences.Theme.Caret, 1);
        }
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

using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Graphics.Canvas;
using Microsoft.Win32.SafeHandles;
using Viem.Windows.Shell;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;

namespace Viem.Windows.Rendering;

internal enum LocalImagePreviewStatus { Ready, Broken, Limited }
internal readonly record struct LocalImagePreviewResult(LocalImagePreview? Preview, LocalImagePreviewStatus Status);

// Raster-only, stream-based decoding. No URI image source or decoder is ever
// given a path or URL, including for SVG or a network-mounted file.
internal sealed class LocalImagePreview(CanvasBitmap bitmap, uint width, uint height) : IDisposable
{
    internal const long MaximumFileBytes = 10_000_000;
    internal const uint MaximumDimension = 5000;
    internal CanvasBitmap Bitmap { get; } = bitmap;
    internal uint Width { get; } = width;
    internal uint Height { get; } = height;
    internal long DecodedBytes => checked((long)Bitmap.SizeInPixels.Width * Bitmap.SizeInPixels.Height * 4);
    public void Dispose() => Bitmap.Dispose();

    internal static async Task<LocalImagePreviewResult> Load(CanvasDevice device, string destination, string? documentPath, CancellationToken cancellation)
    {
        string? path = ImageLocation.LocalPreviewPath(destination, documentPath);
        if (path == null) return new(null, LocalImagePreviewStatus.Broken);
        try
        {
            var read = await Task.Run(() => ReadLocalRaster(path, cancellation), cancellation);
            if (read.Limited) return new(null, LocalImagePreviewStatus.Limited);
            if (read.Bytes is not { } bytes || cancellation.IsCancellationRequested) return new(null, LocalImagePreviewStatus.Broken);
            using var stream = new InMemoryRandomAccessStream();
            using (var writer = new DataWriter(stream))
            {
                writer.WriteBytes(bytes); await writer.StoreAsync(); writer.DetachStream();
            }
            stream.Seek(0);
            var decoder = await BitmapDecoder.CreateAsync(stream);
            if (decoder.PixelWidth == 0 || decoder.PixelHeight == 0 || cancellation.IsCancellationRequested)
                return new(null, LocalImagePreviewStatus.Broken);
            // Header dimensions are checked before any pixel decode. Images
            // over the admission limits use an ordinary URL box, not an error.
            if (decoder.PixelWidth > MaximumDimension || decoder.PixelHeight > MaximumDimension)
                return new(null, LocalImagePreviewStatus.Limited);
            // Keep intrinsic geometry, but cap decoded texture memory. A view
            // cannot retain an unbounded bitmap just because source names one.
            double scale = Math.Min(1, 2048.0 / Math.Max(decoder.PixelWidth, decoder.PixelHeight));
            var transform = new BitmapTransform {
                ScaledWidth = Math.Max(1, (uint)Math.Round(decoder.PixelWidth * scale)),
                ScaledHeight = Math.Max(1, (uint)Math.Round(decoder.PixelHeight * scale)),
                InterpolationMode = BitmapInterpolationMode.Fant
            };
            using var pixels = await decoder.GetSoftwareBitmapAsync(BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied,
                transform, ExifOrientationMode.RespectExifOrientation, ColorManagementMode.ColorManageToSRgb);
            if (cancellation.IsCancellationRequested) return new(null, LocalImagePreviewStatus.Broken);
            return new(new(CanvasBitmap.CreateFromSoftwareBitmap(device, pixels), decoder.OrientedPixelWidth, decoder.OrientedPixelHeight), LocalImagePreviewStatus.Ready);
        }
        catch (Exception error) when (error is not OutOfMemoryException and not StackOverflowException)
        { return new(null, LocalImagePreviewStatus.Broken); } // A failed optional preview retains its URL placeholder.
    }

    internal static async Task<bool> CanOpenFile(string path)
    {
        if (Path.GetExtension(path).ToLowerInvariant() is not (".png" or ".jpg" or ".jpeg" or ".gif" or ".bmp" or ".tif" or ".tiff" or ".webp")) return false;
        // The preview budget does not prevent an explicit Open action. Read
        // only the signature when validating a local raster for its viewer.
        try { return await Task.Run(() => ReadLocalRaster(path, CancellationToken.None, headerOnly: true).Bytes != null); }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or ArgumentException) { return false; }
    }

    private static (byte[]? Bytes, bool Limited) ReadLocalRaster(string path, CancellationToken cancellation, bool headerOnly = false)
    {
        // Reject reparse points, offline/cloud placeholders and all network
        // drives before opening. Previewing must not hydrate a remote resource.
        for (string? item = path; item != null; item = Path.GetDirectoryName(item))
        {
            var attributes = File.GetAttributes(item);
            const FileAttributes remote = FileAttributes.ReparsePoint | FileAttributes.Offline | (FileAttributes)0x00400000 | (FileAttributes)0x00040000;
            if ((attributes & remote) != 0) return (null, false);
        }
        using var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
        if (file.Length <= 0 || cancellation.IsCancellationRequested) return (null, false);
        // Reject oversized files before allocating or reading any file bytes.
        if (!headerOnly && file.Length > MaximumFileBytes) return (null, true);
        // Validate the opened handle as well as the spelling to cover local
        // junction/path replacement between validation and open.
        var final = new StringBuilder(32768);
        uint length = GetFinalPathNameByHandle(file.SafeFileHandle, final, (uint)final.Capacity, 0);
        if (length == 0 || length >= final.Capacity) return (null, false);
        string resolved = final.ToString();
        if (resolved.StartsWith(@"\\?\UNC\", StringComparison.OrdinalIgnoreCase)) return (null, false);
        if (resolved.StartsWith(@"\\?\", StringComparison.Ordinal)) resolved = resolved[4..];
        if (ImageLocation.LocalPreviewPath(new Uri(resolved).AbsoluteUri, null) == null) return (null, false);
        byte[] bytes = new byte[checked((int)(headerOnly ? Math.Min(12, file.Length) : file.Length))];
        file.ReadExactly(bytes); cancellation.ThrowIfCancellationRequested();
        return (IsRaster(bytes) ? bytes : null, false);
    }

    internal static bool IsRaster(ReadOnlySpan<byte> bytes) => bytes.Length >= 12 && (
        bytes[..8].SequenceEqual(new byte[] { 137, 80, 78, 71, 13, 10, 26, 10 })
        || bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff
        || bytes[..6].SequenceEqual("GIF87a"u8) || bytes[..6].SequenceEqual("GIF89a"u8)
        || bytes[0] == 'B' && bytes[1] == 'M'
        || bytes[..4].SequenceEqual(new byte[] { 73, 73, 42, 0 }) || bytes[..4].SequenceEqual(new byte[] { 77, 77, 0, 42 })
        || bytes[..4].SequenceEqual("RIFF"u8) && bytes.Slice(8, 4).SequenceEqual("WEBP"u8));

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern uint GetFinalPathNameByHandle(SafeFileHandle file, StringBuilder path, uint length, uint flags);
}

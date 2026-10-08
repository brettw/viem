using System.Buffers.Binary;

namespace Viem.Windows.Rendering;

// Read only the small tables needed to recognize a packaged variable design.
// Unsupported collections and malformed/oversized tables take the ordinary
// native bundled-font path; this is not a replacement OpenType loader.
internal sealed record FontFileMetadata(byte[] Fvar, byte[] Name, byte[] Stat)
{
    internal string PostScriptName => FontVariations.Name(Name, 6, "");
    internal string Family => FontVariations.Name(Name, 16, FontVariations.Name(Name, 1, ""));
    internal bool Matches(FontFileMetadata other) => Fvar.Length != 0
        && Fvar.AsSpan().SequenceEqual(other.Fvar) && Name.AsSpan().SequenceEqual(other.Name)
        && Stat.AsSpan().SequenceEqual(other.Stat);

    internal static FontFileMetadata? TryRead(string path)
    {
        try {
            using var file = File.OpenRead(path);
            Span<byte> header = stackalloc byte[12];
            file.ReadExactly(header);
            uint signature = BinaryPrimitives.ReadUInt32BigEndian(header);
            if (signature is not (0x00010000 or 0x4f54544f)) return null;
            int count = BinaryPrimitives.ReadUInt16BigEndian(header[4..]);
            if (count is 0 or > 256 || 12L + count * 16 > file.Length) return null;
            byte[] directory = new byte[count * 16];
            file.ReadExactly(directory);
            var tables = new Dictionary<uint, (uint Offset, uint Length)>();
            for (int i = 0; i < count; i++) {
                var entry = directory.AsSpan(i * 16, 16);
                uint tag = BinaryPrimitives.ReadUInt32BigEndian(entry);
                if (tag is not (0x66766172 or 0x6e616d65 or 0x53544154)) continue; // fvar, name, STAT
                uint offset = BinaryPrimitives.ReadUInt32BigEndian(entry[8..]);
                uint length = BinaryPrimitives.ReadUInt32BigEndian(entry[12..]);
                if (length > 4 * 1024 * 1024 || (long)offset + length > file.Length
                    || !tables.TryAdd(tag, (offset, length))) return null;
            }
            byte[] Read(uint tag) {
                if (!tables.TryGetValue(tag, out var table)) return [];
                var bytes = new byte[checked((int)table.Length)];
                file.Position = table.Offset;
                file.ReadExactly(bytes);
                return bytes;
            }
            var result = new FontFileMetadata(Read(0x66766172), Read(0x6e616d65), Read(0x53544154));
            return result.PostScriptName.Length != 0 && result.Family.Length != 0
                && FontVariations.Parse(result.Fvar, result.Name, result.Stat).Axes.Length != 0 ? result : null;
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or ArgumentException or IndexOutOfRangeException or OverflowException)
        { System.Diagnostics.Debug.WriteLine($"Font metadata {path}: {error.Message}"); return null; }
    }
}

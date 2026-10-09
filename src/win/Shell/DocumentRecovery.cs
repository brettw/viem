using System.Security.Cryptography;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using Microsoft.UI.Dispatching;
using Microsoft.Win32.SafeHandles;
using Viem.Windows.Core;

namespace Viem.Windows.Shell;

// The envelope matches the Mac recovery record; UTF-8 JSON follows a magic
// prefix and source bytes are base64. Only an exclusively claimed, still-owned
// slot may be replaced or removed. The source file is never autosaved in place.
internal sealed record RecoverySnapshot(byte[] source, string format, uint encoding, uint fileFormat, ulong documentID, ulong documentRevision)
{
    [System.Text.Json.Serialization.JsonIgnore] public uint Format => format switch { "markdown" => 2, "markdownSource" => 5, "html" or "htmlSource" or "code" => 7, _ => 1 };
    public static RecoverySnapshot Capture(CoreDocument doc)
    { var s = doc.State; return new(doc.Source(s.document_revision), s.format switch { 2 => "markdown", 5 => "markdownSource", 7 => "code", _ => "plainText" }, s.encoding, s.file_format, s.document_id, s.document_revision); }
}
internal sealed record RecoveryRecord(int version, Guid owner, int processID, string host, string targetPath, double created, double updated, RecoverySnapshot? snapshot);
internal sealed record RecoveryFileIdentity(uint Volume, uint High, uint Low, uint Links);
internal sealed record RecoveryCandidate(string Path, RecoverySnapshot? Snapshot, RecoveryRecord? Record,
    RecoveryFileIdentity? Identity, string? Digest)
{
    public bool CanDelete => Record != null && Identity is { Links: 1 } && Digest != null && DocumentRecovery.OwnerHasExited(Record);
}
internal sealed class DocumentRecovery : IDisposable
{
    private static readonly byte[] Magic = "VIEM-RECOVERY\n"u8.ToArray();
    private static readonly DateTime Epoch = new(2001, 1, 1, 0, 0, 0, DateTimeKind.Utc);
    private readonly object gate = new();
    private readonly CoreDocument document;
    private readonly RecoveryRecord record;
    private readonly DispatcherQueueTimer timer;
    private readonly DispatcherQueue dispatcher;
    private readonly Action<Exception> report;
    private long generation;
    private ulong observedRevision;
    private DateTime changedAt;
    private bool disposed;
    private RecoveryCandidate[] pendingRetirement = [];
    internal string Slot { get; }
    internal Task<bool> Pending { get; private set; } = Task.FromResult(true);
    internal string? RetirementWarning { get; private set; }
#if DEBUG
    internal Action? BeforeWriteForTesting { get; set; }
#endif
    private DocumentRecovery(CoreDocument document, string slot, RecoveryRecord record, DispatcherQueue dispatcher, Action<Exception> report)
    {
        this.document = document; Slot = slot; this.record = record; this.report = report; this.dispatcher = dispatcher;
        observedRevision = document.State.document_revision;
        timer = dispatcher.CreateTimer(); timer.Interval = TimeSpan.FromSeconds(1);
        timer.Tick += (_, _) => { if (changedAt != default && DateTime.UtcNow - changedAt >= TimeSpan.FromSeconds(4)) { changedAt = default; try { Write(RecoverySnapshot.Capture(document)); } catch (Exception error) { report(error); } } };
        document.Changed += Changed; document.Disposed += Dispose; timer.Start();
    }
    private static IEnumerable<string> Slots(string path, string profile)
    {
        string name = "." + Path.GetFileName(path) + ".viem";
        for (int i = 0; i < 100; i++) yield return Path.Combine(Path.GetDirectoryName(path)!, name + (i == 0 ? "" : "." + i) + ".swp");
        string hash = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(Path.GetFullPath(path).ToUpperInvariant()))).ToLowerInvariant();
        for (int i = 0; i < 100; i++) yield return Path.Combine(profile, "Recovery", hash + "." + i + ".swp");
    }
    public static RecoveryCandidate[] Candidates(string path, string profile)
    {
        string foreign = Path.Combine(Path.GetDirectoryName(path)!, "." + Path.GetFileName(path) + ".swp");
        return Slots(path, profile).Append(foreign).Where(File.Exists).Select(slot => Candidate(slot, path)).ToArray();
    }
    private static RecoveryCandidate Candidate(string path, string target)
    {
        try
        {
            using var handle = OpenCandidate(path, delete: false);
            var identity = InspectCandidate(handle);
            using var file = new FileStream(handle, FileAccess.Read);
            byte[] bytes = ReadContents(file);
            var record = Decode(bytes);
            if (record == null || record.owner == Guid.Empty || !FileIdentity.Same(record.targetPath, target))
                return new(path, null, null, null, null);
            return new(path, record.snapshot, record, identity, Convert.ToHexString(SHA256.HashData(bytes)));
        }
        catch { return new(path, null, null, null, null); }
    }
    internal static bool OwnerHasExited(RecoveryRecord record)
    {
        if (record.processID <= 0 || record.processID == Environment.ProcessId
            || !string.Equals(record.host, Environment.MachineName, StringComparison.OrdinalIgnoreCase)) return false;
        try { using var process = Process.GetProcessById(record.processID); return process.HasExited; }
        catch (ArgumentException) { return true; }
        catch { return false; } // An inaccessible process or an unknown host is not proof of abandonment.
    }
    [StructLayout(LayoutKind.Sequential)] private struct FileInformation
    { public uint Attributes; public System.Runtime.InteropServices.ComTypes.FILETIME Created, Accessed, Written; public uint Volume, SizeHigh, SizeLow, Links, IndexHigh, IndexLow; }
    [StructLayout(LayoutKind.Sequential)] private struct FileDisposition { public byte Delete; }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
    private static extern SafeFileHandle CreateFileW(string path, uint access, uint share, nint security, uint creation, uint flags, nint template);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool GetFileInformationByHandle(SafeFileHandle file, out FileInformation information);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool SetFileInformationByHandle(SafeFileHandle file, int kind, ref FileDisposition disposition, uint size);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
    private static extern bool MoveFileExW(string existing, string replacement, uint flags);
    private static string NativePath(string path)
    {
        string absolute = Path.GetFullPath(path);
        return absolute.StartsWith(@"\\?\", StringComparison.Ordinal) ? absolute
            : absolute.StartsWith(@"\\", StringComparison.Ordinal) ? @"\\?\UNC\" + absolute[2..] : @"\\?\" + absolute;
    }
    private static SafeFileHandle OpenCandidate(string path, bool delete)
    {
        // Opening the reparse point itself prevents a symlink from authorizing
        // removal of its target. An exclusive delete handle freezes this exact
        // file while its identity, bytes and owner are checked again.
        const uint readAccess = 0x80000000, deleteAccess = 0x00010000, openReparsePoint = 0x00200000;
        var handle = CreateFileW(NativePath(path), readAccess | (delete ? deleteAccess : 0), delete ? 0u : 1u, 0, 3, openReparsePoint, 0);
        if (!handle.IsInvalid) return handle;
        int error = Marshal.GetLastWin32Error(); handle.Dispose();
        throw new IOException("Could not open the recovery file safely.", new Win32Exception(error));
    }
    private static RecoveryFileIdentity InspectCandidate(SafeFileHandle handle)
    {
        if (!GetFileInformationByHandle(handle, out var info))
            throw new IOException("Could not verify the recovery file identity.", new Win32Exception(Marshal.GetLastWin32Error()));
        if (((FileAttributes)info.Attributes & (FileAttributes.ReparsePoint | FileAttributes.Directory)) != 0)
            throw new IOException("The recovery file is not an ordinary file.");
        return new(info.Volume, info.IndexHigh, info.IndexLow, info.Links);
    }
    private static byte[] ReadContents(FileStream file)
    {
        byte[] bytes = new byte[checked((int)file.Length)];
        file.ReadExactly(bytes);
        return bytes;
    }
    internal static void DeleteCandidate(RecoveryCandidate candidate)
    {
        if (!candidate.CanDelete) throw new IOException("The recovery file is not confirmed safe to delete; it was retained.");
        using var handle = OpenCandidate(candidate.Path, delete: true);
        if (InspectCandidate(handle) != candidate.Identity)
            throw new IOException("The recovery file was replaced; it was retained.");
        using var file = new FileStream(handle, FileAccess.Read);
        if (Convert.ToHexString(SHA256.HashData(file)) != candidate.Digest || !OwnerHasExited(candidate.Record!)
            || InspectCandidate(handle) != candidate.Identity)
            throw new IOException("The recovery file or its owner changed; it was retained.");
        // Delete the verified handle, never a path that could now name another
        // file. Windows removes its directory entry when this handle closes.
        var disposition = new FileDisposition { Delete = 1 };
        if (!SetFileInformationByHandle(handle, 4, ref disposition, (uint)Marshal.SizeOf<FileDisposition>()))
            throw new IOException("Could not delete the old recovery file; it was retained.", new Win32Exception(Marshal.GetLastWin32Error()));
    }
    public static DocumentRecovery Claim(CoreDocument document, string path, string profile, DispatcherQueue dispatcher, Action<Exception> report)
    {
        double now = (DateTime.UtcNow - Epoch).TotalSeconds;
        var record = new RecoveryRecord(1, Guid.NewGuid(), Environment.ProcessId, Environment.MachineName, Path.GetFullPath(path), now, now, null);
        byte[] bytes = Encode(record);
        foreach (string slot in Slots(path, profile))
        {
            try
            {
                Directory.CreateDirectory(Path.GetDirectoryName(slot)!);
                using var file = new FileStream(slot, FileMode.CreateNew, FileAccess.Write, FileShare.Read);
                file.Write(bytes); file.Flush(true);
                return new(document, slot, record, dispatcher, report);
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
        }
        throw new IOException("Could not claim a recovery slot for " + path);
    }
    internal static byte[] Encode(RecoveryRecord record) => [.. Magic, .. JsonSerializer.SerializeToUtf8Bytes(record, FrontendJsonContext.Default.RecoveryRecord)];
    internal static RecoveryRecord? Read(string path)
    {
        try { return Decode(File.ReadAllBytes(path)); }
        catch { return null; }
    }
    private static RecoveryRecord? Decode(byte[] bytes)
    {
        if (!bytes.AsSpan().StartsWith(Magic)) return null;
        var value = JsonSerializer.Deserialize(bytes.AsSpan(Magic.Length), FrontendJsonContext.Default.RecoveryRecord);
        return value?.version == 1 ? value : null;
    }
    internal void RetireAfterNextWrite(RecoveryCandidate[] candidates)
    {
        lock (gate) { if (!disposed) pendingRetirement = candidates; }
    }
    private bool OwnsSlot() => (File.GetAttributes(Slot) & FileAttributes.ReparsePoint) == 0 && Read(Slot)?.owner == record.owner;
    private static void WriteDurably(string path, byte[] bytes)
    {
        string temporary = path + ".viem-" + Guid.NewGuid().ToString("N") + ".tmp";
        try
        {
            using (var output = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None))
            { output.Write(bytes); output.Flush(true); }
            // Recovery handoff needs the namespace replacement flushed too.
            // ReplaceFile's WRITE_THROUGH flag is unsupported; use the native
            // same-directory move with REPLACE_EXISTING | WRITE_THROUGH.
            if (!MoveFileExW(NativePath(temporary), NativePath(path), 0x1 | 0x8))
                throw new IOException("Could not commit the recovery snapshot.", new Win32Exception(Marshal.GetLastWin32Error()));
        }
        finally { if (File.Exists(temporary)) File.Delete(temporary); }
    }
    private void Changed()
    {
        var state = document.State;
        if (state.document_revision == observedRevision) return;
        observedRevision = state.document_revision; changedAt = DateTime.UtcNow;
        lock (gate) generation++;
    }
    public void Write(RecoverySnapshot snapshot)
    {
        long request; lock (gate) { if (disposed) return; request = ++generation; }
#if DEBUG
        var beforeWrite = BeforeWriteForTesting;
#endif
        Pending = Task.Run(() => {
            try
            {
                byte[] bytes = Encode(record with { updated = (DateTime.UtcNow - Epoch).TotalSeconds, snapshot = snapshot });
#if DEBUG
                beforeWrite?.Invoke();
#endif
                RecoveryCandidate[] retirement;
                lock (gate)
                {
                    if (disposed || request != generation) return false;
                    if (!OwnsSlot()) throw new IOException("Recovery slot ownership changed; the file was left untouched.");
                    WriteDurably(Slot, bytes);
                    retirement = pendingRetirement; pendingRetirement = [];
                }
                // A failed or superseded write cannot consume this handoff.
                // Removal is attempted once after a durable replacement exists.
                string? firstFailure = null; int failures = 0;
                foreach (var candidate in retirement)
                {
                    try { DeleteCandidate(candidate); }
                    catch (Exception error) { failures++; firstFailure ??= error.Message; }
                }
                if (failures > 0)
                {
                    string warning = $"{failures} previous recovery file(s) were retained. {firstFailure}";
                    RetirementWarning = warning;
                    dispatcher.TryEnqueue(() => {
                        if (document.Handle != 0) document.ConfigurationWarning(warning);
                        report(new IOException(warning));
                    });
                }
                return true;
            }
            catch (Exception error)
            {
                bool retiring; lock (gate) retiring = pendingRetirement.Length > 0;
                string? warning = retiring ? "The previous recovery file was retained because a new recovery backup could not be saved. " + error.Message : null;
                dispatcher.TryEnqueue(() => {
                    if (warning != null && document.Handle != 0) document.ConfigurationWarning(warning);
                    report(warning == null ? error : new IOException(warning, error));
                });
                return false;
            }
        });
    }
    public void Dispose()
    {
        if (disposed) return;
        timer.Stop(); document.Changed -= Changed; document.Disposed -= Dispose;
        lock (gate) { disposed = true; generation++; try { if (File.Exists(Slot) && OwnsSlot()) File.Delete(Slot); } catch (Exception error) { report(error); } }
    }
}

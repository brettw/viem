using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Microsoft.UI.Dispatching;
using Viem.Windows.Core;

namespace Viem.Windows.Shell;

// The envelope matches the Mac recovery record; UTF-8 JSON follows a magic
// prefix and source bytes are base64. Only an exclusively claimed, still-owned
// slot may be replaced or removed. The source file is never autosaved in place.
internal sealed record RecoverySnapshot(byte[] source, string format, uint encoding, uint fileFormat, ulong documentID, ulong documentRevision)
{
    [System.Text.Json.Serialization.JsonIgnore] public uint Format => format switch { "markdown" => 2, "html" => 3, "rtf" => 4, "markdownSource" => 5, "htmlSource" => 6, "code" => 7, _ => 1 };
    public static RecoverySnapshot Capture(CoreDocument doc)
    { var s = doc.State; return new(doc.Source(s.document_revision), s.format switch { 2 => "markdown", 3 => "html", 4 => "rtf", 5 => "markdownSource", 6 => "htmlSource", 7 => "code", _ => "plainText" }, s.encoding, s.file_format, s.document_id, s.document_revision); }
}
internal sealed record RecoveryRecord(int version, Guid owner, int processID, string host, string targetPath, double created, double updated, RecoverySnapshot? snapshot);
internal sealed record RecoveryCandidate(string Path, RecoverySnapshot? Snapshot);
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
    internal string Slot { get; }
    internal Task<bool> Pending { get; private set; } = Task.FromResult(true);
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
        return Slots(path, profile).Append(foreign).Where(File.Exists).Select(slot => { var r = Read(slot); return new RecoveryCandidate(slot, r != null && FileIdentity.Same(r.targetPath, path) ? r.snapshot : null); }).ToArray();
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
        try { byte[] bytes = File.ReadAllBytes(path); if (!bytes.AsSpan().StartsWith(Magic)) return null; var value = JsonSerializer.Deserialize(bytes.AsSpan(Magic.Length), FrontendJsonContext.Default.RecoveryRecord); return value?.version == 1 ? value : null; }
        catch { return null; }
    }
    private bool OwnsSlot() => (File.GetAttributes(Slot) & FileAttributes.ReparsePoint) == 0 && Read(Slot)?.owner == record.owner;
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
        Pending = Task.Run(() => {
            try
            {
                byte[] bytes = Encode(record with { updated = (DateTime.UtcNow - Epoch).TotalSeconds, snapshot = snapshot });
                lock (gate) { if (disposed || request != generation) return false; if (!OwnsSlot()) throw new IOException("Recovery slot ownership changed; the file was left untouched."); Preferences.AtomicWrite(Slot, bytes); return true; }
            }
            catch (Exception error) { dispatcher.TryEnqueue(() => report(error)); return false; }
        });
    }
    public void Dispose()
    {
        if (disposed) return;
        timer.Stop(); document.Changed -= Changed; document.Disposed -= Dispose;
        lock (gate) { disposed = true; generation++; try { if (File.Exists(Slot) && OwnsSlot()) File.Delete(Slot); } catch (Exception error) { report(error); } }
    }
}

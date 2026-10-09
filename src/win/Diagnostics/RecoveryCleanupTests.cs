#if DEBUG
using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Dispatching;
using Viem.Windows.Core;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class RecoveryCleanupTests
{
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
    private static extern bool CreateHardLinkW(string link, string existing, nint security);
    internal static async Task Run(CanvasDevice device, DispatcherQueue dispatcher, string profile, Action<bool, string> check)
    {
        string directory = Path.Combine(profile, "files", "recovery-cleanup-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        var snapshot = new RecoverySnapshot("# recovered\r\n"u8.ToArray(), "markdownSource", 1, 1, 1, 1);
        string NewPath(string name)
        {
            string path = Path.Combine(directory, name + ".md");
            File.WriteAllBytes(path, "# disk\r\n"u8.ToArray());
            return path;
        }
        RecoveryRecord StaleRecord(string path) => new(1, Guid.NewGuid(), int.MaxValue, Environment.MachineName, path, 1, 1, snapshot);
        string WriteRecord(string path, RecoveryRecord record, string suffix = "")
        {
            string slot = Path.Combine(directory, "." + Path.GetFileName(path) + ".viem" + suffix + ".swp");
            File.WriteAllBytes(slot, DocumentRecovery.Encode(record));
            return slot;
        }
        bool Rejects(RecoveryCandidate candidate)
        {
            try { DocumentRecovery.DeleteCandidate(candidate); return false; }
            catch (IOException) { return true; }
        }

        string recoveredPath = NewPath("recover");
        string recoveredSlot = WriteRecord(recoveredPath, StaleRecord(recoveredPath));
        string unselectedSlot = WriteRecord(recoveredPath, StaleRecord(recoveredPath), ".1");
        var recoveredCandidate = DocumentRecovery.Candidates(recoveredPath, profile).Single(c => c.Path == recoveredSlot);
        check(recoveredCandidate.CanDelete, "a local exited Viem owner permits recovery retirement");
        using (var doc = new CoreDocument(snapshot.source, recoveredPath, snapshot.Format, snapshot.encoding, snapshot.fileFormat))
        using (var replacement = DocumentRecovery.Claim(doc, recoveredPath, profile, dispatcher, _ => { }))
        {
            doc.MarkRecovered();
            replacement.RetireAfterNextWrite([recoveredCandidate]);
            check(File.Exists(recoveredSlot), "recover keeps the previous snapshot until a replacement is committed");
            replacement.Write(RecoverySnapshot.Capture(doc));
            check(await replacement.Pending && !File.Exists(recoveredSlot) && File.Exists(unselectedSlot),
                "recover retires only the selected stale snapshot after durable replacement");
            check(doc.IsDirty && DocumentRecovery.Read(replacement.Slot)!.snapshot!.source.AsSpan().SequenceEqual(snapshot.source)
                && File.ReadAllBytes(recoveredPath).AsSpan().SequenceEqual("# disk\r\n"u8),
                "recovery handoff retains recovered bytes and unsaved state without writing the source");
        }

        string editPath = NewPath("edit");
        WriteRecord(editPath, StaleRecord(editPath));
        WriteRecord(editPath, StaleRecord(editPath) with { snapshot = null }, ".1");
        var editCandidates = DocumentRecovery.Candidates(editPath, profile);
        using (var doc = new CoreDocument(File.ReadAllBytes(editPath), editPath))
        using (var replacement = DocumentRecovery.Claim(doc, editPath, profile, dispatcher, _ => { }))
        {
            replacement.RetireAfterNextWrite(editCandidates);
            replacement.Write(RecoverySnapshot.Capture(doc));
            check(await replacement.Pending && editCandidates.All(c => !File.Exists(c.Path)) && !doc.IsDirty,
                "edit and delete recovery file discards every selected stale snapshot and keeps the disk document clean");
        }

        string protectedPath = NewPath("protected");
        string liveSlot = WriteRecord(protectedPath, StaleRecord(protectedPath) with { processID = Environment.ProcessId });
        string remoteSlot = WriteRecord(protectedPath, StaleRecord(protectedPath) with { host = "different-host-" + Guid.NewGuid() }, ".1");
        string foreignSlot = Path.Combine(directory, "." + Path.GetFileName(protectedPath) + ".swp");
        File.WriteAllText(foreignSlot, "foreign Vim swap");
        var protectedCandidates = DocumentRecovery.Candidates(protectedPath, profile);
        check(protectedCandidates.Length == 3 && protectedCandidates.All(c => !c.CanDelete && Rejects(c))
            && File.Exists(liveSlot) && File.Exists(remoteSlot) && File.Exists(foreignSlot),
            "live sessions, unknown hosts and foreign swap files cannot be deleted");
        check(protectedCandidates.Single(c => c.Path == liveSlot).Snapshot != null,
            "a live Viem snapshot remains recoverable without authorizing its deletion");

        string changedPath = NewPath("changed");
        var changedRecord = StaleRecord(changedPath);
        string changedSlot = WriteRecord(changedPath, changedRecord);
        var changedCandidate = DocumentRecovery.Candidates(changedPath, profile).Single();
        File.WriteAllBytes(changedSlot, DocumentRecovery.Encode(changedRecord with { updated = 2 }));
        check(Rejects(changedCandidate) && File.Exists(changedSlot), "changing a recovery file in place prevents stale deletion");

        string replacedPath = NewPath("replaced");
        string replacedSlot = WriteRecord(replacedPath, StaleRecord(replacedPath));
        var replacedCandidate = DocumentRecovery.Candidates(replacedPath, profile).Single();
        byte[] replacedContents = File.ReadAllBytes(replacedSlot);
        File.Move(replacedSlot, replacedSlot + ".previous");
        File.WriteAllBytes(replacedSlot, replacedContents);
        check(Rejects(replacedCandidate) && File.Exists(replacedSlot) && File.Exists(replacedSlot + ".previous"),
            "an identical-byte replacement recovery file has a different identity and is retained");

        string blockedPath = NewPath("blocked");
        string blockedSlot = WriteRecord(blockedPath, StaleRecord(blockedPath));
        var blockedCandidate = DocumentRecovery.Candidates(blockedPath, profile).Single();
        using (var held = new FileStream(blockedSlot, FileMode.Open, FileAccess.Read, FileShare.Read))
            check(Rejects(blockedCandidate) && File.Exists(blockedSlot), "a recovery file held open by another reader is retained");

        string linkedPath = NewPath("linked");
        string linkedSlot = WriteRecord(linkedPath, StaleRecord(linkedPath));
        var beforeLink = DocumentRecovery.Candidates(linkedPath, profile).Single();
        string otherLink = linkedSlot + ".other";
        if (!CreateHardLinkW(otherLink, linkedSlot, 0)) throw new Win32Exception(Marshal.GetLastWin32Error());
        var linkedCandidate = DocumentRecovery.Candidates(linkedPath, profile).Single();
        check(linkedCandidate.Snapshot != null && !linkedCandidate.CanDelete && Rejects(linkedCandidate)
            && Rejects(beforeLink) && File.Exists(linkedSlot) && File.Exists(otherLink),
            "hardlinked snapshots remain recoverable, while existing and newly linked candidates cannot be deleted");

        string failedPath = NewPath("failed-write");
        string failedSlot = WriteRecord(failedPath, StaleRecord(failedPath));
        var failedCandidate = DocumentRecovery.Candidates(failedPath, profile).Single();
        using (var doc = new CoreDocument(snapshot.source, failedPath, snapshot.Format))
        using (var replacement = DocumentRecovery.Claim(doc, failedPath, profile, dispatcher, _ => { }))
        {
            byte[] ownership = File.ReadAllBytes(replacement.Slot);
            replacement.RetireAfterNextWrite([failedCandidate]);
            File.WriteAllText(replacement.Slot, "changed ownership");
            replacement.Write(RecoverySnapshot.Capture(doc));
            check(!await replacement.Pending && File.Exists(failedSlot), "failed replacement writes retain the original recovery file");
            var diagnosticPublished = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            dispatcher.TryEnqueue(() => diagnosticPublished.SetResult());
            await diagnosticPublished.Task.WaitAsync(TimeSpan.FromSeconds(10));
            check(doc.ConfigurationDiagnostics.Contains("previous recovery file was retained"),
                "failed recovery handoffs retain a document diagnostic before its pane becomes active");
            File.WriteAllBytes(replacement.Slot, ownership);
            replacement.Write(RecoverySnapshot.Capture(doc));
            check(await replacement.Pending && !File.Exists(failedSlot), "pending cleanup survives a failed replacement and follows a later durable write");
        }

        string supersededPath = NewPath("superseded");
        string supersededSlot = WriteRecord(supersededPath, StaleRecord(supersededPath));
        var supersededCandidate = DocumentRecovery.Candidates(supersededPath, profile).Single();
        using (var doc = new CoreDocument(snapshot.source, supersededPath, snapshot.Format))
        using (var view = new CoreView(doc, device, dispatcher, 500, 200))
        using (var replacement = DocumentRecovery.Claim(doc, supersededPath, profile, dispatcher, _ => { }))
        using (var release = new ManualResetEventSlim())
        {
            var entered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            replacement.RetireAfterNextWrite([supersededCandidate]);
            replacement.BeforeWriteForTesting = () => { entered.SetResult(); if (!release.Wait(TimeSpan.FromSeconds(10))) throw new TimeoutException(); };
            replacement.Write(RecoverySnapshot.Capture(doc));
            try
            {
                await entered.Task.WaitAsync(TimeSpan.FromSeconds(10));
                view.Command("i"); view.Text("new "); view.Key(VIEM_KEY_ESCAPE);
            }
            finally { release.Set(); }
            check(!await replacement.Pending && File.Exists(supersededSlot), "superseding an in-flight snapshot retains the previous recovery file");
            replacement.BeforeWriteForTesting = null;
            replacement.Write(RecoverySnapshot.Capture(doc));
            check(await replacement.Pending && !File.Exists(supersededSlot), "the next current snapshot completes a superseded recovery handoff");
        }

        string closedPath = NewPath("closed");
        string closedSlot = WriteRecord(closedPath, StaleRecord(closedPath));
        var closedCandidate = DocumentRecovery.Candidates(closedPath, profile).Single();
        using (var doc = new CoreDocument(snapshot.source, closedPath, snapshot.Format))
        using (var replacement = DocumentRecovery.Claim(doc, closedPath, profile, dispatcher, _ => { }))
        using (var release = new ManualResetEventSlim())
        {
            var entered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            replacement.RetireAfterNextWrite([closedCandidate]);
            replacement.BeforeWriteForTesting = () => { entered.SetResult(); if (!release.Wait(TimeSpan.FromSeconds(10))) throw new TimeoutException(); };
            replacement.Write(RecoverySnapshot.Capture(doc));
            try { await entered.Task.WaitAsync(TimeSpan.FromSeconds(10)); replacement.Dispose(); }
            finally { release.Set(); }
            check(!await replacement.Pending && File.Exists(closedSlot), "closing before the replacement commits preserves the old recovery file");
        }

        string warningPath = NewPath("warning");
        var warningRecord = StaleRecord(warningPath);
        string warningSlot = WriteRecord(warningPath, warningRecord);
        var warningCandidate = DocumentRecovery.Candidates(warningPath, profile).Single();
        using (var doc = new CoreDocument(snapshot.source, warningPath, snapshot.Format))
        using (var replacement = DocumentRecovery.Claim(doc, warningPath, profile, dispatcher, _ => { }))
        {
            replacement.RetireAfterNextWrite([warningCandidate]);
            File.WriteAllBytes(warningSlot, DocumentRecovery.Encode(warningRecord with { updated = 2 }));
            replacement.Write(RecoverySnapshot.Capture(doc));
            check(await replacement.Pending && File.Exists(warningSlot) && replacement.RetirementWarning != null,
                "cleanup failure preserves both snapshots and records a diagnostic without failing the new backup");
            string? warning = replacement.RetirementWarning;
            File.WriteAllBytes(warningSlot, DocumentRecovery.Encode(warningRecord));
            replacement.Write(RecoverySnapshot.Capture(doc));
            check(await replacement.Pending && File.Exists(warningSlot) && replacement.RetirementWarning == warning,
                "later backups neither retry a failed retirement nor erase its diagnostic");
        }
        Directory.Delete(directory, recursive: true);
    }
}
#endif

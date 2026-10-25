using System.Security.Cryptography;
using System.Text;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using static Viem.Windows.Interop.Abi;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    // UI-owned, shared by every view of a buffer. Workers receive only a path
    // and cancellation token; neither watchers nor hash workers touch the core.
    private static readonly Dictionary<CoreDocument, ExternalFileState> externalFiles = [];
    private static readonly HashSet<CoreDocument> missingFileBaselines = [];
    // One application-wide queue, with one flag per open buffer rather than
    // one entry per filesystem event. Only this worker presents file reviews.
    private static bool externalFileReviewsRunning;
    private static Task externalFileReviewWork = Task.CompletedTask;
    private int openDialogs;

    private sealed class ExternalFileState : IDisposable
    {
        internal readonly CoreDocument Document;
        internal readonly CancellationTokenSource Lifetime = new();
        internal FileChangeMonitor? Monitor;
        internal string Path = "";
        internal byte[] Baseline = [];
        internal bool Pending, Disposed, BaselineMissing;
        internal int Writes;
        private ulong review;

        internal unsafe ExternalFileState(CoreDocument document)
        {
            Document = document;
            ulong handle = 0;
            Check(viem_external_file_review_create(&handle), "Create file review");
            review = handle;
        }
        internal void Reset() => Check(viem_external_file_review_reset(review), "Reset file review");
        internal void ClearAcknowledged() => Check(viem_external_file_review_clear_acknowledged(review), "Reset file acknowledgement");
        internal unsafe bool Begin(byte[] token, bool canReload)
        {
            uint flags = 0;
            fixed (byte* p = token)
                Check(viem_external_file_review_begin(review, p, (ulong)token.Length, canReload ? 1u : 0, Document.IsDirty ? 1u : 0, &flags), "Review file change");
            return flags != 0;
        }
        internal unsafe void Finish(byte[] token, bool acknowledge)
        {
            if (Disposed) return;
            fixed (byte* p = token)
                Check(viem_external_file_review_finish(review, p, (ulong)token.Length, acknowledge ? 1u : 0), "Finish file review");
        }
        public void Dispose()
        {
            Disposed = true;
            Pending = false;
            Lifetime.Cancel(); Monitor?.Dispose();
            Check(viem_external_file_review_destroy(review), "Close file review");
            review = 0;
        }
    }

    private void TrackExternalFile(CoreDocument document)
    {
        if (document.Handle == 0 || document.FilePath is not string path || !savedSources.TryGetValue(document, out var baseline)) return;
        if (!externalFiles.TryGetValue(document, out var state))
        {
            externalFiles[document] = state = new(document);
            document.Disposed += () => { if (externalFiles.Remove(document, out var removed)) removed.Dispose(); };
        }
        bool pathChanged = !string.Equals(state.Path, path, StringComparison.OrdinalIgnoreCase);
        bool missing = missingFileBaselines.Contains(document);
        if (!pathChanged && state.BaselineMissing == missing && state.Baseline.AsSpan().SequenceEqual(baseline)) return;
        state.Path = path; state.Baseline = baseline; state.BaselineMissing = missing; state.Reset();
        if (pathChanged || state.Monitor == null)
        {
            state.Monitor?.Dispose();
            var dispatcher = DispatcherQueue;
            state.Monitor = new FileChangeMonitor(path, () => dispatcher.TryEnqueue(() => RequestExternalCheck(state)));
        }
        RequestExternalCheck(state);
    }

    private void RequestExternalFileChecks()
    {
        if (closed) return;
        // Activation and dialog completion collect all windows before the
        // worker presents anything, including documents in background windows.
        foreach (var window in App.Instance.Windows.Where(w => !w.closed).ToArray())
        foreach (var document in window.Panes.Select(p => p.Document).Distinct().ToArray())
        {
            window.TrackExternalFile(document);
            if (externalFiles.TryGetValue(document, out var state)) RequestExternalCheck(state);
        }
    }

    private static void RequestExternalCheck(ExternalFileState state)
    {
        if (state.Disposed) return;
        state.Pending = true;
        if (!externalFileReviewsRunning && App.Instance.Windows.Any(w => w.CanReviewExternalFile))
            externalFileReviewWork = ReviewExternalFiles();
    }

    private bool CanReviewExternalFile => !closed && openDialogs == 0 && root.XamlRoot != null &&
#if DEBUG
        (ExternalFileActiveForTesting ?? IsWindowActive);
#else
        IsWindowActive;
#endif

    private sealed record FileObservation(byte[] Token, byte[]? Digest, string Message, bool Missing = false);

    private static async Task<FileObservation> ObserveFile(string path, CancellationToken cancellation)
    {
        // A writer may briefly deny sharing or move the old file out of the way.
        // Retries are bounded and never block the UI or become a polling loop.
        for (int attempt = 0; ; attempt++)
        {
            cancellation.ThrowIfCancellationRequested();
            try
            {
                using var input = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete, 65536, FileOptions.Asynchronous | FileOptions.SequentialScan);
                byte[] hash = await SHA256.HashDataAsync(input, cancellation).ConfigureAwait(false);
                return new(hash, hash, "was changed by another program.");
            }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException)
            {
                if (attempt < 2) { await Task.Delay(50, cancellation).ConfigureAwait(false); continue; }
                bool missing = error is FileNotFoundException or DirectoryNotFoundException;
                return new(Encoding.UTF8.GetBytes(missing ? "missing" : $"unreadable:{error.HResult}"), null,
                    missing ? "was deleted outside Viem. Your buffer is unchanged." : "could not be checked. Your buffer is unchanged. " + error.Message, missing);
            }
        }
    }

    private sealed record PendingFileReview(ExternalFileState State, string Path, byte[] Baseline,
        bool BaselineMissing, FileObservation Observation)
    {
        internal bool Current => !State.Disposed && State.Writes == 0 && State.Path == Path &&
            ReferenceEquals(State.Baseline, Baseline) && State.BaselineMissing == BaselineMissing;
    }

    private sealed record PresentedFileReview(PendingFileReview Pending, EditorWindow Window,
        EditorPane Pane, ulong Revision);

    internal sealed record ExternalFileReviewDialog(IReadOnlyList<CoreDocument> Documents,
        string Title, string Text, string Primary, string Close);

    private static async Task ReviewExternalFiles()
    {
        externalFileReviewsRunning = true;
        try
        {
            // A one-shot collection window joins separate watcher callbacks.
            // It is not polling, and continuous notifications cannot extend it.
            await Task.Delay(75);
            while (App.Instance.Windows.FirstOrDefault(w => w.CanReviewExternalFile) is { } host)
            {
                var observed = new List<PendingFileReview>();
                var checkedFiles = new HashSet<ExternalFileState>();
                // Read at most once per buffer in a collection pass. New files
                // arriving during I/O join this batch; repeats stay queued.
                while (externalFiles.Values.FirstOrDefault(s => s.Pending && s.Writes == 0 && !checkedFiles.Contains(s)) is { } state)
                {
                    checkedFiles.Add(state);
                    state.Pending = false;
                    string path = state.Path; byte[] baseline = state.Baseline;
                    bool missing = state.BaselineMissing;
                    try
                    {
                        var observation = await Task.Run(() => ObserveFile(path, state.Lifetime.Token));
                        var candidate = new PendingFileReview(state, path, baseline, missing, observation);
                        if (!candidate.Current) continue;
                        if (missing ? observation.Missing : observation.Digest is { } digest && digest.AsSpan().SequenceEqual(baseline))
                        { state.ClearAcknowledged(); continue; }
                        observed.Add(candidate);
                    }
                    catch (OperationCanceledException) { }
                    catch (Exception error) { host.ActivePane?.Report(error); }
                }
                if (checkedFiles.Count == 0) break;
                if (!host.CanReviewExternalFile)
                {
                    foreach (var candidate in observed.Where(c => c.Current)) candidate.State.Pending = true;
                    continue;
                }

                var batch = new List<PresentedFileReview>();
                try
                {
                    foreach (var candidate in observed.Where(c => c.Current))
                    {
                        var document = candidate.State.Document;
                        var owner = host.Panes.Any(p => p.Document == document) ? host :
                            App.Instance.Windows.FirstOrDefault(w => !w.closed && w.Panes.Any(p => p.Document == document));
                        if (owner == null || !candidate.State.Begin(candidate.Observation.Token, candidate.Observation.Digest != null)) continue;
                        batch.Add(new(candidate, owner, owner.Panes.First(p => p.Document == document), document.State.document_revision));
                    }
                    if (batch.Count == 0) continue;
                    var dialog = DescribeExternalFileReview(batch);
                    ContentDialogResult result;
#if DEBUG
                    if (host.ExternalFileBatchDecisionForTesting is { } decideBatch) result = await decideBatch(dialog);
                    else
#endif
                    result = await host.Dialog(dialog.Title, dialog.Text, dialog.Primary, dialog.Close, defaultButton: ContentDialogButton.Close);
                    // The list is now fixed. Events received since presentation
                    // only set Pending and cannot inherit this decision.
                    foreach (var entry in batch) await ApplyExternalFileDecision(host, entry, result);
                }
                finally
                {
                    foreach (var entry in batch)
                        entry.Pending.State.Finish(entry.Pending.Observation.Token, false);
                }
            }
        }
        catch (Exception error)
        {
            App.Instance.Windows.FirstOrDefault(w => w.CanReviewExternalFile)?.ActivePane?.Report(error);
        }
        finally { externalFileReviewsRunning = false; }
    }

    private static ExternalFileReviewDialog DescribeExternalFileReview(List<PresentedFileReview> batch)
    {
        bool multiple = batch.Count > 1;
        bool canReload = batch.Any(e => e.Pending.Observation.Digest != null);
        string text = string.Join("\n\n", batch.Select(e =>
            $"{(multiple ? e.Pending.Path : e.Pending.State.Document.Name)} {e.Pending.Observation.Message}" +
            (e.Pending.Observation.Digest != null && e.Pending.State.Document.IsDirty
                ? " Reloading will discard your unsaved edits in all views." : "")));
        if (multiple && canReload && batch.Any(e => e.Pending.Observation.Digest == null))
            text += "\n\nReload all reloads the available files. Deleted or unreadable files keep their current buffers.";
        string suffix = multiple ? " all" : "";
        return new(batch.Select(e => e.Pending.State.Document).ToArray(),
            multiple ? "Files changed on disk" : "File changed on disk", text,
            canReload ? "Reload" + suffix : "", "Keep My Version" + suffix);
    }

    private static async Task ApplyExternalFileDecision(EditorWindow host, PresentedFileReview entry, ContentDialogResult result)
    {
        var candidate = entry.Pending;
        var state = candidate.State;
        var observation = candidate.Observation;
        bool acknowledged = false;
        try
        {
            if (!candidate.Current) return;
            if (result != ContentDialogResult.Primary || observation.Digest == null) { acknowledged = true; return; }
            if (state.Document.State.document_revision != entry.Revision) { state.Pending = true; return; }
            // Consent applies only to the exact disk and buffer revisions in
            // this batch. One stale file must not prevent the others reloading.
            byte[] bytes = await Task.Run(() => File.ReadAllBytesAsync(candidate.Path, state.Lifetime.Token));
            byte[] hash = await Task.Run(() => SHA256.HashData(bytes));
            if (!candidate.Current) return;
            if (state.Document.State.document_revision != entry.Revision || !hash.AsSpan().SequenceEqual(observation.Digest))
            { state.Pending = true; return; }
            if (!host.CanReviewExternalFile || entry.Window.closed || !entry.Window.Panes.Contains(entry.Pane) || entry.Pane.Document != state.Document)
            { state.Pending = true; return; }
            await entry.Window.Reload(entry.Pane, true, bytes);
            if (entry.Pane.Document != state.Document)
            {
                // Reload replaces the shared document and disposes its old
                // queue entry. Its new monitor checks against the installed
                // bytes, suppressing duplicates without hiding later writes.
                state.Pending = false;
                acknowledged = true;
            }
        }
        catch (OperationCanceledException) { }
        catch (Exception error) { entry.Pane.Report(error); }
        finally { state.Finish(observation.Token, acknowledged); }
    }

    private sealed class ExternalFileWrite(EditorWindow window, CoreDocument document) : IDisposable
    {
        private readonly ExternalFileState? state = Begin(document);
        private static ExternalFileState? Begin(CoreDocument document)
        {
            if (!externalFiles.TryGetValue(document, out var state)) return null;
            state.Writes++;
            return state;
        }
        public void Dispose()
        {
            if (state != null) state.Writes--;
            window.TrackExternalFile(document);
            if (state != null) RequestExternalCheck(state);
        }
    }

    private async Task DeferExternalFileReviews(Func<Task> action)
    {
        openDialogs++;
        try { await action(); }
        finally { openDialogs--; RequestExternalFileChecks(); }
    }

    private async Task CheckExternalChanges()
    {
        RequestExternalFileChecks();
        await externalFileReviewWork;
    }
#if DEBUG
    internal Func<ExternalFileReviewDialog, Task<ContentDialogResult>>? ExternalFileBatchDecisionForTesting { get; set; }
    internal bool? ExternalFileActiveForTesting { get; set; }
    internal void ActivateExternalFileChecksForTesting() => RequestExternalFileChecks();
    internal Task CheckExternalChangesForTesting() => CheckExternalChanges();
#endif
}

using System.Security.Cryptography;
using System.Text;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using static Viem.Windows.Interop.Abi;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    // UI-owned, shared by every view of a buffer. Workers receive only a path
    // and cancellation token; neither watchers nor hash workers touch the core.
    private static readonly Dictionary<CoreDocument, ExternalFileState> externalFiles = [];
    private static readonly HashSet<CoreDocument> missingFileBaselines = [];
    private int openDialogs;

    private sealed class ExternalFileState : IDisposable
    {
        internal readonly CoreDocument Document;
        internal readonly CancellationTokenSource Lifetime = new();
        internal FileChangeMonitor? Monitor;
        internal string Path = "";
        internal byte[] Baseline = [];
        internal bool Pending, Running, Disposed, BaselineMissing;
        internal int Writes;
        internal Task Work = Task.CompletedTask;
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
        foreach (var document in Panes.Select(p => p.Document).Distinct().ToArray())
        {
            TrackExternalFile(document);
            if (externalFiles.TryGetValue(document, out var state)) RequestExternalCheck(state);
        }
    }

    private static void RequestExternalCheck(ExternalFileState state)
    {
        if (state.Disposed) return;
        state.Pending = true;
        if (!state.Running && state.Writes == 0) state.Work = CheckExternalFile(state);
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

    private static async Task CheckExternalFile(ExternalFileState state)
    {
        state.Running = true;
        try
        {
            while (state.Pending && !state.Disposed && state.Writes == 0)
            {
                state.Pending = false;
                string path = state.Path; byte[] baseline = state.Baseline;
                bool Current() => !state.Disposed && state.Writes == 0 && state.Path == path && ReferenceEquals(state.Baseline, baseline);
                var observation = await Task.Run(() => ObserveFile(path, state.Lifetime.Token));
                if (!Current()) continue;
                if (state.BaselineMissing ? observation.Missing : observation.Digest is { } digest && digest.AsSpan().SequenceEqual(baseline))
                { state.ClearAcknowledged(); continue; }
                var window = App.Instance.Windows.FirstOrDefault(w => w.CanReviewExternalFile && w.Panes.Any(p => p.Document == state.Document));
                if (window == null) continue; // Activation requests a fresh observation.
                var pane = window.Panes.First(p => p.Document == state.Document);
                bool canReload = observation.Digest != null;
                if (!state.Begin(observation.Token, canReload)) continue;
                bool acknowledged = false;
                var expected = state.Document.State;
                try
                {
                    ContentDialogResult result;
#if DEBUG
                    if (window.ExternalFileReviewDecisionForTesting is { } decide) result = await decide(state.Document, canReload);
                    else
#endif
                    result = await window.Dialog("File changed on disk", $"{state.Document.Name} {observation.Message}" +
                        (canReload && state.Document.IsDirty ? " Reloading will discard your unsaved edits in all views." : ""),
                        canReload ? "Reload" : "", "Keep My Version", defaultButton: ContentDialogButton.Close);
                    if (!Current()) continue;
                    if (result != ContentDialogResult.Primary || !canReload) { acknowledged = true; continue; }
                    if (state.Document.State.document_revision != expected.document_revision)
                    { state.Pending = true; continue; }
                    // Reload only the exact bytes the user reviewed. A later disk
                    // or buffer change requires a new choice, never stale consent.
                    byte[] bytes = await Task.Run(() => File.ReadAllBytesAsync(path, state.Lifetime.Token));
                    byte[] hash = await Task.Run(() => SHA256.HashData(bytes));
                    if (!Current()) continue;
                    if (state.Document.State.document_revision != expected.document_revision || !hash.AsSpan().SequenceEqual(observation.Digest))
                    { state.Pending = true; continue; }
                    if (!window.CanReviewExternalFile || !window.Panes.Contains(pane) || pane.Document != state.Document)
                    { state.Pending = true; continue; }
                    await window.Reload(pane, true, bytes);
                    acknowledged = true;
                }
                finally { state.Finish(observation.Token, acknowledged); }
            }
        }
        catch (OperationCanceledException) { }
        catch (Exception error)
        {
            App.Instance.Windows.FirstOrDefault(w => w.CanReviewExternalFile && w.Panes.Any(p => p.Document == state.Document))?.ActivePane?.Report(error);
        }
        finally { state.Running = false; }
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
        foreach (var document in Panes.Select(p => p.Document).Distinct().ToArray())
            if (externalFiles.TryGetValue(document, out var state)) await state.Work;
    }
#if DEBUG
    internal Func<CoreDocument, bool, Task<ContentDialogResult>>? ExternalFileReviewDecisionForTesting { get; set; }
    internal bool? ExternalFileActiveForTesting { get; set; }
    internal void ActivateExternalFileChecksForTesting() => RequestExternalFileChecks();
    internal Task CheckExternalChangesForTesting() => CheckExternalChanges();
#endif
}

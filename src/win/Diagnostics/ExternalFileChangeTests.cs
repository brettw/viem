#if DEBUG
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class ExternalFileChangeTests
{
    // The old UI poll waited at least 50 dispatcher ticks (two seconds).
    // These waits exercise native file notifications without invoking checktime.
    private static readonly TimeSpan NotificationTimeout = TimeSpan.FromMilliseconds(1500);
    private static readonly TimeSpan CompletionTimeout = TimeSpan.FromSeconds(10);

    private static void Check(bool value, string name)
    { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }

    private static TaskCompletionSource Signal() => new(TaskCreationOptions.RunContinuationsAsynchronously);

    private static EditorWindow Open(Preferences preferences, List<EditorWindow> windows, CoreDocument? document = null)
    {
        var window = new EditorWindow(preferences, document ?? new CoreDocument([])) {
            ExternalFileActiveForTesting = true,
            CloseReviewDecisionForTesting = _ => Task.FromResult(ContentDialogResult.Secondary)
        };
        windows.Add(window);
        App.Instance.Windows.Add(window);
        window.Closed += (_, _) => App.Instance.Windows.Remove(window);
        window.Activate();
        return window;
    }

    private static async Task<EditorWindow> OpenFile(Preferences preferences, List<EditorWindow> windows, string path)
    {
        var window = Open(preferences, windows);
        await window.ActivePane!.Ready;
        await window.OpenPath(path);
        await window.ActivePane!.Ready;
        await window.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        return window;
    }

    private static void Edit(EditorPane pane)
    { pane.View!.Command("i"); pane.View.Text("Unsaved "); pane.View.Key(VIEM_KEY_ESCAPE); }

    internal static async Task Run(Preferences preferences)
    {
        string directory = Path.Combine(preferences.DirectoryPath, "external changes " + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        var windows = new List<EditorWindow>();
        try
        {
            await ActiveChanges(preferences, windows, directory);
            await InactiveChanges(preferences, windows, directory);
            await SharedAcknowledgement(preferences, windows, directory);
            await OwnSave(preferences, windows, directory);
            await MissingFileBaselines(preferences, windows, directory);
            await DeletionAndRecreation(preferences, windows, directory);
            await StaleReload(preferences, windows, directory);
            await RetargetedPaneReload(preferences, windows, directory);
        }
        finally
        {
            foreach (var window in windows.Where(App.Instance.Windows.Contains).ToArray())
                await window.RequestCloseForTesting().WaitAsync(CompletionTimeout);
            foreach (string path in Directory.EnumerateFiles(directory)) File.Delete(path);
            Directory.Delete(directory);
        }
    }

    private static async Task ActiveChanges(Preferences preferences, List<EditorWindow> windows, string directory)
    {
        string path = Path.Combine(directory, "active.txt");
        File.WriteAllBytes(path, "Original"u8.ToArray());
        var window = await OpenFile(preferences, windows, path);
        var document = window.ActivePane!.Document;
        int reviews = 0;
        var reviewed = Signal();
        window.ExternalFileReviewDecisionForTesting = (candidate, canReload) => {
            Check(candidate == document && canReload, "an existing changed file offers Reload for its current document");
            reviews++; reviewed.TrySetResult(); return Task.FromResult(ContentDialogResult.None);
        };

        File.WriteAllBytes(path, "External"u8.ToArray());
        await reviewed.Task.WaitAsync(NotificationTimeout);
        await window.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        Check(reviews == 1 && document.FormattedText() == "Original" && !document.IsDirty,
            "an active native window reviews an external write before the old two-second polling interval");
        Check(window.HasSavedBaselineForTesting(document, "Original"u8.ToArray()),
            "Keep My Version retains the saved baseline for later overwrite checks");

        reviewed = Signal();
        DateTime modified = File.GetLastWriteTimeUtc(path);
        long length = new FileInfo(path).Length;
        File.WriteAllBytes(path, "Replaced"u8.ToArray());
        File.SetLastWriteTimeUtc(path, modified);
        await reviewed.Task.WaitAsync(NotificationTimeout);
        await window.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        Check(new FileInfo(path).Length == length && File.GetLastWriteTimeUtc(path) == modified
            && reviews == 2 && document.FormattedText() == "Original",
            "file notifications detect changed bytes even when length and modification time are preserved");

        File.WriteAllBytes(path, "Replaced"u8.ToArray());
        await window.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        Check(reviews == 2, "duplicate notifications and identical-byte writes do not repeat an acknowledged review");
        await window.RequestCloseForTesting();
    }

    private static async Task InactiveChanges(Preferences preferences, List<EditorWindow> windows, string directory)
    {
        string path = Path.Combine(directory, "inactive.txt");
        File.WriteAllBytes(path, "Original"u8.ToArray());
        var window = await OpenFile(preferences, windows, path);
        int reviews = 0;
        var reviewed = Signal();
        window.ExternalFileReviewDecisionForTesting = (_, _) => {
            reviews++; reviewed.TrySetResult(); return Task.FromResult(ContentDialogResult.None);
        };
        var other = Open(preferences, windows);
        await other.ActivePane!.Ready;
        window.ExternalFileActiveForTesting = false;
        File.WriteAllBytes(path, "Changed while away"u8.ToArray());
        await window.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        Check(reviews == 0 && window.ActivePane!.Document.FormattedText() == "Original",
            "background file changes do not present notifications or change the editor buffer");

        window.ExternalFileActiveForTesting = true;
        window.Activate();
        await reviewed.Task.WaitAsync(NotificationTimeout);
        await window.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        Check(reviews == 1,
            "returning to the window immediately reviews changes deferred while the app was in the background");
        await other.RequestCloseForTesting();
        await window.RequestCloseForTesting();
    }

    private static async Task SharedAcknowledgement(Preferences preferences, List<EditorWindow> windows, string directory)
    {
        string path = Path.Combine(directory, "shared.txt");
        File.WriteAllBytes(path, "Original"u8.ToArray());
        var first = await OpenFile(preferences, windows, path);
        var document = first.ActivePane!.Document;
        var second = Open(preferences, windows, document);
        await second.ActivePane!.Ready;
        first.ExternalFileActiveForTesting = true;
        second.ExternalFileActiveForTesting = false;
        int reviews = 0;
        var reviewed = Signal();
        first.ExternalFileReviewDecisionForTesting = second.ExternalFileReviewDecisionForTesting = (candidate, _) => {
            Check(candidate == document, "external reviews retain a shared document identity across windows");
            reviews++; reviewed.TrySetResult(); return Task.FromResult(ContentDialogResult.None);
        };
        File.WriteAllBytes(path, "First change"u8.ToArray());
        await reviewed.Task.WaitAsync(NotificationTimeout);
        await first.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        first.ExternalFileActiveForTesting = false;
        second.ExternalFileActiveForTesting = true;
        second.Activate();
        second.ActivateExternalFileChecksForTesting();
        await second.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        Check(reviews == 1 && first.ActivePane!.Document == second.ActivePane!.Document
            && first.HasSavedBaselineForTesting(document, "Original"u8.ToArray())
            && second.HasSavedBaselineForTesting(document, "Original"u8.ToArray()),
            "Keep My Version is acknowledged across shared windows without replacing either saved baseline");

        reviewed = Signal();
        File.WriteAllBytes(path, "Another change"u8.ToArray());
        await reviewed.Task.WaitAsync(NotificationTimeout);
        await second.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        Check(reviews == 2 && document.FormattedText() == "Original",
            "a subsequent disk version is reviewed by the active shared window");
        await first.RequestCloseForTesting();
        await second.RequestCloseForTesting();
    }

    private static async Task OwnSave(Preferences preferences, List<EditorWindow> windows, string directory)
    {
        string path = Path.Combine(directory, "own-save.txt");
        File.WriteAllBytes(path, "Original"u8.ToArray());
        var window = await OpenFile(preferences, windows, path);
        var pane = window.ActivePane!;
        int reviews = 0;
        window.ExternalFileReviewDecisionForTesting = (_, _) => {
            reviews++; return Task.FromResult(ContentDialogResult.None);
        };
        Edit(pane);
        byte[] saved = pane.Document.Source(pane.Document.State.document_revision);
        bool success = await window.Save(pane);
        await window.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        Check(success && reviews == 0 && !pane.Document.IsDirty
            && File.ReadAllBytes(path).AsSpan().SequenceEqual(saved)
            && window.HasSavedBaselineForTesting(pane.Document, saved),
            "Viem's own atomic save updates the baseline without presenting an external-change notification");
        await window.RequestCloseForTesting();
    }

    private static async Task DeletionAndRecreation(Preferences preferences, List<EditorWindow> windows, string directory)
    {
        string path = Path.Combine(directory, "recreated.txt");
        File.WriteAllBytes(path, "Original"u8.ToArray());
        var window = await OpenFile(preferences, windows, path);
        var original = window.ActivePane!.Document;
        var reviewed = Signal();
        var availability = new List<bool>();
        window.ExternalFileReviewDecisionForTesting = (_, canReload) => {
            availability.Add(canReload); reviewed.TrySetResult();
            return Task.FromResult(canReload ? ContentDialogResult.Primary : ContentDialogResult.None);
        };
        File.Delete(path);
        await reviewed.Task.WaitAsync(NotificationTimeout);
        await window.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        Check(availability.SequenceEqual(new[] { false }) && window.ActivePane!.Document == original
            && original.FormattedText() == "Original",
            "deleting the file promptly reports its absence without offering Reload or discarding the buffer");

        reviewed = Signal();
        File.WriteAllBytes(path, "Recreated contents"u8.ToArray());
        await reviewed.Task.WaitAsync(NotificationTimeout);
        await window.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        var replacement = window.ActivePane!.Document;
        Check(availability.SequenceEqual(new[] { false, true }) && replacement != original
            && replacement.FormattedText() == "Recreated contents" && !replacement.IsDirty,
            "recreating a deleted file produces a new review and Reload installs its current contents");
        await window.RequestCloseForTesting();
    }

    private static async Task MissingFileBaselines(Preferences preferences, List<EditorWindow> windows, string directory)
    {
        string missing = Path.Combine(directory, "new-file.txt");
        var window = Open(preferences, windows);
        int reviews = 0;
        bool reloadAvailable = false;
        var reviewed = Signal();
        window.ExternalFileReviewDecisionForTesting = (_, canReload) => {
            reviews++; reloadAvailable = canReload; reviewed.TrySetResult();
            return Task.FromResult(ContentDialogResult.None);
        };
        await window.ActivePane!.Ready;
        await window.OpenPath(missing);
        await window.ActivePane!.Ready;
        await window.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        Check(reviews == 0 && !File.Exists(missing) && window.ActivePane!.Document.FilePath == missing
            && window.ActivePane.Document.FormattedText() == "",
            "opening a nonexistent file treats its absence as the baseline without a deletion notification");

        File.WriteAllBytes(missing, []);
        await reviewed.Task.WaitAsync(NotificationTimeout);
        await window.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        Check(reviews == 1 && reloadAvailable,
            "external creation of an empty file is a change from a nonexistent-file baseline");
        await window.RequestCloseForTesting();

        string original = Path.Combine(directory, "rename-original.txt");
        string renamed = Path.Combine(directory, "rename-missing.txt");
        File.WriteAllBytes(original, "Retained contents"u8.ToArray());
        window = await OpenFile(preferences, windows, original);
        var pane = window.ActivePane!;
        var document = pane.Document;
        reviews = 0; reloadAvailable = false; reviewed = Signal();
        window.ExternalFileReviewDecisionForTesting = (_, canReload) => {
            reviews++; reloadAvailable = canReload; reviewed.TrySetResult();
            return Task.FromResult(ContentDialogResult.None);
        };
        pane.View!.Ex("file " + renamed);
        await window.PendingEffectsForTesting.WaitAsync(CompletionTimeout);
        await window.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        Check(reviews == 0 && !File.Exists(renamed) && pane.Document == document
            && document.FilePath == renamed && document.FormattedText() == "Retained contents",
            "file rename to a nonexistent path records an absent baseline without reporting an external deletion");

        File.WriteAllBytes(renamed, []);
        await reviewed.Task.WaitAsync(NotificationTimeout);
        await window.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        Check(reviews == 1 && reloadAvailable && document.FormattedText() == "Retained contents",
            "external empty-file creation after file rename is reviewed without changing the retained buffer");
        await window.RequestCloseForTesting();
    }

    private static async Task StaleReload(Preferences preferences, List<EditorWindow> windows, string directory)
    {
        string path = Path.Combine(directory, "stale-reload.txt");
        File.WriteAllBytes(path, "Original"u8.ToArray());
        var window = await OpenFile(preferences, windows, path);
        var pane = window.ActivePane!;
        var document = pane.Document;
        var reviewing = Signal();
        var decision = new TaskCompletionSource<ContentDialogResult>(TaskCreationOptions.RunContinuationsAsynchronously);
        int reviews = 0;
        window.ExternalFileReviewDecisionForTesting = (_, _) => {
            reviews++; reviewing.TrySetResult();
            return reviews == 1 ? decision.Task : Task.FromResult(ContentDialogResult.None);
        };
        try
        {
            File.WriteAllBytes(path, "External contents"u8.ToArray());
            await reviewing.Task.WaitAsync(NotificationTimeout);
            Edit(pane);
            byte[] unsaved = document.Source(document.State.document_revision);
            decision.SetResult(ContentDialogResult.Primary);
            await window.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
            Check(window.ActivePane!.Document == document && document.IsDirty
                && document.Source(document.State.document_revision).AsSpan().SequenceEqual(unsaved)
                && window.HasSavedBaselineForTesting(document, "Original"u8.ToArray()),
                "Reload approval cannot discard edits made after the external-change review began");
        }
        finally { decision.TrySetResult(ContentDialogResult.None); }
        await window.RequestCloseForTesting();
    }

    private static async Task RetargetedPaneReload(Preferences preferences, List<EditorWindow> windows, string directory)
    {
        string originalPath = Path.Combine(directory, "retarget-original.txt");
        string replacementPath = Path.Combine(directory, "retarget-replacement.txt");
        File.WriteAllBytes(originalPath, "Original"u8.ToArray());
        File.WriteAllBytes(replacementPath, "Replacement"u8.ToArray());
        var first = await OpenFile(preferences, windows, originalPath);
        var pane = first.ActivePane!;
        var original = pane.Document;
        var second = Open(preferences, windows, original);
        await second.ActivePane!.Ready;
        first.ExternalFileActiveForTesting = true;
        second.ExternalFileActiveForTesting = false;
        var reviewing = Signal();
        var decision = new TaskCompletionSource<ContentDialogResult>(TaskCreationOptions.RunContinuationsAsynchronously);
        first.ExternalFileReviewDecisionForTesting = (_, _) => { reviewing.TrySetResult(); return decision.Task; };
        second.ExternalFileReviewDecisionForTesting = (_, _) => Task.FromResult(ContentDialogResult.None);
        try
        {
            File.WriteAllBytes(originalPath, "External original contents"u8.ToArray());
            await reviewing.Task.WaitAsync(NotificationTimeout);
            // Retain the native pane identity while changing the document it
            // displays, as replacement publication does. The second window
            // keeps the reviewed document and its in-flight check alive.
            Task pending = first.CheckExternalChangesForTesting();
            var replacement = new CoreDocument("Replacement"u8.ToArray(), replacementPath);
            try { pane.InstallReplacement(replacement, pane.PrepareReplacement(replacement)); }
            catch { replacement.Dispose(); throw; }
            decision.SetResult(ContentDialogResult.Primary);
            await pending.WaitAsync(CompletionTimeout);
            Check(first.ActivePane == pane && pane.Document == replacement && replacement.FormattedText() == "Replacement"
                && replacement.FilePath == replacementPath && second.ActivePane!.Document == original
                && original.FormattedText() == "Original",
                "stale Reload approval cannot overwrite a different document installed into the reviewed pane");
        }
        finally { decision.TrySetResult(ContentDialogResult.None); }
        await first.RequestCloseForTesting();
        await second.RequestCloseForTesting();
    }
}
#endif

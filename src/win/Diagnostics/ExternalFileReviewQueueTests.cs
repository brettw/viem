#if DEBUG
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Shell;

namespace Viem.Windows.Diagnostics;

internal static partial class ExternalFileChangeTests
{
    private static async Task BatchedActivation(Preferences preferences, List<EditorWindow> windows, string directory)
    {
        string firstPath = Path.Combine(directory, "batch-first.txt");
        string secondPath = Path.Combine(directory, "batch-second.txt");
        File.WriteAllText(firstPath, "First");
        File.WriteAllText(secondPath, "Second");
        var first = await OpenFile(preferences, windows, firstPath);
        var second = await OpenFile(preferences, windows, secondPath);
        var firstDocument = first.ActivePane!.Document;
        var secondDocument = second.ActivePane!.Document;
        var shared = Open(preferences, windows, firstDocument);
        await shared.ActivePane!.Ready;
        Edit(second.ActivePane);
        string unsaved = secondDocument.FormattedText();
        first.ExternalFileActiveForTesting = second.ExternalFileActiveForTesting = shared.ExternalFileActiveForTesting = false;
        int reviews = 0;
        EditorWindow.ExternalFileReviewDialog? firstDialog = null;
        first.ExternalFileBatchDecisionForTesting = dialog => {
            reviews++;
            firstDialog = dialog;
            return Task.FromResult(ContentDialogResult.None);
        };
        File.WriteAllText(firstPath, "Disk first");
        File.WriteAllText(secondPath, "Disk second");
        await first.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        Check(reviews == 0, "multiple changed files remain queued while the application is inactive");
        first.ExternalFileActiveForTesting = true;
        await first.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        await first.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        Check(firstDialog is { Documents.Count: 2 } && firstDialog.Documents.Contains(firstDocument) && firstDialog.Documents.Contains(secondDocument),
            "activation batches changed files across windows and deduplicates shared views");
        Check(firstDialog!.Title == "Files changed on disk" && firstDialog.Primary == "Reload all" && firstDialog.Close == "Keep My Version all"
            && firstDialog.Text.Contains(firstPath) && firstDialog.Text.Contains(secondPath) && firstDialog.Text.Contains("discard your unsaved edits"),
            "batch review lists full paths, warns about dirty buffers and appends all to both buttons");
        Check(reviews == 1 && firstDocument.FormattedText() == "First" && secondDocument.FormattedText() == unsaved
            && secondDocument.IsDirty && first.HasSavedBaselineForTesting(firstDocument, "First"u8.ToArray())
            && second.HasSavedBaselineForTesting(secondDocument, "Second"u8.ToArray()),
            "Keep My Version all acknowledges every listed change while preserving buffers and save baselines");

        first.ExternalFileActiveForTesting = false;
        File.WriteAllText(firstPath, "New disk first");
        File.WriteAllText(secondPath, "New disk second");
        var presenting = Signal();
        var decision = new TaskCompletionSource<ContentDialogResult>(TaskCreationOptions.RunContinuationsAsynchronously);
        EditorWindow.ExternalFileReviewDialog? reloadDialog = null;
        first.ExternalFileBatchDecisionForTesting = dialog => {
            reviews++;
            reloadDialog = dialog;
            presenting.TrySetResult();
            return decision.Task;
        };
        try
        {
            first.ExternalFileActiveForTesting = true;
            Task pending = first.CheckExternalChangesForTesting();
            await presenting.Task.WaitAsync(CompletionTimeout);
            Check(reloadDialog is { Documents.Count: 2 }, "later changes to both acknowledged files form a fresh batch");
            // Exercise the same pending flags set by duplicate watcher callbacks
            // while the prompt is open, without relying on OS event timing.
            first.ActivateExternalFileChecksForTesting();
            second.ActivateExternalFileChecksForTesting();
            decision.SetResult(ContentDialogResult.Primary);
            await pending.WaitAsync(CompletionTimeout);
            await first.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
            Check(reviews == 2 && first.ActivePane.Document != firstDocument && second.ActivePane.Document != secondDocument
                && first.ActivePane.Document.FormattedText() == "New disk first" && second.ActivePane.Document.FormattedText() == "New disk second"
                && shared.ActivePane.Document == first.ActivePane.Document && !second.ActivePane.Document.IsDirty,
                "Reload all replaces every listed buffer and shared view, clearing queued duplicate flags");
        }
        finally { decision.TrySetResult(ContentDialogResult.None); }
        await shared.RequestCloseForTesting();
        await second.RequestCloseForTesting();
        await first.RequestCloseForTesting();
    }

    private static async Task ChangesDuringReview(Preferences preferences, List<EditorWindow> windows, string directory,
        bool changeFirstAgain, bool keepFirst = false)
    {
        string firstPath = Path.Combine(directory, "during-first.txt");
        string secondPath = Path.Combine(directory, "during-second.txt");
        File.WriteAllText(firstPath, "First");
        File.WriteAllText(secondPath, "Second");
        var first = await OpenFile(preferences, windows, firstPath);
        var second = await OpenFile(preferences, windows, secondPath);
        second.ExternalFileActiveForTesting = false;
        var presenting = Signal();
        var decision = new TaskCompletionSource<ContentDialogResult>(TaskCreationOptions.RunContinuationsAsynchronously);
        var shown = new List<EditorWindow.ExternalFileReviewDialog>();
        bool overlapped = false;
        first.ExternalFileBatchDecisionForTesting = dialog => {
            shown.Add(dialog);
            if (shown.Count == 1) { presenting.TrySetResult(); return decision.Task; }
            overlapped |= !decision.Task.IsCompleted;
            return Task.FromResult(ContentDialogResult.None);
        };
        try
        {
            File.WriteAllText(firstPath, "First change");
            Task pending = first.CheckExternalChangesForTesting();
            await presenting.Task.WaitAsync(CompletionTimeout);
            Check(shown[0].Documents.Count == 1 && shown[0].Title == "File changed on disk"
                && shown[0].Primary == "Reload" && shown[0].Close == "Keep My Version"
                && shown[0].Text == "during-first.txt was changed by another program.",
                "a one-file batch keeps the existing single-file dialog");
            if (changeFirstAgain) File.WriteAllText(firstPath, "Newer first change");
            File.WriteAllText(secondPath, "Second change");
            first.ActivateExternalFileChecksForTesting();
            Check(shown.Count == 1 && shown[0].Documents.Count == 1,
                "a displayed review never expands its list or opens a concurrent file review");
            decision.SetResult(keepFirst ? ContentDialogResult.None : ContentDialogResult.Primary);
            await pending.WaitAsync(CompletionTimeout);
            Check(!overlapped && shown.Count == 2 && shown[1].Documents.Count == (changeFirstAgain ? 2 : 1)
                && shown[1].Primary == (changeFirstAgain ? "Reload all" : "Reload"),
                "changes arriving during presentation wait for dismissal and form the next batch");
            Check(shown.Count == 2 && first.ActivePane!.Document.FormattedText() == (changeFirstAgain ? "First" : "First change")
                && second.ActivePane!.Document.FormattedText() == "Second",
                keepFirst ? "Keep acknowledges only the displayed disk state and preserves later notifications for the next batch"
                    : changeFirstAgain ? "stale Reload consent cannot load newer disk bytes or apply to files queued after presentation"
                    : "successful Reload clears only its own duplicate flag and leaves later files queued for separate consent");
            await first.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
            Check(shown.Count == 2, "Keep all suppresses repeat reviews of the second batch");
        }
        finally { decision.TrySetResult(ContentDialogResult.None); }
        await second.RequestCloseForTesting();
        await first.RequestCloseForTesting();
    }

    private static async Task MixedBatchReload(Preferences preferences, List<EditorWindow> windows, string directory)
    {
        string firstPath = Path.Combine(directory, "mixed-reload.txt");
        string secondPath = Path.Combine(directory, "mixed-stale.txt");
        string deletedPath = Path.Combine(directory, "mixed-deleted.txt");
        File.WriteAllText(firstPath, "First");
        File.WriteAllText(secondPath, "Second");
        File.WriteAllText(deletedPath, "Deleted");
        var first = await OpenFile(preferences, windows, firstPath);
        var second = await OpenFile(preferences, windows, secondPath);
        var deleted = await OpenFile(preferences, windows, deletedPath);
        first.ExternalFileActiveForTesting = second.ExternalFileActiveForTesting = deleted.ExternalFileActiveForTesting = false;
        File.WriteAllText(firstPath, "Reloaded");
        File.WriteAllText(secondPath, "Stale");
        File.Delete(deletedPath);
        int reviews = 0;
        var shown = new List<EditorWindow.ExternalFileReviewDialog>();
        first.ExternalFileBatchDecisionForTesting = dialog => {
            shown.Add(dialog);
            if (++reviews == 1)
            {
                Edit(second.ActivePane!);
                return Task.FromResult(ContentDialogResult.Primary);
            }
            return Task.FromResult(ContentDialogResult.None);
        };
        first.ExternalFileActiveForTesting = true;
        await first.CheckExternalChangesForTesting().WaitAsync(CompletionTimeout);
        Check(shown.Count == 2 && shown[0].Documents.Count == 3 && shown[0].Primary == "Reload all" && shown[0].Text.Contains(deletedPath)
            && shown[0].Text.Contains("Deleted or unreadable files keep their current buffers."),
            "mixed batches explain which files cannot be reloaded");
        Check(shown[1].Documents.Count == 1 && shown[1].Documents[0] == second.ActivePane!.Document,
            "a stale member of Reload all is queued separately for fresh consent");
        Check(reviews == 2 && first.ActivePane!.Document.FormattedText() == "Reloaded"
            && second.ActivePane!.Document.FormattedText() == "Unsaved Second" && second.ActivePane.Document.IsDirty
            && deleted.ActivePane!.Document.FormattedText() == "Deleted" && !File.Exists(deletedPath),
            "Reload all handles each member independently, preserving deleted files and edits made during review");
        await deleted.RequestCloseForTesting();
        await second.RequestCloseForTesting();
        await first.RequestCloseForTesting();
    }
}
#endif

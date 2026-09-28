#if DEBUG
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Shell;
using Windows.System;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class DocumentCloseReviewTests
{
    private static void Check(bool value, string name)
    { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }

    private static EditorWindow Open(Preferences preferences, CoreDocument document)
    {
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window);
        window.Closed += (_, _) => App.Instance.Windows.Remove(window);
        window.Activate();
        return window;
    }

    private static async Task Ex(EditorWindow window, EditorPane pane, string command)
    {
        window.Activate(); pane.FocusEditor();
        await Task.Delay(100);
        await InputRoutingTests.Text(":" + command);
        await InputRoutingTests.Key(VirtualKey.Enter);
        await window.PendingEffectsForTesting;
    }

    internal static async Task Run(Preferences preferences)
    {
        var document = new CoreDocument("Shared original"u8.ToArray());
        var window = Open(preferences, document);
        int reviews = 0;
        ContentDialogResult decision = ContentDialogResult.None;
        window.CloseReviewDecisionForTesting = _ => { reviews++; return Task.FromResult(decision); };
        try
        {
            var first = window.ActivePane!;
            await first.Ready;
            await Ex(window, first, "split");
            var second = window.ActivePane!;
            await second.Ready;
            first.View!.Command("i"); first.View.Text("Unsaved "); first.View.Key(VIEM_KEY_ESCAPE);
            byte[] unsaved = document.Source(document.State.document_revision);
            Check(window.Panes.Count == 2 && second.Document == document && document.IsDirty,
                "Ex split shares one dirty document across two panes");

            await Ex(window, second, "q");
            Check(reviews == 0 && window.Panes.Count == 1 && window.Panes.Contains(first)
                && document.IsDirty && document.Source(document.State.document_revision).AsSpan().SequenceEqual(unsaved),
                "native Ex quit closes one shared pane without review or loss of unsaved text");

            await Ex(window, first, "q");
            Check(reviews == 1 && window.Panes.Contains(first) && document.IsDirty,
                "last dirty pane reviews once and Cancel preserves it");
            decision = ContentDialogResult.Secondary;
            await Ex(window, first, "q");
            Check(reviews == 2 && document.Handle == 0,
                "last dirty pane closes only after Discard approval");
        }
        finally { if (App.Instance.Windows.Contains(window)) window.Close(); }

        document = new CoreDocument("Shared across windows"u8.ToArray());
        var firstWindow = Open(preferences, document);
        var secondWindow = Open(preferences, document);
        reviews = 0; decision = ContentDialogResult.None;
        firstWindow.CloseReviewDecisionForTesting = secondWindow.CloseReviewDecisionForTesting =
            _ => { reviews++; return Task.FromResult(decision); };
        try
        {
            var first = firstWindow.ActivePane!; var second = secondWindow.ActivePane!;
            await first.Ready; await second.Ready;
            first.View!.Command("i"); first.View.Text("Unsaved "); first.View.Key(VIEM_KEY_ESCAPE);
            await Ex(firstWindow, first, "q");
            Check(reviews == 0 && document.Handle != 0 && document.IsDirty && second.View != null,
                "Ex quit closes a shared document window without reviewing the surviving view");

            second.View!.Ex("split");
            await secondWindow.PendingEffectsForTesting;
            await secondWindow.ActivePane!.Ready;
            await secondWindow.RequestCloseForTesting();
            Check(reviews == 1 && secondWindow.Panes.Count == 2 && document.IsDirty,
                "native window closure reviews a dirty document once across all its panes");
            decision = ContentDialogResult.Secondary;
            await secondWindow.RequestCloseForTesting();
            Check(reviews == 2 && document.Handle == 0,
                "native final-window Discard closes all shared views after one review");
        }
        finally
        {
            if (App.Instance.Windows.Contains(firstWindow)) firstWindow.Close();
            if (App.Instance.Windows.Contains(secondWindow)) secondWindow.Close();
        }

        await ConcurrentOpen(preferences);
    }

    private static async Task ConcurrentOpen(Preferences preferences)
    {
        string path = Path.Combine(preferences.DirectoryPath, "concurrent-open.txt");
        await File.WriteAllTextAsync(path, "One file shared across windows");
        byte[] baseline = await File.ReadAllBytesAsync(path);
        var firstWindow = Open(preferences, new CoreDocument([]));
        var secondWindow = Open(preferences, new CoreDocument([]));
        var bothConstructed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        int waiting = 0, reviews = 0;
        Task BeforePublish()
        {
            if (++waiting == 2) bothConstructed.SetResult();
            return bothConstructed.Task;
        }
        firstWindow.BeforePublishOpenForTesting = secondWindow.BeforePublishOpenForTesting = BeforePublish;
        firstWindow.CloseReviewDecisionForTesting = secondWindow.CloseReviewDecisionForTesting =
            _ => { reviews++; return Task.FromResult(ContentDialogResult.None); };
        try
        {
            await firstWindow.ActivePane!.Ready; await secondWindow.ActivePane!.Ready;
            await Task.WhenAll(firstWindow.OpenPath(path, split: true), secondWindow.OpenPath(path, split: true));
            var first = firstWindow.ActivePane!; var second = secondWindow.ActivePane!;
            await first.Ready; await second.Ready;
            Check(first.Document == second.Document,
                "concurrent file opens publish one shared document identity");
            Check(firstWindow.HasSavedBaselineForTesting(first.Document, baseline)
                && secondWindow.HasSavedBaselineForTesting(second.Document, baseline),
                "each window adopts the shared document's saved baseline for external-change checks");
            first.View!.Command("i"); first.View.Text("Unsaved "); first.View.Key(VIEM_KEY_ESCAPE);
            byte[] unsaved = first.Document.Source(first.Document.State.document_revision);
            await Ex(firstWindow, first, "q");
            Check(reviews == 0 && second.Document.IsDirty && second.Document.Source(second.Document.State.document_revision).AsSpan().SequenceEqual(unsaved),
                "native Ex quit after concurrent opens skips review while the shared dirty file survives");
            firstWindow.Close();
            Check(secondWindow.HasSavedBaselineForTesting(second.Document, baseline),
                "surviving shared view retains its saved baseline after the other window closes");
        }
        finally
        {
            if (App.Instance.Windows.Contains(firstWindow)) firstWindow.Close();
            if (App.Instance.Windows.Contains(secondWindow)) secondWindow.Close();
            File.Delete(path);
        }
    }
}
#endif

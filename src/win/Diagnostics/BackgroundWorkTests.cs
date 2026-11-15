#if DEBUG
using System.Text;
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Dispatching;
using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class BackgroundWorkTests
{
    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
        FrontendSmokeTests.UiChecks.Add(message);
    }

    internal static async Task Run(CanvasDevice device, DispatcherQueue dispatcher)
    {
        await SettledDocument(device, dispatcher);
        await SyntaxCompletion(device, dispatcher);
        await SharedLargeCodeSettles(device, dispatcher);
        await MarkdownSyntaxAndPreparationSettle(device, dispatcher);
        await CompletionProgress(device, dispatcher);
        await SharedSearchProgress(device, dispatcher);
        await QueuedWakeAfterClose(device, dispatcher);
        await FailedViewAttachment(device, dispatcher);
    }

    private static async Task SettledDocument(CanvasDevice device, DispatcherQueue dispatcher)
    {
        using var document = new CoreDocument("plain text"u8.ToArray(), format: VIEM_FORMAT_PLAIN_TEXT);
        using var first = new CoreView(document, device, dispatcher, 400, 200);
        using var second = new CoreView(document, device, dispatcher, 400, 200);
        first.BackgroundLayout.Enabled = second.BackgroundLayout.Enabled = false;
        Check(await SelfTestRunner.Until(() => document.BackgroundWorkDrainsForTesting > 0
            && !document.BackgroundWorkScheduledForTesting
            && !first.PendingWorkScheduledForTesting && !second.PendingWorkScheduledForTesting),
            "Text background work reaches idle");
        int drains = document.BackgroundWorkDrainsForTesting;
        for (int i = 0; i < 100; i++) document.WakeBackgroundWorkForTesting();
        Check(await SelfTestRunner.Until(() => document.BackgroundWorkDrainsForTesting == drains + 1
            && !document.BackgroundWorkScheduledForTesting), "shared views coalesce a burst into one document drain");
        drains = document.BackgroundWorkDrainsForTesting;
        // This interval asserts the absence of work after observable readiness;
        // it is not a delay used to make asynchronous setup succeed.
        await Task.Delay(200);
        Check(document.BackgroundWorkDrainsForTesting == drains
            && !document.BackgroundWorkScheduledForTesting
            && !first.PendingWorkScheduledForTesting && !second.PendingWorkScheduledForTesting,
            "settled Text views schedule no maintenance callbacks");
        Check(document.BackgroundWorkError == null, "Text background collection has no error");
    }

    private static async Task SyntaxCompletion(CanvasDevice device, DispatcherQueue dispatcher)
    {
        using var document = new CoreDocument("set number\nlet g:example = 'value'\n"u8.ToArray(), path: ".vimrc");
        using var view = new CoreView(document, device, dispatcher, 400, 200);
        view.BackgroundLayout.Enabled = false;
        // Only observe accepted state. Calling ServiceBackgroundWork or WaitForSyntax here
        // would conceal a missing worker-to-owner notification.
        Check(await SelfTestRunner.Until(() => document.SyntaxStyleNames().Length > 0, 10_000),
            "worker completion publishes syntax without frontend polling");
        Check(await SelfTestRunner.Until(() => !document.BackgroundWorkScheduledForTesting, 10_000),
            "syntax editing-pause deadline retires after useful work");
        Check(document.BackgroundWorkError == null, "syntax notification collection has no error");
        var original = document.Source(document.State.document_revision);
        view.Command("A"); view.Text(" "); view.Key(VIEM_KEY_ESCAPE);
        Check(await SelfTestRunner.Until(() => !document.BackgroundWorkScheduledForTesting, 10_000),
            "editing restarts and settles background work without a repeating timer");
        view.Undo();
        Check(document.Source(document.State.document_revision).AsSpan().SequenceEqual(original),
            "background notification publication preserves exact source undo");
    }

    private static async Task CompletionProgress(CanvasDevice device, DispatcherQueue dispatcher)
    {
        // Cross the core's 16 KiB completion slice so the native scheduler
        // must advance the request after the initiating input returns.
        var source = Encoding.UTF8.GetBytes("First\nSecond\n" + string.Concat(Enumerable.Repeat("other\n", 6_000)));
        using var document = new CoreDocument(source, format: VIEM_FORMAT_PLAIN_TEXT);
        using var view = new CoreView(document, device, dispatcher, 400, 200);
        view.BackgroundLayout.Enabled = false;
        view.Command("j0i"); view.Text("Fi"); view.Key(VIEM_KEY_CONTROL_CHARACTER, 'n');
        Check(view.PendingWorkScheduledForTesting, "incremental completion arms its pending-only timer");
        Check(await SelfTestRunner.Until(() => (view.Completion().Info.flags & VIEM_COMPLETION_SEARCHING) == 0),
            "completion progresses without the window maintenance timer");
        Check(view.Completion().Items.Contains("First"), "completion still finds words from the document");
        view.Key(VIEM_KEY_ESCAPE);
        Check(!view.PendingWorkScheduledForTesting && view.PendingWorkError == null,
            "completion leaves no pending-work timer after cancellation");
    }

    private static async Task SharedLargeCodeSettles(CanvasDevice device, DispatcherQueue dispatcher)
    {
        // Just beyond the 256 KiB complete-highlighting threshold. Each view
        // must receive its own distant visible coverage from worker signals.
        const string line = "fn item() { let value = 42; }\n";
        byte[] source = Encoding.UTF8.GetBytes(string.Concat(Enumerable.Repeat(line, 10_000)));
        using var document = new CoreDocument(source, path: "background.rs");
        using var first = new CoreView(document, device, dispatcher, 400, 200);
        using var second = new CoreView(document, device, dispatcher, 400, 200);
        first.BackgroundLayout.Enabled = second.BackgroundLayout.Enabled = false;
        second.GoToLine(8_001);
        void RefreshViews()
        {
            // Match native panes' event-driven materialization. There is no
            // syntax wait or explicit background collector in this fixture.
            if (first.Id != 0) first.Resize(400, 200, notify: false);
            if (second.Id != 0) second.Resize(400, 200, notify: false);
        }
        bool Colored(CoreView view, ulong at)
        {
            try { return view.Layout().PaintRuns.Any(run => run.text_start <= at && run.text_end >= at + 2
                && (run.paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0); }
            catch (Viem.Windows.Interop.CoreException error) when (error.Status == VIEM_STATUS_LAYOUT_UNAVAILABLE) { return false; }
        }
        document.SyntaxChanged += RefreshViews;
        try
        {
            Check(await SelfTestRunner.Until(() => Colored(first, 0) && Colored(second, (ulong)(8_000 * line.Length))
                && !document.BackgroundWorkScheduledForTesting, 10_000),
                "distant shared Code views receive current syntax through worker notifications");
            int drains = document.BackgroundWorkDrainsForTesting;
            await Task.Delay(600);
            Check(document.BackgroundWorkDrainsForTesting == drains && !document.BackgroundWorkScheduledForTesting
                && !first.PendingWorkScheduledForTesting && !second.PendingWorkScheduledForTesting,
                "settled shared Code regions schedule no recurring maintenance callbacks");
            first.Dispose();
            second.GoToLine(9_501);
            Check(await SelfTestRunner.Until(() => Colored(second, (ulong)(9_500 * line.Length))
                && !document.BackgroundWorkScheduledForTesting, 10_000),
                "closing one shared Code view preserves completion delivery to the surviving view");
            Check(document.BackgroundWorkError == null && document.Source(document.State.document_revision).AsSpan().SequenceEqual(source),
                "shared Code notification delivery preserves exact source and reports no collection errors");
        }
        finally { document.SyntaxChanged -= RefreshViews; }
    }

    private static async Task MarkdownSyntaxAndPreparationSettle(CanvasDevice device, DispatcherQueue dispatcher)
    {
        byte[] source = "before\n\n```rust\nfn main() { let value = 42; }\n```\n\n```vim\nset number\n```\n\nafter"u8.ToArray();
        using var document = new CoreDocument(source, path: "background.md", format: VIEM_FORMAT_MARKDOWN);
        using var view = new CoreView(document, device, dispatcher, 500, 600);
        view.BackgroundLayout.Enabled = false;
        string formatted = view.FormattedText();
        ulong rust = (ulong)formatted.IndexOf("fn main", StringComparison.Ordinal);
        ulong vim = (ulong)formatted.IndexOf("set number", StringComparison.Ordinal);
        void RefreshView() => view.Resize(500, 600, notify: false);
        bool Colored(params ulong[] offsets)
        {
            try
            {
                var runs = view.Layout().PaintRuns;
                return offsets.All(at => runs.Any(run => run.text_start <= at && run.text_end >= at + 2
                    && (run.paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0));
            }
            catch (Viem.Windows.Interop.CoreException error) when (error.Status == VIEM_STATUS_LAYOUT_UNAVAILABLE) { return false; }
        }
        async Task BecomesQuiet(string message)
        {
            var deadline = System.Diagnostics.Stopwatch.StartNew();
            while (deadline.Elapsed.TotalSeconds < 10)
            {
                if (!await SelfTestRunner.Until(() => !document.BackgroundWorkScheduledForTesting && !view.PendingWorkScheduledForTesting, 10_000)) break;
                int drains = document.BackgroundWorkDrainsForTesting;
                // Visible syntax is already ready. Alternate preparation can
                // still complete once after its deadline; require eventual
                // silence longer than that 400 ms deadline, without forcing
                // collection or claiming a private preparation-ready state.
                await Task.Delay(600);
                if (document.BackgroundWorkDrainsForTesting == drains && !document.BackgroundWorkScheduledForTesting)
                { Check(true, message); return; }
            }
            Check(false, message);
        }
        document.SyntaxChanged += RefreshView;
        try
        {
            Check(await SelfTestRunner.Until(() => Colored(rust, vim), 10_000),
                "Markdown Rust and Vim fences receive syntax through worker notifications");
            await BecomesQuiet("Markdown syntax and delayed preparation leave no recurring background callbacks");
            view.SetDisplay(VIEM_VIEW_DISPLAY_SOURCE);
            RefreshView();
            ulong sourceRust = (ulong)Encoding.UTF8.GetString(source).IndexOf("fn main", StringComparison.Ordinal);
            Check(await SelfTestRunner.Until(() => Colored(sourceRust), 10_000),
                "switching Markdown to Code receives syntax without manual background collection");
            await BecomesQuiet("Markdown source view settles after its presentation switch");
            view.SetDisplay(VIEM_VIEW_DISPLAY_PREVIEW);
            RefreshView();
            Check(await SelfTestRunner.Until(() => Colored(rust, vim), 10_000),
                "returning to Markdown Preview restores both fenced syntax regions");
            await BecomesQuiet("returning to Markdown Preview leaves no recurring background callbacks");
            Check((document.State.flags & (VIEM_DOCUMENT_STATE_CAN_UNDO | VIEM_DOCUMENT_STATE_CAN_REDO)) == 0
                && !document.IsDirty && document.BackgroundWorkError == null
                && document.Source(document.State.document_revision).AsSpan().SequenceEqual(source),
                "Markdown background work and toggles preserve source, empty history and clean state");
        }
        finally { document.SyntaxChanged -= RefreshView; }
    }

    private static async Task QueuedWakeAfterClose(CanvasDevice device, DispatcherQueue dispatcher)
    {
        var document = new CoreDocument("closing"u8.ToArray(), format: VIEM_FORMAT_PLAIN_TEXT);
        var view = new CoreView(document, device, dispatcher, 400, 200);
        document.WakeBackgroundWorkForTesting();
        view.Dispose(); document.Dispose();
        int drains = document.BackgroundWorkDrainsForTesting;
        var delivered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        Check(dispatcher.TryEnqueue(() => delivered.SetResult()), "close test queues an owner-thread barrier");
        await delivered.Task;
        Check(document.BackgroundWorkDrainsForTesting == drains && document.BackgroundWorkError == null,
            "queued notification after close cannot access the destroyed core");
    }

    private static async Task SharedSearchProgress(CanvasDevice device, DispatcherQueue dispatcher)
    {
        using var document = new CoreDocument("foo bar foo"u8.ToArray(), format: VIEM_FORMAT_PLAIN_TEXT);
        using var first = new CoreView(document, device, dispatcher, 400, 200);
        using var second = new CoreView(document, device, dispatcher, 400, 200);
        first.BackgroundLayout.Enabled = second.BackgroundLayout.Enabled = false;
        Check(await SelfTestRunner.Until(() => !first.PendingWorkScheduledForTesting && !second.PendingWorkScheduledForTesting),
            "shared search fixture settles before changing its pattern");
        first.ExecuteCommandLine("set hls");
        first.Command("/"); first.Text("foo"); first.Key(VIEM_KEY_ENTER);
        Check(await SelfTestRunner.Until(() => !first.PendingWorkScheduledForTesting && !second.PendingWorkScheduledForTesting),
            "search changes reach every shared view without window polling");
        Check(second.Layout().PaintRuns.Any(run => run.text_start == 0
            && (run.paint.flags & VIEM_TEXT_PAINT_HAS_BACKGROUND) != 0),
            "inactive shared view publishes the new search highlight");
        first.ExecuteCommandLine("noh");
        Check(await SelfTestRunner.Until(() => !second.PendingWorkScheduledForTesting),
            "clearing shared search highlights settles without another input in the inactive pane");
        Check(!second.Layout().PaintRuns.Any(run => (run.paint.flags & VIEM_TEXT_PAINT_HAS_BACKGROUND) != 0),
            "inactive shared view removes the suppressed search highlight");
    }

    private static async Task FailedViewAttachment(CanvasDevice device, DispatcherQueue dispatcher)
    {
        using var document = new CoreDocument("attachment"u8.ToArray(), format: VIEM_FORMAT_PLAIN_TEXT);
        CoreView? failed = null;
        document.BeforeViewBackgroundAttachmentForTesting = view =>
        {
            failed = view;
            throw new IOException("Injected background attachment failure.");
        };
        bool rejected = false;
        try { using var unexpected = new CoreView(document, device, dispatcher, 400, 200); }
        catch (IOException error) when (error.Message == "Injected background attachment failure.") { rejected = true; }
        finally { document.BeforeViewBackgroundAttachmentForTesting = null; }
        Check(rejected && failed is { Id: 0 } && !failed.PendingWorkScheduledForTesting,
            "failed background attachment immediately removes the native view and pending timer");
        Check(await SelfTestRunner.Until(() => failed!.Provider.LiveResourceCount == 0
            && failed.BackgroundLayout.IsIdle && failed.TableWidthRefinement.IsIdle, 10_000),
            "failed background attachment releases native resources after queued or active layout work retires");
        using var replacement = new CoreView(document, device, dispatcher, 400, 200);
        Check(replacement.Id != 0 && replacement.Layout().Rows.Length > 0,
            "a document remains usable after a failed view background attachment");
    }
}
#endif

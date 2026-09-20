using Microsoft.UI.Dispatching;
using Viem.Windows.Interop;
using Viem.Windows.Rendering;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

// One speculative chunk per view and one computing worker across the app.
// No timer, unbounded backlog, mutable core access or UI callback on the worker.
internal sealed class BackgroundLayout(CoreView view, DispatcherQueue dispatcher) : IDisposable
{
    private static readonly SemaphoreSlim workerSlot = new(1, 1);
    private readonly record struct Dependencies(ulong Document, ulong Revision, ulong Configuration, ulong Environment, ulong Metrics);
    private Dependencies? dependencies;
    private float top, height;
    private int direction = 1;
    private bool queued, disposed, failed;
    // Also bound retries when a deliberately tiny cache cannot retain a band.
    private int remainingChunks;
    private Work? active;
    public bool IsIdle => !queued && active == null;
    public string? LastError { get; private set; }
    public int Started { get; private set; }
    public int Installed { get; private set; }
    public int Discarded { get; private set; }
    public bool Enabled { get; set; } =
#if DEBUG
        Environment.GetEnvironmentVariable("VIEM_TEST_DISABLE_PRELAYOUT") != "1";
#else
        true;
#endif

    public void Update()
    {
        if (disposed || !Enabled || view.Id == 0) return;
        var state = view.Viewport;
        if ((state.flags & VIEM_VIEWPORT_STATE_HAS_LAYOUT) == 0)
        { active?.Cancel(); dependencies = null; return; }
        var next = new Dependencies(state.document_id, state.document_revision, state.configuration_generation,
            state.measurement_environment_id, state.metrics_generation);
        if (dependencies == next && top == state.top) return;
        int nextDirection = state.top > top ? 1 : state.top < top ? -1 : direction;
        if (dependencies != next || nextDirection != direction || Math.Abs(state.top - top) > height * 3)
            active?.Cancel();
        dependencies = next; top = state.top; height = view.LayoutInfo().viewport_height; direction = nextDirection;
        remainingChunks = 32;
        failed = false; LastError = null;
        Queue();
    }

    private void Queue()
    {
        if (disposed || !Enabled || queued || active != null || failed || remainingChunks == 0) return;
        queued = dispatcher.TryEnqueue(DispatcherQueuePriority.Low, () => {
            queued = false;
            if (disposed || !Enabled || active != null || failed) return;
            try { Start(); }
            catch (Exception error) { failed = true; LastError = error.ToString(); }
        });
    }

    private unsafe void Start()
    {
        ulong request = 0;
        using (Diagnostics.InputPerformance.Measure("prelayout.capture"))
            Check(viem_core_view_prepare_prelayout(view.Document.Handle, view.Id, direction, &request), "Prepare background layout");
        if (request == 0) return;
        try { active = new Work(request, view.Provider.CaptureWorkerFactory()); }
        catch { viem_layout_work_release(request); throw; }
        remainingChunks--;
        Started++;
        _ = Run(active);
    }

    private async Task Run(Work work)
    {
        ulong result = 0;
        Exception? failure = null;
        try
        {
            await workerSlot.WaitAsync(work.Cancellation.Token).ConfigureAwait(false);
            try { result = await Task.Run(() => Compute(work)).ConfigureAwait(false); }
            finally { workerSlot.Release(); }
        }
        catch (OperationCanceledException) { }
        catch (Exception error) { failure = error; }
        // Publish completed cache entries ahead of later input. Leaving this
        // at Low can make the next Page Down synchronously shape work that is
        // already finished. Capture remains low priority; installation is a
        // bounded cache-only update and never shapes or redraws.
        if (!dispatcher.TryEnqueue(DispatcherQueuePriority.Normal, () => Complete(work, result, failure)))
        {
            if (result != 0) viem_layout_work_release(result);
            work.Dispose();
        }
    }

    private static unsafe ulong Compute(Work work)
    {
        work.Cancellation.Token.ThrowIfCancellationRequested();
        using var provider = work.Provider();
        var table = provider.Table;
        ulong result = 0;
        Check(viem_layout_work_compute(work.Request, &table, &result), provider.LastError ?? "Compute background layout");
        return result;
    }

    private unsafe void Complete(Work work, ulong result, Exception? failure)
    {
        active = null;
        try
        {
            if (failure != null && !work.Cancellation.IsCancellationRequested && !disposed)
            { failed = true; LastError = failure.ToString(); }
            if (!disposed && Enabled && !work.Cancellation.IsCancellationRequested && result != 0)
            {
                byte installed = 0;
                uint status;
                using (Diagnostics.InputPerformance.Measure("prelayout.install"))
                    status = viem_core_view_install_prelayout(view.Document.Handle, view.Id, direction, result, &installed);
                result = 0; // Installation consumes the result, including stale rejection.
                Check(status, "Install background layout");
                if (installed != 0) Installed++; else Discarded++;
            }
            else Discarded++;
        }
        catch (Exception error) { failed = true; LastError = error.ToString(); }
        finally
        {
            if (result != 0) viem_layout_work_release(result);
            work.Dispose();
        }
        // Capture the next nearest gap from the current viewport, not a saved
        // queue. Stop as soon as the bounded band is ready; idle schedules nothing.
        Queue();
    }

    public void Dispose()
    {
        disposed = true;
        active?.Cancel(); // Never wait for a worker during close or device changes.
    }

    private sealed class Work(ulong request, Func<DirectWriteProvider> provider) : IDisposable
    {
        public ulong Request { get; } = request;
        public Func<DirectWriteProvider> Provider { get; } = provider;
        public CancellationTokenSource Cancellation { get; } = new();
        public void Cancel() { Cancellation.Cancel(); viem_layout_work_cancel(Request); }
        public void Dispose() { viem_layout_work_release(Request); Cancellation.Dispose(); }
    }
}

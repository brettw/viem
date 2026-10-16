using Microsoft.UI.Dispatching;
using Viem.Windows.Interop;
using Viem.Windows.Rendering;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

// One speculative chunk per view and one computing worker across the app.
// No timer, unbounded backlog, mutable core access or UI callback on the worker.
// Core decides the speculative window (the whole of a small document) and
// returns no request once it is complete.
internal sealed class BackgroundLayout(CoreView view, DispatcherQueue dispatcher, bool tableRefinement = false) : IDisposable
{
    private static readonly SemaphoreSlim workerSlot = new(1, 1);
    private readonly record struct Dependencies(ulong Document, ulong Revision, ulong Configuration, ulong Environment, ulong Metrics, float Width);
    private Dependencies? dependencies;
    private float top, left, height;
    private int direction = 1;
    private bool queued, disposed, failed;
    // Bound stale captures under continuous change, and retries when a
    // deliberately tiny cache cannot retain a window.
    private const int MaximumConsecutiveDiscards = 8;
    private int consecutiveDiscards;
    private int installedSinceIdle;
    private float publishedMaximumTop;
    private ulong publishedLayoutRevision;
    private Work? active;
    public bool IsIdle => !queued && active == null;
    public string? LastError { get; private set; }
    public int Started { get; private set; }
    public int Installed { get; private set; }
    public int Discarded { get; private set; }
    // Cumulative worker timing: waiting for the shared worker slot, computing,
    // and waiting for the UI thread to install a finished result.
    public double WaitMilliseconds { get; private set; }
    public double ComputeMilliseconds { get; private set; }
    public double InstallDelayMilliseconds { get; private set; }
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
        var layout = view.LayoutInfo();
        var next = new Dependencies(state.document_id, state.document_revision, state.configuration_generation,
            state.measurement_environment_id, state.metrics_generation, layout.viewport_width);
        if (dependencies == next && top == state.top && (!tableRefinement || left == state.left)) return;
        int nextDirection = state.top > top ? 1 : state.top < top ? -1 : direction;
        // Viewport speculation survives scrolling and paint-only configuration
        // changes: core validates its geometry and window at installation.
        bool geometryChanged = dependencies is not { } previous || previous with { Configuration = next.Configuration } != next;
        if (tableRefinement ? dependencies != next || top != state.top || left != state.left : geometryChanged || left != state.left)
            active?.Cancel();
        dependencies = next; top = state.top; left = state.left; height = layout.viewport_height; direction = nextDirection;
        publishedMaximumTop = state.maximum_top; publishedLayoutRevision = state.layout_revision;
        consecutiveDiscards = 0;
        failed = false; LastError = null;
        Queue();
    }

    private void Queue()
    {
        if (disposed || !Enabled || queued || active != null || failed || consecutiveDiscards >= MaximumConsecutiveDiscards) return;
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
            Check(tableRefinement ? viem_core_view_prepare_table_refinement(view.Document.Handle, view.Id, &request)
                : viem_core_view_prepare_prelayout(view.Document.Handle, view.Id, direction, &request), "Prepare background layout");
        if (request == 0)
        {
            // A completed window may have made the document extent exact
            // without a frame; publish it behind any pending input.
            if (!tableRefinement && installedSinceIdle > 0)
            {
                installedSinceIdle = 0;
                var state = view.Viewport;
                if (state.maximum_top != publishedMaximumTop || state.layout_revision != publishedLayoutRevision)
                    dispatcher.TryEnqueue(DispatcherQueuePriority.Low, () => { if (!disposed) view.SpeculativeLayoutFinished(); });
            }
            return;
        }
        try { active = new Work(request, view.Provider.CaptureWorkerFactory()); }
        catch { viem_layout_work_release(request); throw; }
        Started++;
        _ = Run(active);
    }

    private async Task Run(Work work)
    {
        ulong result = 0;
        Exception? failure = null;
        long queued = System.Diagnostics.Stopwatch.GetTimestamp(), computed = queued;
        try
        {
            await workerSlot.WaitAsync(work.Cancellation.Token).ConfigureAwait(false);
            long started = System.Diagnostics.Stopwatch.GetTimestamp();
            try { result = await Task.Run(() => Compute(work)).ConfigureAwait(false); }
            finally
            {
                workerSlot.Release();
                computed = System.Diagnostics.Stopwatch.GetTimestamp();
                work.WaitMilliseconds = System.Diagnostics.Stopwatch.GetElapsedTime(queued, started).TotalMilliseconds;
                work.ComputeMilliseconds = System.Diagnostics.Stopwatch.GetElapsedTime(started, computed).TotalMilliseconds;
            }
        }
        catch (OperationCanceledException) { }
        catch (Exception error) { failure = error; }
        work.Computed = computed;
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
        WaitMilliseconds += work.WaitMilliseconds; ComputeMilliseconds += work.ComputeMilliseconds;
        InstallDelayMilliseconds += System.Diagnostics.Stopwatch.GetElapsedTime(work.Computed).TotalMilliseconds;
        try
        {
            if (failure != null && !work.Cancellation.IsCancellationRequested && !disposed)
            { failed = true; LastError = failure.ToString(); }
            if (!disposed && Enabled && !work.Cancellation.IsCancellationRequested && result != 0)
            {
                byte installed = 0;
                uint status;
                using (Diagnostics.InputPerformance.Measure("prelayout.install"))
                    status = tableRefinement ? viem_core_view_install_table_refinement(view.Document.Handle, view.Id, result, &installed)
                        : viem_core_view_install_prelayout(view.Document.Handle, view.Id, direction, result, &installed);
                result = 0; // Installation consumes the result, including stale rejection.
                Check(status, "Install background layout");
                if (installed != 0)
                {
                    Installed++; consecutiveDiscards = 0; installedSinceIdle++;
                    if (tableRefinement) view.TableWidthsChanged();
                }
                else { Discarded++; consecutiveDiscards++; } // A newer viewport can reject work without making discovery fail.
            }
            else { Discarded++; consecutiveDiscards++; }
        }
        catch (Exception error) { failed = true; LastError = error.ToString(); }
        finally
        {
            if (result != 0) viem_layout_work_release(result);
            work.Dispose();
        }
        // Capture the next nearest gap from the current viewport, not a saved
        // queue. Stop as soon as the window is ready; idle schedules nothing.
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
        public double WaitMilliseconds, ComputeMilliseconds;
        public long Computed;
        public void Cancel() { Cancellation.Cancel(); viem_layout_work_cancel(Request); }
        public void Dispose() { viem_layout_work_release(Request); Cancellation.Dispose(); }
    }
}

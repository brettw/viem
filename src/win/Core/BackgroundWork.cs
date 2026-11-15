using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed unsafe partial class CoreDocument
{
    private BackgroundWake? backgroundWake;
    private DispatcherQueueTimer? backgroundDeadline;
    internal string? BackgroundWorkError { get; private set; }

    // A document can be shown by several windows. Register once with its owner
    // dispatcher, rather than running one maintenance loop in every window.
    internal void AttachBackgroundWork(DispatcherQueue dispatcher)
    {
        if (backgroundWake != null) return;
        var wake = new BackgroundWake(this, dispatcher);
        backgroundWake = wake;
        try
        {
            backgroundDeadline = dispatcher.CreateTimer();
            backgroundDeadline.IsRepeating = false;
            backgroundDeadline.Tick += (_, _) => wake.Queue();
            var callbacks = wake.Callbacks;
            Check(viem_core_set_background_wake(Handle, &callbacks), "Observe background work");
            wake.Queue(); // Also collect results completed before registration.
        }
        catch
        {
            wake.Close();
            viem_core_set_background_wake(Handle, null);
            backgroundWake = null;
            backgroundDeadline?.Stop();
            backgroundDeadline = null;
            throw;
        }
        finally { wake.Release(); } // The native registration owns its own reference.
    }

    internal bool ServiceBackgroundWork()
    {
        if (Handle == 0) return false;
        backgroundDeadline?.Stop();
        var status = new ViemBackgroundWorkStatus();
        Check(viem_core_service_background_work(Handle, &status), "Collect background work");
#if DEBUG
        BackgroundWorkDrainsForTesting++;
#endif
        // Arm before publication: a reentrant refresh may replace this schedule
        // with a newer deadline. A worker alone never needs a frontend timer.
        if (status.more_ready != 0 || status.next_deadline_ms == 0)
            backgroundWake?.Queue();
        else if (status.next_deadline_ms != uint.MaxValue && backgroundDeadline != null)
        {
            backgroundDeadline.Interval = TimeSpan.FromMilliseconds(status.next_deadline_ms);
            backgroundDeadline.Start();
        }
        if (status.changed != 0) PublishSyntaxChange();
        else if (status.diagnostics_changed != 0) SyntaxChanged?.Invoke();
        return status.changed != 0;
    }

    private void CloseBackgroundWork()
    {
        backgroundDeadline?.Stop();
        backgroundDeadline = null;
        backgroundWake?.Close();
        backgroundWake = null;
    }

#if DEBUG
    internal Action<CoreView>? BeforeViewBackgroundAttachmentForTesting { get; set; }
    internal int BackgroundWorkDrainsForTesting { get; private set; }
    internal bool BackgroundWorkScheduledForTesting => backgroundWake?.IsQueued == true || backgroundDeadline?.IsRunning == true;
    internal void WakeBackgroundWorkForTesting() => backgroundWake?.Queue();
#endif

    // Native may signal on any worker. The GCHandle owns only this bridge; the
    // document is weak. Native retains it through in-flight callbacks, and a
    // queued dispatcher closure remains safe even after the native release.
    private sealed class BackgroundWake
    {
        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        private delegate void Callback(nint context);
        private static readonly Callback wakeCallback = Wake;
        private static readonly Callback retainCallback = Retain;
        private static readonly Callback releaseCallback = Release;
        private readonly WeakReference<CoreDocument> document;
        private readonly DispatcherQueue dispatcher;
        private readonly ulong core;
        private readonly GCHandle handle;
        private int references = 1, active = 1, queued;

        internal BackgroundWake(CoreDocument document, DispatcherQueue dispatcher)
        {
            this.document = new(document);
            this.dispatcher = dispatcher;
            core = document.Handle;
            handle = GCHandle.Alloc(this);
        }
        internal ViemBackgroundWakeCallbacks Callbacks => new()
        {
            context = (void*)GCHandle.ToIntPtr(handle),
            wake = Marshal.GetFunctionPointerForDelegate(wakeCallback),
            retain = Marshal.GetFunctionPointerForDelegate(retainCallback),
            release = Marshal.GetFunctionPointerForDelegate(releaseCallback),
        };
        private static BackgroundWake From(nint context) => (BackgroundWake)GCHandle.FromIntPtr(context).Target!;
        private static void Wake(nint context) { try { From(context).Queue(); } catch { /* Never unwind across native. */ } }
        private static void Retain(nint context) => Interlocked.Increment(ref From(context).references);
        private static void Release(nint context) => From(context).Release();
        internal void Release() { if (Interlocked.Decrement(ref references) == 0) handle.Free(); }
        internal bool IsQueued => Volatile.Read(ref queued) != 0;
        internal void Close() => Volatile.Write(ref active, 0);

        internal void Queue()
        {
            if (Volatile.Read(ref active) == 0 || Interlocked.Exchange(ref queued, 1) != 0) return;
            bool accepted;
            try { accepted = dispatcher.TryEnqueue(DispatcherQueuePriority.Normal, () =>
            {
                // Clear before collection, so a completion racing the drain
                // schedules another turn instead of disappearing at its end.
                Interlocked.Exchange(ref queued, 0);
                if (Volatile.Read(ref active) == 0 || !document.TryGetTarget(out var owner)
                    || owner.Handle != core || owner.backgroundWake != this) return;
                try { owner.ServiceBackgroundWork(); }
                catch (Exception error)
                {
                    owner.backgroundDeadline?.Stop();
                    owner.BackgroundWorkError = error.ToString();
                    owner.ConfigurationWarning("Could not collect background work. " + error.Message);
                }
            }); }
            catch { accepted = false; } // The WinRT queue can already be disconnected.
            if (!accepted)
            {
                // Dispatcher rejection means its owning thread is shutting
                // down. Do not spin or invoke core from this worker.
                Interlocked.Exchange(ref queued, 0);
                Close();
            }
        }
    }
}

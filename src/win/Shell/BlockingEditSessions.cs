using System.Buffers.Binary;
using System.IO.Pipes;
using Viem.Windows.Core;

namespace Viem.Windows.Shell;

/// <summary>UI-owner-only completion tracking. Every view shares its document;
/// reload transfers that identity, and only release of the last view completes it.</summary>
internal sealed class BlockingEditSessions
{
    private readonly Dictionary<CoreDocument, List<Completion>> sessions = [];

#if DEBUG
    internal int CountForTesting => sessions.Values.Sum(pending => pending.Count);
#endif

    internal static async Task<Completion> Connect(string pipeName)
    {
        if (!pipeName.StartsWith("Viem.Blocking.", StringComparison.Ordinal)
            || !Guid.TryParseExact(pipeName["Viem.Blocking.".Length..], "N", out _))
            throw new InvalidDataException("Invalid blocking editor completion endpoint.");
        var pipe = new NamedPipeClientStream(".", pipeName, PipeDirection.InOut,
            PipeOptions.Asynchronous | PipeOptions.CurrentUserOnly);
        try { await pipe.ConnectAsync(5000); return new Completion(pipe); }
        catch { pipe.Dispose(); throw; }
    }

    internal void Attach(CoreDocument document, Completion completion)
    {
        if (sessions.Values.Sum(p => p.Count) >= 128) throw new IOException("Too many pending blocking editor requests.");
        if (!sessions.TryGetValue(document, out var pending)) sessions[document] = pending = [];
        pending.Add(completion);
    }

    internal void Replace(CoreDocument old, CoreDocument replacement)
    {
        if (!sessions.Remove(old, out var pending)) return;
        if (!sessions.TryGetValue(replacement, out var existing)) sessions[replacement] = pending;
        else existing.AddRange(pending);
    }

    internal void Released(CoreDocument document)
    {
        if (sessions.Remove(document, out var pending)) foreach (var completion in pending) completion.Finish(0);
    }

    internal void Abort(int exitCode)
    {
        foreach (var pending in sessions.Values) foreach (var completion in pending) completion.Finish(exitCode);
        sessions.Clear();
    }

    internal sealed class Completion(NamedPipeClientStream pipe) : IDisposable
    {
        private bool finished;
        internal void Finish(int exitCode)
        {
            if (finished) return;
            finished = true;
            try
            {
                byte[] result = new byte[4];
                BinaryPrimitives.WriteInt32LittleEndian(result, exitCode);
                // Finish before the last native window tears down the process.
                // A dead caller cannot delay close indefinitely. No locks or
                // model publication are held while exchanging this tiny result.
                using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(1));
                pipe.WriteAsync(result, timeout.Token).AsTask().GetAwaiter().GetResult();
            }
            catch (IOException) { }
            catch (OperationCanceledException) { }
            finally { pipe.Dispose(); }
        }
        public void Dispose() => Finish(1);
    }
}

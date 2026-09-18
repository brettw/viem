using System.IO.Pipes;
using System.Security.Cryptography;
using System.Security.Principal;
using System.Text;
using System.Text.Json;
using Microsoft.UI.Dispatching;

namespace Viem.Windows.Shell;

internal sealed record OpenInvocation(string[] Arguments, string Directory);

/// <summary>One UI process per user/profile, with bounded, same-user CLI handoff.</summary>
internal sealed class InstanceBroker : IDisposable
{
    private readonly Mutex mutex;
    private readonly string pipeName;
    private readonly CancellationTokenSource stopped = new();
    public bool IsPrimary { get; }
    public InstanceBroker(string profile)
    {
        string identity = WindowsIdentity.GetCurrent().User!.Value + ":" + Path.GetFullPath(profile).ToUpperInvariant();
        pipeName = "Viem." + Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(identity)));
        mutex = new Mutex(true, "Local\\" + pipeName, out bool primary); IsPrimary = primary;
    }
    public async Task Redirect(OpenInvocation invocation)
    {
        using var client = new NamedPipeClientStream(".", pipeName, PipeDirection.InOut, PipeOptions.Asynchronous | PipeOptions.CurrentUserOnly);
        await client.ConnectAsync(5000);
        byte[] bytes = JsonSerializer.SerializeToUtf8Bytes(invocation);
        if (bytes.Length > 1_048_576) throw new InvalidDataException("The launch request is too large.");
        await client.WriteAsync(BitConverter.GetBytes(bytes.Length)); await client.WriteAsync(bytes); await client.FlushAsync();
        byte[] ack = new byte[1]; using var timeout = new CancellationTokenSource(5000); await client.ReadExactlyAsync(ack, timeout.Token);
        if (ack[0] != 1) throw new IOException("The running Viem instance did not accept the launch request.");
    }
    public void Listen(DispatcherQueue dispatcher, Action<OpenInvocation> receive, Action<Exception> report)
    {
        _ = Task.Run(async () => {
            while (!stopped.IsCancellationRequested)
            {
                try
                {
                    using var server = new NamedPipeServerStream(pipeName, PipeDirection.InOut, 1, PipeTransmissionMode.Byte, PipeOptions.Asynchronous | PipeOptions.CurrentUserOnly);
                    await server.WaitForConnectionAsync(stopped.Token);
                    using var timeout = CancellationTokenSource.CreateLinkedTokenSource(stopped.Token); timeout.CancelAfter(5000);
                    byte[] size = new byte[4]; await server.ReadExactlyAsync(size, timeout.Token); int length = BitConverter.ToInt32(size);
                    if (length is < 1 or > 1_048_576) throw new InvalidDataException("Invalid launch request length.");
                    byte[] bytes = new byte[length]; await server.ReadExactlyAsync(bytes, timeout.Token);
                    var invocation = JsonSerializer.Deserialize<OpenInvocation>(bytes) ?? throw new InvalidDataException("Invalid launch request.");
                    if (invocation.Arguments == null || invocation.Arguments.Any(a => a == null) || !Path.IsPathFullyQualified(invocation.Directory)) throw new InvalidDataException("Invalid launch arguments.");
                    bool accepted = dispatcher.TryEnqueue(() => receive(invocation));
                    await server.WriteAsync(new byte[] { accepted ? (byte)1 : (byte)0 }, timeout.Token);
                }
                catch (OperationCanceledException) when (stopped.IsCancellationRequested) { break; }
                catch (Exception error) { dispatcher.TryEnqueue(() => report(error)); }
            }
        });
    }
    public void Dispose() { stopped.Cancel(); if (IsPrimary) mutex.ReleaseMutex(); mutex.Dispose(); }
}

using System.Diagnostics;
using System.IO.Pipes;

// The GUI can outlive this process. Only the requested document's completion
// determines when Git may continue; exiting a forwarding GUI process does not.
if (args.Length != 1 || args[0] is "--help" or "-h")
{
    Console.Error.WriteLine("Usage: blocking-viem <file>");
    return args.Length == 1 ? 0 : 1;
}
try
{
    string path = Path.GetFullPath(args[0]);
    if (Directory.Exists(path)) throw new IOException("Expected a file, not a directory.");
    string executable = Path.Combine(AppContext.BaseDirectory, "Viem.exe");
    string pipeName = "Viem.Blocking." + Guid.NewGuid().ToString("N");
    using var pipe = new NamedPipeServerStream(pipeName, PipeDirection.InOut, 1,
        PipeTransmissionMode.Byte, PipeOptions.Asynchronous | PipeOptions.CurrentUserOnly);
    var launch = new ProcessStartInfo(executable) { UseShellExecute = false, WorkingDirectory = Environment.CurrentDirectory };
    launch.ArgumentList.Add("--blocking-edit");
    launch.ArgumentList.Add(pipeName);
    launch.ArgumentList.Add(path);
    using var process = Process.Start(launch) ?? throw new IOException("Could not launch Viem.");
    // Bound startup only. Human editing has no timeout. Once connected, a GUI
    // crash closes the pipe and ReadExactlyAsync fails instead of hanging.
    using (var startup = new CancellationTokenSource(TimeSpan.FromSeconds(30)))
        await pipe.WaitForConnectionAsync(startup.Token);
    byte[] result = new byte[4];
    await pipe.ReadExactlyAsync(result);
    return System.Buffers.Binary.BinaryPrimitives.ReadInt32LittleEndian(result);
}
catch (Exception error)
{
    Console.Error.WriteLine("blocking-viem: " + error.Message);
    return 1;
}

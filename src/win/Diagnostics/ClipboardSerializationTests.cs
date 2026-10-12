#if DEBUG
using System.Text;
using Viem.Windows.Input;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage.Streams;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class ClipboardSerializationTests
{
    internal static async Task Run()
    {
        void Check(bool condition, string name)
        { if (!condition) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }
        var original = ClipboardFormats.Import("<b>café 猫 🙂</b>", VIEM_CLIPBOARD_FORMAT_HTML);
        string Details((string Text, string Fragment) actual) => $"text equals: {actual.Text == original.Text}, "
            + $"fragment equals: {actual.Fragment == original.Fragment}, length: {actual.Fragment.Length}/{original.Fragment.Length}, "
            + $"prefix: {string.Join(",", actual.Fragment.Take(12).Select(character => ((int)character).ToString("x4")))}, "
            + ClipboardFormats.PrivateReadDiagnosticForTesting;
        int changes = 0;
        void Changed(object? sender, object value) => changes++;
        ClipboardFormats.ContentChanged += Changed;
        try
        {
            ClipboardFormats.Write(original.Text, original.Fragment);
            var copied = await ClipboardFormats.Read();
            Check(copied == original && changes == 1,
                $"isolated native clipboard round-trips rich JSON strings and publishes content changes "
                    + $"(text: {copied.Text == original.Text}, fragment: {copied.Fragment == original.Fragment}, "
                    + $"length: {copied.Fragment.Length}/{original.Fragment.Length}, events: {changes}, {ClipboardFormats.PrivateReadDiagnosticForTesting})");
            var pending = ClipboardFormats.Read();
            ClipboardFormats.Write("later", "");
            var retained = await pending;
            Check(retained == original,
                "replacing the isolated clipboard preserves an in-flight native payload read (" + Details(retained) + ")");

            async Task Serialized(byte[] bytes)
            {
                var data = new DataPackage(); data.SetText(original.Text);
                var stream = new InMemoryRandomAccessStream();
                try {
                    // Managed Stream adapters cannot provide independent input
                    // views. Use the native stream type exposed by clipboard data.
                    using (var writer = new DataWriter(stream)) {
                        try { writer.WriteBytes(bytes); await writer.StoreAsync(); }
                        finally { writer.DetachStream(); }
                    }
                    stream.Seek(0);
                    data.SetData(ClipboardFormats.PrivateFormat, stream);
                }
                catch { stream.Dispose(); throw; }
                ClipboardFormats.SetPackageForTesting(data, stream);
            }
            foreach (bool bom in new[] { false, true })
            {
                byte[] bytes = Encoding.Unicode.GetBytes(original.Fragment + "\0");
                if (bom) bytes = Encoding.Unicode.GetPreamble().Concat(bytes).ToArray();
                Check(ClipboardFormats.DecodePrivateFragment(bytes) == original.Fragment,
                    "UTF-16 serialization fixture decodes exactly before the native DataPackage exchange");
                await Serialized(bytes);
                var legacy = await ClipboardFormats.Read();
                Check(legacy == original,
                    $"native private clipboard reads WinRT UTF-16LE string serialization with a terminal code unit (BOM: {bom}; {Details(legacy)})");
                var repeated = await ClipboardFormats.Read();
                Check(repeated == original,
                    "reading native private data preserves the publication for subsequent readers (" + Details(repeated) + ")");
            }
            await Serialized(Encoding.UTF8.GetBytes(original.Fragment));
            var utf8 = await ClipboardFormats.Read();
            Check(utf8 == original, "native private clipboard retains BOM-less UTF-8 Unicode payloads (" + Details(utf8) + ")");
            var pendingStream = ClipboardFormats.Read();
            ClipboardFormats.Write("later", "");
            var retainedStream = await pendingStream;
            Check(retainedStream == original,
                "replacing the isolated clipboard retains a native stream until its readers finish (" + Details(retainedStream) + ")");
            foreach (byte[] malformed in new[] {
                new byte[] { 0xff, 0xff, 0x00 }, "{broken"u8.ToArray(),
                new byte[] { (byte)'{', 0, 0x00, 0xd8, (byte)'}', 0 },
                new byte[] { (byte)'{', 0, (byte)'"' }
            })
            {
                await Serialized(malformed);
                var fallback = await ClipboardFormats.Read();
                Check(fallback.Text == original.Text && fallback.Fragment.Length == 0,
                    "malformed binary, JSON and UTF-16 private data preserve usable plain clipboard text (" + Details(fallback) + ")");
            }
        }
        finally { ClipboardFormats.ContentChanged -= Changed; ClipboardFormats.Write("", ""); }
    }
}
#endif

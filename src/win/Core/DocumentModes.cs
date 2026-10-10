using System.Text;
using System.Text.Json;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed record CodeLanguage(string Id, string Name, bool Primary);
internal sealed record DocumentModeState(ulong DocumentId, ulong Revision, string Format, bool Automatic, string? Language, string DetectedName);

internal static unsafe class DocumentModes
{
    public static IReadOnlyList<CodeLanguage> Languages { get; } = ReadLanguages();
    private static CodeLanguage[] ReadLanguages()
    {
        using var json = JsonDocument.Parse(Copy((p, n, r) => viem_copy_code_languages_json(p, n, r)));
        return json.RootElement.EnumerateArray().Select(v => new CodeLanguage(v.GetProperty("id").GetString()!,
            v.GetProperty("name").GetString()!, v.GetProperty("primary").GetBoolean())).ToArray();
    }
    public static DocumentModeState Read(CoreDocument document)
    {
        using var json = JsonDocument.Parse(Copy((p, n, r) => viem_core_copy_document_mode_json(document.Handle, p, n, r)));
        var v = json.RootElement;
        return new(v.GetProperty("documentId").GetUInt64(), v.GetProperty("documentRevision").GetUInt64(),
            v.GetProperty("format").GetString()!, v.GetProperty("automatic").GetBoolean(),
            v.GetProperty("language").GetString(), v.GetProperty("detectedName").GetString()!);
    }
}

internal sealed unsafe partial class CoreView
{
    public void SetDocumentMode(uint mode, string language, bool formattedMarkdown, DocumentModeState expected)
    {
        byte[] bytes = Encoding.UTF8.GetBytes(language);
        Send((c, o, e) => {
            var request = New<ViemSetDocumentModeV1>(); request.mode = mode;
            request.document_id = expected.DocumentId; request.document_revision = expected.Revision;
            request.formatted_markdown = formattedMarkdown ? 1u : 0u;
            fixed (byte* p = bytes) return viem_core_view_set_document_mode_with_effects(Document.Handle, Id, &request, p, (ulong)bytes.Length, o, e);
        });
        if (Document.State.document_revision == expected.Revision) Document.NotifyChanged();
    }
}

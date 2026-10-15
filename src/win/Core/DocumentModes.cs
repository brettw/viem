using System.Text;
using System.Text.Json;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed record CodeLanguage(string Id, string Name, bool Primary);
internal sealed record DocumentModeState(ulong DocumentId, ulong Revision, string Format, bool Automatic, string? Language, string? DetectedLanguage, string DetectedName);

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
            v.GetProperty("language").GetString(), v.GetProperty("detectedLanguage").GetString(),
            v.GetProperty("detectedName").GetString()!);
    }
}

internal sealed unsafe partial class CoreView
{
    public void SetDocumentMode(uint mode, string language, DocumentModeState expected) =>
        SetDocumentMode(mode, language, expected.DocumentId, expected.Revision);

    /// <summary>Markdown Preview on, or off into literal Code with the Markdown language.</summary>
    public void SetMarkdownPreview(bool enabled)
    {
        var state = Document.State;
        SetDocumentMode(enabled ? VIEM_DOCUMENT_MODE_MARKDOWN : VIEM_DOCUMENT_MODE_CODE, enabled ? "" : "markdown",
            state.document_id, state.document_revision);
    }

    private void SetDocumentMode(uint mode, string language, ulong documentId, ulong revision)
    {
        byte[] bytes = Encoding.UTF8.GetBytes(language);
        Send((c, o, e) => {
            var request = New<ViemSetDocumentModeV1>(); request.mode = mode;
            request.document_id = documentId; request.document_revision = revision;
            fixed (byte* p = bytes) return viem_core_view_set_document_mode_with_effects(Document.Handle, Id, &request, p, (ulong)bytes.Length, o, e);
        });
        if (Document.State.document_revision == revision) Document.NotifyChanged();
    }
}

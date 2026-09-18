using System.Text;
using System.Runtime.InteropServices;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed unsafe class CoreDocument : IDisposable
{
    public ulong Handle { get; private set; }
    public string? FilePath { get; set; }
    public string Name => FilePath == null ? "Untitled" : Path.GetFileName(FilePath);
    public event Action? Changed;
    public event Action? Disposed;
    public string StartupDiagnostics { get; private set; } = "";
    private string? configurationKey;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)] private delegate void StartupDiagnostic(nint context, ulong line, byte* message, ulong length);
    public ViemDocumentStateV1 State
    {
        get { var state = New<ViemDocumentStateV1>(); Check(viem_core_document_state(Handle, &state), "Read document"); return state; }
    }
    public bool IsDirty => (State.flags & VIEM_DOCUMENT_STATE_IS_DIRTY) != 0;
    public bool IsReadOnly => (State.flags & VIEM_DOCUMENT_STATE_READ_ONLY) != 0;
    public CoreDocument(byte[] source, string? path = null, uint? format = null, uint encoding = 0, uint fileFormat = 0)
    {
        if (viem_core_abi_version() != VIEM_CORE_ABI_VERSION) throw new InvalidOperationException("Rebuild Viem and its Rust library together: the ABI versions differ.");
        FilePath = path;
        var options = New<ViemDocumentOptions>();
        options.format = format ?? FormatForPath(path);
        options.encoding = encoding; options.file_format = fileFormat;
        ulong handle = 0, revision = 0;
        fixed (byte* bytes = source) Check(viem_core_create(bytes, (ulong)source.Length, &options, &handle, &revision), "Open document");
        Handle = handle;
        try
        {
            Check(viem_core_configure_syntax(Handle, null, 0), "Configure syntax");
            byte[] filename = Encoding.UTF8.GetBytes(path ?? "");
            fixed (byte* p = filename) Check(viem_core_initialize_code_detection(Handle, p, (ulong)filename.Length, (byte)(format == null && path != null ? 1 : 0)), "Detect code language");
        }
        catch { Dispose(); throw; }
    }
    public static uint FormatForPath(string? path) => Path.GetExtension(path ?? "").ToLowerInvariant() switch
    {
        ".md" or ".markdown" => VIEM_FORMAT_MARKDOWN,
        ".html" or ".htm" => VIEM_FORMAT_HTML,
        ".rtf" => VIEM_FORMAT_RTF,
        _ => VIEM_FORMAT_PLAIN_TEXT
    };
    public static string FormatName(uint format) => format switch { 1 => "Text", 2 => "Markdown", 3 => "HTML", 4 => "RTF", 5 => "Markdown Source", 6 => "HTML Source", 7 => "Code", _ => "Text" };
    public byte[] Source(ulong revision) => Copy((p, n, r) => viem_core_copy_source_bytes(Handle, revision, p, n, r));
    public string FormattedText() { ulong revision = State.document_revision; return Encoding.UTF8.GetString(Copy((p, n, r) => viem_core_copy_formatted_utf8(Handle, revision, p, n, r))); }
    public string FormattedRange(ulong start, ulong end)
    {
        var state = State;
        var request = New<ViemFormattedUtf8RangeV1>();
        request.identity = New<ViemFormattedSnapshotIdentityV1>();
        request.identity.document_id = state.document_id; request.identity.document_revision = state.document_revision;
        request.utf8_start = start; request.utf8_end = end;
        // Capture by value; lambda locals may safely have their addresses taken.
        return Encoding.UTF8.GetString(Copy((p, n, r) => { var q = request; return viem_core_copy_formatted_utf8_range(Handle, &q, p, n, r); }));
    }
    public void MarkSaved(ViemDocumentStateV1 state)
    {
        var saved = New<ViemMarkSavedV1>(); saved.document_id = state.document_id; saved.document_revision = state.document_revision;
        Check(viem_core_mark_saved(Handle, &saved), "Mark saved"); NotifyChanged();
    }
    public void MarkRecovered() { var s = State; Check(viem_core_mark_recovered(Handle, s.document_id, s.document_revision), "Restore recovery state"); NotifyChanged(); }
    public void SetReadOnly(bool value) { var s = State; Check(viem_core_set_read_only(Handle, s.document_id, s.document_revision, value ? 1u : 0u), "Set read-only"); }
    public void InitializeStartup(byte[] source)
    {
        var diagnostics = new List<string>();
        StartupDiagnostic callback = (_, line, message, length) => { try { diagnostics.Add($"startup.viem:{line}: {Encoding.UTF8.GetString(new ReadOnlySpan<byte>(message, checked((int)length)))}"); } catch { } };
        fixed (byte* p = source) Check(viem_core_initialize_startup(Handle, p, (ulong)source.Length, Marshal.GetFunctionPointerForDelegate(callback), null), "Load startup.viem");
        GC.KeepAlive(callback); StartupDiagnostics = string.Join("\n", diagnostics);
    }
    public void ConfigureDefaults(byte[] indentation, byte[] whitespace, uint width, string vimDirectory, byte[] associations)
    {
        string key = Convert.ToBase64String(indentation) + Convert.ToBase64String(whitespace) + width + vimDirectory + Convert.ToBase64String(associations);
        if (key == configurationKey) return;
        fixed (byte* p = indentation) Check(viem_core_set_indentation_defaults(Handle, p, (ulong)indentation.Length), "Set indentation defaults");
        fixed (byte* p = whitespace) Check(viem_core_set_whitespace_presentation_defaults(Handle, p, (ulong)whitespace.Length), "Set whitespace defaults");
        Check(viem_core_set_text_width_default(Handle, width), "Set text width default");
        byte[] directory = Encoding.UTF8.GetBytes(vimDirectory);
        fixed (byte* p = directory) Check(viem_core_configure_syntax(Handle, p, (ulong)directory.Length), "Configure syntax");
        fixed (byte* p = associations) Check(viem_core_set_code_filename_associations_json(Handle, p, (ulong)associations.Length), "Set filename associations");
        configurationKey = key;
    }
    public void InitializeStyleDefaults(byte[] json)
    {
        fixed (byte* p = json) Check(viem_core_initialize_style_defaults(Handle, State.document_revision, p, (ulong)json.Length), "Load style defaults");
    }
    public void NotifyChanged() => Changed?.Invoke();
    public void RedetectLanguage()
    {
        byte[] filename = Encoding.UTF8.GetBytes(FilePath ?? "");
        fixed (byte* p = filename) Check(viem_core_redetect_code_language(Handle, p, (ulong)filename.Length), "Detect code language");
    }
    public bool PollSyntax()
    {
        byte changed = 0; Check(viem_core_poll_syntax(Handle, &changed), "Update syntax");
        if (changed != 0) NotifyChanged(); return changed != 0;
    }
    public void Dispose()
    {
        if (Handle == 0) return;
        Check(viem_core_destroy(Handle), "Close document");
        Handle = 0;
        Disposed?.Invoke();
    }
}

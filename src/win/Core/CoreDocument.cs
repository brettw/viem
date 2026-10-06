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
    public event Action? SyntaxChanged;
    public event Action? Disposed;
    public string StartupDiagnostics { get; private set; } = "";
    private readonly List<string> configurationDiagnostics = [];
    public string ConfigurationDiagnostics => string.Join(Environment.NewLine, configurationDiagnostics);

    // Callers validating or editing settings still use the strict APIs. Only
    // optional setup of an editing document uses this recovery boundary.
    internal bool ConfigureForEditing(string name, Action action)
    {
        try { action(); return true; }
        catch (Exception error) when (error is InvalidDataException or IOException or UnauthorizedAccessException
            || (error is CoreException core && core.Status is VIEM_STATUS_INVALID_ARGUMENT
                or VIEM_STATUS_UNSUPPORTED_OPERATION or VIEM_STATUS_UNKNOWN_STYLE or VIEM_STATUS_INVALID_STYLE_VALUE
                or VIEM_STATUS_STYLE_INHERITANCE_CYCLE or VIEM_STATUS_INCOMPATIBLE_STYLE_ROLE
                or VIEM_STATUS_INVALID_STYLE_RELATIONSHIP or VIEM_STATUS_RESOURCE_EXHAUSTED))
        {
            ConfigurationWarning($"Could not load {name}; keeping available defaults. {error.Message}");
            return false;
        }
    }
    internal void ConfigurationWarning(string message)
    {
        message = message.Length > 1024 ? message[..1024] : message;
        if (configurationDiagnostics.Contains(message)) return;
        if (configurationDiagnostics.Count == 8) configurationDiagnostics.RemoveAt(0);
        configurationDiagnostics.Add(message);
    }
    private string? configurationKey;
    private string? editingConfigurationKey;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)] private delegate void StartupDiagnostic(nint context, ulong line, byte* message, ulong length);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)] private delegate void StyleDefaultsDiagnostic(nint context, byte* message, ulong length);
    public ViemDocumentStateV1 State
    {
        get { var state = New<ViemDocumentStateV1>(); Check(viem_core_document_state(Handle, &state), "Read document"); return state; }
    }
    public bool IsDirty => (State.flags & VIEM_DOCUMENT_STATE_IS_DIRTY) != 0;
    public bool IsReadOnly => (State.flags & VIEM_DOCUMENT_STATE_READ_ONLY) != 0;
    public CoreDocument(byte[] source, string? path = null, uint? format = null, uint encoding = 0, uint fileFormat = 0, bool markdownFormattedView = false)
    {
        using var startup = Diagnostics.StartupPerformance.Measure("document.create");
        if (viem_core_abi_version() != VIEM_CORE_ABI_VERSION) throw new InvalidOperationException("Rebuild Viem and its Rust library together: the ABI versions differ.");
        FilePath = path;
        var options = New<ViemDocumentOptions>();
        options.format = format ?? FormatForPath(path);
        if (format == null && options.format == VIEM_FORMAT_MARKDOWN_SOURCE && markdownFormattedView)
            options.format = VIEM_FORMAT_MARKDOWN;
        options.encoding = encoding; options.file_format = fileFormat;
        ulong handle = 0, revision = 0;
        fixed (byte* bytes = source) Check(viem_core_create(bytes, (ulong)source.Length, &options, &handle, &revision), "Open document");
        Handle = handle;
        try
        {
            byte[] directory = Encoding.UTF8.GetBytes(BundledVimRuntime.SyntaxDirectory);
            ConfigureForEditing("syntax resources", () => {
                fixed (byte* p = directory) Check(viem_core_configure_syntax(Handle, p, (ulong)directory.Length), "Configure syntax");
            });
            byte[] filename = Encoding.UTF8.GetBytes(path ?? "");
            ConfigureForEditing("language detection", () => {
                fixed (byte* p = filename) Check(viem_core_initialize_code_detection(Handle, p, (ulong)filename.Length, (byte)(format == null && path != null ? 1 : 0)), "Detect code language");
            });
        }
        catch { Dispose(); throw; }
    }
    public static uint FormatForPath(string? path) => Path.GetExtension(path ?? "").ToLowerInvariant() switch
    {
        ".md" or ".markdown" or ".mdown" or ".mkd" => VIEM_FORMAT_MARKDOWN_SOURCE,
        ".html" or ".htm" or ".xhtml" => VIEM_FORMAT_CODE,
        _ => VIEM_FORMAT_PLAIN_TEXT
    };
    public static string FormatName(uint format) => format switch { 1 => "Text", 2 => "Markdown", 5 => "Markdown Source", 7 => "Code", _ => "Text" };
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
    public string SelectionOption(uint name) => Encoding.UTF8.GetString(Copy((p, n, required) =>
        viem_core_copy_selection_option(Handle, name, p, n, required)));
    public void SetSelectionOption(uint name, string value)
    {
        byte[] bytes = Encoding.UTF8.GetBytes(value);
        fixed (byte* p = bytes) Check(viem_core_set_selection_option(Handle, name, p, (ulong)bytes.Length), "Set global selection option");
    }
    public void ConfigureDefaults(byte[] indentation, byte[] whitespace, uint width, byte[] associations)
    {
        string key = Convert.ToBase64String(indentation) + Convert.ToBase64String(whitespace) + width + Convert.ToBase64String(associations);
        if (key == configurationKey) return;
        fixed (byte* p = indentation) Check(viem_core_set_indentation_defaults(Handle, p, (ulong)indentation.Length), "Set indentation defaults");
        fixed (byte* p = whitespace) Check(viem_core_set_whitespace_presentation_defaults(Handle, p, (ulong)whitespace.Length), "Set whitespace defaults");
        Check(viem_core_set_text_width_default(Handle, width), "Set text width default");
        fixed (byte* p = associations) Check(viem_core_set_code_filename_associations_json(Handle, p, (ulong)associations.Length), "Set filename associations");
        configurationKey = key;
    }
    internal void ConfigureEditingDefaults(byte[] indentation, byte[] whitespace, uint width, byte[] associations)
    {
        string key = Convert.ToBase64String(indentation) + Convert.ToBase64String(whitespace) + width + Convert.ToBase64String(associations);
        if (key == editingConfigurationKey) return;
        ConfigureForEditing("indentation", () => {
            fixed (byte* p = indentation) Check(viem_core_set_indentation_defaults(Handle, p, (ulong)indentation.Length), "Set indentation defaults");
        });
        ConfigureForEditing("whitespace presentation", () => {
            fixed (byte* p = whitespace) Check(viem_core_set_whitespace_presentation_defaults(Handle, p, (ulong)whitespace.Length), "Set whitespace defaults");
        });
        ConfigureForEditing("text width", () => Check(viem_core_set_text_width_default(Handle, width), "Set text width default"));
        ConfigureForEditing("filename associations", () => {
            fixed (byte* p = associations) Check(viem_core_set_code_filename_associations_json(Handle, p, (ulong)associations.Length), "Set filename associations");
        });
        // A failed preference is retried on the next preference change, rather
        // than on every presentation refresh. Strict validation remains separate.
        editingConfigurationKey = key;
    }
    public string[] InitializeStyleDefaults(byte[] json) => SetStyleDefaults(json, false);
    public string[] ReplaceStyleDefaults(byte[] json) => SetStyleDefaults(json, true);
    private string[] SetStyleDefaults(byte[] json, bool live)
    {
        var diagnostics = new List<string>();
        StyleDefaultsDiagnostic callback = (_, message, length) => {
            try { diagnostics.Add(Encoding.UTF8.GetString(new ReadOnlySpan<byte>(message, checked((int)length)))); }
            catch { /* Exceptions cannot cross the native callback boundary. */ }
        };
        uint status;
        fixed (byte* p = json) status = live
            ? viem_core_replace_style_defaults(Handle, State.document_revision, p, (ulong)json.Length, Marshal.GetFunctionPointerForDelegate(callback), null)
            : viem_core_initialize_style_defaults(Handle, State.document_revision, p, (ulong)json.Length, Marshal.GetFunctionPointerForDelegate(callback), null);
        GC.KeepAlive(callback);
        if (status != VIEM_STATUS_OK && diagnostics.Count > 0)
            throw new InvalidDataException(string.Join(Environment.NewLine, diagnostics));
        Check(status, "Load style defaults");
        return diagnostics.ToArray();
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
        if (changed != 0) { NotifyChanged(); SyntaxChanged?.Invoke(); }
        return changed != 0;
    }
    public string SyntaxDiagnostics => Encoding.UTF8.GetString(Copy((p, n, r) => viem_core_copy_syntax_diagnostics(Handle, p, n, r)));
    public void Dispose()
    {
        if (Handle == 0) return;
        Check(viem_core_destroy(Handle), "Close document");
        Handle = 0;
        Disposed?.Invoke();
    }
}

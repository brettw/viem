using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json.Nodes;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal static unsafe class CoreThemes
{
    public static byte[] StyleDefaults(uint variant, IReadOnlyDictionary<string, byte[]> variants)
    {
        var values = new JsonObject();
        foreach (var (key, bytes) in variants) if (bytes.Length > 0) values[key] = JsonNode.Parse(bytes);
        byte[] input = Encoding.UTF8.GetBytes(values.ToJsonString());
        return Copy((p, n, r) => { fixed (byte* source = input)
            return viem_theme_style_defaults_json(variant, source, (ulong)input.Length, p, n, r); });
    }
    public static byte[] Defaults(uint preset = VIEM_THEME_PRESET_MIDNIGHT) => Copy((p, n, r) => viem_theme_default_json(preset, p, n, r));
    public static void Validate(byte[] json)
    { fixed (byte* p = json) Check(viem_theme_validate_json(p, (ulong)json.Length), "Validate theme"); }
    public static void ValidateName(string name)
    {
        byte[] bytes = Encoding.UTF8.GetBytes(name);
        fixed (byte* p = bytes)
            if (viem_theme_validate_name(p, (ulong)bytes.Length) != VIEM_STATUS_OK)
                throw new InvalidDataException("Use a name of 1–32 characters without path separators, reserved filename characters, or a reserved filename.");
    }
    public const int MaximumVimColorSchemeBytes = 4 * 1024 * 1024;
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)] private delegate void ImportDiagnostic(nint context, byte* message, ulong length);
    /// The portable core evaluates the scheme as gVim would and returns a
    /// complete theme based on its built-in Paper or Midnight preset.
    public static byte[] ImportVim(byte[] source)
    {
        if (source.Length > MaximumVimColorSchemeBytes) throw new InvalidDataException("The Vim color scheme is larger than 4 MiB.");
        var diagnostics = new List<string>();
        ImportDiagnostic callback = (_, message, length) => {
            try { diagnostics.Add(Encoding.UTF8.GetString(new ReadOnlySpan<byte>(message, checked((int)length)))); }
            catch { /* Exceptions cannot cross the native callback boundary. */ }
        };
        nint function = Marshal.GetFunctionPointerForDelegate(callback);
        try {
            fixed (byte* input = source) {
                ulong required = 0;
                uint status = viem_theme_import_vim(input, (ulong)source.Length, null, 0, &required, function, null);
                if (status != VIEM_STATUS_BUFFER_TOO_SMALL && status != VIEM_STATUS_OK || required > 20 * 1024 * 1024)
                    throw new InvalidDataException(diagnostics.FirstOrDefault() ?? "The Vim color scheme could not be imported.");
                byte[] theme = new byte[checked((int)required)];
                fixed (byte* output = theme) status = viem_theme_import_vim(input, (ulong)source.Length, output, required, &required, function, null);
                if (status != VIEM_STATUS_OK) throw new InvalidDataException(diagnostics.FirstOrDefault() ?? "The Vim color scheme could not be imported.");
                return theme;
            }
        }
        finally { GC.KeepAlive(callback); }
    }
    /// A portable theme name for an imported file stem, numbered on a
    /// case-insensitive collision with an existing theme.
    public static string UniqueName(string stem, IEnumerable<string> existing)
    {
        byte[] name = Encoding.UTF8.GetBytes(stem), names = Encoding.UTF8.GetBytes(string.Join("\n", existing));
        byte[] result = Copy((p, n, r) => { fixed (byte* a = name) fixed (byte* b = names)
            return viem_theme_unique_name(a, (ulong)name.Length, b, (ulong)names.Length, p, n, r); });
        return Encoding.UTF8.GetString(result);
    }
    public static void ReplaceCodeStyles(byte[] json)
    { fixed (byte* p = json) Check(viem_code_replace_style_json(p, (ulong)json.Length), "Apply theme Code styles"); }
    public static ulong CodeStyleRevision { get { var identity = New<ViemStyleSheetIdentityV1>(); Check(viem_core_style_sheet_identity(0, &identity), "Read Code style revision"); return identity.style_sheet_revision; } }
}

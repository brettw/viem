using System.Text;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal static unsafe class CoreThemes
{
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
    public static void ReplaceCodeStyles(byte[] json)
    { fixed (byte* p = json) Check(viem_code_replace_style_json(p, (ulong)json.Length), "Apply theme Code styles"); }
    public static ulong CodeStyleRevision { get { var info = New<ViemStyleSheetInfoV1>(); Check(viem_code_style_sheet_info(&info), "Read Code style revision"); return info.identity.style_sheet_revision; } }
}

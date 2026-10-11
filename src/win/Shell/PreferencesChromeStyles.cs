using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class Preferences
{
    private System.Text.Json.Nodes.JsonObject? chromeStyleTheme;
    private (ChromeStyle Status, ChromeStyle Command)? chromeStyles;
    internal (ChromeStyle Status, ChromeStyle Command) StatusStyles
    {
        get {
            if (ReferenceEquals(chromeStyleTheme, activeTheme) && chromeStyles is { } retained) return retained;
            byte[] defaults = ThemeStyleContext(VIEM_FORMAT_PLAIN_TEXT);
            // The emergency appearance stays usable if optional styles fail.
            var baseTheme = ReadTheme(activeTheme);
            var status = ChromeStyle.Fallback(baseTheme.Foreground, baseTheme.Background, status: true);
            var command = status with { Foreground = status.Background, Background = status.Foreground, Border = default };
            try {
                using var document = new CoreDocument([]);
                document.InitializeStyleDefaults(defaults);
                var sheet = document.ChromeStyleSheet();
                status = ChromeStyle.Resolve(sheet, "Status bar", status);
                command = ChromeStyle.Resolve(sheet, "Status bar command", command with { Foreground = baseTheme.Foreground });
            }
            catch (Exception error) { System.Diagnostics.Debug.WriteLine("Status styles: " + error.Message); }
            chromeStyleTheme = activeTheme;
            return (chromeStyles = (status, command)).Value;
        }
    }
    internal double StatusHeight => Math.Max(1, StatusStyles.Status.Size * 1.4
        + StatusStyles.Status.Padding.Vertical + StatusStyles.Status.Border.Vertical);
}

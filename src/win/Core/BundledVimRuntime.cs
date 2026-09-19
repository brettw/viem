namespace Viem.Windows.Core;

/// <summary>Installed application resources; never a profile or working-directory path.</summary>
internal static class BundledVimRuntime
{
    public static string SyntaxDirectory => Path.Combine(AppContext.BaseDirectory, "Resources", "vim", "runtime", "syntax");

    public static string? Diagnostic
    {
        get
        {
            try
            {
                // A bounded availability check. Full inventory/hash validation
                // belongs to packaging; syntax workers diagnose individual files.
                using var file = File.OpenRead(Path.Combine(SyntaxDirectory, "vim.vim"));
                return null;
            }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException)
            {
                return "Bundled Vim syntax files are unavailable. Tree-sitter highlighting remains available for supported languages.";
            }
        }
    }
}

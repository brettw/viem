using System.Text.Json.Nodes;
using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class Preferences
{
    public bool MarkdownFormattedView => Get("editing", "markdownFormattedView", true);
    internal uint InitialDisplay(CoreDocument document) => document.IsMarkdownDocument
        ? MarkdownFormattedView ? VIEM_VIEW_DISPLAY_PREVIEW : VIEM_VIEW_DISPLAY_SOURCE
        : VIEM_VIEW_DISPLAY_DEFAULT;
    internal void RememberMarkdownPreview(bool enabled) => Update(candidate => Merge(candidate, new JsonObject {
        ["editing"] = new JsonObject { ["markdownFormattedView"] = enabled }
    }), notify: false);
}

using System.Text.Json.Nodes;
using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class Preferences
{
    private readonly HashSet<CoreDocument> markdownViewDocuments = [];
    public bool MarkdownFormattedView => Get("editing", "markdownFormattedView", false);

    internal void ObserveMarkdownView(CoreDocument document, Action<Exception> report)
    {
        uint previous = document.State.format;
        if (previous is not (VIEM_FORMAT_MARKDOWN or VIEM_FORMAT_MARKDOWN_SOURCE)
            || !markdownViewDocuments.Add(document)) return;
        // Initial attachment, recovery, and switching between existing panes
        // do not replace the last choice. Observe actual shared document changes
        // once, including history navigation, rather than toolbar refreshes.
        void Changed()
        {
            uint current = document.State.format;
            if (current == previous) return;
            uint old = previous; previous = current;
            if (old is not (VIEM_FORMAT_MARKDOWN or VIEM_FORMAT_MARKDOWN_SOURCE)
                || current is not (VIEM_FORMAT_MARKDOWN or VIEM_FORMAT_MARKDOWN_SOURCE)) return;
            bool formatted = current == VIEM_FORMAT_MARKDOWN;
            try
            {
                // Update merges the current file; a cached equal value must
                // not let an external settings edit override this newer choice.
                Update(candidate => Merge(candidate, new JsonObject {
                    ["editing"] = new JsonObject { ["markdownFormattedView"] = formatted }
                }), notify: false);
            }
            catch (Exception error) { report(error); }
        }
        void Disposed()
        {
            document.Changed -= Changed; document.Disposed -= Disposed;
            markdownViewDocuments.Remove(document);
        }
        document.Changed += Changed; document.Disposed += Disposed;
    }
}

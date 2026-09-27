using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Windows.Storage.Pickers;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    internal async Task<bool> Export(EditorPane pane, string? explicitPath = null)
    {
        var document = pane.Document;
        var view = pane.View;
        if (view == null) return false;
        string? path = explicitPath;
        if (path == null)
        {
            string name = Path.GetFileNameWithoutExtension(document.Name);
            if (Path.GetExtension(document.Name).ToLowerInvariant() is ".html" or ".htm" or ".xhtml") name += "-export";
            var picker = new FileSavePicker { SuggestedFileName = name, DefaultFileExtension = ".html", CommitButtonText = "Export" };
            picker.FileTypeChoices.Add("HTML", new List<string> { ".html" });
            WinRT.Interop.InitializeWithWindow.Initialize(picker, Hwnd);
            var file = await picker.PickSaveFileAsync();
            if (file == null) return false;
            path = file.Path;
        }
        if (!Panes.Contains(pane) || pane.View != view) return false;
        path = Path.GetFullPath(path);
        void ValidateDestination()
        {
            if (App.Instance.Windows.SelectMany(window => window.Panes).Any(open =>
                    open.Document.FilePath is string source && FileIdentity.Same(source, path)))
                throw new IOException("Choose a different filename to export without replacing an open document's source.");
        }
        ValidateDestination();
        if (explicitPath != null && File.Exists(path)
            && await Dialog("Replace file?", $"{path} already exists.", "Replace", "Cancel") != ContentDialogResult.Primary)
            return false;

        // Snapshot on the UI thread so it cannot contend with editor commands.
        // The worker only touches the detached snapshot and exported bytes.
        ulong revision = document.State.document_revision;
        ulong snapshot = view.PrepareHtmlExport(revision);
        byte[] html = await Task.Run(() => CoreView.RenderHtmlExport(snapshot));
        // Another document may have adopted this filename during serialization.
        ValidateDestination();
        await Task.Run(() => Preferences.AtomicWrite(path, html));
        if (Panes.Contains(pane)) pane.SetMessage($"Exported {Path.GetFileName(path)}");
        return true;
    }
}

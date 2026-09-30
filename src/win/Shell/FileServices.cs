using System.Security.Cryptography;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private bool checkingFiles;
    private readonly Dictionary<CoreDocument, (long Length, long Time)> observedFiles = [];
    private static string ResolvePath(string path, string? baseDirectory = null)
    {
        // Tilde is a shell convention, so expand it before calling Windows path APIs.
        if (path == "~" || path.StartsWith("~/") || path.StartsWith("~\\"))
            path = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), path.Length > 2 ? path[2..].TrimStart('/', '\\') : "");
        path = Environment.ExpandEnvironmentVariables(path);
        return baseDirectory == null ? Path.GetFullPath(path) : Path.GetFullPath(path, baseDirectory);
    }
    private async Task NavigateArgument(EditorPane pane, FrontendRequest request)
    {
        var r = request.Value;
        if ((r.flags & VIEM_EX_FRONTEND_WRITE_FIRST) != 0) { await Save(pane); if (pane.Document.IsDirty) return; }
        ulong index = ResolveArgument(pane, r.argument_command, r.argument_count);
        await OpenPath(App.Instance.Arguments[checked((int)index)], false, (r.flags & VIEM_EX_FRONTEND_FORCE) != 0);
        var opened = App.Instance.Windows.SelectMany(w => w.Panes).FirstOrDefault(p => p.Document.FilePath is string file && FileIdentity.Same(file, App.Instance.Arguments[(int)index]));
        if (opened != null) opened.RememberedArgument = index;
        if ((r.flags & VIEM_EX_FRONTEND_HAS_LINE) != 0) opened?.View?.GoToLine(r.argument_line);
    }
    private async Task RenameDocument(EditorPane pane, string path)
    {
        var doc = pane.Document;
        if (string.Equals(doc.FilePath, path, StringComparison.OrdinalIgnoreCase)) return;
        if (App.Instance.Windows.SelectMany(w => w.Panes).Any(p => p.Document != doc && p.Document.FilePath is string named && FileIdentity.Same(named, path)))
            throw new IOException("That file is already open in another document.");
        byte[] baseline = SHA256.HashData(File.Exists(path) ? await File.ReadAllBytesAsync(path) : []);
        var replacement = DocumentRecovery.Claim(doc, path, preferences.DirectoryPath, DispatcherQueue, e => pane.Report(e));
        try
        {
            replacement.Write(RecoverySnapshot.Capture(doc));
            if (!await replacement.Pending) throw new IOException("The new recovery snapshot could not be saved. The previous backup was retained.");
        }
        catch { replacement.Dispose(); throw; }
        if (recoveries.Remove(doc, out var previous)) previous.Dispose();
        recoveries[doc] = replacement;
        doc.FilePath = path;
        foreach (var window in App.Instance.Windows.Where(w => w.Panes.Any(p => p.Document == doc))) window.savedSources[doc] = baseline;
        doc.RedetectLanguage(); doc.NotifyChanged();
    }
    private static unsafe ulong ResolveArgument(EditorPane pane, uint command, ulong count)
    {
        var args = App.Instance.Arguments;
        int current = args.FindIndex(p => string.Equals(p, pane.Document.FilePath, StringComparison.OrdinalIgnoreCase));
        var result = viem_argument_list_resolve((ulong)args.Count, current < 0 ? ulong.MaxValue : (ulong)current, pane.RememberedArgument, command, Math.Max(1, count));
        if (result.status != VIEM_ARGUMENT_RESOLVE_OK) throw new InvalidOperationException(result.status switch { VIEM_ARGUMENT_RESOLVE_EMPTY => "The argument list is empty.", VIEM_ARGUMENT_RESOLVE_BEFORE_FIRST => "Already at the first file.", VIEM_ARGUMENT_RESOLVE_AFTER_LAST => "Already at the last file.", _ => "Invalid argument index." });
        return result.index;
    }
    private async Task<bool> ExWrite(EditorPane pane, FrontendRequest request)
    {
        var r = request.Value; var doc = pane.Document; var state = doc.State;
        if (state.document_id != r.document_id || state.document_revision != r.document_revision) throw new InvalidOperationException("The document changed before this file command could run.");
        bool force = (r.flags & VIEM_EX_FRONTEND_FORCE) != 0;
        if (doc.IsReadOnly && !force) throw new InvalidOperationException("E45: readonly option is set (use ! to override)");
        string? requestedPath = request.Text.Length == 0 ? null : request.Text;
        if ((r.flags & VIEM_EX_FRONTEND_HAS_RANGE) != 0)
        {
            string? path = requestedPath == null ? doc.FilePath : ResolvePath(requestedPath);
            if (path == null) throw new InvalidOperationException("A filename is required to write selected lines.");
            if (App.Instance.Windows.SelectMany(w => w.Panes).Any(p => p.Document != doc && p.Document.FilePath is string named && FileIdentity.Same(named, path))) throw new IOException("That file is already open in another document.");
            var (bytes, complete) = HardLineBytes(doc, r);
            bool same = doc.FilePath != null && FileIdentity.Same(doc.FilePath, path);
            if (same && !complete && !force) throw new InvalidOperationException("Use ! to replace the current file with only selected lines.");
            if (File.Exists(path) && !force && await Dialog("Write selected lines?", $"Replace {path} with the selected source lines?", "Replace", "Cancel") != ContentDialogResult.Primary) return false;
            await Task.Run(() => Preferences.AtomicWrite(path, bytes)); pane.SetMessage("Wrote selected source lines.");
            if (same)
            {
                foreach (var window in App.Instance.Windows.Where(w => w.Panes.Any(p => p.Document == doc))) window.savedSources[doc] = SHA256.HashData(bytes);
                if (complete && doc.State.document_revision == state.document_revision) doc.MarkSaved(state);
            }
            return true;
        }
        bool adopt = r.kind == VIEM_EX_FRONTEND_SAVE_AS || doc.FilePath == null;
        return await Save(pane, r.kind == VIEM_EX_FRONTEND_SAVE_AS, requestedPath, force, adopt, native: false);
    }
    private static unsafe (byte[] Bytes, bool Complete) HardLineBytes(CoreDocument doc, ViemExFrontendRequestV1 request)
    {
        // Ex's range is inclusive; the source-copy API takes a half-open range.
        bool all = false;
        byte[] bytes = Copy((p, n, r) => { uint complete = 0; var status = viem_core_copy_hard_line_source_bytes(doc.Handle, request.document_id, request.document_revision, request.hard_line_start, checked(request.hard_line_end + 1), p, n, r, &complete); all = complete != 0; return status; });
        return (bytes, all);
    }
    private async Task CheckExternalChanges()
    {
        if (checkingFiles || closed || !IsWindowActive) return;
        checkingFiles = true;
        try
        {
            foreach (var document in Panes.Select(p => p.Document).Distinct().ToArray())
            {
                string? path = document.FilePath;
                if (path == null || !File.Exists(path) || !savedSources.TryGetValue(document, out var baseline)) continue;
                var file = new FileInfo(path); var token = (file.Length, file.LastWriteTimeUtc.Ticks);
                if (observedFiles.TryGetValue(document, out var previous) && previous == token) continue;
                observedFiles[document] = token;
                byte[] observed;
                using (var input = File.Open(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete)) observed = await SHA256.HashDataAsync(input);
                if (observed.AsSpan().SequenceEqual(baseline)) continue;
                var pane = Panes.FirstOrDefault(p => p.Document == document); if (pane == null) continue;
                var result = await Dialog("File changed on disk", $"{document.Name} was changed by another program." + (document.IsDirty ? " Reloading will discard your unsaved edits." : ""), "Reload", "Keep My Version");
                // A subsequent observation is reviewed separately. Saving still
                // compares against the saved baseline, even after Keep My Version.
                if (result == ContentDialogResult.Primary) await Reload(pane, true);
            }
        }
        catch (Exception e) { ActivePane?.Report(e); }
        finally { checkingFiles = false; }
    }
}

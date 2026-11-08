using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Windows.Storage.Pickers;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private DiffToolbar? diffToolbar;
    private EditorPane? diffLive, diffLeft;
    private DiffComparison? comparison;
    private DiffBase diffBase = new(VIEM_DIFF_BASE_LAST_SAVED);
    private DiffFile[] diffFiles = [];
    private string[] diffBranches = [];
    private string? diffScope;
    private bool diffEnabled, diffAligned, diffGit, mirroring, deletedSide, originalNumbers;
    private bool enteredDiff, changingNumbers;
    private ulong syncedRevision;
    private CancellationTokenSource? diffRequest, folderRequest, branchRequest;
    private long diffGeneration;
    private readonly Dictionary<(DiffBase, string), byte[]> diffBytes = [];
    private long cachedBytes;
    private int CurrentDiffFile => Array.FindIndex(diffFiles, f => SamePath(f.Path, ActivePane?.Document.FilePath));
    private static bool SamePath(string a, string? b) => b != null && StringComparer.OrdinalIgnoreCase.Equals(Path.GetFullPath(a), Path.GetFullPath(b));
    private static string[] UnsavedPaths() => App.Instance.Windows.SelectMany(w => w.Panes).Select(p => p.Document)
        .Distinct().Where(d => d.FilePath != null && d.IsDirty).Select(d => d.FilePath!).ToArray();

    private void BuildDiffMenu()
    {
        var mode = Toggle("Diff Mode", "M", on => Safe(() => SetDiffMode(on)), () => diffEnabled);
        validation.Add((mode, () => View != null));
        var align = Toggle("Align Sides", "A", SetDiffAlignment, () => diffEnabled ? diffAligned : preferences.Get("diff", "alignSides", true));
        validation.Add((align, () => diffEnabled));
        var folder = Item("", "D", () => DiffFolder(Environment.CurrentDirectory));
        void NameFolder() {
            string path = Environment.CurrentDirectory;
            folder.Text = "Diff " + (path.Length <= 40 ? path : "…" + path[^39..]);
        }
        NameFolder();
        folder.Loaded += (_, _) => NameFolder();
        Top("Diff", "D", mode, Separator(), folder, Item("Diff Folder…", "F", ChooseDiffFolder), Separator(), align,
            Item("Refresh Base", "R", RefreshDiff, enabled: () => diffEnabled),
            Item("Choose Base File…", "B", ChooseDiffFile, enabled: () => diffEnabled), Separator(),
            Item("Previous File", "P", () => MoveDiffFile(-1), enabled: () => diffEnabled && diffFiles.Length > 0),
            Item("Next File", "N", () => MoveDiffFile(1), enabled: () => diffEnabled && diffFiles.Length > 0));
    }

    private void BeginDiff()
    {
        if (diffToolbar == null) {
            diffToolbar = new(() => Safe(ChooseDiffFile), () => Safe(RefreshDiff), step => Safe(() => MoveDiffFile(step)),
                next => Safe(() => { NavigateChange(next); return Task.CompletedTask; }));
            diffToolbar.BaseSelected += choice => Safe(() => ChooseDiffBase(choice));
            diffToolbar.FileSelected += index => Safe(() => SelectDiffFile(index));
            diffToolbar.AlignmentChanged += value => Safe(() => { SetDiffAlignment(value); return Task.CompletedTask; });
            AttachToolbar(diffToolbar, 3);
        }
        diffEnabled = true;
        diffAligned = preferences.Get("diff", "alignSides", true);
        diffToolbar.Visibility = Visibility.Visible;
        SynchronizeFormattingToolbar();
    }

    internal async Task SetDiffMode(bool enabled)
    {
        if (diffEnabled == enabled) return;
        if (!enabled) { EndDiff(); return; }
        if (ActivePane == null) return;
        BeginDiff();
        await LoadDiff(detectRepository: true);
    }

    private void EndDiff()
    {
        diffEnabled = false;
        ++diffGeneration;
        branchRequest?.Cancel();
        diffRequest?.Cancel();
        DetachDiff();
        if (diffToolbar != null) diffToolbar.Visibility = Visibility.Collapsed;
        diffFiles = []; diffBranches = []; diffScope = null; diffGit = false;
        diffBase = new(VIEM_DIFF_BASE_LAST_SAVED);
        ClearDiffCache();
        menusDirty = true;
        SynchronizeFormattingToolbar();
    }

    private void DetachDiff()
    {
        var live = diffLive;
        diffLive = null;
        try {
            comparison?.Dispose();
        } finally {
            comparison = null;
            if (diffLeft is { } left) {
                diffLeft = null;
                root.Children.Remove(left);
                left.Dispose();
                left.Document.Dispose();
            }
            root.ColumnDefinitions[0].Width = new(0);
            if (live?.View is { } view) {
                live.ClearDiffMarks();
                if (deletedSide) live.Document.ProtectDeletedSide(false);
                if (enteredDiff) {
                    live.Document.LeaveDiff(view);
                    view.Number(preferences.Get("code", "lineNumbers", originalNumbers));
                }
                live.Refresh();
            }
            deletedSide = enteredDiff = false;
        }
    }

    private void DiffPaneLeaving(EditorPane pane)
    {
        if (diffLive != pane) return;
        diffRequest?.Cancel();
        DetachDiff();
    }

    private void ClearDiffCache() { diffBytes.Clear(); cachedBytes = 0; }

    private async Task LoadDiff(bool detectRepository = false, bool refresh = false, DiffBase? requested = null,
        DiffFile[]? listedFiles = null, bool probeAvailability = false)
    {
        if (!diffEnabled || ActivePane is not { } live) return;
        ++diffGeneration;
        branchRequest?.Cancel();
        diffRequest?.Cancel();
        using var request = new CancellationTokenSource();
        diffRequest = request;
        var token = request.Token;
        var document = live.Document;
        string? path = document.FilePath;
        var basis = requested ?? diffBase;
        bool git = diffGit;
        DiffFile[] files = listedFiles ?? [];
        string? scope = diffScope;
        string[] unsaved = UnsavedPaths();
        try {
            if (diffLive != live) {
                DetachDiff();
                diffLive = live;
            }
            UpdateDiffToolbar();
            await live.Ready.WaitAsync(token);
            if (listedFiles != null) git = true;
            else if (detectRepository || probeAvailability || basis.IsGit) {
                var gitBase = basis.IsGit ? basis : new DiffBase(VIEM_DIFF_BASE_UNSTAGED);
                if (path == null) { git = false; if (basis.IsGit) basis = new(VIEM_DIFF_BASE_LAST_SAVED); }
                else {
                    try {
                        var listed = await Task.Run(() => DiffSources.Files(gitBase, path, scope, unsaved, token), token);
                        token.ThrowIfCancellationRequested();
                        files = listed; git = true;
                        if (detectRepository) basis = gitBase;
                    } catch (IOException) when ((detectRepository || probeAvailability) && !token.IsCancellationRequested) {
                        git = false;
                        if (basis.IsGit) basis = new(VIEM_DIFF_BASE_LAST_SAVED);
                    }
                }
            }
            token.ThrowIfCancellationRequested();
            if (!basis.IsGit) files = basis.Kind == VIEM_DIFF_BASE_LAST_SAVED && path != null ? [new(path, path, VIEM_DIFF_FILE_MODIFIED)] : [];
            var entry = files.FirstOrDefault(f => SamePath(f.Path, path));
            string basePath = basis.Kind == VIEM_DIFF_BASE_FILE ? basis.Argument : entry?.BasePath ?? path ?? "";
            var key = (basis, basePath);
            byte[] bytes;
            if (basis.Kind == VIEM_DIFF_BASE_LAST_SAVED && !refresh && document.SavedSource() is { } saved) bytes = saved;
            else if (basePath.Length == 0) bytes = [];
            else if (diffBytes.TryGetValue(key, out var cached)) bytes = cached;
            else {
                bytes = await Task.Run(() => DiffSources.Load(basis, basePath, token), token);
                token.ThrowIfCancellationRequested();
                const int limit = 64 * 1024 * 1024;
                if (diffBytes.Count >= 8 || cachedBytes + bytes.Length > limit) ClearDiffCache();
                if (bytes.Length <= limit) { diffBytes[key] = bytes; cachedBytes += bytes.Length; }
            }
            await InstallDiff(live, bytes, entry?.Change == VIEM_DIFF_FILE_DELETED && !document.IsDirty, token);
            token.ThrowIfCancellationRequested();
            diffFiles = files;
            diffBase = basis;
            diffGit = git;
        } catch when (token.IsCancellationRequested) { }
        catch {
            if (comparison == null) EndDiff();
            throw;
        } finally {
            if (diffRequest == request) diffRequest = null;
            if (!closed && !token.IsCancellationRequested) UpdateDiffToolbar();
        }
    }

    private async Task InstallDiff(EditorPane live, byte[] bytes, bool deleted, CancellationToken token)
    {
        var view = live.View!;
        // Retain the live view and its mode across Refresh and base changes.
        comparison?.Dispose();
        comparison = null;
        if (!enteredDiff) {
            originalNumbers = view.NumberEnabled;
            live.Document.EnterDiff(view);
            enteredDiff = true;
        }
        if (deletedSide != deleted) {
            live.Document.ProtectDeletedSide(deleted);
            deletedSide = deleted;
        }
        var document = new CoreDocument(bytes, live.Document.FilePath, VIEM_FORMAT_CODE);
        try {
            // The filename selects syntax only. A diff base owns no disk file or recovery claim.
            document.FilePath = null;
            document.ConfigureEditingDefaults(preferences.Indentation, preferences.Whitespace, preferences.TextWidth, preferences.Associations);
            preferences.AttachThemeDocument(document);
            document.SetReadOnly(true);
            document.SetModifiable(false);
            if (diffLeft == null) {
                diffLeft = new EditorPane(this, document, preferences, diffBasePane: true);
                AutomationProperties.SetName(diffLeft, "Diff base");
                root.ColumnDefinitions[0].Width = new(1, GridUnitType.Star);
                Grid.SetRow(diffLeft, 4);
                root.Children.Add(diffLeft);
            } else {
                var old = diffLeft.Document;
                diffLeft.InstallReplacement(document, diffLeft.PrepareReplacement(document));
                old.Dispose();
            }
        } catch {
            if (diffLeft?.Document != document) document.Dispose();
            throw;
        }
        var leftView = await diffLeft.Ready.WaitAsync(token);
        token.ThrowIfCancellationRequested();
        bool numbers = preferences.Get("diff", "lineNumbers", true);
        if (leftView.NumberEnabled != numbers) leftView.Number(numbers);
        if (view.NumberEnabled != numbers) view.Number(numbers);
        comparison = new(leftView, view, diffAligned);
        SyncDiff();
        if (diffAligned) MirrorDiff(true);
        live.FocusEditor();
    }

    private void SyncDiff()
    {
        comparison!.Sync();
        syncedRevision = diffLive!.Document.State.document_revision;
        diffLive.ClearDiffMarks();
        diffLeft!.ClearDiffMarks();
        diffLeft.Refresh();
        diffLive.Refresh();
    }
    internal void BeforeDiffLayout(EditorPane pane)
    {
        if (comparison != null && diffLive == pane && pane.Document.State.document_revision != syncedRevision) {
            comparison.Sync();
            syncedRevision = pane.Document.State.document_revision;
            diffLeft!.Refresh();
        }
    }
    internal void DiffLayoutUpdated(EditorPane pane, bool scrolled)
    {
        if (comparison == null || pane != diffLive && pane != diffLeft) return;
        pane.UpdateDiffMarks(comparison, pane == diffLive);
        if (scrolled && diffAligned) MirrorDiff(pane == diffLive);
    }
    private void MirrorDiff(bool fromLive)
    {
        if (mirroring || comparison == null) return;
        mirroring = true;
        try { comparison.Mirror(fromLive); }
        finally { mirroring = false; }
    }
    private void NavigateChange(bool next)
    {
        if (comparison == null) return;
        mirroring = true;
        try {
            if (!comparison.Navigate(next)) diffLive?.SetMessage("No changes in this file.");
            diffLive?.Refresh(); diffLeft?.Refresh();
        } finally { mirroring = false; }
        diffLive?.FocusEditor();
    }
    private void SetDiffAlignment(bool enabled)
    {
        if (!diffEnabled) return;
        diffAligned = enabled;
        if (comparison != null) {
            comparison.Align(enabled);
            diffLive?.Refresh(); diffLeft?.Refresh();
            if (enabled) MirrorDiff(true);
        }
        preferences.Set("diff", "alignSides", enabled);
        UpdateDiffToolbar();
    }
    private void SetLineNumbers(bool enabled)
    {
        changingNumbers = true;
        try {
            if (View is { } view && view.NumberEnabled != enabled) view.Number(enabled);
            if (diffEnabled) {
                if (diffLeft?.View is { } left && left.NumberEnabled != enabled) left.Number(enabled);
                preferences.Set("diff", "lineNumbers", enabled);
            } else if (ActivePane?.Document.State.format == VIEM_FORMAT_CODE) preferences.Set("code", "lineNumbers", enabled);
        } finally { changingNumbers = false; }
    }

    internal void DiffNumbersChanged(EditorPane pane, bool enabled)
    {
        if (!changingNumbers && comparison != null && (pane == diffLeft || pane == diffLive)) SetLineNumbers(enabled);
    }
    private void UpdateDiffToolbar()
    {
        menusDirty = true;
        if (diffEnabled) diffToolbar?.Update(diffBase, diffGit, diffBranches, diffFiles, CurrentDiffFile,
            ActivePane?.Document.Name ?? "Untitled", diffAligned, comparison != null);
    }

    private Task RefreshDiff() { ClearDiffCache(); return LoadDiff(refresh: true); }
    private async Task ChooseDiffBase(DiffBase choice)
    {
        if (choice.Kind == VIEM_DIFF_BASE_BRANCH && choice.Argument.Length == 0) {
            if (ActivePane?.Document.FilePath is not { } path) return;
            branchRequest?.Cancel();
            using var request = new CancellationTokenSource();
            branchRequest = request;
            try {
                var branches = await Task.Run(() => DiffSources.Branches(path, request.Token));
                request.Token.ThrowIfCancellationRequested();
                diffBranches = branches;
                UpdateDiffToolbar();
                diffToolbar?.OpenBranches();
            } finally { if (branchRequest == request) branchRequest = null; }
            return;
        }
        ClearDiffCache();
        await LoadDiff(requested: choice);
    }
    private async Task ChooseDiffFile()
    {
        long generation = diffGeneration;
        var picker = new FileOpenPicker();
        WinRT.Interop.InitializeWithWindow.Initialize(picker, Hwnd);
        picker.FileTypeFilter.Add("*");
        if (await picker.PickSingleFileAsync() is { } file && diffEnabled && generation == diffGeneration)
            await ChooseDiffBase(new(VIEM_DIFF_BASE_FILE, file.Path));
    }
    private Task MoveDiffFile(int delta)
    {
        if (diffFiles.Length == 0) return Task.CompletedTask;
        int index = CurrentDiffFile;
        if (index < 0) index = delta > 0 ? -1 : 0;
        return SelectDiffFile((index + delta + diffFiles.Length) % diffFiles.Length);
    }
    private async Task SelectDiffFile(int index)
    {
        if ((uint)index >= (uint)diffFiles.Length || index == CurrentDiffFile || ActivePane is not { } pane) return;
        long generation = diffGeneration;
        var entries = diffFiles;
        try {
            if (!await ConfirmDiscard(pane, alwaysReview: true) || !diffEnabled || entries != diffFiles || pane != ActivePane) return;
            await OpenPath(entries[index].Path, targetPane: pane, force: true, compareHere: true, diffGenerationAtOpen: generation);
        } finally { UpdateDiffToolbar(); }
    }
    private async Task ChooseDiffFolder()
    {
        var picker = new FolderPicker();
        WinRT.Interop.InitializeWithWindow.Initialize(picker, Hwnd);
        picker.FileTypeFilter.Add("*");
        if (await picker.PickSingleFolderAsync() is { } folder) await DiffFolder(folder.Path);
    }
    internal async Task DiffFolder(string path)
    {
        folderRequest?.Cancel();
        using var request = new CancellationTokenSource();
        folderRequest = request;
        try {
            var unsaved = UnsavedPaths();
            var basis = new DiffBase(VIEM_DIFF_BASE_UNSTAGED);
            var files = await Task.Run(() => DiffSources.Files(basis, path, path, unsaved, request.Token));
            request.Token.ThrowIfCancellationRequested();
            if (files.Length == 0) { ActivePane?.SetMessage("There are no changes in " + path + "."); return; }
            bool pristine = Panes.Count == 1 && ActivePane is { } pane && pane.Document.FilePath == null && !pane.Document.IsDirty
                && pane.Document.State.source_byte_count == 0
                && (pane.Document.State.flags & (VIEM_DOCUMENT_STATE_CAN_UNDO | VIEM_DOCUMENT_STATE_CAN_REDO)) == 0;
            var target = pristine ? this : NewWindow(null);
            try {
                await target.OpenPath(files[0].Path, compareHere: true, cancellation: request.Token);
                if (target.diffEnabled) target.EndDiff();
                target.BeginDiff();
                target.windowPlacement.ApplyDiffFrame(target, cascade: !pristine);
                target.diffScope = path;
                await target.LoadDiff(requested: basis, listedFiles: files);
            } catch {
                if (!pristine && target.ActivePane is { } empty && empty.Document.FilePath == null && !empty.Document.IsDirty)
                    await target.ClosePane(empty, force: true);
                throw;
            }
        } finally { if (folderRequest == request) folderRequest = null; }
    }
}

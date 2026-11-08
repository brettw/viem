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
    private DiffRequest? diffRequest;
    private CancellationTokenSource? folderRequest, branchRequest;
    private long diffGeneration;
    private readonly Dictionary<(DiffBase, string), byte[]> diffBytes = [];
    private long cachedBytes;
    private readonly Dictionary<(DiffBase, string), (CancellationTokenSource Cancel, Task<byte[]> Bytes)> prefetches = [];
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

    private void ClearDiffCache()
    {
        foreach (var pending in prefetches.Values.ToArray()) pending.Cancel.Cancel();
        prefetches.Clear();
        diffBytes.Clear();
        cachedBytes = 0;
    }

    private void CacheDiffBytes((DiffBase, string) key, byte[] bytes)
    {
        const int limit = 64 * 1024 * 1024;
        if (bytes.Length > limit || diffBytes.ContainsKey(key)) return;
        while (diffBytes.Count >= 8 || cachedBytes + bytes.Length > limit) {
            var oldest = diffBytes.First();
            diffBytes.Remove(oldest.Key);
            cachedBytes -= oldest.Value.Length;
        }
        diffBytes.Add(key, bytes);
        cachedBytes += bytes.Length;
    }

    private async Task<byte[]> LoadDiffBytes(DiffBase basis, string path, CancellationToken token)
    {
        var key = (basis, path);
        if (diffBytes.TryGetValue(key, out var cached)) return cached;
        var bytes = prefetches.TryGetValue(key, out var pending)
            ? await pending.Bytes.WaitAsync(token)
            : await Task.Run(() => DiffSources.Load(basis, path, token), token);
        token.ThrowIfCancellationRequested();
        CacheDiffBytes(key, bytes);
        return bytes;
    }

    private void PrefetchDiffNeighbours()
    {
        int index = CurrentDiffFile;
        if (!diffBase.IsGit || index < 0 || diffFiles.Length < 2) return;
        foreach (int step in new[] { 1, -1 }) {
            var neighbour = diffFiles[(index + step + diffFiles.Length) % diffFiles.Length];
            var key = (diffBase, neighbour.BasePath);
            if (prefetches.Count < 2 && !diffBytes.ContainsKey(key) && !prefetches.ContainsKey(key))
                _ = Prefetch(key);
        }
    }
    private async Task Prefetch((DiffBase Basis, string Path) key)
    {
        using var cancel = new CancellationTokenSource();
        var loading = Task.Run(() => DiffSources.Load(key.Basis, key.Path, cancel.Token), cancel.Token);
        prefetches.Add(key, (cancel, loading));
        try {
            var bytes = await loading;
            if (!cancel.IsCancellationRequested) CacheDiffBytes(key, bytes);
        } catch (Exception) { /* A foreground request reports a base that cannot be read. */ }
        finally {
            if (prefetches.TryGetValue(key, out var pending) && pending.Bytes == loading) prefetches.Remove(key);
        }
    }

    private sealed class DiffRequest(EditorWindow owner, CancellationToken cancellation) : IDisposable
    {
        private readonly CancellationTokenSource source = CancellationTokenSource.CreateLinkedTokenSource(cancellation);
        internal CancellationToken Token => source.Token;
        internal void Cancel() => source.Cancel();
        public void Dispose()
        {
            if (owner.diffRequest == this) owner.diffRequest = null;
            source.Dispose();
        }
    }
    private DiffRequest StartDiffRequest(CancellationToken cancellation = default)
    {
        ++diffGeneration;
        branchRequest?.Cancel();
        diffRequest?.Cancel();
        return diffRequest = new(this, cancellation);
    }

    // A private base core may be constructed on a worker. Its view and native
    // drawing resources are created on the dispatcher only after validation.
    private sealed class PreparedDiff(CoreDocument document, DiffBase basis, DiffFile[] files, bool git, bool deleted) : IDisposable
    {
        internal readonly CoreDocument Document = document;
        internal readonly DiffBase Basis = basis;
        internal readonly DiffFile[] Files = files;
        internal readonly bool Git = git, Deleted = deleted;
        internal bool Installed;
        public void Dispose() { if (!Installed) Document.Dispose(); }
    }

    private async Task<PreparedDiff> PrepareDiff(string? path, DiffBase basis, DiffFile[]? listedFiles,
        Func<byte[]?>? savedSource, bool detectRepository, bool probeAvailability, CancellationToken token)
    {
        bool git = diffGit;
        var files = listedFiles ?? [];
        if (listedFiles != null) git = true;
        else if (detectRepository || probeAvailability || basis.IsGit) {
            var gitBase = basis.IsGit ? basis : new DiffBase(VIEM_DIFF_BASE_UNSTAGED);
            if (path == null) { git = false; if (basis.IsGit) basis = new(VIEM_DIFF_BASE_LAST_SAVED); }
            else {
                var scope = diffScope;
                var unsaved = UnsavedPaths();
                try {
                    files = await Task.Run(() => DiffSources.Files(gitBase, path, scope, unsaved, token), token);
                    git = true;
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
        byte[] bytes;
        if (basis.Kind == VIEM_DIFF_BASE_LAST_SAVED && savedSource?.Invoke() is { } saved) bytes = saved;
        else if (basePath.Length == 0) bytes = [];
        else bytes = await LoadDiffBytes(basis, basePath, token);

        var document = await Task.Run(() => new CoreDocument(bytes, path, VIEM_FORMAT_CODE), token);
        try {
            token.ThrowIfCancellationRequested();
            document.FilePath = null;
            document.ConfigureEditingDefaults(preferences.Indentation, preferences.Whitespace, preferences.TextWidth, preferences.Associations);
            preferences.AttachThemeDocument(document);
            document.SetReadOnly(true);
            document.SetModifiable(false);
            return new(document, basis, files, git, entry?.Change == VIEM_DIFF_FILE_DELETED);
        } catch { document.Dispose(); throw; }
    }

    private async Task LoadDiff(bool detectRepository = false, bool refresh = false, DiffBase? requested = null,
        DiffFile[]? listedFiles = null, bool probeAvailability = false)
    {
        if (!diffEnabled || ActivePane is not { } live) return;
        using var request = StartDiffRequest();
        var token = request.Token;
        try {
            if (diffLive != live) {
                DetachDiff();
                diffLive = live;
            }
            UpdateDiffToolbar();
            var basis = requested ?? diffBase;
            Func<byte[]?>? saved = refresh ? null : live.Document.SavedSource;
            using var prepared = await PrepareDiff(live.Document.FilePath, basis, listedFiles, saved, detectRepository, probeAvailability, token);
            await live.Ready.WaitAsync(token);
            token.ThrowIfCancellationRequested();
            await InstallDiff(live, prepared, token);
        } catch when (token.IsCancellationRequested) { }
        catch {
            if (comparison == null) EndDiff();
            throw;
        } finally {
            if (!closed && !token.IsCancellationRequested) UpdateDiffToolbar();
        }
    }

    private void AttachFirstBase(PreparedDiff prepared, bool numbers)
    {
        var left = new EditorPane(this, prepared.Document, preferences, diffBasePane: true) { InitialNumber = numbers, Opacity = 0 };
        diffLeft = left;
        prepared.Installed = true;
        AutomationProperties.SetName(left, "Diff base");
        root.ColumnDefinitions[0].Width = new(1, GridUnitType.Star);
        Grid.SetRow(left, 4);
        root.Children.Add(left);
    }

    private async Task InstallDiff(EditorPane live, PreparedDiff prepared, CancellationToken token)
    {
        var view = live.View!;
        bool numbers = preferences.Get("diff", "lineNumbers", true);
        comparison?.Dispose();
        comparison = null;
        if (diffLeft == null) {
            AttachFirstBase(prepared, numbers);
        } else {
            var old = diffLeft.Document;
            diffLeft.InstallReplacement(prepared.Document, diffLeft.PrepareReplacement(prepared.Document));
            prepared.Installed = true;
            old.Dispose();
        }
        var leftView = await diffLeft!.Ready.WaitAsync(token);
        token.ThrowIfCancellationRequested();
        if (!enteredDiff) {
            originalNumbers = view.NumberEnabled;
            live.Document.EnterDiff(view);
            enteredDiff = true;
        }
        bool deleted = prepared.Deleted && !live.Document.IsDirty;
        if (deletedSide != deleted) {
            live.Document.ProtectDeletedSide(deleted);
            deletedSide = deleted;
        }
        if (leftView.NumberEnabled != numbers) leftView.Number(numbers);
        if (view.NumberEnabled != numbers) view.Number(numbers);
        comparison = new(leftView, view, diffAligned);
        SyncDiff();
        if (diffAligned) MirrorDiff(true);
        diffLeft.Opacity = 1;
        diffBase = prepared.Basis; diffFiles = prepared.Files; diffGit = prepared.Git;
        PrefetchDiffNeighbours();
        live.FocusEditor();
    }

    // All native views, diff geometry and number settings are ready before either
    // existing canvas changes documents. No dispatcher yield separates the swaps.
    private void SwitchDiffFile(EditorPane live, CoreDocument document, PreparedDiff prepared)
    {
        var left = diffLeft!;
        var oldBase = left.Document;
        var oldLive = live.Document;
        var oldView = live.View!;
        var liveView = live.PrepareReplacement(document, newFile: true)!;
        CoreView? baseView = null;
        DiffComparison? next = null;
        bool entered = false, protectedFile = false;
        bool restoreNumbers = liveView.NumberEnabled;
        bool numbers = preferences.Get("diff", "lineNumbers", true);
        try {
            baseView = prepared.Installed ? left.View! : left.PrepareReplacement(prepared.Document, newFile: true)!;
            document.EnterDiff(liveView);
            entered = true;
            if (prepared.Deleted && !document.IsDirty) { document.ProtectDeletedSide(true); protectedFile = true; }
            if (liveView.NumberEnabled != numbers) liveView.Number(numbers);
            if (baseView.NumberEnabled != numbers) baseView.Number(numbers);
            next = new(baseView, liveView, diffAligned);
            next.Sync();
            if (diffAligned) next.Mirror(true);
        } catch {
            next?.Dispose();
            if (protectedFile) document.ProtectDeletedSide(false);
            if (entered) document.LeaveDiff(liveView);
            if (!prepared.Installed) baseView?.Dispose();
            liveView.Dispose();
            throw;
        }

        comparison?.RetireWithViews();
        comparison = null;
        if (deletedSide) oldLive.ProtectDeletedSide(false);
        if (enteredDiff) oldLive.LeaveDiff(oldView, closingDocument: oldLive != document && !HasSurvivingView(oldLive, live));
        live.AlternatePath = oldLive.FilePath ?? live.AlternatePath;
        live.InstallReplacement(document, liveView, deferPresentation: true);
        bool firstBase = prepared.Installed;
        if (!firstBase) {
            left.InstallReplacement(prepared.Document, baseView, deferPresentation: true);
            prepared.Installed = true;
        }
        diffLive = live;
        originalNumbers = restoreNumbers;
        enteredDiff = true; deletedSide = protectedFile;
        comparison = next;
        syncedRevision = document.State.document_revision;
        diffFiles = prepared.Files; diffBase = prepared.Basis; diffGit = prepared.Git;
        live.ClearDiffMarks(); left.ClearDiffMarks();
        if (firstBase) left.Refresh(); else left.FinishReplacement();
        live.FinishReplacement();
        left.Opacity = 1;
        if (!firstBase) oldBase.Dispose();
        ReleaseDetachedDocument(oldLive);
        TrackExternalFile(document);
        PaneReady(live);
        UpdateDiffToolbar();
        PrefetchDiffNeighbours();
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
        UpdateDiffToolbar(); // The selected label follows the pair actually on screen.
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
            long generation = target.diffGeneration;
            try {
                await target.ActivePane!.Ready.WaitAsync(request.Token);
                if (target.diffEnabled) target.EndDiff();
                target.BeginDiff();
                target.windowPlacement.ApplyDiffFrame(target, cascade: !pristine);
                target.diffScope = path;
                target.diffBase = basis;
                target.diffLive = target.ActivePane;
                var opening = target.OpenPath(files[0].Path, compareHere: true, cancellation: request.Token, initialDiffFiles: files);
                generation = target.diffGeneration;
                await opening;
            } catch {
                if (generation != target.diffGeneration) throw;
                if (target.diffEnabled && target.comparison == null && target.diffScope == path) target.EndDiff();
                if (!pristine && target.ActivePane is { } empty && empty.Document.FilePath == null && !empty.Document.IsDirty)
                    await target.ClosePane(empty, force: true);
                throw;
            }
        } finally { if (folderRequest == request) folderRequest = null; }
    }
}

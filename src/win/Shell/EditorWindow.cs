using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Windowing;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Windows.Storage.Pickers;
using System.Security.Cryptography;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow : Window
{
    private readonly Preferences preferences;
    private readonly Action capturePlacement;
    private readonly Grid root = new();
    private readonly Grid titleBar = new() { Height = 32 };
    private readonly Grid titleDrag = new() { Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
    private readonly TextBlock titleText = new() { FontSize = 12, VerticalAlignment = VerticalAlignment.Center, Margin = new(12, 0, 0, 0), TextTrimming = TextTrimming.CharacterEllipsis };
    private readonly ToggleButton menuToggle = new() { Width = 40, Height = 30, MinHeight = 0, Padding = new(0), BorderThickness = new(0), Content = new FontIcon { Glyph = "\uE700", FontSize = 14 } };
    internal MenuBar Menu { get; } = new();
    private readonly PaneStackPanel paneGrid = new();
    internal PaneStackPanel PaneStack => paneGrid;
    internal List<EditorPane> Panes { get; } = [];
    internal EditorPane? ActivePane { get; private set; }
    internal bool IsWindowActive { get; private set; }
    private readonly DispatcherTimer poll = new() { Interval = TimeSpan.FromMilliseconds(40) };
    private bool closing, closed, updatingPreferences;
    private EditorPane? lastPane;
    private readonly Dictionary<CoreDocument, byte[]> savedSources = [];
    private static readonly Dictionary<CoreDocument, string> styleDefaultsWarnings = [];
    private static readonly Dictionary<CoreDocument, DocumentRecovery> recoveries = [];
    private Task effectQueue = Task.CompletedTask;
#if DEBUG
    internal Task PendingEffectsForTesting => effectQueue;
    internal Func<CoreDocument, Task<ContentDialogResult>>? CloseReviewDecisionForTesting { get; set; }
    internal Task RequestCloseForTesting() => RequestClose();
    internal Func<Task>? BeforePublishOpenForTesting { get; set; }
    internal Func<string, Task<string?>>? PickDirectoryFileForTesting { get; set; }
    internal bool HasSavedBaselineForTesting(CoreDocument document, byte[] source) =>
        savedSources.TryGetValue(document, out var baseline) && baseline.AsSpan().SequenceEqual(SHA256.HashData(source));
#endif
    private int pollTicks;
    internal nint Hwnd => WinRT.Interop.WindowNative.GetWindowHandle(this);

    public EditorWindow(Preferences preferences, CoreDocument? document = null, DocumentWindowPlacement? placement = null, bool openLaunchFiles = false)
    {
        using var startup = Diagnostics.StartupPerformance.Measure("window.initialize");
        this.preferences = preferences;
        paneGrid.Configure(preferences);
        formattingToolbar = new();
        ConfigureFormattingToolbar();
        menuToggle.Resources = new ResourceDictionary { Source = new Uri("ms-appx:///Shell/MenuToggleResources.xaml") };
        Diagnostics.StartupPerformance.Mark("window.resourcesReady");
        Title = "Viem";
        using (Diagnostics.StartupPerformance.Measure("window.attachContent")) Content = root;
        root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new() { Height = new(1, GridUnitType.Star) });
        root.Children.Add(titleBar); root.Children.Add(Menu); Grid.SetRow(Menu, 1); root.Children.Add(formattingToolbar); Grid.SetRow(formattingToolbar, 2); root.Children.Add(paneGrid); Grid.SetRow(paneGrid, 3);
        titleBar.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); titleBar.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); titleBar.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); titleBar.ColumnDefinitions.Add(new() { Width = new(138) });
        titleBar.Children.Add(titleDrag); titleDrag.Children.Add(titleText); titleBar.Children.Add(menuToggle); Grid.SetColumn(menuToggle, 1);
        titleBar.Children.Add(toolbarToggle); Grid.SetColumn(toolbarToggle, 2);
        ExtendsContentIntoTitleBar = true; SetTitleBar(titleDrag);
        Diagnostics.StartupPerformance.Mark("window.chromeReady");
        capturePlacement = (placement ?? App.Instance.WindowPlacement).Track(this);
        WindowSizing.Appearance(this, preferences.Midnight);
        Diagnostics.StartupPerformance.Mark("window.placementReady");
        AppWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Standard;
        titleBar.SizeChanged += (_, _) => UpdateCaptionInset();
        AutomationProperties.SetName(menuToggle, "Show menu bar"); ToolTipService.SetToolTip(menuToggle, "Show or hide the menu bar");
        menuToggle.Click += (_, _) => Safe(() => { preferences.Set("windows", "showMenu", menuToggle.IsChecked == true); return Task.CompletedTask; });
        Activated += (_, e) =>
        {
            IsWindowActive = e.WindowActivationState != WindowActivationState.Deactivated;
            if (!IsWindowActive) formattingToolbar.DismissPopups();
            if (IsWindowActive && ActivePane is { View: { } view } activePane)
                activePane.Run(() => styleInspector?.FollowActiveView(view));
            foreach (var pane in Panes) pane.Canvas.Invalidate();
        };
        AppWindow.Closing += (_, e) => { if (!closing) { e.Cancel = true; Safe(RequestClose); } };
        Closed += (_, _) => {
            closed = true; poll.Stop(); preferences.Changed -= ApplyPreferences; preferences.RecentChanged -= RefreshRecentMenu; preferences.ThemesChanged -= RefreshThemeMenu;
            settingsWindow?.Close();
            var documents = Panes.Select(p => p.Document).Distinct().ToArray();
            foreach (var pane in Panes) pane.Dispose(); Panes.Clear(); paneGrid.Dispose();
            foreach (var doc in documents) if (!App.Instance.Windows.Where(w => w != this).Any(w => w.Panes.Any(p => p.Document == doc))) { App.Instance.BlockingEdits.Released(doc); doc.Dispose(); }
        };
        preferences.Changed += ApplyPreferences;
        preferences.RecentChanged += RefreshRecentMenu; preferences.ThemesChanged += RefreshThemeMenu;
        using (Diagnostics.StartupPerformance.Measure("window.menus")) BuildMenus();
        ApplyPreferences();
        if (openLaunchFiles && HasLaunchFiles())
        {
            // The real document can be opened as soon as the shell is arranged.
            // Do not create and shape an empty editor merely to trigger startup.
            root.Loaded += (_, _) => BeginFileLaunch();
        }
        else using (Diagnostics.StartupPerformance.Measure("window.initialPane")) AddPane(document ?? NewDocument());
        WindowSizing.TrackMinimumSize(this, () => new(Math.Max(480, paneGrid.MinimumSize.Width),
            Math.Max(280, paneGrid.MinimumSize.Height + Math.Max(0, root.ActualHeight - paneGrid.ActualHeight))));
        poll.Tick += (_, _) => {
            foreach (var doc in Panes.Select(p => p.Document).Distinct().ToArray()) ActivePane?.Run(() => doc.PollSyntax());
            foreach (var pane in Panes.ToArray()) pane.Poll();
            if (++pollTicks % 50 == 0) Safe(CheckExternalChanges);
        };
        poll.Start();
    }
    public new void Close()
    {
        if (formattingToolbar.DismissPopups()) { closeAfterToolbarPopup = true; return; }
        // WinUI's Closed event may run after the HWND is gone. Sample before
        // native teardown, including a move whose Changed callback is queued.
        capturePlacement();
        base.Close();
    }
    private void UpdateCaptionInset()
    {
        double scale = root.XamlRoot?.RasterizationScale ?? 1;
        double inset = AppWindow.TitleBar.RightInset / scale;
        titleBar.ColumnDefinitions[3].Width = new(Math.Max(inset, 138 / scale));
    }
    private void ApplyPreferences()
    {
        if (updatingPreferences) return;
        updatingPreferences = true;
        root.RequestedTheme = preferences.Midnight ? ElementTheme.Dark : ElementTheme.Light;
        root.Background = new SolidColorBrush(preferences.Theme.Background);
        Menu.Visibility = preferences.ShowMenu ? Visibility.Visible : Visibility.Collapsed;
        menuToggle.IsChecked = preferences.ShowMenu;
        AppWindow.TitleBar.ButtonBackgroundColor = Microsoft.UI.Colors.Transparent;
        AppWindow.TitleBar.ButtonInactiveBackgroundColor = Microsoft.UI.Colors.Transparent;
        AppWindow.TitleBar.ButtonForegroundColor = preferences.Midnight ? Microsoft.UI.Colors.White : Microsoft.UI.Colors.Black;
        titleBar.Background = Menu.Background = formattingToolbar.Background = new SolidColorBrush(preferences.Midnight ? Theme.Rgb(31, 31, 31) : Theme.Rgb(243, 243, 243));
        SynchronizeFormattingToolbar();
        updatingPreferences = false;
        RefreshRecentMenu();
    }
    private CoreDocument NewDocument(byte[]? source = null, string? path = null, uint? format = null, uint encoding = 0, uint fileFormat = 0)
    {
        var doc = new CoreDocument(source ?? [], path, format, encoding, fileFormat, preferences.MarkdownFormattedView);
        return ConfigureNewDocument(doc, source ?? [], path);
    }
    private CoreDocument ConfigureNewDocument(CoreDocument doc, byte[] source, string? path)
    {
        using var startup = Diagnostics.StartupPerformance.Measure("document.configure");
        try
        {
            doc.ConfigureEditingDefaults(preferences.Indentation, preferences.Whitespace, preferences.TextWidth, preferences.Associations);
            string[] diagnostics = [];
            doc.ConfigureForEditing("theme styles", () => diagnostics = preferences.AttachThemeDocument(doc));
            if (diagnostics.Length > 0) styleDefaultsWarnings[doc] = string.Join(Environment.NewLine, diagnostics);
            if (preferences.StartupCommands.Length > 0) doc.ConfigureForEditing("startup.viem", () => doc.InitializeStartup(preferences.StartupCommands));
            doc.ConfigureForEditing("selection settings", () => GlobalSelectionOptions.Attach(doc, preferences.DirectoryPath));
            if (path != null) {
                savedSources[doc] = SHA256.HashData(source ?? []);
                doc.ConfigureForEditing("file attributes", () => {
                    if (File.Exists(path) && (File.GetAttributes(path) & FileAttributes.ReadOnly) != 0) doc.SetReadOnly(true);
                });
                doc.ConfigureForEditing("recovery backups", () => {
                    recoveries[doc] = DocumentRecovery.Claim(doc, path, preferences.DirectoryPath, DispatcherQueue, e => ActivePane?.Report(e));
                });
            }
            doc.Disposed += () => { recoveries.Remove(doc); styleDefaultsWarnings.Remove(doc); };
            return doc;
        }
        catch { styleDefaultsWarnings.Remove(doc); doc.Dispose(); throw; }
    }
    internal void PaneReady(EditorPane pane)
    {
        ApplyInitialPaneHeight(pane);
        if (pane == ActivePane)
        {
            if (IsWindowActive && pane.View is { } view) pane.Run(() => styleInspector?.FollowActiveView(view));
            pane.FocusEditor();
        }
        UpdateTitle(); RefreshStyleMenus();
        var diagnostics = new List<string>();
        if (styleDefaultsWarnings.Remove(pane.Document, out string? warning)) diagnostics.Add(warning);
        if (pane.Document.StartupDiagnostics.Length > 0) diagnostics.Add(Path.Combine(preferences.DirectoryPath, pane.Document.StartupDiagnostics));
        if (pane.Document.ConfigurationDiagnostics.Length > 0) diagnostics.Add(pane.Document.ConfigurationDiagnostics);
        if (diagnostics.Count > 0) pane.SetMessage(string.Join(Environment.NewLine, diagnostics));
        OnPaneReady(pane);
    }
    partial void OnPaneReady(EditorPane pane);
    internal EditorPane AddPane(CoreDocument doc, int? position = null, int? splitIndex = null, bool vertical = false, EditorPane? replacing = null)
    {
        using var startup = Diagnostics.StartupPerformance.Measure("pane.construct");
        if (!savedSources.ContainsKey(doc))
        {
            var owner = App.Instance.Windows.FirstOrDefault(w => !w.closed
                && w.Panes.Any(p => p.Document == doc) && w.savedSources.ContainsKey(doc));
            if (owner != null) savedSources[doc] = owner.savedSources[doc];
        }
        preferences.ObserveMarkdownView(doc, error => App.Instance.Windows.SelectMany(w => w.Panes)
            .FirstOrDefault(p => p.Document == doc)?.Report(error));
        GlobalSelectionOptions.Attach(doc, preferences.DirectoryPath);
        preferences.AttachThemeDocument(doc, initialize: false);
        var pane = new EditorPane(this, doc, preferences); pane.Focused += SetActive;
        pane.RememberedArgument = ActivePane?.RememberedArgument ?? ulong.MaxValue;
        if (replacing != null)
        {
            int index = Panes.IndexOf(replacing); Panes[index] = pane;
            pane.RememberedArgument = replacing.RememberedArgument;
            paneGrid.ReplacePane(replacing, pane); replacing.Focused -= SetActive; replacing.Dispose();
            if (lastPane == replacing) lastPane = null;
            if (!App.Instance.Windows.SelectMany(w => w.Panes).Any(p => p.Document == replacing.Document)) { savedSources.Remove(replacing.Document); App.Instance.BlockingEdits.Released(replacing.Document); replacing.Document.Dispose(); }
        }
        else { Panes.Insert(position ?? Panes.Count, pane); RebuildPanes(splitIndex, vertical); }
        SetActive(pane); return pane;
    }
    internal void FocusPane(EditorPane pane, bool keepStatusFocus = false) { SetActive(pane); if (!keepStatusFocus) pane.FocusEditor(); }
    private void SetActive(EditorPane pane)
    {
        // WinUI can deliver a queued focus event after a pane was detached.
        if (closed || !Panes.Contains(pane) || pane.Document.Handle == 0) return;
        if (ActivePane != pane)
        {
            lastPane = ActivePane; ActivePane = pane;
        }
        if (IsWindowActive && pane.View is { } view) pane.Run(() => styleInspector?.FollowActiveView(view));
        foreach (var item in Panes) item.IsActive = item == pane;
        UpdateTitle(); RefreshStyleMenus();
    }
    private void RebuildPanes(int? splitIndex = null, bool vertical = false) => paneGrid.Rebuild(Panes, splitIndex, vertical);
    private void SyncPaneOrder() { var next = paneGrid.OrderedPanes.ToArray(); Panes.Clear(); Panes.AddRange(next); }

    internal void RequireSplitRoom(EditorPane? pane = null, bool vertical = false)
    {
        pane ??= ActivePane;
        if (pane == null || !Panes.Contains(pane)) throw new InvalidOperationException("No room to split the current view.");
        paneGrid.RequireSplitRoom(pane, Math.Max(28, preferences.StatusFontSize + 12), vertical);
    }

    internal EditorPane SplitPane(EditorPane pane, CoreDocument document, ulong? lines = null, bool vertical = false)
    {
        RequireSplitRoom(pane, vertical);
        var added = AddPane(document, Panes.IndexOf(pane) + 1, splitIndex: Panes.IndexOf(pane), vertical: vertical);
        SyncPaneOrder();
        paneGrid.UpdateLayout();
        added.InitialHeightLines = lines; added.InitialVerticalSplit = vertical;
        ApplyInitialPaneHeight(added);
        return added;
    }

    private void ApplyInitialPaneHeight(EditorPane pane)
    {
        if (pane.View is not { } view || pane.InitialHeightLines is not { } lines || !Panes.Contains(pane)) return;
        pane.InitialHeightLines = null;
        if (pane.InitialVerticalSplit) paneGrid.ResizeWidth(pane, lines * (double)view.DefaultColumnWidth);
        else paneGrid.ResizePane(pane, lines * (double)view.DefaultLineHeight);
    }

    internal void UpdateTitle()
    {
        if (ActivePane == null || closed) return;
        string value = ActivePane.Document.Name + (ActivePane.Document.IsDirty ? " •" : "") + (Panes.Count > 1 ? $" · {Panes.Count} panes" : "");
        if (titleText.Text != value) { Title = value + " — Viem"; titleText.Text = value; }
        menusDirty = true;
        SynchronizeFormattingToolbar();
    }
    private void Safe(Func<Task> action) { async void Execute() { try { await action(); } catch (OperationCanceledException) { } catch (Exception e) { ActivePane?.Report(e); } } Execute(); }
    internal async Task OpenDialog()
    {
        var picker = new FileOpenPicker(); WinRT.Interop.InitializeWithWindow.Initialize(picker, Hwnd); picker.FileTypeFilter.Add("*");
        var files = await picker.PickMultipleFilesAsync();
        foreach (var file in files) await OpenNative(file.Path);
    }
    internal async Task OpenNative(string path)
    {
        path = ResolvePath(path);
        if (!File.Exists(path)) throw new FileNotFoundException("The file could not be found.", path);
        if (App.Instance.Windows.SelectMany(w => w.Panes).Any(p => p.Document.FilePath is string file && FileIdentity.Same(file, path))) { await OpenPath(path); return; }
        bool reusable = Panes.Count == 1 && ActivePane is { Document: var current } && current.FilePath == null && current.State.source_byte_count == 0 && !current.IsDirty && (current.State.flags & (VIEM_DOCUMENT_STATE_CAN_UNDO | VIEM_DOCUMENT_STATE_CAN_REDO)) == 0;
        if (reusable) { await OpenPath(path); return; }
        // Read before opening a replacement window, so failed native opens leave
        // the existing window and buffer intact.
        if (File.Exists(path)) _ = await File.ReadAllBytesAsync(path);
        var window = NewWindow(null); await window.OpenPath(path);
    }
    private async Task<string?> PickDirectoryFile(string directory)
    {
#if DEBUG
        if (PickDirectoryFileForTesting is { } pick) return await pick(directory);
#endif
        var picker = new Microsoft.Windows.Storage.Pickers.FileOpenPicker(AppWindow.Id) { SuggestedFolder = directory };
        picker.FileTypeFilter.Add("*");
        return (await picker.PickSingleFileAsync())?.Path;
    }
    internal async Task OpenPath(string path, bool split = false, bool force = false, ulong? splitLines = null,
        bool vertical = false, EditorPane? targetPane = null, bool newWindow = false)
    {
        using var startup = Diagnostics.StartupPerformance.Measure("document.open");
        var old = targetPane ?? ActivePane;
        var expected = old?.Document.State;
        string? originalPath = old?.Document.FilePath;
        bool? originalDirty = old?.Document.IsDirty;
        void ValidateTarget()
        {
            if (closed || (old != null && (!Panes.Contains(old) || old.Document.Handle == 0
                || old.Document.State.document_id != expected!.Value.document_id
                || old.Document.State.document_revision != expected.Value.document_revision
                || old.Document.FilePath != originalPath || old.Document.IsDirty != originalDirty)))
                throw new InvalidOperationException("The document changed while opening the file.");
        }
        void RequireReplacementAllowed()
        {
            if (!split && !newWindow && old != null && !force && old.Document.IsDirty
                && App.Instance.Windows.SelectMany(w => w.Panes).Count(p => p.Document == old.Document) == 1)
                throw new InvalidOperationException("E37: No write since last change (add ! to override).");
        }
        var splitSource = split ? old : null;
        ValidateTarget();
        if (split) RequireSplitRoom(splitSource, vertical);
        path = ResolvePath(path);
        bool pickedFile = Directory.Exists(path);
        if (pickedFile)
        {
            path = await PickDirectoryFile(path) ?? throw new OperationCanceledException();
            ValidateTarget();
            path = ResolvePath(path);
            if (!File.Exists(path)) throw new FileNotFoundException("The selected file could not be found.", path);
            if (!split && !newWindow && old?.Document.FilePath is string current && FileIdentity.Same(current, path))
            {
                await Reload(old, force);
                return;
            }
        }
        (EditorWindow Window, EditorPane Pane) FindExisting() => App.Instance.Windows.Prepend(this).Distinct()
            .Where(w => !w.closed).SelectMany(w => w.Panes.Select(p => (Window: w, Pane: p)))
            .FirstOrDefault(x => x.Pane.Document.FilePath is string named && FileIdentity.Same(named, path));
        var existing = FindExisting();
        bool ShowExisting()
        {
            if (existing.Pane == null || split || newWindow || pickedFile) return false;
            existing.Window.Activate(); existing.Pane.FocusEditor(); preferences.Remember(path); return true;
        }
        if (ShowExisting()) return;
        RequireReplacementAllowed();
        byte[] bytes;
        using (Diagnostics.StartupPerformance.Measure("document.read")) bytes = pickedFile || File.Exists(path) ? await File.ReadAllBytesAsync(path) : [];
        ValidateTarget();
        existing = FindExisting();
        if (ShowExisting()) return;
        RecoverySnapshot? recovered = null; bool readOnly = false;
        if (existing.Pane == null)
        {
            var candidates = DocumentRecovery.Candidates(path, preferences.DirectoryPath);
            if (candidates.Length > 0)
            {
                var snapshot = candidates.Select(c => c.Snapshot).FirstOrDefault(s => s != null);
                var choices = new RadioButtons { ItemsSource = snapshot == null ? new[] { "Open Read-Only", "Edit Anyway" } : new[] { "Open Read-Only", "Edit Anyway", "Recover Unsaved Changes" }, SelectedIndex = 0 };
                var content = new StackPanel { Spacing = 12 }; content.Children.Add(new TextBlock { Text = "Another editing session or recovery file exists for " + Path.GetFileName(path) + ". It will be left untouched.", TextWrapping = TextWrapping.Wrap }); content.Children.Add(choices);
                var dialog = new ContentDialog { XamlRoot = root.XamlRoot, RequestedTheme = root.RequestedTheme, Title = "Existing editing session", Content = content, PrimaryButtonText = "Open", CloseButtonText = "Cancel" };
                if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
                ValidateTarget();
                readOnly = choices.SelectedIndex == 0; if (choices.SelectedIndex == 2) recovered = snapshot;
            }
        }
        CoreDocument doc;
        if (existing.Pane != null) doc = existing.Pane.Document;
        else
        {
            byte[] source = recovered?.source ?? bytes;
            bool markdownFormattedView = preferences.MarkdownFormattedView;
            // Parsing a newly opened source needs no view or native shaper.
            // Keep the shell responsive while constructing that private core.
            doc = await Task.Run(() => new CoreDocument(source, path, recovered?.Format, recovered?.encoding ?? 0, recovered?.fileFormat ?? 0, markdownFormattedView));
#if DEBUG
            if (BeforePublishOpenForTesting is { } beforePublish) await beforePublish();
#endif
            try { ValidateTarget(); RequireReplacementAllowed(); }
            catch { doc.Dispose(); throw; }
            // Another window may have published this file while its bytes or
            // private core were loading. Join that document before claiming
            // recovery ownership or exposing a second history for the file.
            existing = FindExisting();
            if (existing.Pane != null)
            {
                doc.Dispose();
                if (ShowExisting()) return;
                doc = existing.Pane.Document;
                recovered = null; readOnly = false;
            }
            else doc = ConfigureNewDocument(doc, source, path);
        }
        try { ValidateTarget(); RequireReplacementAllowed(); if (split) RequireSplitRoom(splitSource, vertical); }
        catch { if (existing.Pane == null) { savedSources.Remove(doc); doc.Dispose(); } throw; }
        if (recovered != null) { savedSources[doc] = SHA256.HashData(bytes); doc.MarkRecovered(); recoveries[doc].Write(RecoverySnapshot.Capture(doc)); }
        if (readOnly) doc.SetReadOnly(true);
        if (newWindow)
        {
            var window = NewWindow(doc);
            if (savedSources.TryGetValue(doc, out var baseline)) window.savedSources[doc] = baseline;
            if (!Panes.Any(p => p.Document == doc)) savedSources.Remove(doc);
            window.ActivePane!.RememberedArgument = old?.RememberedArgument ?? ulong.MaxValue;
        }
        else
        {
            int index = old == null ? Panes.Count : Panes.IndexOf(old) + (split ? 1 : 0);
            ulong argument = old?.RememberedArgument ?? ulong.MaxValue;
            var opened = split && splitSource != null ? SplitPane(splitSource, doc, splitLines, vertical) : AddPane(doc, Math.Min(index, Panes.Count), replacing: old);
            opened.RememberedArgument = argument;
        }
        preferences.Remember(path);
    }
    internal async Task<bool> Save(EditorPane pane, bool saveAs = false, string? explicitPath = null, bool force = false, bool adoptPath = true, bool native = true)
    {
        var doc = pane.Document;
        string? path = explicitPath == null ? (saveAs ? null : doc.FilePath) : ResolvePath(explicitPath);
        if (path == null)
        {
            var picker = new FileSavePicker { SuggestedFileName = doc.Name == "Untitled" ? "Untitled" : Path.GetFileNameWithoutExtension(doc.Name) };
            string extension = doc.State.format switch {
                2 or 5 => ".md",
                VIEM_FORMAT_CODE when Path.GetExtension(doc.FilePath) is { Length: > 0 } codeExtension => codeExtension,
                _ => ".txt"
            };
            picker.FileTypeChoices.Add(CoreDocument.FormatName(doc.State.format), new List<string> { extension });
            WinRT.Interop.InitializeWithWindow.Initialize(picker, Hwnd);
            var file = await picker.PickSaveFileAsync(); if (file == null) return false; path = file.Path;
        }
        path = Path.GetFullPath(path);
        if (App.Instance.Windows.SelectMany(w => w.Panes).Any(p => p.Document != doc && p.Document.FilePath is string named && FileIdentity.Same(named, path))) throw new IOException("That file is already open in another document.");
        bool samePath = string.Equals(doc.FilePath, path, StringComparison.OrdinalIgnoreCase);
        if (doc.IsReadOnly && samePath && !force)
        {
            if (!native) throw new InvalidOperationException("E45: readonly option is set (use ! to override)");
            if (await Dialog("Read-only document", "Save this document anyway?", "Save Anyway", "Cancel") != ContentDialogResult.Primary) return false;
        }
        if (File.Exists(path) && samePath && savedSources.TryGetValue(doc, out var saved) && !force)
        {
            byte[] disk = await File.ReadAllBytesAsync(path);
            if (!SHA256.HashData(disk).AsSpan().SequenceEqual(saved))
            {
                var result = await Dialog("File changed on disk", "Another program changed this file. Overwrite its current contents with this document?", "Overwrite", "Cancel");
                if (result != ContentDialogResult.Primary) return false;
            }
        }
        else if (File.Exists(path) && explicitPath != null && !samePath && !force)
        { if (await Dialog("Replace file?", $"{path} already exists.", "Replace", "Cancel") != ContentDialogResult.Primary) return false; }
        var state = doc.State; byte[] bytes = doc.Source(state.document_revision);
        DocumentRecovery? replacementRecovery = null;
        try
        {
            if (adoptPath && (!samePath || !recoveries.ContainsKey(doc)))
            {
                replacementRecovery = DocumentRecovery.Claim(doc, path, preferences.DirectoryPath, DispatcherQueue, e => pane.Report(e));
                replacementRecovery.Write(RecoverySnapshot.Capture(doc));
                if (!await replacementRecovery.Pending) throw new IOException("The new recovery snapshot could not be saved. The previous backup was retained.");
            }
            await Task.Run(() => Preferences.AtomicWrite(path, bytes));
        }
        catch { replacementRecovery?.Dispose(); throw; }
        if (samePath || adoptPath)
        {
            if (replacementRecovery != null) { if (recoveries.Remove(doc, out var previous)) previous.Dispose(); recoveries[doc] = replacementRecovery; }
            doc.FilePath = path;
            foreach (var w in App.Instance.Windows.Where(w => w.Panes.Any(p => p.Document == doc))) w.savedSources[doc] = SHA256.HashData(bytes);
            if (doc.State.document_revision == state.document_revision) doc.MarkSaved(state);
            if (!samePath) doc.RedetectLanguage();
            preferences.Remember(path);
        }
        pane.SetMessage($"Saved {Path.GetFileName(path)}"); UpdateTitle();
        return true;
    }
    private bool HasSurvivingView(CoreDocument document, EditorPane? removing = null, bool closingWindow = false)
    {
        if (!closingWindow && Panes.Any(p => p != removing && p.Document == document)) return true;
        return App.Instance.Windows.Any(w => w != this && !w.closed && w.Panes.Any(p => p.Document == document));
    }
    private async Task<bool> ConfirmDiscard(EditorPane pane, bool closingWindow = false)
    {
        if (!pane.Document.IsDirty || HasSurvivingView(pane.Document, pane, closingWindow)) return true;
        ContentDialogResult result;
#if DEBUG
        if (CloseReviewDecisionForTesting is { } decide) result = await decide(pane.Document);
        else
#endif
        result = await Dialog("Save changes?", $"Save changes to {pane.Document.Name} before closing?", "Save", "Cancel", "Discard");
        if (result == ContentDialogResult.Secondary) return true;
        if (result != ContentDialogResult.Primary) return false;
        await Save(pane); return !pane.Document.IsDirty;
    }
    private async Task RequestClose()
    {
        foreach (var doc in Panes.Select(p => p.Document).Distinct().ToArray())
        {
            var pane = Panes.First(p => p.Document == doc);
            if (!await ConfirmDiscard(pane, closingWindow: true)) return;
        }
        closing = true; Close();
    }
    internal async Task ClosePane(EditorPane pane, bool force = false)
    {
        if (closed || !Panes.Contains(pane)) return;
        if (!force && !await ConfirmDiscard(pane)) return;
        if (closed || !Panes.Contains(pane)) return;
        if (Panes.Count == 1) { closing = true; Close(); return; }
        RemovePane(pane); ActivePane?.FocusEditor();
    }
    private void RemovePane(EditorPane pane, bool rebuild = true)
    {
        pane.Focused -= SetActive;
        paneGrid.RemovePane(pane); pane.Dispose(); Panes.Remove(pane);
        if (!App.Instance.Windows.SelectMany(w => w.Panes).Any(p => p.Document == pane.Document)) { savedSources.Remove(pane.Document); App.Instance.BlockingEdits.Released(pane.Document); pane.Document.Dispose(); }
        if (ActivePane == pane) ActivePane = Panes.FirstOrDefault();
        if (rebuild) { RebuildPanes(); paneGrid.Equalize(); if (ActivePane != null) SetActive(ActivePane); }
    }
    internal async Task<ContentDialogResult> Dialog(string title, string text, string primary = "OK", string close = "", string secondary = "")
    {
        var dialog = new ContentDialog { XamlRoot = root.XamlRoot, RequestedTheme = root.RequestedTheme, Title = title, Content = new ScrollViewer { Content = new TextBlock { Text = text, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true }, MaxHeight = 450 }, PrimaryButtonText = primary, CloseButtonText = close, SecondaryButtonText = secondary, DefaultButton = ContentDialogButton.Primary };
        return await dialog.ShowAsync();
    }
    internal Task ApplyEffects(EditorPane pane, HostEffects effects)
    {
        GlobalSelectionOptions.Observe(pane.Document, effects.SelectionOptions);
        Task previous = effectQueue;
        async Task Next() { await previous; await ExecuteEffects(pane, effects, 0); }
        return effectQueue = Next();
    }
    private async Task ExecuteEffects(EditorPane pane, HostEffects effects, uint sourceDepth)
    {
            try
            {
                if (effects.Output.Length > 0) pane.ShowCommandOutput(string.Join("\n", effects.Output));
                foreach (var request in effects.Requests)
                {
                    var r = request.Value; bool force = (r.flags & VIEM_EX_FRONTEND_FORCE) != 0;
                    // Reload retains the pane but replaces its document. A
                    // queued command still belongs to the document that emitted it.
                    if (pane.View == null || r.document_id != pane.Document.State.document_id) break;
                    switch (r.kind)
                    {
                        case VIEM_EX_FRONTEND_SPLIT:
                            if (request.Text.Length > 0) await OpenPath(request.Text, true, force, (r.flags & VIEM_EX_FRONTEND_HAS_COUNT) != 0 ? r.window_count : null, vertical: (r.flags & VIEM_EX_FRONTEND_VERTICAL) != 0, targetPane: pane);
                            else SplitPane(pane, pane.Document, (r.flags & VIEM_EX_FRONTEND_HAS_COUNT) != 0 ? r.window_count : null, (r.flags & VIEM_EX_FRONTEND_VERTICAL) != 0);
                            break;
                        case VIEM_EX_FRONTEND_NEW_PANE:
                            RequireSplitRoom(pane, (r.flags & VIEM_EX_FRONTEND_VERTICAL) != 0);
                            SplitPane(pane, NewDocument(), (r.flags & VIEM_EX_FRONTEND_HAS_COUNT) != 0 ? r.window_count : null, (r.flags & VIEM_EX_FRONTEND_VERTICAL) != 0);
                            break;
                        case VIEM_EX_FRONTEND_NEW: if (force || await ConfirmDiscard(pane)) AddPane(NewDocument(), replacing: pane); break;
                        case VIEM_EX_FRONTEND_EDIT: if (request.Text.Length > 0 && !string.Equals(ResolvePath(request.Text), pane.Document.FilePath, StringComparison.OrdinalIgnoreCase)) await OpenPath(request.Text, false, force, targetPane: pane); else if (pane.Document.FilePath != null) await Reload(pane, force); break;
                        case VIEM_EX_FRONTEND_EDIT_NEW_WINDOW:
                            if (request.Text.Length > 0) await OpenPath(request.Text, targetPane: pane, newWindow: true);
                            else NewWindow(null);
                            break;
                        case VIEM_EX_FRONTEND_WRITE: case VIEM_EX_FRONTEND_SAVE_AS: await ExWrite(pane, request); break;
                        case VIEM_EX_FRONTEND_QUIT: await ClosePane(pane, force); break;
                        case VIEM_EX_FRONTEND_CQUIT:
                            int exitCode = unchecked((int)r.window_count);
                            App.Instance.BlockingEdits.Abort(exitCode);
                            foreach (var w in App.Instance.Windows.ToArray()) { w.closing = true; w.Close(); }
                            Environment.Exit(exitCode);
                            break;
                        case VIEM_EX_FRONTEND_QUIT_ALL: foreach (var w in App.Instance.Windows.ToArray()) { if (force) { w.closing = true; w.Close(); } else await w.RequestClose(); } break;
                        case VIEM_EX_FRONTEND_WRITE_QUIT: case VIEM_EX_FRONTEND_XIT:
                            if (r.kind == VIEM_EX_FRONTEND_WRITE_QUIT || pane.Document.IsDirty || request.Text.Length > 0)
                                if (!await ExWrite(pane, request)) break;
                            if (pane.Document.State.document_revision != r.document_revision) throw new InvalidOperationException("The file was saved, but newer edits remain open.");
                            await ClosePane(pane, true); break;
                        case VIEM_EX_FRONTEND_WRITE_ALL: foreach (var doc in App.Instance.Windows.SelectMany(w => w.Panes).DistinctBy(p => p.Document).Where(p => p.Document.IsDirty).ToArray()) await Save(doc, false, null, force, native: false); break;
                        case VIEM_EX_FRONTEND_WINDOW:
                            if (r.window_command == VIEM_WINDOW_RESIZE_INDEXED) ResizeWindow(pane, r.argument_count, (r.flags & VIEM_EX_FRONTEND_VERTICAL) != 0, r.argument_command, (r.flags & VIEM_EX_FRONTEND_HAS_COUNT) != 0 ? r.window_count : null);
                            else await WindowCommand(pane, r.window_command, (r.flags & VIEM_EX_FRONTEND_HAS_COUNT) != 0 ? r.window_count : 0);
                            break;
                        case VIEM_EX_FRONTEND_ONLY: await WindowCommand(pane, VIEM_WINDOW_CLOSE_OTHERS, 1); break;
                        case VIEM_EX_FRONTEND_PWD: pane.SetMessage(Environment.CurrentDirectory); break;
                        case VIEM_EX_FRONTEND_CD:
                            Environment.CurrentDirectory = ResolvePath(request.Text);
                            foreach (var openPane in App.Instance.Windows.SelectMany(w => w.Panes)) openPane.RefreshStatusFilePath();
                            pane.SetMessage(Environment.CurrentDirectory); break;
                        case VIEM_EX_FRONTEND_FILE: if (request.Text.Length > 0) await RenameDocument(pane, ResolvePath(request.Text)); UpdateTitle(); break;
                        case VIEM_EX_FRONTEND_MESSAGE: pane.SetMessage(request.Text); break;
                        case VIEM_EX_FRONTEND_NORMAL: pane.View.Command(request.Text); break;
                        case VIEM_EX_FRONTEND_ARGUMENT: await NavigateArgument(pane, request); break;
                        case VIEM_EX_FRONTEND_READ:
                            byte[] read = await File.ReadAllBytesAsync(ResolvePath(request.Text));
                            pane.View.ReadFile(read, r.document_id, r.document_revision, r.hard_line_start); break;
                        case VIEM_EX_FRONTEND_SOURCE:
                            if (sourceDepth >= 16) throw new InvalidOperationException("Maximum :source nesting depth reached.");
                            string scriptPath = ResolvePath(request.Text);
                            if (new FileInfo(scriptPath).Length > 1_048_576) throw new InvalidOperationException(":source files are limited to 1 MiB.");
                            string source = await File.ReadAllTextAsync(scriptPath, new System.Text.UTF8Encoding(false, true));
                            var commands = source.Split('\n'); if (commands.Length > 10_000) throw new InvalidOperationException(":source is limited to 10,000 lines.");
                            foreach (string line in commands)
                            {
                                var effectsFromLine = pane.View.SourceLine(line.TrimEnd('\r'), sourceDepth + 1);
                                if (effectsFromLine != null) {
                                    GlobalSelectionOptions.Observe(pane.Document, effectsFromLine.SelectionOptions);
                                    await ExecuteEffects(pane, effectsFromLine, sourceDepth + 1);
                                }
                                if (pane.View == null) break;
                            }
                            break;
                        case VIEM_EX_FRONTEND_CHECKTIME: await CheckExternalChanges(); break;
                    }
                }
            }
            catch (OperationCanceledException) { if (sourceDepth > 0) throw; }
            catch (Exception e) { pane.Report(e); }
    }
    private async Task Reload(EditorPane pane, bool force = false)
    {
        var old = pane.Document;
        string? path = old.FilePath; if (path == null) return;
        var expected = old.State;
        if (!force && old.IsDirty && await Dialog("Revert to saved file?", "Unsaved changes will be discarded.", "Revert", "Cancel") != ContentDialogResult.Primary) return;
        byte[] bytes = await File.ReadAllBytesAsync(path);
        if (old.Handle == 0 || old.FilePath != path || old.State.document_revision != expected.document_revision) return;
        var replacement = NewDocument(bytes, path, expected.format);
        var prepared = new List<(EditorWindow Window, EditorPane Pane, CoreView? View)>();
        try
        {
            if (old.IsReadOnly) replacement.SetReadOnly(true);
            preferences.ObserveMarkdownView(replacement, error => App.Instance.Windows.SelectMany(w => w.Panes)
                .FirstOrDefault(p => p.Document == replacement)?.Report(error));
            foreach (var w in App.Instance.Windows.ToArray())
                foreach (var p in w.Panes.Where(p => p.Document == old))
                    prepared.Add((w, p, p.PrepareReplacement(replacement)));
        }
        catch
        {
            foreach (var entry in prepared) entry.View?.Dispose();
            savedSources.Remove(replacement); replacement.Dispose(); throw;
        }
        // All new views are ready before publishing; keep the panes themselves
        // so focus, split sizes, and argument-list positions remain unchanged.
        foreach (var entry in prepared) entry.Pane.InstallReplacement(replacement, entry.View);
        foreach (var w in prepared.Select(entry => entry.Window).Distinct())
        {
            w.savedSources.Remove(old); w.savedSources[replacement] = SHA256.HashData(bytes); w.UpdateTitle();
        }
        App.Instance.BlockingEdits.Replace(old, replacement);
        old.Dispose();
    }
    private EditorWindow NewWindow(CoreDocument? document)
    {
        var w = new EditorWindow(preferences, document); App.Instance.Windows.Add(w); w.Closed += (_, _) => App.Instance.Windows.Remove(w); w.Activate(); return w;
    }
    private void ResizeWindow(EditorPane current, ulong index, bool width, uint change, ulong? size)
    {
        if (index > (ulong)Panes.Count || change > 2) throw new InvalidOperationException("Invalid window number.");
        var pane = index == 0 ? current : Panes[(int)index-1];
        double unit = width ? pane.View?.DefaultColumnWidth ?? 8 : pane.View?.DefaultLineHeight ?? 16;
        paneGrid.Action(pane, 5, width ? 1u : 0u, (size ?? 0) * unit * (change == 2 ? -1 : 1), size == null ? 2u : change == 0 ? 0u : 1u);
    }
    private async Task WindowCommand(EditorPane pane, uint command, ulong count)
    {
        int index = Panes.IndexOf(pane), n = (int)Math.Clamp(count, 1, (ulong)Math.Max(Panes.Count, 1));
        EditorPane target = pane;
        switch (command)
        {
            case VIEM_WINDOW_FOCUS_DOWN: target = paneGrid.Action(pane, 1, 0, count); break;
            case VIEM_WINDOW_FOCUS_UP: target = paneGrid.Action(pane, 1, 1, count); break;
            case VIEM_WINDOW_FOCUS_LEFT: target = paneGrid.Action(pane, 1, 2, count); break;
            case VIEM_WINDOW_FOCUS_RIGHT: target = paneGrid.Action(pane, 1, 3, count); break;
            case VIEM_WINDOW_FOCUS_NEXT: target = Panes[count == 0 ? (index + 1) % Panes.Count : n - 1]; break;
            case VIEM_WINDOW_FOCUS_PREVIOUS: target = Panes[count == 0 ? (index - 1 + Panes.Count) % Panes.Count : n - 1]; break;
            case VIEM_WINDOW_FOCUS_TOP: target = Panes[0]; break;
            case VIEM_WINDOW_FOCUS_BOTTOM: target = Panes[^1]; break;
            case VIEM_WINDOW_FOCUS_LAST_ACCESSED: target = lastPane != null && Panes.Contains(lastPane) ? lastPane : pane; break;
            case VIEM_WINDOW_ROTATE_DOWN: paneGrid.Action(pane, 2, count); SyncPaneOrder(); break;
            case VIEM_WINDOW_ROTATE_UP: paneGrid.Action(pane, 2, count, flags: 1); SyncPaneOrder(); break;
            case VIEM_WINDOW_MOVE_TO_TOP: paneGrid.Action(pane, 4, 1); SyncPaneOrder(); break;
            case VIEM_WINDOW_MOVE_TO_BOTTOM: paneGrid.Action(pane, 4, 0); SyncPaneOrder(); break;
            case VIEM_WINDOW_MOVE_TO_LEFT: paneGrid.Action(pane, 4, 2); SyncPaneOrder(); break;
            case VIEM_WINDOW_MOVE_TO_RIGHT: paneGrid.Action(pane, 4, 3); SyncPaneOrder(); break;
            case VIEM_WINDOW_EXCHANGE: paneGrid.Action(pane, 3, count); SyncPaneOrder(); break;
            case VIEM_WINDOW_CLOSE_OTHERS: foreach (var p in Panes.Where(p => p != pane).ToArray()) await ClosePane(p); break;
            case VIEM_WINDOW_EQUALIZE_HEIGHTS: paneGrid.Equalize(); break;
            case VIEM_WINDOW_EQUALIZE_HEIGHT_ONLY: paneGrid.Equalize(1); break;
            case VIEM_WINDOW_EQUALIZE_WIDTH_ONLY: paneGrid.Equalize(2); break;
            case VIEM_WINDOW_GROW_WIDTH: case VIEM_WINDOW_SHRINK_WIDTH: case VIEM_WINDOW_SET_WIDTH:
                double column = pane.View?.DefaultColumnWidth ?? 8;
                paneGrid.Action(pane, 5, 1, (command == VIEM_WINDOW_SHRINK_WIDTH ? -1d : 1d) * Math.Max(1, count) * column,
                    command == VIEM_WINDOW_SET_WIDTH ? (count == 0 ? 2u : 0u) : 1u); break;
            case VIEM_WINDOW_GROW: case VIEM_WINDOW_SHRINK: case VIEM_WINDOW_SET_HEIGHT:
                paneGrid.UpdateLayout();
                double line = pane.View?.DefaultLineHeight ?? 16;
                double currentText = Math.Max(0, pane.ActualHeight - pane.StatusBarHeight);
                double height = command == VIEM_WINDOW_SET_HEIGHT ? (count == 0 ? double.MaxValue : count * line)
                    : currentText + (command == VIEM_WINDOW_GROW ? 1d : -1d) * Math.Max(1, count) * line;
                paneGrid.ResizePane(pane, height); break;
        }
        if (Panes.Contains(target)) { SetActive(target); target.FocusEditor(); }
    }
}

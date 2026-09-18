using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Windowing;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Windows.Storage.Pickers;
using Windows.Graphics;
using System.Security.Cryptography;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow : Window
{
    private readonly Preferences preferences;
    private readonly Grid root = new();
    private readonly Grid titleBar = new() { Height = 32 };
    private readonly Grid titleDrag = new() { Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
    private readonly TextBlock titleText = new() { FontSize = 12, VerticalAlignment = VerticalAlignment.Center, Margin = new(12, 0, 0, 0), TextTrimming = TextTrimming.CharacterEllipsis };
    private readonly ToggleButton menuToggle = new() { Width = 40, Height = 30, MinHeight = 0, Padding = new(0), BorderThickness = new(0), Content = new FontIcon { Glyph = "\uE700", FontSize = 14 } };
    internal MenuBar Menu { get; } = new();
    private readonly Grid paneGrid = new();
    internal List<EditorPane> Panes { get; } = [];
    internal EditorPane? ActivePane { get; private set; }
    internal bool IsWindowActive { get; private set; } = true;
    private readonly DispatcherTimer poll = new() { Interval = TimeSpan.FromMilliseconds(40) };
    private bool closing, closed, updatingPreferences;
    private EditorPane? lastPane;
    private readonly Dictionary<CoreDocument, byte[]> savedSources = [];
    private static readonly Dictionary<CoreDocument, DocumentRecovery> recoveries = [];
    private Task effectQueue = Task.CompletedTask;
    private int pollTicks;
    internal nint Hwnd => WinRT.Interop.WindowNative.GetWindowHandle(this);

    public EditorWindow(Preferences preferences, CoreDocument? document = null)
    {
        this.preferences = preferences;
        if (document != null)
        {
            var owner = App.Instance.Windows.FirstOrDefault(w => w.savedSources.ContainsKey(document));
            if (owner != null) savedSources[document] = owner.savedSources[document];
        }
        Title = "Viem"; Content = root;
        root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new() { Height = new(1, GridUnitType.Star) });
        root.Children.Add(titleBar); root.Children.Add(Menu); Grid.SetRow(Menu, 1); root.Children.Add(paneGrid); Grid.SetRow(paneGrid, 2);
        titleBar.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); titleBar.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); titleBar.ColumnDefinitions.Add(new() { Width = new(138) });
        titleBar.Children.Add(titleDrag); titleDrag.Children.Add(titleText); titleBar.Children.Add(menuToggle); Grid.SetColumn(menuToggle, 1);
        ExtendsContentIntoTitleBar = true; SetTitleBar(titleDrag);
        AppWindow.Resize(new SizeInt32(1100, 780));
        WindowSizing.Appearance(this, preferences.Midnight);
        AppWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Standard;
        titleBar.SizeChanged += (_, _) => UpdateCaptionInset();
        AutomationProperties.SetName(menuToggle, "Show menu bar"); ToolTipService.SetToolTip(menuToggle, "Show or hide the menu bar");
        menuToggle.Click += (_, _) => Safe(() => { preferences.Set("windows", "showMenu", menuToggle.IsChecked == true); return Task.CompletedTask; });
        Activated += (_, e) => { IsWindowActive = e.WindowActivationState != WindowActivationState.Deactivated; foreach (var pane in Panes) pane.Canvas.Invalidate(); };
        AppWindow.Closing += (_, e) => { if (!closing) { e.Cancel = true; Safe(RequestClose); } };
        Closed += (_, _) => {
            closed = true; poll.Stop(); preferences.Changed -= ApplyPreferences;
            var documents = Panes.Select(p => p.Document).Distinct().ToArray();
            foreach (var pane in Panes) pane.Dispose(); Panes.Clear();
            foreach (var doc in documents) if (!App.Instance.Windows.Where(w => w != this).Any(w => w.Panes.Any(p => p.Document == doc))) doc.Dispose();
        };
        preferences.Changed += ApplyPreferences;
        BuildMenus(); ApplyPreferences();
        AddPane(document ?? NewDocument());
        poll.Tick += (_, _) => {
            foreach (var doc in Panes.Select(p => p.Document).Distinct().ToArray()) ActivePane?.Run(() => doc.PollSyntax());
            foreach (var pane in Panes.ToArray()) pane.Poll();
            if (++pollTicks % 50 == 0) Safe(CheckExternalChanges);
        };
        poll.Start();
    }
    private void UpdateCaptionInset()
    {
        double scale = root.XamlRoot?.RasterizationScale ?? 1;
        double inset = AppWindow.TitleBar.RightInset / scale;
        titleBar.ColumnDefinitions[2].Width = new(Math.Max(inset, 138 / scale));
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
        titleBar.Background = new SolidColorBrush(preferences.Midnight ? Theme.Rgb(31, 31, 31) : Theme.Rgb(243, 243, 243));
        updatingPreferences = false;
        RefreshRecentMenu();
    }
    private CoreDocument NewDocument(byte[]? source = null, string? path = null, uint? format = null, uint encoding = 0, uint fileFormat = 0)
    {
        var doc = new CoreDocument(source ?? [], path, format, encoding, fileFormat);
        try
        {
            doc.ConfigureDefaults(preferences.Indentation, preferences.Whitespace, preferences.TextWidth, preferences.VimDirectory, preferences.Associations);
            string defaults = Path.Combine(preferences.DirectoryPath, CoreDocument.FormatName(doc.State.format).Replace(" Source", "").ToLowerInvariant() + "_style.json");
            if (File.Exists(defaults) && doc.State.format != VIEM_FORMAT_CODE) doc.InitializeStyleDefaults(File.ReadAllBytes(defaults));
            if (preferences.StartupCommands.Length > 0) doc.InitializeStartup(preferences.StartupCommands);
            if (path != null) { savedSources[doc] = SHA256.HashData(source ?? []); if (File.Exists(path) && (File.GetAttributes(path) & FileAttributes.ReadOnly) != 0) doc.SetReadOnly(true); }
            if (path != null) recoveries[doc] = DocumentRecovery.Claim(doc, path, preferences.DirectoryPath, DispatcherQueue, e => ActivePane?.Report(e));
            doc.Disposed += () => recoveries.Remove(doc);
            return doc;
        }
        catch { doc.Dispose(); throw; }
    }
    internal void PaneReady(EditorPane pane)
    {
        if (pane == ActivePane) pane.FocusEditor();
        UpdateTitle(); RefreshStyleMenus();
        if (pane.Document.StartupDiagnostics.Length > 0) pane.SetMessage(Path.Combine(preferences.DirectoryPath, pane.Document.StartupDiagnostics));
        OnPaneReady(pane);
    }
    partial void OnPaneReady(EditorPane pane);
    internal EditorPane AddPane(CoreDocument doc, int? position = null)
    {
        var pane = new EditorPane(this, doc, preferences); pane.Focused += SetActive;
        pane.RememberedArgument = ActivePane?.RememberedArgument ?? ulong.MaxValue;
        Panes.Insert(position ?? Panes.Count, pane); RebuildPanes(); SetActive(pane); return pane;
    }
    private void SetActive(EditorPane pane)
    {
        // WinUI can deliver a queued focus event after a pane was detached.
        if (closed || !Panes.Contains(pane) || pane.Document.Handle == 0) return;
        if (ActivePane != pane) { lastPane = ActivePane; ActivePane = pane; }
        foreach (var item in Panes) item.IsActive = item == pane;
        UpdateTitle(); RefreshStyleMenus();
    }
    private void RebuildPanes()
    {
        paneGrid.Children.Clear(); paneGrid.RowDefinitions.Clear();
        for (int i = 0; i < Panes.Count; i++)
        {
            if (i > 0)
            {
                int upper = paneGrid.RowDefinitions.Count - 1;
                paneGrid.RowDefinitions.Add(new() { Height = new(5) });
                var grip = new Thumb { Height = 5, Background = new SolidColorBrush(preferences.Theme.StatusBackground) };
                AutomationProperties.SetName(grip, "Resize document panes"); Grid.SetRow(grip, paneGrid.RowDefinitions.Count - 1); paneGrid.Children.Add(grip);
                grip.DragStarted += (_, _) => { foreach (var row in paneGrid.RowDefinitions.Where(r => r.Height.IsStar)) row.Height = new(row.ActualHeight, GridUnitType.Star); };
                grip.DragDelta += (_, e) => {
                    var above = paneGrid.RowDefinitions[upper]; var below = paneGrid.RowDefinitions[upper + 2];
                    double change = Math.Clamp(e.VerticalChange, 80 - above.ActualHeight, below.ActualHeight - 80);
                    above.Height = new(Math.Max(80, above.ActualHeight + change), GridUnitType.Star); below.Height = new(Math.Max(80, below.ActualHeight - change), GridUnitType.Star);
                };
            }
            paneGrid.RowDefinitions.Add(new() { Height = new(1, GridUnitType.Star), MinHeight = 55 });
            Grid.SetRow(Panes[i], paneGrid.RowDefinitions.Count - 1); paneGrid.Children.Add(Panes[i]);
        }
    }
    internal void UpdateTitle()
    {
        if (ActivePane == null || closed) return;
        string value = ActivePane.Document.Name + (ActivePane.Document.IsDirty ? " •" : "") + (Panes.Count > 1 ? $" · {Panes.Count} panes" : "");
        if (titleText.Text != value) { Title = value + " — Viem"; titleText.Text = value; }
        menusDirty = true;
    }
    private void Safe(Func<Task> action) { async void Execute() { try { await action(); } catch (Exception e) { ActivePane?.Report(e); } } Execute(); }
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
    internal async Task OpenPath(string path, bool split = false, bool force = false)
    {
        path = ResolvePath(path);
        var existing = App.Instance.Windows.SelectMany(w => w.Panes.Select(p => (Window: w, Pane: p))).FirstOrDefault(x => x.Pane.Document.FilePath is string named && FileIdentity.Same(named, path));
        if (existing.Pane != null && !split) { existing.Window.Activate(); existing.Pane.FocusEditor(); preferences.Remember(path); return; }
        var old = ActivePane;
        if (!split && old != null && !force && old.Document.IsDirty && App.Instance.Windows.SelectMany(w => w.Panes).Count(p => p.Document == old.Document) == 1) throw new InvalidOperationException("E37: No write since last change (add ! to override).");
        byte[] bytes = File.Exists(path) ? await File.ReadAllBytesAsync(path) : [];
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
                readOnly = choices.SelectedIndex == 0; if (choices.SelectedIndex == 2) recovered = snapshot;
            }
        }
        var doc = existing.Pane?.Document ?? NewDocument(recovered?.source ?? bytes, path, recovered?.Format, recovered?.encoding ?? 0, recovered?.fileFormat ?? 0);
        if (recovered != null) { savedSources[doc] = SHA256.HashData(bytes); doc.MarkRecovered(); recoveries[doc].Write(RecoverySnapshot.Capture(doc)); }
        if (readOnly) doc.SetReadOnly(true);
        int index = old == null ? Panes.Count : Panes.IndexOf(old) + (split ? 1 : 0);
        if (!split && old != null) RemovePane(old, false);
        AddPane(doc, Math.Min(index, Panes.Count)); preferences.Remember(path);
    }
    internal async Task<bool> Save(EditorPane pane, bool saveAs = false, string? explicitPath = null, bool force = false, bool adoptPath = true, bool native = true)
    {
        var doc = pane.Document;
        string? path = explicitPath ?? (saveAs ? null : doc.FilePath);
        if (path == null)
        {
            var picker = new FileSavePicker { SuggestedFileName = doc.Name == "Untitled" ? "Untitled" : Path.GetFileNameWithoutExtension(doc.Name) };
            string extension = doc.State.format switch { 2 or 5 => ".md", 3 or 6 => ".html", 4 => ".rtf", _ => ".txt" };
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
    private async Task<bool> ConfirmDiscard(EditorPane pane)
    {
        if (!pane.Document.IsDirty || App.Instance.Windows.SelectMany(w => w.Panes).Count(p => p.Document == pane.Document) > 1) return true;
        var result = await Dialog("Save changes?", $"Save changes to {pane.Document.Name} before closing?", "Save", "Cancel", "Discard");
        if (result == ContentDialogResult.Secondary) return true;
        if (result != ContentDialogResult.Primary) return false;
        await Save(pane); return !pane.Document.IsDirty;
    }
    private async Task RequestClose()
    {
        foreach (var doc in Panes.Select(p => p.Document).Distinct().ToArray())
        {
            var pane = Panes.First(p => p.Document == doc);
            if (!doc.IsDirty || App.Instance.Windows.Where(w => w != this).Any(w => w.Panes.Any(p => p.Document == doc))) continue;
            var result = await Dialog("Save changes?", $"Save changes to {doc.Name} before closing?", "Save", "Cancel", "Discard");
            if (result == ContentDialogResult.None) return;
            if (result == ContentDialogResult.Primary) { await Save(pane); if (doc.IsDirty) return; }
        }
        closing = true; Close();
    }
    private async Task ClosePane(EditorPane pane, bool force = false)
    {
        if (!force && !await ConfirmDiscard(pane)) return;
        if (Panes.Count == 1) { closing = true; Close(); return; }
        RemovePane(pane); ActivePane?.FocusEditor();
    }
    private void RemovePane(EditorPane pane, bool rebuild = true)
    {
        pane.Focused -= SetActive;
        pane.Dispose(); paneGrid.Children.Remove(pane); Panes.Remove(pane);
        if (!App.Instance.Windows.SelectMany(w => w.Panes).Any(p => p.Document == pane.Document)) { savedSources.Remove(pane.Document); pane.Document.Dispose(); }
        if (ActivePane == pane) ActivePane = Panes.FirstOrDefault();
        if (rebuild) { RebuildPanes(); if (ActivePane != null) SetActive(ActivePane); }
    }
    internal async Task<ContentDialogResult> Dialog(string title, string text, string primary = "OK", string close = "", string secondary = "")
    {
        var dialog = new ContentDialog { XamlRoot = root.XamlRoot, RequestedTheme = root.RequestedTheme, Title = title, Content = new ScrollViewer { Content = new TextBlock { Text = text, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true }, MaxHeight = 450 }, PrimaryButtonText = primary, CloseButtonText = close, SecondaryButtonText = secondary, DefaultButton = ContentDialogButton.Primary };
        return await dialog.ShowAsync();
    }
    internal Task ApplyEffects(EditorPane pane, HostEffects effects)
    {
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
                    if (pane.View == null) break;
                    switch (r.kind)
                    {
                        case VIEM_EX_FRONTEND_SPLIT: if (request.Text.Length > 0) await OpenPath(request.Text, true, force); else AddPane(pane.Document, Panes.IndexOf(pane) + 1); break;
                        case VIEM_EX_FRONTEND_NEW_PANE: AddPane(NewDocument(), Panes.IndexOf(pane) + 1); break;
                        case VIEM_EX_FRONTEND_NEW: if (force || await ConfirmDiscard(pane)) { int i = Panes.IndexOf(pane); RemovePane(pane, false); AddPane(NewDocument(), i); } break;
                        case VIEM_EX_FRONTEND_EDIT: if (request.Text.Length > 0 && !string.Equals(Path.GetFullPath(request.Text), pane.Document.FilePath, StringComparison.OrdinalIgnoreCase)) await OpenPath(request.Text, false, force); else if (pane.Document.FilePath != null) await Reload(pane, force); break;
                        case VIEM_EX_FRONTEND_EDIT_NEW_WINDOW: NewWindow(null); break;
                        case VIEM_EX_FRONTEND_WRITE: case VIEM_EX_FRONTEND_SAVE_AS: await ExWrite(pane, request); break;
                        case VIEM_EX_FRONTEND_QUIT: await ClosePane(pane, force); break;
                        case VIEM_EX_FRONTEND_QUIT_ALL: foreach (var w in App.Instance.Windows.ToArray()) { if (force) { w.closing = true; w.Close(); } else await w.RequestClose(); } break;
                        case VIEM_EX_FRONTEND_WRITE_QUIT: case VIEM_EX_FRONTEND_XIT:
                            if (r.kind == VIEM_EX_FRONTEND_WRITE_QUIT || pane.Document.IsDirty || request.Text.Length > 0)
                                if (!await ExWrite(pane, request)) break;
                            if (pane.Document.State.document_revision != r.document_revision) throw new InvalidOperationException("The file was saved, but newer edits remain open.");
                            await ClosePane(pane, true); break;
                        case VIEM_EX_FRONTEND_WRITE_ALL: foreach (var doc in App.Instance.Windows.SelectMany(w => w.Panes).DistinctBy(p => p.Document).Where(p => p.Document.IsDirty).ToArray()) await Save(doc, false, null, force, native: false); break;
                        case VIEM_EX_FRONTEND_WINDOW: await WindowCommand(pane, r.window_command, r.window_count); break;
                        case VIEM_EX_FRONTEND_ONLY: await WindowCommand(pane, VIEM_WINDOW_CLOSE_OTHERS, 1); break;
                        case VIEM_EX_FRONTEND_PWD: pane.SetMessage(Environment.CurrentDirectory); break;
                        case VIEM_EX_FRONTEND_CD: Environment.CurrentDirectory = Path.GetFullPath(request.Text); pane.SetMessage(Environment.CurrentDirectory); break;
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
                                if (effectsFromLine != null) await ExecuteEffects(pane, effectsFromLine, sourceDepth + 1);
                                if (pane.View == null) break;
                            }
                            break;
                        case VIEM_EX_FRONTEND_CHECKTIME: await CheckExternalChanges(); break;
                    }
                }
            }
            catch (Exception e) { pane.Report(e); }
    }
    private async Task Reload(EditorPane pane, bool force = false)
    {
        string? path = pane.Document.FilePath; if (path == null) return;
        if (!force && pane.Document.IsDirty && await Dialog("Revert to saved file?", "Unsaved changes will be discarded.", "Revert", "Cancel") != ContentDialogResult.Primary) return;
        byte[] bytes = await File.ReadAllBytesAsync(path); var replacement = NewDocument(bytes, path, pane.Document.State.format);
        var old = pane.Document;
        foreach (var w in App.Instance.Windows.ToArray())
        {
            foreach (var p in w.Panes.Where(p => p.Document == old).ToArray()) { int index = w.Panes.IndexOf(p); w.RemovePane(p, false); w.AddPane(replacement, index); }
            w.savedSources[replacement] = SHA256.HashData(bytes);
        }
    }
    private EditorWindow NewWindow(CoreDocument? document)
    {
        var w = new EditorWindow(preferences, document); App.Instance.Windows.Add(w); w.Closed += (_, _) => App.Instance.Windows.Remove(w); w.Activate(); return w;
    }
    private async Task WindowCommand(EditorPane pane, uint command, ulong count)
    {
        int index = Panes.IndexOf(pane), n = (int)Math.Clamp(count, 1, (ulong)Math.Max(Panes.Count, 1));
        EditorPane target = pane;
        switch (command)
        {
            case VIEM_WINDOW_FOCUS_DOWN: target = Panes[Math.Min(Panes.Count - 1, index + n)]; break;
            case VIEM_WINDOW_FOCUS_UP: target = Panes[Math.Max(0, index - n)]; break;
            case VIEM_WINDOW_FOCUS_NEXT: target = Panes[(index + n) % Panes.Count]; break;
            case VIEM_WINDOW_FOCUS_PREVIOUS: target = Panes[(index - n + Panes.Count) % Panes.Count]; break;
            case VIEM_WINDOW_FOCUS_TOP: target = Panes[0]; break;
            case VIEM_WINDOW_FOCUS_BOTTOM: target = Panes[^1]; break;
            case VIEM_WINDOW_FOCUS_LAST_ACCESSED: target = lastPane != null && Panes.Contains(lastPane) ? lastPane : pane; break;
            case VIEM_WINDOW_ROTATE_DOWN: Panes.Insert(0, Panes[^1]); Panes.RemoveAt(Panes.Count - 1); RebuildPanes(); break;
            case VIEM_WINDOW_ROTATE_UP: Panes.Add(Panes[0]); Panes.RemoveAt(0); RebuildPanes(); break;
            case VIEM_WINDOW_MOVE_TO_TOP: Panes.Remove(pane); Panes.Insert(0, pane); RebuildPanes(); break;
            case VIEM_WINDOW_MOVE_TO_BOTTOM: Panes.Remove(pane); Panes.Add(pane); RebuildPanes(); break;
            case VIEM_WINDOW_EXCHANGE: int other = (index + 1) % Panes.Count; (Panes[index], Panes[other]) = (Panes[other], Panes[index]); RebuildPanes(); break;
            case VIEM_WINDOW_CLOSE_OTHERS: foreach (var p in Panes.Where(p => p != pane).ToArray()) await ClosePane(p); break;
            case VIEM_WINDOW_EQUALIZE_HEIGHTS: foreach (var row in paneGrid.RowDefinitions.Where(r => r.Height.IsStar)) row.Height = new(1, GridUnitType.Star); break;
            case VIEM_WINDOW_GROW: case VIEM_WINDOW_SHRINK: case VIEM_WINDOW_SET_HEIGHT:
                foreach (var row in paneGrid.RowDefinitions.Where(r => r.Height.IsStar)) row.Height = new(row.ActualHeight, GridUnitType.Star);
                var current = paneGrid.RowDefinitions[Grid.GetRow(pane)]; current.Height = new(Math.Max(60, command == VIEM_WINDOW_SET_HEIGHT ? count * 22 : current.ActualHeight + (command == VIEM_WINDOW_GROW ? 1 : -1) * Math.Max(1, (long)count) * 22), GridUnitType.Star); break;
        }
        if (Panes.Contains(target)) { SetActive(target); target.FocusEditor(); }
    }
}

using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;

namespace Viem.Windows.Shell;

internal sealed partial class DiffToolbar : EditorToolbar
{
    private readonly ComboBox bases = Selector(new ComboBox { MinWidth = 118, MaxWidth = 220 }, "Diff base");
    private readonly ComboBox files = Selector(new ComboBox { MinWidth = 140 }, "Files in this diff");
    private readonly StackPanel fileCountGroup = Group(3);
    internal readonly TextBlock FileCount = new(), AddedLines = new(), RemovedLines = new();
    private (ulong Added, ulong Removed) gitTotals;
    private (ulong Added, ulong Removed)? displayedTotals;
    internal readonly ToggleButton Align = IconButton(new ToggleButton(), "Align Sides");
    internal readonly ToggleButton Collapse = IconButton(new ToggleButton(), "Collapse Context");
    private readonly Button previousFile, nextFile, previousChange, nextChange;
    private bool updating;
    private readonly List<DiffBase> choices = [];
    private (DiffBase Basis, bool Git, string[] Branches)? displayedBases;
    private DiffFile[]? displayedFiles;
    private string? displayedDirectory;
    private const int FileLimit = (int)Interop.Native.VIEM_DIFF_FILE_PICKER_LIMIT;
    private int lastFileIndex = -1;

    internal event Action<DiffBase>? BaseSelected;
    internal event Action<int>? FileSelected;
    internal event Action<bool>? AlignmentChanged;
    internal event Action<bool>? CollapseChanged;

    internal DiffToolbar(Action choose, Action refresh, Action<int> move, Action<bool> navigate, Action openWindow) : base("Diff toolbar")
    {
        var open = IconButton(new Button(), "Open in New Editor Window");
        open.Content = new PathIcon { Width = 16, Height = 18, Data = (Geometry)Microsoft.UI.Xaml.Markup.XamlBindingHelper.ConvertValue(
            typeof(Geometry), "F0 M0,6 L6,0 H16 V18 H0 Z M1.5,7.5 V16.5 H14.5 V1.5 H7.5 V7.5 Z M2.1,6 H6 V2.1 Z " +
            "M4.5,13 L9.8,7.7 H7.8 V5.5 H13 V10.7 H10.8 V9.3 L6.1,14.6 Z") };
        open.Click += (_, _) => openWindow();
        var viewport = new ScrollViewer { Content = row, HorizontalScrollMode = ScrollMode.Auto,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollBarVisibility = ScrollBarVisibility.Disabled };
        viewport.SizeChanged += (_, e) => files.MaxWidth = Math.Max(files.MinWidth, e.NewSize.Width / 2);
        files.ItemTemplate = (DataTemplate)Resources["DiffFilePath"];
        SetLayout(viewport, open);

        var sources = Group();
        row.Children.Add(sources);
        sources.Children.Add(bases);
        AddButton(sources, "Refresh Base", ((char)Symbol.Refresh).ToString(), refresh);

        sources.Margin = new(0, 0, 2 * row.Spacing, 0);
        fileCountGroup.Children.Add(new PathIcon { Width = 14, Height = 17,
            Data = (Geometry)Microsoft.UI.Xaml.Markup.XamlBindingHelper.ConvertValue(typeof(Geometry),
                "F0 M0,6 L6,0 H14 V17 H0 Z M1.5,7.5 V15.5 H12.5 V1.5 H7.5 V7.5 Z M2.1,6 H6 V2.1 Z") });
        fileCountGroup.Children.Add(FileCount);
        row.Children.Add(fileCountGroup);
        var totals = Group(5);
        AddedLines.Style = (Style)Resources["DiffAddedLines"];
        RemovedLines.Style = (Style)Resources["DiffRemovedLines"];
        totals.Children.Add(AddedLines);
        totals.Children.Add(RemovedLines);
        row.Children.Add(totals);
        var navigation = Group();
        navigation.Margin = new(0, 0, 2 * row.Spacing, 0);
        row.Children.Add(navigation);
        previousFile = AddButton(navigation, "Previous File", "\uE76B", () => move(-1), ":N");
        nextFile = AddButton(navigation, "Next File", "\uE76C", () => move(1), ":n");
        navigation.Children.Add(files);

        // Two panes with a shared middle row convey vertical alignment at toolbar size.
        SetIcon(Align,
            "M1.5,2.5 H7.5 V13.5 H1.5 Z M10.5,2.5 H16.5 V13.5 H10.5 Z " +
            "M3.5,5.5 H5.5 M12.5,5.5 H14.5 M3.5,8 H14.5 M3.5,10.5 H5.5 M12.5,10.5 H14.5");
        row.Children.Add(Align);
        Align.Click += (_, _) => AlignmentChanged?.Invoke(Align.IsChecked == true);

        SetIcon(Collapse, "M5,1.5 L9,5.5 L13,1.5 M2,8 H16 M5,14.5 L9,10.5 L13,14.5");
        row.Children.Add(Collapse);
        Collapse.Click += (_, _) => CollapseChanged?.Invoke(Collapse.IsChecked == true);

        var changes = Group();
        row.Children.Add(changes);
        previousChange = AddButton(changes, "Previous Change", "\uE70E", () => navigate(false));
        nextChange = AddButton(changes, "Next Change", "\uE70D", () => navigate(true));
        bases.SelectionChanged += (_, _) => {
            if (updating || bases.SelectedIndex < 0) return;
            if (bases.SelectedIndex == choices.Count) choose();
            else BaseSelected?.Invoke(choices[bases.SelectedIndex]);
        };
        files.SelectionChanged += (_, _) => {
            int slot = files.SelectedIndex;
            if (!updating && slot >= 0 && slot < FileLimit && slot < displayedFiles?.Length)
                FileSelected?.Invoke(slot == FileLimit - 1 ? lastFileIndex : slot);
        };
    }

    private static void SetIcon(ToggleButton button, string path)
    {
        var geometry = (Geometry)Microsoft.UI.Xaml.Markup.XamlBindingHelper.ConvertValue(typeof(Geometry), path);
        var icon = new Canvas { Width = 18, Height = 16 };
        icon.Children.Add(Outline(geometry));
        button.Content = icon;
    }

    private static Button AddButton(Panel group, string name, string glyph, Action action, string shortcut = "")
    {
        var button = IconButton(new Button(), name, Glyph(glyph), shortcut: shortcut);
        button.Click += (_, _) => action();
        group.Children.Add(button);
        return button;
    }

    /// The file list's entry for a folder without changes.
    internal const string NoChangesTitle = "no changes";

    internal void Update(DiffBase basis, bool git, string[] branches, DiffFile[] entries, int current,
        string? documentPath, bool aligned, bool collapsed, bool comparing, (ulong Added, ulong Removed) liveTotals, bool useLiveTotals, bool noChanges = false)
    {
        updating = true;
        try {
            if (displayedBases != (basis, git, branches)) {
                displayedBases = (basis, git, branches);
                bases.Items.Clear();
                choices.Clear();
                void Add(DiffBase choice) {
                    choices.Add(choice);
                    bases.Items.Add(choice.ToString());
                }
                Add(new(Interop.Native.VIEM_DIFF_BASE_LAST_SAVED));
                if (git) {
                    Add(new(Interop.Native.VIEM_DIFF_BASE_UNSTAGED));
                    Add(new(Interop.Native.VIEM_DIFF_BASE_HEAD));
                    foreach (string branch in branches) Add(new(Interop.Native.VIEM_DIFF_BASE_BRANCH, branch));
                    Add(new(Interop.Native.VIEM_DIFF_BASE_BRANCH));
                }
                if (basis.Kind == Interop.Native.VIEM_DIFF_BASE_FILE) Add(basis);
                if (basis.Kind == Interop.Native.VIEM_DIFF_BASE_COMMIT) Add(basis);
                if (git) Add(new(Interop.Native.VIEM_DIFF_BASE_COMMIT));
                bases.Items.Add("Choose File…");
            }
            bases.SelectedIndex = choices.IndexOf(basis);
            bool changed = displayedFiles != entries;
            if (changed) {
                gitTotals = default;
                foreach (var entry in entries) {
                    gitTotals.Added += entry.Added;
                    gitTotals.Removed += entry.Removed;
                }
                FileCount.Text = entries.Length.ToString();
                AutomationProperties.SetName(FileCount, $"{entries.Length} changed files");
            }
            int visible = Math.Min(FileLimit, entries.Length);
            int last = Math.Max(visible - 1, current);
            string directory = Environment.CurrentDirectory;
            if (changed || last != lastFileIndex || directory != displayedDirectory) {
                displayedFiles = entries;
                displayedDirectory = directory;
                lastFileIndex = last;
                files.Items.Clear();
                var shown = entries[..visible];
                if (visible > 0) shown[^1] = entries[last];
                foreach (var file in shown) files.Items.Add(file.TitleRelativeTo(directory));
                if (entries.Length > visible) files.Items.Add(new ComboBoxItem {
                    Content = $"{entries.Length - visible} files clipped", IsEnabled = false
                });
            }
            string name = documentPath == null ? "Untitled" : Path.GetRelativePath(directory, documentPath);
            files.PlaceholderText = noChanges ? NoChangesTitle : name + (entries.Length > 0 ? " (not in this diff)" : "");
            files.FontStyle = noChanges && current < 0 ? global::Windows.UI.Text.FontStyle.Italic : global::Windows.UI.Text.FontStyle.Normal;
            fileCountGroup.Visibility = entries.Length > 1 || noChanges ? Visibility.Visible : Visibility.Collapsed;
            files.SelectedIndex = Math.Min(current, FileLimit - 1);
            ToolTipService.SetToolTip(files, files.SelectedItem as string ?? files.PlaceholderText);
            files.IsEnabled = previousFile.IsEnabled = nextFile.IsEnabled = entries.Length != 0;
            previousChange.IsEnabled = nextChange.IsEnabled = comparing;
            Align.IsChecked = aligned;
            Collapse.IsChecked = collapsed;
            ShowLineTotals(useLiveTotals ? liveTotals : gitTotals);
        } finally { updating = false; }
    }

    internal void ShowLineTotals((ulong Added, ulong Removed) totals)
    {
        if (displayedTotals == totals) return;
        displayedTotals = totals;
        AddedLines.Text = $"+{totals.Added}";
        RemovedLines.Text = $"–{totals.Removed}";
        AutomationProperties.SetName(AddedLines, $"{totals.Added} added lines");
        AutomationProperties.SetName(RemovedLines, $"{totals.Removed} removed lines");
    }

    internal void OpenBranches() => bases.IsDropDownOpen = true;
}

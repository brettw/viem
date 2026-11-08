using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;

namespace Viem.Windows.Shell;

internal sealed partial class DiffToolbar : EditorToolbar
{
    private readonly ComboBox bases = Selector(new ComboBox { MinWidth = 118, MaxWidth = 220 }, "Diff base");
    private readonly ComboBox files = Selector(new ComboBox { MinWidth = 140, MaxWidth = 360 }, "Files in this diff");
    internal readonly ToggleButton Align = IconButton(new ToggleButton(), "Align Sides");
    private readonly Button previousFile, nextFile, previousChange, nextChange;
    private bool updating;
    private readonly List<DiffBase> choices = [];

    internal event Action<DiffBase>? BaseSelected;
    internal event Action<int>? FileSelected;
    internal event Action<bool>? AlignmentChanged;

    internal DiffToolbar(Action choose, Action refresh, Action<int> move, Action<bool> navigate) : base("Diff toolbar")
    {
        SetLayout(new ScrollViewer { Content = row, HorizontalScrollMode = ScrollMode.Auto,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollBarVisibility = ScrollBarVisibility.Disabled });

        var sources = Group();
        row.Children.Add(sources);
        sources.Children.Add(bases);
        AddButton(sources, "Choose Base File…", ((char)Symbol.OpenFile).ToString(), choose);
        AddButton(sources, "Refresh Base", ((char)Symbol.Refresh).ToString(), refresh);

        var navigation = Group();
        row.Children.Add(navigation);
        previousFile = AddButton(navigation, "Previous File", "\uE76B", () => move(-1));
        navigation.Children.Add(files);
        nextFile = AddButton(navigation, "Next File", "\uE76C", () => move(1));

        // Two panes with a shared middle row convey vertical alignment at toolbar size.
        var alignedRows = (Geometry)Microsoft.UI.Xaml.Markup.XamlBindingHelper.ConvertValue(typeof(Geometry),
            "M1.5,2.5 H7.5 V13.5 H1.5 Z M10.5,2.5 H16.5 V13.5 H10.5 Z " +
            "M3.5,5.5 H5.5 M12.5,5.5 H14.5 M3.5,8 H14.5 M3.5,10.5 H5.5 M12.5,10.5 H14.5");
        var icon = new Canvas { Width = 18, Height = 16 };
        icon.Children.Add(Outline(Align, alignedRows));
        Align.Content = icon;
        row.Children.Add(Align);
        Align.Click += (_, _) => AlignmentChanged?.Invoke(Align.IsChecked == true);

        var changes = Group();
        row.Children.Add(changes);
        previousChange = AddButton(changes, "Previous Change", "\uE70E", () => navigate(false));
        nextChange = AddButton(changes, "Next Change", "\uE70D", () => navigate(true));
        bases.SelectionChanged += (_, _) => {
            if (!updating && (uint)bases.SelectedIndex < (uint)choices.Count) BaseSelected?.Invoke(choices[bases.SelectedIndex]);
        };
        files.SelectionChanged += (_, _) => {
            if (!updating && files.SelectedIndex >= 0) FileSelected?.Invoke(files.SelectedIndex);
        };
    }

    private static Button AddButton(Panel group, string name, string glyph, Action action)
    {
        var button = IconButton(new Button(), name, Glyph(glyph));
        button.Click += (_, _) => action();
        group.Children.Add(button);
        return button;
    }

    internal void Update(DiffBase basis, bool git, string[] branches, DiffFile[] entries, int current,
        string documentName, bool aligned, bool comparing)
    {
        updating = true;
        try {
            bases.Items.Clear();
            choices.Clear();
            void Add(DiffBase choice) {
                choices.Add(choice);
                bases.Items.Add(choice.ToString());
                if (choice == basis) bases.SelectedIndex = choices.Count - 1;
            }
            Add(new(Interop.Native.VIEM_DIFF_BASE_LAST_SAVED));
            if (git) {
                Add(new(Interop.Native.VIEM_DIFF_BASE_UNSTAGED));
                Add(new(Interop.Native.VIEM_DIFF_BASE_HEAD));
                foreach (string branch in branches) Add(new(Interop.Native.VIEM_DIFF_BASE_BRANCH, branch));
                Add(new(Interop.Native.VIEM_DIFF_BASE_BRANCH));
            }
            if (basis.Kind == Interop.Native.VIEM_DIFF_BASE_FILE) Add(basis);
            files.Items.Clear();
            var duplicates = entries.GroupBy(file => Path.GetFileName(file.Path), StringComparer.OrdinalIgnoreCase)
                .Where(group => group.Count() > 1).Select(group => group.Key).ToHashSet(StringComparer.OrdinalIgnoreCase);
            string? shared = entries.Length == 0 ? null : Path.GetDirectoryName(entries[0].Path);
            foreach (var entry in entries) {
                while (shared != null && Path.GetRelativePath(shared, entry.Path).StartsWith(".." + Path.DirectorySeparatorChar, StringComparison.Ordinal))
                    shared = Path.GetDirectoryName(shared);
            }
            foreach (var file in entries) {
                string title = file.Title;
                if (duplicates.Contains(Path.GetFileName(file.Path)) && shared != null)
                    title += " — " + Path.GetRelativePath(shared, Path.GetDirectoryName(file.Path)!);
                files.Items.Add(title);
            }
            if (current < 0) files.Items.Add(documentName + (entries.Length > 0 ? " (not in this diff)" : ""));
            files.SelectedIndex = current < 0 ? entries.Length : current;
            files.IsEnabled = previousFile.IsEnabled = nextFile.IsEnabled = entries.Length != 0;
            previousChange.IsEnabled = nextChange.IsEnabled = comparing;
            Align.IsChecked = aligned;
        } finally { updating = false; }
    }

    internal void OpenBranches() => bases.IsDropDownOpen = true;
}

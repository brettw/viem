using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Text;
using Microsoft.Graphics.Canvas.UI.Xaml;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Windowing;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Viem.Windows.Rendering;
using Windows.Graphics;
using Windows.UI.Text;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

/// <summary>A modeless native inspector over the core-owned stylesheet.</summary>
internal sealed partial class StyleWindow : Window
{
    private CoreView view = null!;
    private readonly Preferences preferences;
    private StyleSheet sheet = null!;
    private StyleDefinition selected = null!;
    private bool loading, updating, closed;
    private const int ClientWidth = 680;
    private readonly StackPanel root = new() { Padding = new(24, 16, 24, 14), Spacing = 6, Width = ClientWidth, VerticalAlignment = VerticalAlignment.Top };
    private readonly ComboBox stylePicker = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly ComboBox documentPicker = new() { Width = 140, VerticalAlignment = VerticalAlignment.Center };
    private readonly uint[] documentFormats = [VIEM_FORMAT_PLAIN_TEXT, VIEM_FORMAT_MARKDOWN, VIEM_FORMAT_CODE];
    private readonly Border selectorBox = new() { Padding = new(14, 12, 14, 12), BorderThickness = new(1), CornerRadius = new(6), Margin = new(0, 0, 0, 4) };
    private readonly TextBlock styleType = new() { VerticalAlignment = VerticalAlignment.Center, Opacity = 0.7 };
    private readonly ContentControl styleTypeIcon = new() { Width = 16, Height = 16, VerticalAlignment = VerticalAlignment.Center };
    private readonly ComboBox parent = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly ComboBox next = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly Button visitParent = NavigationButton();
    private readonly Button visitNext = NavigationButton();
    private readonly StackPanel character = new() { Spacing = 12 };
    private readonly StackPanel paragraph = new() { Spacing = 8 };
    private readonly StackPanel block = new() { Spacing = 8 };
    private readonly CanvasControl preview = new() { Height = 200 };
    private readonly Dictionary<uint, BlockStylePreview> blockPreviews = [];
    private readonly TextBlock error = new() { TextWrapping = TextWrapping.Wrap, FontSize = 12, Visibility = Visibility.Collapsed };
    private readonly List<Action> refreshFields = [];

    public StyleWindow(CoreView initialView, Preferences preferences, bool followCaret = true)
    {
        documentView = initialView; this.preferences = preferences;
        CreateThemeSession(initialView.Document.State.format);
        Title = ThemeStyleTitle;
        var scroll = new ScrollViewer { Content = root, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, RequestedTheme = preferences.Midnight ? ElementTheme.Dark : ElementTheme.Light, Background = new SolidColorBrush(preferences.Midnight ? Theme.Rgb(32, 32, 32) : Theme.Rgb(250, 250, 250)) };
        Content = scroll;
        root.RequestedTheme = preferences.Midnight ? ElementTheme.Dark : ElementTheme.Light;
        if (AppWindow.Presenter is OverlappedPresenter presenter) { presenter.IsResizable = false; presenter.IsMaximizable = false; }
        preview.ClearColor = preferences.Theme.Background;
        WindowSizing.Appearance(this, preferences.Midnight);
        var selectors = new Grid { ColumnSpacing = 8 };
        foreach (var width in new[] { GridLength.Auto, GridLength.Auto, GridLength.Auto, new GridLength(1, GridUnitType.Star) })
            selectors.ColumnDefinitions.Add(new() { Width = width });
        documentPicker.ItemsSource = new[] { "Plain Text", "Markdown", "Code" };
        AutomationProperties.SetName(documentPicker, "Document");
        AutomationProperties.SetName(stylePicker, "Style");
        var styleTemplates = new StyleIcons.PickerTemplates();
        foreach (var picker in new[] { stylePicker, parent, next }) picker.ItemTemplateSelector = styleTemplates;
        AutomationProperties.SetName(selectorBox, "Stylesheet selection");
        var selectorControls = new FrameworkElement[] {
            new TextBlock { Text = "Document", VerticalAlignment = VerticalAlignment.Center }, documentPicker,
            new TextBlock { Text = "Style", VerticalAlignment = VerticalAlignment.Center, Margin = new(12, 0, 0, 0) }, stylePicker
        };
        for (int column = 0; column < selectorControls.Length; column++) {
            Grid.SetColumn(selectorControls[column], column); selectors.Children.Add(selectorControls[column]);
        }
        selectorBox.Child = selectors; RefreshSelectorColors(); Add(selectorBox);
        var properties = new Grid { RowSpacing = 8, ColumnSpacing = 10, Margin = new(12, 0, 12, 4) };
        properties.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); properties.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        stylePicker.VerticalAlignment = VerticalAlignment.Center;
        var typeRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, VerticalAlignment = VerticalAlignment.Center };
        typeRow.Children.Add(styleTypeIcon); typeRow.Children.Add(styleType);
        Field(properties, "Style type", typeRow); Field(properties, "Based on", Relationship(parent, visitParent)); Field(properties, "Next paragraph", Relationship(next, visitNext));
        foreach (var row in properties.RowDefinitions) row.Height = new(28);
        Add(properties);
        Add(Separator());
        var tabs = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Center, Spacing = 2 };
        tabs.Children.Add(characterTab); tabs.Children.Add(paragraphTab); tabs.Children.Add(blockTab); Add(tabs);
        var fields = new Grid(); fields.Children.Add(character); fields.Children.Add(paragraph); fields.Children.Add(block);
        Add(new Border { Child = new ScrollViewer { Content = fields, VerticalScrollBarVisibility = ScrollBarVisibility.Auto }, Height = 320, Padding = new(12, 12, 12, 10), BorderBrush = new SolidColorBrush(preferences.Midnight ? Theme.Rgb(58, 58, 58) : Theme.Rgb(210, 210, 210)), BorderThickness = new(1), CornerRadius = new(6) });
        characterTab.Click += (_, _) => SelectTab(false); paragraphTab.Click += (_, _) => SelectTab(true); SelectTab(false);
        characterTab.Checked += (_, _) => SelectTab(false); paragraphTab.Checked += (_, _) => SelectTab(true);
        blockTab.Click += (_, _) => SelectTab(false, true); blockTab.Checked += (_, _) => SelectTab(false, true);
        BuildCharacter(); BuildParagraph(); BuildBlock();
        Add(new Border { Child = preview, BorderBrush = new SolidColorBrush(Theme.Rgb(64, 70, 80)), BorderThickness = new(1), CornerRadius = new(5) });
        preview.Draw += (_, e) => DrawPreview(e.DrawingSession);
        Add(error);
        var buttons = new Grid(); buttons.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); buttons.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        BuildThemeHistory(buttons);
        var close = new Button { Content = "Close", MinWidth = 82 }; close.Click += (_, _) => Close(); buttons.Children.Add(close); Grid.SetColumn(close, 1); Add(buttons);
        visitParent.Click += (_, _) => Navigate(new(selected.Namespace, selected.Parent));
        visitNext.Click += (_, _) => Navigate(new(1, selected.Next));
        stylePicker.SelectionChanged += (_, _) => { if (!loading && stylePicker.SelectedItem is StyleDefinition style) Navigate(style.Key); };
        documentPicker.SelectionChanged += (_, _) => SelectDocument();
        parent.SelectionChanged += (_, _) => { if (loading) return; Try(() => { string id = parent.SelectedItem is StyleDefinition p ? p.Id : ""; view.EditStyleString(selected, id.Length == 0 ? VIEM_STYLE_EDIT_CLEAR_PARENT : VIEM_STYLE_EDIT_SET_PARENT, 0, id); }); };
        next.SelectionChanged += (_, _) => { if (loading) return; Try(() => { string id = next.SelectedItem is StyleDefinition p ? p.Id : ""; view.EditStyleString(selected, id.Length == 0 ? VIEM_STYLE_EDIT_CLEAR_NEXT_STYLE : VIEM_STYLE_EDIT_SET_NEXT_STYLE, 0, id); }); };
        AttachView(followCaret);
        preferences.Changed += ThemeChanged; preferences.ThemesChanged += SelectedThemeChanged;
        AppWindow.Closing += (_, args) => {
            if (visibleColorPickers.Count == 0) return;
            args.Cancel = true; Close();
        };
        Closed += (_, _) => { DetachView(); DismissColorPickers(); closed = true; preferences.Changed -= ThemeChanged; preferences.ThemesChanged -= SelectedThemeChanged; view.Dispose(); styleDocument.Dispose(); foreach (var specimen in blockPreviews.Values) specimen.Dispose(); blockPreviews.Clear(); preview.RemoveFromVisualTree(); };
        Load(followCaret: followCaret);
    }
    private readonly ToggleButton characterTab = new() { Content = "Character", FontSize = 13, Width = 130, Padding = new(12, 3, 12, 3) };
    private readonly ToggleButton paragraphTab = new() { Content = "Paragraph", FontSize = 13, Width = 130, Padding = new(12, 3, 12, 3) };
    private readonly ToggleButton blockTab = new() { Content = "Block", FontSize = 13, Width = 130, Padding = new(12, 3, 12, 3) };
    private void SelectTab(bool paragraphSelected, bool blockSelected = false)
    {
        characterTab.IsChecked = !paragraphSelected && !blockSelected; paragraphTab.IsChecked = paragraphSelected; blockTab.IsChecked = blockSelected;
        character.Visibility = paragraphSelected || blockSelected ? Visibility.Collapsed : Visibility.Visible;
        paragraph.Visibility = paragraphSelected ? Visibility.Visible : Visibility.Collapsed;
        block.Visibility = blockSelected ? Visibility.Visible : Visibility.Collapsed;
    }
    internal void Retarget(CoreView nextView)
    {
        DismissColorPickers();
        DetachView();
        documentView = nextView; CreateThemeSession(nextView.Document.State.format); AttachView(true);
        Load(followCaret: true);
    }
    internal void FollowActiveView(CoreView nextView)
    {
        if (!closed && followsCaret && nextView.Id != 0 && documentView != nextView) Retarget(nextView);
    }
    private void Add(FrameworkElement item) => root.Children.Add(item);
    private void RefreshSelectorColors()
    {
        selectorBox.Background = new SolidColorBrush(preferences.Midnight ? Theme.Rgb(48, 48, 48) : Theme.Rgb(255, 255, 255));
        selectorBox.BorderBrush = new SolidColorBrush(preferences.Midnight ? Theme.Rgb(68, 68, 68) : Theme.Rgb(210, 210, 210));
    }
    private void SelectDocument()
    {
        if (loading || closed || documentPicker.SelectedIndex < 0) return;
        uint format = documentFormats[documentPicker.SelectedIndex];
        DismissColorPickers();
        try {
            CreateThemeSession(format);
            DetachView(); followsCaret = false;
            error.Text = ""; error.Visibility = Visibility.Collapsed;
            Load();
        }
        catch (Exception exception) {
            error.Text = exception.Message; error.Visibility = Visibility.Visible;
            Load(selected?.Key);
        }
    }
    private Border Separator() => new() { Height = 1, Background = new SolidColorBrush(preferences.Midnight ? Theme.Rgb(58, 58, 58) : Theme.Rgb(210, 210, 210)) };
    private static Button NavigationButton() => new() { Content = "↗", Width = 30, Padding = new(0), FontSize = 18, VerticalAlignment = VerticalAlignment.Center };
    private static Grid Relationship(ComboBox picker, Button visit)
    {
        var row = new Grid { ColumnSpacing = 6 };
        row.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        row.Children.Add(picker); Grid.SetColumn(visit, 1); row.Children.Add(visit); return row;
    }
    private void Navigate(StyleKey key)
    {
        if (key.Id.Length == 0) return;
        DismissColorPickers();
        CancelCaretFollow();
        if (key != selected.Key) Load(key);
    }
    private void UpdateNavigation(Button button, StyleKey key, string relationship)
    {
        var destination = sheet.Styles.FirstOrDefault(s => s.Key == key && s.Key != selected.Key);
        button.IsEnabled = destination != null;
        string label = destination == null ? "Go to " + relationship : "Go to " + destination.Name;
        AutomationProperties.SetName(button, label); ToolTipService.SetToolTip(button, label);
    }
    private bool IsDescendant(StyleDefinition style)
    {
        var visited = new HashSet<StyleKey>();
        while (visited.Add(style.Key)) {
            if (style.Key == selected.Key) return true;
            var ancestor = sheet.Styles.FirstOrDefault(s => s.Id == style.Parent && s.Namespace == style.Namespace);
            if (ancestor == null) break; style = ancestor;
        }
        return false;
    }
    private static void Field(Grid grid, string label, FrameworkElement value)
    { int row = grid.RowDefinitions.Count; grid.RowDefinitions.Add(new() { Height = GridLength.Auto, MinHeight = 24 }); var text = new TextBlock { Text = label, VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Right }; Grid.SetRow(text, row); grid.Children.Add(text); Grid.SetRow(value, row); Grid.SetColumn(value, 1); grid.Children.Add(value); }
    private static bool SameChoices(ComboBox picker, IEnumerable<object> choices)
    {
        static (StyleKey? Key, string Name) Identity(object item) => item switch {
            StyleDefinition style => (style.Key, style.Name),
            ComboBoxItem header => (null, header.Content as string ?? ""),
            _ => (null, item as string ?? "")
        };
        return picker.Items.Cast<object>().Select(Identity).SequenceEqual(choices.Select(Identity));
    }
    private StyleDefinition? refreshedDefinition;
    private StyleSheet? refreshedSheet;
    private global::Windows.UI.Color refreshedForeground;
    private StyleSheet? catalogueSheet;
    private void Load(StyleKey? key = null, bool followCaret = false, StyleSheet? snapshot = null)
    {
        if (view.Id == 0) { EnableChildren(root, false); return; }
        loading = true;
        try
        {
            sheet = snapshot ?? view.Styles();
            if (followCaret) key = CurrentCaretStyle(sheet);
#if DEBUG
            StyleLoads++;
#endif
            Title = ThemeStyleTitle;
            documentPicker.SelectedIndex = Array.IndexOf(documentFormats, styleDocument.State.format);
            restoreCodeDefaults.Visibility = view.UsesGlobalStyles ? Visibility.Visible : Visibility.Collapsed;
            var styles = sheet.Styles.Where(s => s.Native.role != VIEM_STYLE_ROLE_DOCUMENT && (s.Native.flags & VIEM_STYLE_DEFINITION_INTERNAL) == 0).ToArray();
            var chosen = styles.FirstOrDefault(s => s.Key == key) ?? styles.FirstOrDefault(s => (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0) ?? styles[0];
            if (selected != null && selected.Key != chosen.Key) {
                DismissColorPickers();
                blockLock.IsChecked = false;
            }
            selected = chosen;
            styleType.Text = selected.Native.role == VIEM_STYLE_ROLE_PARAGRAPH ? "Paragraph"
                : selected.Namespace == 2 ? "Character" : "Container";
            styleTypeIcon.Content = StyleIcons.Icon(StyleIcons.Kind(selected));
            if (!ReferenceEquals(catalogueSheet, sheet)) {
                var catalogue = new List<object>();
                var sections = new[] {
                    ("Paragraph", styles.Where(s => s.Native.role == VIEM_STYLE_ROLE_PARAGRAPH)),
                    ("Container", styles.Where(s => s.Native.role >= VIEM_STYLE_ROLE_QUOTE)),
                    ("Character", styles.Where(s => s.Namespace == 2))
                }.Select(section => (Title: section.Item1, Styles: section.Item2.OrderBy(s => s.Name, StringComparer.CurrentCultureIgnoreCase).ToArray()))
                    .Where(section => section.Styles.Length > 0).ToArray();
                foreach (var section in sections) {
                    if (sections.Length > 1) catalogue.Add(new ComboBoxItem { Content = section.Title, IsEnabled = false });
                    catalogue.AddRange(section.Styles);
                }
                if (!SameChoices(stylePicker, catalogue)) stylePicker.ItemsSource = catalogue;
                var following = new object[] { "Same Style" }.Concat(styles.Where(s => s.Native.role == VIEM_STYLE_ROLE_PARAGRAPH)).ToArray();
                if (!SameChoices(next, following)) next.ItemsSource = following;
                catalogueSheet = sheet;
            }
            stylePicker.SelectedItem = stylePicker.Items.OfType<StyleDefinition>().FirstOrDefault(s => s.Key == selected.Key);
            var parents = new object[] { selected.Namespace == 2 ? "Default Paragraph" : "None" }.Concat(styles.Where(s => s.Namespace == selected.Namespace && (s.Native.role == selected.Native.role || (selected.Native.role >= VIEM_STYLE_ROLE_QUOTE && (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0)) && !IsDescendant(s))).ToArray(); if (!SameChoices(parent, parents)) parent.ItemsSource = parents; parent.SelectedItem = parent.Items.OfType<StyleDefinition>().FirstOrDefault(s => s.Id == selected.Parent) ?? parent.Items[0]; parent.IsEnabled = selected.Has(VIEM_STYLE_CAPABILITY_EDIT_PARENT);
            next.SelectedItem = next.Items.Cast<object>().OfType<StyleDefinition>().FirstOrDefault(s => s.Id == selected.Next) ?? next.Items.Cast<object>().First();
            next.IsEnabled = selected.Has(VIEM_STYLE_CAPABILITY_EDIT_NEXT_STYLE);
            UpdateNavigation(visitParent, new(selected.Namespace, selected.Parent), "parent style"); UpdateNavigation(visitNext, new(1, selected.Next), "next paragraph style");
            paragraphTab.IsEnabled = selected.Namespace == 1;
            if (!paragraphTab.IsEnabled) SelectTab(false);
            blockTab.IsEnabled = selected.Namespace == 1;
            if (selected.Namespace != 1 && blockTab.IsChecked == true) SelectTab(false);
            if (!ReferenceEquals(refreshedDefinition, selected) || !ReferenceEquals(refreshedSheet, sheet)
                || refreshedForeground != preferences.Theme.Foreground) {
                foreach (var refresh in refreshFields) refresh();
                refreshedDefinition = selected; refreshedSheet = sheet; refreshedForeground = preferences.Theme.Foreground;
            }
            if (selected.Namespace != 1 || !selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS)) EnableChildren(paragraph, false);
            if (!selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS)) EnableChildren(character, false);
            if (selected.Namespace != 1 || !selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS)) EnableChildren(block, false);
            // Measure the populated controls before the first Activate. Resizing
            // from SizeChanged exposed successive startup layouts and flashes.
            // Later loads resize only when an error changes the content height.
            root.Measure(new global::Windows.Foundation.Size(ClientWidth, double.PositiveInfinity));
            if (!AppWindow.IsVisible)
            {
                // Initial template bindings settle during arrange. Complete that
                // hidden layout before using DesiredSize for the native window.
                root.Arrange(new global::Windows.Foundation.Rect(0, 0, ClientWidth, root.DesiredSize.Height));
                root.UpdateLayout();
                root.Measure(new global::Windows.Foundation.Size(ClientWidth, double.PositiveInfinity));
            }
            WindowSizing.FitClient(this, ClientWidth, (int)Math.Ceiling(root.DesiredSize.Height));
            preview.Invalidate();
        }
        finally { loading = false; }
    }
    private static void EnableChildren(Panel panel, bool enabled)
    {
        foreach (var child in panel.Children) { if (child is Control control) control.IsEnabled = enabled; if (child is Panel nested) EnableChildren(nested, enabled); }
    }
    private bool Try(Action action, bool reload = true)
    {
        if (loading || closed) return false;
        if (preferences.EnsureCurrentThemeExists()) return false;
        if (reload) DismissColorPickers();
        updating = true;
        byte[]? before = null;
        try
        {
            before = view.ExportStyleDefaults();
            error.Text = ""; error.Visibility = Visibility.Collapsed; action(); var key = selected.Key;
            byte[] after = view.ExportStyleDefaults();
            preferences.SaveThemeStyles(view.Document.State.format, after);
            RecordThemeEdit(before, after, key);
            if (reload) Load(key); else RefreshCommittedColors();
            return true;
        }
        catch (Exception e) {
            if (before != null) { if (view.UsesGlobalStyles) view.ReplaceCodeStyles(before); else CreateThemeSession(); }
            error.Text = e.Message; error.Visibility = Visibility.Visible;
            if (reload) Load(selected.Key); else RefreshCommittedColors();
            return false;
        }
        finally { updating = false; }
    }
    internal readonly record struct PreviewTextGeometry(float FontSize, float Baseline, float Width);
    private PreviewTextGeometry? previewTextGeometry;
    internal PreviewTextGeometry DrawPreviewForTesting(CanvasDrawingSession drawing, float width, float height)
    {
        DrawPreview(drawing, width, height);
        return previewTextGeometry ?? throw new InvalidOperationException("The selected style has no character preview.");
    }
    private void DrawPreview(CanvasDrawingSession drawing, float? width = null, float? height = null)
    {
        previewTextGeometry = null;
        drawing.Clear(preferences.Theme.Background);
        if (selected == null) return;
        if (selected.Namespace == 1) {
            try {
                if (!blockPreviews.TryGetValue(selected.Native.role, out var blockPreview)) {
                    blockPreview = new(selected.Native.role, preview.Device, DispatcherQueue);
                    blockPreviews.Add(selected.Native.role, blockPreview);
                }
                blockPreview.Update(sheet, selected, preferences.Theme.Foreground);
                blockPreview.Draw(drawing, width ?? (float)preview.ActualWidth, height ?? (float)preview.ActualHeight, preferences.Theme.Foreground);
            } catch (Exception exception) {
                using var message = new CanvasTextFormat { FontSize = 12 };
                drawing.DrawText("Style preview: " + exception.Message, 12, 12, preferences.Theme.Foreground, message);
            }
            return;
        }
        string family = sheet.String(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES));
        var resolved = FontCatalog.Resolve(family) ?? (Family: "Segoe UI", Stretch: FontStretch.Normal);
        float size = selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SIZE).number;
        bool superscript = selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SUPERSCRIPT).enum_value != 0;
        bool subscript = selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SUBSCRIPT).enum_value != 0;
        float baselineOffset = superscript ? size * .35f : subscript ? -size * .2f : 0;
        float renderedSize = superscript || subscript ? size * .75f : size;
        uint weight = selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).enum_value;
        if (selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_BOLD).enum_value != 0) weight = Math.Min(1000, weight + 300);
        var slant = selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SLANT).enum_value switch { 1 => FontStyle.Italic, 2 => FontStyle.Oblique, _ => FontStyle.Normal };
        if (slant == FontStyle.Normal) slant = CurrentFace?.Slant ?? slant;
        var renderedFace = FontCatalog.RenderingFace(resolved.Family, weight, slant, resolved.Stretch);
        using var format = new CanvasTextFormat { FontFamily = FontCatalog.RenderingFamily(resolved.Family, weight, slant, resolved.Stretch), FontStretch = resolved.Stretch, FontSize = renderedSize, WordWrapping = CanvasWordWrapping.Wrap };
        format.FontWeight = new FontWeight { Weight = (ushort)Math.Clamp(weight, 1, 999) };
        format.FontStyle = slant;
        format.HorizontalAlignment = selected.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT).enum_value switch { 2 => CanvasHorizontalAlignment.Right, 3 => CanvasHorizontalAlignment.Center, _ => CanvasHorizontalAlignment.Left };
        string sample = selected.Native.role >= VIEM_STYLE_ROLE_QUOTE
            ? "A first paragraph inside this container.\n\nA second paragraph shares its block box."
            : "A calm writing surface shaped with the selected style,\nwith line spacing and alignment visible.";
        float Edge(uint key) => selected.Namespace == 1 ? Math.Clamp(selected.Value(key).number, 0, 24) : 0;
        float left = Edge(VIEM_STYLE_PROPERTY_BLOCK_MARGIN_LEFT), right = Edge(VIEM_STYLE_PROPERTY_BLOCK_MARGIN_RIGHT);
        float borderLeft = Edge(VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_WIDTH), borderRight = Edge(VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_WIDTH);
        float borderTop = Edge(VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_WIDTH), borderBottom = Edge(VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_WIDTH);
        float paddingLeft = Edge(VIEM_STYLE_PROPERTY_BLOCK_PADDING_LEFT), paddingRight = Edge(VIEM_STYLE_PROPERTY_BLOCK_PADDING_RIGHT);
        float boxX = 20 + left, boxY = 36 + Edge(VIEM_STYLE_PROPERTY_BLOCK_MARGIN_TOP);
        float boxWidth = Math.Max(1, (width ?? (float)preview.ActualWidth) - 40 - left - right);
        float textX = boxX + borderLeft + paddingLeft;
        float textY = boxY + borderTop + Edge(VIEM_STYLE_PROPERTY_BLOCK_PADDING_TOP);
        using var layout = new CanvasTextLayout(preview.Device, sample, format, Math.Max(1, boxWidth - borderLeft - borderRight - paddingLeft - paddingRight), 76);
        FontVariations.Apply(layout, 0, sample.Length, FontVariations.Effective(FontVariations.For(renderedFace), CurrentAxisValues, weight, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_BOLD).enum_value != 0, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SLANT).enum_value), renderedFace);
        layout.SetUnderline(0, sample.Length, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE).enum_value != 0);
        layout.SetStrikethrough(0, sample.Length, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH).enum_value != 0);
        layout.SetCharacterSpacing(0, sample.Length, 0, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_LETTER_SPACING).number, 0);
        // Anchor the specimen to the original font's baseline, then apply the
        // same script shift as core shaping. Shrinking the font must not add
        // another baseline shift of its own.
        float nativeBaseline = layout.LineMetrics[0].Baseline;
        textY += nativeBaseline * (size / renderedSize - 1) - baselineOffset;
        previewTextGeometry = new(renderedSize, textY + nativeBaseline, (float)layout.LayoutBounds.Width);
        using var context = new CanvasTextFormat { FontFamily = "Segoe UI", FontSize = 12 };
        var muted = preferences.Theme.Foreground; muted.A = 190;
        drawing.DrawText("Previous paragraph gives the style context.", 20, 20, muted, context);
        float boxHeight = textY - boxY + (float)layout.LayoutBounds.Height + Edge(VIEM_STYLE_PROPERTY_BLOCK_PADDING_BOTTOM) + borderBottom;
        var blockBackground = PreviewColor(VIEM_STYLE_PROPERTY_BLOCK_BACKGROUND);
        if (blockBackground.A > 0) drawing.FillRectangle(boxX, boxY, boxWidth, boxHeight, blockBackground);
        if (borderTop > 0) drawing.FillRectangle(boxX, boxY, boxWidth, borderTop, PreviewColor(VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_COLOR));
        if (borderBottom > 0) drawing.FillRectangle(boxX, boxY + boxHeight - borderBottom, boxWidth, borderBottom, PreviewColor(VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_COLOR));
        if (borderLeft > 0) drawing.FillRectangle(boxX, boxY, borderLeft, boxHeight, PreviewColor(VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_COLOR));
        if (borderRight > 0) drawing.FillRectangle(boxX + boxWidth - borderRight, boxY, borderRight, boxHeight, PreviewColor(VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_COLOR));
        var background = PreviewColor(VIEM_STYLE_PROPERTY_CHARACTER_BACKGROUND);
        if (background.A > 0) drawing.FillRectangle(textX, textY, (float)layout.LayoutBounds.Width, (float)layout.LayoutBounds.Height, background);
        drawing.DrawTextLayout(layout, textX, textY, PreviewColor(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND));
        drawing.DrawText("Following paragraph shows spacing and inheritance.", 20, boxY + boxHeight + 6 + Edge(VIEM_STYLE_PROPERTY_BLOCK_MARGIN_BOTTOM), muted, context);
    }
}

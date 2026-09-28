using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal enum ToolbarAction { Bold, Italic, Strikethrough, CharacterCode, Bullets, Numbers, CodeBlock, Indent, Unindent }

/// <summary>Native controls over the active core view's selection and transactions.</summary>
internal sealed class FormattingToolbar : UserControl
{
    internal readonly ScrollViewer Scroll = new() { HorizontalScrollMode = ScrollMode.Enabled, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollMode = ScrollMode.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Disabled };
    private EditorPane? pane;
    private readonly StackPanel row = new() { Orientation = Orientation.Horizontal, Spacing = 12, Padding = new(10, 0, 0, 0), VerticalAlignment = VerticalAlignment.Center };
    internal readonly DropDownButton Paragraph = Selector("Paragraph style");
    internal readonly DropDownButton Character = Selector("Character style");
    internal readonly Dictionary<ToolbarAction, ButtonBase> Buttons = [];
    internal readonly ToggleButton FormattedView = new() { Width = 28, Height = 26, MinWidth = 0, MinHeight = 0, Padding = new(0), VerticalAlignment = VerticalAlignment.Center, AllowFocusOnInteraction = false };
    private StyleSheet? sheet;
    private SelectedStyles? selected;
    private StyleChoice[] choices = [];
    private bool refreshing;
    private int tracking;
    internal event Action? PopupsClosed;
    internal bool HasOpenPopup => tracking > 0;
    private CoreView? View => pane?.View;

    internal FormattingToolbar()
    {
        Height = 42; Visibility = Visibility.Collapsed;
        Scroll.Content = row;
        var layout = new Grid { ColumnSpacing = 12, Padding = new(0, 0, 10, 0) };
        layout.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        layout.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        layout.Children.Add(Scroll); layout.Children.Add(FormattedView); Grid.SetColumn(FormattedView, 1);
        FormattedView.Resources = new ResourceDictionary { Source = new Uri("ms-appx:///Shell/MenuToggleResources.xaml") };
        FormattedView.Content = FormattedViewIcon(FormattedView);
        AutomationProperties.SetName(FormattedView, "Formatted view");
        ToolTipService.SetToolTip(FormattedView, "Formatted view (WYSIWYG)");
        FormattedView.Click += (_, _) => {
            if (View is not { } view) return;
            Run(() => view.SetMarkdownSource(FormattedView.IsChecked != true));
            RestoreEditorFocus();
        };
        var chrome = new Border { Child = layout };
        chrome.SetBinding(Border.BackgroundProperty, new Microsoft.UI.Xaml.Data.Binding { Source = this, Path = new PropertyPath("Background") });
        Content = chrome;
        AutomationProperties.SetName(this, "Formatting toolbar");
        foreach (var selector in new[] { Paragraph, Character })
        {
            row.Children.Add(selector);
            var menu = (MenuFlyout)selector.Flyout;
            menu.Opening += (_, _) => tracking++;
            menu.Closed += (_, _) => { tracking--; DispatcherQueue.TryEnqueue(Refresh); PopupsClosed?.Invoke(); };
        }
        var character = Group(); row.Children.Add(character);
        Add(character, ToolbarAction.Bold, "Bold", "\uE8DD");
        Add(character, ToolbarAction.Italic, "Italic", "\uE8DB");
        Add(character, ToolbarAction.Strikethrough, "Strikethrough", "\uEDE0");
        Add(character, ToolbarAction.CharacterCode, "Code (Character)", "</>", literal: true);
        var block = Group(); row.Children.Add(block);
        Add(block, ToolbarAction.Bullets, "Bulleted List", "\uE8FD");
        Add(block, ToolbarAction.Numbers, "Numbered List", "\uE8EF");
        Add(block, ToolbarAction.CodeBlock, "Code Block", "{ }", literal: true);
        var indent = Group(); row.Children.Add(indent);
        Add(indent, ToolbarAction.Indent, "Indent", "\uE8F4", toggle: false);
        Add(indent, ToolbarAction.Unindent, "Unindent", "\uE8F3", toggle: false);
    }

    private static StackPanel Group(double spacing = 2) => new() { Orientation = Orientation.Horizontal, Spacing = spacing, VerticalAlignment = VerticalAlignment.Center };
    private static DropDownButton Selector(string label)
    {
        var button = new DropDownButton { Width = 148, Height = 26, MinHeight = 0, Padding = new(8, 0, 8, 0), FontSize = 12, Flyout = new MenuFlyout() };
        AutomationProperties.SetName(button, label); ToolTipService.SetToolTip(button, label); return button;
    }
    private void Add(Panel group, ToolbarAction action, string label, string glyph, bool toggle = true, bool literal = false)
    {
        ButtonBase button = toggle ? new ToggleButton { IsThreeState = true } : new Button();
        button.Width = 28; button.Height = 26; button.MinWidth = 0; button.MinHeight = 0; button.Padding = new(0);
        button.IsTabStop = true; button.AllowFocusOnInteraction = false;
        button.Resources = new ResourceDictionary { Source = new Uri("ms-appx:///Shell/MenuToggleResources.xaml") };
        button.Content = action is ToolbarAction.Bullets or ToolbarAction.Numbers or ToolbarAction.Indent or ToolbarAction.Unindent
            ? StructuralIcon(button, action)
            : literal ? new TextBlock { Text = glyph, FontSize = 13 } : new FontIcon { Glyph = glyph, FontSize = 14 };
        AutomationProperties.SetName(button, label); ToolTipService.SetToolTip(button, label);
        button.Click += (_, _) => Execute(action);
        Buttons.Add(action, button); group.Children.Add(button);
    }

    private static Canvas FormattedViewIcon(ToggleButton owner)
    {
        var canvas = new Canvas { Width = 18, Height = 20 };
        var geometry = new GeometryGroup();
        void Line(double x, double y, double endX, double endY) => geometry.Children.Add(new LineGeometry { StartPoint = new(x, y), EndPoint = new(endX, endY) });
        Line(2, 1, 12, 1); Line(12, 1, 17, 6); Line(17, 6, 17, 19); Line(17, 19, 2, 19); Line(2, 19, 2, 1);
        Line(12, 1, 12, 6); Line(12, 6, 17, 6);
        var outline = new Microsoft.UI.Xaml.Shapes.Path { Data = geometry, StrokeThickness = 1 };
        outline.SetBinding(Microsoft.UI.Xaml.Shapes.Shape.StrokeProperty, new Microsoft.UI.Xaml.Data.Binding { Source = owner, Path = new PropertyPath("Foreground") });
        var text = new TextBlock { Text = "Aa", FontFamily = new FontFamily("Georgia"), FontSize = 10 };
        text.SetBinding(TextBlock.ForegroundProperty, new Microsoft.UI.Xaml.Data.Binding { Source = owner, Path = new PropertyPath("Foreground") });
        Canvas.SetLeft(text, 3); Canvas.SetTop(text, 6);
        canvas.Children.Add(outline); canvas.Children.Add(text); return canvas;
    }

    private static Canvas StructuralIcon(ButtonBase owner, ToolbarAction action)
    {
        var canvas = new Canvas { Width = 18, Height = 16 };
        var geometry = new GeometryGroup();
        void Line(double x, double y, double endX, double endY) => geometry.Children.Add(new LineGeometry { StartPoint = new(x, y), EndPoint = new(endX, endY) });
        if (action is ToolbarAction.Bullets or ToolbarAction.Numbers)
        {
            for (int i = 0; i < 2; i++)
            {
                double y = 4 + i * 8; Line(7, y, 18, y);
                if (action == ToolbarAction.Bullets) geometry.Children.Add(new EllipseGeometry { Center = new(2, y), RadiusX = 1, RadiusY = 1 });
                else
                {
                    var number = new TextBlock { Text = (i + 1).ToString(), FontSize = 8 };
                    Canvas.SetTop(number, y - 6); canvas.Children.Add(number);
                }
            }
        }
        else
        {
            Line(0, 2, 18, 2); Line(9, 6, 18, 6); Line(9, 10, 18, 10); Line(0, 14, 18, 14);
            double tip = action == ToolbarAction.Indent ? 6 : 0, tail = 6 - tip;
            Line(tail, 8, tip, 8); Line(tail + (tip - tail) / 2, 5, tip, 8); Line(tail + (tip - tail) / 2, 11, tip, 8);
        }
        var path = new Microsoft.UI.Xaml.Shapes.Path { Data = geometry, StrokeThickness = 1 };
        path.SetBinding(Microsoft.UI.Xaml.Shapes.Shape.StrokeProperty, new Microsoft.UI.Xaml.Data.Binding { Source = owner, Path = new PropertyPath("Foreground") });
        canvas.Children.Add(path); return canvas;
    }

    internal void Synchronize(EditorPane? active, bool visible)
    {
        if (pane != active) { sheet = null; selected = null; choices = []; }
        pane = active;
        Visibility = visible && View != null ? Visibility.Visible : Visibility.Collapsed;
        if (Visibility == Visibility.Visible) Refresh();
    }

    internal void Refresh()
    {
        if (refreshing || Visibility != Visibility.Visible || View is not { Id: not 0 } view) return;
        using var timing = Diagnostics.InputPerformance.Measure("toolbar.refresh");
        refreshing = true;
        try
        {
            FormattedView.IsChecked = view.Document.State.format == VIEM_FORMAT_MARKDOWN;
            selected = view.SelectedNamedStyles();
            sheet = view.Styles(selected.Identity);
            choices = CoreView.StyleChoices(sheet, selected);
            bool available = view.HasFormattingSelection;
            if (!available) choices = choices.Select(c => c with { Enabled = false }).ToArray();
            if (tracking == 0) { RefreshSelector(Paragraph, 1); RefreshSelector(Character, 2); }
            foreach (var (action, semantic) in new[] { (ToolbarAction.Bold, VIEM_SEMANTIC_STYLE_STRONG), (ToolbarAction.Italic, VIEM_SEMANTIC_STYLE_EMPHASIS) })
            {
                var state = view.SemanticStyle(semantic);
                Set(action, state.state, (state.flags & (VIEM_SEMANTIC_STYLE_CAN_SET | VIEM_SEMANTIC_STYLE_CAN_CLEAR)) != 0);
            }
            Set(ToolbarAction.Strikethrough, available ? view.StrikethroughState() : 0, view.CanFormatStrikethrough);
            RefreshCode(ToolbarAction.CharacterCode, new(2, "Code"), new(2, ""));
            RefreshCode(ToolbarAction.CodeBlock, new(1, "Code Block"), new(1, "Paragraph"));
            Set(ToolbarAction.Bullets, selected.ListState(VIEM_LIST_STYLE_BULLET), available);
            Set(ToolbarAction.Numbers, selected.ListState(VIEM_LIST_STYLE_NUMBERED), available);
            uint indent = available ? view.ListCapabilities() : 0;
            Set(ToolbarAction.Indent, 0, (indent & VIEM_LIST_CAN_INDENT) != 0);
            Set(ToolbarAction.Unindent, 0, (indent & VIEM_LIST_CAN_UNINDENT) != 0);
        }
        catch (Exception error) { pane?.Report(error); }
        finally { refreshing = false; }
    }

    private void Set(ToolbarAction action, uint state, bool enabled = true, bool visible = true)
    {
        var button = Buttons[action];
        if (button is ToggleButton toggle)
        {
            bool? value = state == VIEM_SEMANTIC_STYLE_STATE_MIXED ? null : state == VIEM_SEMANTIC_STYLE_STATE_ON;
            if (toggle.IsChecked != value) toggle.IsChecked = value;
        }
        if (button.IsEnabled != enabled) button.IsEnabled = enabled;
        var visibility = visible ? Visibility.Visible : Visibility.Collapsed;
        if (button.Visibility != visibility) button.Visibility = visibility;
    }
    private void RefreshCode(ToolbarAction action, StyleKey code, StyleKey fallback)
    {
        bool active = choices.Any(c => c.Key == code && c.Selected);
        bool available = choices.Any(c => c.Key == (active ? fallback : code) && c.Enabled);
        Set(action, active ? 1u : 0u, available, available);
    }

    private void RefreshSelector(DropDownButton button, uint space)
    {
        var entries = choices.Where(c => c.Key.Namespace == space).ToArray();
        var menu = (MenuFlyout)button.Flyout;
        // Keep native item objects even when the selection becomes Mixed.
        if (!menu.Items.Select(i => i.Tag).SequenceEqual(entries.Select(e => (object)e.Key)))
        {
            menu.Items.Clear();
            foreach (var entry in entries)
            {
                var item = new ToggleMenuFlyoutItem { Tag = entry.Key };
                item.Click += (_, _) => {
                    if (item.CommandParameter is not ViemStyleSheetIdentityV1 identity || View is not { } view) return;
                    Run(() => view.ChooseStyle((StyleKey)item.Tag, identity));
                    RestoreEditorFocus();
                };
                menu.Items.Add(item);
            }
        }
        // Tag/identity are updated only outside native tracking.
        for (int i = 0; i < entries.Length; i++)
        {
            var item = (ToggleMenuFlyoutItem)menu.Items[i]; var entry = entries[i];
            item.CommandParameter = sheet!.Identity;
            if (item.Text != entry.Name) item.Text = entry.Name;
            if (item.IsEnabled != entry.Enabled) item.IsEnabled = entry.Enabled;
            if (item.IsChecked != entry.Selected) item.IsChecked = entry.Selected;
        }
        string title = entries.FirstOrDefault(c => c.Selected)?.Name ?? "Mixed";
        if (!Equals(button.Content, title)) button.Content = title;
        button.IsEnabled = entries.Any(c => c.Enabled);
    }

    private void Run(Action action) { pane?.Run(action); Refresh(); }
    internal void Execute(ToolbarAction action)
    {
        if (View is not { } view) return;
        Refresh();
        if (!Buttons[action].IsEnabled || Buttons[action].Visibility != Visibility.Visible) return;
        Run(() => {
            switch (action)
            {
                case ToolbarAction.Bold: view.ToggleSemantic(VIEM_SEMANTIC_STYLE_STRONG); break;
                case ToolbarAction.Italic: view.ToggleSemantic(VIEM_SEMANTIC_STYLE_EMPHASIS); break;
                case ToolbarAction.Strikethrough: view.ToggleStrikethrough(); break;
                case ToolbarAction.CharacterCode: Code(new(2, "Code"), new(2, "")); break;
                case ToolbarAction.CodeBlock: Code(new(1, "Code Block"), new(1, "Paragraph")); break;
                case ToolbarAction.Bullets: List(VIEM_LIST_STYLE_BULLET); break;
                case ToolbarAction.Numbers: List(VIEM_LIST_STYLE_NUMBERED); break;
                case ToolbarAction.Indent: view.IndentList(false); break;
                case ToolbarAction.Unindent: view.IndentList(true); break;
            }
        });
        RestoreEditorFocus();
        void List(uint kind) => view.SetList(view.SelectedNamedStyles().ListState(kind) == 1 ? VIEM_LIST_STYLE_NONE : kind);
        void Code(StyleKey code, StyleKey fallback) { if (sheet != null) view.ChooseStyle(choices.Any(c => c.Key == code && c.Selected) ? fallback : code, sheet.Identity); }
    }

    internal bool DismissPopups()
    {
        ((MenuFlyout)Paragraph.Flyout).Hide(); ((MenuFlyout)Character.Flyout).Hide();
        return tracking > 0;
    }

    internal void RestoreEditorFocus()
    {
        // Let the native button/menu finish consuming its activation key before
        // returning to the editor, where Space can replace a text selection.
        var target = pane;
        var timer = DispatcherQueue.CreateTimer();
        timer.Interval = TimeSpan.FromMilliseconds(16); timer.IsRepeating = false;
        timer.Tick += (_, _) => { if (pane == target && target?.View is { Id: not 0 } && target.IsActive) target.FocusEditor(); };
        timer.Start();
    }
}

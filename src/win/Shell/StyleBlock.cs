using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class StyleWindow
{
    private readonly ToggleButton blockLock = new() { Width = 32, Height = 28, MinWidth = 0, Padding = new(0), HorizontalAlignment = HorizontalAlignment.Right };
    // Physical order is also the precedence when linking existing values.
    private static readonly (string Label, uint[] Properties)[] BlockRows = [
        ("Margin", [VIEM_STYLE_PROPERTY_BLOCK_MARGIN_LEFT, VIEM_STYLE_PROPERTY_BLOCK_MARGIN_RIGHT, VIEM_STYLE_PROPERTY_BLOCK_MARGIN_TOP, VIEM_STYLE_PROPERTY_BLOCK_MARGIN_BOTTOM]),
        ("Border weight", [VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_WIDTH, VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_WIDTH, VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_WIDTH, VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_WIDTH]),
        ("Border color", [VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_COLOR, VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_COLOR, VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_COLOR, VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_COLOR]),
        ("Padding", [VIEM_STYLE_PROPERTY_BLOCK_PADDING_LEFT, VIEM_STYLE_PROPERTY_BLOCK_PADDING_RIGHT, VIEM_STYLE_PROPERTY_BLOCK_PADDING_TOP, VIEM_STYLE_PROPERTY_BLOCK_PADDING_BOTTOM])
    ];

    private void BuildBlock()
    {
        var top = new Grid();
        top.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        top.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        block.Children.Add(top);
        ColorControl(top, "Background color", VIEM_STYLE_PROPERTY_BLOCK_BACKGROUND);
        var lockGroup = new StackPanel { Spacing = 3 };
        lockGroup.Children.Add(new TextBlock { Height = 17 });
        lockGroup.Children.Add(blockLock); top.Children.Add(lockGroup); Grid.SetColumn(lockGroup, 1);
        AutomationProperties.SetName(blockLock, "Link block sides");
        void RefreshLock() {
            blockLock.Content = new FontIcon { Glyph = blockLock.IsChecked == true ? "\uE72E" : "\uE785", FontSize = 16 };
            ToolTipService.SetToolTip(blockLock, blockLock.IsChecked == true ? "Unlink block sides" : "Link block sides");
            blockLock.IsEnabled = selected.Namespace == 1 && selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS);
        }
        refreshFields.Add(RefreshLock);
        void LockChanged(object sender, RoutedEventArgs args) {
            if (loading) return;
            if (blockLock.IsChecked == true && !Try(LinkBlockSides)) blockLock.IsChecked = false;
            RefreshLock();
        }
        blockLock.Checked += LockChanged;
        blockLock.Unchecked += LockChanged;

        var grid = new Grid { ColumnSpacing = 8, RowSpacing = 6 };
        grid.ColumnDefinitions.Add(new() { Width = new(82) });
        for (int i = 0; i < 4; i++) grid.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        for (int i = 0; i < 5; i++) grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
        block.Children.Add(grid);
        void Add(FrameworkElement element, int row, int column) {
            grid.Children.Add(element); Grid.SetRow(element, row); Grid.SetColumn(element, column);
        }
        string[] sides = ["Left", "Right", "Top", "Bottom"];
        for (int i = 0; i < sides.Length; i++) Add(new TextBlock { Text = sides[i], FontSize = 11, Opacity = .65 }, 0, i + 1);
        for (int row = 0; row < BlockRows.Length; row++) {
            var (label, properties) = BlockRows[row];
            Add(new TextBlock { Text = label, FontSize = 11, Opacity = .65, VerticalAlignment = VerticalAlignment.Center }, row + 1, 0);
            for (int side = 0; side < sides.Length; side++) {
                var cell = new StackPanel(); Add(cell, row + 1, side + 1);
                string name = sides[side] + " " + label.ToLowerInvariant();
                if (StyleDefinition.IsBorderColor(properties[side])) ColorControl(cell, name, properties[side], caption: false);
                else Number(cell, name, properties[side], min: row == 0 ? -1000 : 0, width: 64, caption: false);
            }
        }
    }

    private void EditLinkedProperty(uint operation, uint property, ViemStyleEditValueV1 value, ViemStyleEditGroupV1? group = null)
    {
        uint[] properties = blockLock.IsChecked == true
            ? BlockRows.FirstOrDefault(row => row.Properties.Contains(property)).Properties ?? [property] : [property];
        foreach (uint side in properties) view.EditStyle(selected, operation, side, value, group);
    }

    private void LinkBlockSides()
    {
        // Read one snapshot before applying any changes. An entirely inherited
        // row stays inherited, including borders using the current text color.
        foreach (var (_, properties) in BlockRows) {
            if (!IsBase && !properties.Any(selected.Declares)) continue;
            if (StyleDefinition.IsBorderColor(properties[0])) {
                var colors = properties.Where(p => !selected.UsesTextColor(p) && selected.Value(p).kind == VIEM_STYLE_VALUE_COLOR).ToArray();
                if (colors.Length == 0) continue;
                uint first = colors.FirstOrDefault(p => selected.Value(p).color.alpha != 0, colors[0]);
                var value = CoreView.Enum(VIEM_STYLE_VALUE_COLOR, 0);
                value.color = selected.Value(first).color;
                EditLinkedProperty(VIEM_STYLE_EDIT_SET_DECLARATION, properties[0], value);
            } else {
                float value = properties.Select(p => selected.Value(p).number).FirstOrDefault(number => number != 0);
                EditLinkedProperty(VIEM_STYLE_EDIT_SET_DECLARATION, properties[0], CoreView.Number(value));
            }
        }
    }
}

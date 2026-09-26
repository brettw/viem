using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Windows.Foundation;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private readonly FormattingToolbar formattingToolbar;
    private readonly ToggleButton toolbarToggle = new() { Width = 40, Height = 30, MinHeight = 0, Padding = new(0), BorderThickness = new(0), Visibility = Visibility.Collapsed, AllowFocusOnInteraction = false };
    private bool closeAfterToolbarPopup;

    private void ConfigureFormattingToolbar()
    {
        toolbarToggle.Resources = new ResourceDictionary { Source = new Uri("ms-appx:///Shell/MenuToggleResources.xaml") };
        // The same thin rectangle and three square buttons as the Mac title bar.
        var geometry = new GeometryGroup();
        geometry.Children.Add(new RectangleGeometry { Rect = new Rect(1.5, 3.5, 21, 11) });
        foreach (double x in new[] { 4d, 10d, 16d }) geometry.Children.Add(new RectangleGeometry { Rect = new Rect(x, 7, 4, 4) });
        var icon = new Microsoft.UI.Xaml.Shapes.Path { Data = geometry, StrokeThickness = 1, Width = 24, Height = 18 };
        icon.SetBinding(Microsoft.UI.Xaml.Shapes.Shape.StrokeProperty, new Microsoft.UI.Xaml.Data.Binding { Source = toolbarToggle, Path = new PropertyPath("Foreground") });
        toolbarToggle.Content = icon;
        AutomationProperties.SetName(toolbarToggle, "Formatting toolbar");
        toolbarToggle.Click += (_, _) => Safe(() => {
            if (ActivePane != null) preferences.SetFormattingToolbar(ActivePane.Document.State.format, toolbarToggle.IsChecked == true);
            SynchronizeFormattingToolbar(); formattingToolbar.RestoreEditorFocus(); return Task.CompletedTask;
        });
        formattingToolbar.PopupsClosed += () => {
            if (closeAfterToolbarPopup && !formattingToolbar.HasOpenPopup) DispatcherQueue.TryEnqueue(Close);
        };
    }

    private void SynchronizeFormattingToolbar()
    {
        if (closed) return;
        uint format = ActivePane?.Document.State.format ?? VIEM_FORMAT_PLAIN_TEXT;
        bool available = format != VIEM_FORMAT_PLAIN_TEXT && format != VIEM_FORMAT_CODE;
        bool visible = available && preferences.ShowFormattingToolbar(format);
        toolbarToggle.Visibility = available ? Visibility.Visible : Visibility.Collapsed;
        toolbarToggle.IsChecked = visible;
        ToolTipService.SetToolTip(toolbarToggle, visible ? "Hide Formatting Toolbar" : "Show Formatting Toolbar");
        formattingToolbar.Synchronize(ActivePane, visible);
    }
#if DEBUG
    internal FormattingToolbar Toolbar => formattingToolbar;
    internal ToggleButton ToolbarToggle => toolbarToggle;
    internal ToggleButton MenuToggle => menuToggle;
#endif
}

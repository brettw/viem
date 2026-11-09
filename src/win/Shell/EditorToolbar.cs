using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Data;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using System.Diagnostics.CodeAnalysis;

namespace Viem.Windows.Shell;

internal partial class EditorToolbar : UserControl
{
    protected readonly StackPanel row = new() { Orientation = Orientation.Horizontal, Spacing = 8,
        Padding = new(10, 0, 0, 0), VerticalAlignment = VerticalAlignment.Center };
    private const double ControlHeight = 26;

    protected EditorToolbar(string name)
    {
        Height = 42;
        Visibility = Visibility.Collapsed;
        Resources = new ResourceDictionary { Source = new Uri("ms-appx:///Shell/ToolbarResources.xaml") };
        AutomationProperties.SetName(this, name);
    }

    protected void SetLayout(FrameworkElement viewport, FrameworkElement? trailing = null)
    {
        var layout = new Grid { Padding = new(0, 0, 10, 0) };
        layout.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star), MinWidth = 0 });
        layout.Children.Add(viewport);
        if (trailing != null) {
            layout.ColumnSpacing = 12;
            layout.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            Grid.SetColumn(trailing, 1);
            layout.Children.Add(trailing);
        }
        layout.SetBinding(Grid.BackgroundProperty, new Binding { Source = this, Path = new PropertyPath("Background") });
        Content = layout;
    }

    protected static StackPanel Group(double spacing = 2) => new() {
        Orientation = Orientation.Horizontal, Spacing = spacing, VerticalAlignment = VerticalAlignment.Center
    };

    protected static T IconButton<T>(T button, string label, object? content = null, double width = 28, string shortcut = "") where T : ButtonBase
    {
        button.Width = width; button.Height = ControlHeight;
        button.MinHeight = button.MinWidth = 0;
        button.Padding = new(0);
        button.AllowFocusOnInteraction = false;
        button.VerticalAlignment = VerticalAlignment.Center;
        button.Content = content;
        Label(button, label, shortcut);
        return button;
    }

    protected static T Selector<T>(T control, string label) where T : Control
    {
        control.Height = ControlHeight;
        control.MinHeight = 0;
        control.Padding = new(8, 0, 8, 0);
        control.FontSize = 12;
        control.VerticalAlignment = VerticalAlignment.Center;
        Label(control, label);
        return control;
    }

    protected static void Label(Control control, string label, string shortcut = "")
    {
        AutomationProperties.SetName(control, label);
        ToolTipService.SetToolTip(control, shortcut.Length == 0 ? label : $"{label} ({shortcut})");
    }

    protected static FontIcon Glyph(string glyph) => new() { Glyph = glyph, FontSize = 14 };

    // Text and FontIcon inherit the presenter's state-specific foreground.
    // Strokes and borders need an explicit brush from the same presenter.
    // WinUI creates this part from its native template. Keep its concrete
    // C#/WinRT projection when NativeAOT trims the release executable.
    [DynamicDependency(DynamicallyAccessedMemberTypes.All, typeof(ContentPresenter))]
    internal static void BindIconBrush(FrameworkElement icon, DependencyProperty brush)
    {
        ContentPresenter? source = null;
        long token = 0;
        icon.Loaded += (_, _) => {
            for (var parent = VisualTreeHelper.GetParent(icon); parent != null; parent = VisualTreeHelper.GetParent(parent)) {
                if (parent is ContentPresenter presenter) {
                    // Use the native property directly, including in trimmed NativeAOT builds.
                    if (source != null) source.UnregisterPropertyChangedCallback(ContentPresenter.ForegroundProperty, token);
                    source = presenter;
                    icon.SetValue(brush, presenter.Foreground);
                    token = presenter.RegisterPropertyChangedCallback(ContentPresenter.ForegroundProperty,
                        (_, _) => icon.SetValue(brush, presenter.Foreground));
                    break;
                }
            }
        };
        icon.Unloaded += (_, _) => {
            source?.UnregisterPropertyChangedCallback(ContentPresenter.ForegroundProperty, token);
            source = null;
        };
    }

    protected static Microsoft.UI.Xaml.Shapes.Path Outline(Geometry geometry)
    {
        var path = new Microsoft.UI.Xaml.Shapes.Path { Data = geometry, StrokeThickness = 1 };
        BindIconBrush(path, Shape.StrokeProperty);
        return path;
    }
}

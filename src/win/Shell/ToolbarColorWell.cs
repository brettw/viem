using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Windows.UI;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

/// <summary>A compact native picker whose delayed samples carry an exact selection.</summary>
internal sealed class ToolbarColorWell : Button
{
    private readonly uint property;
    private readonly Preferences preferences;
    private readonly Action<Action> run;
    private readonly Border swatch = new();
    private readonly Flyout popup = new() { ShouldConstrainToRootBounds = false };
    private readonly Microsoft.UI.Dispatching.DispatcherQueueTimer timer;
    private CoreView? view;
    private ViemLogicalSelectionIdentityV1 selection;
    private Color committed;
    private Color? pending;
    private bool open, applying, assigning;
    internal bool PopupVisible { get; private set; }
    internal event Action? PopupClosed;
    internal ColorPicker? Picker { get; private set; }

    internal ToolbarColorWell(string label, string glyph, uint property, Preferences preferences, Action<Action> run)
    {
        this.property = property; this.preferences = preferences; this.run = run;
        Padding = new(0); BorderThickness = new(0); MinWidth = 28; MinHeight = 0;
        Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        var stack = new StackPanel { Spacing = 1 };
        stack.Children.Add(new FontIcon { Glyph = glyph, FontSize = 11, Height = 12 });
        stack.Children.Add(StyleWindow.ColorSwatch(swatch)); Content = stack;
        AutomationProperties.SetName(this, label); ToolTipService.SetToolTip(this, label);
        var presenter = new Style(typeof(FlyoutPresenter));
        presenter.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(12)));
        presenter.Setters.Add(new Setter(FrameworkElement.MaxWidthProperty, 600d));
        popup.FlyoutPresenterStyle = presenter; Flyout = popup;
        timer = DispatcherQueue.CreateTimer(); timer.Interval = TimeSpan.FromMilliseconds(33); timer.IsRepeating = false;
        timer.Tick += (_, _) => Flush();
        popup.Opening += (_, _) => {
            if (view is not { Id: not 0 } || !IsEnabled) { popup.Hide(); return; }
            if (Picker == null)
            {
                Picker = new ColorPicker { Orientation = Orientation.Horizontal, IsColorPreviewVisible = false, Height = 264 };
                Picker.Resources["ColorPickerVerticalOrientationMinHeight"] = 264d;
                Picker.Resources["ColorPickerVerticalOrientationMaxHeight"] = 264d;
                Picker.Resources["ColorPickerTextInputHorizontalOrientationMargin"] = 68d;
                AutomationProperties.SetName(Picker, label);
                Picker.ColorChanged += (_, args) => {
                    if (!open || assigning) return;
                    pending = args.NewColor == committed ? null : args.NewColor;
                    if (pending == null) timer.Stop(); else if (!timer.IsRunning) timer.Start();
                };
                popup.Content = Picker;
            }
            assigning = true;
            try {
                Picker.RequestedTheme = preferences.Midnight ? ElementTheme.Dark : ElementTheme.Light;
                Picker.IsAlphaEnabled = view.Document.State.format != VIEM_FORMAT_RTF;
                Picker.Color = committed;
            } finally { assigning = false; }
            selection = view.LogicalSelection(); open = true; PopupVisible = true;
        };
        popup.Closed += (_, _) => { Flush(); open = false; PopupVisible = false; timer.Stop(); pending = null; PopupClosed?.Invoke(); };
    }

    internal void Refresh(CoreView target, ViemTypographyInfoV1 typography, bool enabled)
    {
        var current = target.LogicalSelection();
        // A queued sample never follows a moved caret, another pane, or an
        // externally edited document. Our own verified transaction updates it.
        if (open && !applying && (target != view || !current.Equals(selection) || !enabled)) Dismiss();
        view = target;
        IsEnabled = enabled;
        if (open && !applying) return;
        committed = property == VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND
            ? (typography.flags & 4) != 0 ? preferences.Theme.Foreground : Color(typography.foreground)
            : typography.has_background != 0 ? Color(typography.background) : Microsoft.UI.Colors.Transparent;
        if (swatch.Background is not SolidColorBrush brush || brush.Color != committed) swatch.Background = new SolidColorBrush(committed);
    }

    private void Flush()
    {
        timer.Stop(); var color = pending; pending = null;
        if (!open || color is not { } chosen || chosen == committed || view is not { Id: not 0 } target) return;
        if (!selection.Equals(target.LogicalSelection())) { Dismiss(); return; }
        applying = true;
        try
        {
            run(() => {
                var value = CoreView.Enum(VIEM_STYLE_VALUE_COLOR, 0);
                value.color = new() { red = chosen.R / 255f, green = chosen.G / 255f, blue = chosen.B / 255f,
                    alpha = target.Document.State.format == VIEM_FORMAT_RTF ? 1 : chosen.A / 255f };
                target.DirectStyle(property, value, expected: selection);
            });
            selection = target.LogicalSelection();
        }
        finally { applying = false; }
    }

    internal void Dismiss() { timer.Stop(); pending = null; open = false; popup.Hide(); }
    private static Color Color(ViemRgbaV1 value) => global::Windows.UI.Color.FromArgb(
        Channel(value.alpha), Channel(value.red), Channel(value.green), Channel(value.blue));
    private static byte Channel(float value) => (byte)Math.Clamp(Math.Round(value * 255), 0, 255);
}

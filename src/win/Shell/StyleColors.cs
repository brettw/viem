using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Windows.UI;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class StyleWindow
{
    private readonly List<Action<bool>> dismissColorPickers = [];
    private readonly List<Action> refreshColors = [];
    private readonly HashSet<uint> openColorPickers = [];
    private readonly HashSet<uint> visibleColorPickers = [];
    private bool refreshAfterColorPopup;
    private bool closeAfterColorPopup;

    public new void Close()
    {
        if (visibleColorPickers.Count > 0)
        {
            // A windowed WinUI flyout must finish closing before its owner HWND
            // is destroyed. Hiding it from Window.Closed is already too late.
            closeAfterColorPopup = true;
            DismissColorPickers();
            return;
        }
        base.Close();
    }

    private Color PreviewColor(uint property) => StyleColor(selected, property);

    private void RefreshCommittedColors()
    {
        sheet = view.Styles();
        selected = sheet.Styles.Single(s => s.Key == selected.Key);
        foreach (var refresh in refreshColors) refresh();
        preview.Invalidate();
    }

    private Color StyleColor(StyleDefinition style, uint property)
    {
        if (property == VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND && style.UsesThemeForeground) return preferences.Theme.Foreground;
        var value = style.Value(property);
        if (value.kind != VIEM_STYLE_VALUE_COLOR) return Microsoft.UI.Colors.Transparent;
        static byte Channel(float channel) => (byte)Math.Round(Math.Clamp(channel, 0, 1) * 255);
        return Color.FromArgb(Channel(value.color.alpha), Channel(value.color.red), Channel(value.color.green), Channel(value.color.blue));
    }
    private void DeclareEffective(uint property)
    {
        if (property == VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND && selected.UsesThemeForeground) SetColor(property, preferences.Theme.Foreground);
        else view.DeclareEffectiveStyle(selected, property, sheet);
    }
    private void DismissColorPickers(bool commit = true) { foreach (var dismiss in dismissColorPickers) dismiss(commit); refreshAfterColorPopup = false; }
    private void ThemeChanged()
    {
        if (closed) return;
        foreach (var refresh in refreshColors) refresh();
        preview.ClearColor = preferences.Theme.Background;
        preview.Invalidate();
    }
    internal static Grid ColorSwatch(Border colorLayer)
    {
        const double width = 28, height = 24, squareSize = 5;
        // WinUI layout units are DIPs. The rounded Grid clips both layers while
        // the selected color composites over a fixed, opaque checkerboard.
        var swatch = new Grid {
            Width = width, Height = height, CornerRadius = new(12),
            Background = new SolidColorBrush(Microsoft.UI.Colors.White),
            IsHitTestVisible = false
        };
        var checkerboard = new Canvas();
        var gray = new SolidColorBrush(Color.FromArgb(255, 179, 179, 179));
        for (int row = 0; row * squareSize < height; row++)
            for (int column = 0; column * squareSize < width; column++)
            {
                if ((row + column) % 2 == 0) continue;
                var square = new Microsoft.UI.Xaml.Shapes.Rectangle {
                    Width = Math.Min(squareSize, width - column * squareSize),
                    Height = Math.Min(squareSize, height - row * squareSize), Fill = gray
                };
                Canvas.SetLeft(square, column * squareSize); Canvas.SetTop(square, row * squareSize);
                checkerboard.Children.Add(square);
            }
        swatch.Children.Add(checkerboard);
        swatch.Children.Add(colorLayer);
        swatch.Children.Add(new Border {
            CornerRadius = new(12), BorderThickness = new(1),
            BorderBrush = new SolidColorBrush(Color.FromArgb(160, 150, 150, 150))
        });
        return swatch;
    }

    private void ColorControl(Panel row, string label, uint property)
    {
        var well = new Border();
        var button = new Button { Content = ColorSwatch(well), Padding = new(0), MinWidth = 28, Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent), BorderThickness = new(0) };
        var picker = new ColorPicker {
            IsAlphaEnabled = true, Orientation = Orientation.Horizontal,
            IsColorPreviewVisible = false, Height = 264,
            RequestedTheme = root.RequestedTheme
        };
        // Compact.xaml already supplies 24px text/combo controls application-wide.
        // ColorPicker has separate dimensions that Compact.xaml does not reduce.
        picker.Resources["ColorPickerVerticalOrientationMinHeight"] = 264d;
        picker.Resources["ColorPickerVerticalOrientationMaxHeight"] = 264d;
        picker.Resources["ColorPickerTextInputHorizontalOrientationMargin"] = 68d;
        var flyout = new Flyout { Content = picker, ShouldConstrainToRootBounds = false };
        var presenter = new Style(typeof(FlyoutPresenter));
        presenter.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(12)));
        // The default flyout width cap clips the stock horizontal picker even
        // when the popup itself is allowed outside the owner window.
        presenter.Setters.Add(new Setter(FrameworkElement.MaxWidthProperty, 600d));
        flyout.FlyoutPresenterStyle = presenter;
        button.Flyout = flyout;
        Property(row, label, property, button);
        bool open = false, assigning = false, didChange = false;
        Color committed = default;
        Color? pending = null;
        CoreView? owner = null;
        StyleDefinition? target = null;
        ViemStyleEditGroupV1? group = null;
        // A fixed cadence, not an inactivity debounce: sustained drags must
        // repaint the document too. Idle pickers never schedule work.
        var timer = DispatcherQueue.CreateTimer();
        timer.Interval = TimeSpan.FromMilliseconds(33);
        timer.IsRepeating = false;
        void EndGroup() { owner?.EndStyleEditGroup(group); group = null; }
        bool IsCurrent() => open && !closed && owner?.Id != 0 && owner == view && target?.Key == selected.Key;
        void Flush()
        {
            timer.Stop();
            var color = pending; pending = null;
            if (!IsCurrent() || color == null || color.Value == committed) return;
            bool success = Try(() => {
                group ??= view.BeginStyleEditGroup();
                SetColor(property, color.Value, group);
            }, reload: false);
            didChange |= success;
            committed = StyleColor(selected, property);
            well.Background = new SolidColorBrush(committed);
            if (!success)
            {
                EndGroup();
                // Only a rejected edit writes back to the active picker. Never
                // round-trip successful RGB changes through its HSV controls.
                assigning = true;
                try { picker.Color = committed; } finally { assigning = false; }
            }
        }
        timer.Tick += (_, _) => Flush();
        void Finish(bool commit)
        {
            if (!open) return;
            if (commit) Flush();
            timer.Stop(); pending = null;
            EndGroup();
            open = false;
            openColorPickers.Remove(property);
        }
        void Refresh()
        {
            // Do not feed rounded RGB values back into an active HSV gesture.
            if (open) return;
            picker.Color = StyleColor(selected, property);
            well.Background = new SolidColorBrush(picker.Color);
        }
        refreshFields.Add(Refresh); refreshColors.Add(Refresh);
        picker.ColorChanged += (_, args) => {
            if (assigning || !IsCurrent()) return;
            pending = args.NewColor;
            if (pending == committed) { pending = null; timer.Stop(); }
            else if (!timer.IsRunning) timer.Start();
        };
        flyout.Opening += (_, _) => {
            // Picking a property is an explicit edit of this style. A queued
            // caret follow must not retarget or rebuild the inspector mid-drag.
            refreshAfterColorPopup |= refreshAfterFollowing;
            CancelCaretFollow();
            Refresh(); committed = picker.Color; owner = view; target = selected; didChange = false; open = true;
            openColorPickers.Add(property);
            visibleColorPickers.Add(property);
        };
        flyout.Closed += (_, _) => {
            visibleColorPickers.Remove(property);
            if (closeAfterColorPopup && visibleColorPickers.Count == 0)
                DispatcherQueue.TryEnqueue(Close);
            if (!open) return;
            Finish(commit: true);
            bool refresh = refreshAfterColorPopup && openColorPickers.Count == 0;
            if (refresh) refreshAfterColorPopup = false;
            if ((didChange || refresh) && !closed && !loading) Load(selected.Key);
            if (!closed) { Refresh(); preview.Invalidate(); }
        };
        dismissColorPickers.Add(commit => { if (!open) return; Finish(commit); flyout.Hide(); });
#if DEBUG
        colorUpdateScheduled.Add(() => timer.IsRunning);
#endif
    }
#if DEBUG
    private readonly List<Func<bool>> colorUpdateScheduled = [];
    internal bool ColorUpdateScheduled => colorUpdateScheduled.Any(scheduled => scheduled());
    internal Color PreviewForeground => PreviewColor(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND);
    internal Color PreviewBackground => PreviewColor(VIEM_STYLE_PROPERTY_CHARACTER_BACKGROUND);
#endif
}

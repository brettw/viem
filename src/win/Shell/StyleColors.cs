using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Windows.UI;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class StyleWindow
{
    private readonly List<Action> dismissColorPickers = [];
    private readonly List<Action> refreshColors = [];
    private readonly Dictionary<uint, Color> colorDrafts = [];
    private readonly HashSet<uint> openColorPickers = [];
    private bool refreshAfterColorPopup;

    private Color PreviewColor(uint property) => colorDrafts.TryGetValue(property, out var color) ? color : StyleColor(selected, property);

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
    private void DismissColorPickers() { foreach (var dismiss in dismissColorPickers) dismiss(); refreshAfterColorPopup = false; }
    private void ThemeChanged()
    {
        if (closed) return;
        foreach (var refresh in refreshColors) refresh();
        preview.ClearColor = preferences.Theme.Background;
        preview.Invalidate();
    }
    private void ColorControl(Panel row, string label, uint property)
    {
        var well = new Border { Width = 28, Height = 24, CornerRadius = new(12), BorderThickness = new(1), BorderBrush = new SolidColorBrush(Color.FromArgb(160, 150, 150, 150)) };
        var button = new Button { Content = well, Padding = new(0), MinWidth = 28, Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent), BorderThickness = new(0) };
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
        bool open = false;
        Color original = default;
        CoreView? owner = null;
        StyleDefinition? target = null;
        void Refresh()
        {
            // Presentation and theme refreshes must not overwrite a popup draft.
            if (open) return;
            picker.Color = StyleColor(selected, property);
            well.Background = new SolidColorBrush(picker.Color);
        }
        refreshFields.Add(Refresh); refreshColors.Add(Refresh);
        picker.ColorChanged += (_, args) => {
            if (!open || closed || owner != view || target?.Key != selected.Key) return;
            colorDrafts[property] = args.NewColor;
            well.Background = new SolidColorBrush(args.NewColor);
            // CanvasControl coalesces invalidations into its next draw. This
            // changes only the preview; source and undo are committed on close.
            preview.Invalidate();
        };
        flyout.Opening += (_, _) => {
            // Picking a property is an explicit edit of this style. A queued
            // caret follow must not retarget or rebuild the inspector mid-drag.
            refreshAfterColorPopup |= refreshAfterFollowing;
            CancelCaretFollow();
            Refresh(); original = picker.Color; owner = view; target = selected; open = true;
            openColorPickers.Add(property);
        };
        flyout.Closed += (_, _) => {
            bool commit = open && !closed && !loading && owner == view && target?.Id == selected.Id
                && target.Namespace == selected.Namespace && ShowsValue(property) && selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS);
            open = false;
            openColorPickers.Remove(property);
            colorDrafts.Remove(property);
            bool refresh = refreshAfterColorPopup && openColorPickers.Count == 0;
            if (refresh) refreshAfterColorPopup = false;
            if (commit && picker.Color != original) Try(() => SetColor(property, picker.Color));
            else if (refresh && !closed && !loading) Load(selected.Key);
            if (!closed) { Refresh(); preview.Invalidate(); }
        };
        dismissColorPickers.Add(() => { open = false; openColorPickers.Remove(property); colorDrafts.Remove(property); flyout.Hide(); });
    }
#if DEBUG
    internal Color PreviewForeground => PreviewColor(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND);
    internal Color PreviewBackground => PreviewColor(VIEM_STYLE_PROPERTY_CHARACTER_BACKGROUND);
#endif
}

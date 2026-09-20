using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Viem.Windows.Rendering;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

/// <summary>Persistent Windows formatting controls following their chosen editor view.</summary>
internal abstract class FormattingPanelWindow : Window
{
    internal static readonly TimeSpan RefreshDelay = TimeSpan.FromMilliseconds(150);
    protected CoreView View { get; private set; }
    protected readonly Preferences Preferences;
    protected readonly StackPanel Root = new() { Padding = new(20), Spacing = 12 };
    protected bool Refreshing { get; private set; }
    private readonly TextBlock error = new() { TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed };
    private readonly DispatcherTimer refreshTimer = new() { Interval = RefreshDelay };
    private bool closed;

    protected FormattingPanelWindow(CoreView view, Preferences preferences, string title, int width, int height)
    {
        View = view; Preferences = preferences; Title = title;
        Root.RequestedTheme = preferences.Midnight ? ElementTheme.Dark : ElementTheme.Light;
        Content = new ScrollViewer { Content = Root, Background = new SolidColorBrush(preferences.Midnight ? Theme.Rgb(32, 32, 32) : Theme.Rgb(250, 250, 250)) };
        Root.Children.Add(error);
        WindowSizing.Resize(this, width, height); WindowSizing.Appearance(this, preferences.Midnight);
        if (AppWindow.Presenter is OverlappedPresenter presenter) { presenter.IsResizable = false; presenter.IsMaximizable = false; }
        Root.SizeChanged += (_, _) => { if (!closed && Root.ActualHeight > 0) WindowSizing.FitClient(this, width, (int)Math.Ceiling(Root.ActualHeight)); };
        refreshTimer.Tick += (_, _) => RefreshNow();
        Subscribe();
        Closed += (_, _) => { closed = true; refreshTimer.Stop(); Unsubscribe(); };
    }
    private void Subscribe() { View.Changed += ScheduleRefresh; View.Document.Changed += ScheduleRefresh; View.Disposed += Close; Preferences.Changed += ScheduleRefresh; }
    private void Unsubscribe() { View.Changed -= ScheduleRefresh; View.Document.Changed -= ScheduleRefresh; View.Disposed -= Close; Preferences.Changed -= ScheduleRefresh; }
    internal void Retarget(CoreView view)
    {
        if (closed || view == View) return;
        Unsubscribe(); View = view; Subscribe(); RefreshNow();
    }
    private void ScheduleRefresh()
    {
        if (closed || Refreshing) return;
        refreshTimer.Stop(); refreshTimer.Start();
    }
    protected void RefreshNow()
    {
        refreshTimer.Stop();
        if (closed || View.Id == 0) return;
        Refreshing = true;
        try { RefreshControls(); }
        catch (Exception exception) { Report(exception); }
        finally { Refreshing = false; }
    }
    protected abstract void RefreshControls();
    protected void Edit(Action action)
    {
        if (Refreshing || closed || !View.CanFormatCharacter) return;
        try { error.Visibility = Visibility.Collapsed; action(); }
        catch (Exception exception) { Report(exception); }
        RefreshNow();
    }
    private void Report(Exception exception) { error.Text = exception.Message; error.Visibility = Visibility.Visible; }
#if DEBUG
    internal int RefreshCount { get; private set; }
    protected void CountRefresh() => RefreshCount++;
    internal bool RefreshPending => refreshTimer.IsEnabled;
    internal string Error => error.Visibility == Visibility.Visible ? error.Text : "";
#endif
}

internal sealed class FontPanelWindow : FormattingPanelWindow
{
    private readonly ComboBox family = new() { IsEditable = true, Header = "Font family", HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly ComboBox variant = new() { Header = "Variant", PlaceholderText = "Custom / mixed", HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly NumberBox size = new() { Header = "Size (pt)", Minimum = .5, Maximum = 1000, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact };
    internal FontPanelWindow(CoreView view, Preferences preferences) : base(view, preferences, "Fonts", 360, 320)
    {
        family.ItemsSource = FontCatalog.Families;
        AutomationProperties.SetName(family, "Font family"); AutomationProperties.SetName(variant, "Font variant"); AutomationProperties.SetName(size, "Font size");
        Root.Children.Add(family); Root.Children.Add(variant); Root.Children.Add(size);
        family.SelectionChanged += (_, _) => { if (!Refreshing && family.SelectedItem is string value) ChooseFamily(value); };
        family.LostFocus += (_, _) => { if (!Refreshing) ChooseFamily(family.Text.Trim()); };
        variant.SelectionChanged += (_, _) => {
            if (!Refreshing && variant.SelectedItem is FontFace face)
                Edit(() => View.SetFont(face.Name, View.Typography().Info.size, face));
        };
        size.ValueChanged += (_, _) => {
            if (!Refreshing && double.IsFinite(size.Value))
                Edit(() => View.DirectStyle(VIEM_STYLE_PROPERTY_CHARACTER_SIZE, CoreView.Number((float)size.Value)));
        };
        RefreshNow();
    }
    private void ChooseFamily(string name)
    {
        if (name.Length == 0 || !View.CanFormatCharacter) return;
        var current = View.Typography();
        if (string.Equals(name, FontCatalog.DisplayFamily(current.Family), StringComparison.OrdinalIgnoreCase)) return;
        var face = FontCatalog.ForFamilyChange(name, FontCatalog.Current(current.Family, current.Info.base_weight, current.Info.slant));
        Edit(() => View.SetFont(name, current.Info.size, face));
    }
    protected override void RefreshControls()
    {
        var current = View.Typography();
        family.Text = FontCatalog.DisplayFamily(current.Family);
        family.SelectedItem = FontCatalog.Families.FirstOrDefault(f => string.Equals(f, family.Text, StringComparison.OrdinalIgnoreCase));
        variant.ItemsSource = FontCatalog.Faces(current.Family);
        variant.SelectedItem = FontCatalog.Current(current.Family, current.Info.base_weight, current.Info.slant);
        size.Value = current.Info.size;
        family.IsEnabled = variant.IsEnabled = size.IsEnabled = View.CanFormatCharacter;
#if DEBUG
        CountRefresh();
#endif
    }
#if DEBUG
    internal ComboBox FamilyControl => family;
    internal NumberBox SizeControl => size;
#endif
}

internal sealed class ColorPanelWindow : FormattingPanelWindow
{
    private readonly uint property;
    private readonly ColorPicker picker = new() { IsAlphaEnabled = true, IsColorSpectrumVisible = true, HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly Button reset = new() { Content = "Default", HorizontalAlignment = HorizontalAlignment.Right };
    private bool pointerEditing, commitQueued;
    private global::Windows.UI.Color? pendingColor;
    private global::Windows.UI.Color displayedColor;
    private CoreView? pendingView;
    private ViemLogicalSelectionIdentityV1? pendingSelection;
    internal ColorPanelWindow(CoreView view, Preferences preferences, uint property)
        : base(view, preferences, property == VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND ? "Text Color" : "Highlight Color", 360, 570)
    {
        this.property = property;
        AutomationProperties.SetName(picker, Title);
        Root.Children.Add(picker); Root.Children.Add(reset);
        picker.AddHandler(UIElement.PointerPressedEvent, new PointerEventHandler((_, _) => {
            pointerEditing = true; CaptureSelection();
        }), true);
        picker.AddHandler(UIElement.PointerReleasedEvent, new PointerEventHandler((_, _) => FinishPointerEdit()), true);
        picker.AddHandler(UIElement.PointerCaptureLostEvent, new PointerEventHandler((_, _) => FinishPointerEdit()), true);
        picker.AddHandler(UIElement.PointerCanceledEvent, new PointerEventHandler((_, _) => FinishPointerEdit()), true);
        picker.ColorChanged += (_, args) => {
            if (Refreshing) return;
            if (args.NewColor == displayedColor) { pendingColor = null; return; }
            pendingColor = args.NewColor;
            CaptureSelection();
            // Let the bubbling PointerPressed event identify a spectrum drag
            // before committing. The whole gesture then produces one undo.
            if (commitQueued) return;
            commitQueued = true;
            DispatcherQueue.TryEnqueue(() => {
                commitQueued = false;
                if (!pointerEditing) CommitColor();
            });
        };
        reset.Click += (_, _) => {
            pendingColor = null; pendingView = null; pendingSelection = null; pointerEditing = false;
            Edit(() => View.DirectStyle(property, default, true));
        };
        RefreshNow();
    }
    private void CaptureSelection()
    {
        if (pendingSelection != null || View.Id == 0) return;
        pendingView = View; pendingSelection = View.LogicalSelection();
    }
    private void FinishPointerEdit() { pointerEditing = false; CommitColor(); }
    private void CommitColor()
    {
        var color = pendingColor; var target = pendingView; var expected = pendingSelection;
        pendingColor = null; pendingView = null; pendingSelection = null;
        if (color is not { } chosen) { RefreshNow(); return; }
        if (target != View || View.Id == 0 || expected is not { } selection || !selection.Equals(View.LogicalSelection())) { RefreshNow(); return; }
        Edit(() => {
            var value = CoreView.Enum(VIEM_STYLE_VALUE_COLOR, 0);
            value.color = new() {
                red = chosen.R / 255f, green = chosen.G / 255f, blue = chosen.B / 255f,
                // Disabling the alpha control does not necessarily replace
                // a transparent inherited Color value in the native picker.
                alpha = View.Document.State.format == VIEM_FORMAT_RTF ? 1 : chosen.A / 255f
            };
            View.DirectStyle(property, value);
        });
    }
    protected override void RefreshControls()
    {
        if (pointerEditing) return;
        var current = View.Typography().Info;
        picker.IsAlphaEnabled = View.Document.State.format != VIEM_FORMAT_RTF;
        displayedColor = property == VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND
            ? (current.flags & 4) != 0 ? Preferences.Theme.Foreground : Color(current.foreground)
            : current.has_background != 0 ? Color(current.background) : Microsoft.UI.Colors.Transparent;
        picker.Color = displayedColor;
        picker.IsEnabled = reset.IsEnabled = View.CanFormatCharacter;
#if DEBUG
        CountRefresh();
#endif
    }
    private static global::Windows.UI.Color Color(ViemRgbaV1 value) => global::Windows.UI.Color.FromArgb(
        (byte)Math.Clamp(Math.Round(value.alpha * 255), 0, 255), (byte)Math.Clamp(Math.Round(value.red * 255), 0, 255),
        (byte)Math.Clamp(Math.Round(value.green * 255), 0, 255), (byte)Math.Clamp(Math.Round(value.blue * 255), 0, 255));
#if DEBUG
    internal ColorPicker Picker => picker;
    internal Button ResetControl => reset;
#endif
}

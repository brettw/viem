using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.System;
using Viem.Windows.Shell;
using Viem.Windows.Input;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    internal const double StatusInset = 10;
    internal double StatusBarHeight => status.Visibility == Visibility.Visible ? status.Height : 0;
    private readonly Button locationToggle = new() { MinWidth = 0, MinHeight = 0, Padding = new(2, 0, 2, 0), BorderThickness = new(0), VerticalAlignment = VerticalAlignment.Stretch };
    private readonly FontIcon lineModeIcon = new() { FontSize = 14 };
    private uint? statusPointer;
    private global::Windows.Foundation.Point statusPress, statusLastPoint;
    private bool statusDragging;
    private bool statusSelectingOutput;
    private readonly Grid normalStatus = new() { Margin = new(StatusInset, 0, 0, 0), ColumnSpacing = 12 };
    private readonly TextBlock filePath = new() { VerticalAlignment = VerticalAlignment.Center, TextWrapping = TextWrapping.NoWrap };
    private readonly TextBlock statusMeasure = new() { TextWrapping = TextWrapping.NoWrap };
    private readonly Grid filePathHost = new();
    private (string? Path, string Cwd)? filePathIdentity;
    private string fullStatusFilePath = "Untitled";
    private readonly Grid commandOutput = new() { Margin = new(StatusInset, 0, 0, 0), ColumnSpacing = 6, Visibility = Visibility.Collapsed };
    private readonly TextBlock outputText = new() { IsTextSelectionEnabled = true, TextWrapping = TextWrapping.NoWrap, VerticalAlignment = VerticalAlignment.Center };
    private readonly ScrollViewer outputScroll = new() {
        HorizontalScrollMode = ScrollMode.Enabled, VerticalScrollMode = ScrollMode.Enabled,
        HorizontalScrollBarVisibility = ScrollBarVisibility.Hidden, VerticalScrollBarVisibility = ScrollBarVisibility.Hidden,
        ZoomMode = ZoomMode.Disabled, VerticalContentAlignment = VerticalAlignment.Center
    };
    private readonly Button outputClose = new() { Content = "\uE711", FontFamily = new("Segoe MDL2 Assets"), FontSize = 10,
        Width = 20, Height = 20, MinWidth = 0, MinHeight = 0, Padding = new(0), BorderThickness = new(0), VerticalAlignment = VerticalAlignment.Center };
    private readonly DispatcherTimer outputTimer = new() { Interval = TimeSpan.FromSeconds(30) };
    private string? output;
    private bool outputHadFocus;
    private long outputDeadline;
    private (ulong Document, ulong Revision, ulong Cursor, uint Affinity, uint Mode, ulong Configuration, ulong Environment, ulong Metrics)? locationIdentity;
    private string locationText = "Ln —, Col —";

    private bool IsOutputDescendant(DependencyObject source)
    {
        for (DependencyObject? current = source; current != null; current = VisualTreeHelper.GetParent(current))
            if (current == outputText) return true;
        return false;
    }

    private void UpdateLocation()
    {
        if (View == null || snapshot == null) return;
        uint lineMode = View.CurrentLineMode;
        var id = snapshot.Info.identity;
        var key = (presentation.document_id, presentation.document_revision, presentation.cursor_utf8_offset,
            presentation.cursor_affinity, lineMode, id.configuration_generation, id.measurement_environment_id, id.metrics_generation);
        bool covered = snapshot.Rows.Any(row => row.text_start <= presentation.cursor_utf8_offset && presentation.cursor_utf8_offset <= row.text_end);
        if (lineMode == VIEM_LINE_MODE_PHYSICAL_SOURCE || covered)
        {
            var position = View.Location;
            string line = (position.flags & VIEM_LINE_LOCATION_GLOBAL_LINE_EXACT) != 0
                ? position.line.ToString() : $"{position.hard_line}·{position.fragment}";
            locationText = $"Ln {line}, Col {position.column}";
            locationIdentity = key;
        }
        else if (locationIdentity != key)
        {
            // Scrolling may deliberately leave the caret outside regional
            // geometry. Never abort prompt/viewport refresh to measure it.
            locationText = "Ln —, Col —";
            locationIdentity = null;
        }
        location.Text = (Document.IsReadOnly ? "🔒  " : "") + locationText;
        lineModeIcon.Glyph = lineMode == VIEM_LINE_MODE_VISUAL ? "\uE890" : "\uE8A5";
        ToolTipService.SetToolTip(locationToggle, lineMode == VIEM_LINE_MODE_VISUAL
            ? "Visual lines · click for physical source lines" : "Physical source lines · click for visual lines");
        AutomationProperties.SetName(locationToggle, (lineMode == VIEM_LINE_MODE_VISUAL ? "Visual" : "Physical source") + " lines: " + locationText);
    }

    private void BuildStatus()
    {
        status.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        status.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        foreach (var width in new[] { GridLength.Auto, new GridLength(1, GridUnitType.Star), GridLength.Auto })
            normalStatus.ColumnDefinitions.Add(new() { Width = width });
        normalStatus.Children.Add(mode);
        normalStatus.Children.Add(filePathHost); SetColumn(filePathHost, 1); filePathHost.Children.Add(filePath);
        normalStatus.Children.Add(message); SetColumn(message, 2);
        var pathMenu = new MenuFlyout();
        foreach (var (title, relative) in new[] { ("Copy full path", false), ("Copy relative path", true) })
        {
            var item = new MenuFlyoutItem { Text = title, IsEnabled = false };
            pathMenu.Items.Add(item);
            pathMenu.Opening += (_, _) => item.IsEnabled = !disposed && PathToCopy(relative) != null;
            item.Click += (_, _) => Run(() => CopyFilePath(relative));
        }
        filePath.ContextFlyout = pathMenu;
        filePathHost.SizeChanged += (_, _) => UpdateFilePathText();
        normalStatus.SizeChanged += (_, _) => message.MaxWidth = Math.Max(0, normalStatus.ActualWidth / 2);
        status.Children.Add(normalStatus); status.Children.Add(prompt); status.Children.Add(commandOutput);
        var position = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 5, VerticalAlignment = VerticalAlignment.Center };
        position.Children.Add(lineModeIcon); position.Children.Add(location); locationToggle.Content = position;
        locationToggle.Margin = new(0, 0, StatusInset, 0); status.Children.Add(locationToggle); SetColumn(locationToggle, 1);
        locationToggle.Click += (_, _) => {
            if (statusDragging || View == null) return;
            Run(() => View.LineMode(View.CurrentLineMode == VIEM_LINE_MODE_VISUAL ? VIEM_LINE_MODE_PHYSICAL_SOURCE : VIEM_LINE_MODE_VISUAL));
            FocusEditor();
        };
        // Listen even when a child control handles the press. Capture only once
        // movement crosses the drag threshold, retaining ordinary native clicks.
        status.AddHandler(PointerPressedEvent, new PointerEventHandler((_, e) => {
            var point = e.GetCurrentPoint(window.PaneStack);
            if (!point.Properties.IsLeftButtonPressed) return;
            statusPointer = e.Pointer.PointerId; statusPress = statusLastPoint = point.Position; statusDragging = false; statusSelectingOutput = !outputScroll.Visibility.Equals(Visibility.Collapsed) && e.OriginalSource is DependencyObject source && IsOutputDescendant(source);
        }), true);
        status.AddHandler(PointerMovedEvent, new PointerEventHandler((_, e) => {
            if (statusPointer != e.Pointer.PointerId) return;
            var point = e.GetCurrentPoint(window.PaneStack);
            if (!point.Properties.IsLeftButtonPressed) { statusPointer = null; statusDragging = false; status.SetDragging(false); return; }
            if (!statusDragging)
            {
                if (Math.Abs(point.Position.Y - statusPress.Y) < 4 && Math.Abs(point.Position.X - statusPress.X) < 4) return;
                statusDragging = true;
                if (window.PaneStack.CanDragBar(this)) status.SetDragging(true);
                if (!statusSelectingOutput && !status.CapturePointer(e.Pointer)) { statusPointer = null; statusDragging = false; status.SetDragging(false); return; }
            }
            double delta = point.Position.Y - statusLastPoint.Y;
            statusLastPoint = point.Position; // No blocked overshoot survives a reversal.
            window.PaneStack.DragBar(this, delta); e.Handled = true;
        }), true);
        status.AddHandler(PointerReleasedEvent, new PointerEventHandler((_, e) => {
            if (statusPointer != e.Pointer.PointerId) return;
            bool dragged = statusDragging;
            statusPointer = null; statusDragging = false; status.SetDragging(false);
            if (!dragged) window.FocusPane(this, keepStatusFocus: statusSelectingOutput);
            if (dragged && !statusSelectingOutput) { status.ReleasePointerCapture(e.Pointer); e.Handled = true; }
        }), true);
        status.PointerCaptureLost += (_, e) => {
            if (e.OriginalSource == status) { statusPointer = null; statusDragging = false; status.SetDragging(false); }
        };
        commandOutput.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        commandOutput.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        commandOutput.Children.Add(outputClose); commandOutput.Children.Add(outputScroll); SetColumn(outputScroll, 1);
        outputScroll.Content = outputText;
        AutomationProperties.SetName(status, "Editor status");
        AutomationProperties.SetName(outputText, "Command output");
        AutomationProperties.SetName(outputClose, "Close command output");
        ToolTipService.SetToolTip(outputClose, "Close command output");
        outputClose.Click += (_, _) => DismissCommandOutput();
        outputTimer.Tick += (_, _) => ExpireCommandOutput(Environment.TickCount64);
        outputText.SelectionChanged += (_, _) => window.UpdateTitle();
        commandOutput.GotFocus += (_, _) => { outputHadFocus = true; window.UpdateTitle(); };
        commandOutput.LostFocus += (_, _) => window.UpdateTitle();
        commandOutput.PreviewKeyDown += (_, e) => {
            bool control = Down(VirtualKey.Control), alt = Down(VirtualKey.Menu);
            if (control && !alt && e.Key is VirtualKey.C or VirtualKey.A) return;
            if (control && !alt && e.Key == VirtualKey.X) { e.Handled = true; return; }
            if (e.Key is VirtualKey.Left or VirtualKey.Right or VirtualKey.Up or VirtualKey.Down
                or VirtualKey.Home or VirtualKey.End or VirtualKey.PageUp or VirtualKey.PageDown or VirtualKey.Tab) return;
            DismissCommandOutput(); FocusEditor();
            // Command keys use the same routing as the editor. Printable keys
            // deliver their subsequent native character event to the input host.
            OnKey(input, e);
        };
    }

    private bool OutputHasFocus
    {
        get
        {
            if (output == null || XamlRoot == null) return false;
            for (var element = FocusManager.GetFocusedElement(XamlRoot) as DependencyObject; element != null; element = VisualTreeHelper.GetParent(element))
                if (element == commandOutput) return true;
            return false;
        }
    }

    private bool OutputIsActionTarget
    {
        get
        {
            if (OutputHasFocus) return true;
            if (output == null || !outputHadFocus || XamlRoot == null) return false;
            // Opening the Edit menu temporarily moves keyboard focus out of
            // the text. Its copy/select commands still target that selection.
            for (var element = FocusManager.GetFocusedElement(XamlRoot) as DependencyObject; element != null; element = VisualTreeHelper.GetParent(element))
                if (element is MenuBarItem or MenuFlyoutItemBase or MenuFlyoutPresenter) return true;
            return false;
        }
    }

    private void ApplyStatusTheme()
    {
        outputText.FontFamily = new("Consolas"); outputText.FontSize = preferences.StatusFontSize;
        outputText.Foreground = outputClose.Foreground = new SolidColorBrush(preferences.Theme.StatusForeground);
        outputClose.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        locationToggle.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        locationToggle.Foreground = new SolidColorBrush(preferences.Theme.StatusForeground);
        filePath.Foreground = new SolidColorBrush(preferences.Theme.StatusForeground);
        filePath.FontFamily = statusMeasure.FontFamily = mode.FontFamily;
        filePath.FontSize = statusMeasure.FontSize = mode.FontSize;
        mode.Width = Math.Ceiling(new[] { "NORMAL", "INSERT", "REPLACE", "VISUAL", "V-LINE", "V-BLOCK",
            "SELECTION", "SELECT", "S-LINE", "S-BLOCK", "COMMAND" }.Max(MeasureStatusText));
        UpdateFilePathText();
        UpdateStatusPresentation();
    }

    private double MeasureStatusText(string text)
    {
        statusMeasure.Text = text;
        // This detached probe is reused synchronously, before XAML's next
        // layout pass has processed Text changes. Discard its prior measure.
        statusMeasure.InvalidateMeasure();
        statusMeasure.Measure(new(double.PositiveInfinity, double.PositiveInfinity));
        return statusMeasure.DesiredSize.Width;
    }

    internal void RefreshStatusFilePath()
    {
        var identity = (Document.FilePath, Environment.CurrentDirectory);
        if (filePathIdentity == identity) return;
        filePathIdentity = identity;
        fullStatusFilePath = StatusFilePath.Display(identity.Item1, identity.Item2);
        ToolTipService.SetToolTip(filePath, fullStatusFilePath);
        AutomationProperties.SetName(filePath, "File: " + fullStatusFilePath);
        UpdateFilePathText();
    }

    private string? PathToCopy(bool relative) => relative
        ? StatusFilePath.Relative(Document.FilePath, Environment.CurrentDirectory)
        : StatusFilePath.Full(Document.FilePath);

    private void CopyFilePath(bool relative)
    {
        if (disposed || PathToCopy(relative) is not { } path) return;
#if DEBUG
        if (FilePathClipboardWriterForTesting is { } write) { write(path); return; }
#endif
        ClipboardFormats.Write(path, "");
    }

    private void UpdateFilePathText()
    {
        filePath.Text = StatusFilePath.TrimLeft(fullStatusFilePath, filePathHost.ActualWidth, MeasureStatusText);
    }

    private void UpdateStatusPresentation(bool refreshPrompt = true)
    {
        RefreshStatusFilePath();
        message.Visibility = message.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        bool command = presentation.mode == VIEM_MODE_COMMAND_LINE;
        if (command) { output = null; outputText.Text = ""; outputTimer.Stop(); outputDeadline = 0; outputHadFocus = false; }
        bool showingOutput = !command && output != null;
        normalStatus.Visibility = command || showingOutput ? Visibility.Collapsed : Visibility.Visible;
        prompt.Visibility = command ? Visibility.Visible : Visibility.Collapsed;
        commandOutput.Visibility = showingOutput ? Visibility.Visible : Visibility.Collapsed;
        status.Visibility = command || showingOutput || message.Text.Length > 0 || preferences.ShowStatus ? Visibility.Visible : Visibility.Collapsed;
        if (command && refreshPrompt) prompt.Refresh();
    }

    public void ShowCommandOutput(string text)
    {
        if (disposed) return;
        output = text; outputText.Text = text; outputScroll.ChangeView(0, 0, null, true);
        outputDeadline = Environment.TickCount64 + 30_000;
        outputTimer.Stop(); outputTimer.Start(); UpdateStatusPresentation(false);
        window.UpdateTitle();
    }

    internal void DismissCommandOutput(bool restoreFocus = true)
    {
        if (output == null) return;
        bool heldFocus = OutputHasFocus;
        output = null; outputText.Text = ""; outputTimer.Stop(); outputDeadline = 0; outputHadFocus = false;
        UpdateStatusPresentation(); window.UpdateTitle();
        if (restoreFocus && heldFocus) FocusEditor();
    }

    internal void ExpireCommandOutput(long now)
    { if (output != null && now >= outputDeadline) DismissCommandOutput(); }

#if DEBUG
    internal Action<string>? FilePathClipboardWriterForTesting { get; set; }
    internal CommandPrompt PromptControl => prompt;
    internal string? CommandOutputText => output;
    internal Grid StatusControl => status;
    internal TextBlock OutputTextControl => outputText;
    internal ScrollViewer OutputScrollControl => outputScroll;
    internal Button OutputCloseControl => outputClose;
    internal TextBlock LocationControl => location;
    internal Button LocationToggleControl => locationToggle;
    internal TextBlock ModeControl => mode;
    internal double ModeSlotWidth => normalStatus.ColumnDefinitions[0].ActualWidth;
    internal TextBlock FilePathControl => filePath;
    internal Grid FilePathHost => filePathHost;
    internal string FullStatusFilePath => fullStatusFilePath;
#endif
}

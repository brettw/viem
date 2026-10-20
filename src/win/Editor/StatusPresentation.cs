using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.Graphics.Canvas.UI.Xaml;
using Microsoft.Graphics.Canvas;
using Viem.Windows.Core;
using Viem.Windows.Rendering;
using Windows.System;
using Viem.Windows.Shell;
using Viem.Windows.Input;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    private ChromeStyle? statusStyle, commandStyle;
    private readonly CanvasControl statusBox = new() { IsHitTestVisible = false };
    internal ChromeStyle CommandStyle => commandStyle ?? preferences.StatusStyles.Command;
    internal float CommandInset => (statusStyle ?? preferences.StatusStyles.Status).Padding.Left;
    internal float CommandRightInset => (statusStyle ?? preferences.StatusStyles.Status).Padding.Right;
    internal double StatusBarHeight => status.Visibility == Visibility.Visible ? status.Height : 0;
    private readonly Button locationToggle = new() { MinWidth = 0, MinHeight = 0, Padding = new(2, 0, 2, 0), BorderThickness = new(0), VerticalAlignment = VerticalAlignment.Stretch };
    private readonly FontIcon lineModeIcon = new() { FontSize = 14 };
    private uint? statusPointer;
    private DependencyObject? statusPointerSource;
    private global::Windows.Foundation.Point statusPress, statusLastPoint;
    private bool statusDragging;
    private EditorPane? statusDragTarget;
    private bool statusDragNeedsSplit;
    private bool statusSelectingOutput;
    private double filePathTextWidth, modeTextWidth, messageTextWidth;
    private (string Mode, string Message, string Family, double Size)? resizeHoverTextIdentity;
    private readonly Grid normalStatus = new();
    private readonly TextBlock filePath = new() { VerticalAlignment = VerticalAlignment.Center, TextWrapping = TextWrapping.NoWrap };
    private readonly TextBlock statusMeasure = new() { TextWrapping = TextWrapping.NoWrap };
    private readonly Grid filePathHost = new();
    private (string? Path, string Cwd)? filePathIdentity;
    private string fullStatusFilePath = "Untitled";
    private readonly Grid commandOutput = new() { ColumnSpacing = 6, Visibility = Visibility.Collapsed };
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
    private string locationText = "Ln —, Col —";

    private bool IsOutputDescendant(DependencyObject source)
    {
        for (DependencyObject? current = source; current != null; current = VisualTreeHelper.GetParent(current))
            if (current == outputText) return true;
        return false;
    }

    internal bool StatusResizeHoverAt(global::Windows.Foundation.Point point)
    {
        if (status.Visibility != Visibility.Visible
            || point.X < 0 || point.Y < 0 || point.X >= status.ActualWidth || point.Y >= status.ActualHeight) return false;
        bool Over(FrameworkElement control, double? textWidth = null)
        {
            if (control.Visibility != Visibility.Visible) return false;
            var origin = control.TransformToVisual(status).TransformPoint(new global::Windows.Foundation.Point());
            double width = Math.Min(control.ActualWidth, textWidth ?? control.ActualWidth);
            return new global::Windows.Foundation.Rect(origin.X, origin.Y, width, control.ActualHeight).Contains(point);
        }
        if (Over(locationToggle)) return false;
        if (prompt.Visibility == Visibility.Visible && Over(prompt)) return false;
        if (commandOutput.Visibility == Visibility.Visible && (Over(outputScroll) || Over(outputClose))) return false;
        if (normalStatus.Visibility == Visibility.Visible && (Over(mode, modeTextWidth)
            || Over(filePath, filePathTextWidth) || Over(message, messageTextWidth))) return false;
        return true;
    }

    private void UpdateLocation()
    {
        if (View == null) return;
        uint lineMode = View.CurrentLineMode;
        // Source coordinates remain exact when the caret is outside cached layout.
        var position = View.Location;
        locationText = $"Ln {position.line}, Col {position.column}";
        if ((presentation.flags & VIEM_VIEW_PRESENTATION_HAS_SEARCH_COUNT) != 0)
            locationText += $" [{presentation.search_match_current}/{presentation.search_match_total}]";
        location.Text = (Document.IsReadOnly ? "🔒  " : "") + locationText;
        location.Width = Math.Ceiling(MeasureStatusText(location.Text));
        statusTextCanvas.Invalidate();
        lineModeIcon.Glyph = lineMode == VIEM_LINE_MODE_VISUAL ? "\uE890" : "\uE8A5";
        ToolTipService.SetToolTip(locationToggle, lineMode == VIEM_LINE_MODE_VISUAL
            ? "Commands use visual lines · click for physical source lines. Position uses physical source coordinates." : "Commands use physical source lines · click for visual lines. Position uses physical source coordinates.");
        AutomationProperties.SetName(locationToggle, "Physical source position: " + locationText);
    }

    private bool CaptureStatusDragPointer(PointerRoutedEventArgs args)
    {
        // Buttons and other native status controls own the press until it
        // becomes a pane drag. WinUI will not transfer their existing capture
        // implicitly; release this pointer only after crossing the threshold.
        for (var element = statusPointerSource ?? args.OriginalSource as DependencyObject; element != null && element != status;
            element = VisualTreeHelper.GetParent(element))
        {
            // Native controls can capture internally without populating the
            // public PointerCaptures list. Releasing this exact pointer is a
            // no-op for descendants that do not own it.
            if (element is UIElement owner) owner.ReleasePointerCapture(args.Pointer);
        }
        return status.CapturePointer(args.Pointer);
    }

    private void BuildStatus()
    {
        status.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        status.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        status.Children.Add(statusBox); SetColumnSpan(statusBox, 2);
        statusBox.Draw += (_, args) => {
            args.DrawingSession.Clear(Microsoft.UI.Colors.Transparent);
            var style = statusStyle ?? preferences.StatusStyles.Status;
            style.DrawBox(args.DrawingSession, new(-style.Border.Left, 0,
                statusBox.ActualWidth + style.Border.Horizontal,
                statusBox.ActualHeight + (hidesStatusBottomBorder ? style.Border.Bottom : 0)));
        };
        status.SizeChanged += (_, _) => status.Clip = new RectangleGeometry { Rect = new(0, 0, status.ActualWidth, status.ActualHeight) };
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
        pathMenu.Items.Add(new MenuFlyoutSeparator());
        var changeDirectory = new MenuFlyoutItem { Text = "Change directory to here", IsEnabled = false };
        pathMenu.Items.Add(changeDirectory);
        pathMenu.Opening += (_, _) => changeDirectory.IsEnabled = !disposed && Document.FilePath != null;
        changeDirectory.Click += (_, _) => Run(() => {
            if (!disposed && Document.FilePath is { } path && Path.GetDirectoryName(Path.GetFullPath(path)) is { } directory)
                window.ChangeDirectory(this, directory);
        });
        filePath.ContextFlyout = pathMenu;
        filePathHost.SizeChanged += (_, _) => UpdateFilePathText();
        normalStatus.SizeChanged += (_, _) => UpdateNormalStatusLayout();
        status.Children.Add(normalStatus); status.Children.Add(prompt); status.Children.Add(commandOutput);
        var position = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 5, VerticalAlignment = VerticalAlignment.Center };
        position.Children.Add(lineModeIcon); position.Children.Add(location); locationToggle.Content = position;
        status.Children.Add(locationToggle); SetColumn(locationToggle, 1);
        BuildStatusText();
        locationToggle.Click += (_, _) => {
            if (statusDragging || View == null) return;
            Run(() => View.LineMode(View.CurrentLineMode == VIEM_LINE_MODE_VISUAL ? VIEM_LINE_MODE_PHYSICAL_SOURCE : VIEM_LINE_MODE_VISUAL));
            FocusEditor();
        };
        // Listen even when a child control handles the press. Native controls
        // retain their clicks until movement becomes a pane drag.
        status.AddHandler(PointerPressedEvent, new PointerEventHandler((_, e) => {
            var point = e.GetCurrentPoint(window.PaneStack);
            if (!point.Properties.IsLeftButtonPressed) return;
            statusPointer = e.Pointer.PointerId; statusPress = statusLastPoint = point.Position; statusDragging = false; statusSelectingOutput = !outputScroll.Visibility.Equals(Visibility.Collapsed) && e.OriginalSource is DependencyObject source && IsOutputDescendant(source);
            statusPointerSource = e.OriginalSource as DependencyObject;
            statusDragTarget = this; statusDragNeedsSplit = !window.PaneStack.CanDragBar(this);
            // Empty background has no child control to retain pointer events.
            // Capture immediately so a first move beyond the bar still reaches
            // the threshold handler, without taking focus or starting a drag.
            // Text selection and native buttons keep their original press.
            if (!statusSelectingOutput && StatusResizeHoverAt(e.GetCurrentPoint(status).Position)) status.CapturePointer(e.Pointer);
        }), true);
        status.AddHandler(PointerMovedEvent, new PointerEventHandler((_, e) => {
            status.SetResizeHover(StatusResizeHoverAt(e.GetCurrentPoint(status).Position));
            if (statusPointer != e.Pointer.PointerId) return;
            var point = e.GetCurrentPoint(window.PaneStack);
            if (!point.Properties.IsLeftButtonPressed) { statusPointer = null; statusPointerSource = null; statusDragTarget = null; statusDragNeedsSplit = false; statusDragging = false; status.SetDragging(false); return; }
            if (!statusDragging)
            {
                if (Math.Abs(point.Position.Y - statusPress.Y) < 4 && Math.Abs(point.Position.X - statusPress.X) < 4) return;
                statusDragging = true;
                status.SetDragging(true);
                if (!statusSelectingOutput) {
                    bool captured = CaptureStatusDragPointer(e);
                    if (!captured) { statusPointer = null; statusPointerSource = null; statusDragTarget = null; statusDragNeedsSplit = false; statusDragging = false; status.SetDragging(false); return; }
                }
            }
            double delta = point.Position.Y - statusLastPoint.Y;
            statusLastPoint = point.Position; // No blocked overshoot survives a reversal.
            if (statusDragNeedsSplit)
            {
                double top = status.TransformToVisual(window.PaneStack).TransformPoint(new global::Windows.Foundation.Point()).Y;
                if (point.Position.Y < top)
                {
                    // Keep capture on this bar and move the new upper boundary.
                    // Admission failures are reported only once per gesture.
                    statusDragNeedsSplit = false;
                    Run(() => {
                        var added = window.SplitPane(this, Document, focusNewPane: false);
                        statusDragTarget = added;
                        double bottom = added.status.TransformToVisual(window.PaneStack)
                            .TransformPoint(new global::Windows.Foundation.Point(0, added.StatusBarHeight)).Y;
                        window.PaneStack.DragBar(added, point.Position.Y - bottom);
                    });
                }
            }
            else if (statusDragTarget is { } target) window.PaneStack.DragBar(target, delta);
            e.Handled = true;
        }), true);
        status.AddHandler(PointerReleasedEvent, new PointerEventHandler((_, e) => {
            if (statusPointer != e.Pointer.PointerId) return;
            bool dragged = statusDragging;
            statusDragTarget = null; statusDragNeedsSplit = false;
            statusPointer = null; statusPointerSource = null; statusDragging = false; status.SetDragging(false);
            if (!dragged) window.FocusPane(this, keepStatusFocus: statusSelectingOutput);
            if (!statusSelectingOutput) { status.ReleasePointerCapture(e.Pointer); if (dragged) e.Handled = true; }
        }), true);
        status.PointerCaptureLost += (_, e) => {
            if (ReferenceEquals(e.OriginalSource, status)) { statusPointer = null; statusPointerSource = null; statusDragging = false; statusDragTarget = null; statusDragNeedsSplit = false; status.SetDragging(false); }
        };
        status.PointerExited += (_, e) => { if (ReferenceEquals(e.OriginalSource, status)) status.SetResizeHover(false); };
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

    private bool hidesStatusBottomBorder;
    internal void SetStatusBottomEdge(bool bottom) {
        if (hidesStatusBottomBorder == bottom) return;
        hidesStatusBottomBorder = bottom;
        ApplyStatusTheme(force: true);
    }
    private double StatusBottomBorder => hidesStatusBottomBorder ? 0 : (statusStyle ?? preferences.StatusStyles.Status).Border.Bottom;
    internal float CommandVerticalInset => ((statusStyle ?? preferences.StatusStyles.Status).Padding.Top - (statusStyle ?? preferences.StatusStyles.Status).Padding.Bottom) / 2;

    private void ApplyStatusTheme(bool force = false)
    {
        var styles = preferences.StatusStyles;
        if (!force && statusStyle == styles.Status && commandStyle == styles.Command) return;
        if (statusStyle?.FontIdentity != styles.Status.FontIdentity) {
            ClearStatusText(); resizeHoverTextIdentity = null;
        }
        statusStyle = styles.Status; commandStyle = styles.Command;
        var style = statusStyle;
        status.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        status.BorderThickness = new(0);
        // The box owns padding and border. Chrome placement ignores margins
        // and alignment; command font changes cannot change container height.
        var inset = new Thickness(style.Padding.Left, style.Padding.Top + style.Border.Top,
            style.Padding.Right, style.Padding.Bottom + StatusBottomBorder);
        normalStatus.Margin = commandOutput.Margin = inset;
        locationToggle.Margin = inset;
        prompt.Margin = new(0, style.Border.Top, 0, StatusBottomBorder);
        var font = FontCatalog.Resolve(style.Family, style.Face) ?? (Family: "Segoe UI", Stretch: global::Windows.UI.Text.FontStretch.Normal);
        foreach (var label in new[] { mode, message, location, filePath, outputText, statusMeasure }) {
            label.FontFamily = new(font.Family); label.FontStretch = font.Stretch; label.FontSize = style.Size;
            label.FontWeight = new() { Weight = style.Weight }; label.FontStyle = style.Slant;
            label.CharacterSpacing = (int)Math.Clamp(Math.Round(style.Tracking * 1000d / style.Size), int.MinValue, int.MaxValue);
            label.Foreground = new SolidColorBrush(label == outputText ? style.Foreground : Microsoft.UI.Colors.Transparent);
        }
        using var statusFontProbe = style.Layout(CanvasDevice.GetSharedDevice(), "Mg");
        status.Height = Math.Max(1, Math.Ceiling(statusFontProbe.LayoutBounds.Height) + style.Padding.Vertical + style.Border.Top + StatusBottomBorder);
        outputText.Foreground = outputClose.Foreground = new SolidColorBrush(style.Foreground);
        outputClose.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        locationToggle.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        locationToggle.Foreground = new SolidColorBrush(style.Foreground);
        filePath.FontFamily = statusMeasure.FontFamily = mode.FontFamily;
        filePath.FontSize = statusMeasure.FontSize = mode.FontSize;
        mode.Width = Math.Ceiling(new[] { "NORMAL", "INSERT", "REPLACE", "VISUAL", "V-LINE", "V-BLOCK",
            "SELECTION", "SELECT", "S-LINE", "S-BLOCK", "COMMAND" }.Max(MeasureStatusText));
        UpdateFilePathText();
        UpdateStatusPresentation();
        statusBox.Invalidate(); statusTextCanvas.Invalidate(); prompt.Refresh();
    }

    internal void RefreshStatusFilePath()
    {
        var identity = (Document.FilePath, Environment.CurrentDirectory);
        if (filePathIdentity == identity) return;
        filePathIdentity = identity;
        fullStatusFilePath = StatusFilePath.Display(identity.Item1, identity.Item2);
        ToolTipService.SetToolTip(filePath, fullStatusFilePath);
        AutomationProperties.SetName(filePath, "File: " + fullStatusFilePath);
        UpdateNormalStatusLayout();
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
        filePath.Text = StatusFilePath.Fit(fullStatusFilePath, filePathHost.ActualWidth, MeasureStatusText);
        filePath.Width = Math.Max(0, filePathHost.ActualWidth);
        filePathTextWidth = MeasureStatusText(filePath.Text);
        statusTextCanvas.Invalidate();
    }

    private void UpdateNormalStatusLayout()
    {
        double available = Math.Max(0, normalStatus.ActualWidth);
        double messageWidth = Math.Min(Math.Ceiling(MeasureStatusText(message.Text)), available / 2);
        message.Width = messageWidth;
        filePathHost.Margin = new(0, 0, messageWidth > 0 ? 12 : 0, 0);
        bool showMode = mode.Width + 12 + MeasureStatusText(fullStatusFilePath)
            <= available - messageWidth - filePathHost.Margin.Right;
        mode.Visibility = showMode ? Visibility.Visible : Visibility.Collapsed;
        mode.Margin = new(0, 0, showMode ? 12 : 0, 0);
        UpdateFilePathText();
    }

    private void UpdateStatusPresentation(bool refreshPrompt = true)
    {
        RefreshStatusFilePath();
        var resizeText = (mode.Text, message.Text, mode.FontFamily.Source, mode.FontSize);
        if (resizeHoverTextIdentity != resizeText)
        {
            modeTextWidth = mode.Text.Length == 0 ? 0 : MeasureStatusText(mode.Text);
            messageTextWidth = message.Text.Length == 0 ? 0 : MeasureStatusText(message.Text);
            resizeHoverTextIdentity = resizeText;
        }
        message.Visibility = message.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        UpdateNormalStatusLayout();
        bool command = presentation.mode == VIEM_MODE_COMMAND_LINE;
        if (command) { output = null; outputText.Text = ""; outputTimer.Stop(); outputDeadline = 0; outputHadFocus = false; }
        bool showingOutput = !command && output != null;
        normalStatus.Visibility = command || showingOutput ? Visibility.Collapsed : Visibility.Visible;
        prompt.Visibility = command ? Visibility.Visible : Visibility.Collapsed;
        commandOutput.Visibility = showingOutput ? Visibility.Visible : Visibility.Collapsed;
        status.Visibility = command || showingOutput || message.Text.Length > 0 || preferences.ShowStatus ? Visibility.Visible : Visibility.Collapsed;
        statusTextCanvas.Invalidate();
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

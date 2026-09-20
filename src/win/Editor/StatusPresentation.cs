using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.System;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    internal const double StatusInset = 10;
    private readonly Grid normalStatus = new() { Margin = new(StatusInset, 0, 0, 0), ColumnSpacing = 12 };
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
    }

    private void BuildStatus()
    {
        status.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        status.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        foreach (var width in new[] { GridLength.Auto, GridLength.Auto, new GridLength(1, GridUnitType.Star) })
            normalStatus.ColumnDefinitions.Add(new() { Width = width });
        normalStatus.Children.Add(mode); normalStatus.Children.Add(format); SetColumn(format, 1);
        normalStatus.Children.Add(message); SetColumn(message, 2);
        status.Children.Add(normalStatus); status.Children.Add(prompt); status.Children.Add(commandOutput);
        location.Margin = new(0, 0, StatusInset, 0); status.Children.Add(location); SetColumn(location, 1);
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
        UpdateStatusPresentation();
    }

    private void UpdateStatusPresentation(bool refreshPrompt = true)
    {
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
    internal CommandPrompt PromptControl => prompt;
    internal string? CommandOutputText => output;
    internal Grid StatusControl => status;
    internal TextBlock OutputTextControl => outputText;
    internal ScrollViewer OutputScrollControl => outputScroll;
    internal Button OutputCloseControl => outputClose;
    internal TextBlock LocationControl => location;
    internal TextBlock ModeControl => mode;
#endif
}

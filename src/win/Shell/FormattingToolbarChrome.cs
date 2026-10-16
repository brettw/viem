using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Windows.Foundation;
using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private FormattingToolbar? formattingToolbar;
    private readonly ToggleButton toolbarToggle = new() { Width = 40, Height = 30, MinHeight = 0, Padding = new(0), BorderThickness = new(0), Visibility = Visibility.Collapsed, AllowFocusOnInteraction = false };
    // Markdown with Preview off has no toolbar; this title-bar button takes
    // the toolbar toggle's place and turns Preview back on.
    private readonly Button previewToggle = new() { Width = 40, Height = 30, MinHeight = 0, Padding = new(0), BorderThickness = new(0), Visibility = Visibility.Collapsed, AllowFocusOnInteraction = false };
    private bool closeAfterToolbarPopup;

    private void ConfigureFormattingToolbar()
    {
        // The same thin rectangle and three square buttons as the Mac title bar.
        var geometry = new GeometryGroup();
        geometry.Children.Add(new RectangleGeometry { Rect = new Rect(1.5, 3.5, 21, 11) });
        foreach (double x in new[] { 4d, 10d, 16d }) geometry.Children.Add(new RectangleGeometry { Rect = new Rect(x, 7, 4, 4) });
        var icon = new Microsoft.UI.Xaml.Shapes.Path { Data = geometry, StrokeThickness = 1, Width = 24, Height = 18 };
        icon.SetBinding(Microsoft.UI.Xaml.Shapes.Shape.StrokeProperty, new Microsoft.UI.Xaml.Data.Binding { Source = toolbarToggle, Path = new PropertyPath("Foreground") });
        toolbarToggle.Content = icon;
        AutomationProperties.SetName(toolbarToggle, "Formatting toolbar");
        toolbarToggle.Click += (_, _) => Safe(() => {
            if (ActivePane != null) preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN, toolbarToggle.IsChecked == true);
            SynchronizeFormattingToolbar(); RestoreEditorFocusAfterControl(); return Task.CompletedTask;
        });
        previewToggle.Content = FormattingToolbar.FormattedViewIcon(previewToggle);
        AutomationProperties.SetName(previewToggle, "Markdown Preview");
        ToolTipService.SetToolTip(previewToggle, "Markdown Preview");
        previewToggle.Click += (_, _) => Safe(() => {
            if (ActivePane is { View: { } view } pane) pane.Run(() => view.SetMarkdownPreview(true));
            SynchronizeFormattingToolbar(); RestoreEditorFocusAfterControl(); return Task.CompletedTask;
        });
    }

    private FormattingToolbar EnsureFormattingToolbar()
    {
        if (formattingToolbar != null) return formattingToolbar;
        using var startup = Diagnostics.StartupPerformance.Measure("window.formattingToolbar");
        var toolbar = new FormattingToolbar { Background = titleBar.Background };
        formattingToolbar = toolbar;
        toolbar.PopupsClosed += () => {
            if (closeAfterToolbarPopup && !toolbar.HasOpenPopup) DispatcherQueue.TryEnqueue(Close);
        };
        root.Children.Add(toolbar); Grid.SetRow(toolbar, 2);
        return toolbar;
    }

    // Only Markdown Preview has a formatting toolbar.
    private void PrepareFormattingToolbar(CoreDocument document)
    {
        bool visible = document.State.format == VIEM_FORMAT_MARKDOWN && preferences.ShowFormattingToolbar(VIEM_FORMAT_MARKDOWN);
        if (visible) EnsureFormattingToolbar();
        if (formattingToolbar != null) formattingToolbar.Visibility = visible ? Visibility.Visible : Visibility.Collapsed;
    }

    private void RestoreEditorFocusAfterControl()
    {
        var target = ActivePane;
        var timer = DispatcherQueue.CreateTimer();
        timer.Interval = TimeSpan.FromMilliseconds(16); timer.IsRepeating = false;
        timer.Tick += (_, _) => { if (!closed && ActivePane == target && target?.View is { Id: not 0 }) target.FocusEditor(); };
        timer.Start();
    }

    private void SynchronizeFormattingToolbar()
    {
        if (closed) return;
        var document = ActivePane?.Document;
        bool preview = document?.State.format == VIEM_FORMAT_MARKDOWN;
        bool visible = preview && preferences.ShowFormattingToolbar(VIEM_FORMAT_MARKDOWN);
        // Preview shows the toolbar toggle; literal Markdown offers Preview
        // in its place. Text and other Code show neither.
        bool literalMarkdown = !preview && document?.State.format == VIEM_FORMAT_CODE && document.IsMarkdownDocument;
        toolbarToggle.Visibility = preview ? Visibility.Visible : Visibility.Collapsed;
        previewToggle.Visibility = literalMarkdown ? Visibility.Visible : Visibility.Collapsed;
        toolbarToggle.IsChecked = visible;
        ToolTipService.SetToolTip(toolbarToggle, visible ? "Hide Formatting Toolbar" : "Show Formatting Toolbar");
        if (visible) EnsureFormattingToolbar();
        formattingToolbar?.Synchronize(ActivePane, visible);
    }
    internal void ExecuteFormattingAction(ToolbarAction action)
    {
        if (ActivePane is not { CanUseFormattingCommands: true } pane) return;
        var toolbar = EnsureFormattingToolbar();
        toolbar.Synchronize(pane, preferences.ShowFormattingToolbar(VIEM_FORMAT_MARKDOWN));
        toolbar.Execute(action);
    }
#if DEBUG
    internal FormattingToolbar Toolbar => formattingToolbar ?? throw new InvalidOperationException("The formatting toolbar has not been needed.");
    internal bool ToolbarCreated => formattingToolbar != null;
    internal ToggleButton ToolbarToggle => toolbarToggle;
    internal Button PreviewToggle => previewToggle;
    internal ToggleButton MenuToggle => menuToggle;
#endif
}

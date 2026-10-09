using System.Numerics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Hosting;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Markup;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Viem.Windows.Shell;
using Windows.ApplicationModel.DataTransfer;
using Windows.Foundation;
using Windows.System;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    private Border? linkPopup;
    private StackPanel? linkActions, linkEditor;
    private Button? linkDestinationButton;
    private TextBox? linkText;
    private ComboBox? linkDestination;
    private TextBlock? linkError;
    private Button? linkApply;
    private LinkContext? shownLink, dismissedLink;
    private bool editingLink;
    private bool insertingLink;
    private Rect linkAnchor;
    internal bool LinkPopupVisible => linkPopup?.Visibility == Visibility.Visible;
    internal bool LinkEditorVisible => LinkPopupVisible && editingLink;
    internal ComboBox? LinkDestinationControl => linkDestination;
    internal double LinkPopupWidth => linkPopup?.Width ?? 0;
    internal string LinkSummary => (linkDestinationButton?.Content as TextBlock)?.Text ?? "";
    internal string LinkTextValue { get => linkText?.Text ?? ""; set { EnsureLinkPopup(); linkText!.Text = value; } }
    internal string LinkDestinationValue { get => linkDestination?.Text ?? ""; set { EnsureLinkPopup(); linkDestination!.Text = value; } }

    private void EnsureLinkPopup()
    {
        if (linkPopup != null) return;
        linkActions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2 };
        linkDestinationButton = new Button { MinWidth = 0, Width = 208, Padding = new(6), HorizontalContentAlignment = HorizontalAlignment.Left,
            Content = new TextBlock { TextTrimming = TextTrimming.CharacterEllipsis } };
        AutomationProperties.SetName(linkDestinationButton, "Open link");
        linkDestinationButton.Click += (_, _) => Enqueue(OpenShownLink);
        linkActions.Children.Add(linkDestinationButton);
        Button Action(string label, string glyph, System.Action action)
        {
            var button = new Button { Width = 30, Height = 30, MinWidth = 0, MinHeight = 0, Padding = new(0), Content = new FontIcon { Glyph = glyph, FontSize = 14 } };
            AutomationProperties.SetName(button, label); ToolTipService.SetToolTip(button, label);
            button.Click += (_, _) => Run(action); linkActions.Children.Add(button); return button;
        }
        Action("Copy link", "\uE8C8", CopyShownLink);
        Action("Edit link", "\uE70F", () => BeginLinkEditor(animate: true));
        var remove = Action("Remove link", "", RemoveShownLink);
        remove.Content = UnlinkIcon(remove);
        linkText = new TextBox { Header = "Text", MinWidth = 0, IsSpellCheckEnabled = false, IsTextPredictionEnabled = false };
        linkDestination = new ComboBox { Header = "Destination", PlaceholderText = "https://…, file.md, or #heading", MinWidth = 0,
            IsEditable = true, IsTextSearchEnabled = false, HorizontalAlignment = HorizontalAlignment.Stretch,
            ItemTemplate = (DataTemplate)XamlReader.Load("<DataTemplate xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation'><TextBlock Text='{Binding Text}'/></DataTemplate>") };
        AutomationProperties.SetName(linkText, "Link text"); AutomationProperties.SetName(linkDestination, "Link destination");
        linkError = new TextBlock { TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed, MaxWidth = 320, FontSize = 12 };
        AutomationProperties.SetLiveSetting(linkError, AutomationLiveSetting.Polite);
        linkEditor = new StackPanel { Spacing = 8, Visibility = Visibility.Collapsed };
        linkEditor.Children.Add(linkText); linkEditor.Children.Add(linkDestination); linkEditor.Children.Add(linkError);
        var controls = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
        var cancel = new Button { Content = "Cancel", MinWidth = 60 };
        linkApply = new Button { Content = "Apply", MinWidth = 60 };
        cancel.Click += (_, _) => { DismissLinkPopup(); FocusEditor(); };
        linkApply.Click += (_, _) => ApplyLinkEditor();
        controls.Children.Add(cancel); controls.Children.Add(linkApply); linkEditor.Children.Add(controls);
        linkDestination.RegisterPropertyChangedCallback(ComboBox.TextProperty, (_, _) => {
            linkApply.IsEnabled = linkDestination.Text.Trim().Length > 0;
            linkError.Visibility = Visibility.Collapsed;
        });
        linkDestination.DropDownOpened += (_, _) => {
            if (View is not { } view || shownLink is not { } context) return;
            try {
                var choices = view.LinkHeadings(context);
                linkDestination.ItemsSource = choices.Headings;
                if (choices.Truncated) { linkError.Text = "Some headings are omitted from this list. You can also enter any #heading destination."; linkError.Visibility = Visibility.Visible; }
            } catch (Exception error) { linkDestination.ItemsSource = null; linkError.Text = error.Message; linkError.Visibility = Visibility.Visible; }
        };
        linkDestination.SelectionChanged += (_, _) => {
            if (linkDestination.SelectedItem is LinkHeading heading) linkDestination.Text = heading.Destination;
        };
        linkText.TextChanged += (_, _) => linkError.Visibility = Visibility.Collapsed;
        linkApply.IsEnabled = false;
        var contents = new Grid(); contents.Children.Add(linkActions); contents.Children.Add(linkEditor);
        linkPopup = new Border { Child = contents, Padding = new(5), BorderThickness = new(1), CornerRadius = new(8), Visibility = Visibility.Collapsed, Width = 340 };
        AutomationProperties.SetName(linkPopup, "Link");
        linkPopup.KeyDown += (_, e) => {
            if (linkDestination.IsDropDownOpen && e.Key is VirtualKey.Escape or VirtualKey.Enter) return;
            if (e.Key == VirtualKey.Escape) { e.Handled = true; DismissLinkPopup(); FocusEditor(); }
            else if (e.Key == VirtualKey.Enter && editingLink) { e.Handled = true; ApplyLinkEditor(); }
        };
        inputLayer.Children.Add(linkPopup);
    }

    private static Microsoft.UI.Xaml.Shapes.Path UnlinkIcon(Button owner)
    {
        var geometry = new GeometryGroup();
        geometry.Children.Add(new EllipseGeometry { Center = new(5, 8), RadiusX = 4, RadiusY = 3 });
        geometry.Children.Add(new EllipseGeometry { Center = new(13, 8), RadiusX = 4, RadiusY = 3 });
        geometry.Children.Add(new LineGeometry { StartPoint = new(1, 1), EndPoint = new(17, 15) });
        var icon = new Microsoft.UI.Xaml.Shapes.Path { Data = geometry, Width = 18, Height = 16, StrokeThickness = 1.4 };
        icon.SetBinding(Microsoft.UI.Xaml.Shapes.Shape.StrokeProperty, new Microsoft.UI.Xaml.Data.Binding { Source = owner, Path = new PropertyPath("Foreground") });
        return icon;
    }

    private static bool SameLinkContext(LinkContext? left, LinkContext? right) => left != null && right != null
        && CoreView.SameSelection(left.Selection, right.Selection) && left.Link == right.Link
        && left.Linked == right.Linked && left.CanExitLink == right.CanExitLink && left.CanRemoveSelection == right.CanRemoveSelection;

    internal void ShowInsertLink(bool insertOnly = false)
    {
        CaptureCommittedText();
        Enqueue(() => { PresentInsertLink(insertOnly); return Task.CompletedTask; });
    }

    private void PresentInsertLink(bool insertOnly)
    {
        DismissImagePopup(suppress: false);
        if (View is not { } view || !view.HasFormattingSelection || view.Composing) return;
        var context = view.LinkContext();
        if (insertOnly ? !context.CanInsert : !context.CanInsert && context.Link?.Editable != true)
        { SetMessage("Links can be inserted within a single paragraph of Markdown prose."); return; }
        insertingLink = insertOnly;
        shownLink = context; dismissedLink = null;
        linkAnchor = caretRect;
        if (linkAnchor.Height <= 0) return;
        EnsureLinkPopup();
        BeginLinkEditor(animate: LinkPopupVisible);
    }

    private void BeginLinkEditor(bool animate)
    {
        if (View is not { } view || shownLink is not { } context) return;
        view.ValidateLinkContext(context);
        if (!insertingLink && context.Link is { Editable: false }) return;
        EnsureLinkPopup();
        editingLink = true;
        linkDestination!.IsDropDownOpen = false;
        linkDestination.SelectedItem = null;
        linkDestination.ItemsSource = null;
        linkText!.Text = insertingLink ? context.Text : context.Link?.Text ?? context.Text;
        linkDestination.Text = insertingLink ? "" : context.Link?.Destination ?? "";
        linkApply!.IsEnabled = linkDestination.Text.Trim().Length > 0;
        linkError!.Text = ""; linkError.Visibility = Visibility.Collapsed;
        linkActions!.Visibility = Visibility.Collapsed; linkEditor!.Visibility = Visibility.Visible;
        linkPopup!.Visibility = Visibility.Visible;
        PositionLinkPopup();
        if (animate && new global::Windows.UI.ViewManagement.UISettings().AnimationsEnabled)
        {
            // Animate the same caret-anchored surface from compact controls into
            // its expanded form without moving focus back through the editor.
            var visual = ElementCompositionPreview.GetElementVisual(linkPopup);
            var scale = visual.Compositor.CreateVector3KeyFrameAnimation();
            scale.InsertKeyFrame(0, new Vector3(.96f, .65f, 1)); scale.InsertKeyFrame(1, Vector3.One);
            scale.Duration = TimeSpan.FromMilliseconds(170); visual.StartAnimation("Scale", scale);
            var opacity = visual.Compositor.CreateScalarKeyFrameAnimation();
            opacity.InsertKeyFrame(0, .6f); opacity.InsertKeyFrame(1, 1); opacity.Duration = scale.Duration;
            visual.StartAnimation("Opacity", opacity);
        }
        linkPopup.UpdateLayout();
        if (linkText.Text.Length == 0) { linkText.Focus(FocusState.Programmatic); linkText.SelectAll(); }
        else linkDestination.Focus(FocusState.Programmatic);
    }

    internal void ApplyLinkEditor()
    {
        if (!editingLink || shownLink is not { } context || View is not { } view) return;
        try
        {
            string target = linkDestination!.Text.Trim();
            if (target.Length == 0) throw new InvalidOperationException("Enter a destination for the link.");
            // Validate supported schemes before creating a link; relative paths
            // may be authored before a document is first saved.
            LinkDestination.Validate(target);
            view.EditLink(context, linkText!.Text.Length == 0 ? target : linkText.Text, target, insertOnly: insertingLink);
            DismissLinkPopup(suppress: false); FocusEditor(); Refresh();
        }
        catch (Exception error)
        {
            // A stale refresh dismisses the edit instead of retargeting typed
            // fields to another link. Other validation errors stay in the form.
            if (!LinkEditorVisible) { Report(error); return; }
            linkError!.Text = error.Message; linkError.Visibility = Visibility.Visible;
            PositionLinkPopup();
        }
    }

    internal void DismissLinkPopup(bool suppress = true)
    {
        if (suppress) dismissedLink = shownLink;
        editingLink = false; insertingLink = false; shownLink = null;
        if (linkDestination != null) {
            linkDestination.IsDropDownOpen = false;
            linkDestination.SelectedItem = null;
            linkDestination.ItemsSource = null;
        }
        if (linkPopup != null) linkPopup.Visibility = Visibility.Collapsed;
    }

    private void RefreshLinkPopup()
    {
        if (View is not { } view || !IsActive || !window.IsWindowActive || view.Composing || composing
            || !view.HasFormattingSelection || Document.State.format is not (VIEM_FORMAT_MARKDOWN or VIEM_FORMAT_MARKDOWN_SOURCE))
        { DismissLinkPopup(suppress: false); return; }
        if (!HasLinkInteractionFocus()) { DismissLinkPopup(suppress: false); return; }
        // A linked image has both contexts. Its object toolbar owns the passive
        // surface, while an explicitly opened link editor retains its draft.
        if (!editingLink && ImageEditorVisible)
        { DismissLinkPopup(suppress: false); return; }
        if (view.HasSelection && !editingLink) { dismissedLink = null; DismissLinkPopup(suppress: false); return; }
        var context = view.LinkContext();
        if (!editingLink && context.Link != null && view.ImageContext().Image != null)
        { DismissLinkPopup(suppress: false); return; }
        if (dismissedLink != null && !SameLinkContext(dismissedLink, context)) dismissedLink = null;
        if (editingLink)
        {
            if (!SameLinkContext(shownLink, context)) { DismissLinkPopup(suppress: false); FocusEditor(); return; }
            if (!TryLinkAnchor(context, out linkAnchor)) { DismissLinkPopup(); return; }
            PositionLinkPopup(); return;
        }
        // Selections seed the insert form; the passive toolbar follows a caret.
        if (view.HasSelection || context.Link == null || SameLinkContext(dismissedLink, context)
            || !TryLinkAnchor(context, out linkAnchor))
        { DismissLinkPopup(suppress: false); return; }
        EnsureLinkPopup(); shownLink = context; insertingLink = false;
        var label = (TextBlock)linkDestinationButton!.Content;
        label.Text = context.Link.Text.Length == 0 ? "empty" : context.Link.Text;
        label.FontStyle = context.Link.Text.Length == 0 ? global::Windows.UI.Text.FontStyle.Italic : global::Windows.UI.Text.FontStyle.Normal;
        AutomationProperties.SetName(linkDestinationButton, "Open " + context.Link.Destination);
        ToolTipService.SetToolTip(linkDestinationButton, context.Link.Destination);
        foreach (var button in linkActions!.Children.OfType<Button>().Skip(2)) button.IsEnabled = context.Link.Editable;
        linkActions.Visibility = Visibility.Visible; linkEditor!.Visibility = Visibility.Collapsed;
        linkPopup!.Visibility = Visibility.Visible; PositionLinkPopup();
    }

    private bool TryLinkAnchor(LinkContext context, out Rect anchor)
    {
        anchor = caretRect;
        if (context.Link is not { } link) return anchor.Height > 0;
        if (snapshot == null || snapshot.Info.identity.document_revision != context.Selection.document_revision) return false;
        // Use the first visible row of a wrapped link, taking its leftmost
        // cluster for bidi text. Geometry always belongs to the exact snapshot.
        var clusters = snapshot.Clusters.Where(c => c.text_end > link.Start && c.text_start < link.End)
            .Where(c => c.typographic_bounds.y + c.typographic_bounds.height > viewport.top
                && c.typographic_bounds.y < viewport.top + Canvas.ActualHeight
                && c.typographic_bounds.x + c.typographic_bounds.width > viewport.left
                && c.typographic_bounds.x < viewport.left + Canvas.ActualWidth).ToArray();
        if (clusters.Length == 0) return false;
        ulong row = clusters.Min(c => c.row_index);
        var firstRow = clusters.Where(c => c.row_index == row).ToArray();
        float top = firstRow.Min(c => c.typographic_bounds.y);
        double left = firstRow.Min(c => c.typographic_bounds.x) - viewport.left;
        double bottom = firstRow.Max(c => c.typographic_bounds.y + c.typographic_bounds.height) - viewport.top;
        anchor = new(left, top - viewport.top, 1, bottom - top + viewport.top);
        return true;
    }

    private bool HasLinkInteractionFocus()
    {
        if (LinkEditorVisible && linkDestination?.IsDropDownOpen == true) return true;
        if (XamlRoot == null) return false;
        for (var element = FocusManager.GetFocusedElement(XamlRoot) as DependencyObject; element != null; element = VisualTreeHelper.GetParent(element))
            if (element == input || element == linkPopup) return true;
        return false;
    }

    private void DismissLinkPopupIfFocusLeft()
    {
        if (!disposed && LinkPopupVisible && !HasLinkInteractionFocus()) DismissLinkPopup(suppress: false);
    }

    private void PositionLinkPopup()
    {
        if (linkPopup == null) return;
        linkPopup.Background = new SolidColorBrush(preferences.Theme.StatusBackground);
        linkPopup.BorderBrush = new SolidColorBrush(preferences.Theme.StatusForeground);
        linkError!.Foreground = new SolidColorBrush(preferences.Theme.StatusForeground);
        linkPopup.Padding = editingLink ? new(10) : new(5);
        double maximumWidth = Math.Min(360, Math.Max(180, Canvas.ActualWidth - 12));
        var label = (TextBlock)linkDestinationButton!.Content;
        label.MaxWidth = double.PositiveInfinity;
        label.Measure(new Size(double.PositiveInfinity, double.PositiveInfinity));
        linkPopup.Width = editingLink ? maximumWidth : Math.Min(maximumWidth, label.DesiredSize.Width + 12 + 108);
        linkDestinationButton.Width = Math.Max(12, linkPopup.Width - linkPopup.Padding.Left - linkPopup.Padding.Right - 98);
        ((TextBlock)linkDestinationButton.Content).MaxWidth = linkDestinationButton.Width - 12;
        linkPopup.Measure(new Size(linkPopup.Width, double.PositiveInfinity));
        double height = linkPopup.DesiredSize.Height;
        double y = linkAnchor.Bottom + 4;
        if (y + height > Canvas.ActualHeight) y = Math.Max(4, linkAnchor.Top - height - 4);
        Microsoft.UI.Xaml.Controls.Canvas.SetLeft(linkPopup, Math.Clamp(linkAnchor.Left, 4, Math.Max(4, Canvas.ActualWidth - linkPopup.Width - 4)));
        Microsoft.UI.Xaml.Controls.Canvas.SetTop(linkPopup, y);
    }

    private void CopyShownLink()
    {
        if (View is not { } view || shownLink is not { Link: { } link } context) return;
        view.ValidateLinkContext(context);
        var data = new DataPackage(); data.SetText(link.Destination); Clipboard.SetContent(data);
        DismissLinkPopup(); FocusEditor();
    }
    private void RemoveShownLink()
    {
        if (View is not { } view || shownLink is not { Link: { Editable: true } } context) return;
        view.EditLink(context, "", "", remove: true);
        DismissLinkPopup(suppress: false); FocusEditor(); Refresh();
    }
    private async Task OpenShownLink()
    {
        if (View is not { } view || shownLink is not { Link: { } link } context) return;
        view.ValidateLinkContext(context);
        var target = LinkDestination.Resolve(link.Destination, Document.FilePath);
        DismissLinkPopup();
        if (target.Kind == LinkDestinationKind.Fragment) { view.GoToLinkFragment(target.Fragment); FocusEditor(); }
        else if (target.Kind == LinkDestinationKind.Document) await window.OpenLinkedDocument(this, target);
        else if (!await Launcher.LaunchUriAsync(new Uri(target.Value))) throw new InvalidOperationException("The web browser could not open this link.");
    }
}

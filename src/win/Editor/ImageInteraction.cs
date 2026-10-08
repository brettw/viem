using System.Numerics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Hosting;
using Microsoft.UI.Xaml.Input;
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
    private Border? imagePopup;
    private StackPanel? imageActions, imageEditor;
    private Button? imageDestinationButton;
    private TextBox? imageText, imageDestination;
    private TextBlock? imageError;
    private Button? imageApply, imageReload;
    private ImageContext? shownImage, dismissedImage;
    private bool editingImage;
    private Rect imageAnchor;
    internal bool ImagePopupVisible => imagePopup?.Visibility == Visibility.Visible;
    internal bool ImageEditorVisible => ImagePopupVisible && editingImage;
    internal bool ImageReloadVisible => ImagePopupVisible && !editingImage && imageReload?.Visibility == Visibility.Visible;
    internal bool ImageReloadEnabled => imageReload?.IsEnabled == true;
    internal string ImageTextValue { get => imageText?.Text ?? ""; set { EnsureImagePopup(); imageText!.Text = value; } }
    internal string ImageDestinationValue { get => imageDestination?.Text ?? ""; set { EnsureImagePopup(); imageDestination!.Text = value; } }

    private void EnsureImagePopup()
    {
        if (imagePopup != null) return;
        imageActions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2 };
        imageDestinationButton = new Button { MinWidth = 0, Width = 208, Padding = new(6), HorizontalContentAlignment = HorizontalAlignment.Left,
            Content = new TextBlock { TextTrimming = TextTrimming.CharacterEllipsis } };
        AutomationProperties.SetName(imageDestinationButton, "Open image location");
        imageDestinationButton.Click += (_, _) => Enqueue(OpenShownImage);
        imageActions.Children.Add(imageDestinationButton);
        Button Action(string label, string glyph, System.Action action)
        {
            var button = new Button { Width = 30, Height = 30, MinWidth = 0, MinHeight = 0, Padding = new(0), Content = new FontIcon { Glyph = glyph, FontSize = 14 } };
            AutomationProperties.SetName(button, label); ToolTipService.SetToolTip(button, label);
            button.Click += (_, _) => Run(action); imageActions.Children.Add(button); return button;
        }
        Action("Copy image location", "\uE8C8", CopyShownImage);
        Action("Edit image", "\uE70F", () => BeginImageEditor(animate: true));
        imageReload = Action("Reload image", "\uE72C", ReloadShownImage);
        Action("Delete image", "\uE74D", RemoveShownImage);
        imageText = new TextBox { Header = "Alt text", MinWidth = 0, IsSpellCheckEnabled = false, IsTextPredictionEnabled = false };
        imageDestination = new TextBox { Header = "Location", PlaceholderText = "image.png or https://…", MinWidth = 0, IsSpellCheckEnabled = false, IsTextPredictionEnabled = false };
        AutomationProperties.SetName(imageText, "Image alt text"); AutomationProperties.SetName(imageDestination, "Image location");
        imageError = new TextBlock { TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed, MaxWidth = 320, FontSize = 12 };
        AutomationProperties.SetLiveSetting(imageError, AutomationLiveSetting.Polite);
        imageEditor = new StackPanel { Spacing = 8, Visibility = Visibility.Collapsed };
        imageEditor.Children.Add(imageText); imageEditor.Children.Add(imageDestination); imageEditor.Children.Add(imageError);
        var controls = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
        var cancel = new Button { Content = "Cancel", MinWidth = 60 };
        imageApply = new Button { Content = "Apply", MinWidth = 60 };
        cancel.Click += (_, _) => { DismissImagePopup(); FocusEditor(); };
        imageApply.Click += (_, _) => ApplyImageEditor();
        controls.Children.Add(cancel); controls.Children.Add(imageApply); imageEditor.Children.Add(controls);
        imageDestination.TextChanged += (_, _) => {
            imageApply.IsEnabled = imageDestination.Text.Trim().Length > 0;
            imageError.Visibility = Visibility.Collapsed;
        };
        imageText.TextChanged += (_, _) => imageError.Visibility = Visibility.Collapsed;
        imageApply.IsEnabled = false;
        var contents = new Grid(); contents.Children.Add(imageActions); contents.Children.Add(imageEditor);
        imagePopup = new Border { Child = contents, Padding = new(5), BorderThickness = new(1), CornerRadius = new(8), Visibility = Visibility.Collapsed, Width = 340 };
        AutomationProperties.SetName(imagePopup, "Image");
        imagePopup.KeyDown += (_, e) => {
            if (e.Key == VirtualKey.Escape) { e.Handled = true; DismissImagePopup(); FocusEditor(); }
            else if (e.Key == VirtualKey.Enter && editingImage) { e.Handled = true; ApplyImageEditor(); }
        };
        inputLayer.Children.Add(imagePopup);
    }

    private static bool SameImageContext(ImageContext? left, ImageContext? right) => left != null && right != null
        && CoreView.SameSelection(left.Selection, right.Selection) && left.Image == right.Image;

    internal void ShowInsertImage()
    {
        CaptureCommittedText();
        Enqueue(() => { PresentInsertImage(); return Task.CompletedTask; });
    }

    private void PresentInsertImage()
    {
        DismissLinkPopup(suppress: false);
        if (View is not { } view || !view.HasFormattingSelection || view.Composing) return;
        var context = view.ImageContext();
        if (!context.CanInsert && context.Image?.Editable != true)
        { SetMessage("Images can be inserted within a single paragraph of Markdown prose."); return; }
        shownImage = context; dismissedImage = null;
        imageAnchor = caretRect;
        if (imageAnchor.Height <= 0) return;
        EnsureImagePopup();
        BeginImageEditor(animate: ImagePopupVisible);
    }

    private void BeginImageEditor(bool animate)
    {
        if (View is not { } view || shownImage is not { } context) return;
        view.ValidateImageContext(context);
        if (context.Image is { Editable: false }) return;
        EnsureImagePopup();
        editingImage = true;
        imageText!.Text = context.Image?.Text ?? context.Text;
        imageDestination!.Text = context.Image?.Destination ?? "";
        imageApply!.IsEnabled = imageDestination.Text.Trim().Length > 0;
        imageError!.Text = ""; imageError.Visibility = Visibility.Collapsed;
        imageActions!.Visibility = Visibility.Collapsed; imageEditor!.Visibility = Visibility.Visible;
        imagePopup!.Visibility = Visibility.Visible;
        PositionImagePopup();
        if (animate && new global::Windows.UI.ViewManagement.UISettings().AnimationsEnabled)
        {
            // Animate the same caret-anchored surface from compact controls into
            // its expanded form without moving focus back through the editor.
            var visual = ElementCompositionPreview.GetElementVisual(imagePopup);
            var scale = visual.Compositor.CreateVector3KeyFrameAnimation();
            scale.InsertKeyFrame(0, new Vector3(.96f, .65f, 1)); scale.InsertKeyFrame(1, Vector3.One);
            scale.Duration = TimeSpan.FromMilliseconds(170); visual.StartAnimation("Scale", scale);
            var opacity = visual.Compositor.CreateScalarKeyFrameAnimation();
            opacity.InsertKeyFrame(0, .6f); opacity.InsertKeyFrame(1, 1); opacity.Duration = scale.Duration;
            visual.StartAnimation("Opacity", opacity);
        }
        imagePopup.UpdateLayout();
        var focus = imageDestination;
        focus.Focus(FocusState.Programmatic); focus.SelectAll();
    }

    internal void ApplyImageEditor()
    {
        if (!editingImage || shownImage is not { } context || View is not { } view) return;
        try
        {
            string target = imageDestination!.Text.Trim();
            if (target.Length == 0) throw new InvalidOperationException("Enter a location for the image.");
            // Validate supported schemes before creating an image; relative paths
            // may be authored before a document is first saved.
            ImageLocation.Validate(target);
            view.EditImage(context, imageText!.Text, target);
            DismissImagePopup(suppress: false); FocusEditor(); Refresh();
        }
        catch (Exception error)
        {
            // A stale refresh dismisses the edit instead of retargeting typed
            // fields to another image. Other validation errors stay in the form.
            if (!ImageEditorVisible) { Report(error); return; }
            imageError!.Text = error.Message; imageError.Visibility = Visibility.Visible;
            PositionImagePopup();
        }
    }

    internal void DismissImagePopup(bool suppress = true)
    {
        if (suppress) dismissedImage = shownImage;
        editingImage = false; shownImage = null;
        if (imagePopup != null) imagePopup.Visibility = Visibility.Collapsed;
    }

    private void RefreshImagePopup()
    {
        if (View is not { } view || !IsActive || !window.IsWindowActive || view.Composing || composing
            || !view.HasFormattingSelection || Document.State.format is not (VIEM_FORMAT_MARKDOWN or VIEM_FORMAT_MARKDOWN_SOURCE))
        { DismissImagePopup(suppress: false); return; }
        if (!HasImageInteractionFocus()) { DismissImagePopup(suppress: false); return; }
        if (!editingImage && LinkEditorVisible) { DismissImagePopup(suppress: false); return; }
        var context = view.ImageContext();
        if (dismissedImage != null && !SameImageContext(dismissedImage, context)) dismissedImage = null;
        if (editingImage)
        {
            if (!SameImageContext(shownImage, context)) { DismissImagePopup(suppress: false); FocusEditor(); return; }
            if (!TryImageAnchor(context, out imageAnchor)) { DismissImagePopup(); return; }
            PositionImagePopup(); return;
        }
        // A native image selection is one atomic object. Other selections
        // seed insertion rather than revealing a passive object toolbar.
        bool selectedImage = context.Image is { } selected && context.Selection.text_start == selected.Start && context.Selection.text_end == selected.End;
        if (view.HasSelection && !selectedImage || context.Image == null || SameImageContext(dismissedImage, context)
            || !TryImageAnchor(context, out imageAnchor))
        { DismissImagePopup(suppress: false); return; }
        DismissLinkPopup(suppress: false);
        EnsureImagePopup(); shownImage = context;
        ((TextBlock)imageDestinationButton!.Content).Text = context.Image.Destination;
        AutomationProperties.SetName(imageDestinationButton, "Open " + context.Image.Destination);
        ToolTipService.SetToolTip(imageDestinationButton, context.Image.Destination);
        foreach (var button in imageActions!.Children.OfType<Button>().Skip(2)) button.IsEnabled = context.Image.Editable;
        imageReload!.IsEnabled = ImageLocation.LocalPreviewPath(context.Image.Destination, Document.FilePath) != null;
        imageActions.Visibility = Visibility.Visible; imageEditor!.Visibility = Visibility.Collapsed;
        imagePopup!.Visibility = Visibility.Visible; PositionImagePopup();
    }

    private bool TryImageAnchor(ImageContext context, out Rect anchor)
    {
        anchor = caretRect;
        if (context.Image is not { } image) return anchor.Height > 0;
        if (snapshot == null || snapshot.Info.identity.document_revision != context.Selection.document_revision) return false;
        // Use the first visible row of a wrapped image, taking its leftmost
        // cluster for bidi text. Geometry always belongs to the exact snapshot.
        var clusters = snapshot.Clusters.Where(c => c.text_end > image.Start && c.text_start < image.End)
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

    private bool HasImageInteractionFocus()
    {
        if (XamlRoot == null) return false;
        for (var element = FocusManager.GetFocusedElement(XamlRoot) as DependencyObject; element != null; element = VisualTreeHelper.GetParent(element))
            if (element == input || element == imagePopup) return true;
        return false;
    }

    private void DismissImagePopupIfFocusLeft()
    {
        if (!disposed && ImagePopupVisible && !HasImageInteractionFocus()) DismissImagePopup(suppress: false);
    }

    private void PositionImagePopup()
    {
        if (imagePopup == null) return;
        imagePopup.Background = new SolidColorBrush(preferences.Theme.StatusBackground);
        imagePopup.BorderBrush = new SolidColorBrush(preferences.Theme.StatusForeground);
        imageError!.Foreground = new SolidColorBrush(preferences.Theme.StatusForeground);
        imagePopup.Padding = editingImage ? new(10) : new(5);
        imageReload!.Visibility = Document.State.format == VIEM_FORMAT_MARKDOWN ? Visibility.Visible : Visibility.Collapsed;
        imagePopup.Width = Math.Min(360, Math.Max(180, Canvas.ActualWidth - 12));
        double actionsWidth = imageActions!.Children.OfType<Button>().Skip(1).Count(button => button.Visibility == Visibility.Visible) * 32 + 2;
        imageDestinationButton!.Width = Math.Max(48, imagePopup.Width - imagePopup.Padding.Left - imagePopup.Padding.Right - actionsWidth);
        ((TextBlock)imageDestinationButton.Content).MaxWidth = imageDestinationButton.Width - 12;
        imagePopup.Measure(new Size(imagePopup.Width, double.PositiveInfinity));
        double height = imagePopup.DesiredSize.Height;
        double y = imageAnchor.Bottom + 4;
        if (y + height > Canvas.ActualHeight) y = Math.Max(4, imageAnchor.Top - height - 4);
        Microsoft.UI.Xaml.Controls.Canvas.SetLeft(imagePopup, Math.Clamp(imageAnchor.Left, 4, Math.Max(4, Canvas.ActualWidth - imagePopup.Width - 4)));
        Microsoft.UI.Xaml.Controls.Canvas.SetTop(imagePopup, y);
    }

    private void CopyShownImage()
    {
        if (View is not { } view || shownImage is not { Image: { } image } context) return;
        view.ValidateImageContext(context);
        var data = new DataPackage(); data.SetText(image.Destination); Clipboard.SetContent(data);
        DismissImagePopup(); FocusEditor();
    }
    private void RemoveShownImage()
    {
        if (View is not { } view || shownImage is not { Image: { Editable: true } } context) return;
        view.EditImage(context, "", "", remove: true);
        DismissImagePopup(suppress: false); FocusEditor(); Refresh();
    }
    private async Task OpenShownImage()
    {
        if (View is not { } view || shownImage is not { Image: { } image } context) return;
        view.ValidateImageContext(context);
        var target = ImageLocation.Resolve(image.Destination, Document.FilePath);
        if (target.IsFile)
        {
            string? local = ImageLocation.LocalPreviewPath(image.Destination, Document.FilePath);
            if (local == null || !await Rendering.LocalImagePreview.CanOpenFile(local))
                throw new InvalidOperationException("Only local raster image files can be opened as images.");
            // An awaited read must not turn a now-stale popup into an external action.
            view.ValidateImageContext(context);
        }
        DismissImagePopup();
        if (!await Launcher.LaunchUriAsync(target)) throw new InvalidOperationException("The image location could not be opened.");
    }
}

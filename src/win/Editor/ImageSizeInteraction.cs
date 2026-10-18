using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Viem.Windows.Core;
using Viem.Windows.Rendering;
using Viem.Windows.Shell;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    private readonly ImageSizeDraft imageSize = new();
    private Button? imageSizeDisclosure, imageUseNatural;
    private ToggleButton? imageSizeLock;
    private TextBox? imageWidth, imageHeight;
    private StackPanel? imageSizeFields;
    private bool updatingImageSize, imageSizeProbeRunning;
    private string? imageSizeDestination;
    private CancellationTokenSource? imageSizeProbeCancellation;
    private ImageSizeProbe? imageSizeProbe;
    private sealed record ImageSizeProbe(object Draft, ImageContext Context, CoreView Owner, string Destination,
        string? DocumentPath, string Path, CancellationTokenSource Cancellation);

    internal string ImageApplyLabel => imageApply?.Content as string ?? "";
    internal bool ImageSizeExpanded => imageSize.Expanded;
    internal bool ImageSizeLocked => imageSize.Locked;
    internal bool ImageSizeNaturalKnown => imageSize.Natural != null;
    internal string ImageWidthValue { get => imageWidth?.Text ?? ""; set => imageWidth!.Text = value; }
    internal string ImageHeightValue { get => imageHeight?.Text ?? ""; set => imageHeight!.Text = value; }

    private void BuildImageSizeControls(StackPanel editor)
    {
        imageSizeDisclosure = new Button { HorizontalAlignment = HorizontalAlignment.Left, Padding = new(0),
            MinHeight = 24, MinWidth = 0, Background = null, BorderThickness = new(0) };
        AutomationProperties.SetName(imageSizeDisclosure, "Specify image size");
        imageSizeDisclosure.Click += (_, _) => SetImageSizeExpanded(!imageSize.Expanded);
        editor.Children.Add(imageSizeDisclosure);
        imageWidth = new TextBox { Width = 64, MinWidth = 0, Padding = new(5, 3, 5, 3),
            IsSpellCheckEnabled = false, IsTextPredictionEnabled = false };
        imageHeight = new TextBox { Width = 64, MinWidth = 0, Padding = new(5, 3, 5, 3),
            IsSpellCheckEnabled = false, IsTextPredictionEnabled = false };
        AutomationProperties.SetName(imageWidth, "Image width in pixels");
        AutomationProperties.SetName(imageHeight, "Image height in pixels");
        imageSizeLock = new ToggleButton { Width = 32, Height = 28, MinWidth = 0, Padding = new(0) };
        imageSizeLock.Checked += (_, _) => { if (!updatingImageSize) SetImageSizeLocked(true); };
        imageSizeLock.Unchecked += (_, _) => { if (!updatingImageSize) SetImageSizeLocked(false); };
        var dimensions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 5 };
        TextBlock Label(string label) => new() { Text = label, VerticalAlignment = VerticalAlignment.Center };
        dimensions.Children.Add(Label("Width")); dimensions.Children.Add(imageWidth);
        dimensions.Children.Add(Label("×")); dimensions.Children.Add(Label("Height"));
        dimensions.Children.Add(imageHeight); dimensions.Children.Add(imageSizeLock);
        imageUseNatural = new Button { Content = "Use image size", HorizontalAlignment = HorizontalAlignment.Left };
        imageUseNatural.Click += (_, _) => UseNaturalImageSize();
        imageSizeFields = new StackPanel { Spacing = 8, Visibility = Visibility.Collapsed };
        imageSizeFields.Children.Add(dimensions); imageSizeFields.Children.Add(imageUseNatural);
        editor.Children.Add(imageSizeFields);
        imageWidth.TextChanged += (_, _) => {
            if (updatingImageSize) return;
            imageSize.EditWidth(imageWidth.Text); UpdateImageSizeControls();
        };
        imageHeight.TextChanged += (_, _) => {
            if (updatingImageSize) return;
            imageSize.EditHeight(imageHeight.Text); UpdateImageSizeControls();
        };
    }

    internal void SetImageSizeExpanded(bool expanded)
    {
        imageSize.SetExpanded(expanded); UpdateImageSizeControls();
        if (expanded) RefreshImageSizeNatural();
        PositionImagePopup();
    }

    internal void SetImageSizeLocked(bool locked)
    {
        imageSize.SetLocked(locked); UpdateImageSizeControls();
    }

    internal void UseNaturalImageSize()
    {
        imageSize.UseNatural(); UpdateImageSizeControls();
    }

    private void UpdateImageSizeControls()
    {
        if (imageWidth == null) return;
        updatingImageSize = true;
        imageSizeDisclosure!.Content = (imageSize.Expanded ? "⌄" : "›") + " Specify size";
        AutomationProperties.SetHelpText(imageSizeDisclosure, imageSize.Expanded ? "Expanded" : "Collapsed");
        imageSizeFields!.Visibility = imageSize.Expanded ? Visibility.Visible : Visibility.Collapsed;
        if (imageWidth.Text != imageSize.Width) imageWidth.Text = imageSize.Width;
        if (imageHeight!.Text != imageSize.Height) imageHeight.Text = imageSize.Height;
        imageSizeLock!.IsChecked = imageSize.Locked;
        imageSizeLock.Content = new FontIcon { Glyph = imageSize.Locked ? "\uE72E" : "\uE785", FontSize = 16 };
        string action = imageSize.Locked ? "Unlock image aspect ratio" : "Lock image aspect ratio";
        AutomationProperties.SetName(imageSizeLock, action); ToolTipService.SetToolTip(imageSizeLock, action);
        imageSizeLock.IsEnabled = imageSize.Natural != null;
        imageUseNatural!.IsEnabled = imageSize.Natural != null;
        updatingImageSize = false;
        if (imageError != null) imageError.Visibility = Visibility.Collapsed;
    }

    private void CancelImageSizeProbe()
    {
        imageSizeProbeCancellation?.Cancel(); imageSizeProbeCancellation = null;
        imageSizeProbe?.Cancellation.Dispose();
        imageSizeProbe = null; imageSizeDestination = null;
    }

    private void RefreshImageSizeNatural()
    {
        if (!ImageEditorVisible || imageEditDraft is not { } draft || View is not { } view || shownImage is not { } context) return;
        string destination = ImageDraftDestination.Trim();
        if (destination == imageSizeDestination) return;
        CancelImageSizeProbe();
        imageSize.ClearNatural(); UpdateImageSizeControls();
        if (!imageSize.Expanded) return;
        imageSizeDestination = destination;
        string? path = ImageLocation.LocalPreviewPath(destination, Document.FilePath);
        if (path == null) return;
        // Known dimensions have already passed the same local raster admission
        // rules. Reuse them without rereading a stable or rejected resource.
        if (view.Provider.ImageDimensions(destination) is { } natural)
        {
            imageSize.SetNatural(natural.Width, natural.Height); UpdateImageSizeControls(); return;
        }
        if (view.Provider.ImageState(destination) is InlineImageState.Broken or InlineImageState.Limited) return;
        var cancellation = new CancellationTokenSource(); imageSizeProbeCancellation = cancellation;
        imageSizeProbe = new(draft, context, view, destination, Document.FilePath, path, cancellation);
        if (!imageSizeProbeRunning) _ = RunImageSizeProbes();
    }

    private async Task RunImageSizeProbes()
    {
        imageSizeProbeRunning = true;
        try
        {
            // At most one bounded header read is active; typing coalesces to
            // the newest destination, and all obsolete results are discarded.
            while (imageSizeProbe is { } request)
            {
                imageSizeProbe = null;
                var cancellation = request.Cancellation.Token;
                try
                {
                    await Task.Delay(150, cancellation);
                    var result = await Task.Run(() => LocalImagePreview.ProbeDimensions(request.Path, cancellation), cancellation);
                    if (disposed || cancellation.IsCancellationRequested || !ImageEditorVisible
                        || !ReferenceEquals(imageEditDraft, request.Draft) || !ReferenceEquals(View, request.Owner)
                        || !SameImageContext(shownImage, request.Context) || ImageDraftDestination.Trim() != request.Destination
                        || !string.Equals(Document.FilePath, request.DocumentPath, StringComparison.OrdinalIgnoreCase)) continue;
                    request.Owner.ValidateImageContext(request.Context);
                    if (result.Status == LocalImagePreviewStatus.Ready)
                    {
                        imageSize.SetNatural(result.Width, result.Height); UpdateImageSizeControls();
                    }
                }
                catch (OperationCanceledException) { }
                catch (InvalidOperationException) { }
                catch (Viem.Windows.Interop.CoreException) { }
                finally
                {
                    if (ReferenceEquals(imageSizeProbeCancellation, request.Cancellation)) imageSizeProbeCancellation = null;
                    request.Cancellation.Dispose();
                }
            }
        }
        finally { imageSizeProbeRunning = false; }
    }
}

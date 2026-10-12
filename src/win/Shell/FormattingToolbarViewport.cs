using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.Foundation;

namespace Viem.Windows.Shell;

/// Keeps native controls at their natural width while clipping overflow.
/// There is no scroll owner, so focus, wheel, touch and automation cannot pan it.
internal sealed partial class FormattingToolbarViewport : Panel
{
    protected override Size MeasureOverride(Size availableSize)
    {
        double width = 0, height = 0;
        foreach (var child in Children)
        {
            child.Measure(new Size(double.PositiveInfinity, availableSize.Height));
            width = Math.Max(width, child.DesiredSize.Width);
            height = Math.Max(height, child.DesiredSize.Height);
        }
        return new Size(Math.Min(width, availableSize.Width), Math.Min(height, availableSize.Height));
    }

    protected override Size ArrangeOverride(Size finalSize)
    {
        foreach (var child in Children)
            child.Arrange(new Rect(0, 0, Math.Max(finalSize.Width, child.DesiredSize.Width), finalSize.Height));
        Clip = new RectangleGeometry { Rect = new Rect(0, 0, finalSize.Width, finalSize.Height) };
        return finalSize;
    }
}

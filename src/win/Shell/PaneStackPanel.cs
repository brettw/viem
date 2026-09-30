using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Editor;
using Windows.Foundation;

namespace Viem.Windows.Shell;

/// Stacked editor gaps may collapse, but status bars retain their full height.
internal sealed class PaneStackPanel : Panel
{
    private double[] heights = [];
    private double[] Minimums => Children.Cast<EditorPane>().Select(p => p.StatusBarHeight).ToArray();

    internal void Rebuild(IReadOnlyList<EditorPane> panes, int? splitIndex = null)
    {
        if (splitIndex is { } index && heights.Length + 1 == panes.Count)
        {
            var next = heights.ToList();
            double gap = Math.Max(0, next[index] - panes[index].StatusBarHeight - panes[index + 1].StatusBarHeight) / 2;
            next[index] = panes[index].StatusBarHeight + gap;
            next.Insert(index + 1, panes[index + 1].StatusBarHeight + gap);
            heights = next.ToArray();
        }
        Children.Clear();
        foreach (var pane in panes) Children.Add(pane);
        if (heights.Length != panes.Count) Equalize();
        InvalidateMeasure();
    }

    private void Fit(double total)
    {
        var minimums = Minimums;
        if (minimums.Length == 0) { heights = []; return; }
        double required = minimums.Sum(), available = Math.Max(0, total - required);
        if (heights.Length != minimums.Length) heights = minimums.ToArray();
        if (Math.Abs(heights.Sum() - Math.Max(total, required)) < .001
            && heights.Zip(minimums).All(pair => pair.First >= pair.Second)) return;
        var gaps = heights.Zip(minimums).Select(pair => Math.Max(0, pair.First - pair.Second)).ToArray();
        double gapTotal = gaps.Sum();
        heights = minimums.Select((minimum, index) => minimum +
            (gapTotal > 0 ? available * gaps[index] / gapTotal : available / minimums.Length)).ToArray();
    }

    internal void Equalize()
    {
        var minimums = Minimums;
        double gap = minimums.Length == 0 ? 0 : Math.Max(0, ActualHeight - minimums.Sum()) / minimums.Length;
        heights = minimums.Select(minimum => minimum + gap).ToArray();
        InvalidateMeasure();
    }

    /// No collected group is retained: each incremental delta pushes only the
    /// bars encountered in that direction, including after a blocked delta.
    internal void DragBar(EditorPane pane, double delta)
    {
        int index = Children.IndexOf(pane);
        if (index < 0 || index == Children.Count - 1 || !double.IsFinite(delta)) return;
        Fit(ActualHeight);
        var minimums = Minimums;
        double remaining = Math.Abs(delta);
        int receiver = delta > 0 ? index : index + 1;
        var donors = delta > 0 ? Enumerable.Range(index + 1, heights.Length - index - 1)
            : Enumerable.Range(0, index + 1).Reverse();
        foreach (int donor in donors)
        {
            double change = Math.Min(remaining, Math.Max(0, heights[donor] - minimums[donor]));
            heights[donor] -= change; heights[receiver] += change; remaining -= change;
            if (remaining <= 0) break;
        }
        InvalidateMeasure();
        UpdateLayout();
    }

    internal void ResizePane(EditorPane pane, double textHeight)
    {
        int index = Children.IndexOf(pane);
        if (index < 0 || Children.Count < 2) return;
        Fit(ActualHeight);
        var minimums = Minimums;
        double target = Math.Clamp(textHeight + minimums[index], minimums[index],
            Math.Max(minimums[index], heights.Sum() - minimums.Sum() + minimums[index]));
        double remaining = target - heights[index];
        for (int distance = 1; distance < heights.Length && remaining != 0; distance++)
            foreach (int donor in new[] { index + distance, index - distance })
            {
                if (donor < 0 || donor >= heights.Length) continue;
                double change = remaining > 0 ? Math.Min(remaining, Math.Max(0, heights[donor] - minimums[donor])) : remaining;
                heights[donor] -= change; heights[index] += change; remaining -= change;
            }
        InvalidateMeasure();
        UpdateLayout();
    }

    protected override Size MeasureOverride(Size availableSize)
    {
        double height = double.IsFinite(availableSize.Height) ? availableSize.Height : ActualHeight;
        Fit(height);
        for (int index = 0; index < Children.Count; index++) Children[index].Measure(new(availableSize.Width, heights[index]));
        return new(double.IsFinite(availableSize.Width) ? availableSize.Width : 0, height);
    }

    protected override Size ArrangeOverride(Size finalSize)
    {
        Fit(finalSize.Height);
        double offset = 0;
        for (int index = 0; index < Children.Count; index++)
        {
            Children[index].Arrange(new Rect(0, offset, finalSize.Width, heights[index]));
            offset += heights[index];
        }
        return finalSize;
    }
}

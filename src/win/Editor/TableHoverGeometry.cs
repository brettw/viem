using Windows.Foundation;

namespace Viem.Windows.Editor;

internal enum TableHoverKind { Column, LeftRow, RightRow }

/// <summary>Table hover bands use document edges, never viewport clipping edges.</summary>
internal static class TableHoverGeometry
{
    internal static TableHoverKind? Hit(Point point, Rect cell, Rect table, Rect viewport)
    {
        if (point.X < viewport.Left || point.Y < viewport.Top
            || point.X >= viewport.Right || point.Y >= viewport.Bottom) return null;
        bool top = point.Y >= table.Top - 3 && point.Y <= table.Top + 1
            && point.X >= table.Left && point.X <= table.Right;
        if (top) {
            bool ownsColumn = point.X >= cell.Left && (point.X < cell.Right
                || point.X == cell.Right && cell.Right == table.Right);
            return ownsColumn ? TableHoverKind.Column : null;
        }
        bool ownsRow = point.Y >= cell.Top && (point.Y < cell.Bottom
            || point.Y == cell.Bottom && cell.Bottom == table.Bottom);
        if (!ownsRow) return null;
        if (point.X >= table.Left - 20 && point.X <= table.Left + 1) return TableHoverKind.LeftRow;
        if (point.X >= table.Right - 1 && point.X <= table.Right + 20) return TableHoverKind.RightRow;
        return null;
    }

    internal static Rect Activation(TableHoverKind kind, Rect cell, Rect table) => kind switch {
        TableHoverKind.Column => new(cell.X, table.Y - 3, cell.Width, 4),
        TableHoverKind.LeftRow => new(table.X - 20, cell.Y, 21, cell.Height),
        _ => new(table.Right - 1, cell.Y, 21, cell.Height),
    };

    internal static Rect Placement(TableHoverKind kind, Rect cell, Rect table, Size widget, Size viewport)
    {
        bool column = kind == TableHoverKind.Column;
        double left = table.Left - widget.Width - 4, right = table.Right + 4;
        double x = column ? cell.X + cell.Width / 2 - widget.Width / 2
            : kind == TableHoverKind.RightRow ? right : left;
        bool Fits(double candidate) => candidate >= 0 && candidate + widget.Width <= viewport.Width;
        if (!column && !Fits(x)) {
            double opposite = kind == TableHoverKind.RightRow ? left : right;
            if (Fits(opposite)) x = opposite;
        }
        double y = column ? table.Y - widget.Height - 4 : cell.Y + cell.Height / 2 - widget.Height / 2;
        if (y < 0) y = cell.Bottom + 4;
        return new(Math.Clamp(x, 0, Math.Max(0, viewport.Width - widget.Width)),
            Math.Clamp(y, 0, Math.Max(0, viewport.Height - widget.Height)), widget.Width, widget.Height);
    }

    internal static Rect RetentionPath(Rect widget, Rect activation) => new(
        Math.Min(widget.Left, activation.Left) - 3, Math.Min(widget.Top, activation.Top) - 3,
        Math.Max(widget.Right, activation.Right) - Math.Min(widget.Left, activation.Left) + 6,
        Math.Max(widget.Bottom, activation.Bottom) - Math.Min(widget.Top, activation.Top) + 6);
}

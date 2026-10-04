using System.Text;
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Dispatching;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Windows.UI;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Shell;

/// <summary>An isolated specimen using the editor's portable cascade and box layout.</summary>
internal sealed unsafe class BlockStylePreview : IDisposable
{
    public uint Role { get; }
    private readonly CoreDocument document;
    private readonly CoreView view;
    private readonly StyleDefinition target;
    private readonly StyleDefinition baseStyle;
    private StyleSheet? appliedSheet;
    private StyleDefinition? appliedStyle;
    private Color appliedForeground;
    private (float Width, float Height) size;
    private ulong metricsGeneration;
#if DEBUG
    internal int PropertyEdits { get; private set; }
    internal void InvalidateFontsForTesting() => view.Provider.InvalidateMetrics();
#endif
    // Compare payloads, not offsets into independently exported string arenas.
    private static bool SameValue(StyleSheet left, ViemStyleValueV1 a, StyleSheet right, ViemStyleValueV1 b)
    {
        if (a.kind != b.kind || a.number != b.number || a.enum_value != b.enum_value
            || !a.color.Equals(b.color) || a.item_count != b.item_count) return false;
        if (a.kind == VIEM_STYLE_VALUE_STRING && left.String(a) != right.String(b)) return false;
        for (ulong i = 0; i < a.item_count; i++) {
            var x = left.Items[checked((int)(a.first_item + i))];
            var y = right.Items[checked((int)(b.first_item + i))];
            if (x.kind != y.kind || x.unsigned_value != y.unsigned_value
                || Text(left.Strings, x.@string) != Text(right.Strings, y.@string)) return false;
        }
        return true;
    }

    public BlockStylePreview(uint role, CanvasDevice device, DispatcherQueue dispatcher)
    {
        Role = role;
        (string id, string markdown) = role switch {
            VIEM_STYLE_ROLE_TABLE => ("Table", "| Header | Detail |\n| --- | --- |\n| One | Two |"),
            VIEM_STYLE_ROLE_QUOTE => ("Block quote", "> A quotation contains a paragraph.\n>\n> Another paragraph shares its border.\n>\n>> A nested quotation has its own box."),
            VIEM_STYLE_ROLE_CODE_BLOCK => ("Code Block", "```\nA literal code block\n\nkeeps its lines and spaces.\n```"),
            VIEM_STYLE_ROLE_LIST => ("Bulleted List", "- A list item contains a paragraph.\n\n  And a second paragraph.\n\n- Another item."),
            VIEM_STYLE_ROLE_LIST_ITEM => ("List item", "- A list item contains a paragraph.\n\n  And a second paragraph.\n\n- Another item."),
            _ => ("Heading1", "Previous paragraph gives the style context.\n\n# A calm writing surface shaped with the selected style, with line spacing and alignment visible.\n\nFollowing paragraph shows spacing and inheritance.")
        };
        document = new(Encoding.UTF8.GetBytes(markdown), format: VIEM_FORMAT_MARKDOWN);
        CoreView? created = null;
        try {
            view = created = new(document, device, dispatcher, 560, 200);
            var initialSheet = view.Styles();
            target = initialSheet.Styles.Single(s => s.Id == id && s.Namespace == 1);
            baseStyle = initialSheet.Styles.Single(s => s.Id == "Paragraph" && s.Namespace == 1);
        } catch { created?.Dispose(); document.Dispose(); throw; }
    }

    public void Update(StyleSheet sheet, StyleDefinition style, Color foreground)
    {
        if (ReferenceEquals(appliedSheet, sheet) && ReferenceEquals(appliedStyle, style) && appliedForeground == foreground) return;
        try { Apply(sheet, style, foreground); }
        catch { appliedSheet = null; appliedStyle = null; throw; }
    }
    private void Apply(StyleSheet sheet, StyleDefinition style, Color foreground)
    {
        var context = New<ViemStyleEditValueV1>(); context.kind = VIEM_STYLE_VALUE_COLOR;
        context.color = new() { red = foreground.R / 255f, green = foreground.G / 255f, blue = foreground.B / 255f, alpha = foreground.A / 255f };
        if (appliedSheet == null || appliedForeground != foreground)
            view.EditStyle(baseStyle, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND, context);
        foreach (uint property in target.Properties.Keys) {
            var effective = style.UsesTextColor(property) ? default : style.Value(property);
            if (appliedSheet != null && appliedStyle != null) {
                bool same = property == VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND
                    && (style.UsesThemeForeground || appliedStyle.UsesThemeForeground)
                    ? style.UsesThemeForeground && appliedStyle.UsesThemeForeground && foreground == appliedForeground
                    : SameValue(appliedSheet, appliedStyle.UsesTextColor(property) ? default : appliedStyle.Value(property), sheet, effective);
                if (same) continue;
            }
#if DEBUG
            PropertyEdits++;
#endif
            if (property == VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND && style.UsesThemeForeground) {
                view.EditStyle(target, VIEM_STYLE_EDIT_SET_DECLARATION, property, context);
                continue;
            }
            using var arena = new NativeArena();
            var value = New<ViemStyleEditValueV1>();
            value.kind = effective.kind; value.number = effective.number; value.enum_value = effective.enum_value; value.color = effective.color;
            if (effective.kind == VIEM_STYLE_VALUE_STRING) value.text = arena.Utf8(sheet.String(effective));
            if (effective.item_count > 0) {
                var items = sheet.Items.Skip(checked((int)effective.first_item)).Take(checked((int)effective.item_count)).Select(item => {
                    var copy = New<ViemStyleEditValueItemV1>(); copy.kind = item.kind;
                    copy.text = arena.Utf8(Text(sheet.Strings, item.@string)); copy.unsigned_value = item.unsigned_value; return copy;
                }).ToArray();
                value.items = arena.Copy<ViemStyleEditValueItemV1>(items); value.item_count = (ulong)items.Length;
            }
            view.EditStyle(target, effective.kind == VIEM_STYLE_VALUE_NONE ? VIEM_STYLE_EDIT_CLEAR_DECLARATION : VIEM_STYLE_EDIT_SET_DECLARATION, property, value);
        }
        appliedSheet = sheet; appliedStyle = style; appliedForeground = foreground;
    }

    internal LayoutSnapshot Layout(float width, float height) {
        if (size != (width, height) || metricsGeneration != view.Provider.Generation) { view.Resize(Math.Max(1, width), Math.Max(1, height)); size = (width, height); metricsGeneration = view.Provider.Generation; }
        return view.Layout();
    }
    public void Draw(CanvasDrawingSession drawing, float width, float height, Color fallback)
    {
        var snapshot = Layout(width, height);
        ViemTextPaintV1 Paint(ulong at) {
            var paint = snapshot.PaintRuns.FirstOrDefault(run => run.text_start <= at && at < run.text_end).paint;
            return paint.struct_size != 0 ? paint : snapshot.Paint.default_paint;
        }
        Color Foreground(ViemTextPaintV1 paint) => (paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) != 0 ? fallback : ConvertColor(paint.foreground);
        const uint boxFlags = VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND | VIEM_LAYOUT_DECORATION_BLOCK_BORDER | VIEM_LAYOUT_DECORATION_BLOCK_QUOTE_BORDER;
        var antialiasing = drawing.Antialiasing;
        drawing.Antialiasing = Microsoft.Graphics.Canvas.CanvasAntialiasing.Aliased;
        try {
            foreach (var box in snapshot.Decorations.Where(d => (d.flags & boxFlags) != 0))
                drawing.FillRectangle(Rect(box.typographic_bounds), Foreground(box.paint));
        } finally { drawing.Antialiasing = antialiasing; }
        foreach (var cluster in snapshot.Clusters) {
            var paint = Paint(cluster.text_start);
            if ((paint.flags & VIEM_TEXT_PAINT_HAS_BACKGROUND) != 0) drawing.FillRectangle(Rect(cluster.typographic_bounds), ConvertColor(paint.background));
        }
        using var glyphs = view.Provider.BeginDrawing(drawing);
        foreach (var row in snapshot.Rows) {
            for (ulong i = row.first_cluster; i < row.first_cluster + row.cluster_count; i++) {
                var cluster = snapshot.Clusters[checked((int)i)]; var paint = Paint(cluster.text_start); var color = Foreground(paint);
                glyphs.Draw(cluster.render_run, new(cluster.x, row.baseline), color);
                if ((paint.flags & (VIEM_TEXT_PAINT_UNDERLINE | VIEM_TEXT_PAINT_STRIKETHROUGH)) != 0) glyphs.Flush();
                var bounds = cluster.typographic_bounds;
                if ((paint.flags & VIEM_TEXT_PAINT_UNDERLINE) != 0) drawing.DrawLine(bounds.x, row.baseline + 2, bounds.x + bounds.width, row.baseline + 2, color);
                if ((paint.flags & VIEM_TEXT_PAINT_STRIKETHROUGH) != 0) drawing.DrawLine(bounds.x, row.baseline - row.ascent * .3f, bounds.x + bounds.width, row.baseline - row.ascent * .3f, color);
            }
        }
        glyphs.Flush();
        foreach (var decoration in snapshot.Decorations) {
            if ((decoration.flags & boxFlags) != 0) continue;
            var row = snapshot.Rows.FirstOrDefault(r => r.row_index == decoration.row_index);
            glyphs.Draw(decoration.render_run, new(decoration.x, row.baseline), Foreground(decoration.paint));
        }
    }

    private static global::Windows.Foundation.Rect Rect(ViemLayoutRectV1 value) => new(value.x, value.y, value.width, value.height);
    private static Color ConvertColor(ViemRgbaV1 value) {
        static byte Channel(float value) => (byte)Math.Round(Math.Clamp(value, 0, 1) * 255);
        return Color.FromArgb(Channel(value.alpha), Channel(value.red), Channel(value.green), Channel(value.blue));
    }
    public void Dispose() { view.Dispose(); document.Dispose(); }
}

using System.Text;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Text;
using Microsoft.UI.Xaml.Controls;
using Windows.Foundation;
using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    private bool ShowCodeBlockLanguageMenu(Point point)
    {
        if (snapshot == null || View == null) return false;
        foreach (var item in snapshot.Decorations)
        {
            if ((item.flags & VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) == 0) continue;
            var bounds = OffsetRect(item.typographic_bounds, viewport);
            if (!bounds.Contains(point)) continue;
            var row = snapshot.Rows.FirstOrDefault(row => row.row_index == item.row_index);
            var document = snapshot.Info.identity.document_id;
            var revision = snapshot.Info.identity.document_revision;
            var offset = row.text_start;
            var label = Encoding.UTF8.GetString(snapshot.DecorationLabels, checked((int)item.label_byte_start), checked((int)item.label_byte_length));
            var menu = new MenuFlyout();
            bool canEdit = (Document.State.flags & VIEM_DOCUMENT_STATE_READ_ONLY) == 0;
            void Add(string title, string language)
            {
                var choice = new ToggleMenuFlyoutItem { Text = title, IsChecked = label == title + " ▾", IsEnabled = canEdit };
                choice.Click += (_, _) => Run(() => View?.SetCodeBlockLanguage(document, revision, offset, language));
                menu.Items.Add(choice);
            }
            Add("None", ""); menu.Items.Add(new MenuFlyoutSeparator());
            foreach (var language in DocumentModes.Languages) Add(language.Name, language.Id);
            menu.ShowAt(Canvas, new Microsoft.UI.Xaml.Controls.Primitives.FlyoutShowOptions { Position = new(bounds.X, bounds.Bottom) });
            return true;
        }
        return false;
    }

    private void DrawCodeBlockLanguages(CanvasDrawingSession drawing)
    {
        if (snapshot == null) return;
        foreach (var item in snapshot.Decorations)
        {
            if ((item.flags & VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) == 0) continue;
            var bounds = OffsetRect(item.typographic_bounds, viewport);
            if (bounds.Bottom < 0 || bounds.Top > Canvas.ActualHeight) continue;
            var label = Encoding.UTF8.GetString(snapshot.DecorationLabels, checked((int)item.label_byte_start), checked((int)item.label_byte_length));
            var foreground = (item.paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) != 0 ? preferences.Theme.Foreground : Color(item.paint.foreground);
            using var format = new CanvasTextFormat { FontFamily = "Segoe UI", FontSize = item.font_size,
                HorizontalAlignment = CanvasHorizontalAlignment.Right, VerticalAlignment = CanvasVerticalAlignment.Top,
                WordWrapping = CanvasWordWrapping.NoWrap, TrimmingGranularity = CanvasTextTrimmingGranularity.Character };
            drawing.DrawText(label, bounds, foreground, format);
        }
    }
}

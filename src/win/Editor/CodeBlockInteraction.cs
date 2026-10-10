using System.Text;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Text;
using Microsoft.UI.Xaml.Controls;
using Windows.Foundation;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    private Point? codeBlockLanguageHoverLocation;
    private Rect? codeBlockLanguageHoverRect;
    private (ulong Document, ulong Offset)? codeBlockLanguageMenuTarget;
    internal MenuFlyout? CodeBlockLanguageFlyout { get; private set; }

    internal void MoveCodeBlockLanguageHover(Point? point)
    {
        codeBlockLanguageHoverLocation = point;
        UpdateCodeBlockLanguageHover();
    }

    private void UpdateCodeBlockLanguageHover()
    {
        Rect? next = null;
        if (snapshot != null && codeBlockLanguageHoverLocation is Point point
            && point.X >= 0 && point.Y >= 0 && point.X < Canvas.ActualWidth && point.Y < Canvas.ActualHeight)
        {
            foreach (var item in snapshot.Decorations)
            {
                if ((item.flags & VIEM_LAYOUT_DECORATION_CODE_LANGUAGE) == 0) continue;
                var bounds = OffsetRect(item.typographic_bounds, viewport);
                if (bounds.Contains(point)) { next = bounds; break; }
            }
        }
        if (codeBlockLanguageHoverRect == next) return;
        codeBlockLanguageHoverRect = next;
        Canvas.Invalidate();
    }

    internal float CodeBlockLanguageOpacity(ViemLayoutDecorationV1 item)
    {
        bool hovered = codeBlockLanguageHoverRect == OffsetRect(item.typographic_bounds, viewport);
        bool menuOpen = snapshot != null && codeBlockLanguageMenuTarget is { } target
            && target.Document == snapshot.Info.identity.document_id
            && snapshot.Rows.FirstOrDefault(row => row.row_index == item.row_index).text_start == target.Offset;
        return hovered || menuOpen ? 1 : .25f;
    }

    private void DismissCodeBlockLanguageMenu()
    {
        var menu = CodeBlockLanguageFlyout;
        // Hide can synchronously raise Closed after the pane has detached.
        // Retire ownership first so that callback cannot query old geometry.
        CodeBlockLanguageFlyout = null;
        codeBlockLanguageMenuTarget = null;
        MoveCodeBlockLanguageHover(null);
        menu?.Hide();
    }

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
            bool canEdit = (Document.State.flags & VIEM_DOCUMENT_STATE_READ_ONLY) == 0;
            var menu = CodeBlockLanguageMenu(label, canEdit,
                language => Run(() => View?.SetCodeBlockLanguage(document, revision, offset, language)));
            DismissCodeBlockLanguageMenu();
            codeBlockLanguageMenuTarget = (document, offset);
            CodeBlockLanguageFlyout = menu;
            menu.Closed += (_, _) => {
                if (!ReferenceEquals(CodeBlockLanguageFlyout, menu)) return;
                CodeBlockLanguageFlyout = null;
                codeBlockLanguageMenuTarget = null;
                MoveCodeBlockLanguageHover(!disposed && IsLoaded && Parent != null
                    ? CurrentPointerLocation() : null);
                Canvas.Invalidate();
            };
            Canvas.Invalidate();
            menu.ShowAt(Canvas, new Microsoft.UI.Xaml.Controls.Primitives.FlyoutShowOptions { Position = new(bounds.X, bounds.Bottom) });
            return true;
        }
        return false;
    }

    internal static MenuFlyout CodeBlockLanguageMenu(string label, bool canEdit, Action<string> select)
    {
        var menu = new MenuFlyout();
        ToggleMenuFlyoutItem Choice(string title, string language)
        {
            var choice = new ToggleMenuFlyoutItem { Text = title, IsChecked = label == title + " ▾", IsEnabled = canEdit };
            choice.Click += (_, _) => select(language);
            return choice;
        }
        var obscure = new MenuFlyoutSubItem { Text = "Obscure languages", AccessKey = "O", IsEnabled = canEdit,
            IsAccessKeyScope = true, ExitDisplayModeOnAccessKeyInvoked = false,
            Icon = new FontIcon { FontFamily = new Microsoft.UI.Xaml.Media.FontFamily("Segoe Fluent Icons"), FontSize = 12, Width = 16,
                Glyph = DocumentModes.Languages.Any(language => !language.Primary && label == language.Name + " ▾") ? "\uE73E" : "" } };
        bool loaded = false;
        obscure.Loaded += (_, _) =>
        {
            if (loaded) return;
            foreach (var language in DocumentModes.Languages.Where(language => !language.Primary))
                obscure.Items.Add(Choice(language.Name, language.Id));
            loaded = true;
        };
        var none = Choice("None", ""); none.AccessKey = "N";
        menu.Items.Add(none); menu.Items.Add(obscure); menu.Items.Add(new MenuFlyoutSeparator());
        foreach (var language in DocumentModes.Languages.Where(language => language.Primary))
            menu.Items.Add(Choice(language.Name, language.Id));
        return menu;
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
            foreground.A = (byte)Math.Round(foreground.A * CodeBlockLanguageOpacity(item));
            using var format = new CanvasTextFormat { FontFamily = "Segoe UI", FontSize = item.font_size,
                HorizontalAlignment = CanvasHorizontalAlignment.Right, VerticalAlignment = CanvasVerticalAlignment.Top,
                WordWrapping = CanvasWordWrapping.NoWrap, TrimmingGranularity = CanvasTextTrimmingGranularity.Character };
            using var clip = drawing.CreateLayer(1, bounds);
            drawing.DrawText(label, bounds, foreground, format);
        }
    }
}

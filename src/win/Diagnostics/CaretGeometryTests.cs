#if DEBUG
using System.Text;
using Viem.Windows.Core;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class CaretGeometryTests
{
    internal static async Task Run(EditorWindow owner, Preferences ownerPreferences)
    {
        void Check(bool value, string name)
        { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }
        var preferences = new Preferences(Path.Combine(ownerPreferences.DirectoryPath, "caret-tests", Guid.NewGuid().ToString("N")));
        try {
            foreach (string source in new[] { "", "body\n\n" }) {
                byte[] bytes = Encoding.UTF8.GetBytes(source);
                var document = new CoreDocument(bytes, format: VIEM_FORMAT_PLAIN_TEXT);
                preferences.AttachThemeDocument(document);
                var window = new EditorWindow(preferences, document);
                App.Instance.Windows.Add(window); window.Activate();
                try {
                    var pane = window.ActivePane!;
                    var view = await pane.Ready;
                    var paragraph = view.Styles().Styles.Single(style => style.Id == "Paragraph");
                    view.EditStyle(paragraph, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_SIZE, CoreView.Number(80));
                    var exact = CoreView.Enum(VIEM_STYLE_VALUE_LINE_SPACING, VIEM_STYLE_LINE_SPACING_EXACT); exact.number = 8;
                    view.EditStyle(paragraph, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING, exact);
                    view.Command("G");
                    foreach (float scale in new[] { .75f, 1f, 1.5f }) {
                        view.Zoom(scale); pane.Refresh();
                        var caret = pane.CaretRectangle;
                        Check(Math.Abs(caret.Width - Math.Ceiling(40 * scale)) < .01,
                            $"empty native block caret uses its 80-DIP typing font half-em at zoom {scale} independently of exact line spacing");
                        long shaped = view.Provider.ShapedCharacters;
                        pane.Refresh(); pane.Refresh();
                        Check(view.Provider.ShapedCharacters == shaped && pane.CaretRectangle == caret,
                            "empty caret refresh reuses current geometry without additional shaping");
                    }
                    view.Command("i"); pane.Refresh();
                    Check(pane.CaretRectangle.Width == 2, "large-font empty Insert caret retains its 2-DIP native width");
                    view.Key(VIEM_KEY_ESCAPE);
                    Check(document.Source(document.State.document_revision).AsSpan().SequenceEqual(bytes),
                        "empty and terminal-empty caret presentation preserves exact source bytes");
                    if (pane.LastError is { } error) throw error;
                } finally { window.Close(); App.Instance.Windows.Remove(window); }
            }
        } finally {
            owner.Activate(); owner.ActivePane?.FocusEditor();
            CoreThemes.ReplaceCodeStyles(ownerPreferences.ThemeStyleDefaults(VIEM_FORMAT_CODE));
        }
    }
}
#endif

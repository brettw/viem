#if DEBUG
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class FormatMenuTests
{
    private static void Check(bool condition, string name)
    { if (!condition) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }

    internal static async Task Run(Preferences preferences)
    {
        var document = new CoreDocument("alpha beta"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        await window.ActivePane!.Ready;
        var styles = window.Menu.Items.Single(m => m.Title == "Style");
        var stylesPeer = new MenuBarItemAutomationPeer(styles);
        try
        {
            var formatMenu = window.Menu.Items.Single(m => m.Title == "Format");
            int formatIndex = window.Menu.Items.IndexOf(formatMenu);
            Check(window.Menu.Items[formatIndex - 1].Title == "Edit" && window.Menu.Items[formatIndex + 1].Title == "Style",
                "Format sits between Edit and Style");
            Check(formatMenu.Items.OfType<ToggleMenuFlyoutItem>().Select(item => item.Text)
                    .SequenceEqual(new[] { "Plain text", "Markdown Preview", "Code", "Word wrap" })
                && formatMenu.Items.Count == 5 && formatMenu.Items[3] is MenuFlyoutSeparator,
                "Format groups checked document types before wrapping");
            var view = window.Menu.Items.Single(m => m.Title == "View");
            Check(!view.Items.OfType<ToggleMenuFlyoutItem>().Any(item => item.Text is "Plain text" or "Markdown Preview" or "Code" or "Word wrap")
                && !view.Items.OfType<MenuFlyoutSubItem>().Any(item => item.Text is "Code" or "Code language"),
                "View no longer contains document types, languages or wrapping");
            Check(styles.Items.OfType<MenuFlyoutSubItem>().Any(i => i.Text == "Paragraph"),
                "paragraph style and list actions remain in the Style menu");
            foreach (string title in new[] { "Convert to", "Reinterpret as" })
                Check(!window.Menu.Items.Single(m => m.Title == "File").Items
                    .OfType<MenuFlyoutSubItem>().Any(i => i.Text == title), title + " is absent");
            foreach (uint format in new[] { VIEM_FORMAT_PLAIN_TEXT, VIEM_FORMAT_CODE })
            {
                var literal = window.AddPane(new CoreDocument("text"u8.ToArray(), format: format)); await literal.Ready;
                Check(!literal.CanUseFormattingCommands, "literal formats disable rich formatting commands");
                var editStyles = styles.Items.OfType<MenuFlyoutItem>().Single(i => i.Text == "Edit Styles…");
                await MenuTestHelpers.Transition(true, () => { stylesPeer.Expand(); return Task.CompletedTask; }, editStyles);
                Check(editStyles.IsEnabled,
                    "independent style editing remains available for literal formats");
                await MenuTestHelpers.Transition(false, () => { stylesPeer.Collapse(); return Task.CompletedTask; }, editStyles);
            }
        }
        finally { App.Instance.Windows.Remove(window); window.Close(); }
    }
}
#endif

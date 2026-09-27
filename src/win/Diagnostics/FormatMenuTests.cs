#if DEBUG
using System.Text;
using System.Text.Json;
using Microsoft.UI.Xaml;
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
    private static IEnumerable<MenuFlyoutItemBase> Descendants(IEnumerable<MenuFlyoutItemBase> items)
    {
        foreach (var item in items) {
            yield return item;
            if (item is MenuFlyoutSubItem sub) foreach (var child in Descendants(sub.Items)) yield return child;
        }
    }
    internal static async Task Run(Preferences preferences)
    {
        var document = new CoreDocument("{\\rtf1{\\colortbl;\\red255\\green0\\blue0;\\red255\\green255\\blue0;\\red0\\green0\\blue255;\\red0\\green255\\blue0;}{\\cf1\\highlight2\\fs24 red} {\\cf3\\highlight4\\fs48 blue}}"u8.ToArray(), format: VIEM_FORMAT_RTF);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate(); await Task.Delay(250);
        var pane = window.ActivePane!; var view = await pane.Ready;
        var menu = window.Menu.Items.Single(m => m.Title == "Format");
        var menuPeer = new MenuBarItemAutomationPeer(menu);
        var styles = window.Menu.Items.Single(m => m.Title == "Style");
        var stylesPeer = new MenuBarItemAutomationPeer(styles);
        var themeMenu = styles.Items.OfType<MenuFlyoutSubItem>().Single(i => i.Text == "Theme");
        var paragraphStyles = styles.Items.OfType<MenuFlyoutSubItem>().Single(i => i.Text == "Paragraph");
        var characterStyles = styles.Items.OfType<MenuFlyoutSubItem>().Single(i => i.Text == "Character");
        async Task CheckStyleAvailability(bool enabled) {
            window.Activate(); stylesPeer.Expand(); await Task.Delay(40);
            Check(paragraphStyles.IsEnabled == enabled && characterStyles.IsEnabled == enabled,
                $"Style submenus validate for format {window.ActivePane!.Document.State.format}");
            Check(styles.Items.OfType<MenuFlyoutItem>().Single(i => i.Text == "Edit Styles…").IsEnabled
                && themeMenu.IsEnabled,
                "theme selection and style editing remain available for every format");
            stylesPeer.Collapse();
        }
        async Task Open() { window.Activate(); menuPeer.Expand(); await Task.Delay(40); }
        MenuFlyoutItem Item(string name) => Descendants(menu.Items).OfType<MenuFlyoutItem>().Single(i => i.Text == name);
        ToggleMenuFlyoutItem Toggle(string name) => menu.Items.OfType<ToggleMenuFlyoutItem>().Single(i => i.Text == name);
        async Task Invoke(string name) { await Open(); new MenuFlyoutItemAutomationPeer(Item(name)).Invoke(); await Task.Delay(40); }
        async Task Flip(string name, bool reselect = true) {
            if (reselect) { view.Key(VIEM_KEY_ESCAPE); view.Command("gg0viw"); }
            await Open(); Check(Toggle(name).Focus(FocusState.Programmatic), "native Format item receives keyboard focus: " + name);
            await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space); menuPeer.Collapse(); await Task.Delay(40);
        }
        try {
            Check(window.Menu.Items.All(m => m.Title is not ("Paragraph" or "Character"))
                && styles.Items.First() == themeMenu
                && styles.Items.Skip(2).Take(2).SequenceEqual(new MenuFlyoutItemBase[] { paragraphStyles, characterStyles }),
                "Style starts with Theme, then the paragraph and character assignment submenus");
            Check(!Descendants(menu.Items).Any(i => i is MenuFlyoutSubItem { Text: "Style" })
                && Descendants(styles.Items).OfType<MenuFlyoutItem>().Count(i => i.Text == "Edit Styles…") == 1,
                "Style commands have one home and the assignment menus have no editor footers");
            await CheckStyleAvailability(true);
            Check(!Descendants(menu.Items).Any(i => i is MenuFlyoutSubItem { Text: "Baseline" } || i is MenuFlyoutItem { Text: "Bigger" or "Smaller" }),
                "Format omits Baseline, Bigger and Smaller and exposes top-level script toggles");
            view.Command("gg0viw"); await Open();
            Check(Descendants(menu.Items).OfType<MenuFlyoutItem>().All(i => i.IsEnabled)
                && menu.Items.OfType<ToggleMenuFlyoutItem>().All(i => i.IsEnabled), "every applicable Format action is enabled for a rich linear selection");
            menuPeer.Collapse();
            foreach (var (name, style) in new[] { ("Bold", VIEM_SEMANTIC_STYLE_STRONG), ("Italic", VIEM_SEMANTIC_STYLE_EMPHASIS) }) {
                await Flip(name); Check(view.SemanticStyle(style).state == VIEM_SEMANTIC_STYLE_STATE_ON, name + " Format action applies");
                view.Undo();
            }
            foreach (var (name, property) in new[] { ("Underline", VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE), ("Strikethrough", VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH) }) {
                await Flip(name); await Open();
                Check(view.DecorationState(property) == VIEM_SEMANTIC_STYLE_STATE_ON && Toggle(name).IsChecked, name + " action and native checkmark agree");
                menuPeer.Collapse(); view.Undo();
            }
            await Flip("Superscript"); Check(view.Typography().Info.script_position == VIEM_SCRIPT_POSITION_SUPERSCRIPT, "Format Superscript changes selected text");
            await Flip("Subscript"); await Open();
            Check(view.Typography().Info.script_position == VIEM_SCRIPT_POSITION_SUBSCRIPT && Toggle("Subscript").IsChecked && !Toggle("Superscript").IsChecked,
                "Format script actions are mutually exclusive"); menuPeer.Collapse();
            await Flip("Subscript"); Check(view.Typography().Info.script_position == VIEM_SCRIPT_POSITION_NORMAL, "Format active script toggles back to normal");
            view.Undo(); view.Undo(); view.Undo();

            foreach (var name in new[] { "Center", "End", "Start", "Left to Right", "Right to Left", "Automatic", "Normal", "Single", "1.5 Lines", "Double" }) {
                byte[] before = document.Source(document.State.document_revision);
                await Invoke(name);
                Check(pane.LastError == null, "Format paragraph action succeeds: " + name);
                using var fragment = JsonDocument.Parse(view.ClipboardJson(0, (ulong)Encoding.UTF8.GetByteCount(document.FormattedText())));
                var paragraph = fragment.RootElement.GetProperty("paragraph_runs")[0];
                if (name is "Center" or "End" or "Start")
                    Check(paragraph.GetProperty("alignment").GetString() == name, "Format alignment stores " + name);
                else if (name is "Left to Right" or "Right to Left")
                    Check(paragraph.GetProperty("resolved_direction").GetString() == (name == "Left to Right" ? "LeftToRight" : "RightToLeft"), "Format direction stores " + name);
                else if (name is "Single" or "1.5 Lines" or "Double")
                    Check(Math.Abs(paragraph.GetProperty("line_spacing").GetProperty("Multiplier").GetDouble() - (name == "Single" ? 1 : name == "1.5 Lines" ? 1.5 : 2)) < .001,
                        "Format line spacing stores " + name);
                if (!before.AsSpan().SequenceEqual(document.Source(document.State.document_revision))) view.Undo();
            }

            view.Key(VIEM_KEY_ESCAPE); view.Command("gg0");
            await Invoke("Font…"); await Invoke("Text Color…"); await Invoke("Highlight Color…");
            var font = window.FontPanel!;
            var foreground = window.ColorPanel(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND)!;
            var background = window.ColorPanel(VIEM_STYLE_PROPERTY_CHARACTER_BACKGROUND)!;
            Check(font != null && foreground != null && background != null && !foreground.Picker.IsEnabled,
                "font and color menus open persistent native controls at a normal caret");
            Check(foreground.Picker.Color == Microsoft.UI.Colors.Red && background.Picker.Color == Microsoft.UI.Colors.Yellow,
                "persistent colors initialize from the caret foreground and highlight");
            byte[] untouched = document.Source(document.State.document_revision);
            int refreshes = foreground.RefreshCount;
            view.Command("w"); view.Command("b"); view.Command("w");
            Check(foreground.RefreshPending && foreground.RefreshCount == refreshes, "caret notifications schedule a delayed color refresh");
            await Task.Delay(250);
            Check(foreground.RefreshCount == refreshes + 1 && foreground.Picker.Color == Microsoft.UI.Colors.Blue
                && background.Picker.Color == Microsoft.UI.Colors.Lime && font.SizeControl.Value == 24,
                "rapid caret movement coalesces and updates font, text color and highlight together");
            Check(untouched.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "passive panel synchronization preserves source bytes");
            await Invoke("Text Color…"); Check(window.ColorPanel(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND) == foreground,
                "reopening a color menu raises the existing persistent panel");
            view.Command("viw"); await Task.Delay(250);
            foreground.Picker.Color = Microsoft.UI.Colors.Orange; await Task.Delay(250);
            Check(foreground.Error.Length == 0 && view.Typography().Info.foreground.red > .99f && view.Typography().Info.foreground.blue < .01f,
                "persistent color controls apply to the current selection");
            view.Undo(); Check(untouched.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "one undo restores a color picker edit");
            view.Command("gg0wviw"); await Task.Delay(250);
            foreground.Picker.Color = Microsoft.UI.Colors.Orange; foreground.Picker.Color = Microsoft.UI.Colors.Blue;
            await Task.Delay(200);
            Check(untouched.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "returning a pending color gesture to its original color produces no source edit");
            foreground.Picker.Color = Microsoft.UI.Colors.Orange; foreground.Picker.Color = Microsoft.UI.Colors.Lime;
            await Task.Delay(200);
            Check(foreground.Error.Length == 0 && view.Typography().Info.foreground.green == 1, "coalesced native color samples commit their final value");
            view.Undo(); Check(untouched.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "coalesced color samples form one undo operation");
            view.Command("gg0wviw"); await Task.Delay(200);
            foreground.Picker.Color = Microsoft.UI.Colors.Orange;
            view.Key(VIEM_KEY_ESCAPE); view.Command("gg0"); await Task.Delay(200);
            Check(untouched.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "a pending color edit is discarded when its target selection changes");
            view.Command("gg0wviw"); await Task.Delay(200);
            font.SizeControl.Value = 18; await Task.Delay(200);
            Check(font.Error.Length == 0 && view.Typography().Info.size == 18, "persistent font size edits the selected text");
            view.Undo();
            view.Key(VIEM_KEY_ESCAPE); view.Command("i"); await Open();
            Check(Toggle("Superscript").IsEnabled && Toggle("Underline").IsEnabled, "Format character actions enable for an Insert caret");
            menuPeer.Collapse(); await Flip("Superscript", reselect: false);
            Check(view.Typography().Info.script_position == VIEM_SCRIPT_POSITION_SUPERSCRIPT, "script menu applies pending Insert typography");
            view.Key(VIEM_KEY_ESCAPE);

            foreach (uint format in new[] { VIEM_FORMAT_PLAIN_TEXT, VIEM_FORMAT_CODE }) {
                var literal = window.AddPane(new CoreDocument("plain text"u8.ToArray(), format: format));
                await literal.Ready;
                await CheckStyleAvailability(false);
            }
            var markdown = window.AddPane(new CoreDocument("plain markdown"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN));
            (await markdown.Ready).SelectAll(); await CheckStyleAvailability(true); await Open();
            Check(!Toggle("Superscript").IsEnabled && !Toggle("Underline").IsEnabled && !Item("Center").IsEnabled && !Item("Font…").IsEnabled,
                "unsupported Markdown direct formatting is disabled instead of failing after selection"); menuPeer.Collapse();
            var rich = window.AddPane(new CoreDocument("{\\rtf1 source}"u8.ToArray(), format: VIEM_FORMAT_RTF));
            (await rich.Ready).Command("i"); await CheckStyleAvailability(true); await Open();
            Check(Toggle("Superscript").IsEnabled && Item("Text Color…").IsEnabled, "RTF exposes supported rich formatting controls");
            menuPeer.Collapse(); await Task.Delay(200);
            Check(foreground.Picker.Color == Microsoft.UI.Colors.Blue,
                "persistent formatting panels retain their chosen document when another pane becomes active");
            await Invoke("Text Color…");
            Check(window.ColorPanel(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND) == foreground && foreground.Picker.Color == preferences.Theme.Foreground,
                "explicit color menu invocation retargets the existing panel to the chosen document");
            var rtf = window.AddPane(new CoreDocument("{\\rtf1\\ansi\\pard text\\par}"u8.ToArray(), format: VIEM_FORMAT_RTF));
            var rtfView = await rtf.Ready;
            await Invoke("Right to Left"); await Open();
            Check(!Item("Automatic").IsEnabled, "RTF disables explicit Automatic direction because its native syntax cannot express it");
            menuPeer.Collapse(); rtfView.Undo();
            await Invoke("Text Color…");
            Check(!foreground.Picker.IsAlphaEnabled, "RTF color panels offer only colors the source format can store");
            rtfView.Command("ggviw"); await Task.Delay(200);
            byte[] beforeRtfColor = rtf.Document.Source(rtf.Document.State.document_revision);
            foreground.Picker.Color = global::Windows.UI.Color.FromArgb(0, 50, 100, 150); await Task.Delay(200);
            Check(foreground.Error.Length == 0 && rtfView.Typography().Info.foreground.alpha == 1,
                "RTF color selection makes an inherited transparent picker value opaque");
            rtfView.Undo(); Check(beforeRtfColor.AsSpan().SequenceEqual(rtf.Document.Source(rtf.Document.State.document_revision)), "RTF picker edits retain one-step undo");
            rtfView.Command("ggviw"); await Task.Delay(200);
            foreground.Picker.Color = Microsoft.UI.Colors.Orange;
            new ButtonAutomationPeer(foreground.ResetControl).Invoke(); await Task.Delay(200);
            Check(beforeRtfColor.AsSpan().SequenceEqual(rtf.Document.Source(rtf.Document.State.document_revision)), "Default cancels a pending picker sample even when no direct color exists");
        }
        finally { App.Instance.Windows.Remove(window); window.Close(); }
    }
}
#endif

#if DEBUG
using System.Diagnostics;
using System.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Viem.Windows.Core;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class FormattingToolbarTests
{
    private static void Check(bool condition, string name)
    { if (!condition) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }
    private static ToggleMenuFlyoutItem Entry(DropDownButton selector, string id) => ((MenuFlyout)selector.Flyout).Items
        .OfType<ToggleMenuFlyoutItem>().Single(i => i.Tag is StyleKey key && key.Id == id);
    private static async Task Choose(DropDownButton selector, string id)
    {
        selector.Focus(FocusState.Programmatic);
        selector.Flyout.ShowAt(selector); await Task.Delay(40);
        Check(Entry(selector, id).Focus(FocusState.Programmatic), "toolbar style entry takes keyboard focus");
        await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
        selector.Flyout.Hide(); await Task.Delay(50);
    }
    private static bool? State(FormattingToolbar toolbar, ToolbarAction action) => ((ToggleButton)toolbar.Buttons[action]).IsChecked;
    private static async Task Click(FormattingToolbar toolbar, ToolbarAction action)
    {
        Check(toolbar.Buttons[action].Focus(FocusState.Programmatic), "toolbar action takes keyboard focus: " + action);
        await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
        await Task.Delay(40);
    }

    internal static async Task Run(Preferences preferences)
    {
        const string source = "<h1>Heading</h1><p>plain <code>code</code></p>";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_HTML);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        var pane = window.ActivePane!; var view = await pane.Ready; var toolbar = window.Toolbar;
        try
        {
            await Task.Delay(150);
            Check(window.ToolbarToggle.Visibility == Visibility.Visible && toolbar.Visibility == Visibility.Visible, "HTML toolbar defaults visible");
            Check(Grid.GetColumn(window.ToolbarToggle) == Grid.GetColumn(window.MenuToggle) + 1, "toolbar toggle immediately follows the menu toggle");
            Check(toolbar.Paragraph.Content as string == "Heading 1", "toolbar reads the initial named paragraph");
            view.Command("j0");
            Check(toolbar.Paragraph.Content as string == "Base Paragraph", "toolbar follows a caret command synchronously");
            var items = ((MenuFlyout)toolbar.Paragraph.Flyout).Items.ToArray();
            var sheet = view.Styles(view.SelectedNamedStyles().Identity);
            view.Command("w");
            Check(toolbar.Character.Content as string == "Code", "toolbar tracks the Code character assignment");
            Check(items.SequenceEqual(((MenuFlyout)toolbar.Paragraph.Flyout).Items) && ReferenceEquals(sheet, view.Styles(view.SelectedNamedStyles().Identity)),
                "cursor movement retains native menu items and the exact stylesheet snapshot");
            view.SelectAll();
            Check(toolbar.Paragraph.Content as string == "Mixed" && toolbar.Character.Content as string == "Mixed", "toolbar selectors show mixed assignments");
            await Choose(toolbar.Paragraph, "Heading2");
            Check(toolbar.Paragraph.Content as string == "Heading 2" && pane.LastError == null, "toolbar selector invokes the shared heading action");
            byte[] changed = document.Source(document.State.document_revision);
            view.Undo(); Check(Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source, "toolbar paragraph assignment is one undo transaction");
            view.Redo(); Check(changed.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "toolbar paragraph redo restores exact source"); view.Undo();

            view.Key(VIEM_KEY_ESCAPE); view.Command("j0viw");
            foreach (var action in new[] { ToolbarAction.Bold, ToolbarAction.Italic, ToolbarAction.Underline, ToolbarAction.Strikethrough, ToolbarAction.Superscript, ToolbarAction.Subscript, ToolbarAction.CharacterCode })
            {
                byte[] before = document.Source(document.State.document_revision);
                await Click(toolbar, action);
                Check(State(toolbar, action) == true && pane.LastError == null, "toolbar applies and immediately reflects " + action);
                view.Undo(); Check(before.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "toolbar undo restores " + action);
                view.Key(VIEM_KEY_ESCAPE); view.Command("ggj0viw");
            }
            await Click(toolbar, ToolbarAction.CharacterCode); await Click(toolbar, ToolbarAction.CharacterCode);
            Check(State(toolbar, ToolbarAction.CharacterCode) == false && toolbar.Character.Content as string == "Default Paragraph", "character Code toggles off through Default Paragraph");
            Check(toolbar.Buttons[ToolbarAction.CodeBlock].Visibility == Visibility.Collapsed, "unsupported generated Code Block action is omitted");

            view.Key(VIEM_KEY_ESCAPE); view.Command("ggj0i");
            ulong revision = document.State.document_revision;
            await Click(toolbar, ToolbarAction.Italic);
            Check(State(toolbar, ToolbarAction.Italic) == true && document.State.document_revision == revision, "toolbar supports pending typing formatting without a source edit");
            view.Text("typed"); view.Key(VIEM_KEY_ESCAPE); view.Undo();

            view.Command("ggj0viw");
            byte[] beforeColor = document.Source(document.State.document_revision);
            var beforeSelection = view.LogicalSelection();
            toolbar.TextColor.Flyout.ShowAt(toolbar.TextColor); await Task.Delay(100);
            Check(beforeColor.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "opening a toolbar color well creates no edit");
            toolbar.TextColor.Picker!.Color = Microsoft.UI.Colors.Orange;
            await Task.Delay(100);
            Check(view.Typography().Info.foreground.red > .99f && view.Typography().Info.foreground.blue < .01f, "toolbar color changes apply live through the direct formatting transaction");
            var afterSelection = view.LogicalSelection();
            Check(beforeSelection.text_start == afterSelection.text_start && beforeSelection.text_end == afterSelection.text_end, "toolbar colors retain the logical selection");
            toolbar.TextColor.Flyout.Hide(); await Task.Delay(100);
            view.Undo(); Check(beforeColor.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "toolbar color edit undoes exactly");
            toolbar.TextColor.Flyout.ShowAt(toolbar.TextColor); await Task.Delay(80);
            toolbar.TextColor.Picker.Color = Microsoft.UI.Colors.Blue;
            view.Key(VIEM_KEY_ESCAPE); view.Command("gg0"); await Task.Delay(100);
            Check(beforeColor.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "queued toolbar color samples never follow a moved selection");

            view.Command("j0viw");
            var staleIdentity = view.SelectedNamedStyles().Identity;
            view.ToggleSemantic(VIEM_SEMANTIC_STYLE_STRONG);
            ulong afterBold = document.State.document_revision;
            view.ChooseStyle(new(1, "Heading3"), staleIdentity);
            Check(document.State.document_revision == afterBold, "stale catalogue actions cannot edit a later source revision");
            view.Undo();
            var oldSheet = view.Styles(view.SelectedNamedStyles().Identity);
            string custom = view.CreateStyle(2, "Toolbar Test");
            Check(!ReferenceEquals(oldSheet, view.Styles(view.SelectedNamedStyles().Identity)) && Entry(toolbar.Character, custom).Text == "Toolbar Test",
                "stylesheet edits invalidate the toolbar catalogue and expose new stable identities");
            view.Key(VIEM_KEY_ESCAPE); view.Command("ggj0viw");
            await Choose(toolbar.Character, custom);
            await Click(toolbar, ToolbarAction.Bold);
            await Choose(toolbar.Character, custom);
            Check(toolbar.Character.Content as string == "Toolbar Test" && State(toolbar, ToolbarAction.Bold) == false,
                "rechoosing a named style clears direct overrides through the shared assignment action");
            view.Key(VIEM_KEY_ESCAPE); view.Command("gg0");

            view.Key(VIEM_KEY_CONTROL_CHARACTER, 'q');
            Check(!toolbar.Buttons[ToolbarAction.Bold].IsEnabled && !toolbar.Paragraph.IsEnabled && pane.LastError == null, "Visual Block disables unsupported toolbar actions without errors");
            view.Key(VIEM_KEY_ESCAPE); view.Command(":");
            Check(!toolbar.Paragraph.IsEnabled && !toolbar.TextColor.IsEnabled && pane.LastError == null, "command prompt disables formatting without stale selection errors");
            view.Key(VIEM_KEY_ESCAPE);

            // Save the native chrome in both appearances and verify overflow.
            await WindowCapture.Save(window.Hwnd, pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".toolbar.png");
            var originalTheme = preferences.Theme;
            preferences.SetSections(new() { ["theme"] = Preferences.ThemeJson(Theme.Paper) }); await Task.Delay(80);
            await WindowCapture.Save(window.Hwnd, pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".toolbar-paper.png");
            preferences.SetSections(new() { ["theme"] = Preferences.ThemeJson(originalTheme) });
            var size = window.AppWindow.Size; window.AppWindow.Resize(new(540, 450)); await Task.Delay(120);
            Check(toolbar.Scroll.ScrollableWidth > 0, "narrow Windows toolbar keeps all groups accessible by scrolling");
            toolbar.Scroll.ChangeView(toolbar.Scroll.ScrollableWidth, null, null, true); await Task.Delay(50);
            Check(toolbar.Scroll.HorizontalOffset > 0, "toolbar overflow scrolls to its trailing indent controls");
            window.AppWindow.Resize(size);

            bool showMenu = preferences.ShowMenu;
            preferences.Set("windows", "showMenu", false);
            Check(toolbar.Visibility == Visibility.Visible, "toolbar visibility is independent of menu visibility");
            preferences.Set("windows", "showMenu", showMenu);
            window.ToolbarToggle.Focus(FocusState.Programmatic);
            await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space); await Task.Delay(50);
            Check(toolbar.Visibility == Visibility.Collapsed && !new Preferences(preferences.DirectoryPath).ShowFormattingToolbar(VIEM_FORMAT_HTML), "title-bar toggle persists the HTML toolbar preference");

            var markdown = window.AddPane(new CoreDocument("- One\n  - Two\n\nPlain"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN));
            var md = await markdown.Ready;
            Check(toolbar.Visibility == Visibility.Visible, "switching panes restores each format's toolbar preference");
            Check(toolbar.Buttons[ToolbarAction.Underline].Visibility == Visibility.Collapsed && toolbar.Buttons[ToolbarAction.Superscript].Visibility == Visibility.Collapsed,
                "Markdown omits rich direct formatting controls");
            md.Command("vj");
            Check(State(toolbar, ToolbarAction.Bullets) == true && toolbar.Paragraph.Content as string == "Mixed", "list state is structural across nested paragraph styles");
            md.SelectAll(); Check(State(toolbar, ToolbarAction.Bullets) == null, "mixed list and non-list selection shows a mixed toggle");
            md.Key(VIEM_KEY_ESCAPE); md.Command("ggj0");
            byte[] nested = markdown.Document.Source(markdown.Document.State.document_revision);
            await Click(toolbar, ToolbarAction.Unindent);
            Check(toolbar.Buttons[ToolbarAction.Indent].IsEnabled && State(toolbar, ToolbarAction.Bullets) == true, "Unindent preserves structural list membership");
            await Click(toolbar, ToolbarAction.Indent);
            Check(toolbar.Buttons[ToolbarAction.Unindent].IsEnabled && State(toolbar, ToolbarAction.Bullets) == true, "Indent preserves structural list membership");
            md.Undo(); md.Undo(); Check(nested.AsSpan().SequenceEqual(markdown.Document.Source(markdown.Document.State.document_revision)), "toolbar indentation actions each undo atomically");
            md.Key(VIEM_KEY_ESCAPE); md.Command("gg0");
            await Click(toolbar, ToolbarAction.Bullets); Check(State(toolbar, ToolbarAction.Bullets) == false, "active bulleted list toolbar action removes the list");
            await Click(toolbar, ToolbarAction.Numbers); Check(State(toolbar, ToolbarAction.Numbers) == true, "numbered list toolbar action applies structural numbering");
            md.Format(VIEM_FORMAT_MARKDOWN_SOURCE);
            Check(toolbar.Visibility == Visibility.Visible, "Markdown Source has its own visible toolbar");
            preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN_SOURCE, false);
            md.Format(VIEM_FORMAT_MARKDOWN); Check(toolbar.Visibility == Visibility.Visible, "Source and WYSIWYG toolbar preferences are independent");
            pane.FocusEditor(); await Task.Delay(80);
            Check(toolbar.Visibility == Visibility.Collapsed, "returning to HTML restores its hidden toolbar");
            preferences.SetFormattingToolbar(VIEM_FORMAT_HTML, true); preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN_SOURCE, true);

            foreach (uint format in new[] { VIEM_FORMAT_PLAIN_TEXT, VIEM_FORMAT_CODE })
            {
                var literal = window.AddPane(new CoreDocument("text"u8.ToArray(), format: format)); await literal.Ready;
                Check(toolbar.Visibility == Visibility.Collapsed && window.ToolbarToggle.Visibility == Visibility.Collapsed, "literal format omits toolbar and title-bar toggle: " + format);
            }
            var pre = window.AddPane(new CoreDocument("<pre>Words</pre>"u8.ToArray(), format: VIEM_FORMAT_HTML)); await pre.Ready;
            Check(State(toolbar, ToolbarAction.CodeBlock) == true && toolbar.Buttons[ToolbarAction.CodeBlock].Visibility == Visibility.Visible, "source-backed Code Block can be cleared");
            await Click(toolbar, ToolbarAction.CodeBlock); Check(State(toolbar, ToolbarAction.CodeBlock) == false && pre.LastError == null, "Code Block toggle clears its named assignment");
            Check(window.Panes.All(p => p.LastError == null), "toolbar scenarios leave no presentation errors");
        }
        finally { App.Instance.Windows.Remove(window); window.Close(); }
        await Performance(preferences);
    }

    private static async Task Performance(Preferences preferences)
    {
        string md = "# Heading\n\n- First item\n- Second item\n\n" + string.Concat(Enumerable.Repeat("A paragraph with **bold** and `code` text.\n\n", 4000));
        string html = "<h1>Heading</h1><ul><li>First item</li><li>Second item</li></ul>" + string.Concat(Enumerable.Repeat("<p>A paragraph with <b>bold</b> and <code>code</code> text.</p>", 4000));
        string rtf = "{\\rtf1 First item\\par Second item\\par " + string.Concat(Enumerable.Repeat("A paragraph with {\\b bold} text.\\par ", 4000)) + "}";
        foreach (var (format, source) in new[] { (VIEM_FORMAT_MARKDOWN, md), (VIEM_FORMAT_MARKDOWN_SOURCE, md), (VIEM_FORMAT_HTML, html), (VIEM_FORMAT_HTML_SOURCE, html), (VIEM_FORMAT_RTF, rtf) })
        {
            var doc = new CoreDocument(Encoding.UTF8.GetBytes(source), format: format);
            var window = new EditorWindow(preferences, doc); App.Instance.Windows.Add(window); window.Activate();
            var pane = window.ActivePane!; var view = await pane.Ready; var toolbar = window.Toolbar;
            try
            {
                ulong at = (ulong)doc.FormattedText().IndexOf("Second item", StringComparison.Ordinal);
                view.Place(at, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, doc.State.document_revision);
                toolbar.Refresh();
                var items = ((MenuFlyout)toolbar.Paragraph.Flyout).Items.ToArray();
                double elapsed = 0;
                for (ulong i = 0; i < 10; i++)
                {
                    view.Place(at + i, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, doc.State.document_revision);
                    long start = Stopwatch.GetTimestamp(); toolbar.Refresh(); elapsed += Stopwatch.GetElapsedTime(start).TotalMilliseconds;
                }
                Check(elapsed / 10 < 50 && items.SequenceEqual(((MenuFlyout)toolbar.Paragraph.Flyout).Items), $"large {CoreDocument.FormatName(format)} toolbar refresh averages {elapsed / 10:F2} ms and retains controls");
                view.Place(at, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, doc.State.document_revision); view.Command("viw");
                long actionStart = Stopwatch.GetTimestamp(); toolbar.Execute(ToolbarAction.Bold); double actionMs = Stopwatch.GetElapsedTime(actionStart).TotalMilliseconds;
                // HTML Source is deliberately one huge physical line; measuring
                // that line is separate from the local toolbar/source work.
                Check(State(toolbar, ToolbarAction.Bold) == true && pane.LastError == null && (format == VIEM_FORMAT_HTML_SOURCE || actionMs < 500),
                    $"large {CoreDocument.FormatName(format)} selected Bold applies in {actionMs:F2} ms");
                view.Key(VIEM_KEY_ESCAPE); view.Command("i"); ulong revision = doc.State.document_revision;
                actionStart = Stopwatch.GetTimestamp(); toolbar.Execute(ToolbarAction.Italic); actionMs = Stopwatch.GetElapsedTime(actionStart).TotalMilliseconds;
                Check(actionMs < 100 && State(toolbar, ToolbarAction.Italic) == true && doc.State.document_revision == revision,
                    $"large {CoreDocument.FormatName(format)} pending Italic applies in {actionMs:F2} ms");
            }
            finally { App.Instance.Windows.Remove(window); window.Close(); }
        }
    }
}
#endif

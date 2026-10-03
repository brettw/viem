#if DEBUG
using System.Diagnostics;
using System.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
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
    private static async Task SwitchView(FormattingToolbar toolbar)
    {
        Check(toolbar.FormattedView.Focus(FocusState.Programmatic), "formatted-view toggle takes keyboard focus");
        await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
        await Task.Delay(40);
    }

    internal static async Task Run(Preferences preferences)
    {
        TablePickerTests.Run();
        await TableInteractionTests.Run(preferences);
        const string source = "# Heading\n\nplain `code`";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        var pane = window.ActivePane!; var view = await pane.Ready; var toolbar = window.Toolbar;
        try
        {
            await Task.Delay(150);
            Check(window.ToolbarToggle.Visibility == Visibility.Visible && toolbar.Visibility == Visibility.Visible, "Markdown toolbar defaults visible");
            Check(Grid.GetColumn(window.ToolbarToggle) == Grid.GetColumn(window.MenuToggle) + 1, "toolbar toggle immediately follows the menu toggle");
            Check(toolbar.Paragraph.Content as string == "Heading 1", "toolbar reads the initial named paragraph");
            Check(toolbar.FormattedView.IsChecked == true && !toolbar.FormattedView.IsThreeState
                && AutomationProperties.GetName(toolbar.FormattedView) == "Formatted view"
                && ToolTipService.GetToolTip(toolbar.FormattedView) as string == "Formatted view (WYSIWYG)",
                "formatted-view toggle has a stable accessible label and WYSIWYG pressed state");
            object viewIcon = toolbar.FormattedView.Content;
            byte[] beforeSwitch = document.Source(document.State.document_revision);
            ulong caret = (ulong)document.FormattedText().IndexOf("plain", StringComparison.Ordinal) + 2;
            view.Place(caret, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            await SwitchView(toolbar);
            Check(document.State.format == VIEM_FORMAT_MARKDOWN_SOURCE && toolbar.FormattedView.IsChecked == false
                && view.Presentation.cursor_utf8_offset == (ulong)source.IndexOf("plain", StringComparison.Ordinal) + 2
                && beforeSwitch.AsSpan().SequenceEqual(document.Source(document.State.document_revision)),
                "toolbar switches to Markdown Source while preserving source and semantic caret");
            view.Undo();
            Check(document.State.format == VIEM_FORMAT_MARKDOWN && toolbar.FormattedView.IsChecked == true,
                "undoing a view switch synchronizes the formatted-view toggle");
            view.Redo();
            Check(document.State.format == VIEM_FORMAT_MARKDOWN_SOURCE && toolbar.FormattedView.IsChecked == false,
                "redoing a view switch synchronizes the formatted-view toggle");
            await SwitchView(toolbar);
            Check(document.State.format == VIEM_FORMAT_MARKDOWN && toolbar.FormattedView.IsChecked == true
                && view.Presentation.cursor_utf8_offset == caret
                && ReferenceEquals(viewIcon, toolbar.FormattedView.Content)
                && beforeSwitch.AsSpan().SequenceEqual(document.Source(document.State.document_revision)),
                "formatted view round trip retains the caret, fixed icon and source bytes");
            view.Command("gg0");
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
            foreach (var action in new[] { ToolbarAction.Bold, ToolbarAction.Italic, ToolbarAction.Strikethrough })
            {
                byte[] before = document.Source(document.State.document_revision);
                await Click(toolbar, action);
                Check(State(toolbar, action) == true && pane.LastError == null, "toolbar applies and immediately reflects " + action);
                view.Undo(); Check(before.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "toolbar undo restores " + action);
                view.Key(VIEM_KEY_ESCAPE); view.Command("ggj0viw");
            }

            view.Key(VIEM_KEY_ESCAPE); view.Command("ggj0i");
            ulong revision = document.State.document_revision;
            await Click(toolbar, ToolbarAction.Italic);
            Check(State(toolbar, ToolbarAction.Italic) == true && document.State.document_revision == revision, "toolbar supports pending typing formatting without a source edit");
            view.Text("typed"); view.Key(VIEM_KEY_ESCAPE); view.Undo();

            view.Command("j0viw");
            var staleIdentity = view.SelectedNamedStyles().Identity;
            view.ToggleSemantic(VIEM_SEMANTIC_STYLE_STRONG);
            ulong afterBold = document.State.document_revision;
            view.ChooseStyle(new(1, "Heading3"), staleIdentity);
            Check(document.State.document_revision == afterBold, "stale catalogue actions cannot edit a later source revision");
            view.Undo();
            view.Key(VIEM_KEY_ESCAPE); view.Command("gg0");

            view.Key(VIEM_KEY_CONTROL_CHARACTER, 'q');
            Check(!toolbar.Buttons[ToolbarAction.Bold].IsEnabled && !toolbar.Paragraph.IsEnabled && pane.LastError == null, "Visual Block disables unsupported toolbar actions without errors");
            view.Key(VIEM_KEY_ESCAPE); view.Command(":");
            Check(!toolbar.Paragraph.IsEnabled && !toolbar.Character.IsEnabled && pane.LastError == null, "command prompt disables formatting without stale selection errors");
            view.Key(VIEM_KEY_ESCAPE);

            // Save the native chrome in both appearances and verify overflow.
            await WindowCapture.Save(window.Hwnd, pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".toolbar.png");
            var originalTheme = preferences.Theme;
            preferences.SetSections(new() { ["theme"] = Preferences.ThemeJson(Theme.Paper) }); await Task.Delay(80);
            await WindowCapture.Save(window.Hwnd, pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".toolbar-paper.png");
            preferences.SetSections(new() { ["theme"] = Preferences.ThemeJson(originalTheme) });
            var size = window.AppWindow.Size; window.AppWindow.Resize(new(1100, 450)); await Task.Delay(120);
            double toggleX = toolbar.FormattedView.TransformToVisual(toolbar).TransformPoint(new()).X;
            Check(Math.Abs(toggleX + toolbar.FormattedView.ActualWidth - (toolbar.ActualWidth - 10)) < 1
                && toolbar.Scroll.ScrollableWidth == 0,
                "wide toolbar aligns the formatted-view toggle at the right edge across a flexible gap");
            window.AppWindow.Resize(new(540, 450)); await Task.Delay(120);
            Check(toolbar.Scroll.ScrollableWidth > 0, "narrow Windows toolbar keeps all groups accessible by scrolling");
            toggleX = toolbar.FormattedView.TransformToVisual(toolbar).TransformPoint(new()).X;
            Check(toggleX >= 0 && toggleX + toolbar.FormattedView.ActualWidth <= toolbar.ActualWidth,
                "formatted-view toggle stays visible beside narrow toolbar overflow");
            toolbar.Scroll.ChangeView(toolbar.Scroll.ScrollableWidth, null, null, true); await Task.Delay(50);
            Check(toolbar.Scroll.HorizontalOffset > 0, "toolbar overflow scrolls to its trailing indent controls");
            Check(Math.Abs(toolbar.FormattedView.TransformToVisual(toolbar).TransformPoint(new()).X - toggleX) < 1,
                "scrolling formatting controls leaves the formatted-view toggle fixed");
            window.AppWindow.Resize(size);

            bool showMenu = preferences.ShowMenu;
            preferences.Set("windows", "showMenu", false);
            Check(toolbar.Visibility == Visibility.Visible, "toolbar visibility is independent of menu visibility");
            preferences.Set("windows", "showMenu", showMenu);
            window.ToolbarToggle.Focus(FocusState.Programmatic);
            await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space); await Task.Delay(50);
            Check(toolbar.Visibility == Visibility.Collapsed && !new Preferences(preferences.DirectoryPath).ShowFormattingToolbar(VIEM_FORMAT_MARKDOWN), "title-bar toggle persists the Markdown toolbar preference");

            preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN, true);
            var markdown = window.AddPane(new CoreDocument("- One\n  - Two\n\nPlain"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN));
            var md = await markdown.Ready;
            Check(toolbar.Visibility == Visibility.Visible && toolbar.FormattedView.IsChecked == true,
                "switching panes restores each format's toolbar preference and formatted-view state");
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
            md.SetMarkdownSource(true);
            Check(toolbar.Visibility == Visibility.Visible && toolbar.FormattedView.IsChecked == false,
                "Markdown Source has its own visible toolbar and an unpressed formatted-view toggle");
            preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN_SOURCE, false);
            md.SetMarkdownSource(false); Check(toolbar.Visibility == Visibility.Visible, "Source and WYSIWYG toolbar preferences are independent");
            pane.FocusEditor(); await Task.Delay(80);
            Check(toolbar.Visibility == Visibility.Visible, "returning to Markdown restores its own toolbar preference");
            preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN, true); preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN_SOURCE, true);

            foreach (uint format in new[] { VIEM_FORMAT_MARKDOWN, VIEM_FORMAT_MARKDOWN_SOURCE })
            {
                const string inner = "# Heading";
                var quotedPane = window.AddPane(new CoreDocument(Encoding.UTF8.GetBytes(inner), format: format));
                var quotedView = await quotedPane.Ready;
                await Click(toolbar, ToolbarAction.BlockQuote);
                Check(State(toolbar, ToolbarAction.BlockQuote) == true && quotedPane.Document.FormattedText().Contains("Heading"), "quote toggle adds an enclosing container");
                Check(Encoding.UTF8.GetString(quotedPane.Document.Source(quotedPane.Document.State.document_revision)) == "> # Heading", "quote toggle preserves heading syntax");
                await Click(toolbar, ToolbarAction.BlockQuote);
                Check(State(toolbar, ToolbarAction.BlockQuote) == false && Encoding.UTF8.GetString(quotedPane.Document.Source(quotedPane.Document.State.document_revision)) == inner, "quote toggle removes only quote treatment");
                quotedView.Undo();
                Check(State(toolbar, ToolbarAction.BlockQuote) == true, "quote toggle follows shared history");

                const string code = "Before\n\n```mermaid\ngraph LR\n    Writing --> Editing\n```\n\nAfter";
                var codePane = window.AddPane(new CoreDocument(Encoding.UTF8.GetBytes(code), format: format));
                var codeView = await codePane.Ready;
                var codeText = codePane.Document.FormattedText();
                codeView.Place((ulong)codeText.IndexOf("graph", StringComparison.Ordinal), VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, codePane.Document.State.document_revision);
                toolbar.Refresh();
                Check(toolbar.Buttons[ToolbarAction.BlockQuote].Visibility == Visibility.Visible && !toolbar.Buttons[ToolbarAction.BlockQuote].IsEnabled,
                    "code blocks keep the quote toggle visible but disabled");
                toolbar.Execute(ToolbarAction.BlockQuote);
                Check(Encoding.UTF8.GetString(codePane.Document.Source(codePane.Document.State.document_revision)) == code,
                    "unavailable quote action preserves code source");
                codeView.Place((ulong)codeText.IndexOf("After", StringComparison.Ordinal), VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, codePane.Document.State.document_revision);
                toolbar.Refresh();
                Check(toolbar.Buttons[ToolbarAction.BlockQuote].IsEnabled, "quote toggle re-enables outside code");
            }
            var tablePane = window.AddPane(new CoreDocument("> | H | V |\n> | --- | ---: |\n> | body | 7 |"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN_SOURCE));
            var tableView = await tablePane.Ready;
            ulong delimiter = (ulong)tablePane.Document.FormattedText().IndexOf("---", StringComparison.Ordinal);
            tableView.Place(delimiter, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, tablePane.Document.State.document_revision);
            toolbar.Refresh();
            Check(!toolbar.Paragraph.IsEnabled && !toolbar.Buttons[ToolbarAction.Bullets].IsEnabled && !toolbar.Buttons[ToolbarAction.Numbers].IsEnabled,
                "source table delimiter disables paragraph and list formatting");
            Check(toolbar.Buttons[ToolbarAction.BlockQuote].Visibility == Visibility.Visible && !toolbar.Buttons[ToolbarAction.BlockQuote].IsEnabled,
                "source table delimiter keeps the quote toggle visible but disabled");
            Check(toolbar.Buttons[ToolbarAction.CodeBlock].Visibility == Visibility.Collapsed,
                "source table delimiter omits the code block toggle");

            foreach (uint format in new[] { VIEM_FORMAT_PLAIN_TEXT, VIEM_FORMAT_CODE })
            {
                var literal = window.AddPane(new CoreDocument("text"u8.ToArray(), format: format)); await literal.Ready;
                Check(toolbar.Visibility == Visibility.Collapsed && window.ToolbarToggle.Visibility == Visibility.Collapsed, "literal format omits toolbar and title-bar toggle: " + format);
            }
            Check(window.Panes.All(p => p.LastError == null), "toolbar scenarios leave no presentation errors");
        }
        finally { App.Instance.Windows.Remove(window); window.Close(); }
        await MarkdownViewPreferenceTests.Run(preferences.DirectoryPath);
        await Performance(preferences);
    }

    private static async Task Performance(Preferences preferences)
    {
        string md = "# Heading\n\n- First item\n- Second item\n\n" + string.Concat(Enumerable.Repeat("A paragraph with **bold** and `code` text.\n\n", 4000));
        foreach (var (format, source) in new[] { (VIEM_FORMAT_MARKDOWN, md), (VIEM_FORMAT_MARKDOWN_SOURCE, md) })
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
                Check(State(toolbar, ToolbarAction.Bold) == true && pane.LastError == null && actionMs < 500,
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

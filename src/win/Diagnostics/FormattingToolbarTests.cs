#if DEBUG
using System.Diagnostics;
using System.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class FormattingToolbarTests
{
    private static void Check(bool condition, string name, string? details = null)
    { if (!condition) throw new InvalidOperationException(details == null ? name : $"{name} ({details})"); FrontendSmokeTests.UiChecks.Add(name); }
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
    private static string? Title(DropDownButton selector) => (selector.Content as StackPanel)?.Children.OfType<TextBlock>().SingleOrDefault()?.Text;
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

    internal static async Task RunStartup(Preferences preferences)
    {
        await LazyConstruction(preferences);
        await CharacterCaretContext(preferences);
        TablePickerTests.Run();
        await TableInteractionTests.Run(preferences);
        await LinkInteractionTests.Run(preferences);
        await ImageInteractionTests.Run(preferences);
    }

    private static async Task CharacterCaretContext(Preferences preferences)
    {
        const string source = "**bold** *italic* <u>underline</u> ~~strike~~ <sup>super</sup> <sub>sub</sub> `code` plain";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        var pane = window.ActivePane!; var view = await pane.Ready; var toolbar = window.Toolbar;
        try
        {
            await Task.Delay(100);
            foreach (var (action, word) in new[] { (ToolbarAction.Bold, "bold"), (ToolbarAction.Italic, "italic"),
                (ToolbarAction.Underline, "underline"), (ToolbarAction.Strikethrough, "strike"),
                (ToolbarAction.Superscript, "super"), (ToolbarAction.Subscript, "sub"), (ToolbarAction.CharacterCode, "code") })
            {
                view.Key(VIEM_KEY_ESCAPE);
                ulong offset = (ulong)document.FormattedText().IndexOf(word, StringComparison.Ordinal) + 1;
                view.Place(offset, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
                Check(view.Presentation.mode == VIEM_MODE_NORMAL && State(toolbar, action) == true && toolbar.Buttons[action].IsEnabled,
                    "Normal caret reflects active character formatting: " + action);
                ulong revision = document.State.document_revision;
                await Click(toolbar, action);
                Check(view.Presentation.mode == VIEM_MODE_INSERT && State(toolbar, action) == false && document.State.document_revision == revision,
                    "Normal character toggle starts Insert with pending cleared formatting: " + action);
                view.Text("X"); view.Key(VIEM_KEY_ESCAPE); view.Undo();
                Check(Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                    "Normal character toggle and insertion retain exact undo: " + action);
            }
            document.SetReadOnly(true);
            foreach (var (action, word) in new[] { (ToolbarAction.Bold, "bold"), (ToolbarAction.Italic, "italic"),
                (ToolbarAction.Underline, "underline"), (ToolbarAction.Strikethrough, "strike"),
                (ToolbarAction.Superscript, "super"), (ToolbarAction.Subscript, "sub"), (ToolbarAction.CharacterCode, "code") })
            {
                view.Place((ulong)document.FormattedText().IndexOf(word, StringComparison.Ordinal) + 1,
                    VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
                toolbar.Refresh();
                Check(State(toolbar, action) == true && !toolbar.Buttons[action].IsEnabled && toolbar.Buttons[action].Visibility == Visibility.Visible,
                    "read-only character controls retain their active caret state while disabled: " + action);
                ulong revision = document.State.document_revision;
                toolbar.Execute(action);
                Check(view.Presentation.mode == VIEM_MODE_NORMAL && document.State.document_revision == revision,
                    "a disabled read-only character action preserves mode and source revision: " + action);
            }
            document.SetReadOnly(false);
            view.Key(VIEM_KEY_ESCAPE); view.Command("G$");
            await Click(toolbar, ToolbarAction.Bold);
            Check(view.Presentation.mode == VIEM_MODE_INSERT && State(toolbar, ToolbarAction.Bold) == true,
                "Normal character toggle starts Insert with newly requested formatting");
            Check(pane.LastError == null, "caret character formatting tests finish without native presentation errors");
        }
        finally { await window.ClosePane(pane, force: true); App.Instance.Windows.Remove(window); }
    }

    internal static async Task Run(Preferences preferences)
    {
        await RunStartup(preferences);
        const string source = "# Heading\n\nplain `code`";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        var pane = window.ActivePane!; var view = await pane.Ready; var toolbar = window.Toolbar;
        async Task CloseFixture(EditorPane fixture)
        {
            Check(fixture.LastError == null, "completed toolbar fixture has no presentation error", fixture.LastError?.ToString());
            await window.ClosePane(fixture, force: true);
        }
        try
        {
            await Task.Delay(150);
            Check(window.ToolbarToggle.Visibility == Visibility.Visible && toolbar.Visibility == Visibility.Visible, "Markdown toolbar defaults visible");
            Check(Grid.GetColumn(window.ToolbarToggle) == Grid.GetColumn(window.MenuToggle) + 1, "toolbar toggle immediately follows the menu toggle");
            Check(Title(toolbar.Paragraph) == "Heading 1", "toolbar reads the initial named paragraph");
            Check(((StackPanel)toolbar.Paragraph.Content).Children.First() is IconElement
                && ((MenuFlyout)toolbar.Paragraph.Flyout).Items.OfType<ToggleMenuFlyoutItem>().All(i => i.Icon != null),
                "toolbar style selectors annotate every style with its type icon");
            var toolbarRow = (StackPanel)toolbar.Scroll.Content;
            var characterGroup = (StackPanel)toolbar.Buttons[ToolbarAction.Bold].Parent;
            Check(characterGroup.Children.OfType<ButtonBase>().Select(b => AutomationProperties.GetName(b))
                .SequenceEqual(new[] { "Bold", "Italic", "Underline", "Strikethrough" }), "toolbar emphasis order includes underline between italic and strikeout");
            var scripts = (StackPanel)toolbar.Buttons[ToolbarAction.Superscript].Parent;
            foreach (var action in new[] { ToolbarAction.Superscript, ToolbarAction.Subscript })
            {
                var icon = (Canvas)toolbar.Buttons[action].Content;
                var glyphs = icon.Children.OfType<TextBlock>().ToArray();
                Check(glyphs.Length == 2 && glyphs[0].Text == "x" && glyphs[0].FontStyle == global::Windows.UI.Text.FontStyle.Italic
                    && Math.Abs(glyphs[0].FontSize - 11.7) < .01 && glyphs[1].Text == "2" && glyphs[1].FontSize < glyphs[0].FontSize,
                    "script toolbar icons use a smaller italic x and small numeral: " + action);
            }
            var initialStyles = view.Styles(view.SelectedNamedStyles().Identity);
            var supportedStyles = initialStyles.Styles.Where(s => s.Namespace == 1 && s.Has(VIEM_STYLE_CAPABILITY_ASSIGN)).Select(s => s.Key).ToHashSet();
            Check(((MenuFlyout)toolbar.Paragraph.Flyout).Items.Select(i => (StyleKey)i.Tag).ToHashSet().SetEquals(supportedStyles),
                "WYSIWYG paragraph selector lists only styles supported for source assignment");
            foreach (var listStyle in initialStyles.Styles.Where(s => (s.Native.flags & VIEM_STYLE_DEFINITION_INTERNAL_LIST) != 0 || s.Native.role == VIEM_STYLE_ROLE_LIST))
                Check(StyleIcons.Kind(listStyle) == StyleIconKind.Paragraph, "list definitions use paragraph style icons: " + listStyle.Name);
            foreach (var action in new[] { ToolbarAction.Bullets, ToolbarAction.Numbers })
            {
                view.Command("j0");
                await Click(toolbar, action);
                var list = view.SelectedNamedStyles();
                var current = ((MenuFlyout)toolbar.Paragraph.Flyout).Items.OfType<ToggleMenuFlyoutItem>()
                    .Single(item => ((StyleKey)item.Tag).Id == list.Paragraph);
                Check(current.IsChecked && !current.IsEnabled && current.Icon is FontIcon { Glyph: "¶" }
                    && Title(toolbar.Paragraph) == current.Text,
                    "current list paragraph retains its exact disabled style row: " + action);
                view.Undo(); view.Command("gg0");
            }
            var codeLink = (StackPanel)toolbar.Buttons[ToolbarAction.CharacterCode].Parent;
            int scriptIndex = toolbarRow.Children.IndexOf(scripts), codeIndex = toolbarRow.Children.IndexOf(codeLink), tableIndex = toolbarRow.Children.IndexOf(toolbar.InsertTable);
            Check(toolbarRow.Children[0] == characterGroup && scriptIndex == 1 && codeIndex == scriptIndex + 1
                && !toolbarRow.Children.OfType<Border>().Any(),
                "toolbar groups use ordinary gaps and place script immediately before code and link");
            Check(toolbar.InsertImage.Parent == toolbarRow.Children[tableIndex - 1]
                && toolbar.Buttons[ToolbarAction.Indent].Parent == toolbarRow.Children[tableIndex + 1]
                && toolbarRow.Children[tableIndex + 2] == toolbar.Paragraph && toolbarRow.Children[tableIndex + 3] == toolbar.Character,
                "table precedes indent and the two style selectors trail the command buttons");
            toolbar.UpdateLayout();
            double Left(FrameworkElement element) => element.TransformToVisual(toolbarRow).TransformPoint(new()).X;
            double Gap(FrameworkElement before, FrameworkElement after) => Left(after) - Left(before) - before.ActualWidth;
            double standardGap = Gap(codeLink, (FrameworkElement)toolbar.InsertImage.Parent);
            Check(Math.Abs(standardGap - 8) < 1 && toolbarRow.Children.OfType<FrameworkElement>().Zip(toolbarRow.Children.OfType<FrameworkElement>().Skip(1))
                .All(pair => Math.Abs(Gap(pair.First, pair.Second) - standardGap) < 1),
                "every toolbar group uses the same gap as Link to Bulleted List");
            Check(toolbar.FormattedView.IsChecked == true && !toolbar.FormattedView.IsThreeState
                && AutomationProperties.GetName(toolbar.FormattedView) == "Formatted view"
                && ToolTipService.GetToolTip(toolbar.FormattedView) as string == "Formatted view (WYSIWYG)",
                "formatted-view toggle has a stable accessible label and WYSIWYG pressed state");
            object viewIcon = toolbar.FormattedView.Content;
            byte[] beforeSwitch = document.Source(document.State.document_revision);
            ulong caret = (ulong)document.FormattedText().IndexOf("plain", StringComparison.Ordinal) + 2;
            view.Place(caret, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            ulong placedCaret = view.Presentation.cursor_utf8_offset;
            await SwitchView(toolbar);
            // Presentation offsets are in the current logical projection;
            // Source view keeps syntax but folds a paragraph separator to LF.
            ulong sourceCaret = (ulong)document.FormattedText().IndexOf("plain", StringComparison.Ordinal) + 2;
            Check(document.State.format == VIEM_FORMAT_MARKDOWN_SOURCE && toolbar.FormattedView.IsChecked == false
                && view.Presentation.cursor_utf8_offset == sourceCaret
                && beforeSwitch.AsSpan().SequenceEqual(document.Source(document.State.document_revision)),
                "toolbar switches to Markdown Source while preserving source and semantic caret",
                $"format={document.State.format}, checked={toolbar.FormattedView.IsChecked}, caret={view.Presentation.cursor_utf8_offset}, expected={sourceCaret}, placed={placedCaret}, requested={caret}, sourceEqual={beforeSwitch.AsSpan().SequenceEqual(document.Source(document.State.document_revision))}, mode={view.Presentation.mode}, error={pane.LastError}");
            var sourceStyleIds = ((MenuFlyout)toolbar.Paragraph.Flyout).Items.Select(i => ((StyleKey)i.Tag).Id).ToHashSet();
            Check(new[] { "Table", "Table cell", "Table header", "Image" }.All(sourceStyleIds.Contains),
                "Source paragraph catalogue retains structural presentation styles");
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
            Check(Title(toolbar.Paragraph) == "Base Paragraph", "toolbar follows a caret command synchronously");
            var items = ((MenuFlyout)toolbar.Paragraph.Flyout).Items.ToArray();
            var sheet = view.Styles(view.SelectedNamedStyles().Identity);
            view.Command("w");
            Check(Title(toolbar.Character) == "Code", "toolbar tracks the Code character assignment");
            Check(items.SequenceEqual(((MenuFlyout)toolbar.Paragraph.Flyout).Items) && ReferenceEquals(sheet, view.Styles(view.SelectedNamedStyles().Identity)),
                "cursor movement retains native menu items and the exact stylesheet snapshot");
            view.SelectAll();
            Check(Title(toolbar.Paragraph) == "Mixed" && Title(toolbar.Character) == "Mixed", "toolbar selectors show mixed assignments");
            await Choose(toolbar.Paragraph, "Heading2");
            Check(Title(toolbar.Paragraph) == "Heading 2" && pane.LastError == null, "toolbar selector invokes the shared heading action");
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
            Check(view.Presentation.mode == VIEM_MODE_VISUAL_BLOCK && !toolbar.Buttons[ToolbarAction.Bold].IsEnabled && !toolbar.Paragraph.IsEnabled && pane.LastError == null,
                "Visual Block disables unsupported toolbar actions without errors",
                $"mode={view.Presentation.mode}, formatting={view.HasFormattingSelection}, bold={toolbar.Buttons[ToolbarAction.Bold].IsEnabled}, paragraph={toolbar.Paragraph.IsEnabled}, error={pane.LastError}");
            view.Key(VIEM_KEY_ESCAPE); view.Command(":");
            Check(!toolbar.Paragraph.IsEnabled && !toolbar.Character.IsEnabled && pane.LastError == null, "command prompt disables formatting without stale selection errors");
            view.Key(VIEM_KEY_ESCAPE);

            // Save the native chrome in both appearances and verify overflow.
            await WindowCapture.Save(window.Hwnd, pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".toolbar.png");
            var originalTheme = preferences.Theme;
            preferences.SetSections(new() { ["theme"] = Preferences.ThemeJson(Theme.Paper) }); await Task.Delay(80);
            await WindowCapture.Save(window.Hwnd, pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".toolbar-paper.png");
            preferences.SetSections(new() { ["theme"] = Preferences.ThemeJson(originalTheme) });
            var size = window.AppWindow.Size;
            double scale = toolbar.XamlRoot.RasterizationScale;
            void Resize(double width, double height) => window.AppWindow.Resize(new((int)Math.Round(width * scale), (int)Math.Round(height * scale)));
            Resize(1100, 450); await Task.Delay(120);
            double toggleX = toolbar.FormattedView.TransformToVisual(toolbar).TransformPoint(new()).X;
            Check(Math.Abs(toggleX + toolbar.FormattedView.ActualWidth - (toolbar.ActualWidth - 10)) < 1
                && toolbar.Scroll.ScrollableWidth == 0,
                "wide toolbar aligns the formatted-view toggle at the right edge across a flexible gap",
                $"toggleX={toggleX}, toggleWidth={toolbar.FormattedView.ActualWidth}, toolbarWidth={toolbar.ActualWidth}, scrollable={toolbar.Scroll.ScrollableWidth}, scale={scale}");
            Resize(700, 450); await Task.Delay(120);
            toolbar.Scroll.ChangeView(0, null, null, true); await Task.Delay(50);
            double unindentX = toolbar.Buttons[ToolbarAction.Unindent].TransformToVisual(toolbar.Scroll).TransformPoint(new()).X;
            double paragraphX = toolbar.Paragraph.TransformToVisual(toolbar.Scroll).TransformPoint(new()).X;
            double characterX = toolbar.Character.TransformToVisual(toolbar.Scroll).TransformPoint(new()).X;
            Check(unindentX + toolbar.Buttons[ToolbarAction.Unindent].ActualWidth <= toolbar.Scroll.ActualWidth
                && paragraphX < toolbar.Scroll.ActualWidth && paragraphX + toolbar.Paragraph.ActualWidth > toolbar.Scroll.ActualWidth
                && characterX >= toolbar.Scroll.ActualWidth,
                "shrinking the toolbar clips Paragraph then Character while every earlier command remains visible",
                $"unindentRight={unindentX + toolbar.Buttons[ToolbarAction.Unindent].ActualWidth}, paragraphX={paragraphX}, characterX={characterX}, viewport={toolbar.Scroll.ActualWidth}");
            Resize(540, 450); await Task.Delay(120);
            toolbar.Scroll.ChangeView(0, null, null, true); await Task.Delay(50);
            Check(toolbar.Scroll.ScrollableWidth > 0, "narrow Windows toolbar keeps all groups accessible by scrolling");
            toggleX = toolbar.FormattedView.TransformToVisual(toolbar).TransformPoint(new()).X;
            Check(toggleX >= 0 && toggleX + toolbar.FormattedView.ActualWidth <= toolbar.ActualWidth,
                "formatted-view toggle stays visible beside narrow toolbar overflow");
            double scrollX = toolbar.Scroll.TransformToVisual(toolbar).TransformPoint(new()).X;
            Check(scrollX + toolbar.Scroll.ActualWidth <= toggleX && toolbar.Scroll.Clip is RectangleGeometry clip
                && Math.Abs(clip.Rect.Width - toolbar.Scroll.ActualWidth) < 1 && Math.Abs(clip.Rect.Height - toolbar.Scroll.ActualHeight) < 1,
                "the scroll viewport clips later controls before the fixed formatted-view toggle");
            Check(toolbar.Paragraph.TransformToVisual(toolbar.Scroll).TransformPoint(new()).X >= toolbar.Scroll.ActualWidth
                && Left(toolbar.Paragraph) >= Left((FrameworkElement)toolbar.Buttons[ToolbarAction.Unindent].Parent)
                    + ((FrameworkElement)toolbar.Buttons[ToolbarAction.Unindent].Parent).ActualWidth,
                "narrow windows clip the trailing style selectors after the earlier command buttons");
            Check(toolbarRow.Children.OfType<FrameworkElement>().Zip(toolbarRow.Children.OfType<FrameworkElement>().Skip(1))
                .All(pair => Gap(pair.First, pair.Second) >= standardGap - 1),
                "narrow toolbar layout never lets later controls overlap earlier groups");
            toolbar.Scroll.ChangeView(toolbar.Scroll.ScrollableWidth, null, null, true); await Task.Delay(50);
            Check(toolbar.Scroll.HorizontalOffset > 0 && toolbar.Character.TransformToVisual(toolbar.Scroll).TransformPoint(new()).X + toolbar.Character.ActualWidth <= toolbar.Scroll.ActualWidth + 1,
                "toolbar overflow scrolls to its trailing style selectors");
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
            Check(State(toolbar, ToolbarAction.Bullets) == true && Title(toolbar.Paragraph) == "Mixed", "list state is structural across nested paragraph styles");
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
            await CloseFixture(markdown);

            foreach (uint format in new[] { VIEM_FORMAT_MARKDOWN, VIEM_FORMAT_MARKDOWN_SOURCE })
            {
                const string inner = "# Heading";
                var quotedPane = window.AddPane(new CoreDocument(Encoding.UTF8.GetBytes(inner), format: format));
                var quotedView = await quotedPane.Ready;
                await Click(toolbar, ToolbarAction.BlockQuote);
                Check(State(toolbar, ToolbarAction.BlockQuote) == true && quotedPane.Document.FormattedText().Contains("Heading"), "quote toggle adds an enclosing container");
                Check(Encoding.UTF8.GetString(quotedPane.Document.Source(quotedPane.Document.State.document_revision)) == "> # Heading", "quote toggle preserves heading syntax",
                    $"format={format}, source={Encoding.UTF8.GetString(quotedPane.Document.Source(quotedPane.Document.State.document_revision)).Replace("\n", "\\n")}, mode={quotedView.Presentation.mode}, error={quotedPane.LastError}");
                await Click(toolbar, ToolbarAction.BlockQuote);
                Check(State(toolbar, ToolbarAction.BlockQuote) == false && Encoding.UTF8.GetString(quotedPane.Document.Source(quotedPane.Document.State.document_revision)) == inner, "quote toggle removes only quote treatment");
                quotedView.Undo();
                Check(State(toolbar, ToolbarAction.BlockQuote) == true, "quote toggle follows shared history");
                await CloseFixture(quotedPane);

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
                var codeStyle = codeView.Styles().Styles.Single(style => style.Key == new StyleKey(1, "Code Block"));
                codeView.EditStyle(codeStyle, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH,
                    CoreView.Enum(VIEM_STYLE_VALUE_BOOLEAN, 1));
                toolbar.Refresh();
                Check(State(toolbar, ToolbarAction.Strikethrough) == true && !toolbar.Buttons[ToolbarAction.Strikethrough].IsEnabled,
                    "literal code keeps resolved strikethrough appearance visible while disabling document formatting: " + format);
                Check(toolbar.Buttons[ToolbarAction.CharacterCode].Visibility == Visibility.Visible
                    && !toolbar.Buttons[ToolbarAction.CharacterCode].IsEnabled && !toolbar.Character.IsEnabled,
                    "literal code retains its character toolbar controls and disables unsupported assignments: " + format);
                ulong literalRevision = codePane.Document.State.document_revision;
                toolbar.Execute(ToolbarAction.Strikethrough);
                toolbar.Execute(ToolbarAction.CharacterCode);
                Check(codeView.Presentation.mode == VIEM_MODE_NORMAL && codePane.Document.State.document_revision == literalRevision
                    && Encoding.UTF8.GetString(codePane.Document.Source(codePane.Document.State.document_revision)) == code,
                    "disabled literal-code strikethrough preserves its mode and exact source: " + format);
                codeView.Undo();
                codeView.Place((ulong)codeText.IndexOf("After", StringComparison.Ordinal), VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, codePane.Document.State.document_revision);
                toolbar.Refresh();
                Check(toolbar.Buttons[ToolbarAction.BlockQuote].IsEnabled, "quote toggle re-enables outside code");
                await CloseFixture(codePane);
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
            await CloseFixture(tablePane);

            foreach (uint format in new[] { VIEM_FORMAT_PLAIN_TEXT, VIEM_FORMAT_CODE })
            {
                var literal = window.AddPane(new CoreDocument("text"u8.ToArray(), format: format)); await literal.Ready;
                Check(toolbar.Visibility == Visibility.Collapsed && window.ToolbarToggle.Visibility == Visibility.Collapsed, "literal format omits toolbar and title-bar toggle: " + format);
                await CloseFixture(literal);
            }
            Check(window.Panes.All(p => p.LastError == null), "toolbar scenarios leave no presentation errors");
        }
        finally { App.Instance.Windows.Remove(window); window.Close(); }
        await MarkdownViewPreferenceTests.Run(preferences.DirectoryPath);
        await Performance(preferences);
    }

    private static async Task LazyConstruction(Preferences preferences)
    {
        bool formatted = preferences.ShowFormattingToolbar(VIEM_FORMAT_MARKDOWN);
        bool source = preferences.ShowFormattingToolbar(VIEM_FORMAT_MARKDOWN_SOURCE);
        try {
            foreach (uint format in new[] { VIEM_FORMAT_PLAIN_TEXT, VIEM_FORMAT_CODE, VIEM_FORMAT_MARKDOWN, VIEM_FORMAT_MARKDOWN_SOURCE }) {
                preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN, false);
                preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN_SOURCE, false);
                var document = new CoreDocument("Text"u8.ToArray(), format: format);
                var window = new EditorWindow(preferences, document);
                App.Instance.Windows.Add(window);
                try {
                    Check(!window.ToolbarCreated, "hidden or unavailable toolbar is not constructed with its window: " + format);
                    window.Activate();
                    var pane = window.ActivePane!; var view = await pane.Ready;
                    view.Command("i"); view.Text("X"); view.Key(VIEM_KEY_ESCAPE);
                    Check(!window.ToolbarCreated && document.FormattedText() == "XText" && pane.LastError == null,
                        "first editing commands work without constructing hidden formatting controls: " + format);
                    if (format is not (VIEM_FORMAT_MARKDOWN or VIEM_FORMAT_MARKDOWN_SOURCE)) {
                        Check(window.ToolbarToggle.Visibility == Visibility.Collapsed, "literal startup omits the formatting toggle: " + format);
                        continue;
                    }
                    await Task.Delay(80);
                    Check(window.ToolbarToggle.Focus(FocusState.Programmatic), "the first formatting-toolbar toggle takes native focus");
                    await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space); await Task.Delay(50);
                    var toolbar = window.Toolbar;
                    Check(toolbar.Visibility == Visibility.Visible && Title(toolbar.Paragraph) == "Base Paragraph"
                        && document.FormattedText() == "XText", "first native toolbar activation creates current controls without editing text");
                    preferences.SetFormattingToolbar(format, false);
                    preferences.SetFormattingToolbar(format, true);
                    Check(ReferenceEquals(toolbar, window.Toolbar) && toolbar.Visibility == Visibility.Visible,
                        "hiding and reopening retains the first toolbar instance");
                } finally { App.Instance.Windows.Remove(window); window.Close(); }
            }

            preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN, true);
            var visibleWindow = new EditorWindow(preferences, new CoreDocument("# Heading\n\nText"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN));
            App.Instance.Windows.Add(visibleWindow);
            try {
                Check(visibleWindow.ToolbarCreated && visibleWindow.Toolbar.Visibility == Visibility.Visible,
                    "visible Markdown constructs and reserves its toolbar before window activation");
                var pane = visibleWindow.ActivePane!;
                var frames = new List<(double Height, double Top)>();
                pane.Canvas.Draw += (_, _) => frames.Add((pane.Canvas.ActualHeight, pane.Canvas.TransformToVisual(null).TransformPoint(new()).Y));
                visibleWindow.Activate(); await pane.Ready; await Task.Delay(150);
                Check(frames.Count > 0 && frames.All(frame => Math.Abs(frame.Height - frames[0].Height) < .01 && Math.Abs(frame.Top - frames[0].Top) < .01),
                    "visible Markdown's first drawn frame already has the final toolbar geometry");
            } finally { App.Instance.Windows.Remove(visibleWindow); visibleWindow.Close(); }
        } finally {
            preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN, formatted);
            preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN_SOURCE, source);
        }
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

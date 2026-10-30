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
using Windows.System;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class FormattingToolbarTests
{
    private static void Check(bool condition, string name, string? details = null)
    {
        if (!condition) throw new InvalidOperationException(details == null ? name : $"{name} ({details})");
        FrontendSmokeTests.UiChecks.Add(name);
    }
    private static IEnumerable<T> Descendants<T>(DependencyObject root) where T : DependencyObject
    {
        if (root is T match) yield return match;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(root); i++)
            foreach (var child in Descendants<T>(VisualTreeHelper.GetChild(root, i))) yield return child;
    }
    private static ToggleMenuFlyoutItem Entry(DropDownButton selector, string id) => ((MenuFlyout)selector.Flyout).Items
        .OfType<ToggleMenuFlyoutItem>().Single(i => i.Tag is StyleKey key && key.Id == id);
    private static async Task Choose(DropDownButton selector, string id)
    {
        selector.Focus(FocusState.Programmatic);
        selector.Flyout.ShowAt(selector);
        await InputRoutingTests.Settle();
        Check(Entry(selector, id).Focus(FocusState.Programmatic), "toolbar style entry takes keyboard focus");
        await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
        selector.Flyout.Hide();
        await InputRoutingTests.Settle();
    }
    private static string? Title(DropDownButton selector) => (selector.Content as StackPanel)?.Children.OfType<TextBlock>().SingleOrDefault()?.Text;
    private static bool? State(FormattingToolbar toolbar, ToolbarAction action) => ((ToggleButton)toolbar.Buttons[action]).IsChecked;
    private static async Task Click(FormattingToolbar toolbar, ToolbarAction action)
    {
        Check(toolbar.Buttons[action].Focus(FocusState.Programmatic), "toolbar action takes keyboard focus: " + action);
        await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
        await InputRoutingTests.Settle();
    }
    private static async Task SwitchView(FormattingToolbar toolbar)
    {
        Check(toolbar.FormattedView.Focus(FocusState.Programmatic), "formatted-view toggle takes keyboard focus");
        await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
        await InputRoutingTests.Settle();
    }
    private static async Task PreviewFromTitleBar(EditorWindow window)
    {
        Check(window.PreviewToggle.Focus(FocusState.Programmatic), "title-bar Markdown Preview button takes keyboard focus");
        await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
        await InputRoutingTests.Settle();
    }
    private static bool PreviewChrome(EditorWindow window, FormattingToolbar toolbar, bool toolbarVisible) =>
        window.ToolbarToggle.Visibility == Visibility.Visible && window.PreviewToggle.Visibility == Visibility.Collapsed
        && toolbar.Visibility == (toolbarVisible ? Visibility.Visible : Visibility.Collapsed);
    private static bool LiteralMarkdownChrome(EditorWindow window, FormattingToolbar toolbar) =>
        window.ToolbarToggle.Visibility == Visibility.Collapsed && window.PreviewToggle.Visibility == Visibility.Visible
        && toolbar.Visibility == Visibility.Collapsed;

    internal static async Task RunShortcuts(Preferences preferences)
    {
        foreach (var kind in Enum.GetValues<StyleIconKind>())
        {
            var icon = StyleIcons.Icon(kind);
            // Do not construct the expected subtype here: that would retain
            // metadata in diagnostics that the shipping native build needs.
            string expectedType = kind == StyleIconKind.Block ? "PathIcon"
                : kind == StyleIconKind.Document ? "SymbolIcon" : "FontIcon";
            Check(icon.GetType().FullName == "Microsoft.UI.Xaml.Controls." + expectedType
                && AutomationProperties.GetName(icon) == StyleIcons.TypeName(kind),
                "native style icons retain their concrete projection and accessible type name: " + kind);
        }
        await CharacterCaretContext(preferences);
        await KeyboardShortcuts(preferences);
    }

    private static async Task KeyboardShortcuts(Preferences preferences)
    {
        foreach (uint format in new[] { VIEM_FORMAT_MARKDOWN })
        {
            const string source = "alpha beta";
            var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: format);
            bool showToolbar = preferences.ShowFormattingToolbar(format);
            preferences.SetFormattingToolbar(format, false);
            var window = new EditorWindow(preferences, document);
            App.Instance.Windows.Add(window);
            window.Activate();
            var pane = window.ActivePane!;
            var view = await pane.Ready;
            try
            {
                await InputRoutingTests.Settle();
                pane.FocusEditor();
                Check(!window.ToolbarCreated, "hidden toolbar starts unconstructed");
                foreach (var (action, key, shift) in new[] {
                    (ToolbarAction.CharacterCode, VirtualKey.E, true),
                    (ToolbarAction.Superscript, (VirtualKey)190, false),
                    (ToolbarAction.Subscript, (VirtualKey)188, false),
                    (ToolbarAction.Bullets, VirtualKey.Number8, true),
                    (ToolbarAction.Numbers, VirtualKey.Number7, true) })
                {
                    view.SelectAll();
                    await InputRoutingTests.Key(key, control: true, shift: shift);
                    Check(action switch
                    {
                        ToolbarAction.CharacterCode => view.SelectedNamedStyles().Character == "Code" && window.Toolbar.Visibility == Visibility.Collapsed,
                        ToolbarAction.Superscript => view.SuperscriptState() == VIEM_SEMANTIC_STYLE_STATE_ON,
                        ToolbarAction.Subscript => view.SubscriptState() == VIEM_SEMANTIC_STYLE_STATE_ON,
                        ToolbarAction.Bullets => view.SelectedNamedStyles().ListState(VIEM_LIST_STYLE_BULLET) == VIEM_SEMANTIC_STYLE_STATE_ON,
                        _ => view.SelectedNamedStyles().ListState(VIEM_LIST_STYLE_NUMBERED) == VIEM_SEMANTIC_STYLE_STATE_ON
                    }, $"native formatting shortcut applies {action} in format {format}");
                    view.Key(VIEM_KEY_ESCAPE);
                    view.Undo();
                }
                view.SelectAll();
                await InputRoutingTests.Key(VirtualKey.K, control: true, shift: true);
                Check(pane.LinkEditorVisible && pane.LinkTextValue == source, $"native link shortcut opens the selected text in format {format}");
                pane.DismissLinkPopup();
                pane.FocusEditor();
                view.EditLink(view.LinkContext(), source, "https://example.test");
                view.Key(VIEM_KEY_ESCAPE);
                view.Place(2, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
                await InputRoutingTests.Key(VirtualKey.K, control: true, shift: true);
                Check(pane.LinkEditorVisible && pane.LinkDestinationValue == "https://example.test", "link shortcut edits an existing link");
                pane.DismissLinkPopup();
                pane.FocusEditor();
                view.Undo();
                view.Key(VIEM_KEY_ESCAPE);
                view.Command("d");
                Check(!pane.CanUseFormattingCommands, "pending Vim input disables formatting");
                view.Key(VIEM_KEY_ESCAPE);
                view.Command(":");
                Check(!pane.CanUseFormattingCommands, "command prompts disable formatting");
                view.Key(VIEM_KEY_ESCAPE);
                view.Command("i");
                view.BeginComposition();
                Check(!pane.CanUseFormattingCommands, "composition disables formatting");
                view.CancelComposition();
                view.Key(VIEM_KEY_ESCAPE);
                document.SetReadOnly(true);
                Check(!pane.CanUseFormattingCommands, "read-only buffers disable formatting");
                document.SetReadOnly(false);
                var gate = new TaskCompletionSource<bool>();
                view.Command("g");
                pane.HoldInputForTest(gate.Task);
                pane.FocusEditor();
                try
                {
                    await InputRoutingTests.Text("g");
                    await InputRoutingTests.Key((VirtualKey)190, control: true);
                }
                finally
                {
                    gate.SetResult(true);
                }
                await InputRoutingTests.Settle();
                Check(view.Presentation.mode == VIEM_MODE_INSERT && view.SuperscriptState() == VIEM_SEMANTIC_STYLE_STATE_ON,
                    "formatting candidate waits for queued text to finish the Vim command");
                view.Text("X");
                view.Key(VIEM_KEY_ESCAPE);
                view.Undo();
                Check(Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                    "queued formatting shortcut and typing preserve exact undo");
                view.Key(VIEM_KEY_ESCAPE);
                view.Command("i");
                view.Key(VIEM_KEY_CONTROL_CHARACTER, 'q');
                await InputRoutingTests.Key(VirtualKey.K, control: true, shift: true);
                Check(!pane.LinkEditorVisible && Encoding.UTF8.GetString(document.Source(document.State.document_revision)).Contains('\u000b'),
                    "literal Ctrl+Shift+K inserts Vim Ctrl+K instead of opening a link");
                Check(pane.LastError == null, "formatting shortcut diagnostics have no native errors", pane.LastError?.ToString());
            }
            finally
            {
                preferences.SetFormattingToolbar(format, showToolbar);
                await window.ClosePane(pane, force: true);
                App.Instance.Windows.Remove(window);
            }
        }
    }

    private static async Task CharacterCaretContext(Preferences preferences)
    {
        const string source = "**bold** *italic* <u>underline</u> ~~strike~~ <sup>super</sup> <sub>sub</sub> `code` plain";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window);
        window.Activate();
        var pane = window.ActivePane!;
        var view = await pane.Ready;
        var toolbar = window.Toolbar;
        try
        {
            await InputRoutingTests.Settle();
            foreach (var (action, word) in new[] { (ToolbarAction.Bold, "bold"), (ToolbarAction.Superscript, "super"), (ToolbarAction.CharacterCode, "code") })
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
                view.Text("X");
                view.Key(VIEM_KEY_ESCAPE);
                view.Undo();
                Check(Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                    "Normal character toggle and insertion retain exact undo: " + action);
            }
            document.SetReadOnly(true);
            foreach (var (action, word) in new[] { (ToolbarAction.Bold, "bold"), (ToolbarAction.Superscript, "super"), (ToolbarAction.CharacterCode, "code") })
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
            view.Key(VIEM_KEY_ESCAPE);
            view.Command("G$");
            await Click(toolbar, ToolbarAction.Bold);
            Check(view.Presentation.mode == VIEM_MODE_INSERT && State(toolbar, ToolbarAction.Bold) == true,
                "Normal character toggle starts Insert with newly requested formatting");
            Check(pane.LastError == null, "caret character formatting tests finish without native presentation errors");
        }
        finally
        {
            await window.ClosePane(pane, force: true);
            App.Instance.Windows.Remove(window);
        }
    }

    private const string source = "# Heading\n\nplain `code`";

    internal static SelfTestRunner.Stage Group(Preferences preferences)
    {
        bool visible = true;
        return TestEditor.Group("Toolbar", preferences, source, [
            new("AppearanceAndIcons", editor => RunCase(editor, fixture => fixture.AppearanceAndIcons())),
            new("CurrentListStyle", editor => RunCase(editor, fixture => fixture.CurrentListStyle())),
            new("GroupSpacing", editor => RunCase(editor, fixture => fixture.GroupSpacing())),
            new("PreviewRoundTrip", editor => RunCase(editor, fixture => fixture.PreviewRoundTrip())),
            new("StyleAssignmentUndo", editor => RunCase(editor, fixture => fixture.StyleAssignmentUndo())),
            new("CharacterFormattingUndo", editor => RunCase(editor, fixture => fixture.CharacterFormattingUndo())),
            new("PendingTypingFormat", editor => RunCase(editor, fixture => fixture.PendingTypingFormat())),
            new("StaleStyleIdentity", editor => RunCase(editor, fixture => fixture.StaleStyleIdentity())),
            new("UnsupportedInputModes", editor => RunCase(editor, fixture => fixture.UnsupportedInputModes())),
            new("NarrowLayout", editor => RunCase(editor, fixture => fixture.NarrowLayout())),
            new("VisibilityPreferences", editor => RunCase(editor, fixture => fixture.VisibilityPreferences())),
            new("NestedListsAndPreview", editor => RunCase(editor, fixture => fixture.NestedListsAndPreview())),
            new("QuoteTreatment", editor => RunCase(editor, fixture => fixture.QuoteTreatment())),
            new("AlertTreatment", editor => RunCase(editor, fixture => fixture.AlertTreatment())),
            new("CodeBlockProtection", editor => RunCase(editor, fixture => fixture.CodeBlockProtection())),
            new("LiteralFormats", editor => RunCase(editor, fixture => fixture.LiteralFormats())),
        ], initialize: () =>
        {
            visible = preferences.ShowFormattingToolbar(VIEM_FORMAT_MARKDOWN);
            if (!visible) preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN, true);
        }, cleanup: () => preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN, visible),
            resetCase: editor => editor.RestoreCaret(source));

        async Task RunCase(TestEditor editor, Func<ToolbarFixture, Task> test)
        {
            if (!preferences.ShowFormattingToolbar(VIEM_FORMAT_MARKDOWN))
                preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN, true);
            await test(new ToolbarFixture(editor, preferences));
            Check(editor.Window.Panes.All(p => p.LastError == null), "toolbar case leaves no presentation errors");
        }
    }

    private sealed class ToolbarFixture
    {
        private readonly EditorWindow window;
        private readonly EditorPane pane;
        private readonly CoreView view;
        private readonly CoreDocument document;
        private readonly FormattingToolbar toolbar;
        private readonly Preferences preferences;
        private StackPanel toolbarRow => (StackPanel)toolbar.Viewport.Children.Single();
        private StackPanel characterGroup => (StackPanel)toolbar.Buttons[ToolbarAction.Bold].Parent;
        private StackPanel scripts => (StackPanel)toolbar.Buttons[ToolbarAction.Superscript].Parent;
        private double Left(FrameworkElement element) => element.TransformToVisual(toolbarRow).TransformPoint(new()).X;
        private double Gap(FrameworkElement before, FrameworkElement after) => Left(after) - Left(before) - before.ActualWidth;

        internal ToolbarFixture(TestEditor editor, Preferences preferences)
        {
            window = editor.Window;
            pane = editor.Pane;
            view = editor.View;
            document = editor.Document;
            toolbar = window.Toolbar;
            this.preferences = preferences;
        }

        private async Task CloseFixture(EditorPane fixture)
        {
            Check(fixture.LastError == null, "completed toolbar fixture has no presentation error", fixture.LastError?.ToString());
            await window.ClosePane(fixture, force: true);
        }

        internal async Task AppearanceAndIcons()
        {
            Check(window.ToolbarToggle.Visibility == Visibility.Visible && toolbar.Visibility == Visibility.Visible, "Markdown toolbar defaults visible");
            Check(Grid.GetColumn(window.ToolbarToggle) == Grid.GetColumn(window.MenuToggle) + 1, "toolbar toggle immediately follows the menu toggle");
            Check(Title(toolbar.Paragraph) == "Heading 1", "toolbar reads the initial named paragraph");
            Check(((StackPanel)toolbar.Paragraph.Content).Children.First() is IconElement
                && ((MenuFlyout)toolbar.Paragraph.Flyout).Items.OfType<ToggleMenuFlyoutItem>().All(i => i.Icon != null),
                "toolbar style selectors annotate every style with its type icon");
            var toolbarRow = (StackPanel)toolbar.Viewport.Children.Single();
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
                "Markdown Preview paragraph selector lists only styles supported for source assignment");
            foreach (var listStyle in initialStyles.Styles.Where(s => (s.Native.flags & VIEM_STYLE_DEFINITION_INTERNAL_LIST) != 0 || s.Native.role == VIEM_STYLE_ROLE_LIST))
                Check(StyleIcons.Kind(listStyle) == StyleIconKind.Paragraph, "list definitions use paragraph style icons: " + listStyle.Name);
            await WindowCapture.SaveElement(toolbar, pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".toolbar-icons.png");
        }

        internal async Task CurrentListStyle()
        {
            foreach (var action in new[] { ToolbarAction.Bullets, ToolbarAction.Numbers, ToolbarAction.Tasks })
            {
                view.Command("j0");
                await Click(toolbar, action);
                var list = view.SelectedNamedStyles();
                var current = ((MenuFlyout)toolbar.Paragraph.Flyout).Items.OfType<ToggleMenuFlyoutItem>()
                    .Single(item => ((StyleKey)item.Tag).Id == list.Paragraph);
                Check(current.IsChecked && !current.IsEnabled && current.Icon is FontIcon { Glyph: "¶" }
                    && Title(toolbar.Paragraph) == current.Text,
                    "current list paragraph retains its exact disabled style row: " + action);
                view.Undo();
                view.Command("gg0");
            }
        }

        internal Task GroupSpacing()
        {
            var codeLink = (StackPanel)toolbar.Buttons[ToolbarAction.CharacterCode].Parent;
            int scriptIndex = toolbarRow.Children.IndexOf(scripts), codeIndex = toolbarRow.Children.IndexOf(codeLink), tableIndex = toolbarRow.Children.IndexOf(toolbar.InsertTable);
            Check(toolbarRow.Children[0] == characterGroup && scriptIndex == 1 && codeIndex == scriptIndex + 1
                && !toolbarRow.Children.OfType<Border>().Any(),
                "toolbar groups use ordinary gaps and place script immediately before code and link");
            Check(toolbar.InsertImage.Parent == toolbarRow.Children[tableIndex - 1]
                && toolbar.Buttons[ToolbarAction.Indent].Parent == toolbarRow.Children[tableIndex + 1]
                && toolbarRow.Children[tableIndex + 2] == toolbar.Paragraph && toolbarRow.Children.Last() == toolbar.Paragraph
                && toolbarRow.Children.OfType<DropDownButton>().SequenceEqual(new[] { toolbar.Paragraph }),
                "table precedes indent and the paragraph selector trails the command buttons");
            toolbar.UpdateLayout();
            double standardGap = Gap(codeLink, (FrameworkElement)toolbar.InsertImage.Parent);
            Check(Math.Abs(standardGap - 8) < 1 && toolbarRow.Children.OfType<FrameworkElement>().Zip(toolbarRow.Children.OfType<FrameworkElement>().Skip(1))
                .All(pair => Math.Abs(Gap(pair.First, pair.Second) - standardGap) < 1),
                "every toolbar group uses the same gap as Link to Bulleted List");
            Check(toolbar.FormattedView.IsChecked == true && !toolbar.FormattedView.IsThreeState
                && AutomationProperties.GetName(toolbar.FormattedView) == "Markdown Preview"
                && ToolTipService.GetToolTip(toolbar.FormattedView) as string == "Markdown Preview",
                "Markdown Preview toggle has a stable accessible label and pressed state");
            return Task.CompletedTask;
        }

        internal async Task PreviewRoundTrip()
        {
            object viewIcon = toolbar.FormattedView.Content;
            byte[] beforeSwitch = document.Source(document.State.document_revision);
            ulong caret = (ulong)document.FormattedText().IndexOf("plain", StringComparison.Ordinal) + 2;
            view.Place(caret, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
            ulong placedCaret = view.Presentation.cursor_utf8_offset;
            await SwitchView(toolbar);
            // Presentation offsets are in the current logical projection;
            // Code shows the complete literal source.
            ulong sourceCaret = (ulong)document.FormattedText().IndexOf("plain", StringComparison.Ordinal) + 2;
            Check(document.State.format == VIEM_FORMAT_CODE && toolbar.FormattedView.IsChecked == false
                && view.Presentation.cursor_utf8_offset == sourceCaret
                && beforeSwitch.AsSpan().SequenceEqual(document.Source(document.State.document_revision)),
                "toolbar turns Markdown Preview off while preserving source and semantic caret",
                $"format={document.State.format}, checked={toolbar.FormattedView.IsChecked}, caret={view.Presentation.cursor_utf8_offset}, expected={sourceCaret}, placed={placedCaret}, requested={caret}, sourceEqual={beforeSwitch.AsSpan().SequenceEqual(document.Source(document.State.document_revision))}, mode={view.Presentation.mode}, error={pane.LastError}");
            Check(LiteralMarkdownChrome(window, toolbar)
                && Grid.GetColumn(window.PreviewToggle) == Grid.GetColumn(window.ToolbarToggle)
                && AutomationProperties.GetName(window.PreviewToggle) == "Markdown Preview",
                "with Preview off the toolbar is hidden and the title bar offers Markdown Preview in its toggle's place");
            view.Undo();
            Check(document.State.format == VIEM_FORMAT_MARKDOWN && toolbar.FormattedView.IsChecked == true && PreviewChrome(window, toolbar, true),
                "undoing a view switch restores the toolbar and its title-bar toggle");
            view.Redo();
            Check(document.State.format == VIEM_FORMAT_CODE && LiteralMarkdownChrome(window, toolbar),
                "redoing a view switch hides the toolbar again");
            await PreviewFromTitleBar(window);
            Check(PreviewChrome(window, toolbar, true), "the title-bar Preview button restores the visible toolbar and toolbar toggle");
            Check(document.State.format == VIEM_FORMAT_MARKDOWN && toolbar.FormattedView.IsChecked == true
                && view.Presentation.cursor_utf8_offset == caret
                && ReferenceEquals(viewIcon, toolbar.FormattedView.Content)
                && beforeSwitch.AsSpan().SequenceEqual(document.Source(document.State.document_revision)),
                "formatted view round trip retains the caret, fixed icon and source bytes");
        }

        internal async Task StyleAssignmentUndo()
        {
            view.Command("gg0");
            view.Command("j0");
            Check(Title(toolbar.Paragraph) == "Base Paragraph", "toolbar follows a caret command synchronously");
            var items = ((MenuFlyout)toolbar.Paragraph.Flyout).Items.ToArray();
            var sheet = view.Styles(view.SelectedNamedStyles().Identity);
            view.Command("w");
            Check(State(toolbar, ToolbarAction.CharacterCode) == true, "toolbar tracks the Code character assignment");
            Check(items.SequenceEqual(((MenuFlyout)toolbar.Paragraph.Flyout).Items) && ReferenceEquals(sheet, view.Styles(view.SelectedNamedStyles().Identity)),
                "cursor movement retains native menu items and the exact stylesheet snapshot");
            view.SelectAll();
            Check(Title(toolbar.Paragraph) == "Mixed", "toolbar selectors show mixed assignments");
            await Choose(toolbar.Paragraph, "Heading2");
            Check(Title(toolbar.Paragraph) == "Heading 2" && pane.LastError == null, "toolbar selector invokes the shared heading action");
            byte[] changed = document.Source(document.State.document_revision);
            view.Undo();
            Check(Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source, "toolbar paragraph assignment is one undo transaction");
            view.Redo();
            Check(changed.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "toolbar paragraph redo restores exact source");
            view.Undo();
        }

        internal async Task CharacterFormattingUndo()
        {
            view.Key(VIEM_KEY_ESCAPE);
            view.Command("j0viw");
            foreach (var action in new[] { ToolbarAction.Bold, ToolbarAction.Italic, ToolbarAction.Strikethrough })
            {
                byte[] before = document.Source(document.State.document_revision);
                await Click(toolbar, action);
                Check(State(toolbar, action) == true && pane.LastError == null, "toolbar applies and immediately reflects " + action);
                view.Undo();
                Check(before.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "toolbar undo restores " + action);
                view.Key(VIEM_KEY_ESCAPE);
                view.Command("ggj0viw");
            }
        }

        internal async Task PendingTypingFormat()
        {
            view.Key(VIEM_KEY_ESCAPE);
            view.Command("ggj0i");
            ulong revision = document.State.document_revision;
            await Click(toolbar, ToolbarAction.Italic);
            Check(State(toolbar, ToolbarAction.Italic) == true && document.State.document_revision == revision, "toolbar supports pending typing formatting without a source edit");
            view.Text("typed");
            view.Key(VIEM_KEY_ESCAPE);
            view.Undo();
        }

        internal Task StaleStyleIdentity()
        {
            view.Command("j0viw");
            var staleIdentity = view.SelectedNamedStyles().Identity;
            view.ToggleSemantic(VIEM_SEMANTIC_STYLE_STRONG);
            ulong afterBold = document.State.document_revision;
            view.ChooseStyle(new(1, "Heading3"), staleIdentity);
            Check(document.State.document_revision == afterBold, "stale catalogue actions cannot edit a later source revision");
            view.Undo();
            view.Key(VIEM_KEY_ESCAPE);
            view.Command("gg0");
            return Task.CompletedTask;
        }

        internal Task UnsupportedInputModes()
        {
            view.Key(VIEM_KEY_CONTROL_CHARACTER, 'q');
            Check(view.Presentation.mode == VIEM_MODE_VISUAL_BLOCK && !toolbar.Buttons[ToolbarAction.Bold].IsEnabled && !toolbar.Paragraph.IsEnabled && pane.LastError == null,
                "Visual Block disables unsupported toolbar actions without errors",
                $"mode={view.Presentation.mode}, formatting={view.HasFormattingSelection}, bold={toolbar.Buttons[ToolbarAction.Bold].IsEnabled}, paragraph={toolbar.Paragraph.IsEnabled}, error={pane.LastError}");
            view.Key(VIEM_KEY_ESCAPE);
            view.Command(":");
            Check(!toolbar.Paragraph.IsEnabled && pane.LastError == null, "command prompt disables formatting without stale selection errors");
            view.Key(VIEM_KEY_ESCAPE);
            return Task.CompletedTask;
        }

        internal async Task NarrowLayout()
        {
            double standardGap = Gap((FrameworkElement)toolbar.Buttons[ToolbarAction.CharacterCode].Parent, (FrameworkElement)toolbar.InsertImage.Parent);
            var size = window.AppWindow.Size;
            double scale = toolbar.XamlRoot.RasterizationScale;
            void Resize(double width, double height) => window.AppWindow.Resize(new((int)Math.Round(width * scale), (int)Math.Round(height * scale)));
            Resize(1100, 450);
            await InputRoutingTests.Settle();
            toolbar.UpdateLayout();
            double toggleX = toolbar.FormattedView.TransformToVisual(toolbar).TransformPoint(new()).X;
            Check(Math.Abs(toggleX + toolbar.FormattedView.ActualWidth - (toolbar.ActualWidth - 10)) < 1
                && toolbarRow.ActualWidth <= toolbar.Viewport.ActualWidth + 1,
                "wide toolbar aligns the formatted-view toggle at the right edge across a flexible gap",
                $"toggleX={toggleX}, toggleWidth={toolbar.FormattedView.ActualWidth}, toolbarWidth={toolbar.ActualWidth}, viewport={toolbar.Viewport.ActualWidth}, scale={scale}");
            Resize(700, 450);
            await InputRoutingTests.Settle();
            toolbar.UpdateLayout();
            double unindentX = toolbar.Buttons[ToolbarAction.Unindent].TransformToVisual(toolbar.Viewport).TransformPoint(new()).X;
            double paragraphX = toolbar.Paragraph.TransformToVisual(toolbar.Viewport).TransformPoint(new()).X;
            Check(unindentX + toolbar.Buttons[ToolbarAction.Unindent].ActualWidth <= toolbar.Viewport.ActualWidth
                && paragraphX < toolbar.Viewport.ActualWidth && paragraphX + toolbar.Paragraph.ActualWidth > toolbar.Viewport.ActualWidth,
                "shrinking the toolbar clips Paragraph while every earlier command remains visible",
                $"unindentRight={unindentX + toolbar.Buttons[ToolbarAction.Unindent].ActualWidth}, paragraphX={paragraphX}, viewport={toolbar.Viewport.ActualWidth}");
            Resize(540, 450);
            await InputRoutingTests.Settle();
            toolbar.UpdateLayout();
            Check(toolbarRow.ActualWidth > toolbar.Viewport.ActualWidth
                && !Descendants<ScrollViewer>(toolbar).Any() && !Descendants<ScrollBar>(toolbar).Any(),
                "narrow toolbar clips its natural-width controls without a scroll container or scrollbar");
            toggleX = toolbar.FormattedView.TransformToVisual(toolbar).TransformPoint(new()).X;
            Check(toggleX >= 0 && toggleX + toolbar.FormattedView.ActualWidth <= toolbar.ActualWidth,
                "formatted-view toggle stays visible beside narrow toolbar overflow");
            double viewportX = toolbar.Viewport.TransformToVisual(toolbar).TransformPoint(new()).X;
            Check(viewportX + toolbar.Viewport.ActualWidth <= toggleX && toolbar.Viewport.Clip is RectangleGeometry clip
                && Math.Abs(clip.Rect.Width - toolbar.Viewport.ActualWidth) < 1 && Math.Abs(clip.Rect.Height - toolbar.Viewport.ActualHeight) < 1,
                "the toolbar clips later controls before the fixed formatted-view toggle");
            Check(toolbar.Paragraph.TransformToVisual(toolbar.Viewport).TransformPoint(new()).X >= toolbar.Viewport.ActualWidth
                && Left(toolbar.Paragraph) >= Left((FrameworkElement)toolbar.Buttons[ToolbarAction.Unindent].Parent)
                    + ((FrameworkElement)toolbar.Buttons[ToolbarAction.Unindent].Parent).ActualWidth,
                "narrow windows clip the trailing style selectors after the earlier command buttons");
            Check(toolbarRow.Children.OfType<FrameworkElement>().Zip(toolbarRow.Children.OfType<FrameworkElement>().Skip(1))
                .All(pair => Gap(pair.First, pair.Second) >= standardGap - 1),
                "narrow toolbar layout never lets later controls overlap earlier groups");
            double rowX = toolbarRow.TransformToVisual(toolbar.Viewport).TransformPoint(new()).X;
            toolbar.Paragraph.Focus(FocusState.Programmatic);
            toolbar.Paragraph.StartBringIntoView(new BringIntoViewOptions { AnimationDesired = false });
            await InputRoutingTests.Settle();
            toolbar.UpdateLayout();
            Check(Math.Abs(toolbarRow.TransformToVisual(toolbar.Viewport).TransformPoint(new()).X - rowX) < .01
                && toolbar.Paragraph.TransformToVisual(toolbar.Viewport).TransformPoint(new()).X >= toolbar.Viewport.ActualWidth
                && Math.Abs(toolbar.FormattedView.TransformToVisual(toolbar).TransformPoint(new()).X - toggleX) < 1,
                "focus and bring-into-view requests cannot scroll the toolbar or displace its fixed toggle");
            Check(toolbar.FormattedView.Focus(FocusState.Programmatic), "the formatted-view toggle remains keyboard reachable beside clipped controls");
            pane.FocusEditor();
            window.AppWindow.Resize(size);
        }

        internal async Task VisibilityPreferences()
        {
            bool showMenu = preferences.ShowMenu;
            preferences.Set("windows", "showMenu", false);
            Check(toolbar.Visibility == Visibility.Visible, "toolbar visibility is independent of menu visibility");
            preferences.Set("windows", "showMenu", showMenu);
            window.ToolbarToggle.Focus(FocusState.Programmatic);
            await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
            await InputRoutingTests.Settle();
            Check(toolbar.Visibility == Visibility.Collapsed && !new Preferences(preferences.DirectoryPath).ShowFormattingToolbar(VIEM_FORMAT_MARKDOWN), "title-bar toggle persists the Markdown toolbar preference");
        }

        internal async Task NestedListsAndPreview()
        {
            preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN, true);
            var markdown = window.AddPane(new CoreDocument("- One\n  - Two\n\nPlain"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN));
            var md = await markdown.Ready;
            Check(toolbar.Visibility == Visibility.Visible && toolbar.FormattedView.IsChecked == true,
                "switching panes restores each format's toolbar preference and formatted-view state");
            md.Command("vj");
            Check(State(toolbar, ToolbarAction.Bullets) == true && Title(toolbar.Paragraph) == "Mixed", "list state is structural across nested paragraph styles");
            md.SelectAll();
            Check(State(toolbar, ToolbarAction.Bullets) == null, "mixed list and non-list selection shows a mixed toggle");
            md.Key(VIEM_KEY_ESCAPE);
            md.Command("ggj0");
            byte[] nested = markdown.Document.Source(markdown.Document.State.document_revision);
            await Click(toolbar, ToolbarAction.Unindent);
            Check(toolbar.Buttons[ToolbarAction.Indent].IsEnabled && State(toolbar, ToolbarAction.Bullets) == true, "Unindent preserves structural list membership");
            await Click(toolbar, ToolbarAction.Indent);
            Check(toolbar.Buttons[ToolbarAction.Unindent].IsEnabled && State(toolbar, ToolbarAction.Bullets) == true, "Indent preserves structural list membership");
            md.Undo();
            md.Undo();
            Check(nested.AsSpan().SequenceEqual(markdown.Document.Source(markdown.Document.State.document_revision)), "toolbar indentation actions each undo atomically");
            md.Key(VIEM_KEY_ESCAPE);
            md.Command("gg0");
            await Click(toolbar, ToolbarAction.Bullets);
            Check(State(toolbar, ToolbarAction.Bullets) == false, "active bulleted list toolbar action removes the list");
            await Click(toolbar, ToolbarAction.Numbers);
            Check(State(toolbar, ToolbarAction.Numbers) == true, "numbered list toolbar action applies structural numbering");
            md.SetMarkdownPreview(false);
            Check(LiteralMarkdownChrome(window, toolbar), "Markdown Preview off hides the toolbar and shows the title-bar Preview button");
            md.SetMarkdownPreview(true);
            Check(PreviewChrome(window, toolbar, true), "Preview on restores the remembered visible toolbar");
            preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN, false);
            md.SetMarkdownPreview(false);
            Check(LiteralMarkdownChrome(window, toolbar), "literal Markdown offers Preview whatever the toolbar preference");
            await PreviewFromTitleBar(window);
            Check(markdown.Document.State.format == VIEM_FORMAT_MARKDOWN && PreviewChrome(window, toolbar, false) && window.ToolbarToggle.IsChecked == false,
                "Preview from the title bar keeps a hidden toolbar hidden and shows the unchecked toolbar toggle");
            preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN, true);
            pane.FocusEditor();
            await InputRoutingTests.Settle();
            Check(toolbar.Visibility == Visibility.Visible, "returning to Markdown restores its own toolbar preference");
            preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN, true);
            await CloseFixture(markdown);
        }

        internal async Task QuoteTreatment()
        {
            const uint format = VIEM_FORMAT_MARKDOWN;
            const string inner = "# Heading";
            var quotedPane = window.AddPane(new CoreDocument(Encoding.UTF8.GetBytes(inner), format: format));
            var quotedView = await quotedPane.Ready;
            string quoteTargetBefore = $"active={ReferenceEquals(window.ActivePane, quotedPane)}, activeView={window.ActivePane?.View?.Id}, toolbarView={toolbar.BoundViewIdForTesting}, expectedView={quotedView.Id}";
            Check(ReferenceEquals(window.ActivePane, quotedPane) && quotedPane.IsActive,
                "adding a pane makes its document the native toolbar action target", quoteTargetBefore);
            await Click(toolbar, ToolbarAction.BlockQuote);
            Check(State(toolbar, ToolbarAction.BlockQuote) == true && quotedPane.Document.FormattedText().Contains("Heading"), "quote toggle adds an enclosing container");
            Check(Encoding.UTF8.GetString(quotedPane.Document.Source(quotedPane.Document.State.document_revision)) == "> # Heading", "quote toggle preserves heading syntax",
                $"format={format}, source={Encoding.UTF8.GetString(quotedPane.Document.Source(quotedPane.Document.State.document_revision)).Replace("\n", "\\n")}, mode={quotedView.Presentation.mode}, "
                + $"before: {quoteTargetBefore}; after active={ReferenceEquals(window.ActivePane, quotedPane)}, activeView={window.ActivePane?.View?.Id}, toolbarView={toolbar.BoundViewIdForTesting}, "
                + $"coreQuote={quotedView.SelectedNamedStyles().QuoteState}, toolbarQuote={State(toolbar, ToolbarAction.BlockQuote)}, "
                + $"foregroundOwnsWindow={InputRoutingTests.ForegroundWindowForTesting == window.Hwnd}, error={quotedPane.LastError}");
            await Click(toolbar, ToolbarAction.BlockQuote);
            Check(State(toolbar, ToolbarAction.BlockQuote) == false && Encoding.UTF8.GetString(quotedPane.Document.Source(quotedPane.Document.State.document_revision)) == inner, "quote toggle removes only quote treatment");
            quotedView.Undo();
            Check(State(toolbar, ToolbarAction.BlockQuote) == true, "quote toggle follows shared history");
            await CloseFixture(quotedPane);

        }

        internal async Task AlertTreatment()
        {
            var alertPane = window.AddPane(new CoreDocument(Encoding.UTF8.GetBytes("Words"), format: VIEM_FORMAT_MARKDOWN));
            var alertView = await alertPane.Ready;
            string Source() => Encoding.UTF8.GetString(alertPane.Document.Source(alertPane.Document.State.document_revision));
            toolbar.RefreshActions();
            var items = ((MenuFlyout)toolbar.BlockQuoteAlerts.Flyout).Items.Cast<ToggleMenuFlyoutItem>().ToArray();
            Check(toolbar.BlockQuoteAlerts.IsEnabled && items.Select(item => item.Text).SequenceEqual(new[] { "Note", "Tip", "Important", "Warning", "Caution" })
                && items.All(item => item.IsEnabled && item.IsChecked == false && item.Icon is PathIcon), "alert menu offers every GitHub kind with an icon");
            toolbar.ChooseAlert(VIEM_ALERT_NOTE);
            await InputRoutingTests.Settle();
            Check(Source() == "> [!NOTE]\n> Words" && State(toolbar, ToolbarAction.BlockQuote) == true && toolbar.AlertSelected(VIEM_ALERT_NOTE),
                "choosing Note quotes the paragraph behind a hidden marker", Source());
            toolbar.ChooseAlert(VIEM_ALERT_WARNING);
            toolbar.ChooseAlert(VIEM_ALERT_WARNING);
            await InputRoutingTests.Settle();
            Check(Source() == "> Words" && State(toolbar, ToolbarAction.BlockQuote) == true, "choosing the current kind restores a plain quote", Source());
            alertView.Undo();
            Check(Source() == "> [!WARNING]\n> Words", "alert changes follow shared history", Source());
            await CloseFixture(alertPane);
        }

        internal async Task CodeBlockProtection()
        {
            const uint format = VIEM_FORMAT_MARKDOWN;
            // None uses document-owned rich code styling. An annotated
            // fence instead uses the shared application Code stylesheet.
            const string code = "Before\n\n```\ngraph LR\n    Writing --> Editing\n```\n\nAfter";
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
            // Declare the expected body appearance explicitly; bundled
            // theme values are not this fixture's baseline.
            var bodyStyle = codeView.Styles().Styles.Single(style => style.Key == new StyleKey(1, "Paragraph"));
            codeView.EditStyle(bodyStyle, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH,
                CoreView.Enum(VIEM_STYLE_VALUE_BOOLEAN, 1));
            toolbar.Refresh();
            var literalStyles = codeView.SelectedNamedStyles();
            var literalSheet = codeView.Styles();
            Check(State(toolbar, ToolbarAction.Strikethrough) == true && !toolbar.Buttons[ToolbarAction.Strikethrough].IsEnabled,
                "literal code keeps resolved strikethrough appearance visible while disabling document formatting: " + format,
                $"toggle={State(toolbar, ToolbarAction.Strikethrough)}, enabled={toolbar.Buttons[ToolbarAction.Strikethrough].IsEnabled}, "
                + $"coreState={codeView.StrikethroughState()}, canFormat={codeView.CanFormatStrikethrough}, "
                + $"selected={literalStyles.Paragraph}/{literalStyles.Character}, flags={literalStyles.Flags}, "
                + $"styles={string.Join(", ", literalSheet.Styles.Where(style => style.Id is "Paragraph" or "Code Block" or "Code").Select(style => $"{style.Namespace}:{style.Id} declared={style.Declares(VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH)} effective={style.Value(VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH).enum_value}"))}, "
                + $"mode={codeView.Presentation.mode}, cursor={codeView.Presentation.cursor_utf8_offset}, error={codePane.LastError}");
            Check(toolbar.Buttons[ToolbarAction.CharacterCode].Visibility == Visibility.Visible
                && !toolbar.Buttons[ToolbarAction.CharacterCode].IsEnabled,
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

        internal async Task LiteralFormats()
        {
            foreach (uint format in new[] { VIEM_FORMAT_PLAIN_TEXT, VIEM_FORMAT_CODE })
            {
                var literal = window.AddPane(new CoreDocument("text"u8.ToArray(), format: format));
                await literal.Ready;
                Check(toolbar.Visibility == Visibility.Collapsed && window.ToolbarToggle.Visibility == Visibility.Collapsed
                    && window.PreviewToggle.Visibility == Visibility.Collapsed, "literal format omits toolbar and both title-bar toggles: " + format);
                await CloseFixture(literal);
            }
        }

    }
}
#endif

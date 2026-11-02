#if DEBUG
using System.Text;
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Viem.Windows.Core;
using Viem.Windows.Shell;
using Windows.Foundation;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class CheckboxInteractionTests
{
    private static void Check(bool condition, string name)
    {
        if (!condition) throw new InvalidOperationException(name);
        FrontendSmokeTests.UiChecks.Add(name);
    }

    internal static async Task Run(Preferences preferences)
    {
        bool originalToolbar = preferences.ShowFormattingToolbar(VIEM_FORMAT_MARKDOWN);
        bool originalMenu = preferences.ShowMenu;
        preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN, true);
        preferences.Set("windows", "showMenu", true);
        var document = new CoreDocument("alpha"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window);
        window.Activate();
        try
        {
            var pane = window.ActivePane!;
            var view = await pane.Ready;
            await InputRoutingTests.Settle();
            var toolbar = window.Toolbar;
            var menus = window.Menu.Items.Select(item => item.Title).ToArray();
            Check(Array.IndexOf(menus, "Insert") == Array.IndexOf(menus, "Edit") + 1
                && Array.IndexOf(menus, "Format") == Array.IndexOf(menus, "Insert") + 1,
                "Insert is between Edit and Format on Windows");
            var insert = window.Menu.Items.Single(item => item.Title == "Insert");
            Check(insert.Items.Select(item => item switch { MenuFlyoutItem command => command.Text,
                    ToggleMenuFlyoutItem toggle => toggle.Text, MenuFlyoutSubItem submenu => submenu.Text, _ => "separator" })
                .SequenceEqual(new[] { "Heading", "Code", "Link", "Reference", "separator", "Bulleted List", "Numbered List", "Task List", "separator", "Blockquote", "Code Block", "Image", "Table" }),
                "Insert contains the toolbar actions in the requested order");
            Check(!menus.Contains("Style"), "the Style top-level menu is removed");
            var heading = (MenuFlyoutSubItem)insert.Items[0];
            Check(heading.Items.Cast<ToggleMenuFlyoutItem>().Select(item => ((StyleKey)item.Tag).Id)
                .SequenceEqual(Enumerable.Range(1, 6).Select(level => $"Heading{level}")),
                "Insert Heading exposes all six catalogue heading levels in order");
            var format = window.Menu.Items.Single(item => item.Title == "Format");
            Check(format.Items.Skip(3).Select(item => item switch { MenuFlyoutItem command => command.Text,
                    ToggleMenuFlyoutItem toggle => toggle.Text, MenuFlyoutSubItem submenu => submenu.Text, _ => "separator" })
                .SequenceEqual(new[] { "separator", "Theme", "Edit Styles…", "Reload Style Sheet", "separator", "Word wrap", "Align indents to previous line" }),
                "Format places theme and style commands before Word wrap and indent alignment");
            var heading6 = heading.Items.Cast<ToggleMenuFlyoutItem>().Last();
            var insertPeer = new MenuBarItemAutomationPeer(insert);
            await MenuTestHelpers.Transition(true, () => { insertPeer.Expand(); return Task.CompletedTask; }, heading);
            heading.Focus(FocusState.Programmatic);
            await MenuTestHelpers.Transition(true, () => InputRoutingTests.Key(global::Windows.System.VirtualKey.Right), heading6);
            heading6.Focus(FocusState.Programmatic);
            await MenuTestHelpers.Transition(false, () => InputRoutingTests.Key(global::Windows.System.VirtualKey.Space), heading6);
            Check(Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == "###### alpha",
                "Insert Heading uses the paragraph selector's catalogue-bound assignment behavior");
            view.Undo();
            await InputRoutingTests.Settle();
            var taskList = (ToggleButton)toolbar.Buttons[ToolbarAction.Tasks];
            var group = (StackPanel)toolbar.Buttons[ToolbarAction.Numbers].Parent;
            Check(group.Children.IndexOf(taskList) == group.Children.IndexOf(toolbar.Buttons[ToolbarAction.Numbers]) + 1
                && taskList.IsEnabled && taskList.IsChecked == false,
                "Task List follows Numbered List as an enabled toolbar toggle");
            taskList.Focus(FocusState.Programmatic);
            await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
            await InputRoutingTests.Settle();
            string Source() => Encoding.UTF8.GetString(document.Source(document.State.document_revision));
            window.ValidateMenusForTesting();
            Check(Source() == "- [ ] alpha" && taskList.IsEnabled && taskList.IsChecked == true
                && !toolbar.ActionSelected(ToolbarAction.Bullets) && !toolbar.ActionSelected(ToolbarAction.Numbers),
                "Task List creates an unchecked item and remains the exclusive active list type");
            Check(insert.Items.OfType<ToggleMenuFlyoutItem>().Single(item => item.Text == "Task List").IsChecked,
                "Insert Task List reflects the shared active list state");
            pane.Refresh();
            var layout = view.Layout();
            var decoration = layout.Decorations.Single(item => (item.flags & VIEM_LAYOUT_DECORATION_CHECKBOX) != 0);
            var row = layout.Rows.Single(item => item.row_index == decoration.row_index);
            Check(decoration.typographic_bounds.width > 0
                && Math.Abs(decoration.typographic_bounds.width - decoration.typographic_bounds.height) < .001f
                && Math.Abs(decoration.typographic_bounds.y + decoration.typographic_bounds.height - row.baseline) < .001f,
                "native checkbox geometry is a square ending exactly on the first-row baseline");
            using var image = new CanvasRenderTarget(pane.Canvas.Device, (float)pane.Canvas.ActualWidth, (float)pane.Canvas.ActualHeight, pane.Canvas.Dpi);
            byte[] Draw()
            {
                using (var drawing = image.CreateDrawingSession()) pane.Draw(drawing);
                return image.GetPixelBytes();
            }
            byte[] before = Draw();
            pane.InvalidateDrawingCache();
            Check(Draw().AsSpan().SequenceEqual(before), "checkbox pixels are identical with fresh and cached text drawing");
            var viewport = view.Viewport;
            var center = new Point(decoration.typographic_bounds.x + decoration.typographic_bounds.width / 2 - viewport.left,
                decoration.typographic_bounds.y + decoration.typographic_bounds.height / 2 - viewport.top);
            await InputRoutingTests.Drag(window, pane.Canvas,
                new[] { new Point(center.X / pane.Canvas.ActualWidth, center.Y / pane.Canvas.ActualHeight) },
                _ => Check(pane.PointerCursorForTesting == "arrow", "checkbox hover uses the normal arrow instead of the margin-selection cursor"),
                focusTarget: false);
            await InputRoutingTests.Settle();
            Check(Source() == "- [x] alpha", "native checkbox pointer click changes only its task marker");
            Check(!Draw().AsSpan().SequenceEqual(before), "checked native checkbox changes the painted box and checkmark");
            view.Undo();
            Check(Source() == "- [ ] alpha", "checkbox click has one exact undo unit");
            view.Redo();
            Check(Source() == "- [x] alpha", "checkbox click has exact redo");
            bool staleRejected = false;
            try { view.ToggleCheckbox(layout.Info.identity, decoration.paragraph_id); }
            catch (CoreException) { staleRejected = true; }
            Check(staleRejected && Source() == "- [x] alpha", "checkbox click cannot use stale native layout geometry");
            view.Undo(); view.Undo();
            Check(Source() == "alpha", "undoing checkbox authoring restores the original ordinary paragraph");
            toolbar.Execute(ToolbarAction.Tasks);
            toolbar.Execute(ToolbarAction.Tasks);
            Check(Source() == "alpha" && !toolbar.ActionSelected(ToolbarAction.Tasks),
                "Task List toggles back to an ordinary paragraph like the other list types");
            toolbar.Execute(ToolbarAction.CodeBlock);
            string codeSource = Source();
            window.ValidateMenusForTesting();
            Check(!taskList.IsEnabled
                && !insert.Items.OfType<ToggleMenuFlyoutItem>().Single(item => item.Text == "Task List").IsEnabled,
                "Task List toolbar and menu share core eligibility and disable inside a code block");
            toolbar.Execute(ToolbarAction.Tasks);
            Check(Source() == codeSource, "disabled Task List leaves code source unchanged");
            view.Undo();
            pane.Refresh();
            var table = insert.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == "Table");
            var peer = new MenuBarItemAutomationPeer(insert);
            await MenuTestHelpers.Transition(true, () => { peer.Expand(); return Task.CompletedTask; }, table);
            new MenuFlyoutItemAutomationPeer(table).Invoke();
            await SelfTestRunner.Until(() => toolbar.HasOpenPopup);
            Check(toolbar.HasOpenPopup, "Insert Table opens the existing interactive toolbar size picker");
            toolbar.DismissPopups();
            Check(pane.LastError == null, "checkbox and Insert menu actions complete without native errors");
            await ImportedOrderedTask(preferences);
            await DragFromCheckbox(preferences);
        }
        finally
        {
            window.Close();
            preferences.SetFormattingToolbar(VIEM_FORMAT_MARKDOWN, originalToolbar);
            preferences.Set("windows", "showMenu", originalMenu);
        }
    }

    private static async Task ImportedOrderedTask(Preferences preferences)
    {
        const string source = "3. [X] done";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window);
        window.Activate();
        try
        {
            var pane = window.ActivePane!;
            var view = await pane.Ready;
            await InputRoutingTests.Settle();
            var toolbar = window.Toolbar;
            Check(toolbar.ActionSelected(ToolbarAction.Tasks) && !toolbar.ActionSelected(ToolbarAction.Numbers)
                && !toolbar.ActionSelected(ToolbarAction.Bullets),
                "an imported ordered task is Task List without activating Numbered List");
            toolbar.Execute(ToolbarAction.Bullets);
            Check(toolbar.ActionSelected(ToolbarAction.Bullets) && !toolbar.ActionSelected(ToolbarAction.Tasks)
                && Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == "- done",
                "switching an ordered task to Bulleted List removes its hidden checkbox");
            view.Undo();
            Check(Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                "undo restores the imported ordinal and uppercase checked marker exactly");
            Check(pane.LastError == null, "imported task list changes complete without native errors");
        }
        finally { window.Close(); }
    }

    private static async Task DragFromCheckbox(Preferences preferences)
    {
        var document = new CoreDocument("- [ ] Task\n- Next"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window);
        window.Activate();
        try
        {
            var pane = window.ActivePane!;
            var view = await pane.Ready;
            await InputRoutingTests.Settle();
            view.Place(2, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, view.Presentation.document_revision);
            pane.Refresh();
            var caret = view.Presentation.cursor_utf8_offset;
            var mode = view.Presentation.mode;
            var layout = view.Layout();
            var viewport = view.Viewport;
            var decoration = layout.Decorations.Single(item => (item.flags & VIEM_LAYOUT_DECORATION_CHECKBOX) != 0);
            var last = layout.Clusters.Last(item => item.row_index == decoration.row_index);
            var row = layout.Rows.Single(item => item.row_index == decoration.row_index);
            var box = decoration.typographic_bounds;
            Point Fraction(double x, double y) => new((x - viewport.left) / pane.Canvas.ActualWidth, (y - viewport.top) / pane.Canvas.ActualHeight);
            var points = new[] {
                Fraction(box.x + box.width * .5, box.y + box.height * .5),
                Fraction(box.x + box.width * .25, box.y + box.height * .25),
                Fraction(box.x + box.width * .75, box.y + box.height * .75),
                Fraction(last.x + last.advance * .9, row.y + row.ascent * .5),
            };
            await InputRoutingTests.Drag(window, pane.Canvas, points, i => {
                if (i is 0 or 3) return;
                var current = view.Presentation;
                Check(!view.HasSelection && current.cursor_utf8_offset == caret && current.mode == mode
                    && pane.PointerCursorForTesting == "arrow",
                    "a drag within a checkbox keeps the caret, selects nothing and keeps the arrow cursor");
            }, focusTarget: false);
            await InputRoutingTests.Settle();
            var selection = view.LogicalSelection();
            Check(view.HasSelection && selection.text_start == 0 && selection.text_end == 4
                && Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == "- [x] Task\n- Next",
                "a drag that leaves a checkbox toggles it once and selects from the press to the pointer");
            Check(pane.LastError == null, "checkbox drag selection completes without native errors");
        }
        finally { window.Close(); }
    }
}
#endif

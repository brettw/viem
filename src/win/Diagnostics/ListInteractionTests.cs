#if DEBUG
using System.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Input;
using Viem.Windows.Interop;
using Viem.Windows.Shell;
using Windows.System;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Diagnostics;

/// <summary>Exercise native list menus and keyboard routing through the real WinUI input host.</summary>
internal static class ListInteractionTests
{
    private static void Check(bool condition, string name)
    {
        if (!condition) throw new InvalidOperationException(name);
        FrontendSmokeTests.UiChecks.Add(name);
    }

    private static unsafe string[] DecorationLabels(CoreView view)
    {
        var identity = view.LayoutInfo().identity;
        var info = New<ViemLayoutDecorationsInfoV1>();
        uint status = viem_core_view_copy_layout_decorations(
            view.Document.Handle, view.Id, &identity, null, 0, null, 0, &info);
        if (status != VIEM_STATUS_BUFFER_TOO_SMALL) Abi.Check(status, "Read list marker decorations");
        var decorations = new ViemLayoutDecorationV1[checked((int)info.decoration_count)];
        var labels = new byte[checked((int)info.label_bytes)];
        fixed (ViemLayoutDecorationV1* decorationData = decorations)
        fixed (byte* labelData = labels)
            Abi.Check(viem_core_view_copy_layout_decorations(
                view.Document.Handle, view.Id, &identity,
                decorationData, (ulong)decorations.Length, labelData, (ulong)labels.Length, &info),
                "Copy list marker decorations");
        return decorations
            .Where(decoration => decoration.label_byte_length > 0)
            .GroupBy(decoration => decoration.row_index)
            .OrderBy(row => row.Key)
            .Select(row => string.Concat(row.Select(decoration => Encoding.UTF8.GetString(
                labels, checked((int)decoration.label_byte_start), checked((int)decoration.label_byte_length)))))
            .ToArray();
    }

    internal static async Task Run(Preferences preferences)
    {
        const string original = "1. First\n2. Second";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(original), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window);
        window.Activate();
        await Task.Delay(250);
        var pane = window.ActivePane!;
        var view = await pane.Ready;
        var styles = window.Menu.Items.Single(item => item.Title == "Style");
        var stylesPeer = new MenuBarItemAutomationPeer(styles);
        var paragraph = styles.Items.OfType<MenuFlyoutSubItem>().Single(item => item.Text == "Paragraph");
        var indent = paragraph.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == "Indent");
        var unindent = paragraph.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == "Unindent");
        ulong second = checked((ulong)document.FormattedText().IndexOf("Second", StringComparison.Ordinal));

        string Source() => Encoding.UTF8.GetString(document.Source(document.State.document_revision));
        void PlaceInSecond() => view.Place(second + 3, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
        void PlaceAtSecondStart() => view.Place(second, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
        async Task OpenParagraphMenu()
        {
            window.Activate();
            stylesPeer.Expand();
            await Task.Delay(40);
            Check(paragraph.Focus(FocusState.Programmatic), "Style Paragraph submenu receives keyboard focus");
            await InputRoutingTests.Key(VirtualKey.Right);
            await Task.Delay(80);
        }
        async Task Invoke(MenuFlyoutItem item)
        {
            await OpenParagraphMenu();
            new MenuFlyoutItemAutomationPeer(item).Invoke();
            await Task.Delay(100);
        }
        void CheckNested(string name)
        {
            string source = Source();
            uint capabilities = view.ListCapabilities();
            bool correct = document.FormattedText() == "First\nSecond" && source != original
                && (capabilities & VIEM_LIST_CAN_UNINDENT) != 0;
            if (!correct) throw new InvalidOperationException(
                $"{name} (source={source}, text={document.FormattedText()}, capabilities={capabilities})");
            FrontendSmokeTests.UiChecks.Add(name);
        }
        void CheckTopLevel(string name)
        {
            string source = Source();
            uint capabilities = view.ListCapabilities();
            bool correct = document.FormattedText() == "First\nSecond"
                && (capabilities & VIEM_LIST_CAN_INDENT) != 0;
            if (!correct) throw new InvalidOperationException(
                $"{name} (source={source}, text={document.FormattedText()}, capabilities={capabilities})");
            FrontendSmokeTests.UiChecks.Add(name);
        }

        try
        {
            PlaceInSecond();
            await OpenParagraphMenu();
            Check((view.ListCapabilities() & VIEM_LIST_CAN_INDENT) != 0 && indent.IsEnabled && !unindent.IsEnabled,
                "Paragraph Indent enables for the second list item at a mid-item caret");
            stylesPeer.Collapse();
            await Invoke(indent);
            CheckNested("Paragraph Indent dispatches the verified structural list action");
            Check(DecorationLabels(view).Contains("a."),
                "Paragraph Indent renders the first nested ordered item with a lower-alpha marker");
            await OpenParagraphMenu();
            Check(!indent.IsEnabled && unindent.IsEnabled,
                "Paragraph menu refreshes Indent and Unindent after nesting changes");
            stylesPeer.Collapse();
            await Invoke(unindent);
            CheckTopLevel("Paragraph Unindent restores the item to the top-level list");

            PlaceInSecond();
            view.Key(VIEM_KEY_CHARACTER, 'i');
            pane.FocusEditor();
            await Task.Delay(80);
            await InputRoutingTests.Key(VirtualKey.Tab);
            CheckNested("native Tab indents from the middle of a list paragraph");
            Check(DecorationLabels(view).Contains("a."),
                "native Tab renders the first nested ordered item with a lower-alpha marker");
            await InputRoutingTests.Key(VirtualKey.Tab, shift: true);
            CheckTopLevel("native Shift-Tab unindents from the middle of a list paragraph");
            Check(view.Presentation.mode == VIEM_MODE_INSERT,
                "native structural Tab keys preserve Insert mode");
            await InputRoutingTests.Key(VirtualKey.Escape);

            PlaceInSecond();
            await Invoke(indent);
            PlaceAtSecondStart();
            view.Key(VIEM_KEY_CHARACTER, 'i');
            pane.FocusEditor();
            await Task.Delay(80);
            await InputRoutingTests.Key(VirtualKey.Back);
            CheckTopLevel("native Backspace at a nested item start unindents exactly one level");
            await InputRoutingTests.Key(VirtualKey.Back);
            Check(document.FormattedText() == "First\nSecond" && Source() != original
                && view.ListCapabilities() == 0,
                "native Backspace at a top-level item start removes its list treatment");

            Check(KeyPolicy.Route(VirtualKey.Tab, false, false, false).Kind == VIEM_KEY_TAB
                && KeyPolicy.Route(VirtualKey.Tab, false, true, false).Kind == VIEM_KEY_BACK_TAB,
                "Windows routes Tab and Shift-Tab to distinct core list keys");
            if (pane.LastError != null) throw pane.LastError;
        }
        finally
        {
            stylesPeer.Collapse();
            App.Instance.Windows.Remove(window);
            window.Close();
        }
    }
}
#endif

#if DEBUG
using System.Text;
using Viem.Windows.Core;
using Viem.Windows.Input;
using Viem.Windows.Interop;
using Viem.Windows.Shell;
using Windows.System;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Diagnostics;

/// <summary>Exercise list keyboard routing through the real WinUI input host.</summary>
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

    private const string original = "1. First\n2. Second";

    internal static SelfTestRunner.Stage Group(Preferences preferences)
    {
        return TestEditor.Group("ListInteraction", preferences, original, [
            new("KeyRouting", _ => {
                KeyRouting();
                return Task.CompletedTask;
            }, UsesEditor: false),
            new("InsertAndReplaceAltEnter", editor => RunCase(editor, fixture => fixture.InsertAndReplaceAltEnter())),
            new("AcceptCompletionBeforeSplit", editor => RunCase(editor, fixture => fixture.AcceptCompletionBeforeSplit())),
            new("TabAndShiftTab", editor => RunCase(editor, fixture => fixture.TabAndShiftTab())),
            new("BackspaceUnindentsThenRemovesList", editor => RunCase(editor, fixture => fixture.BackspaceUnindentsThenRemovesList())),
        ], resetCase: editor => editor.Source == original ? editor.RestoreCaret(original) : editor.Reset(original));

        static async Task RunCase(TestEditor editor, Func<ListFixture, Task> test)
        {
            var fixture = new ListFixture(editor);
            await test(fixture);
            if (editor.Pane.LastError != null) throw editor.Pane.LastError;
        }
    }

    private static void KeyRouting()
    {
        Check(KeyPolicy.Route(VirtualKey.Enter, false, false, true).Kind == VIEM_KEY_ALT_ENTER
            && KeyPolicy.Route(VirtualKey.Enter, false, false, true, literal: true).Kind == VIEM_KEY_ALT_ENTER,
            "Windows preserves Alt-Enter as a distinct core key, including literal-next input");
        Check(KeyPolicy.Route(VirtualKey.Enter, false, false, false).Kind == VIEM_KEY_ENTER
            && KeyPolicy.Route(VirtualKey.Enter, false, true, false).Kind == VIEM_KEY_SHIFT_ENTER
            && KeyPolicy.Route(VirtualKey.Enter, true, false, true).Kind == VIEM_KEY_ENTER
            && KeyPolicy.Route(VirtualKey.Enter, true, true, true).Kind == VIEM_KEY_SHIFT_ENTER,
            "Alt-Enter routing preserves Enter, Shift-Enter and Ctrl-Alt/AltGr input");

        Check(KeyPolicy.Route(VirtualKey.Tab, false, false, false).Kind == VIEM_KEY_TAB
            && KeyPolicy.Route(VirtualKey.Tab, false, true, false).Kind == VIEM_KEY_BACK_TAB,
            "Windows routes Tab and Shift-Tab to distinct core list keys");
    }

    private sealed class ListFixture
    {
        private readonly Viem.Windows.Editor.EditorPane pane;
        private readonly CoreDocument document;
        private readonly CoreView view;
        private readonly ulong second;

        internal ListFixture(TestEditor editor)
        {
            pane = editor.Pane;
            document = editor.Document;
            view = editor.View;
            second = checked((ulong)document.FormattedText().IndexOf("Second", StringComparison.Ordinal));
        }

        private string Source() => Encoding.UTF8.GetString(document.Source(document.State.document_revision));
        private void PlaceInSecond() => view.Place(second + 3, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
        private void PlaceAtSecondStart() => view.Place(second, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);

        private void CheckNested(string name)
        {
            string source = Source();
            uint capabilities = view.ListCapabilities();
            bool correct = document.FormattedText() == "First\nSecond" && source != original
                && (capabilities & VIEM_LIST_CAN_UNINDENT) != 0;
            if (!correct) throw new InvalidOperationException(
                $"{name} (source={source}, text={document.FormattedText()}, capabilities={capabilities})");
            FrontendSmokeTests.UiChecks.Add(name);
        }
        private void CheckTopLevel(string name)
        {
            string source = Source();
            uint capabilities = view.ListCapabilities();
            bool correct = document.FormattedText() == "First\nSecond"
                && (capabilities & VIEM_LIST_CAN_INDENT) != 0;
            if (!correct) throw new InvalidOperationException(
                $"{name} (source={source}, text={document.FormattedText()}, capabilities={capabilities})");
            FrontendSmokeTests.UiChecks.Add(name);
        }


        internal async Task InsertAndReplaceAltEnter()
        {
            foreach (var (mode, entry) in new[] { (VIEM_MODE_INSERT, 'i'), (VIEM_MODE_REPLACE, 'R') })
            {
                PlaceInSecond();
                view.Key(VIEM_KEY_CHARACTER, entry);
                pane.FocusEditor();
                await InputRoutingTests.Settle();
                await InputRoutingTests.Key(VirtualKey.Enter, alt: true);
                Check(document.FormattedText() == "First\nSec\nond" && view.Presentation.mode == mode
                    && DecorationLabels(view).SequenceEqual(new[] { "1.", "2." }),
                    $"native Alt-Enter inserts a paragraph in the same list item and preserves mode {mode}");
                if (mode == VIEM_MODE_REPLACE)
                {
                    await InputRoutingTests.Key(VirtualKey.Back);
                    Check(Source() == original && view.Presentation.mode == VIEM_MODE_REPLACE,
                        "native Replace Backspace restores the source patches from Alt-Enter");
                    await InputRoutingTests.Key(VirtualKey.Enter, alt: true);
                    Check(document.FormattedText() == "First\nSec\nond",
                        "native Alt-Enter can split again after Replace Backspace restoration");
                }
                await InputRoutingTests.Key(VirtualKey.Escape);
                view.Undo();
                Check(Source() == original, $"native Alt-Enter in mode {mode} undoes to exact source bytes");
            }
        }

        internal async Task AcceptCompletionBeforeSplit()
        {
            PlaceAtSecondStart();
            view.Key(VIEM_KEY_CHARACTER, 'i');
            view.Text("Fi");
            view.Key(VIEM_KEY_CONTROL_CHARACTER, 'n');
            var completion = view.Completion();
            for (int attempt = 0; attempt < 100 && (completion.Info.flags & VIEM_COMPLETION_SEARCHING) != 0; attempt++)
            {
                await Task.Delay(20);
                view.Poll();
                completion = view.Completion();
            }
            Check((completion.Info.flags & VIEM_COMPLETION_ACTIVE) != 0 && completion.Info.selected_index >= 0
                && completion.Info.selected_index < completion.Items.Length
                && completion.Items[(int)completion.Info.selected_index] == "First"
                && document.FormattedText() == "First\nFiSecond",
                "Alt-Enter fixture previews a completion without changing source");
            pane.FocusEditor();
            await InputRoutingTests.Key(VirtualKey.Enter, alt: true);
            Check(document.FormattedText() == "First\nFirst\nSecond"
                && view.Presentation.cursor_utf8_offset == (ulong)"First\nFirst\n".Length
                && (view.Completion().Info.flags & VIEM_COMPLETION_ACTIVE) == 0,
                "Alt-Enter accepts completion before capturing the split caret");
            view.Key(VIEM_KEY_ESCAPE);
            view.Undo();
            Check(Source() == original, "completion acceptance and Alt-Enter insertion share the typing undo unit");
        }

        internal async Task TabAndShiftTab()
        {
            PlaceInSecond();
            view.Key(VIEM_KEY_CHARACTER, 'i');
            pane.FocusEditor();
            await InputRoutingTests.Settle();
            await InputRoutingTests.Key(VirtualKey.Tab);
            CheckNested("native Tab indents from the middle of a list paragraph");
            Check(DecorationLabels(view).Contains("a."),
                "native Tab renders the first nested ordered item with a lower-alpha marker");
            await InputRoutingTests.Key(VirtualKey.Tab, shift: true);
            CheckTopLevel("native Shift-Tab unindents from the middle of a list paragraph");
            Check(view.Presentation.mode == VIEM_MODE_INSERT,
                "native structural Tab keys preserve Insert mode");
            await InputRoutingTests.Key(VirtualKey.Escape);
        }

        internal async Task BackspaceUnindentsThenRemovesList()
        {
            PlaceInSecond();
            view.IndentList(false);
            PlaceAtSecondStart();
            view.Key(VIEM_KEY_CHARACTER, 'i');
            pane.FocusEditor();
            await InputRoutingTests.Settle();
            await InputRoutingTests.Key(VirtualKey.Back);
            CheckTopLevel("native Backspace at a nested item start unindents exactly one level");
            await InputRoutingTests.Key(VirtualKey.Back);
            Check(document.FormattedText() == "First\nSecond" && Source() != original
                && view.ListCapabilities() == 0,
                "native Backspace at a top-level item start removes its list treatment");
        }

    }
}
#endif

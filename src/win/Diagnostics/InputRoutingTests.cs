#if DEBUG
using System.Runtime.InteropServices;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Editor;
using Windows.System;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

/// <summary>Exercise WinUI's native input window, not the core input wrappers.</summary>
internal static class InputRoutingTests
{
    [DllImport("user32.dll")] private static extern nint GetFocus();
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(nint hwnd, out uint process);
    [DllImport("user32.dll")] private static extern uint MapVirtualKey(uint key, uint type);
    [DllImport("user32.dll")] private static extern bool GetKeyboardState([Out] byte[] state);
    [DllImport("user32.dll")] private static extern bool SetKeyboardState(byte[] state);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] private static extern bool PostMessage(nint hwnd, uint message, nuint wparam, nint lparam);

    private static T? Find<T>(DependencyObject root) where T : DependencyObject
    {
        if (root is T item) return item;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(root); i++)
            if (Find<T>(VisualTreeHelper.GetChild(root, i)) is T child) return child;
        return null;
    }
    private static nint NativeTarget()
    {
        nint focused = GetFocus();
        GetWindowThreadProcessId(focused, out uint process);
        if (focused == 0 || process != Environment.ProcessId) throw new InvalidOperationException("The native keyboard target is not owned by the test application.");
        return focused;
    }
    private static void Post(uint message, nuint value, nint data)
    {
        // Never inject global input or target another application's window.
        if (!PostMessage(NativeTarget(), message, value, data)) throw new InvalidOperationException("Cannot post input to the native test window.");
    }
    internal static async Task Text(string text)
    {
        foreach (char value in text) Post(0x0102, value, 1); // WM_CHAR, including UTF-16 pairs.
        await Task.Delay(100);
    }
    internal static async Task Key(VirtualKey key, bool control = false, bool shift = false)
    {
        byte[]? original = null;
        if (control || shift)
        {
            _ = NativeTarget();
            original = new byte[256];
            if (!GetKeyboardState(original)) throw new InvalidOperationException("Cannot read the test thread's keyboard state.");
            byte[] modified = (byte[])original.Clone();
            // SetKeyboardState affects only this UI thread; no global input is
            // injected. Keep modifiers down until WinUI consumes the messages.
            modified[(int)VirtualKey.Control] = modified[(int)VirtualKey.LeftControl] = control ? (byte)0x80 : (byte)0;
            modified[(int)VirtualKey.Shift] = modified[(int)VirtualKey.LeftShift] = shift ? (byte)0x80 : (byte)0;
            if (!SetKeyboardState(modified)) throw new InvalidOperationException("Cannot set the test thread's keyboard modifiers.");
        }
        try
        {
            uint scan = MapVirtualKey((uint)key, 0);
            Post(0x0100, (uint)key, (nint)(1u | scan << 16)); // WM_KEYDOWN
            Post(0x0101, (uint)key, (nint)(0xC0000001u | scan << 16)); // WM_KEYUP
            await Task.Delay(100);
        }
        finally
        {
            if (original != null && !SetKeyboardState(original)) throw new InvalidOperationException("Cannot restore the test thread's keyboard state.");
        }
    }
    private static void Check(bool value, string name)
    {
        if (!value) throw new InvalidOperationException(name);
        FrontendSmokeTests.UiChecks.Add(name);
    }
    public static async Task Run(EditorPane pane)
    {
        await Task.Delay(150);
        var input = Find<TextBox>(pane) ?? throw new InvalidOperationException("No native editor input host.");
        Check(ReferenceEquals(FocusManager.GetFocusedElement(pane.XamlRoot), input), "editor receives initial native keyboard focus");
        await Text("i");
        Check(pane.View!.Presentation.mode == VIEM_MODE_INSERT, "native character input enters Insert mode");
        await Text("Native café 日本語 👩‍💻");
        Check(pane.Document.FormattedText() == "Native café 日本語 👩‍💻", "native text input preserves Unicode and rapid character order");
        await Key(VirtualKey.Back);
        Check(pane.Document.FormattedText() == "Native café 日本語 ", "native Backspace deletes one grapheme through vi");
        await Key(VirtualKey.Enter); await Text("next"); await Key(VirtualKey.Escape);
        Check(pane.Document.FormattedText() == "Native café 日本語 \nnext" && pane.View.Presentation.mode == VIEM_MODE_NORMAL, "native Enter and Escape retain editor command behavior");
        await Text(":set nowrap");
        Check(pane.View.Prompt().Text == "set nowrap", "native characters enter the command prompt");
        await Key(VirtualKey.Back); await Text("p"); await Key(VirtualKey.Enter);
        Check((pane.View.Viewport.flags & VIEM_VIEWPORT_STATE_WRAP) == 0, "native command-line editing and execution");
        await Text("ggiX"); await Key(VirtualKey.Escape);
        Check(pane.Document.FormattedText().StartsWith("XNative"), "queued characters observe preceding vi mode changes");
        await Text("u");
        Check(pane.Document.FormattedText() == "Native café 日本語 \nnext", "native vi undo after Insert mode");
        await Key(VirtualKey.Z, control: true, shift: true);
        Check(pane.Document.FormattedText().StartsWith("XNative"), "native Ctrl+Shift+Z redoes the core edit");
        await Key(VirtualKey.Z, control: true);
        Check(pane.Document.FormattedText() == "Native café 日本語 \nnext", "native Ctrl+Z undoes the core edit");
        await Text("ggiUndo café 👩‍💻");
        await Key(VirtualKey.Z, control: true);
        Check(pane.Document.FormattedText() == "Native café 日本語 \nnext" && pane.View.Presentation.mode == VIEM_MODE_NORMAL,
            "Ctrl+Z finalizes and undoes the active Insert group");
        await Key(VirtualKey.Z, control: true, shift: true);
        Check(pane.Document.FormattedText() == "Undo café 👩‍💻Native café 日本語 \nnext", "Ctrl+Shift+Z restores the entire Unicode Insert group");
        await Key(VirtualKey.Z, control: true);
        await Key(VirtualKey.R, control: true);
        Check(pane.Document.FormattedText().StartsWith("Undo café 👩‍💻"), "vi Ctrl+R remains available for redo");
        await Text("u");
        await Text("ggi"); await Key(VirtualKey.Q, control: true); await Key(VirtualKey.Z, control: true);
        Check(pane.Document.FormattedText() == "\u001aNative café 日本語 \nnext", "literal-next Ctrl+Z inserts its control character instead of undoing");
        await Key(VirtualKey.Escape); await Key(VirtualKey.Z, control: true);
        await Text(":%d"); await Key(VirtualKey.Enter);
        Check(pane.Document.FormattedText() == "" && pane.LastError == null, "native input test leaves an empty document without routing errors");
        pane.View.Wrap(true);
    }
    public static async Task FocusPane(EditorPane pane, string text)
    {
        pane.FocusEditor(); await Task.Delay(80);
        var input = Find<TextBox>(pane)!;
        Check(ReferenceEquals(FocusManager.GetFocusedElement(pane.XamlRoot), input), "pane switch transfers native keyboard focus");
        await Text("i" + text); await Key(VirtualKey.Escape);
        if (pane.LastError != null) throw pane.LastError;
        Check(pane.Document.FormattedText().Contains(text), "typing reaches the focused pane after a split");
        await Text("u");
    }
}
#endif

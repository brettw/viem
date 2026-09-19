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
    [StructLayout(LayoutKind.Sequential)] private struct NativePoint { public int X, Y; }
    [DllImport("user32.dll")] private static extern nint WindowFromPoint(NativePoint point);
    [DllImport("user32.dll")] private static extern nint GetForegroundWindow();
    [DllImport("user32.dll")] private static extern bool GetCursorPos(out NativePoint point);
    [DllImport("user32.dll")] private static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] private static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll")] private static extern bool ClientToScreen(nint hwnd, ref NativePoint point);
    [StructLayout(LayoutKind.Sequential)] private struct MouseInput { public int X, Y; public uint Data, Flags, Time; public nuint Extra; }
    [StructLayout(LayoutKind.Sequential)] private struct NativeInput { public uint Type; public MouseInput Mouse; }
    [DllImport("user32.dll", SetLastError = true)] private static extern uint SendInput(uint count, NativeInput[] inputs, int size);

    internal static async Task Drag(Window owner, FrameworkElement element, IReadOnlyList<global::Windows.Foundation.Point> fractions, Action<int> inspect)
    {
        element.Focus(FocusState.Programmatic);
        await Task.Delay(150);
        // The automation peer for a windowed flyout omits the owner's client
        // origin. Map XAML root coordinates explicitly to physical screen pixels.
        var logical = element.TransformToVisual(owner.Content).TransformBounds(new(0, 0, element.ActualWidth, element.ActualHeight));
        var origin = new NativePoint();
        if (!ClientToScreen(WinRT.Interop.WindowNative.GetWindowHandle(owner), ref origin)) throw new InvalidOperationException("Cannot map the test window's client origin.");
        double scale = element.XamlRoot.RasterizationScale;
        var bounds = new global::Windows.Foundation.Rect(origin.X + logical.X * scale, origin.Y + logical.Y * scale, logical.Width * scale, logical.Height * scale);
        bool Owned(nint hwnd)
        {
            GetWindowThreadProcessId(hwnd, out uint process);
            return hwnd != 0 && process == Environment.ProcessId;
        }
        NativePoint Position(global::Windows.Foundation.Point fraction) => new() {
            X = (int)Math.Round(bounds.X + bounds.Width * fraction.X), Y = (int)Math.Round(bounds.Y + bounds.Height * fraction.Y)
        };
        void Move(NativePoint point)
        {
            // WinUI's pointer pipeline ignores posted legacy mouse messages.
            // Only drive a real pointer while both the foreground window and
            // every point in the drag belong to this isolated test process.
            if (!Owned(GetForegroundWindow()) || !Owned(WindowFromPoint(point)))
                throw new InvalidOperationException("The pointer test target is not owned by the test application.");
            NativeInput[] input = [new() { Mouse = new() {
                X = (int)Math.Round((point.X - GetSystemMetrics(76)) * 65535d / (GetSystemMetrics(78) - 1)),
                Y = (int)Math.Round((point.Y - GetSystemMetrics(77)) * 65535d / (GetSystemMetrics(79) - 1)),
                Flags = 0xE001 // MOVE | ABSOLUTE | VIRTUALDESK | MOVE_NOCOALESCE
            } }];
            if (SendInput(1, input, Marshal.SizeOf<NativeInput>()) != 1) throw new InvalidOperationException("Cannot position the test pointer.");
        }
        void Button(uint flags)
        {
            NativeInput[] input = [new() { Mouse = new() { Flags = flags } }];
            if (SendInput(1, input, Marshal.SizeOf<NativeInput>()) != 1) throw new InvalidOperationException("Cannot send the test pointer button.");
        }
        if (!GetCursorPos(out var original)) throw new InvalidOperationException("Cannot save the pointer position.");
        var position = Position(fractions[0]);
        Move(position); Button(0x0002);
        try
        {
            for (int i = 0; i < fractions.Count; i++)
            {
                position = Position(fractions[i]); Move(position);
                await Task.Delay(8); inspect(i);
            }
        }
        finally
        {
            Button(0x0004);
            if (Owned(GetForegroundWindow()) && GetCursorPos(out var current) && Math.Abs(current.X - position.X) <= 1 && Math.Abs(current.Y - position.Y) <= 1)
                SetCursorPos(original.X, original.Y);
        }
        await Task.Delay(60);
    }

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

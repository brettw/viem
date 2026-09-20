using Windows.System;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Input;

internal enum NativeAction { None, Copy, Cut, Paste, PastePlain, Undo, Redo, Save, SaveAs, Heading, Styles }
internal readonly record struct KeyRoute(uint Kind = 0, uint Codepoint = 0, uint Modifiers = 0, NativeAction Action = NativeAction.None);

/// <summary>Reserve Windows shortcuts explicitly; preserve all other vi bindings.</summary>
internal static class KeyPolicy
{
    public static KeyRoute Route(VirtualKey key, bool control, bool shift, bool alt, bool literal = false)
    {
        if (!literal && key == VirtualKey.F8 && !control && !shift && !alt) return new(Action: NativeAction.Styles);
        // These Windows clipboard overrides also apply during literal-next input.
        // AltGr is text input, not a Control shortcut.
        if (control && !alt)
        {
            if (key == VirtualKey.C) return new(Action: NativeAction.Copy);
            if (key == VirtualKey.X) return new(Action: NativeAction.Cut);
            if (key == VirtualKey.V) return new(Action: shift ? NativeAction.PastePlain : NativeAction.Paste);
            if (!literal && key == VirtualKey.Z) return new(Action: shift ? NativeAction.Redo : NativeAction.Undo);
            if (!literal && key == VirtualKey.S) return new(Action: shift ? NativeAction.SaveAs : NativeAction.Save);
            // Ctrl+6/Ctrl+^ belongs to vi. Heading 6 remains a menu action.
            if (!literal && !shift && key >= VirtualKey.Number0 && key <= VirtualKey.Number5) return new(Codepoint: (uint)(key - VirtualKey.Number0), Action: NativeAction.Heading);
            if (key >= VirtualKey.A && key <= VirtualKey.Z) return new(VIEM_KEY_CONTROL_CHARACTER, (uint)('a' + key - VirtualKey.A));
            if (key == VirtualKey.Space) return new(VIEM_KEY_CONTROL_CHARACTER, '@');
            if ((int)key == 219) return new(VIEM_KEY_CONTROL_CHARACTER, '[');
            if ((int)key == 220) return new(VIEM_KEY_CONTROL_CHARACTER, '\\');
            if ((int)key == 221) return new(VIEM_KEY_CONTROL_CHARACTER, ']');
            if (key == VirtualKey.Number6) return new(VIEM_KEY_CONTROL_CHARACTER, '^');
            if ((int)key == 189) return new(VIEM_KEY_CONTROL_CHARACTER, '_');
        }
        uint modifiers = (shift ? VIEM_KEY_MODIFIER_SHIFT : 0) | (control ? VIEM_KEY_MODIFIER_CONTROL : 0) | (alt ? VIEM_KEY_MODIFIER_ALT : 0);
        if (key >= VirtualKey.F1 && key <= VirtualKey.F24)
            return new(VIEM_KEY_FUNCTION, (uint)(key - VirtualKey.F1 + 1), modifiers);
        uint kind = key switch
        {
            VirtualKey.Escape => VIEM_KEY_ESCAPE,
            VirtualKey.Enter => shift ? VIEM_KEY_SHIFT_ENTER : VIEM_KEY_ENTER,
            VirtualKey.Tab => shift ? VIEM_KEY_BACK_TAB : VIEM_KEY_TAB,
            VirtualKey.Back => VIEM_KEY_BACKSPACE,
            VirtualKey.Delete => VIEM_KEY_DELETE,
            VirtualKey.Left => control ? VIEM_KEY_WORD_LEFT : VIEM_KEY_LEFT,
            VirtualKey.Right => control ? VIEM_KEY_WORD_RIGHT : VIEM_KEY_RIGHT,
            VirtualKey.Up => VIEM_KEY_UP, VirtualKey.Down => VIEM_KEY_DOWN,
            VirtualKey.Home => control ? VIEM_KEY_DOCUMENT_START : VIEM_KEY_HOME,
            VirtualKey.End => control ? VIEM_KEY_DOCUMENT_END : VIEM_KEY_END,
            VirtualKey.PageUp => VIEM_KEY_PAGE_UP, VirtualKey.PageDown => VIEM_KEY_PAGE_DOWN,
            _ => 0
        };
        bool movement = kind is VIEM_KEY_LEFT or VIEM_KEY_RIGHT or VIEM_KEY_UP or VIEM_KEY_DOWN
            or VIEM_KEY_HOME or VIEM_KEY_END or VIEM_KEY_PAGE_UP or VIEM_KEY_PAGE_DOWN
            or VIEM_KEY_WORD_LEFT or VIEM_KEY_WORD_RIGHT or VIEM_KEY_DOCUMENT_START or VIEM_KEY_DOCUMENT_END;
        return new(kind, Modifiers: movement ? modifiers : 0);
    }
}

using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Viem.Windows.Core;
using Windows.System;

namespace Viem.Windows.Shell;

internal sealed partial class StyleWindow
{
    private sealed record ThemeStyleEdit(byte[] Before, byte[] After, StyleKey Selection);
    private readonly List<ThemeStyleEdit> themeUndo = new();
    private readonly Stack<ThemeStyleEdit> themeRedo = new();
    private readonly Button undoTheme = new() { Content = "Undo", MinWidth = 72, IsEnabled = false };
    private readonly Button redoTheme = new() { Content = "Redo", MinWidth = 72, IsEnabled = false };
    private readonly Button restoreCodeDefaults = new() { Content = "Restore Defaults", Visibility = Visibility.Collapsed };
    private int themeHistoryGroupDepth;
    private ThemeStyleEdit? themeHistoryGroup;
    private sealed record ThemeFamilyHistory(string? ThemeName, string? ThemePath, byte[] Styles, ThemeStyleEdit[] Undo, ThemeStyleEdit[] Redo);
    private readonly Dictionary<string, ThemeFamilyHistory> familyHistory = new();

    private void SwitchThemeHistory(string family, uint format)
    {
        if (sessionFamily.Length > 0)
            familyHistory[sessionFamily] = new(sessionThemeName, sessionThemePath, sessionThemeStyles, themeUndo.ToArray(), themeRedo.ToArray());
        ClearThemeHistory();
        if (familyHistory.Remove(family, out var history) && history.ThemeName == preferences.SelectedTheme
            && history.ThemePath == preferences.SelectedThemePath
            && history.Styles.AsSpan().SequenceEqual(preferences.ThemeStyleDefaults(format))) {
            themeUndo.AddRange(history.Undo);
            foreach (var edit in history.Redo.Reverse()) themeRedo.Push(edit);
            RefreshThemeHistory();
        }
    }

    private void BuildThemeHistory(Grid buttons)
    {
        var history = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        history.Children.Add(undoTheme); history.Children.Add(redoTheme); history.Children.Add(restoreCodeDefaults); buttons.Children.Add(history);
        ToolTipService.SetToolTip(undoTheme, "Undo theme style change (Ctrl+Z)");
        ToolTipService.SetToolTip(redoTheme, "Redo theme style change (Ctrl+Y)");
        undoTheme.Click += (_, _) => ReplayThemeHistory(redo: false);
        redoTheme.Click += (_, _) => ReplayThemeHistory(redo: true);
        restoreCodeDefaults.Click += (_, _) => Try(() => view.ReplaceCodeStyles([]));
        foreach (var (key, modifiers, redo) in new[] {
            (VirtualKey.Z, VirtualKeyModifiers.Control, false),
            (VirtualKey.Y, VirtualKeyModifiers.Control, true),
            (VirtualKey.Z, VirtualKeyModifiers.Control | VirtualKeyModifiers.Shift, true)
        }) {
            var accelerator = new KeyboardAccelerator { Key = key, Modifiers = modifiers };
            accelerator.Invoked += (_, args) => { args.Handled = true; ReplayThemeHistory(redo); };
            root.KeyboardAccelerators.Add(accelerator);
        }
    }
    private void RefreshThemeHistory() { undoTheme.IsEnabled = themeUndo.Count > 0 || themeHistoryGroup != null; redoTheme.IsEnabled = themeRedo.Count > 0; }
    private void ClearThemeHistory()
    {
        themeUndo.Clear(); themeRedo.Clear(); themeHistoryGroup = null; themeHistoryGroupDepth = 0; RefreshThemeHistory();
    }
    private void BeginThemeHistoryGroup() => themeHistoryGroupDepth++;
    private void EndThemeHistoryGroup()
    {
        if (themeHistoryGroupDepth == 0 || --themeHistoryGroupDepth > 0) return;
        if (themeHistoryGroup is { } edit && !edit.Before.AsSpan().SequenceEqual(edit.After)) PushThemeUndo(edit);
        themeHistoryGroup = null; RefreshThemeHistory();
    }
    private void PushThemeUndo(ThemeStyleEdit edit)
    {
        themeUndo.Add(edit);
        if (themeUndo.Count > 64) themeUndo.RemoveAt(0);
    }
    private void RecordThemeEdit(byte[] before, byte[] after, StyleKey key)
    {
        if (before.AsSpan().SequenceEqual(after)) return;
        themeRedo.Clear();
        if (themeHistoryGroupDepth > 0) themeHistoryGroup = new(themeHistoryGroup?.Before ?? before, after, key);
        else PushThemeUndo(new(before, after, key));
        RefreshThemeHistory();
    }
    private void ReplayThemeHistory(bool redo)
    {
        if (closed || updating) return;
        DismissColorPickers(commit: false);
        if (preferences.EnsureCurrentThemeExists()) return;
        var edit = redo ? themeRedo.FirstOrDefault() : themeUndo.LastOrDefault();
        if (edit == null) return;
        updating = true;
        try {
            preferences.SaveThemeStyles(view.Document.State.format, redo ? edit.After : edit.Before);
            CreateThemeSession(); Load(edit.Selection);
            if (redo) { themeRedo.Pop(); PushThemeUndo(edit); }
            else { themeUndo.RemoveAt(themeUndo.Count - 1); themeRedo.Push(edit); }
            error.Text = ""; error.Visibility = Visibility.Collapsed;
        }
        catch (Exception exception) { error.Text = exception.Message; error.Visibility = Visibility.Visible; }
        finally { updating = false; RefreshThemeHistory(); }
    }
#if DEBUG
    internal void UndoThemeForTesting() => ReplayThemeHistory(redo: false);
    internal void RedoThemeForTesting() => ReplayThemeHistory(redo: true);
#endif
}

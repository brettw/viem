using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed record CodeSyntaxStyle(string Name);

internal sealed partial class EditorWindow
{
    private sealed record CodeStyleAction(CoreView View, ulong DocumentId, ulong DocumentRevision,
        ulong StyleRevision, StyleKey? Key, string? SyntaxName);
    private string codeStyleCatalogueWarning = "";

    private string[] AcceptedSyntaxStyleNames(CoreDocument document)
    {
        try { return document.SyntaxStyleNames(); }
        catch (Exception error) when (error is CoreException or InvalidDataException or System.Text.Json.JsonException)
        {
            string warning = "Could not display additional syntax styles. " + error.Message;
            if (warning != codeStyleCatalogueWarning) { codeStyleCatalogueWarning = warning; ActivePane?.SetMessage(warning); }
            return [];
        }
    }

    private void RefreshCodeStyleMenus(CoreView current, SelectedStyles selection, StyleSheet snapshot)
    {
        var state = current.Document.State;
        bool matches = selection.Identity.document_id == state.document_id
            && selection.Identity.document_revision == state.document_revision
            && selection.Identity.style_sheet_revision == snapshot.Identity.style_sheet_revision;
        var choices = snapshot.Styles.Where(style => style.Namespace is 1 or 2
            && (style.Native.flags & VIEM_STYLE_DEFINITION_INTERNAL) == 0)
            .Select(style => new StyleChoice(style.Key, style.Name, true, matches && (style.Namespace == 1
                ? !selection.ParagraphMixed && selection.Paragraph == style.Id
                : !selection.CharacterMixed && selection.Character == style.Id))).ToList();
        choices.Add(new(new(2, ""), "Default Paragraph", true,
            matches && !selection.CharacterMixed && selection.Character.Length == 0));
        var definedNames = snapshot.Styles.Where(style => style.Namespace == 2).Select(style => style.Name).ToHashSet(StringComparer.Ordinal);
        // This query only inspects accepted syntax output; opening a menu never
        // starts analysis or waits for offscreen text to be highlighted.
        var syntaxNames = AcceptedSyntaxStyleNames(current.Document).Where(name => !definedNames.Contains(name)).ToArray();
        foreach (var menu in new[] { paragraphMenu, characterMenu })
        {
            uint space = menu == paragraphMenu ? 1u : 2u;
            var entries = choices.Where(choice => choice.Key.Namespace == space)
                .Select(choice => (Tag: (object)choice.Key, choice.Name, choice.Selected))
                .Concat(space == 2 ? syntaxNames.Select(name => (Tag: (object)new CodeSyntaxStyle(name), Name: name, Selected: false)) : [])
                .OrderBy(choice => choice.Name, StringComparer.CurrentCultureIgnoreCase).ToArray();
            var old = menu.Items.OfType<ToggleMenuFlyoutItem>().Where(item => item.Tag is StyleKey or CodeSyntaxStyle).ToArray();
            if (!old.Select(item => item.Tag).SequenceEqual(entries.Select(entry => entry.Tag)))
            {
                foreach (var item in old) menu.Items.Remove(item);
                foreach (var entry in entries)
                {
                    var item = new ToggleMenuFlyoutItem { Tag = entry.Tag };
                    item.Click += (_, _) => Safe(() => {
                        if (item.CommandParameter is CodeStyleAction action) OpenCodeStyle(action);
                        else if (item.Tag is StyleKey key && item.CommandParameter is Viem.Windows.Interop.ViemStyleSheetIdentityV1 identity) {
                            View?.ChooseStyle(key, identity); RestoreEditorFocusAfterControl();
                        }
                        return Task.CompletedTask;
                    });
                    menu.Items.Add(item);
                }
            }
            var items = menu.Items.OfType<ToggleMenuFlyoutItem>().Where(item => item.Tag is StyleKey or CodeSyntaxStyle).ToArray();
            for (int index = 0; index < entries.Length; index++)
            {
                var entry = entries[index]; var item = items[index];
                item.Text = entry.Name; item.IsEnabled = true; item.IsChecked = entry.Selected;
                item.KeyboardAcceleratorTextOverride = "";
                item.CommandParameter = new CodeStyleAction(current, state.document_id, state.document_revision,
                    snapshot.Identity.style_sheet_revision, entry.Tag is StyleKey key ? key : null,
                    (entry.Tag as CodeSyntaxStyle)?.Name);
            }
        }
    }

    private void OpenCodeStyle(CodeStyleAction action)
    {
        // Toggle controls display the caret's syntax assignment. Opening a
        // definition must not leave their temporary click checkmark selected.
        menusDirty = true;
        if (!ReferenceEquals(View, action.View) || action.View.Id == 0 || !action.View.UsesGlobalStyles) return;
        var state = action.View.Document.State;
        var snapshot = action.View.Styles();
        if (state.document_id != action.DocumentId || state.document_revision != action.DocumentRevision
            || snapshot.Identity.style_sheet_revision != action.StyleRevision) return;
        StyleKey? key = action.Key;
        if (action.SyntaxName is { } name)
        {
            if (!action.View.Document.SyntaxStyleNames().Contains(name, StringComparer.Ordinal)) return;
            action.View.MaterializeCodeStyle(name);
            snapshot = action.View.Styles();
            key = snapshot.Styles.FirstOrDefault(style => style.Namespace == 2 && style.Name == name)?.Key;
        }
        if (key is { Namespace: 2, Id.Length: 0 })
            key = snapshot.Styles.FirstOrDefault(style => (style.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0)?.Key;
        if (key is not { } target || !snapshot.Styles.Any(style => style.Key == target
            && (style.Native.flags & VIEM_STYLE_DEFINITION_INTERNAL) == 0)) return;
        ShowStyles();
        styleInspector?.SelectStyle(target);
    }
}

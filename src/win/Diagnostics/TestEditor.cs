#if DEBUG
using System.Text;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

/// Owns native test chrome and retires each case's document and view state.
internal sealed class TestEditor : IAsyncDisposable
{
    internal sealed record Case(string Name, Func<TestEditor, Task> Run, bool UsesEditor = true);

    /// Share expensive native chrome, with an explicit reset policy for each group.
    internal static SelfTestRunner.Stage Group(string name, Preferences preferences, string source,
        Case[] cases, uint format = VIEM_FORMAT_MARKDOWN,
        Action? initialize = null, Action? cleanup = null, Func<TestEditor, Task>? resetCase = null)
    {
        TestEditor? editor = null;
        bool first = true;
        bool previousFailed = false;
        return new(name, async () =>
        {
            initialize?.Invoke();
            editor = await Open(preferences, source, format);
        }, Cases: cases.Select(test => new SelfTestRunner.Stage(name + "." + test.Name, async () =>
        {
            try
            {
                if (test.UsesEditor)
                {
                    if (!first)
                    {
                        if (previousFailed || resetCase == null) await editor!.Reset(source, format);
                        else await resetCase(editor!);
                    }
                    first = false;
                }
                await test.Run(editor!);
                previousFailed = false;
            }
            catch
            {
                previousFailed = true;
                throw;
            }
        })).ToArray(), Cleanup: async () =>
        {
            try
            {
                if (editor != null) await editor.DisposeAsync();
            }
            finally
            {
                cleanup?.Invoke();
            }
        });
    }

    /// For cases that undo their own edits and change no view options or styles.
    /// Check their source baseline before clearing transient UI/caret state;
    /// a failed case always takes the fresh-document path in Group instead.
    internal Task RestoreCaret(string expectedSource, uint format = VIEM_FORMAT_MARKDOWN)
    {
        if (Window.Panes.Count != 1 || Document.State.format != format || Source != expectedSource
            || Pane.LastError != null)
            throw new InvalidOperationException("The previous case did not restore its editor fixture.");
        View.Key(VIEM_KEY_ESCAPE);
        Pane.DismissLinkPopup(suppress: false);
        Pane.DismissImagePopup(suppress: false);
        View.Place(0, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, Document.State.document_revision);
        Window.FocusPane(Pane);
        return Task.CompletedTask;
    }

    internal EditorWindow Window { get; }
    internal EditorPane Pane => Window.ActivePane!;
    internal CoreView View { get; private set; } = null!;
    internal CoreDocument Document { get; private set; }
    internal string Source => Encoding.UTF8.GetString(Document.Source(Document.State.document_revision));

    private TestEditor(Preferences preferences, string source, uint format)
    {
        Document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: format);
        Window = new EditorWindow(preferences, Document);
        App.Instance.Windows.Add(Window);
    }

    internal static async Task<TestEditor> Open(Preferences preferences, string source, uint format = VIEM_FORMAT_MARKDOWN)
    {
        var editor = new TestEditor(preferences, source, format);
        try
        {
            editor.Window.Activate();
            editor.View = await editor.Pane.Ready;
            await InputRoutingTests.Settle();
            return editor;
        }
        catch
        {
            await editor.DisposeAsync();
            throw;
        }
    }

    /// Reuses native chrome while retiring all document, view and popup state.
    internal async Task Reset(string source, uint format = VIEM_FORMAT_MARKDOWN)
    {
        var previous = Window.Panes[0];
        foreach (var pane in Window.Panes.Skip(1).ToArray())
            await Window.ClosePane(pane, force: true);
        Document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: format);
        var next = Window.AddPane(Document, replacing: previous);
        View = await next.Ready;
        Window.FocusPane(next);
        next.UpdateLayout();
        await InputRoutingTests.Settle();
    }

    public async ValueTask DisposeAsync()
    {
        try
        {
            foreach (var pane in Window.Panes.ToArray())
                await Window.ClosePane(pane, force: true);
        }
        finally
        {
            App.Instance.Windows.Remove(Window);
        }
    }
}
#endif

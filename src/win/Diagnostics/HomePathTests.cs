#if DEBUG
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Shell;

namespace Viem.Windows.Diagnostics;

internal static class HomePathTests
{
    private static void Check(bool value, string name)
    { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }

    private static EditorWindow Open(Preferences preferences, string text = "Original")
    {
        var window = new EditorWindow(preferences, new CoreDocument(System.Text.Encoding.UTF8.GetBytes(text)));
        App.Instance.Windows.Add(window);
        window.Closed += (_, _) => App.Instance.Windows.Remove(window);
        window.Activate();
        return window;
    }

    private static async Task Ex(EditorWindow window, EditorPane pane, string command)
    {
        await pane.Ready;
        pane.View!.Ex(command);
        await window.PendingEffectsForTesting.WaitAsync(TimeSpan.FromSeconds(10));
        if (pane.LastError is { } error) throw error;
    }

    internal static async Task Run(Preferences preferences)
    {
        string home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        // Keep the disposable fixture on the home drive even when the test
        // profile/repository lives on another volume.
        string directory = Path.Combine(home, ".viem-home-path-test-" + Guid.NewGuid().ToString("N"));
        string HomePath(string path, bool backslash = false) => "~" + (backslash ? "\\" : "/")
            + Path.GetRelativePath(home, path).Replace('\\', backslash ? '\\' : '/');
        string callerDirectory = Path.Combine(directory, "caller");
        string source = Path.Combine(directory, "source.txt");
        string launched = Path.Combine(callerDirectory, "launch relative.txt");
        Directory.CreateDirectory(callerDirectory);
        await File.WriteAllTextAsync(source, "Home file\n");
        await File.WriteAllTextAsync(launched, "Caller file\n");
        string cwd = Environment.CurrentDirectory;
        string[] arguments = App.Instance.Arguments.ToArray();
        var initialWindows = App.Instance.Windows.ToHashSet();
        void CloseTests()
        { foreach (var window in App.Instance.Windows.Where(w => !initialWindows.Contains(w)).ToArray()) window.Close(); }
        try
        {
            var window = Open(preferences); var pane = window.ActivePane!;
            await Ex(window, pane, "cd ~");
            Check(Environment.CurrentDirectory == home, ":cd ~ resolves the Windows user profile");
            foreach (bool backslash in new[] { false, true })
            {
                await Ex(window, pane, "cd " + HomePath(directory, backslash));
                Check(Environment.CurrentDirectory == directory, ":cd expands the home prefix with " + (backslash ? "backslashes" : "slashes"));
            }
            int picks = 0;
            window.PickDirectoryFileForTesting = path => {
                picks++;
                Check(Path.TrimEndingDirectorySeparator(path) == home, "home directory opens the picker at the user profile");
                return Task.FromResult<string?>(null);
            };
            foreach (string command in new[] { "e", "sp" })
                foreach (string path in new[] { "~", "~/", "~\\" }) await Ex(window, pane, command + " " + path);
            Check(picks == 6 && window.Panes.Count == 1 && window.ActivePane == pane,
                "cancelled home-directory edits and splits preserve the original pane");
            foreach (bool backslash in new[] { false, true })
            {
                await Ex(window, window.ActivePane!, "e " + HomePath(source, backslash));
                Check(window.ActivePane!.Document.FilePath == source && window.ActivePane.Document.FormattedText() == "Home file\n",
                    ":edit opens a home-relative file with " + (backslash ? "backslashes" : "slashes"));
            }
            CloseTests();

            window = Open(preferences); pane = window.ActivePane!;
            string written = Path.Combine(directory, "written.txt"), saved = Path.Combine(directory, "saved.txt"), direct = Path.Combine(directory, "direct.txt");
            await Ex(window, pane, "w " + HomePath(written));
            Check(File.ReadAllText(written) == "Original" && pane.Document.FilePath == written, ":write expands a home-relative destination");
            await Ex(window, pane, "saveas " + HomePath(saved, true));
            Check(File.ReadAllText(saved) == "Original" && pane.Document.FilePath == saved, ":saveas expands a backslash home-relative destination");
            Check(await window.Save(pane, explicitPath: HomePath(direct)) && File.ReadAllText(direct) == "Original"
                && pane.Document.FilePath == direct, "explicit native Save expands a home-relative destination");
            string exported = Path.Combine(directory, "exported.html");
            Check(await window.Export(pane, HomePath(exported, true)) && File.ReadAllText(exported).Contains("Original")
                && pane.Document.FilePath == direct, "explicit HTML export expands a home-relative destination without rebinding the document");
            CloseTests();

            Environment.CurrentDirectory = home;
            window = Open(preferences, "");
            await window.OpenArgumentsForTesting(new OpenInvocation([HomePath(source, true), "lazy.txt"], callerDirectory));
            Check(window.ActivePane!.Document.FilePath == source && App.Instance.Arguments.SequenceEqual(new[] { source, Path.Combine(callerDirectory, "lazy.txt") }),
                "launch expands tilde paths and resolves lazy relative arguments against caller cwd");
            CloseTests();
            window = Open(preferences, "");
            await window.OpenArgumentsForTesting(new OpenInvocation([Path.GetFileName(launched)], callerDirectory));
            Check(window.ActivePane!.Document.FilePath == launched && window.ActivePane.Document.FormattedText() == "Caller file\n",
                "ordinary launch filenames retain the caller cwd independently of the editor cwd");
        }
        finally
        {
            Environment.CurrentDirectory = cwd;
            CloseTests();
            App.Instance.Arguments.Clear(); App.Instance.Arguments.AddRange(arguments);
            foreach (string path in Directory.EnumerateFiles(directory)) File.Delete(path);
            File.Delete(launched); Directory.Delete(callerDirectory); Directory.Delete(directory);
        }
    }
}
#endif

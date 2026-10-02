#if DEBUG
using System.Diagnostics;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class BlockingEditTests
{
    private static void Check(bool value, string name)
    { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }

    // Separate GUI processes exercise cold launch, exit status and broken pipes.
    internal static async Task OnBlockingLaunch(EditorWindow window)
    {
        string? scenario = Environment.GetEnvironmentVariable("VIEM_TEST_BLOCKING_LAUNCH");
        if (scenario == null) return;
        var pane = window.ActivePane!;
        var view = await pane.Ready;
        if (scenario == "crash") Environment.Exit(123);
        view.Command("i"); view.Text("Edited "); view.Key(VIEM_KEY_ESCAPE);
        view.Ex(scenario == "cquit" ? "7cq" : "wq");
        await window.PendingEffectsForTesting;
    }

    private static Process Start(string path, string directory, string? scenario = null)
    {
        var launch = new ProcessStartInfo(Path.Combine(AppContext.BaseDirectory, "blocking-viem.exe"))
        { UseShellExecute = false, CreateNoWindow = true, WorkingDirectory = directory, RedirectStandardError = true };
        launch.ArgumentList.Add(path);
        if (scenario != null)
        {
            launch.Environment["VIEM_CONFIG_DIR"] = Path.Combine(directory, "profile-" + scenario);
            launch.Environment["VIEM_TEST_BLOCKING_LAUNCH"] = scenario;
        }
        return Process.Start(launch)!;
    }

    private static async Task Until(Func<bool> ready)
    {
        var timer = Stopwatch.StartNew();
        while (!ready())
        {
            if (timer.Elapsed > TimeSpan.FromSeconds(30)) throw new TimeoutException("Blocking edit launch timed out.");
            await Task.Delay(20);
        }
    }

    internal static async Task Run(Preferences preferences)
    {
        string directory = Path.Combine(preferences.DirectoryPath, "blocking files");
        Directory.CreateDirectory(directory);
        string path = Path.Combine(directory, "COMMIT_EDITMSG");
        File.WriteAllText(path, "Original\r\n");
        var initial = new EditorWindow(preferences);
        App.Instance.Windows.Add(initial); initial.Closed += (_, _) => App.Instance.Windows.Remove(initial); initial.Activate();
        await initial.ActivePane!.Ready;
        await initial.OpenPath(path);
        await initial.ActivePane!.Ready;
        using var first = Start("COMMIT_EDITMSG", directory);
        using var second = Start("COMMIT_EDITMSG", directory);
        try
        {
            try { await Until(() => App.Instance.BlockingEdits.CountForTesting == 2); }
            catch (Exception error)
            {
                string failures = string.Join("; ", App.Instance.Windows.SelectMany(w => w.Panes).Select(p => p.LastError?.ToString()));
                if (first.HasExited) failures += " first: " + first.ExitCode + " " + await first.StandardError.ReadToEndAsync();
                if (second.HasExited) failures += " second: " + second.ExitCode + " " + await second.StandardError.ReadToEndAsync();
                throw new InvalidOperationException("Pending sessions: " + App.Instance.BlockingEdits.CountForTesting + "; " + failures, error);
            }
            var window = App.Instance.Windows.First(w => w.Panes.Any(p => p.Document.FilePath == path));
            var pane = window.Panes.First(p => p.Document.FilePath == path);
            var document = pane.Document;
            var other = new EditorWindow(preferences, document);
            App.Instance.Windows.Add(other); other.Closed += (_, _) => App.Instance.Windows.Remove(other); other.Activate();
            await other.ActivePane!.Ready;
            await window.ClosePane(pane, true);
            Check(!first.HasExited && !second.HasExited, "blocking callers wait for every window containing the document");
            pane = other.ActivePane!;
            pane.View!.Ex("split"); await other.PendingEffectsForTesting;
            var split = other.ActivePane!; await split.Ready;
            await other.ClosePane(split, true);
            Check(!first.HasExited && !second.HasExited, "closing a split also waits for the remaining view");
            pane.View!.Ex("edit!"); await other.PendingEffectsForTesting;
            Check(App.Instance.BlockingEdits.CountForTesting == 2 && !first.HasExited, "reload retains blocking edit sessions");
            pane.View!.Command("i"); pane.View.Text("Edited "); pane.View.Key(VIEM_KEY_ESCAPE);
            other.CloseReviewDecisionForTesting = _ => Task.FromResult(Microsoft.UI.Xaml.Controls.ContentDialogResult.None);
            await other.RequestCloseForTesting();
            Check(!first.HasExited && App.Instance.BlockingEdits.CountForTesting == 2, "cancelled close keeps Git waiting");
            pane.View.Ex("w"); await other.PendingEffectsForTesting;
            Check(!first.HasExited && File.ReadAllText(path) == "Edited Original\r\n", "saving writes exact message and keeps Git waiting");
            await other.ClosePane(pane);
            await first.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(10));
            await second.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(10));
            Check(first.ExitCode == 0 && second.ExitCode == 0 && App.Instance.Windows.Count > 0,
                "last document close completes concurrent callers without quitting unrelated windows");
        }
        finally
        {
            if (!first.HasExited) first.Kill();
            if (!second.HasExited) second.Kill();
        }

        string lockedPath = Path.Combine(directory, "locked.txt"); File.WriteAllText(lockedPath, "locked");
        using (var locked = new FileStream(lockedPath, FileMode.Open, FileAccess.ReadWrite, FileShare.None))
        using (var failed = Start(lockedPath, directory))
        {
            try
            {
                await failed.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(30));
                Check(failed.ExitCode != 0, "GUI file-open failure releases the caller with failure");
            }
            finally { if (!failed.HasExited) failed.Kill(); }
        }

        foreach (string scenario in new[] { "write-quit", "cquit", "crash" })
        {
            string file = Path.Combine(directory, scenario + ".txt"); File.WriteAllText(file, "Original\r\n");
            using var child = Start(file, directory, scenario);
            try
            {
                await child.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(45));
                int expected = scenario == "write-quit" ? 0 : scenario == "cquit" ? 7 : 1;
                Check(child.ExitCode == expected, "cold blocking launch returns " + scenario + " status: " + await child.StandardError.ReadToEndAsync());
                Check(File.ReadAllText(file) == (scenario == "write-quit" ? "Edited Original\r\n" : "Original\r\n"),
                    "cold " + scenario + " preserves correct on-disk source");
            }
            finally { if (!child.HasExited) child.Kill(); }
        }
        using var invalid = Start(directory, directory);
        await invalid.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(10));
        Check(invalid.ExitCode != 0, "blocking launcher rejects directories");
    }
}
#endif

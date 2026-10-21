#if DEBUG
namespace Viem.Windows.Diagnostics;

internal static class SelfTestRunner
{
    internal sealed record Stage(string Name, Func<Task> Run, bool Setup = false,
        IReadOnlyList<Stage>? Cases = null, Func<Task>? Cleanup = null);

    /// Waits for asynchronous UI work by polling its observable result, up to
    /// a bound, instead of sleeping for a guessed duration. Returns whether
    /// the condition became true; the caller's check reports the failure.
    internal static async Task<bool> Until(Func<bool> condition, int timeoutMilliseconds = 2000)
    {
        var timer = System.Diagnostics.Stopwatch.StartNew();
        while (!condition())
        {
            if (timer.ElapsedMilliseconds > timeoutMilliseconds) return false;
            await Task.Delay(10);
        }
        return true;
    }

    /// Runs every selected stage, continuing after failures so one report
    /// lists each failing stage with its duration. After each stage,
    /// `closeStrayWindows` closes windows the stage left open and returns
    /// how many it closed, so later stages never relayout leaked fixtures.
    internal static async Task<bool> Run(IReadOnlyList<Stage> stages, Func<int> closeStrayWindows)
    {
        var selected = Environment.GetEnvironmentVariable("VIEM_TEST_SUITES") is { Length: > 0 } filter
            ? filter.Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries).ToHashSet(StringComparer.OrdinalIgnoreCase)
            : null;
        var timings = new List<NativeTestStage>();
        bool Matches(string filter, string name) => name.Equals(filter, StringComparison.OrdinalIgnoreCase)
            || name.StartsWith(filter + ".", StringComparison.OrdinalIgnoreCase);
        IEnumerable<Stage> AllStages() => stages.SelectMany(stage =>
            stage.Cases == null ? new[] { stage } : new[] { stage }.Concat(stage.Cases));
        bool IsSelected(Stage stage) => selected == null || selected.Any(filter => Matches(filter, stage.Name))
            || stage.Cases?.Any(IsSelected) == true;
        if (selected?.Where(filter => !AllStages().Any(stage => Matches(filter, stage.Name))).ToArray() is { Length: > 0 } unknown)
        {
            FrontendSmokeTests.WriteFailure($"Unknown VIEM_TEST_SUITES entries: {string.Join(", ", unknown)}. Stages: {string.Join(", ", AllStages().Select(s => s.Name))}.", timings);
            return false;
        }
        var failures = new List<string>();

        async Task<bool> Execute(Stage stage, bool closeWindows)
        {
            FrontendSmokeTests.WriteProgress(stage.Name);
            int before = FrontendSmokeTests.UiChecks.Count;
            FrontendSmokeTests.UiChecks.Mark();
            var timer = System.Diagnostics.Stopwatch.StartNew();
            bool failed = false;
            try {
                await stage.Run();
            }
            catch (Exception error)
            {
                failed = true;
                string last = FrontendSmokeTests.UiChecks.Count > before ? FrontendSmokeTests.UiChecks[^1] : "(none in this stage)";
                failures.Add($"{stage.Name} failed after its check \"{last}\":\n{error}");
            }
            int stray = closeWindows ? closeStrayWindows() : 0;
            timings.Add(new(stage.Name, Math.Round(timer.Elapsed.TotalSeconds, 2), FrontendSmokeTests.UiChecks.Count - before, failed, stray));
            return !failed;
        }

        foreach (var stage in stages)
        {
            if (!stage.Setup && !IsSelected(stage)) continue;
            if (stage.Cases == null)
            {
                if (!await Execute(stage, closeWindows: true) && stage.Setup) break;
                continue;
            }

            // Case groups own one native fixture. Document/view state is reset
            // between cases, and one failed case does not hide later failures.
            bool ready = await Execute(stage with { Name = stage.Name + ".Setup" }, closeWindows: false);
            try
            {
                if (ready)
                    foreach (var test in stage.Cases.Where(IsSelected))
                        await Execute(test, closeWindows: false);
            }
            finally
            {
                await Execute(new(stage.Name + ".Cleanup", stage.Cleanup ?? (() => Task.CompletedTask)), closeWindows: true);
            }
        }
        if (failures.Count > 0) FrontendSmokeTests.WriteFailure(string.Join("\n\n", failures), timings);
        else FrontendSmokeTests.WriteSuccess(FrontendSmokeTests.UiChecks, timings);
        return failures.Count == 0;
    }
}
#endif

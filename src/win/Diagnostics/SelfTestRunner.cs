#if DEBUG
namespace Viem.Windows.Diagnostics;

internal static class SelfTestRunner
{
    internal sealed record Stage(string Name, Func<Task> Run, bool Setup = false);

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
        if (selected?.Except(stages.Select(s => s.Name), StringComparer.OrdinalIgnoreCase).ToArray() is { Length: > 0 } unknown)
        {
            FrontendSmokeTests.WriteFailure($"Unknown VIEM_TEST_SUITES entries: {string.Join(", ", unknown)}. Stages: {string.Join(", ", stages.Select(s => s.Name))}.", timings);
            return false;
        }
        var failures = new List<string>();
        foreach (var stage in stages)
        {
            if (!stage.Setup && selected != null && !selected.Contains(stage.Name)) continue;
            FrontendSmokeTests.WriteProgress(stage.Name);
            int before = FrontendSmokeTests.UiChecks.Count;
            FrontendSmokeTests.UiChecks.Mark();
            var timer = System.Diagnostics.Stopwatch.StartNew();
            bool failed = false;
            try { await stage.Run(); }
            catch (Exception error)
            {
                failed = true;
                string last = FrontendSmokeTests.UiChecks.Count > before ? FrontendSmokeTests.UiChecks[^1] : "(none in this stage)";
                failures.Add($"{stage.Name} failed after its check \"{last}\":\n{error}");
            }
            int stray = closeStrayWindows();
            timings.Add(new(stage.Name, Math.Round(timer.Elapsed.TotalSeconds, 2), FrontendSmokeTests.UiChecks.Count - before, failed, stray));
            if (failed && stage.Setup) break;
        }
        if (failures.Count > 0) FrontendSmokeTests.WriteFailure(string.Join("\n\n", failures), timings);
        else FrontendSmokeTests.WriteSuccess(FrontendSmokeTests.UiChecks, timings);
        return failures.Count == 0;
    }
}
#endif

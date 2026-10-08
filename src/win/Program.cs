using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Viem.Windows.Diagnostics;
using Viem.Windows.Shell;

namespace Viem.Windows;

// Keep the generated WinUI entry point's initialization order, with trace
// boundaries before Application construction (including its field initializers).
internal static class Program
{
    [STAThread]
    private static void Main(string[] args)
    {
        StartupPerformance.Mark("process.main");
        // Reading/validating preferences uses only portable state and files.
        // Overlap it with native WinUI initialization, before creating controls.
        var preferences = Task.Run(() => new Preferences());
        using (StartupPerformance.Measure("runtime.comWrappers")) WinRT.ComWrappersSupport.InitializeComWrappers();
        // These agile font indices need COM projections but no XAML controls.
        // Start them before Application.Start so native font discovery overlaps
        // framework initialization as well as construction of the first window.
        Rendering.FontCatalog.PrepareFonts();
        StartupPerformance.Mark("application.start");
        Application.Start(parameters => {
            StartupPerformance.Mark("application.callback");
            SynchronizationContext.SetSynchronizationContext(new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread()));
            Preferences initialPreferences;
            using (StartupPerformance.Measure("preferences.join")) initialPreferences = preferences.GetAwaiter().GetResult();
            using (StartupPerformance.Measure("application.construct")) _ = new App(initialPreferences);
        });
    }
}

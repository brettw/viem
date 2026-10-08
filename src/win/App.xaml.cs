using Microsoft.UI.Xaml;
using Viem.Windows.Shell;

namespace Viem.Windows;

public partial class App : Application
{
    internal static App Instance => (App)Current;
    internal List<EditorWindow> Windows { get; } = [];
    internal List<string> Arguments { get; } = [];
    internal Preferences Preferences { get; }
    internal DocumentWindowPlacement WindowPlacement { get; }
    private InstanceBroker? broker;
    internal BlockingEditSessions BlockingEdits { get; } = new();
    internal OpenInvocation LaunchInvocation { get; private set; } = new([], Environment.CurrentDirectory);
    internal App(Preferences preferences)
    {
        using var startup = Diagnostics.StartupPerformance.Measure("app.initialize");
        Preferences = preferences;
        InitializeComponent();
        if (Diagnostics.StartupPerformance.Enabled) UnhandledException += (_, e) => Diagnostics.StartupPerformance.Failed(e.Exception);
        WindowPlacement = new(Preferences);
#if DEBUG
        UnhandledException += (_, e) => { Diagnostics.FrontendSmokeTests.WriteFailure(e.Exception); };
#endif
    }
    protected override async void OnLaunched(LaunchActivatedEventArgs args)
    {
        Diagnostics.StartupPerformance.Mark("app.launched");
        try { LaunchInvocation = OpenInvocation.FromCommandLine(Environment.GetCommandLineArgs().Skip(1).ToArray(), Environment.CurrentDirectory); }
        catch (Exception error) { _ = MessageBox(0, error.Message, "Viem could not open the file", 0x10); Exit(); return; }
        using (Diagnostics.StartupPerformance.Measure("instance.broker")) broker = new InstanceBroker(Preferences.DirectoryPath);
        if (!broker.IsPrimary)
        {
            try { await broker.Redirect(LaunchInvocation); }
            catch (Exception error) { _ = MessageBox(0, error.Message, "Viem could not open the file", 0x10); }
            broker.Dispose(); Exit(); return;
        }
        EditorWindow window;
        using (Diagnostics.StartupPerformance.Measure("window.construct")) window = new EditorWindow(Preferences, openLaunchFiles: true);
        Diagnostics.StartupPerformance.Mark("window.constructed");
        Windows.Add(window);
        window.Closed += (_, _) => Windows.Remove(window);
        using (Diagnostics.StartupPerformance.Measure("window.activate")) window.Activate();
        broker.Listen(window.DispatcherQueue, request => {
            var target = Windows.LastOrDefault(w => w.IsWindowActive) ?? Windows.LastOrDefault();
            target?.ReceiveInvocation(request);
        }, error => Windows.LastOrDefault()?.ActivePane?.Report(error));
    }
    [System.Runtime.InteropServices.DllImport("user32.dll", CharSet = System.Runtime.InteropServices.CharSet.Unicode)] private static extern int MessageBox(nint window, string text, string caption, uint kind);
}

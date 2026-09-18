using Microsoft.UI.Xaml;
using Viem.Windows.Shell;

namespace Viem.Windows;

public partial class App : Application
{
    internal static App Instance => (App)Current;
    internal List<EditorWindow> Windows { get; } = [];
    internal List<string> Arguments { get; } = [];
    internal Preferences Preferences { get; } = new();
    private InstanceBroker? broker;
    public App()
    {
        InitializeComponent();
#if DEBUG
        UnhandledException += (_, e) => { Diagnostics.FrontendSmokeTests.WriteFailure(e.Exception); };
#endif
    }
    protected override async void OnLaunched(LaunchActivatedEventArgs args)
    {
        broker = new InstanceBroker(Preferences.DirectoryPath);
        if (!broker.IsPrimary)
        {
            try { await broker.Redirect(new(Environment.GetCommandLineArgs().Skip(1).ToArray(), Environment.CurrentDirectory)); }
            catch (Exception error) { _ = MessageBox(0, error.Message, "Viem could not open the file", 0x10); }
            broker.Dispose(); Exit(); return;
        }
        var window = new EditorWindow(Preferences);
        Windows.Add(window);
        window.Closed += (_, _) => Windows.Remove(window);
        window.Activate();
        broker.Listen(window.DispatcherQueue, request => {
            var target = Windows.LastOrDefault(w => w.IsWindowActive) ?? Windows.LastOrDefault();
            target?.ReceiveInvocation(request);
        }, error => Windows.LastOrDefault()?.ActivePane?.Report(error));
    }
    [System.Runtime.InteropServices.DllImport("user32.dll", CharSet = System.Runtime.InteropServices.CharSet.Unicode)] private static extern int MessageBox(nint window, string text, string caption, uint kind);
}

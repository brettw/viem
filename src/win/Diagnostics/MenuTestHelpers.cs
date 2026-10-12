#if DEBUG
using Microsoft.UI.Xaml;

namespace Viem.Windows.Diagnostics;

internal static class MenuTestHelpers
{
    internal static async Task Transition(bool loaded, Func<Task> transition, params FrameworkElement[] contents)
    {
        var waits = new List<Task>();
        var subscriptions = new List<(FrameworkElement Content, RoutedEventHandler Changed)>();
        foreach (var content in contents.Distinct().Where(content => content.IsLoaded != loaded)) {
            var changed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            RoutedEventHandler handler = (_, _) => content.DispatcherQueue.TryEnqueue(() => {
                if (content.IsLoaded == loaded) changed.TrySetResult();
            });
            if (loaded) content.Loaded += handler; else content.Unloaded += handler;
            subscriptions.Add((content, handler)); waits.Add(changed.Task);
        }
        try {
            await transition();
            await Task.WhenAll(waits).WaitAsync(TimeSpan.FromSeconds(5));
        }
        finally {
            foreach (var (content, handler) in subscriptions)
                if (loaded) content.Loaded -= handler; else content.Unloaded -= handler;
        }
    }
}
#endif

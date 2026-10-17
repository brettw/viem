namespace Viem.Windows.Shell;

/// <summary>Reports possible disk changes; callers compare contents and decide whether to review them.</summary>
internal sealed class FileChangeMonitor : IDisposable
{
    private static readonly TimeSpan CoalescingDelay = TimeSpan.FromMilliseconds(75);
    private readonly object gate = new();
    private readonly string path;
    private readonly Action changed;
    private readonly System.Threading.Timer timer;
    private List<FileSystemWatcher> watchers = [];
    private bool disposed, scheduled, processing;
    private int consecutiveWatchErrors;

    internal FileChangeMonitor(string path, Action changed)
    {
        this.path = Path.GetFullPath(path);
        this.changed = changed;
        timer = new(_ => Process(), null, Timeout.InfiniteTimeSpan, Timeout.InfiniteTimeSpan);
        // Installation and directory discovery can touch a remote filesystem.
        // An initial observation also covers changes made before watches attach.
        Schedule(immediate: true);
    }

    private void Schedule(bool immediate = false, bool watchError = false)
    {
        lock (gate)
        {
            if (disposed) return;
            // A failing network watch must not create an endless rearm loop.
            // A later real directory event permits another bounded recovery.
            if (watchError && ++consecutiveWatchErrors > 3) return;
            if (!watchError) consecutiveWatchErrors = 0;
            if (scheduled) return;
            scheduled = true;
            if (!processing) timer.Change(immediate ? TimeSpan.Zero : CoalescingDelay, Timeout.InfiniteTimeSpan);
        }
    }

    private void Process()
    {
        lock (gate)
        {
            if (disposed || processing) return;
            processing = true;
            scheduled = false;
        }
        try
        {
            var replacement = CreateWatches();
            List<FileSystemWatcher> retired;
            bool notify;
            lock (gate)
            {
                notify = !disposed;
                retired = notify ? watchers : replacement;
                if (notify) watchers = replacement;
            }
            foreach (var watcher in retired) watcher.Dispose();
            // Callers enqueue onto their serial owner and validate their lifetime.
            // No filesystem operation or callback runs under the monitor lock.
            if (notify) changed();
        }
        finally
        {
            lock (gate)
            {
                processing = false;
                if (!disposed && scheduled) timer.Change(CoalescingDelay, Timeout.InfiniteTimeSpan);
            }
        }
    }

    private List<FileSystemWatcher> CreateWatches()
    {
        var result = new List<FileSystemWatcher>(2);
        string? directory = Path.GetDirectoryName(path);
        while (directory != null && !Directory.Exists(directory)) directory = Path.GetDirectoryName(directory);
        if (directory == null) return result;

        // When the target's parent is missing, watch its nearest existing
        // ancestor for the next path component and descend as it is recreated.
        string relative = Path.GetRelativePath(directory, path);
        int separator = relative.IndexOfAny([Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar]);
        string entry = Path.Combine(directory, separator < 0 ? relative : relative[..separator]);
        AddWatch(result, directory, entry, directoryOnly: false);
        // The containing directory's watch cannot observe its own replacement.
        // Keep a parent watch live before retiring the previous generation.
        if (Path.GetDirectoryName(directory) is string parent)
            AddWatch(result, parent, directory, directoryOnly: true);
        return result;
    }

    private void AddWatch(List<FileSystemWatcher> result, string directory, string entry, bool directoryOnly)
    {
        FileSystemWatcher? watcher = null;
        try
        {
            watcher = new(directory)
            {
                NotifyFilter = directoryOnly ? NotifyFilters.DirectoryName
                    : NotifyFilters.FileName | NotifyFilters.DirectoryName | NotifyFilters.LastWrite
                        | NotifyFilters.Size | NotifyFilters.CreationTime | NotifyFilters.Attributes,
                IncludeSubdirectories = false,
            };
            bool Matches(string candidate) => string.Equals(candidate, entry, StringComparison.OrdinalIgnoreCase);
            void OnChange(object sender, FileSystemEventArgs args) { if (Matches(args.FullPath)) Schedule(); }
            watcher.Created += OnChange;
            watcher.Deleted += OnChange;
            if (!directoryOnly) watcher.Changed += OnChange;
            watcher.Renamed += (_, args) => { if (Matches(args.FullPath) || Matches(args.OldFullPath)) Schedule(); };
            watcher.Error += (_, _) => Schedule(watchError: true);
            watcher.EnableRaisingEvents = true;
            result.Add(watcher);
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or ArgumentException)
        {
            watcher?.Dispose();
            // A concurrently removed or inaccessible directory is recoverable
            // through the surviving ancestor watch or an activation check.
        }
    }

    public void Dispose()
    {
        List<FileSystemWatcher> retired;
        lock (gate)
        {
            if (disposed) return;
            disposed = true;
            scheduled = false;
            retired = watchers;
            watchers = [];
        }
        timer.Dispose();
        foreach (var watcher in retired) watcher.Dispose();
    }
}

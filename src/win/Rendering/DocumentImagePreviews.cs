using Microsoft.Graphics.Canvas;
using Viem.Windows.Shell;

namespace Viem.Windows.Rendering;

/// UI-thread owner of a document's decoded images. Views contribute visibility,
/// but closing or scrolling one view cannot cancel another view's resources.
internal sealed class DocumentImagePreviews : IDisposable
{
    internal const int MaximumEntries = 48;
    internal const long MaximumBytes = 64 * 1024 * 1024;
    private sealed class Entry
    {
        internal LocalImagePreview? Preview;
        internal bool Complete, Loading, Reload;
        internal CancellationTokenSource? Cancellation;
        internal long LastUse;
    }
    private sealed record Knowledge(uint Width, uint Height, InlineImageState State);
    private sealed record Client(DirectWriteProvider Provider, Action<ulong, string[]> GeometryChanged, Action<bool> Repaint)
    {
        internal string[] Wanted = [];
    }
    private readonly Dictionary<string, Entry> entries = new(StringComparer.Ordinal);
    private readonly Dictionary<string, Knowledge> known = new(StringComparer.Ordinal);
    private readonly Dictionary<object, Client> clients = [];
    private CancellationTokenSource lifetime = new();
    private CanvasDevice? device;
    private string? documentPath;
    private long epoch, clock, metadataBytes;
    private bool disposed, refreshing;
    internal long ByteBudget { get; set; } = MaximumBytes;
    internal int LoadAttempts { get; private set; }
    internal int PendingLoads { get; private set; }
    internal int Count => entries.Count;
    internal int CompleteCount => entries.Values.Count(e => e.Complete);
    internal int RasterCount => entries.Values.Count(e => e.Preview != null);
    internal long RasterBytes => entries.Values.Sum(e => e.Preview?.DecodedBytes ?? 0);

    internal LocalImagePreview? Get(string destination) => entries.GetValueOrDefault(destination)?.Preview;

    internal void Seed(DirectWriteProvider provider, CanvasDevice nextDevice, string? path)
    {
        EnsureIdentity(nextDevice, path);
        // At most the raster-cache bound; do not scan document-sized metadata
        // when opening a view. Other known locations are supplied on admission.
        foreach (string destination in entries.Keys)
            if (known.TryGetValue(destination, out var value) && value.State != InlineImageState.Pending)
                provider.RecordImage(destination, value.Width, value.Height, value.State);
    }

    internal void EnsureIdentity(CanvasDevice nextDevice, string? path)
    {
        bool deviceChanged = device != null && !device.Equals(nextDevice);
        device = nextDevice;
        bool pathChanged = SetDocumentPath(path);
        if (deviceChanged && !pathChanged) Reset();
    }

    internal bool SetDocumentPath(string? path)
    {
        if (StringComparer.OrdinalIgnoreCase.Equals(documentPath, path)) return false;
        documentPath = path;
        foreach (var client in clients.Values.ToArray()) client.Provider.SetImageDocumentPath(path);
        Reset(forgetDimensions: true);
        return true;
    }

    internal void Update(object owner, DirectWriteProvider provider, string[] wanted,
        Action<ulong, string[]> geometryChanged, Action<bool> repaint)
    {
        if (disposed) return;
        if (!clients.TryGetValue(owner, out var client) || client.Provider != provider)
            clients[owner] = client = new(provider, geometryChanged, repaint);
        client.Wanted = wanted;
        foreach (string destination in wanted)
        {
            if (known.TryGetValue(destination, out var value))
            {
                // A pending decode cannot replace dimensions already measured
                // by this view's layout worker.
                if (value.State != InlineImageState.Pending) Publish(client, destination, value);
            }
            else if (provider.ImageState(destination) is { } state && state is InlineImageState.Broken or InlineImageState.Limited)
                Remember(destination, new(300, 64, state));
            else if (!ImageLocation.IsWeb(destination) && ImageLocation.LocalPreviewPath(destination, documentPath) == null)
                Publish(client, destination, new(300, 64, InlineImageState.Broken));
        }
        Refresh();
    }

    internal void Remove(object owner) { clients.Remove(owner); Refresh(); }

    internal void Reset(bool forgetDimensions = false)
    {
        epoch++;
        lifetime.Cancel(); lifetime.Dispose(); lifetime = new();
        foreach (var entry in entries.Values) { entry.Preview?.Dispose(); entry.Cancellation?.Dispose(); }
        entries.Clear();
        if (forgetDimensions) { known.Clear(); metadataBytes = 0; }
        foreach (var client in clients.Values.ToArray()) client.Repaint(true);
    }

    internal void Reload(string destination)
    {
        if (disposed || ImageLocation.LocalPreviewPath(destination, documentPath) == null) return;
        if (!entries.TryGetValue(destination, out var entry))
        {
            if (entries.Count >= MaximumEntries)
            {
                var oldest = entries.Where(e => e.Value.Complete).MinBy(e => e.Value.LastUse);
                if (oldest.Key == null) return;
                RemoveEntry(oldest.Key);
            }
            entries[destination] = entry = new();
        }
        entry.Complete = false; entry.Reload = true;
        entry.Cancellation?.Cancel();
        Refresh();
    }

    private void RemoveEntry(string destination)
    {
        if (entries.Remove(destination, out var entry)) { entry.Preview?.Dispose(); entry.Cancellation?.Dispose(); }
    }

    private HashSet<string> Wanted() => clients.Values.SelectMany(c => c.Wanted)
        .Distinct(StringComparer.Ordinal).Take(MaximumEntries).ToHashSet(StringComparer.Ordinal);

    private void Refresh()
    {
        if (disposed || refreshing || device == null) return;
        refreshing = true;
        try
        {
            var wanted = Wanted();
            foreach (var pair in entries.Where(e => e.Value.Loading && !wanted.Contains(e.Key) && !e.Value.Reload).ToArray())
                pair.Value.Cancellation?.Cancel();
            foreach (string destination in wanted.Concat(entries.Where(e => e.Value.Reload).Select(e => e.Key)).Distinct(StringComparer.Ordinal).ToArray())
            {
                if (!entries.TryGetValue(destination, out var entry))
                {
                    if (entries.Count >= MaximumEntries)
                    {
                        var oldest = entries.Where(e => e.Value.Complete && !wanted.Contains(e.Key)).MinBy(e => e.Value.LastUse);
                        if (oldest.Key == null) continue;
                        RemoveEntry(oldest.Key);
                    }
                    entries[destination] = entry = new();
                }
                entry.LastUse = ++clock;
                if (entry.Complete || entry.Loading) continue;
                if (!entry.Reload && known.TryGetValue(destination, out var previous)
                    && previous.State is InlineImageState.Broken or InlineImageState.Limited)
                { entry.Complete = true; continue; }
                if (ImageLocation.LocalPreviewPath(destination, documentPath) == null)
                { entry.Complete = true; continue; }
                if (!Remember(destination, new(300, 64, InlineImageState.Pending)))
                { entry.Complete = true; continue; }
                if (PendingLoads >= 2) continue;
                entry.Loading = true; entry.Reload = false;
                entry.Cancellation = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token);
                PendingLoads++; LoadAttempts++;
                _ = Load(destination, entry, device, documentPath, epoch, entry.Cancellation.Token);
            }
        }
        finally { refreshing = false; }
    }

    private async Task Load(string destination, Entry entry, CanvasDevice loadDevice, string? path, long version, CancellationToken cancellation)
    {
        var result = await LocalImagePreview.Load(loadDevice, destination, path, cancellation);
        var decoded = result.Preview;
        if (disposed || version != epoch || !entries.TryGetValue(destination, out var current) || current != entry)
        { PendingLoads--; decoded?.Dispose(); Refresh(); return; }
        PendingLoads--; entry.Loading = false; entry.Cancellation?.Dispose(); entry.Cancellation = null;
        if (cancellation.IsCancellationRequested)
        {
            decoded?.Dispose();
            if (!entry.Reload) RemoveEntry(destination);
            Refresh(); return;
        }
        entry.Complete = true;
        var value = new Knowledge(decoded?.Width ?? 300, decoded?.Height ?? 64, result.Status switch {
            LocalImagePreviewStatus.Ready => InlineImageState.Ready,
            LocalImagePreviewStatus.Limited => InlineImageState.Limited,
            _ => InlineImageState.Broken });
        known[destination] = value;
        entry.Preview?.Dispose(); entry.Preview = null;
        var wanted = Wanted();
        if (decoded != null && wanted.Contains(destination))
        {
            long used = RasterBytes;
            foreach (var stale in entries.Where(e => e.Value.Complete && !wanted.Contains(e.Key)).OrderBy(e => e.Value.LastUse).ToArray())
            {
                if (used + decoded.DecodedBytes <= ByteBudget) break;
                used -= stale.Value.Preview?.DecodedBytes ?? 0; RemoveEntry(stale.Key);
            }
            if (used + decoded.DecodedBytes <= ByteBudget) entry.Preview = decoded;
            else decoded.Dispose();
        }
        else { decoded?.Dispose(); if (!wanted.Contains(destination)) RemoveEntry(destination); }
        // Publish once, then inform every view. Each provider owns its metrics
        // generation and acknowledges changed geometry independently.
        foreach (var pair in clients.ToArray())
        {
            if (disposed || version != epoch) break;
            if (!clients.TryGetValue(pair.Key, out var client) || !ReferenceEquals(client, pair.Value)) continue;
            Publish(client, destination, value);
            if (clients.TryGetValue(pair.Key, out var currentClient) && ReferenceEquals(currentClient, client))
                client.Repaint(false);
        }
        Refresh();
    }

    private bool Remember(string destination, Knowledge initial)
    {
        if (known.ContainsKey(destination)) return true;
        long cost = destination.Length * 2L + 128;
        if (known.Count >= 65536 || metadataBytes + cost > 32 * 1024 * 1024) return false;
        known[destination] = initial; metadataBytes += cost;
        return true;
    }

    private static void Publish(Client client, string destination, Knowledge value)
    {
        ulong previous = client.Provider.Generation;
        if (!client.Provider.RecordImage(destination, value.Width, value.Height, value.State)) return;
        client.Provider.InvalidateMetrics();
        client.GeometryChanged(previous, [destination]);
    }

    public void Dispose()
    {
        if (disposed) return;
        disposed = true; clients.Clear(); Reset(forgetDimensions: true); lifetime.Dispose();
    }
}

using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

/// <summary>Native and Vim global selection options shared by the document cores in one profile.</summary>
internal static class GlobalSelectionOptions
{
    private static readonly Dictionary<string, Dictionary<uint, string>> profiles = new(StringComparer.OrdinalIgnoreCase);
    private static readonly Dictionary<CoreDocument, string> documents = [];
    private static readonly uint[] names = [VIEM_EX_OPTION_KEYMODEL, VIEM_EX_OPTION_SELECTMODE, VIEM_EX_OPTION_AUTOSELECT];

    internal static void Attach(CoreDocument document, string directory)
    {
        if (documents.ContainsKey(document)) return;
        string profile = Path.TrimEndingDirectorySeparator(Path.GetFullPath(directory));
        if (!profiles.TryGetValue(profile, out var values))
        {
            // The first document has already loaded startup.viem. Later
            // documents inherit live changes instead of replaying old defaults.
            profiles[profile] = values = names.ToDictionary(name => name, document.SelectionOption);
        }
        else foreach (var (name, value) in values) document.SetSelectionOption(name, value);
        documents[document] = profile;
        document.Disposed += () => documents.Remove(document);
    }

    internal static void Observe(CoreDocument source, IReadOnlyDictionary<uint, string> options)
    {
        if (options.Count == 0 || !documents.TryGetValue(source, out string? profile)) return;
        var values = profiles[profile];
        var changed = options.Where(option => values.GetValueOrDefault(option.Key) != option.Value).ToArray();
        if (changed.Length == 0) return;
        foreach (var (name, value) in changed) values[name] = value;
        var targets = documents.Where(entry => entry.Value.Equals(profile, StringComparison.OrdinalIgnoreCase))
            .Select(entry => entry.Key).Where(document => document.Handle != 0).ToArray();
        foreach (var document in targets)
            foreach (var (name, value) in changed) document.SetSelectionOption(name, value);
        // Setters preserve command modes and selections. Repaint their status
        // only after all cores share the same values.
        foreach (var document in targets) document.NotifyChanged();
    }
}

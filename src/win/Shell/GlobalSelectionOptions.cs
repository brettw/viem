using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

/// <summary>Native and Vim global selection options shared by the document cores in one profile.</summary>
internal static class GlobalSelectionOptions
{
    private static readonly Dictionary<string, Dictionary<uint, string>> profiles = new(StringComparer.OrdinalIgnoreCase);
    private static readonly Dictionary<CoreDocument, string> documents = [];
    private static readonly uint[] names = [VIEM_EX_OPTION_KEYMODEL, VIEM_EX_OPTION_SELECTMODE, VIEM_EX_OPTION_AUTOSELECT, VIEM_EX_OPTION_SHIFTSELECT];
    // Settings defaults last distributed per profile. Unrelated preference
    // changes leave live :set overrides in place independently for each option.
    private static readonly Dictionary<string, Dictionary<uint, string>> configured = new(StringComparer.OrdinalIgnoreCase);

    private static string Profile(string directory) => Path.TrimEndingDirectorySeparator(Path.GetFullPath(directory));
    private static IEnumerable<KeyValuePair<uint, string>> Defaults(bool autoselect, bool shiftselect) =>
        new Dictionary<uint, string> {
            [VIEM_EX_OPTION_AUTOSELECT] = autoselect ? "1" : "0",
            [VIEM_EX_OPTION_SHIFTSELECT] = shiftselect ? "1" : "0"
        };

    /// <summary>Applies Settings defaults before startup.viem can override them.</summary>
    internal static void Seed(CoreDocument document, string directory, bool autoselect, bool shiftselect)
    {
        string profile = Profile(directory);
        if (!configured.TryGetValue(profile, out var values))
            configured[profile] = values = Defaults(autoselect, shiftselect).ToDictionary(pair => pair.Key, pair => pair.Value);
        foreach (var (name, value) in values) document.SetSelectionOption(name, value);
    }

    /// <summary>A changed Settings default replaces that live option in every
    /// document of its profile, including one chosen with :set or startup.viem.</summary>
    internal static void Configure(string directory, bool autoselect, bool shiftselect)
    {
        string profile = Profile(directory);
        // Profiles without a seeded document read the preferences when one opens.
        if (!configured.TryGetValue(profile, out var previous)) return;
        var changed = Defaults(autoselect, shiftselect).Where(option => previous[option.Key] != option.Value).ToArray();
        if (changed.Length == 0) return;
        foreach (var (name, value) in changed) previous[name] = value;
        if (!profiles.TryGetValue(profile, out var values)) return;
        changed = changed.Where(option => values[option.Key] != option.Value).ToArray();
        if (changed.Length == 0) return;
        foreach (var (name, value) in changed) values[name] = value;
        var targets = documents.Where(entry => entry.Value.Equals(profile, StringComparison.OrdinalIgnoreCase))
            .Select(entry => entry.Key).Where(document => document.Handle != 0).ToArray();
        foreach (var document in targets)
            foreach (var (name, value) in changed) document.SetSelectionOption(name, value);
        foreach (var document in targets) document.NotifyChanged();
    }

    internal static void Attach(CoreDocument document, string directory)
    {
        if (documents.ContainsKey(document)) return;
        string profile = Profile(directory);
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

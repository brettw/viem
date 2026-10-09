namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
#if DEBUG
    internal Func<string?, Task<string?>>? PickImageFileForTesting { get; set; }
#endif

    internal async Task<string?> PickImageFile(string? directory)
    {
#if DEBUG
        if (PickImageFileForTesting is { } pick) return await pick(directory);
#endif
        var picker = new Microsoft.Windows.Storage.Pickers.FileOpenPicker(AppWindow.Id);
        if (directory != null) picker.SuggestedFolder = directory;
        picker.FileTypeFilter.Add("*");
        return (await picker.PickSingleFileAsync())?.Path;
    }
}

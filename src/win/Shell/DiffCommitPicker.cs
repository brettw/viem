using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private async Task<DiffCommit?> PickDiffCommit(string path, CancellationToken token)
    {
        var commits = await Task.Run(() => DiffSources.Commits(path, null, token), token);
        token.ThrowIfCancellationRequested();
        var input = new TextBox { Header = "Commit hash", FontFamily = new Microsoft.UI.Xaml.Media.FontFamily("Consolas"),
            FontSize = 12, Text = diffBase.Kind == Interop.Native.VIEM_DIFF_BASE_COMMIT ? diffBase.Argument : "" };
        var list = new ListBox { MaxHeight = 280 };
        AutomationProperties.SetName(list, "Recent commits");
        foreach (var commit in commits) list.Items.Add(commit.ToString());
        list.SelectionChanged += (_, _) => {
            if (list.SelectedIndex >= 0) input.Text = commits[list.SelectedIndex].Hash;
        };
        var error = new TextBlock { TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed };
        var content = new StackPanel { Spacing = 8, Width = Math.Min(492, Math.Max(240, root.ActualWidth - 80)) };
        content.Children.Add(input);
        content.Children.Add(list);
        content.Children.Add(error);
        var dialog = new ContentDialog { XamlRoot = root.XamlRoot, RequestedTheme = root.RequestedTheme,
            Title = "Choose a commit", Content = content, PrimaryButtonText = "Compare", CloseButtonText = "Cancel",
            DefaultButton = ContentDialogButton.Primary, IsPrimaryButtonEnabled = input.Text.Length > 0 };
        input.TextChanged += (_, _) => {
            dialog.IsPrimaryButtonEnabled = !string.IsNullOrWhiteSpace(input.Text);
            error.Visibility = Visibility.Collapsed;
            if (list.SelectedIndex >= 0 && commits[list.SelectedIndex].Hash != input.Text) list.SelectedIndex = -1;
        };
        DiffCommit? selected = null;
        dialog.PrimaryButtonClick += async (_, args) => {
            var deferral = args.GetDeferral();
            try {
                string hash = input.Text.Trim();
                selected = commits.FirstOrDefault(c => c.Hash.Equals(hash, StringComparison.OrdinalIgnoreCase))
                    ?? (await Task.Run(() => DiffSources.Commits(path, hash, token), token)).Single();
            } catch (OperationCanceledException) when (token.IsCancellationRequested) {
                selected = null;
            } catch (Exception e) {
                args.Cancel = true;
                error.Text = e.Message;
                error.Visibility = Visibility.Visible;
            } finally { deferral.Complete(); }
        };
        using var cancel = token.Register(() => DispatcherQueue.TryEnqueue(() => dialog.Hide()));
        await DeferExternalFileReviews(async () => { await dialog.ShowAsync(); });
        return selected;
    }
}

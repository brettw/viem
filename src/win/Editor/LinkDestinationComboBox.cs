using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;

namespace Viem.Windows.Editor;

internal sealed partial class LinkDestinationComboBox : ComboBox
{
    protected override void OnGotFocus(RoutedEventArgs args)
    {
        // WinUI delivers GotFocus asynchronously. Its editable ComboBox base
        // can move focus into its TextBox even for an old notification, after
        // the user or the current link draft has focused a different field.
        if (!IsLoaded || XamlRoot is not { } root) return;
        for (var element = FocusManager.GetFocusedElement(root) as DependencyObject;
            element != null; element = VisualTreeHelper.GetParent(element))
        {
            // Dropdown items can live under a separate Popup visual root.
            bool ownedItem = element is ComboBoxItem item
                && ReferenceEquals(ItemsControl.ItemsControlFromItemContainer(item), this);
            if (!ReferenceEquals(element, this) && !ownedItem) continue;
            base.OnGotFocus(args);
            return;
        }
    }
}

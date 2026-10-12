using System.Runtime.InteropServices;
using Microsoft.UI.Input;
using Microsoft.UI.Xaml.Input;
using Windows.Foundation;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    private int pointerClickCount;
    private long pointerClickTime;
    private Point pointerClickPosition;
    private uint pointerClickReturnMode;
    private ulong pointerClickDocument, pointerClickRevision;
    [DllImport("user32.dll")] private static extern uint GetDoubleClickTime();
    [DllImport("user32.dll")] private static extern int GetSystemMetrics(int index);

    private int TrackPointerClick(PointerRoutedEventArgs args, Point point)
    {
        if (args.Pointer.PointerDeviceType != PointerDeviceType.Mouse) { pointerClickCount = 0; return 1; }
        long now = Environment.TickCount64;
        double scale = Canvas.XamlRoot?.RasterizationScale ?? 1;
        var current = View!.Presentation;
        bool continues = pointerClickCount > 0 && current.document_id == pointerClickDocument
            && current.document_revision == pointerClickRevision
            && now - pointerClickTime <= GetDoubleClickTime()
            && Math.Abs(point.X - pointerClickPosition.X) * scale <= Math.Max(1, GetSystemMetrics(36) / 2d)
            && Math.Abs(point.Y - pointerClickPosition.Y) * scale <= Math.Max(1, GetSystemMetrics(37) / 2d);
        if (continues) pointerClickCount = Math.Min(3, pointerClickCount + 1);
        else {
            pointerClickCount = 1;
            pointerClickReturnMode = current.mode;
            pointerClickPosition = point;
            pointerClickDocument = current.document_id;
            pointerClickRevision = current.document_revision;
        }
        pointerClickTime = now;
        return pointerClickCount;
    }
}

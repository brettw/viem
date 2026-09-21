#if DEBUG
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Interop;
using Viem.Windows.Shell;
using Windows.UI;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Diagnostics;

internal static class StyleInspectorBehaviorTests
{
    private static void Check(bool condition, string message)
    {
        if (!condition) throw new InvalidOperationException(message);
        FrontendSmokeTests.UiChecks.Add(message);
        File.AppendAllText(FrontendSmokeTests.ReportPath + ".styles.log", message + Environment.NewLine);
    }
    private static StyleDefinition Selected(StyleWindow inspector) => (StyleDefinition)inspector.StylePicker.SelectedItem;
    private static void Move(CoreView view, ulong offset, bool extend = false)
        => view.Place(offset, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, view.Document.State.document_revision, extend);
    private static IEnumerable<T> Children<T>(DependencyObject root) where T : DependencyObject
    {
        if (root is T match) yield return match;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(root); i++)
            foreach (var child in Children<T>(VisualTreeHelper.GetChild(root, i))) yield return child;
    }
    private static async Task Open(Flyout flyout, FrameworkElement target)
    {
        var opened = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        void Complete(object? sender, object args) => opened.TrySetResult();
        flyout.Opened += Complete;
        try { flyout.ShowAt(target); await opened.Task.WaitAsync(TimeSpan.FromSeconds(5)); }
        finally { flyout.Opened -= Complete; }
    }
    private static async Task Close(Flyout flyout, Action? dismiss = null)
    {
        var closed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        void Complete(object? sender, object args) => closed.TrySetResult();
        flyout.Closed += Complete;
        try { if (dismiss != null) dismiss(); else flyout.Hide(); await closed.Task.WaitAsync(TimeSpan.FromSeconds(5)); }
        finally { flyout.Closed -= Complete; }
    }
    private static unsafe void IncludeStyles(CoreView view)
    {
        // Generated HTML defaults are presentation-only until style export is
        // enabled. Exercise persisted colors and clean/dirty state explicitly.
        view.Apply(outcome => {
            var state = view.Document.State;
            var request = New<ViemSetIncludeStyleDefinitionsV1>();
            request.enabled = 1; request.document_id = state.document_id; request.document_revision = state.document_revision;
            return viem_core_view_set_include_style_definitions(view.Document.Handle, view.Id, &request, outcome);
        });
        view.Document.MarkSaved(view.Document.State);
    }
    internal static async Task Run(EditorPane pane, Preferences preferences)
    {
        await Following(pane, preferences);
        await Colors(pane, preferences);
        await CodeColors(preferences);
    }
    private static async Task Following(EditorPane pane, Preferences preferences)
    {
        using var document = new CoreDocument("# Title `code` tail\n\nBody"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN);
        using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        Move(view, 7);
        var inspector = new StyleWindow(view, preferences);
        var preparedSize = inspector.AppWindow.ClientSize;
        var shownSizes = new List<global::Windows.Graphics.SizeInt32>();
        inspector.AppWindow.Changed += (_, change) => { if (change.DidSizeChange && inspector.AppWindow.IsVisible) shownSizes.Add(inspector.AppWindow.ClientSize); };
        Check(!inspector.AppWindow.IsVisible && inspector.RootControl.DesiredSize.Height > 500, "style inspector measures populated controls before its first visible frame");
        inspector.Activate();
        try
        {
            await Task.Delay(200);
            Check(inspector.AppWindow.ClientSize == preparedSize && shownSizes.All(size => size == preparedSize), "opening the inspector does not resize through intermediate visible layouts");
            Check(Math.Abs(inspector.RootControl.XamlRoot.Size.Height - inspector.RootControl.ActualHeight) < 2,
                $"initial caret-style inspector fits its rendered content (desired={inspector.RootControl.DesiredSize.Height}, actual={inspector.RootControl.ActualHeight}, client={inspector.RootControl.XamlRoot.Size.Height}, scale={inspector.RootControl.XamlRoot.RasterizationScale})");
            Check(Selected(inspector).Id == "Code", "Styles opens the current named character style immediately");
            Check(!inspector.CaretFollowScheduled, "opening Styles does not schedule an idle timer");
            byte[] source = document.Source(document.State.document_revision);
            int queries = inspector.CaretStyleQueries, loads = inspector.StyleLoads;
            Move(view, 0);
            for (int i = 0; i < 30; i++) { Move(view, (ulong)(1 + i % 18), true); await Task.Delay(40); }
            Move(view, 1);
            Check(inspector.CaretStyleQueries == queries && inspector.StyleLoads == loads && Selected(inspector).Id == "Code",
                "rapid caret/selection dragging performs no style queries or dialog reloads");
            await Task.Delay(250);
            Check(inspector.CaretStyleQueries == queries && inspector.CaretFollowScheduled, "caret following waits for half a second of inactivity");
            await Task.Delay(450);
            Check(Selected(inspector).Id == "Heading1" && inspector.CaretStyleQueries == queries + 1 && !inspector.CaretFollowScheduled,
                "a drag burst produces one settled paragraph-style update");
            queries = inspector.CaretStyleQueries;
            inspector.StylePicker.SelectedItem = ((StyleDefinition[])inspector.StylePicker.ItemsSource).Single(s => s.Id == "Heading2");
            view.Refresh(); view.Zoom(1.1f); document.NotifyChanged(); Move(view, 1);
            await Task.Delay(1100);
            Check(Selected(inspector).Id == "Heading2" && inspector.CaretStyleQueries == queries && !inspector.CaretFollowScheduled,
                "unchanged caret, repaint, layout and document notifications retain a manual style without scheduling");
            inspector.Retarget(view);
            Check(Selected(inspector).Id == "Heading1" && !inspector.CaretFollowScheduled, "reopening the same inspector immediately reselects the caret style");
            Move(view, 7);
            inspector.StylePicker.SelectedItem = ((StyleDefinition[])inspector.StylePicker.ItemsSource).Single(s => s.Id == "Heading2");
            await Task.Delay(1100);
            Check(Selected(inspector).Id == "Heading2" && !inspector.CaretFollowScheduled, "an explicit picker choice cancels a pending caret follow");
            Move(view, 0); Move(view, 10, true); await Task.Delay(1100);
            Check(Selected(inspector).Id == "Heading1", "mixed character styles follow their uniform paragraph");
            Move(view, 19, true); await Task.Delay(1100);
            Check((Selected(inspector).Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0, "mixed paragraphs and characters fall back to Base Paragraph");
            using var other = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
            Move(other, 7); await Task.Delay(1100);
            Check(!inspector.CaretFollowScheduled && (Selected(inspector).Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0,
                "another view of the same document does not drive caret following");
            Check(source.AsSpan().SequenceEqual(document.Source(document.State.document_revision)) && !document.IsDirty,
                "following styles preserves document bytes and clean state");
            Move(view, 7); inspector.Close(); await Task.Delay(1100);
            Check(!inspector.CaretFollowScheduled, "closing the style inspector cancels pending follow work");
        }
        finally { inspector.Close(); }
        using var code = new CoreDocument("fn main() {}\n"u8.ToArray(), "tracking.rs");
        using var codeView = new CoreView(code, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        for (int i = 0; i < 300; i++)
        {
            code.PollSyntax(); codeView.Resize(700, 400);
            if (codeView.CurrentStyleEditorKey(codeView.Styles()) is { Namespace: 2 }) break;
            await Task.Delay(10);
        }
        var codeInspector = new StyleWindow(codeView, preferences); codeInspector.Activate();
        try
        {
            await Task.Delay(200);
            Check(Math.Abs(codeInspector.RootControl.XamlRoot.Size.Height - codeInspector.RootControl.ActualHeight) < 2,
                $"initial Code inspector fits its rendered content (desired={codeInspector.RootControl.DesiredSize.Height}, actual={codeInspector.RootControl.ActualHeight}, client={codeInspector.RootControl.XamlRoot.Size.Height}, scale={codeInspector.RootControl.XamlRoot.RasterizationScale})");
            Check(Selected(codeInspector).Namespace == 2 && codeInspector.Title == "Code Styles", "Code Styles opens the retained syntax style under the caret");
            Move(codeView, 2); await Task.Delay(1100);
            Check((Selected(codeInspector).Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0, "Code whitespace follows Base Paragraph within the global sheet");
        }
        finally { codeInspector.Close(); }
        var standalone = new StyleWindow(codeView, preferences, followCaret: false);
        try { Move(codeView, 0); Check(!standalone.CaretFollowScheduled && standalone.CaretStyleQueries == 0, "standalone global Code inspectors never follow a document caret"); }
        finally { standalone.Close(); }
    }
    private static async Task ColorDragging(StyleWindow inspector, ColorPicker picker)
    {
        int loads = inspector.StyleLoads;
        var spectrum = Children<Microsoft.UI.Xaml.Controls.Primitives.ColorSpectrum>(picker).Single();
        var sweep = Enumerable.Range(0, 81).Select(i => new global::Windows.Foundation.Point(.1 + i * .01, .1 + i * .008))
            .Concat(Enumerable.Repeat(new global::Windows.Foundation.Point(.9, .74), 50))
            .Concat(Enumerable.Range(0, 81).Select(i => new global::Windows.Foundation.Point(.9 - i * .01, .74 - i * .008))).ToArray();
        var colors = new List<System.Numerics.Vector4>();
        var layoutPositions = new List<global::Windows.Foundation.Rect>();
        var peer = Microsoft.UI.Xaml.Automation.Peers.FrameworkElementAutomationPeer.CreatePeerForElement(spectrum);
        await InputRoutingTests.Drag(inspector, spectrum, sweep, i => { colors.Add(spectrum.HsvColor); layoutPositions.Add(peer.GetBoundingRectangle()); });
        Check(colors.Select((color, i) => Math.Abs(color.X - sweep[i].X * 359) < 12 && Math.Abs(color.Y - (1 - sweep[i].Y)) < .04).All(v => v),
            "native spectrum drag tracks pointer coordinates through fast diagonal moves, pauses and reversals");
        Check(colors.Skip(90).Take(40).All(c => c == colors[90]), "the color marker stays still while the pointer pauses during a drag");
        Check(layoutPositions.All(r => r == layoutPositions[0]) && inspector.StyleLoads == loads,
            "live color dragging keeps the popup geometry and inspector controls stable");
    }
    private static bool HasColor(CoreView target, Color color)
    {
        var layout = target.Layout();
        bool Matches(ViemRgbaV1 c) => Math.Abs(c.red - color.R / 255f) < .001 && Math.Abs(c.green - color.G / 255f) < .001
            && Math.Abs(c.blue - color.B / 255f) < .001 && Math.Abs(c.alpha - color.A / 255f) < .001;
        return Matches(layout.Paint.default_paint.foreground) || layout.PaintRuns.Any(r => Matches(r.paint.foreground));
    }
    private static async Task CodeColors(Preferences preferences)
    {
        using var document = new CoreDocument("First Code sample."u8.ToArray(), format: VIEM_FORMAT_CODE);
        using var second = new CoreDocument("Second Code sample."u8.ToArray(), format: VIEM_FORMAT_CODE);
        var editor = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(editor); editor.Activate();
        var view = await editor.ActivePane!.Ready;
        var other = await editor.AddPane(second).Ready;
        byte[] original = view.ExportStyleDefaults();
        string file = Path.Combine(preferences.DirectoryPath, "code_style.json");
        byte[]? saved = File.Exists(file) ? File.ReadAllBytes(file) : null;
        var inspector = new StyleWindow(view, preferences, followCaret: false); inspector.Activate(); await Task.Delay(200);
        try
        {
            var button = Children<Button>(inspector.RootControl).Single(b => AutomationProperties.GetName(b) == "Text Color");
            var flyout = (Flyout)button.Flyout; var picker = (ColorPicker)flyout.Content;
            await Open(flyout, button);
            picker.Color = Microsoft.UI.Colors.Red; await Task.Delay(150);
            bool firstColor = HasColor(view, Microsoft.UI.Colors.Red), otherColor = HasColor(other, Microsoft.UI.Colors.Red);
            bool persisted = File.Exists(file) && File.ReadAllBytes(file).AsSpan().SequenceEqual(view.ExportStyleDefaults());
            Check(inspector.Error.Length == 0 && firstColor && otherColor && persisted && !document.IsDirty && !second.IsDirty,
                $"live Code color changes repaint separate documents and persist shared styles without dirtying source (first={firstColor}, other={otherColor}, saved={persisted}, dirty={document.IsDirty}/{second.IsDirty}, error={inspector.Error})");
            byte[] red = view.ExportStyleDefaults();
            using (var locked = new FileStream(file, FileMode.Open, FileAccess.Read, FileShare.None))
            {
                picker.Color = Microsoft.UI.Colors.Green; await Task.Delay(150);
                Check(inspector.Error.Length > 0 && red.AsSpan().SequenceEqual(view.ExportStyleDefaults()) && picker.Color == Microsoft.UI.Colors.Red
                    && inspector.PreviewForeground == Microsoft.UI.Colors.Red && !inspector.ColorUpdateScheduled,
                    "a rejected live Code color restores committed style, picker and preview without rescheduling");
            }
            await Close(flyout);
        }
        finally
        {
            inspector.Close(); view.ReplaceCodeStyles(original);
            if (saved != null) Preferences.AtomicWrite(file, saved); else File.Delete(file);
            editor.Close(); App.Instance.Windows.Remove(editor);
        }
    }
    private static async Task Colors(EditorPane pane, Preferences preferences)
    {
        using var document = new CoreDocument("<p>Color sample.</p>"u8.ToArray(), format: VIEM_FORMAT_HTML);
        var editor = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(editor); editor.Activate();
        var renderedPane = editor.ActivePane!;
        var view = await renderedPane.Ready;
        var mirrorPane = editor.AddPane(document);
        var mirror = await mirrorPane.Ready;
        IncludeStyles(view);
        var inspector = new StyleWindow(view, preferences); inspector.Activate(); await Task.Delay(200);
        byte[] Pixels(EditorPane target)
        {
            using var surface = new CanvasRenderTarget(target.Canvas.Device, (float)target.Canvas.ActualWidth, (float)target.Canvas.ActualHeight, target.Canvas.Dpi);
            using (var drawing = surface.CreateDrawingSession()) target.Draw(drawing);
            return surface.GetPixelBytes();
        }
        try
        {
            inspector.StylePicker.SelectedItem = ((StyleDefinition[])inspector.StylePicker.ItemsSource).Single(s => (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0);
            var button = Children<Button>(inspector.RootControl).Single(b => AutomationProperties.GetName(b) == "Text Color");
            var flyout = (Flyout)button.Flyout;
            var picker = (ColorPicker)flyout.Content;
            var original = document.Source(document.State.document_revision);
            flyout.Placement = Microsoft.UI.Xaml.Controls.Primitives.FlyoutPlacementMode.Right;
            int initialLoads = inspector.StyleLoads;
            Move(view, 1); document.NotifyChanged();
            await Open(flyout, button);
            Check(picker.Orientation == Orientation.Horizontal && picker.ActualHeight <= 280 && picker.ActualWidth <= 600,
                $"color picker uses a compact horizontal layout ({picker.ActualWidth} x {picker.ActualHeight})");
            Check(!flyout.IsConstrainedToRootBounds, "color flyout uses native hosting outside the style window bounds");
            DependencyObject? popupParent = picker;
            while (popupParent != null && popupParent is not FlyoutPresenter) popupParent = VisualTreeHelper.GetParent(popupParent);
            Check(popupParent is FlyoutPresenter presenter && presenter.ActualWidth >= picker.ActualWidth + 24,
                "the flyout presenter fits the whole horizontal picker without clipping or horizontal scrolling");
            Check(Children<TextBox>(picker).Where(t => t.ActualHeight > 0).All(t => t.ActualHeight <= 28)
                && Children<ComboBox>(picker).Any(c => c.ActualHeight is > 0 and <= 28), "the color picker inherits compact text and combo control sizing");
            // Opened precedes the first compositor frame of the popup HWND.
            await Task.Delay(200);
            Check(await WindowCapture.SavePopup(WinRT.Interop.WindowNative.GetWindowHandle(inspector), pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".color-picker.png"),
                "the native color popup can visibly extend beyond the style dialog");
            Check(picker.Color == preferences.Theme.Foreground && ((SolidColorBrush)((Border)button.Content).Background).Color == preferences.Theme.Foreground,
                "default text color well and popup use the theme foreground instead of emergency black");
            await Close(flyout);
            Check(inspector.StyleLoads == initialLoads + 1, "color picking preserves a deferred document refresh until popup closure");
            Check(original.AsSpan().SequenceEqual(document.Source(document.State.document_revision)) && !document.IsDirty, "opening and dismissing an unchanged picker adds no edit");
            Check(!inspector.ColorUpdateScheduled, "an unchanged color popup schedules no work");
            flyout.Placement = Microsoft.UI.Xaml.Controls.Primitives.FlyoutPlacementMode.Top;
            Move(view, 2);
            Check(inspector.CaretFollowScheduled, "caret motion schedules following before a color edit");
            await Open(flyout, button);
            Check(!inspector.CaretFollowScheduled, "opening a color picker cancels pending caret following");
            int loads = inspector.StyleLoads;
            byte[] firstPixels = Pixels(renderedPane), mirrorPixels = Pixels(mirrorPane);
            int firstBuilds = renderedPane.DrawingCacheBuilds, mirrorBuilds = mirrorPane.DrawingCacheBuilds;
            if (Environment.GetEnvironmentVariable("VIEM_TEST_POINTER_INPUT") == "1")
                await ColorDragging(inspector, picker);
            ulong revision = document.State.document_revision;
            for (int i = 0; i < 100; i++) picker.Color = Color.FromArgb(255, (byte)i, 64, 128);
            var custom = Color.FromArgb(102, 31, 64, 128); picker.Color = custom;
            Check(inspector.ColorUpdateScheduled && document.State.document_revision == revision, "rapid color input queues one coalesced edit");
            await Task.Delay(150);
            Check(inspector.PreviewForeground == custom && ((SolidColorBrush)((Border)button.Content).Background).Color == custom
                && inspector.StyleLoads == loads && document.IsDirty && !original.AsSpan().SequenceEqual(document.Source(document.State.document_revision)),
                $"picker changes update the committed document, preview and swatch without dialog reloads (error={inspector.Error})");
            Check(document.State.document_revision == revision + 1 && !inspector.ColorUpdateScheduled, "one color burst commits once and leaves no idle timer");
            Check(HasColor(view, custom) && HasColor(mirror, custom)
                && !firstPixels.AsSpan().SequenceEqual(Pixels(renderedPane)) && !mirrorPixels.AsSpan().SequenceEqual(Pixels(mirrorPane))
                && renderedPane.DrawingCacheBuilds > firstBuilds && mirrorPane.DrawingCacheBuilds > mirrorBuilds,
                "both visible document views invalidate cached drawing and render the live color before popup dismissal");
            document.NotifyChanged();
            Check(picker.Color == custom && inspector.StyleLoads == loads, "a no-op document notification leaves the active picker stable");
            picker.Color = Microsoft.UI.Colors.Blue; await Task.Delay(100);
            Check(inspector.PreviewForeground == Microsoft.UI.Colors.Blue && document.State.document_revision > revision + 1,
                "a sustained popup gesture commits subsequent colors before dismissal");
            picker.Color = custom; await Task.Delay(100);
            await WindowCapture.Save(WinRT.Interop.WindowNative.GetWindowHandle(inspector), pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".color-preview.png");
            await Close(flyout);
            Check(inspector.Error.Length == 0 && document.IsDirty && picker.Color == custom,
                $"closing the live color picker retains custom RGBA (style={Selected(inspector).Id}, enabled={button.IsEnabled}, dirty={document.IsDirty}, color={picker.Color}, error={inspector.Error})");
            byte[] colored = document.Source(document.State.document_revision);
            await Open(flyout, button); await Close(flyout);
            Check(colored.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "reopening a custom color has no round-trip edit");
            view.Undo();
            Check(original.AsSpan().SequenceEqual(document.Source(document.State.document_revision)) && !document.IsDirty, "one undo restores every live color change in the popup session");
            var backgroundButton = Children<Button>(inspector.RootControl).Single(b => AutomationProperties.GetName(b) == "Background Color");
            var backgroundFlyout = (Flyout)backgroundButton.Flyout;
            var backgroundPicker = (ColorPicker)backgroundFlyout.Content;
            await Open(backgroundFlyout, backgroundButton);
            Check(backgroundPicker.Color.A == 0, "an absent text background opens as transparent");
            var background = Color.FromArgb(128, 192, 64, 32); backgroundPicker.Color = background;
            await Task.Delay(100);
            Check(inspector.PreviewBackground == background && document.IsDirty, "background color and alpha update the document and preview live");
            await Close(backgroundFlyout);
            Check(inspector.Error.Length == 0 && backgroundPicker.Color == background, "background picker commits and retains transparency");
            view.Undo();
            Check(original.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "background color is a single undoable edit");
            using var second = new CoreDocument("<p>Second color target.</p>"u8.ToArray(), format: VIEM_FORMAT_HTML);
            using var other = new CoreView(second, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
            IncludeStyles(other);
            inspector.Retarget(other);
            inspector.StylePicker.SelectedItem = ((StyleDefinition[])inspector.StylePicker.ItemsSource).Single(s => (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0);
            await Open(flyout, button); picker.Color = Microsoft.UI.Colors.Red; await Close(flyout);
            Check(second.IsDirty && !document.IsDirty, "a retargeted inspector edits the new view rather than its constructor's view");
            colored = second.Source(second.State.document_revision);
            await Open(flyout, button); picker.Color = Microsoft.UI.Colors.Green;
            await Close(flyout, () => inspector.Retarget(view));
            Check(!colored.AsSpan().SequenceEqual(second.Source(second.State.document_revision)) && !document.IsDirty && !inspector.ColorUpdateScheduled,
                "retargeting flushes pending color to its original target and cancels the timer");
            other.Undo();
            Check(colored.AsSpan().SequenceEqual(second.Source(second.State.document_revision)), "retargeting closes the original target's undo group");
            Check(inspector.PreviewForeground == preferences.Theme.Foreground, "retargeting shows the new document's committed color");
            await Open(flyout, button); picker.Color = Microsoft.UI.Colors.Red; await Task.Delay(100);
            picker.Color = Microsoft.UI.Colors.Green;
            await Close(flyout, view.Undo);
            await Task.Delay(100);
            Check(!inspector.ColorUpdateScheduled && !document.IsDirty && original.AsSpan().SequenceEqual(document.Source(document.State.document_revision)),
                "undo during an open picker cancels queued colors without replaying them");
            await Open(flyout, button); picker.Color = custom;
            await Close(flyout, inspector.Close);
            Check(document.IsDirty && !inspector.ColorUpdateScheduled, "closing the inspector flushes the final color and cancels pending work");
            view.Undo();
            Check(!document.IsDirty && original.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "closing the inspector ends its live undo group");
        }
        finally { inspector.Close(); document.MarkSaved(document.State); editor.Close(); App.Instance.Windows.Remove(editor); }
    }
}
#endif

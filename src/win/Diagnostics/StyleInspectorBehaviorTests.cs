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
    private static Color SwatchColor(Button button)
    {
        var colorLayer = ((Grid)button.Content).Children.OfType<Border>()
            .Single(child => child.Background is SolidColorBrush);
        return ((SolidColorBrush)colorLayer.Background).Color;
    }
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

    internal static async Task Run(EditorPane pane, Preferences preferences)
    {
        BlockPreview(pane, preferences);
        if (Environment.GetEnvironmentVariable("VIEM_TEST_STYLE_PERFORMANCE_ONLY") == "1") {
            await Following(pane, preferences);
            await CacheRegressions(pane, preferences);
            return;
        }
        await LinkedBlockControls(pane, preferences);
        await CodeBlockBackground(pane, preferences);
        DocumentPicker(pane, preferences);
        await Following(pane, preferences);
        await Colors(pane, preferences);
        await CodeColors(preferences);
    }
    private static void DocumentPicker(EditorPane pane, Preferences preferences)
    {
        uint[] formats = [VIEM_FORMAT_PLAIN_TEXT, VIEM_FORMAT_MARKDOWN, VIEM_FORMAT_CODE];
        byte[][] original = formats.Select(preferences.ThemeStyleDefaults).ToArray();
        byte[] source = "# Heading\n\nBody"u8.ToArray();
        using var document = new CoreDocument(source, format: VIEM_FORMAT_MARKDOWN_SOURCE);
        preferences.AttachThemeDocument(document);
        using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        var inspector = new StyleWindow(view, preferences);
        float Size() => inspector.ThemeView.Styles().Styles.Single(s => (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0)
            .Value(VIEM_STYLE_PROPERTY_CHARACTER_SIZE).number;
        try {
            Check(inspector.DocumentPicker.Items.Cast<string>().SequenceEqual(new[] { "Plain Text", "Markdown", "Code" })
                && inspector.DocumentPicker.SelectedIndex == 1, "Document picker groups Markdown views and uses Plain Text, Markdown, Code order");
            var originalSizes = new float[formats.Length];
            for (int index = 0; index < formats.Length; index++) {
                byte[][] before = formats.Select(preferences.ThemeStyleDefaults).ToArray();
                inspector.DocumentPicker.SelectedIndex = index;
                if (formats[index] == VIEM_FORMAT_PLAIN_TEXT)
                    Check(inspector.StylePicker.Items.Count == 1 && inspector.StylePicker.Items[0] is StyleDefinition { Id: "Paragraph" },
                        "the single Plain Text paragraph section contains its style without a redundant heading");
                Check(formats.Select((format, i) => before[i].AsSpan().SequenceEqual(preferences.ThemeStyleDefaults(format))).All(same => same),
                    "selecting a Document stylesheet makes no theme edit");
                originalSizes[index] = Size();
                Check(inspector.EditPropertyForTesting(VIEM_STYLE_PROPERTY_CHARACTER_SIZE, 32 + index) && Size() == 32 + index,
                    "Document picker targets editable theme definitions even without an open document of that format");
                Check(formats.Select((format, i) => i == index || before[i].AsSpan().SequenceEqual(preferences.ThemeStyleDefaults(format))).All(same => same),
                    "style edits affect only the chosen Document family");
                Move(view, 1); document.NotifyChanged(); inspector.FollowActiveView(view);
                Check(inspector.DocumentPicker.SelectedIndex == index && !inspector.CaretFollowScheduled,
                    "explicit Document selection remains independent of caret following");
            }
            for (int index = formats.Length - 1; index >= 0; index--) {
                inspector.DocumentPicker.SelectedIndex = index;
                Check(Size() == 32 + index, "returning to a Document stylesheet retains edits");
                inspector.UndoThemeForTesting();
                Check(Size() == originalSizes[index], "returning to a Document stylesheet retains its independent Undo");
                inspector.RedoThemeForTesting();
                Check(Size() == 32 + index, "Document stylesheet Redo restores the edit");
            }
            Check(document.State.format == VIEM_FORMAT_MARKDOWN_SOURCE && !document.IsDirty
                && source.AsSpan().SequenceEqual(document.Source(document.State.document_revision)),
                "Document stylesheet selection and editing preserve the open source and format");
            inspector.Retarget(view);
            Check(inspector.DocumentPicker.SelectedIndex == 1, "reopening Edit Styles resumes the active document family");
        }
        finally {
            inspector.Close();
            for (int index = 0; index < formats.Length; index++) preferences.SaveThemeStyles(formats[index], original[index]);
        }
    }

    private static async Task LinkedBlockControls(EditorPane pane, Preferences preferences)
    {
        byte[] original = preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN);
        using var document = new CoreDocument("> Block controls"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN);
        preferences.AttachThemeDocument(document);
        using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        var inspector = new StyleWindow(view, preferences, followCaret: false);
        inspector.Activate(); await Task.Delay(150);
        uint[] margins = [VIEM_STYLE_PROPERTY_BLOCK_MARGIN_LEFT, VIEM_STYLE_PROPERTY_BLOCK_MARGIN_RIGHT, VIEM_STYLE_PROPERTY_BLOCK_MARGIN_TOP, VIEM_STYLE_PROPERTY_BLOCK_MARGIN_BOTTOM];
        uint[] colors = [VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_COLOR, VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_COLOR, VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_COLOR, VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_COLOR];
        try {
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == "Block quote");
            var selected = Selected(inspector);
            for (int i = 0; i < margins.Length; i++) inspector.ThemeView.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, margins[i], CoreView.Number(new float[] { 0, -3, 8, 4 }[i]));
            foreach (uint property in colors) inspector.ThemeView.EditStyle(selected, VIEM_STYLE_EDIT_CLEAR_DECLARATION, property, default);
            var foreground = CoreView.Enum(VIEM_STYLE_VALUE_COLOR, 0);
            foreground.color = new() { red = .2f, green = .4f, blue = .6f, alpha = 1 };
            inspector.ThemeView.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND, foreground);
            inspector.RefreshForTesting();
            var tab = Children<Microsoft.UI.Xaml.Controls.Primitives.ToggleButton>(inspector.RootControl).Single(b => Equals(b.Content, "Block"));
            tab.IsChecked = true; inspector.RootControl.UpdateLayout();
            var toggle = Children<Microsoft.UI.Xaml.Controls.Primitives.ToggleButton>(inspector.RootControl).Single(b => AutomationProperties.GetName(b) == "Link block sides");
            toggle.IsChecked = true;
            Check(margins.All(p => Selected(inspector).Value(p).number == -3), "Block lock copies the first nonzero value in left, right, top, bottom order");
            Check(colors.All(p => !Selected(inspector).Declares(p)), "Block lock preserves unspecified border colors");
            var button = Children<Button>(inspector.RootControl).Single(b => AutomationProperties.GetName(b) == "Bottom border color");
            Check(SwatchColor(button) == Color.FromArgb(255, 51, 102, 153), "Unspecified border swatches follow the style text color");
            var bottom = Children<NumberBox>(inspector.RootControl).Single(b => AutomationProperties.GetName(b) == "Bottom margin");
            bottom.Value = 3;
            Check(margins.All(p => Selected(inspector).Value(p).number == 3), "A native bottom margin edit updates all linked margins");
            inspector.UndoThemeForTesting();
            Check(margins.All(p => Selected(inspector).Value(p).number == -3), "One theme Undo restores every linked side");
            inspector.RedoThemeForTesting();
            Check(margins.All(p => Selected(inspector).Value(p).number == 3), "Theme Redo reapplies every linked side");
            toggle.IsChecked = false;
            Check(margins.All(p => Selected(inspector).Value(p).number == 3), "Unlocking preserves side values");
            bottom.Value = 7;
            Check(Selected(inspector).Value(margins[3]).number == 7 && Selected(inspector).Value(margins[0]).number == 3,
                "An unlocked native field changes only its side");
            Check(!document.IsDirty, "Linked block controls leave source clean");
        }
        finally { inspector.Close(); preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, original); }
    }

    private static unsafe void BlockPreview(EditorPane pane, Preferences preferences)
    {
        byte[] source = System.Text.Encoding.UTF8.GetBytes("> Preview source stays unchanged.");
        using var document = new CoreDocument(source, format: VIEM_FORMAT_MARKDOWN);
        preferences.AttachThemeDocument(document);
        using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        var quote = view.Styles().Styles.Single(s => s.Id == "Block quote" && s.Namespace == 1);
        view.EditStyle(quote, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_BLOCK_PADDING_LEFT, CoreView.Number(17));
        var background = New<ViemStyleEditValueV1>(); background.kind = VIEM_STYLE_VALUE_COLOR;
        background.color = new() { red = .6f, green = .2f, blue = .7f, alpha = .5f };
        view.EditStyle(quote, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_BLOCK_BACKGROUND, background);
        var sheet = view.Styles(); quote = sheet.Styles.Single(s => s.Key == quote.Key);
        using var preview = new BlockStylePreview(VIEM_STYLE_ROLE_QUOTE, pane.Canvas.Device, pane.DispatcherQueue);
        preview.Update(sheet, quote, preferences.Theme.Foreground);
        var layout = preview.Layout(560, 300);
        var boxes = layout.Decorations.Where(d => (d.flags & VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND) != 0).ToArray();
        Check(boxes.Length >= 2 && boxes.Select(box => box.typographic_bounds.x).Distinct().Count() == 2,
            "Block preview uses core nested-container geometry");
        Check(boxes.All(box => Math.Abs(box.paint.foreground.alpha - .5f) < .001f),
            "Block preview copies committed background opacity");
        int edits = preview.PropertyEdits;
        double fullMs = 0, incrementalMs = 0;
        for (int i = 0; i < 8; i++) {
            view.EditStyle(quote, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_BLOCK_PADDING_LEFT, CoreView.Number(20 + i));
            sheet = view.Styles(); quote = sheet.Styles.Single(s => s.Key == quote.Key);
            using var fresh = new BlockStylePreview(VIEM_STYLE_ROLE_QUOTE, pane.Canvas.Device, pane.DispatcherQueue);
            long start = System.Diagnostics.Stopwatch.GetTimestamp();
            fresh.Update(sheet, quote, preferences.Theme.Foreground);
            fullMs += System.Diagnostics.Stopwatch.GetElapsedTime(start).TotalMilliseconds;
            start = System.Diagnostics.Stopwatch.GetTimestamp();
            preview.Update(sheet, quote, preferences.Theme.Foreground);
            incrementalMs += System.Diagnostics.Stopwatch.GetElapsedTime(start).TotalMilliseconds;
            var actual = preview.Layout(560, 300);
            var expected = fresh.Layout(560, 300);
            Check(actual.Decorations.Select(d => d.typographic_bounds).SequenceEqual(expected.Decorations.Select(d => d.typographic_bounds)),
                "Incremental block preview matches fresh native geometry after padding edit " + i);
        }
        Check(preview.PropertyEdits - edits == 8, "Eight single-property preview changes submit exactly eight property edits");
        Check(true, $"Preview property update benchmark: full {fullMs / 8:F2} ms, incremental {incrementalMs / 8:F2} ms, speedup {fullMs / Math.Max(.001, incrementalMs):F2}x");
        edits = preview.PropertyEdits;
        // A change to another style yields a new string arena, but should not
        // submit unchanged effective values to this specimen.
        var heading = sheet.Styles.Single(s => s.Id == "Heading1" && s.Namespace == 1);
        view.EditStyle(heading, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_SIZE, CoreView.Number(31));
        sheet = view.Styles(); quote = sheet.Styles.Single(s => s.Key == quote.Key);
        preview.Update(sheet, quote, preferences.Theme.Foreground);
        Check(preview.PropertyEdits == edits, "An unrelated stylesheet revision does not resubmit unchanged preview values");
        view.EditStyleFont(quote, ["Consolas"], null);
        sheet = view.Styles(); quote = sheet.Styles.Single(s => s.Key == quote.Key);
        var foreground = Color.FromArgb(255, 36, 72, 108);
        preview.Update(sheet, quote, foreground);
        using var freshPixels = new BlockStylePreview(VIEM_STYLE_ROLE_QUOTE, pane.Canvas.Device, pane.DispatcherQueue);
        freshPixels.Update(sheet, quote, foreground);
        byte[] Pixels(BlockStylePreview specimen) {
            using var surface = new CanvasRenderTarget(pane.Canvas.Device, 560, 300, 96);
            using (var drawing = surface.CreateDrawingSession()) {
                drawing.Clear(Microsoft.UI.Colors.White);
                specimen.Draw(drawing, 560, 300, foreground);
            }
            return surface.GetPixelBytes();
        }
        Check(Pixels(preview).SequenceEqual(Pixels(freshPixels)),
            "Incremental preview pixels match a fresh specimen after font and theme foreground changes");
        preview.InvalidateFontsForTesting();
        Check(Pixels(preview).SequenceEqual(Pixels(freshPixels)),
            "Font invalidation at unchanged preview dimensions rebuilds usable matching pixels");
        int exports = view.StyleExports;
        for (int i = 0; i < 40; i++) view.Styles();
        Check(view.StyleExports == exports, "Unchanged stylesheet reads perform no full exports");
        foreach (uint property in new[] { VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_COLOR, VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_COLOR,
            VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_COLOR, VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_COLOR, VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND })
            view.EditStyle(quote, VIEM_STYLE_EDIT_CLEAR_DECLARATION, property, default);
        var baseStyle = view.Styles().Styles.Single(s => (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0);
        view.EditStyle(baseStyle, VIEM_STYLE_EDIT_CLEAR_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND, default);
        view.EditStyle(quote, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_WIDTH, CoreView.Number(2));
        sheet = view.Styles(); quote = sheet.Styles.Single(s => s.Key == quote.Key);
        foreach (var current in new[] { foreground, Color.FromArgb(255, 120, 60, 30) }) {
            foreground = current;
            preview.Update(sheet, quote, foreground);
            var borders = preview.Layout(560, 300).Decorations.Where(d => (d.flags & (VIEM_LAYOUT_DECORATION_BLOCK_BORDER | VIEM_LAYOUT_DECORATION_BLOCK_QUOTE_BORDER)) != 0).ToArray();
            Check(borders.Length > 0 && borders.All(d => Math.Abs(d.paint.foreground.red - foreground.R / 255f) < .001f
                && Math.Abs(d.paint.foreground.green - foreground.G / 255f) < .001f && Math.Abs(d.paint.foreground.blue - foreground.B / 255f) < .001f),
                "Unspecified preview borders follow the current theme text color");
            using var fresh = new BlockStylePreview(VIEM_STYLE_ROLE_QUOTE, pane.Canvas.Device, pane.DispatcherQueue);
            fresh.Update(sheet, quote, foreground);
            Check(Pixels(preview).SequenceEqual(Pixels(fresh)), "Current text border fallback matches fresh preview pixels after a theme color change");
        }
        Check(document.Source(document.State.document_revision).SequenceEqual(source),
            "Block preview leaves the inspected document source unchanged");
    }
    private static async Task CacheRegressions(EditorPane pane, Preferences preferences)
    {
        byte[] original = preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN);
        using var document = new CoreDocument("A paragraph with `code`."u8.ToArray(), format: VIEM_FORMAT_MARKDOWN);
        preferences.AttachThemeDocument(document);
        using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        var inspector = new StyleWindow(view, preferences, followCaret: false);
        inspector.Activate(); await Task.Delay(150);
        try {
            var styleMenu = inspector.StylePicker.ItemsSource;
            var parents = inspector.ParentPicker.ItemsSource;
            var following = inspector.NextPicker.ItemsSource;
            var faces = inspector.FontVariantControl.ItemsSource;
            inspector.RefreshForTesting();
            Check(ReferenceEquals(parents, inspector.ParentPicker.ItemsSource), "Unchanged inspector refresh retains parent menu entries");
            Check(inspector.EditPropertyForTesting(VIEM_STYLE_PROPERTY_CHARACTER_SIZE, 19), "Inspector commits a size edit");
            Check(ReferenceEquals(styleMenu, inspector.StylePicker.ItemsSource)
                && ReferenceEquals(parents, inspector.ParentPicker.ItemsSource)
                && ReferenceEquals(following, inspector.NextPicker.ItemsSource)
                && ReferenceEquals(faces, inspector.FontVariantControl.ItemsSource)
                && Selected(inspector).Id == "Paragraph", "A property edit retains all unchanged menus and the selected style");
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == "Code" && s.Namespace == 2);
            inspector.ParentPicker.SelectedItem = inspector.ParentPicker.Items.OfType<StyleDefinition>().First(s => s.Id == "Comment");
            Check(inspector.Error.Length == 0, "Character style accepts a named parent");
            inspector.ParentPicker.SelectedItem = inspector.ParentPicker.Items[0];
            Check(inspector.Error.Length == 0 && inspector.ParentPicker.SelectedItem is string,
                "Default Paragraph clears a named character parent without an invalid relationship");
        }
        finally { inspector.Close(); preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, original); }
    }
    private static async Task CodeBlockBackground(EditorPane pane, Preferences preferences)
    {
        byte[] source = "```\nCode block\n```"u8.ToArray();
        using var document = new CoreDocument(source, format: VIEM_FORMAT_MARKDOWN);
        preferences.AttachThemeDocument(document);
        using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        var inspector = new StyleWindow(view, preferences, followCaret: false);
        inspector.Activate(); await Task.Delay(150);
        try
        {
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == "Code Block");
            var tab = Children<Microsoft.UI.Xaml.Controls.Primitives.ToggleButton>(inspector.RootControl).Single(b => b.Content as string == "Block");
            tab.Focus(FocusState.Programmatic); await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
            var checkbox = Children<CheckBox>(inspector.RootControl).Single(c => AutomationProperties.GetName(c) == "Declare Background color");
            checkbox.Focus(FocusState.Programmatic); await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space);
            var button = Children<Button>(inspector.RootControl).Single(b => AutomationProperties.GetName(b) == "Background color");
            var flyout = (Flyout)button.Flyout; var picker = (ColorPicker)flyout.Content;
            await Open(flyout, button);
            Check(picker.IsAlphaEnabled && picker.Color.A == 255, "A new Code Block background starts opaque in the picker");
            foreach (byte alpha in new byte[] { 255, 128, 0 })
            {
                var chosen = Color.FromArgb(alpha, 255, 0, 0);
                picker.Color = chosen; await Task.Delay(100);
                await Close(flyout);
                var fills = view.Layout().Decorations.Where(d => (d.flags & VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND) != 0).ToArray();
                Check(inspector.Error.Length == 0 && fills.Length > 0 && fills.All(d => Math.Abs(d.paint.foreground.alpha - alpha / 255f) < .001f),
                    $"Code Block background commits and exports opacity {alpha}");
                await Open(flyout, button);
                Check(picker.Color == chosen, "Reopening the Block background picker preserves explicit transparency");
            }
            await Close(flyout);
            Check(document.Source(document.State.document_revision).SequenceEqual(source), "Block color edits preserve Markdown source");
        }
        finally { inspector.Close(); }
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
            Check(Selected(inspector).Id == "Code" && inspector.Title == "Theme styles — Markdown — " + preferences.ThemeDisplayName, "Markdown Styles opens the current named character style immediately");
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
            var catalogue = inspector.StylePicker.ItemsSource;
            var families = inspector.FontFamilyControl.ItemsSource;
            var faces = inspector.FontVariantControl.ItemsSource;
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == "Heading3");
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == "Heading2");
            Check(ReferenceEquals(catalogue, inspector.StylePicker.ItemsSource)
                && ReferenceEquals(families, inspector.FontFamilyControl.ItemsSource)
                && ReferenceEquals(faces, inspector.FontVariantControl.ItemsSource),
                "Following styles with the same font retains catalogue, family and variant menu resources");
            queries = inspector.CaretStyleQueries;
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == "Heading2");
            view.Refresh(); view.Zoom(1.1f); document.NotifyChanged(); Move(view, 1);
            await Task.Delay(1100);
            Check(Selected(inspector).Id == "Heading2" && inspector.CaretStyleQueries == queries && !inspector.CaretFollowScheduled,
                "unchanged caret, repaint, layout and document notifications retain a manual style without scheduling");
            inspector.Retarget(view);
            Check(Selected(inspector).Id == "Heading1" && !inspector.CaretFollowScheduled, "reopening the same inspector immediately reselects the caret style");
            Move(view, 7);
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == "Heading2");
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
            Check(Selected(codeInspector).Namespace == 2 && codeInspector.Title == "Theme styles — Code — " + preferences.ThemeDisplayName, "Code Styles opens the retained syntax style under the caret");
            Move(codeView, 2); await Task.Delay(1100);
            Check((Selected(codeInspector).Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0, "Code whitespace follows Base Paragraph within the global sheet");
        }
        finally { codeInspector.Close(); }
        var standalone = new StyleWindow(codeView, preferences, followCaret: false);
        try { Move(codeView, 0); Check(!standalone.CaretFollowScheduled && standalone.CaretStyleQueries == 0, "standalone global Code inspectors never follow a document caret"); }
        finally { standalone.Close(); }
        await MarkdownCharacterFollowing(pane, preferences);
        await MarkdownCodeFollowing(pane, preferences);
    }
    private static async Task MarkdownCharacterFollowing(EditorPane pane, Preferences preferences)
    {
        byte[] source = "plain [link](other.md) ~~strike~~ `code` <!--comment-->\n\n[label][ref]\n\n[ref]: other.md"u8.ToArray();
        foreach (uint format in new[] { VIEM_FORMAT_MARKDOWN, VIEM_FORMAT_MARKDOWN_SOURCE })
        {
            byte[] originalStyles = preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN);
            using var document = new CoreDocument(source, "following.md", format);
            using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
            var inspector = new StyleWindow(view, preferences);
            try
            {
                foreach (var (text, style) in new[] { ("link", "Link"), ("strike", "Strikethrough"), ("code", "Code"),
                    ("comment", "Comment"), ("label", "Markdown reference") })
                {
                    int index = document.FormattedText().IndexOf(text, StringComparison.Ordinal) + 1;
                    Move(view, (ulong)index);
                    Check(view.CurrentStyleEditorKey(view.Styles()) == new StyleKey(2, style),
                        "Markdown caret exposes its automatic character style to the inspector: " + style + " / " + format);
                    inspector.Retarget(view);
                    Check(Selected(inspector).Key == new StyleKey(2, style),
                        "reopening Markdown styles selects the current character treatment: " + style + " / " + format);
                }
                Move(view, 1); inspector.Retarget(view);
                Move(view, (ulong)document.FormattedText().IndexOf("link", StringComparison.Ordinal) + 1);
                await Task.Delay(650);
                Check(Selected(inspector).Key == new StyleKey(2, "Link"),
                    "moving onto a link updates the open style inspector after the caret settles: " + format);
                ulong linkStart = view.LinkContext().Link!.Start;
                Check(linkStart == (ulong)document.FormattedText().IndexOf(format == VIEM_FORMAT_MARKDOWN_SOURCE ? "[link]" : "link", StringComparison.Ordinal),
                    "the mode-change fixture uses the complete projected Link span boundary: " + format);
                view.Place(linkStart, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
                inspector.Retarget(view);
                Check(Selected(inspector).Key == new StyleKey(2, "Link"), "the Normal projected link boundary follows its current character");
                inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>()
                    .Single(style => (style.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0);
                var beforeInsert = view.Presentation;
                view.Command("i");
                Check(view.Presentation.cursor_utf8_offset == beforeInsert.cursor_utf8_offset
                    && view.Presentation.cursor_affinity == beforeInsert.cursor_affinity && inspector.CaretFollowScheduled,
                    "a collapsed-caret mode change schedules style following without position or affinity changes: " + format);
                await Task.Delay(650);
                Check(Selected(inspector).Key == new StyleKey(2, "Link"),
                    "Insert mode resumes caret-style following after a manual inspector choice without changing position: " + format);
                view.Key(VIEM_KEY_ESCAPE);
                Move(view, (ulong)document.FormattedText().IndexOf("link", StringComparison.Ordinal) + 1);
                inspector.Retarget(view);
                view.Command("i");
                ulong caret = view.Presentation.cursor_utf8_offset;
                view.ToggleLink(view.LinkContext());
                Check(view.Presentation.cursor_utf8_offset == caret && inspector.CaretFollowScheduled,
                    "pending link clearing schedules the inspector without caret movement: " + format);
                await Task.Delay(650);
                Check((Selected(inspector).Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0,
                    "the stationary Insert caret follows pending unlinked appearance: " + format);
                view.ChooseStyle(new(2, "Code"), view.SelectedNamedStyles().Identity);
                await Task.Delay(650);
                Check(view.Presentation.cursor_utf8_offset == caret && Selected(inspector).Key == new StyleKey(2, "Code"),
                    "the stationary Insert caret follows pending Code appearance: " + format);
                view.ChooseStyle(new(2, ""), view.SelectedNamedStyles().Identity);
                await Task.Delay(650);
                Check((Selected(inspector).Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0,
                    "clearing pending Code updates the inspector at the same caret: " + format);
                view.Key(VIEM_KEY_ESCAPE);
                Move(view, (ulong)document.FormattedText().IndexOf("strike", StringComparison.Ordinal) + 1);
                view.Command("i"); inspector.Retarget(view);
                Check(Selected(inspector).Key == new StyleKey(2, "Strikethrough"), "the inspector opens the strike treatment before a pending clear");
                caret = view.Presentation.cursor_utf8_offset;
                view.ToggleStrikethrough();
                await Task.Delay(650);
                Check(view.Presentation.cursor_utf8_offset == caret && (Selected(inspector).Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0,
                    "clearing pending strikethrough updates the inspector without movement: " + format);
                inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(style => style.Key == new StyleKey(2, "Link"));
                Check(inspector.EditPropertyForTesting(VIEM_STYLE_PROPERTY_CHARACTER_SIZE, 27), "the manually selected link definition accepts an explicit size edit");
                await Task.Delay(650);
                Check(Selected(inspector).Key == new StyleKey(2, "Link") && !inspector.CaretFollowScheduled,
                    "editing a manually selected definition preserves the inspector selection: " + format);
                Check(source.AsSpan().SequenceEqual(document.Source(document.State.document_revision)) && !document.IsDirty,
                    "following automatic Markdown character styles preserves source bytes and clean state");
            }
            finally { inspector.Close(); preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, originalStyles); }
        }
    }
    private static async Task MarkdownCodeFollowing(EditorPane pane, Preferences preferences)
    {
        foreach (uint format in new[] { VIEM_FORMAT_MARKDOWN, VIEM_FORMAT_MARKDOWN_SOURCE })
        foreach (bool explicitChoice in new[] { false, true })
        {
            byte[] source = "# Heading\n\n**words**\n"u8.ToArray();
            using var document = new CoreDocument(source, "tracking.md", format: format);
            using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
            var inspector = new StyleWindow(view, preferences);
            try
            {
                view.SetDocumentMode(VIEM_DOCUMENT_MODE_CODE, "markdown", false, DocumentModes.Read(document));
                Check(inspector.DocumentPicker.SelectedIndex == 2, "a Markdown-to-Code menu transition retargets the open inspector");
                StyleKey? manual = null;
                if (explicitChoice)
                {
                    var choice = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Name == "Comment");
                    inspector.StylePicker.SelectedItem = choice;
                    manual = choice.Key;
                }
                for (int i = 0; i < 300; i++)
                {
                    document.PollSyntax(); view.Refresh();
                    if (view.CurrentStyleEditorKey(view.Styles()) is { Namespace: 2 }) break;
                    await Task.Delay(10);
                }
                var heading = view.CurrentStyleEditorKey(view.Styles());
                Check(heading is { Namespace: 2 }, "Markdown Code syntax publishes the stationary caret's heading capture");
                await Task.Delay(650);
                Check(Selected(inspector).Key == (manual ?? heading),
                    "delayed syntax completes mode-switch following while preserving an intervening explicit style choice");
                Move(view, 4); await Task.Delay(650);
                Check(Selected(inspector).Key == heading, "caret following continues after a Markdown-to-Code transition");
                Check(source.AsSpan().SequenceEqual(document.Source(document.State.document_revision)) && !document.IsDirty,
                    "mode-switch style following preserves source and clean state");
            }
            finally { inspector.Close(); }
        }
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
        string file = preferences.SelectedThemePath ?? throw new InvalidOperationException("Persistence diagnostics require a named theme.");
        var inspector = new StyleWindow(view, preferences, followCaret: false); inspector.Activate(); await Task.Delay(200);
        try
        {
            var button = Children<Button>(inspector.RootControl).Single(b => AutomationProperties.GetName(b) == "Text Color");
            var flyout = (Flyout)button.Flyout; var picker = (ColorPicker)flyout.Content;
            await Open(flyout, button);
            picker.Color = Microsoft.UI.Colors.Red; await Task.Delay(150);
            bool firstColor = HasColor(view, Microsoft.UI.Colors.Red), otherColor = HasColor(other, Microsoft.UI.Colors.Red);
            bool persisted = File.Exists(file) && System.Text.Json.Nodes.JsonNode.DeepEquals(System.Text.Json.Nodes.JsonNode.Parse(File.ReadAllBytes(file))!["styles"]!["code"], System.Text.Json.Nodes.JsonNode.Parse(view.ExportStyleDefaults()));
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
            inspector.UndoThemeForTesting();
            Check(original.AsSpan().SequenceEqual(view.ExportStyleDefaults()) && !document.IsDirty && !second.IsDirty,
                "theme Undo restores an entire Code color gesture independently of source history");
            inspector.RedoThemeForTesting();
            Check(HasColor(view, Microsoft.UI.Colors.Red) && HasColor(other, Microsoft.UI.Colors.Red),
                "theme Redo reapplies Code color across all documents");
        }
        finally
        {
            inspector.Close(); preferences.SaveThemeStyles(VIEM_FORMAT_CODE, original);
            editor.Close(); App.Instance.Windows.Remove(editor);
        }
    }
    private static async Task Colors(EditorPane pane, Preferences preferences)
    {
        using var document = new CoreDocument("Color sample."u8.ToArray(), format: VIEM_FORMAT_PLAIN_TEXT);
        byte[] originalTheme = preferences.ThemeStyleDefaults(VIEM_FORMAT_PLAIN_TEXT);
        var editor = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(editor); editor.Activate();
        var renderedPane = editor.ActivePane!;
        var view = await renderedPane.Ready;
        var mirrorPane = editor.AddPane(document);
        var mirror = await mirrorPane.Ready;
        var inspector = new StyleWindow(view, preferences); inspector.Activate(); await Task.Delay(200);
        byte[] Pixels(EditorPane target)
        {
            using var surface = new CanvasRenderTarget(target.Canvas.Device, (float)target.Canvas.ActualWidth, (float)target.Canvas.ActualHeight, target.Canvas.Dpi);
            using (var drawing = surface.CreateDrawingSession()) target.Draw(drawing);
            return surface.GetPixelBytes();
        }
        try
        {
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0);
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
            Check(picker.Color == preferences.Theme.Foreground && SwatchColor(button) == preferences.Theme.Foreground,
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
            var custom = Color.FromArgb(255, 31, 64, 128); picker.Color = custom;
            Check(inspector.ColorUpdateScheduled && document.State.document_revision == revision, "rapid color input queues one coalesced edit");
            await Task.Delay(150);
            Check(inspector.PreviewForeground == custom && SwatchColor(button) == custom
                && inspector.StyleLoads == loads && !document.IsDirty && original.AsSpan().SequenceEqual(document.Source(document.State.document_revision)),
                $"picker changes update the theme, preview and swatch without changing source without dialog reloads (error={inspector.Error})");
            Check(document.State.document_revision == revision && !inspector.ColorUpdateScheduled, "one color burst commits without advancing source revision and leaves no idle timer");
            Check(HasColor(view, custom) && HasColor(mirror, custom)
                && !firstPixels.AsSpan().SequenceEqual(Pixels(renderedPane)) && !mirrorPixels.AsSpan().SequenceEqual(Pixels(mirrorPane))
                && renderedPane.DrawingCacheBuilds > firstBuilds && mirrorPane.DrawingCacheBuilds > mirrorBuilds,
                "both visible document views invalidate cached drawing and render the live color before popup dismissal");
            document.NotifyChanged();
            Check(picker.Color == custom && inspector.StyleLoads == loads, "a no-op document notification leaves the active picker stable");
            picker.Color = Microsoft.UI.Colors.Blue; await Task.Delay(100);
            Check(inspector.PreviewForeground == Microsoft.UI.Colors.Blue && document.State.document_revision == revision,
                "a sustained popup gesture commits subsequent colors before dismissal");
            picker.Color = custom; await Task.Delay(100);
            await WindowCapture.Save(WinRT.Interop.WindowNative.GetWindowHandle(inspector), pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".color-preview.png");
            await Close(flyout);
            Check(inspector.Error.Length == 0 && !document.IsDirty && picker.Color == custom,
                $"closing the live color picker retains custom RGBA (style={Selected(inspector).Id}, enabled={button.IsEnabled}, dirty={document.IsDirty}, color={picker.Color}, error={inspector.Error})");
            byte[] colored = document.Source(document.State.document_revision);
            await Open(flyout, button); await Close(flyout);
            Check(colored.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "reopening a custom color has no round-trip edit");
            Check(original.AsSpan().SequenceEqual(document.Source(document.State.document_revision)) && !document.IsDirty, "theme color changes do not create source undo records");
            inspector.UndoThemeForTesting();
            Check(inspector.PreviewForeground == preferences.Theme.Foreground && !document.IsDirty,
                "one theme Undo restores all coalesced colors in the popup gesture");
            inspector.RedoThemeForTesting();
            Check(inspector.PreviewForeground == custom && HasColor(mirror, custom),
                "theme Redo restores the gesture and refreshes sibling views");
            var backgroundButton = Children<Button>(inspector.RootControl).Single(b => AutomationProperties.GetName(b) == "Background Color");
            var backgroundFlyout = (Flyout)backgroundButton.Flyout;
            var backgroundPicker = (ColorPicker)backgroundFlyout.Content;
            await Open(backgroundFlyout, backgroundButton);
            Check(backgroundPicker.Color.A == 0, "an absent text background opens as transparent");
            var background = Color.FromArgb(255, 192, 64, 32); backgroundPicker.Color = background;
            await Task.Delay(100);
            Check(inspector.PreviewBackground == background && !document.IsDirty, "background color and alpha update the document and preview live");
            await Close(backgroundFlyout);
            Check(inspector.Error.Length == 0 && backgroundPicker.Color == background, "background picker commits and retains transparency");
            Check(original.AsSpan().SequenceEqual(document.Source(document.State.document_revision)), "theme background changes preserve source bytes");
            using var second = new CoreDocument("Second color target."u8.ToArray(), format: VIEM_FORMAT_PLAIN_TEXT);
            preferences.AttachThemeDocument(second);
            using var other = new CoreView(second, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
            inspector.Retarget(other);
            Check(inspector.Title == "Theme styles — Plain Text — " + preferences.ThemeDisplayName, "retargeting identifies the Plain Text stylesheet");
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0);
            await Open(flyout, button); picker.Color = Microsoft.UI.Colors.Red; await Close(flyout);
            Check(!second.IsDirty && !document.IsDirty && HasColor(other, Microsoft.UI.Colors.Red) && HasColor(view, Microsoft.UI.Colors.Red),
                "a retargeted inspector edits the shared theme across documents without dirtying either source");
            await Open(flyout, button); picker.Color = Microsoft.UI.Colors.Green;
            await Close(flyout, () => inspector.Retarget(view));
            Check(HasColor(other, Microsoft.UI.Colors.Green) && HasColor(view, Microsoft.UI.Colors.Green) && !inspector.ColorUpdateScheduled,
                "retargeting flushes pending theme color once and cancels the timer");
            await Open(flyout, button); picker.Color = custom;
            await Close(flyout, inspector.Close);
            Check(HasColor(view, custom) && !document.IsDirty && !inspector.ColorUpdateScheduled,
                "closing the inspector flushes the final theme color without a source edit");
        }
        finally { inspector.Close(); preferences.SaveThemeStyles(VIEM_FORMAT_PLAIN_TEXT, originalTheme); editor.Close(); App.Instance.Windows.Remove(editor); }
    }
}
#endif

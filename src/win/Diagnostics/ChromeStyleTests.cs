#if DEBUG
using System.Text;
using System.Text.Json.Nodes;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Text;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Interop;
using Viem.Windows.Shell;
using Viem.Windows.Rendering;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class ChromeStyleTests
{
    private static void Check(bool condition, string name)
    { if (!condition) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }
    private static JsonNode Definition(JsonNode sheet, string id) => sheet["block_styles"]!.AsArray().Single(entry => entry!["id"]!.GetValue<string>() == id)!;
    private static byte[] Bytes(JsonNode node) => Encoding.UTF8.GetBytes(node.ToJsonString());
    private static byte[] Rows(int count) => Encoding.UTF8.GetBytes(string.Join("\n", Enumerable.Repeat("A physical source line", count)));

    private static void CheckGutterPixels(CanvasDevice device)
    {
        // CanvasRenderTarget rounds its requested dimensions to the nearest
        // pixel. Allocate through the outer fractional pixel so these fixtures
        // exercise the gutter painter's complete canvas-edge coverage.
        CanvasRenderTarget Target(double width, double height, float dpi) => new(device,
            (float)(Math.Ceiling(width * dpi / 96) * 96 / dpi),
            (float)(Math.Ceiling(height * dpi / 96) * 96 / dpi), dpi);
        var normal = ChromeStyle.Fallback(Microsoft.UI.Colors.Black, Microsoft.UI.Colors.White) with {
            Background = global::Windows.UI.Color.FromArgb(128, 0, 0, 0), Margin = default,
            Padding = default, Border = new(0, 0, 1, 0), RightBorder = Microsoft.UI.Colors.Red
        };
        var current = normal with { Background = global::Windows.UI.Color.FromArgb(128, 0, 255, 0), RightBorder = Microsoft.UI.Colors.Blue };
        foreach (float dpi in new[] { 96f, 120f, 144f, 192f }) {
            foreach (float zoom in new[] { .75f, 1f, 1.25f, 1.5f }) {
                foreach (float scroll in new[] { .125f, .375f, .625f }) {
                    void Require(bool condition, string name) {
                        if (!condition) throw new InvalidOperationException($"{name}: DPI {dpi}, zoom {zoom}, scroll {scroll}");
                    }
                    double width = 31 * zoom, height = 60.25 * zoom;
                    var regions = Enumerable.Range(0, 7).Select(index => new ViemLineNumberRegionV1 {
                        source_line = (ulong)index + 1, top = index * 10.125f * zoom, bottom = (index + 1) * 10.125f * zoom,
                        flags = index == 2 ? VIEM_LINE_NUMBER_REGION_CURRENT : 0
                    }).ToArray();
                    using var target = Target(width, height, dpi);
                    using (var drawing = target.CreateDrawingSession()) {
                        var priorAntialiasing = drawing.Antialiasing;
                        EditorPane.DrawLineNumberBoxesForTesting(drawing, width, height, scroll, regions, normal.Scaled(zoom), current.Scaled(zoom));
                        Require(drawing.Antialiasing == priorAntialiasing, "gutter box painting restores antialiasing");
                    }
                    int pixelsWide = (int)target.SizeInPixels.Width, pixelsHigh = (int)target.SizeInPixels.Height;
                    byte[] background = target.GetPixelBytes(1, 0, 1, pixelsHigh);
                    byte[] border = target.GetPixelBytes(pixelsWide - 1, 0, 1, pixelsHigh);
                    int badBackground = Enumerable.Range(0, pixelsHigh).FirstOrDefault(y => background[y * 4 + 3] != 128, -1);
                    int badBorder = Enumerable.Range(0, pixelsHigh).FirstOrDefault(y => border[y * 4 + 3] != 255
                        || border[y * 4] != 255 && border[y * 4 + 2] != 255, -1);
                    Require(badBackground < 0 && badBorder < 0,
                        $"adjacent backgrounds paint once and the right border covers every pixel (surface={target.Size}, pixels={pixelsWide}x{pixelsHigh}, background row={badBackground}, border row={badBorder}, border={(badBorder < 0 ? "ok" : Convert.ToHexString(border.AsSpan(badBorder * 4, 4)))})");
                    int currentY = (int)((25.3125 * zoom - scroll) * dpi / 96);
                    byte[] currentBackground = target.GetPixelBytes(1, currentY, 1, 1);
                    byte[] currentBorder = target.GetPixelBytes(pixelsWide - 1, currentY, 1, 1);
                    Require(currentBackground[1] > 0 && currentBackground[3] == 128 && currentBorder[0] == 255,
                        "current region flags select the current background and border overrides");

                    using var gapTarget = Target(width, height, dpi);
                    using (var drawing = gapTarget.CreateDrawingSession())
                        EditorPane.DrawLineNumberBoxesForTesting(drawing, width, height, scroll, regions.Take(5).ToArray(), normal.Scaled(zoom), current.Scaled(zoom));
                    byte[] gapBackground = gapTarget.GetPixelBytes(1, 0, 1, pixelsHigh);
                    byte[] gapBorder = gapTarget.GetPixelBytes(pixelsWide - 1, 0, 1, pixelsHigh);
                    Require(Enumerable.Range(0, pixelsHigh).All(y => gapBackground[y * 4 + 3] == 128 && gapBorder[y * 4 + 3] == 255),
                        "the column gap after the last line covers its final canvas pixel without alpha seams");

                    var transparent = current with { Background = Microsoft.UI.Colors.Transparent, Margin = new(2.125f, .375f, 3.125f, .375f) };
                    using var transparentTarget = Target(width, height, dpi);
                    using (var drawing = transparentTarget.CreateDrawingSession())
                        EditorPane.DrawLineNumberBoxesForTesting(drawing, width, height, scroll, regions, normal.Scaled(zoom), transparent.Scaled(zoom));
                    int currentX = (int)(10 * zoom * dpi / 96);
                    byte[] empty = transparentTarget.GetPixelBytes(currentX, currentY, 1, 1);
                    byte[] outer = transparentTarget.GetPixelBytes(0, currentY, 1, 1);
                    Require(empty[3] == 0 && outer[3] == 128,
                        "transparent current boxes and explicit margins survive pixel-aligned gap subtraction");
                }
            }
        }
        Check(true, "gutter raster diagnostics cover continuous right borders, single background compositing, current overrides, transparency, and margins at 96/120/144/192 DPI, fractional scroll, and zoom");
        foreach (float dpi in new[] { 96f, 120f, 144f, 192f }) {
            var edges = normal with {
                Background = Microsoft.UI.Colors.Transparent, Border = new(2.125f, 2.125f, 2.125f, 2.125f),
                LeftBorder = global::Windows.UI.Color.FromArgb(128, 255, 0, 0), RightBorder = global::Windows.UI.Color.FromArgb(128, 255, 0, 0),
                TopBorder = global::Windows.UI.Color.FromArgb(128, 0, 0, 255), BottomBorder = global::Windows.UI.Color.FromArgb(128, 0, 0, 255)
            };
            foreach (float boxHeight in new[] { 24f, 1.25f }) {
                using var target = Target(24, boxHeight, dpi);
                using (var drawing = target.CreateDrawingSession())
                    EditorPane.DrawLineNumberBoxesForTesting(drawing, 24, boxHeight, 0,
                        new[] { new ViemLineNumberRegionV1 { source_line = 1, top = 0, bottom = boxHeight } }, edges, edges);
                int pixelsWide = (int)target.SizeInPixels.Width, pixelsHigh = (int)target.SizeInPixels.Height;
                byte[] corners = target.GetPixelBytes(0, 0, 1, pixelsHigh);
                byte[] top = target.GetPixelBytes(pixelsWide / 2, 0, 1, 1);
                Check(Enumerable.Range(0, pixelsHigh).All(y => corners[y * 4 + 3] == 128 && corners[y * 4 + 2] > 0 && corners[y * 4] == 0)
                    && top[3] == 128 && top[0] > 0,
                    "translucent border corners and too-short gutter boxes paint once, with vertical edges owning their corners");
            }
            var overlapping = normal with { Margin = new(0, -.75f, 0, -.75f) };
            using (var target = Target(24, 20, dpi)) {
                using (var drawing = target.CreateDrawingSession())
                    EditorPane.DrawLineNumberBoxesForTesting(drawing, 24, 20, 0, new[] {
                        new ViemLineNumberRegionV1 { source_line = 1, top = 0, bottom = 10 },
                        new ViemLineNumberRegionV1 { source_line = 2, top = 10, bottom = 20 }
                    }, overlapping, overlapping);
                byte[] overlap = target.GetPixelBytes(3, (int)(10 * dpi / 96), 1, 1);
                byte[] single = target.GetPixelBytes(3, (int)(3 * dpi / 96), 1, 1);
                Check(overlap[3] == 192 && single[3] == 128,
                    "explicit negative gutter margins retain their intentional overlap without adding an ordinary-column underpaint");
            }
            foreach (double edge in new[] { .125, 10.375, 20.625 }) {
                // Projected Rect stores native floats; compare the actual pixel
                // edge rather than cancellation error in Top + Height.
                long Pixel(double value) => (long)Math.Round(value * dpi / 96);
                var before = EditorPane.NumberPaintBounds(new(0, edge - 10, 30, 10), dpi);
                var after = EditorPane.NumberPaintBounds(new(0, edge, 30, 10), dpi);
                Check(Pixel(before.Bottom) == Pixel(after.Top),
                    "adjacent gutter boxes reuse the identical device-pixel edge");
                before = EditorPane.NumberPaintBounds(new(0, edge - 10, 30, 10), dpi, canvasHeight: edge);
                after = EditorPane.NumberPaintBounds(new(0, edge, 30, 10), dpi, canvasHeight: edge);
                Check(Pixel(before.Bottom) == Pixel(after.Top),
                    $"adjacent gutter boxes share the enclosing pixel edge at the bottom of a fractional canvas (dpi={dpi}, edge={edge}, before={before.Bottom:R}, after={after.Top:R})");
            }
        }
    }

    internal static async Task Run(EditorWindow owner, Preferences ownerPreferences)
    {
        byte[] mask = [0b10100000, 0, 0, 0, 0b01000000, 0, 0, 0];
        byte[] mirroredMask = new byte[mask.Length];
        GutterCursor.MirrorRows(mask, mirroredMask, 3, 2, 4, monochrome: true);
        Check(mirroredMask.SequenceEqual(mask), "symmetric cursor mask rows retain MSB-first bit order and row padding");
        GutterCursor.MirrorRows([0b10000000, 0, 0, 0], mirroredMask.AsSpan(0, 4), 3, 1, 4, monochrome: true);
        Check(mirroredMask[0] == 0b00100000, "cursor mask flips across the actual width, excluding bitmap padding");
        byte[] color = [1, 2, 3, 4, 5, 6, 7, 8], mirroredColor = new byte[8];
        GutterCursor.MirrorRows(color, mirroredColor, 2, 1, 8, monochrome: false);
        Check(mirroredColor.SequenceEqual(new byte[] { 5, 6, 7, 8, 1, 2, 3, 4 }), "cursor mirroring preserves complete BGRA pixels including alpha");
        using (var cursor = GutterCursor.Create())
            Check(cursor is not Microsoft.UI.Input.InputSystemCursor, "gutter selection creates an owned custom mirrored cursor");
        var area = new WindowFrame(-1000, 30, 1000, 800);
        var original = new WindowFrame(-700, 80, 500, 500);
        Check(EditorWindow.GutterFrame(original, area, 40) == original with { X = -740, Width = 540 },
            "gutter expansion preserves content width and screen position by growing left");
        Check(EditorWindow.GutterFrame(original, area, -40) == original with { X = -660, Width = 460 },
            "gutter removal returns space by shrinking from the left");
        Check(EditorWindow.GutterFrame(new(-1000, 80, 500, 500), area, 40) == new WindowFrame(-1000, 80, 540, 500),
            "gutter expansion grows right when the monitor blocks the preferred left side");
        Check(EditorWindow.GutterFrame(new(-1000, 80, 980, 500), area, 40) == new WindowFrame(-1000, 80, 1000, 500),
            "gutter expansion consumes only available monitor space before reducing content width");
        Check(EditorPane.DecimalDigits(0) == 1 && EditorPane.DecimalDigits(9) == 1 && EditorPane.DecimalDigits(10) == 2
            && EditorPane.DecimalDigits(99) == 2 && EditorPane.DecimalDigits(100) == 3 && EditorPane.DecimalDigits(ulong.MaxValue) == 20,
            "gutter digit thresholds use the minimum decimal digits for the whole document");

        string profile = Path.Combine(ownerPreferences.DirectoryPath, "chrome-tests", Guid.NewGuid().ToString("N"));
        var preferences = new Preferences(profile);
        var fixture = JsonNode.Parse(preferences.ThemeStyleDefaults(VIEM_FORMAT_PLAIN_TEXT))!;
        var normal = Definition(fixture, "Builtin line number");
        normal["character"]!["font_families"] = new JsonArray("Missing Viem diagnostic font", "Georgia"); normal["character"]!["size"] = 17;
        normal["character"]!["open_type_features"] = new JsonObject { ["pnum"] = 1, ["tnum"] = 0 };
        normal["block"]!["padding_left"] = 3; normal["block"]!["padding_right"] = 7;
        normal["block"]!["border_right_width"] = 2;
        normal["block"]!["background"] = new JsonObject { ["red"] = 0, ["green"] = 0, ["blue"] = 0, ["alpha"] = .5 };
        var current = Definition(fixture, "Builtin current line number"); current["character"]!["size"] = 23;
        var status = Definition(fixture, "Status bar"); status["character"]!["size"] = 13;
        status["block"]!["padding_top"] = 4; status["block"]!["padding_bottom"] = 6;
        status["block"]!["margin_top"] = 100; status["block"]!["alignment"] = "End";
        var command = Definition(fixture, "Status bar command"); command["character"]!["size"] = 80;
        preferences.SaveThemeStyles(VIEM_FORMAT_PLAIN_TEXT, Bytes(fixture));
        // Code exports are deltas. Missing defaults in an older theme must be
        // expanded on both sides of a change comparison, including a no-op save.
        string themePath = preferences.SelectedThemePath!;
        var legacyTheme = JsonNode.Parse(File.ReadAllBytes(themePath))!;
        var codeDefinitions = legacyTheme["styles"]!["code"]!["block_styles"]!.AsArray();
        foreach (var entry in codeDefinitions.Where(entry => entry?["id"]?.GetValue<string>() is "Status bar" or "Status bar command").ToArray())
            codeDefinitions.Remove(entry);
        File.WriteAllBytes(themePath, Bytes(legacyTheme));
        preferences.SelectTheme(preferences.SelectedTheme, themePath);
        var unrelatedCode = JsonNode.Parse(preferences.ThemeStyleDefaults(VIEM_FORMAT_CODE))!;
        Definition(unrelatedCode, "Paragraph")["character"]!["letter_spacing"] = .75;
        preferences.SaveThemeStyles(VIEM_FORMAT_CODE, Bytes(unrelatedCode));
        Check(preferences.StatusStyles.Status.Size == 13 && preferences.StatusStyles.Command.Size == 80,
            "an unrelated Code edit with omitted status defaults preserves the shared status declarations");
        var document = new CoreDocument(Rows(9)); preferences.AttachThemeDocument(document);
        var window = new EditorWindow(preferences, document); App.Instance.Windows.Add(window);
        try {
            window.Activate(); var pane = window.ActivePane!; var view = await pane.Ready; await Task.Delay(150);
            byte[] source = document.Source(document.State.document_revision); ulong revision = document.State.document_revision;
            var frame = WindowFrame.From(window.AppWindow);
            var viewMenu = window.Menu.Items.Single(menu => menu.Title == "View");
            var menuPeer = new MenuBarItemAutomationPeer(viewMenu);
            var numberItem = viewMenu.Items.OfType<ToggleMenuFlyoutItem>().Single(item => item.Text == "Show line numbers");
            async Task SetViewMenuOpen(bool open)
            {
                if (numberItem.IsLoaded == open) { if (!open) menuPeer.Collapse(); return; }
                var changed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
                void Changed(object sender, RoutedEventArgs args) => pane.DispatcherQueue.TryEnqueue(() => changed.TrySetResult());
                if (open) numberItem.Loaded += Changed; else numberItem.Unloaded += Changed;
                try {
                    // An expansion requested while the previous flyout is still
                    // closing is ignored, so retry until the flyout loads.
                    if (open)
                        for (int attempt = 0; attempt < 16 && !changed.Task.IsCompleted; attempt++)
                        { menuPeer.Expand(); await Task.WhenAny(changed.Task, Task.Delay(300)); }
                    else menuPeer.Collapse();
                    await changed.Task.WaitAsync(TimeSpan.FromSeconds(5));
                } finally { if (open) numberItem.Loaded -= Changed; else numberItem.Unloaded -= Changed; }
            }
            var invisibleItem = viewMenu.Items.OfType<ToggleMenuFlyoutItem>().Single(item => item.Text == "Show Invisible Characters");
            var commentMarkdownItem = viewMenu.Items.OfType<ToggleMenuFlyoutItem>().Single(item => item.Text == "Markdown in comments");
            Check(viewMenu.Items.IndexOf(numberItem) + 1 == viewMenu.Items.IndexOf(commentMarkdownItem)
                && viewMenu.Items.IndexOf(commentMarkdownItem) + 1 == viewMenu.Items.IndexOf(invisibleItem),
                "Show line numbers and Markdown in comments appear immediately before Show Invisible Characters in the View menu");
            await SetViewMenuOpen(true);
            Check(!numberItem.IsChecked && numberItem.IsEnabled && numberItem.Focus(FocusState.Programmatic),
                "the line-number menu check reflects the active view's initially disabled number option");
            var beforeMenu = document.State;
            double beforeMenuStatusHeight = pane.StatusBarHeight;
            await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space); await SetViewMenuOpen(false);
            Check(view.NumberEnabled && pane.GutterWidth > 0 && document.State.document_revision == beforeMenu.document_revision
                && document.State.flags == beforeMenu.flags && pane.StatusBarHeight == beforeMenuStatusHeight
                && document.Source(document.State.document_revision).AsSpan().SequenceEqual(source),
                "the native line-number menu enables the active gutter without changing source, history, or status presentation");
            view.Ex("set nonumber"); await SetViewMenuOpen(true);
            Check(!numberItem.IsChecked, "reopening View refreshes the line-number check after an Ex option change");
            await SetViewMenuOpen(false);
            view.Ex("set number"); await Task.Delay(150); pane.UpdateLayout();
            var pointerRows = view.Layout().Rows;
            float RowY(int index) => pointerRows[index].y + pointerRows[index].ascent * .5f - view.Viewport.top;
            view.SelectPointerRow(RowY(1));
            var pointerSelection = view.LogicalSelection();
            ulong originStart = pointerRows[1].text_start, originEnd = pointerRows[1].text_end + 1;
            Check(pointerSelection.text_start == originStart && pointerSelection.text_end == originEnd,
                "gutter click selects the complete visible row and its break");
            view.SelectPointerRow(RowY(2), true);
            Check(view.LogicalSelection().text_start == originStart && view.LogicalSelection().text_end == pointerRows[2].text_end + 1,
                "gutter downward drag includes its complete original row");
            view.SelectPointerRow(RowY(0), true);
            Check(view.LogicalSelection().text_start == 0 && view.LogicalSelection().text_end == originEnd
                && view.Presentation.cursor_utf8_offset == 0, "gutter reversal includes its original end and places the caret at the active beginning");
            view.SelectPointerRow(RowY(1), true);
            Check(view.LogicalSelection().text_start == originStart && view.LogicalSelection().text_end == originEnd,
                "gutter returning to the initial row retains the complete row");
            view.Key(VIEM_KEY_ESCAPE); view.Command("gg");

            Check(pane.GutterWidth > 0 && pane.LineNumbersForTesting?.Info.source_line_count == 9,
                "set number displays physical source line numbers in a native gutter");
            var sheet = view.Styles();
            var fallback = ChromeStyle.Fallback(preferences.Theme.Foreground, preferences.Theme.Background);
            var number = ChromeStyle.ResolveLineNumber(sheet, current: false, fallback);
            var highlighted = ChromeStyle.ResolveLineNumber(sheet, current: true, number);
            var builtinNumber = sheet.Styles.Single(style => (style.Native.flags & (VIEM_STYLE_DEFINITION_INTERNAL | VIEM_STYLE_DEFINITION_LINE_NUMBER | VIEM_STYLE_DEFINITION_CURRENT_LINE_NUMBER))
                == (VIEM_STYLE_DEFINITION_INTERNAL | VIEM_STYLE_DEFINITION_LINE_NUMBER));
            var builtinCurrent = sheet.Styles.Single(style => (style.Native.flags & (VIEM_STYLE_DEFINITION_INTERNAL | VIEM_STYLE_DEFINITION_LINE_NUMBER | VIEM_STYLE_DEFINITION_CURRENT_LINE_NUMBER))
                == (VIEM_STYLE_DEFINITION_INTERNAL | VIEM_STYLE_DEFINITION_LINE_NUMBER | VIEM_STYLE_DEFINITION_CURRENT_LINE_NUMBER));
            Check(builtinNumber.Name == "Line number" && builtinCurrent.Name == "Current line number",
                "gutter styles use simple display names and distinct builtin identities");
            StyleDefinition SyntaxCollision(StyleDefinition builtin, string id) {
                var native = builtin.Native; native.flags = 0;
                var properties = new Dictionary<uint, ViemStylePropertyV1>(builtin.Properties);
                var size = properties[VIEM_STYLE_PROPERTY_CHARACTER_SIZE]; size.effective.number = 99;
                properties[VIEM_STYLE_PROPERTY_CHARACTER_SIZE] = size;
                return builtin with { Native = native, Id = id, Properties = properties };
            }
            var collisions = sheet with { Styles = new[] { SyntaxCollision(builtinNumber, "syntax.line-number"), SyntaxCollision(builtinCurrent, "syntax.current-line-number") }.Concat(sheet.Styles).ToArray() };
            Check(ChromeStyle.ResolveLineNumber(collisions, current: false, fallback).Size == number.Size
                && ChromeStyle.ResolveLineNumber(collisions, current: true, number).Size == highlighted.Size,
                "content styles with the same line-number names cannot affect gutter appearance");
            var renamed = sheet with { Styles = sheet.Styles.Select(style => ReferenceEquals(style, builtinNumber)
                ? style with { Id = "opaque.gutter", Name = "Other internal label" } : style).ToArray() };
            Check(ChromeStyle.ResolveLineNumber(renamed, current: false, fallback).Size == number.Size,
                "the gutter identifies its builtin by exported flags instead of display names or stable-ID spellings");
            CheckGutterPixels(pane.Canvas.Device);
            Check(number.Family == "Georgia", "chrome skips unavailable font families and uses the next declared fallback");
            double Required(ChromeStyle style) {
                double max = 0;
                for (char digit = '0'; digit <= '9'; digit++) { using var label = style.Layout(pane.Canvas.Device, digit.ToString()); max = Math.Max(max, label.LayoutBounds.Width); }
                return max + style.Margin.Horizontal + style.Padding.Horizontal + style.Border.Horizontal;
            }
            Check(pane.GutterWidth == Math.Ceiling(Math.Max(Required(number), Required(highlighted))),
                "gutter width reserves the widest numeral and both ordinary/current style boxes");
            using (var target = new CanvasRenderTarget(pane.Canvas.Device, (float)pane.GutterWidth, (float)pane.Canvas.ActualHeight, pane.Canvas.Dpi)) {
                using (var drawing = target.CreateDrawingSession()) {
                    pane.DrawLineNumbersForTesting(drawing);
                    Check(pane.DrawnLineNumbers.Count > 0 && pane.DrawnLineNumbers.All(glyph =>
                        glyph.Text.Height <= glyph.Available.Height + .01
                        && Math.Abs(glyph.Text.Y + glyph.Text.Height / 2 - glyph.Available.Y - glyph.Available.Height / 2) < .01),
                        "line numbers shrink to their physical line height and center vertically in the available box");
                }
                var firstBox = pane.DrawnLineNumbers[0].Available;
                int y = (int)Math.Round((firstBox.Y + firstBox.Height / 2) * pane.Canvas.Dpi / 96);
                byte[] inLine = target.GetPixelBytes(1, y, 1, 1);
                byte[] gap = target.GetPixelBytes(1, (int)target.SizeInPixels.Height - 2, 1, 1);
                Check(inLine.AsSpan().SequenceEqual(gap) && gap[0] == 0 && gap[1] == 0 && gap[2] == 0 && gap[3] == 128,
                    "translucent gutter backgrounds paint each line and the column gaps only once");
            }
            EditorPane.DrawnLineNumber DrawFirstNumber() {
                using var target = new CanvasRenderTarget(pane.Canvas.Device, (float)pane.GutterWidth, (float)pane.Canvas.ActualHeight, pane.Canvas.Dpi);
                using var drawing = target.CreateDrawingSession(); pane.DrawLineNumbersForTesting(drawing);
                return pane.DrawnLineNumbers[0];
            }
            normal["block"]!["base_direction"] = "RightToLeft"; normal["block"]!["alignment"] = "Start";
            preferences.SaveThemeStyles(VIEM_FORMAT_PLAIN_TEXT, Bytes(fixture)); await Task.Delay(100);
            var rtlStart = DrawFirstNumber();
            normal["block"]!["alignment"] = "End";
            preferences.SaveThemeStyles(VIEM_FORMAT_PLAIN_TEXT, Bytes(fixture)); await Task.Delay(100);
            var rtlEnd = DrawFirstNumber();
            Check(rtlStart.Text.X > rtlEnd.Text.X + .1
                && Math.Abs(rtlStart.Text.Right - rtlStart.Available.Right) < .01
                && Math.Abs(rtlEnd.Text.Left - rtlEnd.Available.Left) < .01,
                "RTL gutter styles place Start on the right and End on the left within their available boxes");
            normal["block"]!["base_direction"] = "Natural";
            preferences.SaveThemeStyles(VIEM_FORMAT_PLAIN_TEXT, Bytes(fixture)); await Task.Delay(100);
            var staleNumbers = pane.LineNumbersForTesting!.Info;
            try {
                view.Resize((float)pane.Canvas.ActualWidth - 24, (float)pane.Canvas.ActualHeight);
                Check(!view.LineNumbersAreCurrent(staleNumbers), "gutter paint rejects its old identity and viewport after core resize and reflow");
            }
            finally { view.Resize((float)pane.Canvas.ActualWidth, (float)pane.Canvas.ActualHeight); }
            var statusForeground = status["character"]!["foreground"]?.DeepClone();
            var commandForeground = command["character"]!["foreground"]?.DeepClone();
            status["character"]!["foreground"] = null; command["character"]!["foreground"] = null;
            preferences.SaveThemeStyles(VIEM_FORMAT_PLAIN_TEXT, Bytes(fixture)); await Task.Delay(100);
            Check(preferences.StatusStyles.Status.Foreground == preferences.Theme.Foreground
                && preferences.StatusStyles.Command.Foreground == preferences.StatusStyles.Status.Foreground
                && pane.CommandStyle.Foreground == preferences.StatusStyles.Status.Foreground,
                "clearing both status foreground declarations inherits the theme foreground without an inverse fallback");
            status["character"]!["foreground"] = statusForeground; command["character"]!["foreground"] = commandForeground;
            preferences.SaveThemeStyles(VIEM_FORMAT_PLAIN_TEXT, Bytes(fixture)); await Task.Delay(100);
            var originalStatusCharacter = status["character"]!.DeepClone();
            status["character"]!["font_families"] = new JsonArray("Missing Viem diagnostic font", "ui-monospace");
            status["character"]!["font_face"] = ""; status["character"]!["font_axes"] = new JsonObject();
            status["character"]!["letter_spacing"] = 1.5;
            preferences.SaveThemeStyles(VIEM_FORMAT_PLAIN_TEXT, Bytes(fixture)); await Task.Delay(150); pane.UpdateLayout();
            void DrawStatus() {
                using var target = new CanvasRenderTarget(pane.Canvas.Device, (float)pane.StatusControl.ActualWidth, (float)pane.StatusBarHeight, pane.Canvas.Dpi);
                using var drawing = target.CreateDrawingSession(); pane.DrawStatusTextForTesting(drawing);
            }
            var originalStatusBlock = status["block"]!.DeepClone();
            status["block"]!["padding_top"] = 3; status["block"]!["padding_bottom"] = 11;
            status["block"]!["border_top_width"] = 7; status["block"]!["border_bottom_width"] = 30;
            status["block"]!["border_left_width"] = 24; status["block"]!["border_right_width"] = 28;
            preferences.SaveThemeStyles(VIEM_FORMAT_PLAIN_TEXT, Bytes(fixture)); await Task.Delay(150); pane.UpdateLayout(); DrawStatus();
            double bottomHeight = pane.StatusBarHeight;
            double center = (bottomHeight + 7 + 3 - 11) / 2;
            var normalText = pane.DrawnStatusTexts.Single(text => text.Original == "NORMAL");
            Check(Math.Abs(normalText.Bounds.Y + normalText.Bounds.Height / 2 - center) < 1,
                "bottom status text centers inside padding with its thick bottom border outside the window");
            pane.SetStatusBottomEdge(false);
            Check(Math.Abs(pane.StatusBarHeight - bottomHeight - 30) < 1,
                "internal status bars reserve the bottom border that separates viewports");
            pane.SetStatusBottomEdge(true);
            status["block"] = originalStatusBlock;
            preferences.SaveThemeStyles(VIEM_FORMAT_PLAIN_TEXT, Bytes(fixture)); await Task.Delay(150); pane.UpdateLayout();
            DrawStatus();
            var mono = preferences.StatusStyles.Status;
            using (var untracked = (mono with { Tracking = 0 }).Layout(pane.Canvas.Device, "NORMAL"))
                Check(pane.ModeControl.FontFamily.Source == FontCatalog.Resolve("ui-monospace")!.Value.Family
                    && pane.OutputTextControl.CharacterSpacing == (int)Math.Round(mono.Tracking * 1000 / mono.Size)
                    && pane.DrawnStatusTexts.Single(text => text.Original == "NORMAL").Bounds.Width > untracked.LayoutBounds.Width + 4,
                    "status labels resolve generic family fallbacks and paint tracking with the shared shaper while native output retains its spacing");
            var variableFace = FontCatalog.Faces("Flightline Code").First(face => face.Slant == global::Windows.UI.Text.FontStyle.Normal);
            status["character"]!["font_families"] = new JsonArray("Flightline Code");
            status["character"]!["font_face"] = variableFace.PortableStyle;
            status["character"]!["font_axes"] = new JsonObject { ["wght"] = 437 };
            status["character"]!["open_type_features"] = new JsonObject { ["zero"] = 1 };
            preferences.SaveThemeStyles(VIEM_FORMAT_PLAIN_TEXT, Bytes(fixture)); await Task.Delay(150); pane.UpdateLayout(); DrawStatus();
            var fonts = new List<CanvasFontFace>();
            var capture = new GlyphCapture(font => { fonts.Add(font); return new GlyphFontMetadata(font); });
            try {
                var statusLayout = pane.StatusTextLayoutForTesting(pane.ModeControl)!;
                DirectWriteGlyphCapture.Draw(statusLayout, capture);
                var parts = capture.Extract(0, pane.ModeControl.Text.Length, 0, 0);
                Check(parts.Count > 0 && parts.All(part => Math.Abs(FontVariations.NativeAxes(part.Font).GetValueOrDefault("wght", float.NaN) - 437) < .001),
                    "painted status labels use the selected variable face and its explicit native font axes");
            }
            finally { foreach (var font in fonts.Distinct()) font.Dispose(); }
            status["character"] = originalStatusCharacter;
            preferences.SaveThemeStyles(VIEM_FORMAT_PLAIN_TEXT, Bytes(fixture)); await Task.Delay(100);
            int measurements = pane.NumeralMeasurements;
            double oneDigit = pane.GutterWidth;
            view.Command("G"); view.Command("gg"); pane.Refresh();
            Check(pane.GutterWidth == oneDigit && pane.NumeralMeasurements == measurements,
                "moving the caret between current-line styles never changes width or remeasures numerals");
            view.Command("Go"); view.Text("A tenth source line"); view.Key(VIEM_KEY_ESCAPE); await Task.Delay(100);
            Check(pane.LineNumbersForTesting?.Info.source_line_count == 10 && pane.GutterWidth > oneDigit && pane.NumeralMeasurements == measurements,
                "crossing ten physical lines grows the gutter using retained numeral metrics");
            view.Undo(); await Task.Delay(100);
            Check(pane.GutterWidth == oneDigit && pane.NumeralMeasurements == measurements
                && source.AsSpan().SequenceEqual(document.Source(document.State.document_revision)),
                "undo crossing a digit threshold shrinks the gutter without font work and restores source");
            view.Zoom(1.5f); await Task.Delay(100);
            Check(pane.GutterWidth > oneDigit && pane.NumeralMeasurements > measurements,
                "document zoom updates gutter fonts and box geometry with the viewport metrics");
            view.Zoom(1); await Task.Delay(100);
            Check(pane.GutterWidth == oneDigit, "restoring document zoom restores the original gutter width");
            view.Ex("set nonumber"); await Task.Delay(150);
            Check(pane.GutterWidth == 0 && Math.Abs(WindowFrame.From(window.AppWindow).Width - frame.Width) <= 1,
                "hiding the gutter restores a single restored window's original width");
            double statusHeight = pane.StatusBarHeight;
            view.Command(":"); view.Text("w"); pane.Refresh(); await Task.Delay(100);
            Check(pane.StatusBarHeight == statusHeight && pane.CommandStyle.Size == 80,
                "command style font changes clip inside the status container without affecting its height");
            view.Key(VIEM_KEY_ESCAPE);
            var markdown = JsonNode.Parse(preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN))!;
            Definition(markdown, "Status bar")["character"]!["size"] = 19;
            preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, Bytes(markdown)); await Task.Delay(100);
            Check(preferences.StatusStyles.Status.Size == 19 && pane.ModeControl.FontSize == 19
                && Definition(JsonNode.Parse(preferences.ThemeStyleDefaults(VIEM_FORMAT_CODE))!, "Status bar")["character"]!["size"]!.GetValue<int>() == 19,
                "status edits from another stylesheet atomically update the shared chrome authority across families");
            Check(document.Source(document.State.document_revision).AsSpan().SequenceEqual(source)
                && !document.IsDirty, "gutter and status presentation changes preserve source and saved state");
            var largeDocument = new CoreDocument(Rows(99)); preferences.AttachThemeDocument(largeDocument);
            var extra = window.SplitPane(pane, largeDocument); var extraView = await extra.Ready; await Task.Delay(100);
            var multipleFrame = WindowFrame.From(window.AppWindow);
            extraView.Ex("set number"); await Task.Delay(100);
            Check(WindowFrame.From(window.AppWindow) == multipleFrame, "multiple documents consume gutter space within their viewports");
            int retainedMeasurements = extra.NumeralMeasurements; double twoDigits = extra.GutterWidth;
            extraView.Command("Go"); extraView.Text("A hundredth source line"); extraView.Key(VIEM_KEY_ESCAPE); await Task.Delay(100);
            Check(extra.LineNumbersForTesting?.Info.source_line_count == 100 && extra.GutterWidth > twoDigits
                && extra.NumeralMeasurements == retainedMeasurements, "crossing one hundred lines grows the gutter without remeasuring its font");
            extraView.Undo(); await Task.Delay(100);
            Check(extra.GutterWidth == twoDigits && extra.NumeralMeasurements == retainedMeasurements,
                "returning to ninety-nine lines removes a digit while reusing the same metrics");
            await window.ClosePane(extra); window.FocusPane(pane); await Task.Delay(100);
            Check(window.AppWindow.Presenter is OverlappedPresenter,
                $"restored native editor has an overlapped presenter (kind={window.AppWindow.Presenter.Kind}, projectedType={window.AppWindow.Presenter.GetType().FullName})");
            ((OverlappedPresenter)window.AppWindow.Presenter).Maximize(); await Task.Delay(150);
            var maximized = WindowFrame.From(window.AppWindow);
            view.Ex("set number"); await Task.Delay(150);
            Check(WindowFrame.From(window.AppWindow) == maximized, "maximized windows take gutter width from the content viewport");
            view.Ex("set nonumber");
            ((OverlappedPresenter)window.AppWindow.Presenter).Restore(); await Task.Delay(150);
            var second = window.SplitPane(pane, document, vertical: true); var secondView = await second.Ready; await Task.Delay(150);
            var splitFrame = WindowFrame.From(window.AppWindow);
            view.Ex("set number"); await Task.Delay(150);
            Check(WindowFrame.From(window.AppWindow) == splitFrame && !secondView.NumberEnabled,
                "vertical splits reserve independent gutters without resizing the window or another view's option");
            window.FocusPane(pane); await SetViewMenuOpen(true);
            Check(numberItem.IsChecked, "the line-number menu checks the enabled view when it becomes active");
            await SetViewMenuOpen(false); window.FocusPane(second); await SetViewMenuOpen(true);
            Check(!numberItem.IsChecked && numberItem.Focus(FocusState.Programmatic),
                "switching panes refreshes the line-number menu check from that view's own option");
            await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space); await SetViewMenuOpen(false);
            Check(secondView.NumberEnabled && view.NumberEnabled && WindowFrame.From(window.AppWindow) == splitFrame,
                "the native menu toggles only the active split's number option and keeps split window geometry stable");
            await SetViewMenuOpen(true);
            Check(numberItem.IsChecked && numberItem.Focus(FocusState.Programmatic),
                "the line-number menu remains checked after enabling the active split");
            var beforeSplitToggle = document.State;
            await InputRoutingTests.Key(global::Windows.System.VirtualKey.Space); await SetViewMenuOpen(false);
            Check(!secondView.NumberEnabled && view.NumberEnabled
                && document.State.document_revision == beforeSplitToggle.document_revision && document.State.flags == beforeSplitToggle.flags,
                "disabling line numbers through the menu preserves the other view's option and document history");
        }
        finally {
            window.Close(); App.Instance.Windows.Remove(window); owner.Activate(); owner.ActivePane?.FocusEditor();
            CoreThemes.ReplaceCodeStyles(ownerPreferences.ThemeStyleDefaults(VIEM_FORMAT_CODE));
        }
        await CaretGeometryTests.Run(owner, ownerPreferences);
    }
}
#endif

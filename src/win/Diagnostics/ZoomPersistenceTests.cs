#if DEBUG
using System.Text.Json.Nodes;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Viem.Windows.Shell;
using Windows.System;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class ZoomPersistenceTests
{
    private static void Check(bool value, string name)
    { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }

    internal static async Task Run(string profile)
    {
        string directory = Path.Combine(profile, "zoom-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        string config = Path.Combine(directory, "config.json");
        File.WriteAllText(config, "{\"version\":1,\"future\":42,\"view\":{\"future\":true}}");
        var preferences = new Preferences(directory);
        Check(preferences.ZoomScale == 1, "missing remembered zoom defaults to actual size");
        int notifications = 0;
        preferences.Changed += () => notifications++;
        preferences.SetZoomScale(1.75f);
        Check(new Preferences(directory).ZoomScale == 1.75f && notifications == 0
            && JsonNode.Parse(File.ReadAllText(config))!["future"]!.GetValue<int>() == 42
            && preferences.Get("view", "future", false),
            "zoom persists without editor-wide notifications or losing unknown settings");
        var timestamp = new DateTime(2001, 2, 3, 4, 5, 6, DateTimeKind.Utc);
        File.SetLastWriteTimeUtc(config, timestamp);
        preferences.SetZoomScale(1.75f);
        Check(File.GetLastWriteTimeUtc(config) == timestamp, "remembering unchanged zoom does not rewrite settings");
        byte[] before = File.ReadAllBytes(config);
        foreach (float invalid in new[] { 0, .249f, 5.001f, float.NaN, float.PositiveInfinity })
        {
            bool rejected = false;
            try { preferences.SetZoomScale(invalid); } catch (InvalidDataException) { rejected = true; }
            Check(rejected && File.ReadAllBytes(config).AsSpan().SequenceEqual(before),
                "invalid zoom leaves preferences untouched: " + invalid);
        }
        foreach (string invalid in new[] { "null", "true", "\"1.25\"", "[]", "0.24", "5.01" })
        {
            bool rejected = false;
            try { preferences.Set("view", "zoomScale", JsonNode.Parse(invalid)); } catch (InvalidDataException) { rejected = true; }
            Check(rejected && File.ReadAllBytes(config).AsSpan().SequenceEqual(before),
                "shared schema rejects invalid remembered zoom: " + invalid);
        }
        var external = JsonNode.Parse(File.ReadAllText(config))!;
        external["view"]!["zoomScale"] = 1.5f;
        File.WriteAllText(config, external.ToJsonString());
        preferences.SetZoomScale(1.75f);
        Check(new Preferences(directory).ZoomScale == 1.75f,
            "an explicit zoom choice replaces external edits even when the cached value matches");

        preferences.SetZoomScale(1);
        await using (var editor = await TestEditor.Open(preferences, "Words for zooming", VIEM_FORMAT_PLAIN_TEXT))
        {
            var first = editor.Pane;
            var view = editor.View;
            var other = editor.Window.AddPane(editor.Document);
            var otherView = await other.Ready;
            editor.Window.FocusPane(first);
            await InputRoutingTests.CheckForegroundEditor(editor.Window, first, "zoom persistence fixture receives native shortcuts");
            ulong revision = editor.Document.State.document_revision;
            await InputRoutingTests.Key((VirtualKey)187, control: true);
            Check(view.Viewport.scale == 1.1f && new Preferences(directory).ZoomScale == 1.1f,
                "Ctrl+= persists the resulting zoom");
            var menu = editor.Window.Menu.Items.Single(item => item.Title == "View");
            var zoomIn = menu.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == "Zoom In");
            var actual = menu.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == "Actual Size");
            async Task Invoke(MenuFlyoutItem item)
            {
                var peer = new MenuBarItemAutomationPeer(menu);
                await MenuTestHelpers.Transition(true, () => { peer.Expand(); return Task.CompletedTask; }, item);
                await MenuTestHelpers.Transition(false, () => {
                    new MenuFlyoutItemAutomationPeer(item).Invoke(); peer.Collapse(); return Task.CompletedTask;
                }, item);
            }
            await Invoke(zoomIn);
            Check(view.Viewport.scale == 1.25f && new Preferences(directory).ZoomScale == 1.25f,
                "View Zoom In persists through the same route as shortcuts and Ctrl+wheel");
            foreach (float limit in new[] { .25f, 5f })
            {
                view.Zoom(limit);
                File.SetLastWriteTimeUtc(config, timestamp);
                view.StepZoom(limit == 5);
                Check(view.Viewport.scale == limit && new Preferences(directory).ZoomScale == limit
                    && File.GetLastWriteTimeUtc(config) == timestamp,
                    "zoom saturates without rewriting its remembered limit: " + limit);
            }
            await Invoke(actual);
            Check(view.Viewport.scale == 1 && new Preferences(directory).ZoomScale == 1,
                "Actual Size persists 100 percent");
            view.Zoom(1.75f);
            Check(otherView.Viewport.scale == 1 && notifications == 0,
                "remembering zoom retains independent magnification in existing panes");
            before = File.ReadAllBytes(config);
            bool rejectedScale = false;
            try { view.Zoom(5.01f); } catch (CoreException) { rejectedScale = true; }
            Check(rejectedScale && view.Viewport.scale == 1.75f && File.ReadAllBytes(config).AsSpan().SequenceEqual(before),
                "failed zoom cannot change the remembered value");
            Check(editor.Document.State.document_revision == revision && !editor.Document.IsDirty
                && (editor.Document.State.flags & (VIEM_DOCUMENT_STATE_CAN_UNDO | VIEM_DOCUMENT_STATE_CAN_REDO)) == 0
                && editor.Source == "Words for zooming", "zoom persistence does not edit source or history");
        }

        var restartedPreferences = new Preferences(directory);
        await using (var restarted = await TestEditor.Open(restartedPreferences, "Reopened", VIEM_FORMAT_PLAIN_TEXT))
        {
            Check(restarted.View.Viewport.scale == 1.75f && restartedPreferences.ZoomScale == 1.75f,
                "a fresh preferences store and native pane restore the remembered zoom after restart");
            restartedPreferences.SetZoomScale(2);
            using (var replacement = new CoreDocument("Replacement"u8.ToArray(), format: VIEM_FORMAT_PLAIN_TEXT))
            using (var prepared = restarted.Pane.PrepareReplacement(replacement))
                Check(prepared?.Viewport.scale == 1.75f && new Preferences(directory).ZoomScale == 2,
                    "document replacement preserves live zoom without overwriting the latest remembered default");
            File.WriteAllText(config, "{\"version\":99}");
            restarted.View.Zoom(1.25f);
            Check(restarted.View.Viewport.scale == 1.25f && restarted.Pane.LastError == null
                && restarted.Pane.CommandOutputText?.StartsWith("Unable to save editor zoom:", StringComparison.Ordinal) == true
                && File.ReadAllText(config) == "{\"version\":99}",
                "failed preference persistence warns without reverting zoom or overwriting invalid settings");
        }
    }
}
#endif

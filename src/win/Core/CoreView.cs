using System.Text;
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Dispatching;
using Viem.Windows.Interop;
using Viem.Windows.Rendering;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed unsafe partial class CoreView : IDisposable
{
    public CoreDocument Document { get; }
    public ulong Id { get; private set; }
    public DirectWriteProvider Provider { get; }
    public BackgroundLayout BackgroundLayout { get; }
    public BackgroundLayout TableWidthRefinement { get; }
    public ViemCoreOutcomeV1 Outcome { get; private set; }
    public event Action<HostEffects>? Effects;
    public event Action? Changed;
    public event Action? FormattingContextChanged;
    public event Action? Disposed;
    public string ClipboardText { get; set; } = "";
    public string ClipboardFragment { get; set; } = "";
    public ulong ClipboardGeneration { get; set; } = 1;
    public CoreView(CoreDocument document, CanvasDevice device, DispatcherQueue dispatcher, float width, float height, ViemLayoutInsetsV1 padding = default)
    {
        Document = document;
        using (Diagnostics.StartupPerformance.Measure("view.provider")) Provider = new(device, dispatcher);
        var table = Provider.Table;
        var options = New<ViemViewOptionsV1>(); options.execution_context = VIEM_LAYOUT_EXECUTION_FRONTEND_MAIN; options.width = Math.Max(1, width); options.height = Math.Max(1, height);
        options.padding_top = padding.top; options.padding_left = padding.left;
        options.padding_bottom = padding.bottom; options.padding_right = padding.right;
        var outcome = New<ViemCoreOutcomeV1>(); ulong id = 0;
        uint status;
        using (Diagnostics.StartupPerformance.Measure("view.initialLayout")) status = viem_core_view_add(document.Handle, &options, &table, &id, &outcome);
        if (status != 0) { string detail = Provider.LastError ?? ""; Provider.Dispose(); throw new InvalidOperationException($"Create editor view failed ({status}). {detail}"); }
        Id = id; Outcome = outcome;
        using (Diagnostics.StartupPerformance.Measure("view.backgroundSetup"))
        {
            TableWidthRefinement = new(this, dispatcher, tableRefinement: true);
            TableWidthRefinement.Update();
            BackgroundLayout = new(this, dispatcher);
            BackgroundLayout.Update();
        }
    }
    public ViemViewPresentationV1 Presentation { get { var p = New<ViemViewPresentationV1>(); Check(viem_core_view_presentation(Document.Handle, Id, &p), "Read presentation"); return p; } }
    public ViemViewportStateV1 Viewport { get { var p = New<ViemViewportStateV1>(); Check(viem_core_view_viewport_state(Document.Handle, Id, &p), "Read viewport"); return p; } }
    public ViemViewLineLocationV1 Location { get { var p = New<ViemViewLineLocationV1>(); Check(viem_core_view_line_location(Document.Handle, Id, &p), "Read location"); return p; } }
    public bool IsVisual => Presentation.mode is VIEM_MODE_VISUAL_CHARACTER or VIEM_MODE_VISUAL_LINE or VIEM_MODE_VISUAL_BLOCK;
    public static bool IsTextSelectionMode(uint mode) => mode is VIEM_MODE_SELECT_CHARACTER or VIEM_MODE_SELECT_LINE or VIEM_MODE_SELECT_BLOCK
        or VIEM_MODE_SELECTION_CHARACTER or VIEM_MODE_SELECTION_LINE or VIEM_MODE_SELECTION_BLOCK;
    public bool IsTextSelection => IsTextSelectionMode(Presentation.mode);
    public bool HasSelection => Presentation.mode is VIEM_MODE_VISUAL_CHARACTER or VIEM_MODE_VISUAL_LINE or VIEM_MODE_VISUAL_BLOCK
        or VIEM_MODE_SELECT_CHARACTER or VIEM_MODE_SELECT_LINE or VIEM_MODE_SELECT_BLOCK
        or VIEM_MODE_SELECTION_CHARACTER or VIEM_MODE_SELECTION_LINE or VIEM_MODE_SELECTION_BLOCK;
    public bool HasPendingMapping { get { byte value = 0; Check(viem_core_view_has_pending_mapping(Document.Handle, Id, &value), "Read mapping"); return value != 0; } }
    private delegate uint Turn(ViemCommandTurnContextV2* context, ViemCoreOutcomeV1* outcome, ulong* effects);
    private HostEffects? Send(Turn turn, bool publish = true)
    {
        using var arena = new NativeArena();
        var entry = New<ViemClipboardTurnEntryV2>();
        entry.flags = VIEM_CLIPBOARD_TURN_WRITABLE | VIEM_CLIPBOARD_TURN_HAS_READ; entry.generation = ClipboardGeneration;
        entry.plain_text = arena.Utf8(ClipboardText); entry.fragment_json = arena.Utf8(ClipboardFragment);
        // Windows has one system clipboard rather than a separate X11-style
        // primary selection. Expose that same snapshot under both Vim names so
        // both the + and * registers remain usable without conflating their
        // identities inside the core.
        ViemClipboardTurnEntryV2* entries = stackalloc ViemClipboardTurnEntryV2[2];
        entries[0] = entry; entries[0].target = VIEM_CLIPBOARD_TARGET_CLIPBOARD;
        entries[1] = entry; entries[1].target = VIEM_CLIPBOARD_TARGET_PRIMARY;
        var context = New<ViemCommandTurnContextV2>(); context.clipboards = entries; context.clipboard_count = 2;
        var outcome = New<ViemCoreOutcomeV1>(); ulong batch = 0;
        uint status;
        using (Diagnostics.InputPerformance.Measure("core.turn")) status = turn(&context, &outcome, &batch);
        HostEffects? effects = null;
        try { if (batch != 0) effects = HostEffects.Read(batch); }
        finally { if (batch != 0) Check(viem_effect_batch_release(batch), "Release effects"); }
        Finish(status, outcome);
        if (effects != null && publish) Effects?.Invoke(effects);
        return effects;
    }
    public void Key(uint kind, uint codepoint = 0, uint modifiers = 0) => Send((context, outcome, effects) =>
    {
        var input = New<ViemKeyInputV1>(); input.kind = kind; input.codepoint = codepoint; input.modifiers = modifiers;
        return viem_core_view_send_key_with_host_context_v2(Document.Handle, Id, &input, context, outcome, effects);
    });
    public void Text(string text) => Send((context, outcome, effects) =>
    {
        byte[] bytes = Encoding.UTF8.GetBytes(text);
        fixed (byte* p = bytes) return viem_core_view_send_text_with_host_context_v2(Document.Handle, Id, p, (ulong)bytes.Length, context, outcome, effects);
    });
    public void FlushMapping() => Send((c, o, e) => viem_core_view_flush_mapping_with_host_context_v2(Document.Handle, Id, c, o, e));
    public void Command(string keys) { foreach (var rune in keys.EnumerateRunes()) Key(VIEM_KEY_CHARACTER, (uint)rune.Value); }
    public void Ex(string command) { Key(VIEM_KEY_ESCAPE); Command(":"); Text(command); Key(VIEM_KEY_ENTER); }
    public void CopyOrCut(bool cut)
    {
        if (cut) SelectionCommand("\"+d");
        else if (HasSelection) Key(VIEM_KEY_COPY_SELECTION);
    }
    public void SelectionCommand(string command)
    {
        if (!HasSelection) return;
        // Native actions operate on the selection; their command spelling
        // must not become replacement text in Select mode.
        if (IsTextSelection) Key(VIEM_KEY_CONTROL_CHARACTER, 'o');
        Command(command);
    }
    public void SelectFromCommand(string command, uint origin = VIEM_SELECTION_ORIGIN_KEY, uint? returnMode = null)
    {
        var before = Presentation;
        returnMode ??= before.mode;
        Key(VIEM_KEY_ESCAPE);
        if ((before.mode is VIEM_MODE_INSERT or VIEM_MODE_REPLACE) && before.document_revision == Document.State.document_revision)
            Place(before.cursor_utf8_offset, before.cursor_affinity, before.document_revision);
        foreach (var rune in command.EnumerateRunes())
        {
            // selectmode=cmd may make the leading v enter Select mode. The
            // remaining synthetic text-object keys are still commands.
            if (IsTextSelection) Key(VIEM_KEY_CONTROL_CHARACTER, 'g');
            Key(VIEM_KEY_CHARACTER, (uint)rune.Value);
        }
        if (HasSelection) SetSelectionOrigin(origin, returnMode.Value);
    }
    public void SetSelectionOrigin(uint origin, uint returnMode = VIEM_MODE_NORMAL) => Apply(o => viem_core_view_set_selection_origin(Document.Handle, Id, origin,
        returnMode is VIEM_MODE_INSERT or VIEM_MODE_REPLACE ? returnMode : VIEM_MODE_NORMAL, o));
    public void Paste(bool plain = false)
    {
        var presentation = Presentation;
        if (presentation.mode == VIEM_MODE_COMMAND_LINE)
        {
            var prompt = Prompt();
            EditPrompt(prompt, Math.Min(prompt.Anchor, prompt.Active), Math.Max(prompt.Anchor, prompt.Active), ClipboardText);
            return;
        }
        string rich = ClipboardFragment;
        if (plain) ClipboardFragment = "";
        try
        {
            Key(VIEM_KEY_PASTE_CLIPBOARD);
        }
        finally { ClipboardFragment = rich; }
    }
    internal delegate uint Operation(ViemCoreOutcomeV1* outcome);
    internal void Apply(Operation operation)
    { var outcome = New<ViemCoreOutcomeV1>(); uint status; using (Diagnostics.InputPerformance.Measure("core.operation")) status = operation(&outcome); Finish(status, outcome); }
    private void Finish(uint status, ViemCoreOutcomeV1 outcome)
    {
        Check(status, Provider.LastError ?? "Editor operation"); Outcome = outcome;
        if ((outcome.flags & VIEM_OUTCOME_DOCUMENT_CHANGED) != 0) Document.NotifyChanged();
        else Changed?.Invoke();
        BackgroundLayout.Update();
        TableWidthRefinement.Update();
        if (outcome.command_status == VIEM_COMMAND_STATUS_READ_ONLY) throw new InvalidOperationException("E45: readonly option is set (use ! to override)");
        if (outcome.command_status == VIEM_COMMAND_STATUS_ERROR) throw new InvalidOperationException("The command could not be completed.");
    }
    internal void TableWidthsChanged() { Changed?.Invoke(); TableWidthRefinement.Update(); }
    public void RestorePosition(CoreView previous)
    {
        var state = New<ViemViewRestorationV1>();
        Check(viem_core_view_capture_restoration(previous.Document.Handle, previous.Id, &state), "Capture document position");
        var target = Document.State;
        var captured = state;
        Apply(outcome => { var copy = captured; return viem_core_view_restore(Document.Handle, Id, target.document_id, target.document_revision, &copy, outcome); });
    }
    // The core owns the bounded deadline and current viewport identity.
    // Publication refreshes all panes; those secondary refreshes must not wait.
    public bool WaitForSyntax()
    {
        if (Document.IsPublishingSyntaxChange) return false;
        byte changed = 0;
        Check(viem_core_view_wait_for_syntax(Document.Handle, Id, &changed), "Prepare syntax paint");
        if (changed != 0) Document.PublishSyntaxChange();
        return changed != 0;
    }
    public void Refresh() => Apply(o => viem_core_view_state(Document.Handle, Id, o));
    public void Resize(float width, float height) => Apply(o => viem_core_view_resize(Document.Handle, Id, Math.Max(1, width), Math.Max(1, height), o));
    public void Undo() => Apply(o => viem_core_view_undo(Document.Handle, Id, o));
    public void Redo() => Apply(o => viem_core_view_redo(Document.Handle, Id, o));
    public void SelectAll() => Apply(o => { var s = Document.State; return viem_core_view_select_all(Document.Handle, Id, s.document_id, s.document_revision, o); });
    public void Wrap(bool value) => Apply(o => viem_core_view_set_wrap(Document.Handle, Id, value ? 1u : 0u, o));
    public void ParagraphFlow(bool value) => Apply(o => viem_core_view_set_paragraph_flow(Document.Handle, Id, value ? 1u : 0u, o));
    public bool ParagraphFlowEnabled { get { uint value = 0; Check(viem_core_view_paragraph_flow(Document.Handle, Id, &value), "Read paragraph flow"); return value != 0; } }
    public uint CurrentLineMode { get { uint value = 0; Check(viem_core_view_line_mode(Document.Handle, Id, &value), "Read line mode"); return value; } }
    public float DefaultColumnWidth { get { float width = 0; Check(viem_core_view_default_column_width(Document.Handle, Id, &width), "Read default column width"); return width; } }
    public float DefaultLineHeight { get { float height = 0; Check(viem_core_view_default_line_height(Document.Handle, Id, &height), "Read default line height"); return height; } }
    public void LineMode(uint value) => Apply(o => viem_core_view_set_line_mode(Document.Handle, Id, value, o));
    public void GoToLine(ulong value) => Apply(o => { var s = Document.State; return viem_core_view_go_to_line(Document.Handle, Id, s.document_id, s.document_revision, value, o); });
    public void Zoom(float scale) => Apply(o => viem_core_view_set_scale(Document.Handle, Id, scale, o));
    public void StepZoom(bool increase) { float target = 1; Check(viem_core_adjacent_zoom_scale(Viewport.scale, increase ? 1u : 0u, &target), "Zoom"); Zoom(target); }
    public void Padding(float top, float left, float bottom, float right) { Check(viem_core_view_set_padding(Document.Handle, Id, top, left, bottom, right), "Set margins"); Refresh(); }
    public void MarkdownAutodetect(bool enabled) => Check(viem_core_view_set_markdown_autodetect(Document.Handle, Id, enabled ? 1u : 0u), "Markdown typing");
    public void SmartQuotes(bool enabled) => Check(viem_core_view_set_smart_quotes(Document.Handle, Id, enabled ? 1u : 0u), "Smart quotes");
    public void VisibleWhitespace(bool enabled) { Check(viem_core_view_set_visible_whitespace(Document.Handle, Id, (byte)(enabled ? 1 : 0)), "Visible whitespace"); Refresh(); }
    public void Scroll(float left, float top)
    {
        var state = Viewport;
        Apply(o => {
            var request = New<ViemViewportOriginV1>(); request.flags = VIEM_VIEWPORT_ORIGIN_HAS_TOP;
            request.left = Math.Max(0, left); request.top = Math.Max(0, top);
            request.expected_document_id = state.document_id; request.expected_document_revision = state.document_revision;
            request.expected_layout_revision = state.layout_revision; request.expected_configuration_generation = state.configuration_generation;
            request.expected_measurement_environment_id = state.measurement_environment_id; request.expected_metrics_generation = state.metrics_generation;
            return viem_core_view_set_viewport_origin(Document.Handle, Id, &request, o);
        });
    }
    public void Place(float x, float y, bool extend = false, bool wholeWords = false)
    {
        var info = LayoutInfo(); var viewport = Viewport;
        var request = New<ViemLayoutHitTestRequestV1>(); request.identity = info.identity; request.flags = extend ? 0 : VIEM_LAYOUT_HIT_TEST_POINTER_DOWN; request.x = x + viewport.left; request.y = y + viewport.top;
        var point = New<ViemLayoutCaretPointV1>(); Check(viem_core_view_layout_hit_test(Document.Handle, Id, &request, &point), "Place cursor");
        Place(point.text_offset, point.affinity, point.document_revision, extend, wholeWords, beginPointerGesture: !extend && !wholeWords);
    }
    public void Place(ulong offset, uint affinity, ulong revision, bool extend = false, bool wholeWords = false, bool beginPointerGesture = false) => Apply(o =>
    {
        var request = New<ViemPlaceCursorV1>(); request.text_offset = offset; request.affinity = affinity; request.document_revision = revision; request.flags = (extend ? VIEM_PLACE_CURSOR_EXTEND_SELECTION : 0) | (wholeWords ? VIEM_PLACE_CURSOR_WORD_SELECTION : 0) | (beginPointerGesture ? VIEM_PLACE_CURSOR_BEGIN_POINTER_GESTURE : 0);
        return viem_core_view_place_cursor(Document.Handle, Id, &request, o);
    });
    public string CommandLine()
    {
        var info = New<ViemCommandLineInfoV1>(); Check(viem_core_view_command_line_info(Document.Handle, Id, &info), "Read prompt");
        var identity = info.identity;
        var bytes = Copy((p, n, r) => { var expected = identity; var result = New<ViemCommandLineInfoV1>(); uint s = viem_core_view_copy_command_line(Document.Handle, Id, &expected, p, n, &result); *r = result.utf8_length; return s; });
        return (identity.kind switch { 1 => ":", 2 => "/", 3 => "?", _ => "" }) + Encoding.UTF8.GetString(bytes);
    }
    public string SubstitutePrompt() => Encoding.UTF8.GetString(Copy((p, n, r) => viem_core_view_copy_substitute_confirmation(Document.Handle, Id, p, n, r)));
    public void Poll()
    {
        byte pending = 0;
        Check(viem_core_view_search_work_pending(Document.Handle, Id, &pending), "Read search progress");
        byte changed = 0;
        if (pending != 0) { Check(viem_core_view_poll_search(Document.Handle, Id, &changed), "Update search"); if (changed != 0) Changed?.Invoke(); }
        var completion = New<ViemCompletionInfoV1>();
        Check(viem_core_view_completion_info(Document.Handle, Id, &completion), "Read completion");
        if ((completion.flags & VIEM_COMPLETION_SEARCHING) != 0) { Check(viem_core_view_poll_completion(Document.Handle, Id, &changed), "Update completion"); if (changed != 0) Changed?.Invoke(); }
    }
    public void Dispose()
    {
        if (Id == 0) return;
        BackgroundLayout.Dispose();
        TableWidthRefinement.Dispose();
        Check(viem_core_view_remove(Document.Handle, Id), "Close pane"); Id = 0; Provider.Dispose();
        Disposed?.Invoke();
    }
}

use viem_core::ffi::*;
use std::ffi::c_void;
use std::ptr;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;

/// Exercise the sole input route even when a test has no clipboard or host.
/// The effect-aware tests below inspect the batches instead of discarding them.
fn discard_test_effects(status: ViemStatus, effects: ViemEffectBatchHandle) -> ViemStatus {
    if effects != 0 {
        assert_eq!(viem_effect_batch_release(effects), ViemStatus::Ok);
    }
    status
}

unsafe fn test_send_key(
    core: ViemCoreHandle,
    view: ViemViewId,
    input: *const ViemKeyInputV1,
    outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    let mut effects = 0;
    let status = unsafe {
        viem_core_view_send_key_with_host_context_v2(
            core,
            view,
            input,
            &ViemCommandTurnContextV2::default(),
            outcome,
            &mut effects,
        )
    };
    discard_test_effects(status, effects)
}

unsafe fn test_send_text(
    core: ViemCoreHandle,
    view: ViemViewId,
    text: *const u8,
    length: u64,
    outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    let mut effects = 0;
    let status = unsafe {
        viem_core_view_send_text_with_host_context_v2(
            core,
            view,
            text,
            length,
            &ViemCommandTurnContextV2::default(),
            outcome,
            &mut effects,
        )
    };
    discard_test_effects(status, effects)
}

unsafe fn test_set_format(
    core: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemSetFormatV1,
    outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    let mut effects = 0;
    let status = unsafe {
        viem_core_view_set_format_with_effects(core, view, request, outcome, &mut effects)
    };
    discard_test_effects(status, effects)
}

unsafe fn test_set_encoding(
    core: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemSetEncodingV1,
    outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    let mut effects = 0;
    let status = unsafe {
        viem_core_view_set_encoding_with_effects(core, view, request, outcome, &mut effects)
    };
    discard_test_effects(status, effects)
}

const FAKE_FONT: &[u8] = b"FFI Fake Sans";

struct FakeResponseStorage {
    _carets: Vec<Vec<ViemClusterCaretStopV1>>,
    clusters: Vec<ViemShapedClusterV1>,
    visual_order: Vec<u64>,
}

struct FakeProviderContext {
    core: ViemCoreHandle,
    metrics_generation: u64,
    expected_environment: u64,
    expected_owner: u64,
    shape_calls: usize,
    shaped_bytes: usize,
    minimum_request_start: u64,
    maximum_request_end: u64,
    fail_next: Option<ViemStatus>,
    saw_complete_request: bool,
    saw_crossing_cluster_tail: bool,
    last_requested_scale: f32,
    reentrant_status: u32,
    responses: Vec<FakeResponseStorage>,
}

impl FakeProviderContext {
    fn new(core: ViemCoreHandle) -> Self {
        Self {
            core,
            metrics_generation: 1,
            expected_environment: 0xa11c_e001,
            expected_owner: 0xf00d,
            shape_calls: 0,
            shaped_bytes: 0,
            minimum_request_start: u64::MAX,
            maximum_request_end: 0,
            fail_next: None,
            saw_complete_request: false,
            saw_crossing_cluster_tail: false,
            last_requested_scale: 0.0,
            reentrant_status: u32::MAX,
            responses: Vec::new(),
        }
    }
}

#[derive(Default)]
struct CallbackGateState {
    entered: bool,
    released: bool,
}

#[derive(Default)]
struct CallbackGate {
    state: Mutex<CallbackGateState>,
    changed: Condvar,
}

impl CallbackGate {
    fn enter_and_wait_for_release(&self) {
        let mut state = self.state.lock().unwrap();
        state.entered = true;
        self.changed.notify_all();
        while !state.released {
            state = self.changed.wait(state).unwrap();
        }
    }

    fn wait_until_entered(&self) {
        let state = self.state.lock().unwrap();
        let (state, timeout) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(5), |state| !state.entered)
            .unwrap();
        assert!(
            state.entered,
            "provider callback did not start before the test timeout ({timeout:?})"
        );
    }

    fn release(&self) {
        let mut state = self.state.lock().unwrap();
        state.released = true;
        self.changed.notify_all();
    }
}

struct BlockingProviderContext {
    provider: FakeProviderContext,
    gate: Arc<CallbackGate>,
}

unsafe extern "C" fn fake_metrics_generation(context: *mut c_void) -> u64 {
    // SAFETY: Tests keep the boxed context alive until after view removal.
    unsafe { (*(context as *mut FakeProviderContext)).metrics_generation }
}

unsafe extern "C" fn blocking_metrics_generation(context: *mut c_void) -> u64 {
    // SAFETY: The lifetime test retains this context until core destruction
    // succeeds, and a busy destroy explicitly does not end its lifetime.
    unsafe {
        (*(context as *mut BlockingProviderContext))
            .provider
            .metrics_generation
    }
}

fn response_storage(request: &ViemShapeRequestV1) -> Option<FakeResponseStorage> {
    fn owned_utf8(value: ViemUtf8Slice) -> Option<String> {
        if value.length > usize::MAX as u64 || (value.length != 0 && value.data.is_null()) {
            return None;
        }
        // SAFETY: The core callback contract supplies this readable request
        // slice for the complete synchronous callback; the test immediately
        // copies it into owned storage.
        let bytes = unsafe { std::slice::from_raw_parts(value.data, value.length as usize) };
        std::str::from_utf8(bytes).ok().map(str::to_owned)
    }

    let context_before = owned_utf8(request.context_before)?;
    let text = owned_utf8(request.text)?;
    let context_after = owned_utf8(request.context_after)?;
    if request.text_end.checked_sub(request.text_start)? != text.len() as u64 {
        return None;
    }
    let context_start = request
        .text_start
        .checked_sub(context_before.len() as u64)?;
    let shaping_capacity = context_before
        .len()
        .checked_add(text.len())?
        .checked_add(context_after.len())?;
    let mut shaping_text = String::with_capacity(shaping_capacity);
    shaping_text.push_str(&context_before);
    shaping_text.push_str(&text);
    shaping_text.push_str(&context_after);
    let graphemes: Vec<_> = shaping_text.grapheme_indices(true).collect();
    let metrics = ViemTextMetricsV1 {
        ascent: 10.0,
        descent: 3.0,
        leading: 1.0,
    };
    let mut carets = Vec::new();
    let mut cluster_ranges = Vec::new();
    let mut grapheme_index = 0;
    while grapheme_index < graphemes.len() {
        let local_start = graphemes[grapheme_index].0;
        let mut grapheme_count = 1;
        for candidate in [3, 2] {
            if grapheme_index + candidate <= graphemes.len() {
                let candidate_end = if grapheme_index + candidate == graphemes.len() {
                    shaping_text.len()
                } else {
                    graphemes[grapheme_index + candidate].0
                };
                if matches!(
                    &shaping_text[local_start..candidate_end],
                    "fi" | "fl" | "ffi" | "ffl"
                ) {
                    grapheme_count = candidate;
                    break;
                }
            }
        }
        let local_end = if grapheme_index + grapheme_count == graphemes.len() {
            shaping_text.len()
        } else {
            graphemes[grapheme_index + grapheme_count].0
        };
        let text_start = context_start.checked_add(local_start as u64)?;
        let text_end = context_start.checked_add(local_end as u64)?;
        if request.text_start <= text_start && text_start < request.text_end {
            let advance = 8.0 * grapheme_count as f32;
            carets.push(vec![
                ViemClusterCaretStopV1 {
                    text_offset: text_start,
                    inline_offset: 0.0,
                    affinity: VIEM_BOUNDARY_AFFINITY_DOWNSTREAM,
                },
                ViemClusterCaretStopV1 {
                    text_offset: text_end,
                    inline_offset: advance,
                    affinity: VIEM_BOUNDARY_AFFINITY_UPSTREAM,
                },
            ]);
            cluster_ranges.push((text_start, text_end, advance));
        }
        grapheme_index += grapheme_count;
    }
    let clusters = cluster_ranges
        .iter()
        .zip(&carets)
        .enumerate()
        .map(
            |(index, ((text_start, text_end, advance), carets))| ViemShapedClusterV1 {
                struct_size: VIEM_SHAPED_CLUSTER_V1_SIZE,
                reserved: 0,
                text_start: *text_start,
                text_end: *text_end,
                advance: *advance,
                metrics,
                typographic_bounds: ViemShapedBoundsV1 {
                    x: 0.0,
                    y: -10.0,
                    width: *advance,
                    height: 13.0,
                },
                ink_bounds: ViemShapedBoundsV1 {
                    x: 0.0,
                    y: -10.0,
                    width: *advance,
                    height: 13.0,
                },
                bidi_level: 0,
                has_render_run: 1,
                fallback_font: ViemUtf8Slice {
                    data: FAKE_FONT.as_ptr(),
                    length: FAKE_FONT.len() as u64,
                },
                caret_stops: carets.as_ptr(),
                caret_stop_count: carets.len() as u64,
                render_run: ViemRenderRunHandleV1 {
                    owner: request.render_run_owner,
                    identifier: *text_start + index as u64 + 1,
                    metrics_generation: request.metrics_generation,
                    threading: request.render_run_threading,
                    reserved: 0,
                },
            },
        )
        .collect::<Vec<_>>();
    let visual_order = (0..clusters.len() as u64).collect();
    Some(FakeResponseStorage {
        _carets: carets,
        clusters,
        visual_order,
    })
}

unsafe extern "C" fn fake_shape_batch(
    context: *mut c_void,
    requests: *const ViemShapeRequestV1,
    request_count: u64,
    responses: *mut ViemShapeResponseV1,
    response_capacity: u64,
) -> u32 {
    if context.is_null()
        || request_count != response_capacity
        || request_count > usize::MAX as u64
        || (request_count != 0 && (requests.is_null() || responses.is_null()))
    {
        return ViemStatus::InvalidArgument as u32;
    }
    // SAFETY: Core supplies equal readable/writable arrays for the duration of
    // this synchronous callback, and the test owns the context.
    let context = unsafe { &mut *(context as *mut FakeProviderContext) };
    let requests = if request_count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(requests, request_count as usize) }
    };
    let responses = if response_capacity == 0 {
        &mut []
    } else {
        unsafe { std::slice::from_raw_parts_mut(responses, response_capacity as usize) }
    };
    context.shape_calls += 1;
    if let Some(status) = context.fail_next.take() {
        return status as u32;
    }
    let mut revision = u64::MAX;
    // A same-core call must acquire the registry immediately and report Busy;
    // deadlock here would prove the callback was made under the registry lock.
    context.reentrant_status = unsafe { viem_core_revision(context.core, &mut revision) } as u32;
    context.responses.clear();
    for request in requests {
        context.last_requested_scale = request.scale;
        context.shaped_bytes = context
            .shaped_bytes
            .saturating_add(request.text.length as usize)
            .saturating_add(request.context_before.length as usize)
            .saturating_add(request.context_after.length as usize);
        context.minimum_request_start = context.minimum_request_start.min(request.text_start);
        context.maximum_request_end = context.maximum_request_end.max(request.text_end);
        let style_is_complete = request.default_style.struct_size
            >= VIEM_RESOLVED_TEXT_STYLE_V1_SIZE
            && request.default_style.font_family_count > 0
            && !request.default_style.font_families.is_null()
            && request.default_style.size > 0.0;
        context.saw_complete_request |= request.struct_size >= VIEM_SHAPE_REQUEST_V1_SIZE
            && request.measurement_environment_id == context.expected_environment
            && request.metrics_generation == context.metrics_generation
            && request.purpose == VIEM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA
            && request.has_render_run_policy == 1
            && request.render_run_owner == context.expected_owner
            && request.render_run_threading == VIEM_RENDER_THREADING_ANY
            && request.paragraph_base_direction == VIEM_TEXT_DIRECTION_AUTO
            && style_is_complete;
        let Some(storage) = response_storage(request) else {
            return ViemStatus::ProviderFailure as u32;
        };
        context.saw_crossing_cluster_tail |= storage
            .clusters
            .iter()
            .any(|cluster| cluster.text_end > request.text_end);
        context.responses.push(storage);
    }
    for ((request, storage), output) in requests
        .iter()
        .zip(&context.responses)
        .zip(responses.iter_mut())
    {
        *output = ViemShapeResponseV1 {
            struct_size: VIEM_SHAPE_RESPONSE_V1_SIZE,
            reserved: 0,
            document_id: request.document_id,
            document_revision: request.document_revision,
            measurement_environment_id: request.measurement_environment_id,
            metrics_generation: request.metrics_generation,
            text_start: request.text_start,
            text_end: request.text_end,
            clusters: if storage.clusters.is_empty() {
                ptr::null()
            } else {
                storage.clusters.as_ptr()
            },
            cluster_count: storage.clusters.len() as u64,
            visual_order: if storage.visual_order.is_empty() {
                ptr::null()
            } else {
                storage.visual_order.as_ptr()
            },
            visual_order_count: storage.visual_order.len() as u64,
            default_metrics: ViemTextMetricsV1 {
                ascent: 10.0,
                descent: 3.0,
                leading: 1.0,
            },
            diagnostics: ptr::null(),
            diagnostic_count: 0,
        };
    }
    ViemStatus::Ok as u32
}

unsafe extern "C" fn blocking_shape_batch(
    context: *mut c_void,
    requests: *const ViemShapeRequestV1,
    request_count: u64,
    responses: *mut ViemShapeResponseV1,
    response_capacity: u64,
) -> u32 {
    if context.is_null() {
        return ViemStatus::InvalidArgument as u32;
    }
    // SAFETY: The test retains the boxed context through the callback and
    // through the later successful destroy. The main thread accesses only its
    // separately owned Arc gate while this mutable reference is live.
    let context = unsafe { &mut *(context as *mut BlockingProviderContext) };
    context.gate.enter_and_wait_for_release();
    // SAFETY: Delegate with the live embedded provider context and the exact
    // pointer/count pairs supplied by core for this synchronous callback.
    unsafe {
        fake_shape_batch(
            (&mut context.provider as *mut FakeProviderContext).cast(),
            requests,
            request_count,
            responses,
            response_capacity,
        )
    }
}

unsafe extern "C" fn malformed_shape_batch(
    _context: *mut c_void,
    requests: *const ViemShapeRequestV1,
    request_count: u64,
    responses: *mut ViemShapeResponseV1,
    response_capacity: u64,
) -> u32 {
    if request_count != response_capacity
        || request_count > usize::MAX as u64
        || (request_count != 0 && (requests.is_null() || responses.is_null()))
    {
        return ViemStatus::InvalidArgument as u32;
    }
    let requests = if request_count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(requests, request_count as usize) }
    };
    let responses = if response_capacity == 0 {
        &mut []
    } else {
        unsafe { std::slice::from_raw_parts_mut(responses, response_capacity as usize) }
    };
    for (request, response) in requests.iter().zip(responses) {
        *response = ViemShapeResponseV1 {
            struct_size: VIEM_SHAPE_RESPONSE_V1_SIZE,
            document_id: request.document_id,
            document_revision: request.document_revision,
            measurement_environment_id: request.measurement_environment_id,
            metrics_generation: request.metrics_generation,
            text_start: request.text_start,
            text_end: request.text_end,
            clusters: ptr::null(),
            cluster_count: 1,
            default_metrics: ViemTextMetricsV1 {
                ascent: 10.0,
                descent: 3.0,
                leading: 1.0,
            },
            ..ViemShapeResponseV1::default()
        };
    }
    ViemStatus::Ok as u32
}

unsafe extern "C" fn unstable_context_shape_batch(
    _context: *mut c_void,
    _requests: *const ViemShapeRequestV1,
    _request_count: u64,
    _responses: *mut ViemShapeResponseV1,
    _response_capacity: u64,
) -> u32 {
    ViemStatus::UnstableShapingContext as u32
}

unsafe extern "C" fn fake_retain_render_runs(
    _context: *mut c_void, _handles: *const ViemRenderRunHandleV1, _count: u64,
) -> *mut c_void { ptr::NonNull::<u8>::dangling().as_ptr().cast() }
unsafe extern "C" fn fake_release_render_runs(_lease: *mut c_void) {}

fn provider(
    context: *mut c_void,
    shape_batch: ViemShapeBatchCallback,
) -> ViemTextMeasurementProviderV1 {
    ViemTextMeasurementProviderV1 {
        struct_size: VIEM_TEXT_MEASUREMENT_PROVIDER_V1_SIZE,
        abi_version: VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION,
        context,
        measurement_environment_id: 0xa11c_e001,
        threading: VIEM_PROVIDER_THREADING_ANY_WORKER,
        has_render_run_policy: 1,
        render_run_owner: 0xf00d,
        render_run_threading: VIEM_RENDER_THREADING_ANY,
        reserved: 0,
        metrics_generation: Some(fake_metrics_generation),
        shape_batch: Some(shape_batch),
        retain_render_runs: Some(fake_retain_render_runs),
        release_render_runs: Some(fake_release_render_runs),
    }
}

struct TestCore {
    handle: ViemCoreHandle,
    revision: u64,
}

impl Drop for TestCore {
    fn drop(&mut self) {
        if self.handle != 0 {
            let _ = viem_core_destroy(self.handle);
        }
    }
}

fn create_core(source: &[u8], options: ViemDocumentOptions) -> TestCore {
    let mut handle = 0;
    let mut revision = u64::MAX;
    let status = unsafe {
        viem_core_create(
            source.as_ptr(),
            source.len() as u64,
            &options,
            &mut handle,
            &mut revision,
        )
    };
    assert_eq!(status, ViemStatus::Ok);
    assert_ne!(handle, 0);
    TestCore { handle, revision }
}

fn document_state(core: &TestCore) -> ViemDocumentStateV1 {
    let mut state = ViemDocumentStateV1::default();
    assert_eq!(
        unsafe { viem_core_document_state(core.handle, &mut state) },
        ViemStatus::Ok
    );
    state
}

type CoreCopy = unsafe extern "C" fn(ViemCoreHandle, u64, *mut u8, u64, *mut u64) -> ViemStatus;

fn copy_core_bytes(function: CoreCopy, core: &TestCore, revision: u64) -> Vec<u8> {
    let mut required = 0;
    let status = unsafe { function(core.handle, revision, ptr::null_mut(), 0, &mut required) };
    assert_eq!(
        status,
        if required == 0 {
            ViemStatus::Ok
        } else {
            ViemStatus::BufferTooSmall
        }
    );
    let mut bytes = vec![0; required as usize];
    let output = if bytes.is_empty() {
        ptr::null_mut()
    } else {
        bytes.as_mut_ptr()
    };
    assert_eq!(
        unsafe {
            function(
                core.handle,
                revision,
                output,
                bytes.len() as u64,
                &mut required,
            )
        },
        ViemStatus::Ok
    );
    bytes
}

fn key(kind: u32, codepoint: u32) -> ViemKeyInputV1 {
    ViemKeyInputV1 {
        struct_size: VIEM_KEY_INPUT_V1_SIZE,
        kind,
        codepoint,
        modifiers: 0,
    }
}

fn utf8_slice(bytes: &[u8]) -> ViemUtf8Slice {
    ViemUtf8Slice {
        data: if bytes.is_empty() {
            ptr::null()
        } else {
            bytes.as_ptr()
        },
        length: bytes.len() as u64,
    }
}

fn add_test_view(
    core: &TestCore,
    context: *mut FakeProviderContext,
) -> (ViemViewId, ViemCoreOutcomeV1) {
    let provider = provider(context.cast(), fake_shape_batch);
    let mut view = 0;
    let mut outcome = ViemCoreOutcomeV1::default();
    assert_eq!(
        unsafe {
            viem_core_view_add(
                core.handle,
                &ViemViewOptionsV1::default(),
                &provider,
                &mut view,
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    (view, outcome)
}

#[allow(dead_code)]
struct CopiedEffectBatch {
    info: ViemEffectBatchInfoV1,
    clipboard_writes: Vec<ViemClipboardWriteV1>,
    ex_requests: Vec<ViemExFrontendRequestV1>,
    ex_options: Vec<ViemExOptionDisplayV1>,
    ex_marks: Vec<ViemExMarkV1>,
    ex_registers: Vec<ViemExRegisterV1>,
    ex_jumps: Vec<ViemExJumpV1>,
    ex_text_lines: Vec<ViemExTextLineV1>,
    file_formats: Vec<u32>,
    hard_breaks: Vec<u64>,
    strings: Vec<u8>,
}

impl CopiedEffectBatch {
    fn text(&self, reference: ViemEffectBytesRefV1) -> &str {
        let start = usize::try_from(reference.offset).unwrap();
        let length = usize::try_from(reference.length).unwrap();
        std::str::from_utf8(&self.strings[start..start + length]).unwrap()
    }
}

fn copy_effect_batch(handle: ViemEffectBatchHandle) -> CopiedEffectBatch {
    let mut info = ViemEffectBatchInfoV1::default();
    assert_eq!(
        unsafe { viem_effect_batch_info(handle, &mut info) },
        ViemStatus::Ok
    );
    let mut clipboard_writes =
        vec![ViemClipboardWriteV1::default(); info.clipboard_write_count as usize];
    let mut ex_requests = vec![ViemExFrontendRequestV1::default(); info.ex_request_count as usize];
    let mut ex_options = vec![ViemExOptionDisplayV1::default(); info.ex_option_count as usize];
    let mut ex_marks = vec![ViemExMarkV1::default(); info.ex_mark_count as usize];
    let mut ex_registers = vec![ViemExRegisterV1::default(); info.ex_register_count as usize];
    let mut ex_jumps = vec![ViemExJumpV1::default(); info.ex_jump_count as usize];
    let mut ex_text_lines = vec![ViemExTextLineV1::default(); info.ex_text_line_count as usize];
    let mut file_formats = vec![0; info.file_format_count as usize];
    let mut hard_breaks = vec![0; info.hard_break_count as usize];
    let mut strings = vec![0; info.string_bytes as usize];
    let mut copied_info = ViemEffectBatchInfoV1::default();
    assert_eq!(
        unsafe {
            viem_effect_batch_copy(
                handle,
                clipboard_writes.as_mut_ptr(),
                clipboard_writes.len() as u64,
                ex_requests.as_mut_ptr(),
                ex_requests.len() as u64,
                ex_options.as_mut_ptr(),
                ex_options.len() as u64,
                ex_marks.as_mut_ptr(),
                ex_marks.len() as u64,
                ex_registers.as_mut_ptr(),
                ex_registers.len() as u64,
                ex_jumps.as_mut_ptr(),
                ex_jumps.len() as u64,
                ex_text_lines.as_mut_ptr(),
                ex_text_lines.len() as u64,
                file_formats.as_mut_ptr(),
                file_formats.len() as u64,
                hard_breaks.as_mut_ptr(),
                hard_breaks.len() as u64,
                strings.as_mut_ptr(),
                strings.len() as u64,
                &mut copied_info,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(copied_info, info);
    CopiedEffectBatch {
        info,
        clipboard_writes,
        ex_requests,
        ex_options,
        ex_marks,
        ex_registers,
        ex_jumps,
        ex_text_lines,
        file_formats,
        hard_breaks,
        strings,
    }
}

fn host_key(
    core: &TestCore,
    view: ViemViewId,
    input: ViemKeyInputV1,
    entries: &[ViemClipboardTurnEntryV2],
) -> (ViemStatus, ViemCoreOutcomeV1, ViemEffectBatchHandle) {
    let context = ViemCommandTurnContextV2 {
        clipboards: if entries.is_empty() {
            ptr::null()
        } else {
            entries.as_ptr()
        },
        clipboard_count: entries.len() as u64,
        ..ViemCommandTurnContextV2::default()
    };
    let mut outcome = ViemCoreOutcomeV1::default();
    let mut effects = 0;
    let status = unsafe {
        viem_core_view_send_key_with_host_context_v2(
            core.handle,
            view,
            &input,
            &context,
            &mut outcome,
            &mut effects,
        )
    };
    (status, outcome, effects)
}

fn host_text(
    core: &TestCore,
    view: ViemViewId,
    text: &str,
) -> (ViemStatus, ViemCoreOutcomeV1, ViemEffectBatchHandle) {
    let context = ViemCommandTurnContextV2::default();
    let mut outcome = ViemCoreOutcomeV1::default();
    let mut effects = 0;
    let status = unsafe {
        viem_core_view_send_text_with_host_context_v2(
            core.handle,
            view,
            text.as_ptr(),
            text.len() as u64,
            &context,
            &mut outcome,
            &mut effects,
        )
    };
    (status, outcome, effects)
}

fn host_chars(core: &TestCore, view: ViemViewId, chars: &str) {
    for character in chars.chars() {
        let (status, _, effects) = host_key(
            core,
            view,
            key(VIEM_KEY_CHARACTER, u32::from(character)),
            &[],
        );
        assert_eq!(status, ViemStatus::Ok);
        assert_eq!(effects, 0);
    }
}

fn host_ex(core: &TestCore, view: ViemViewId, command: &str) -> ViemEffectBatchHandle {
    host_chars(core, view, ":");
    let (status, _, effects) = host_text(core, view, command);
    assert_eq!(status, ViemStatus::Ok);
    assert_eq!(effects, 0);
    let (status, outcome, effects) = host_key(core, view, key(VIEM_KEY_ENTER, 0), &[]);
    assert_eq!(status, ViemStatus::Ok, "executing :{command}");
    assert_ne!(
        outcome.flags & VIEM_OUTCOME_HAS_EXTERNAL_EFFECTS,
        0,
        ":{command} did not publish external effects (status {})",
        outcome.command_status
    );
    assert_ne!(effects, 0, ":{command} did not publish an effect batch");
    effects
}

#[test]
fn host_context_turns_exchange_clipboards_and_own_raw_ex_effects() {
    let mut core = create_core("a".as_bytes(), ViemDocumentOptions::default());
    let mut provider_context = Box::new(FakeProviderContext::new(core.handle));
    let (view, _) = add_test_view(&core, &mut *provider_context);

    let invalid = [0xff_u8];
    let bad_entry = ViemClipboardTurnEntryV2 {
        flags: VIEM_CLIPBOARD_TURN_HAS_READ,
        target: VIEM_CLIPBOARD_TARGET_CLIPBOARD,
        generation: 4,
        plain_text: utf8_slice(&invalid),
        ..ViemClipboardTurnEntryV2::default()
    };
    let mut rejected_outcome = ViemCoreOutcomeV1 {
        flags: u32::MAX,
        ..ViemCoreOutcomeV1::default()
    };
    let mut rejected_effect = u64::MAX;
    let bad_context = ViemCommandTurnContextV2 {
        clipboards: &bad_entry,
        clipboard_count: 1,
        ..ViemCommandTurnContextV2::default()
    };
    assert_eq!(
        unsafe {
            viem_core_view_send_key_with_host_context_v2(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, u32::from('x')),
                &bad_context,
                &mut rejected_outcome,
                &mut rejected_effect,
            )
        },
        ViemStatus::InvalidUtf8
    );
    assert_eq!(rejected_outcome, ViemCoreOutcomeV1::default());
    assert_eq!(rejected_effect, 0);
    assert_eq!(document_state(&core).document_revision, 0);

    host_chars(&core, view, "\"+");
    let pasted = "世界👋";
    let read_entry = ViemClipboardTurnEntryV2 {
        flags: VIEM_CLIPBOARD_TURN_HAS_READ,
        target: VIEM_CLIPBOARD_TARGET_CLIPBOARD,
        generation: 77,
        plain_text: utf8_slice(pasted.as_bytes()),
        ..ViemClipboardTurnEntryV2::default()
    };
    let (status, paste_outcome, paste_effects) = host_key(
        &core,
        view,
        key(VIEM_KEY_CHARACTER, u32::from('p')),
        &[read_entry],
    );
    assert_eq!(status, ViemStatus::Ok);
    assert_ne!(paste_outcome.flags & VIEM_OUTCOME_DOCUMENT_CHANGED, 0);
    assert_eq!(paste_effects, 0, "a clipboard read emits no host write");
    let revision = paste_outcome.document_revision;
    assert_eq!(
        copy_core_bytes(viem_core_copy_formatted_utf8, &core, revision),
        "a世界👋".as_bytes()
    );

    host_chars(&core, view, "\"+y");
    let writable = ViemClipboardTurnEntryV2 {
        flags: VIEM_CLIPBOARD_TURN_WRITABLE,
        target: VIEM_CLIPBOARD_TARGET_CLIPBOARD,
        ..ViemClipboardTurnEntryV2::default()
    };
    let (status, yank_outcome, yank_effects) = host_key(
        &core,
        view,
        key(VIEM_KEY_CHARACTER, u32::from('y')),
        &[writable],
    );
    assert_eq!(status, ViemStatus::Ok);
    assert_ne!(yank_outcome.flags & VIEM_OUTCOME_HAS_EXTERNAL_EFFECTS, 0);
    assert_ne!(yank_effects, 0);
    let yank = copy_effect_batch(yank_effects);
    assert_eq!(yank.clipboard_writes.len(), 1);
    let write = yank.clipboard_writes[0];
    assert_eq!(write.target, VIEM_CLIPBOARD_TARGET_CLIPBOARD);
    assert_eq!(write.register_kind, VIEM_REGISTER_KIND_LINE);
    assert_ne!(write.flags & VIEM_CLIPBOARD_WRITE_HAS_PORTABLE_REGISTER, 0);
    assert_eq!(yank.text(write.plain_text), "a世界👋\n");
    assert_eq!(viem_effect_batch_release(yank_effects), ViemStatus::Ok);

    let write_effects = host_ex(&core, view, "w");
    let captured = copy_effect_batch(write_effects);
    assert_eq!(captured.ex_requests.len(), 1);
    assert_eq!(captured.ex_requests[0].kind, VIEM_EX_FRONTEND_WRITE);
    assert_eq!(captured.ex_requests[0].flags, 0);
    assert_eq!(
        captured.ex_requests[0].document_id,
        captured.info.document_id
    );
    assert_eq!(
        captured.ex_requests[0].document_revision,
        captured.info.document_revision
    );

    // The immutable raw request remains usable after the originating core and
    // its current revision no longer exist. Release, rather than core lifetime,
    // is the sole ownership boundary.
    assert_eq!(viem_core_destroy(core.handle), ViemStatus::Ok);
    core.handle = 0;
    let after_destroy = copy_effect_batch(write_effects);
    assert_eq!(after_destroy.info, captured.info);
    assert_eq!(after_destroy.ex_requests, captured.ex_requests);
    assert_eq!(viem_effect_batch_release(write_effects), ViemStatus::Ok);
    assert_eq!(
        viem_effect_batch_release(write_effects),
        ViemStatus::InvalidHandle
    );
    let mut stale_info = ViemEffectBatchInfoV1 {
        batch_handle: u64::MAX,
        ..ViemEffectBatchInfoV1::default()
    };
    assert_eq!(
        unsafe { viem_effect_batch_info(write_effects, &mut stale_info) },
        ViemStatus::InvalidHandle
    );
    assert_eq!(stale_info, ViemEffectBatchInfoV1::default());
}

#[test]
fn ex_info_effects_capture_marks_registers_jumps_and_printed_text() {
    let core = create_core(
        "α one\nβ two\nγ three".as_bytes(),
        ViemDocumentOptions::default(),
    );
    let mut provider_context = Box::new(FakeProviderContext::new(core.handle));
    let (view, _) = add_test_view(&core, &mut *provider_context);

    host_chars(&core, view, "maG\"ayy");

    let marks_handle = host_ex(&core, view, "marks a");
    let marks = copy_effect_batch(marks_handle);
    assert_eq!(marks.ex_requests[0].kind, VIEM_EX_FRONTEND_MARKS);
    assert_eq!(
        (
            marks.ex_requests[0].first_payload,
            marks.ex_requests[0].payload_count
        ),
        (0, 1)
    );
    assert_eq!(marks.ex_marks.len(), 1);
    assert_eq!(marks.ex_marks[0].name, u32::from('a'));
    assert_eq!(marks.ex_marks[0].utf8_offset, 0);
    assert_eq!(marks.ex_marks[0].hard_line_index, 0);
    assert_eq!(marks.ex_marks[0].grapheme_column, 0);
    assert_eq!(marks.text(marks.ex_marks[0].line_text), "α one");
    assert_eq!(viem_effect_batch_release(marks_handle), ViemStatus::Ok);

    let registers_handle = host_ex(&core, view, "registers a");
    let registers = copy_effect_batch(registers_handle);
    assert_eq!(registers.ex_requests[0].kind, VIEM_EX_FRONTEND_REGISTERS);
    assert_eq!(registers.ex_registers.len(), 1);
    assert_eq!(registers.ex_registers[0].name, u32::from('a'));
    assert_eq!(
        registers.ex_registers[0].register_kind,
        VIEM_REGISTER_KIND_LINE
    );
    assert_eq!(registers.text(registers.ex_registers[0].text), "γ three\n");
    assert_eq!(viem_effect_batch_release(registers_handle), ViemStatus::Ok);

    let jumps_handle = host_ex(&core, view, "jumps");
    let jumps = copy_effect_batch(jumps_handle);
    assert_eq!(jumps.ex_requests[0].kind, VIEM_EX_FRONTEND_JUMPS);
    assert_eq!(jumps.ex_jumps.len(), 2);
    assert_eq!(jumps.ex_jumps[0].hard_line_index, 0);
    assert_eq!(jumps.ex_jumps[1].hard_line_index, 2);
    assert_eq!(jumps.ex_jumps[1].flags, VIEM_EX_JUMP_CURRENT);
    assert_eq!(jumps.text(jumps.ex_jumps[1].line_text), "γ three");
    assert_eq!(viem_effect_batch_release(jumps_handle), ViemStatus::Ok);

    let print_handle = host_ex(&core, view, "1,2s/o/o/p");
    let printed = copy_effect_batch(print_handle);
    assert_eq!(printed.ex_requests[0].kind, VIEM_EX_FRONTEND_PRINT_LINES);
    assert_eq!(printed.ex_requests[0].flags, VIEM_EX_FRONTEND_HAS_RANGE);
    assert_eq!(
        (
            printed.ex_requests[0].first_payload,
            printed.ex_requests[0].payload_count
        ),
        (0, 2)
    );
    assert_eq!(printed.ex_text_lines.len(), 2);
    assert_eq!(printed.text(printed.ex_text_lines[0].text), "α one");
    assert_eq!(printed.text(printed.ex_text_lines[1].text), "β two");
    assert_eq!(viem_effect_batch_release(print_handle), ViemStatus::Ok);
}

#[test]
fn view_creation_applies_initial_padding_and_rejects_invalid_values() {
    let mut core = create_core(b"first\nsecond", ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let provider = provider((&mut *context as *mut FakeProviderContext).cast(), fake_shape_batch);
    let mut options = ViemViewOptionsV1 { padding_top: 10.0, padding_left: 17.0, padding_bottom: 12.0, padding_right: 23.0, ..Default::default() };
    let mut view = 0;
    let mut outcome = ViemCoreOutcomeV1::default();
    assert_eq!(unsafe { viem_core_view_add(core.handle, &options, &provider, &mut view, &mut outcome) }, ViemStatus::Ok);
    let mut info = ViemLayoutSnapshotInfoV1::default();
    assert_eq!(unsafe { viem_core_view_layout_snapshot_info(core.handle, view, &mut info) }, ViemStatus::Ok);
    assert_eq!(info.content_insets, ViemLayoutInsetsV1 { top: 10.0, left: 17.0, bottom: 12.0, right: 23.0 });
    let initial = info.identity;
    assert_eq!(viem_core_view_set_padding(core.handle, view, 10.0, 17.0, 12.0, 23.0), ViemStatus::Ok);
    assert_eq!(unsafe { viem_core_view_layout_snapshot_info(core.handle, view, &mut info) }, ViemStatus::Ok);
    assert_eq!(info.identity, initial);
    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
    for value in [f32::NAN, f32::INFINITY, -1.0] {
        options.padding_left = value;
        assert_eq!(unsafe { viem_core_view_add(core.handle, &options, &provider, &mut view, &mut outcome) }, ViemStatus::InvalidArgument);
        assert_eq!(view, 0);
    }
    assert_eq!(viem_core_destroy(core.handle), ViemStatus::Ok);
    core.handle = 0;
}

#[test]
fn completion_anchor_exports_exact_layout_and_rejects_geometry_from_before_resize() {
    let mut core = create_core(b"alphabet almanac\nal", ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let provider = provider((&mut *context as *mut FakeProviderContext).cast(), fake_shape_batch);
    let mut view = 0;
    let mut outcome = ViemCoreOutcomeV1::default();
    let options = ViemViewOptionsV1 { width: 400.0, height: 200.0, ..ViemViewOptionsV1::default() };
    assert_eq!(unsafe { viem_core_view_add(core.handle, &options, &provider, &mut view, &mut outcome) }, ViemStatus::Ok);
    host_chars(&core, view, "GA");
    assert_eq!(unsafe { test_send_key(core.handle, view, &key(VIEM_KEY_CONTROL_CHARACTER, 'p' as u32), &mut outcome) }, ViemStatus::Ok);
    let mut first = ViemCompletionInfoV1::default();
    assert_eq!(unsafe { viem_core_view_completion_info(core.handle, view, &mut first) }, ViemStatus::Ok);
    assert_ne!(first.flags & VIEM_COMPLETION_HAS_ANCHOR, 0);
    assert_eq!(first.flags & VIEM_COMPLETION_RIGHT_TO_LEFT, 0);
    assert_eq!(first.anchor_layout.view_id, view);
    assert_eq!(first.anchor_layout.document_revision, first.document_revision);
    assert!(first.anchor_rect.height > 0.0);
    assert_eq!(first.anchor_rect.width, 0.0);
    assert_eq!(unsafe { test_send_key(core.handle, view, &key(VIEM_KEY_CONTROL_CHARACTER, 'p' as u32), &mut outcome) }, ViemStatus::Ok);
    let mut second = ViemCompletionInfoV1::default();
    assert_eq!(unsafe { viem_core_view_completion_info(core.handle, view, &mut second) }, ViemStatus::Ok);
    assert_eq!(first.anchor_rect, second.anchor_rect);
    assert_ne!(first.anchor_layout.layout_revision, second.anchor_layout.layout_revision);
    assert_eq!(unsafe { viem_core_view_resize(core.handle, view, 320.0, 180.0, &mut outcome) }, ViemStatus::Ok);
    let mut resized = ViemCompletionInfoV1::default();
    assert_eq!(unsafe { viem_core_view_completion_info(core.handle, view, &mut resized) }, ViemStatus::Ok);
    assert_eq!(second.generation, resized.generation, "resize retains the candidate selection");
    assert_ne!(second.anchor_layout, resized.anchor_layout);
    let mut count = 99;
    assert_eq!(unsafe { viem_core_view_copy_completion_items(core.handle, view, &second, ptr::null_mut(), 0, &mut count) }, ViemStatus::StaleRevision);
    assert_eq!(count, 0);
    assert_eq!(copy_core_bytes(viem_core_copy_source_bytes, &core, core.revision), b"alphabet almanac\nal");
    assert_eq!(viem_core_destroy(core.handle), ViemStatus::Ok);
    core.handle = 0;
}

#[test]
fn callback_backed_core_round_trips_controller_view_and_exact_source() {
    let mut core = create_core(
        b"caf\xe9\r\n",
        ViemDocumentOptions {
            encoding: VIEM_ENCODING_LATIN1,
            file_format: VIEM_FILE_FORMAT_DOS,
            ..ViemDocumentOptions::default()
        },
    );
    assert_eq!(core.revision, 0);
    assert_eq!(
        copy_core_bytes(viem_core_copy_source_bytes, &core, 0),
        b"caf\xe9\r\n"
    );
    assert_eq!(
        copy_core_bytes(viem_core_copy_formatted_utf8, &core, 0),
        "café\n".as_bytes()
    );

    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let provider = provider(
        (&mut *context as *mut FakeProviderContext).cast(),
        fake_shape_batch,
    );
    let options = ViemViewOptionsV1 {
        width: 240.0,
        height: 100.0,
        ..ViemViewOptionsV1::default()
    };
    let mut view = 0;
    let mut outcome = ViemCoreOutcomeV1::default();
    assert_eq!(
        unsafe { viem_core_view_add(core.handle, &options, &provider, &mut view, &mut outcome,) },
        ViemStatus::Ok
    );
    assert_eq!(view, 1);
    assert_eq!(outcome.document_revision, 0);
    assert_eq!(outcome.mode, VIEM_MODE_NORMAL);
    assert_eq!(outcome.view_id, view);
    assert_eq!(outcome.measurement_environment_id, 0xa11c_e001);
    assert_eq!(outcome.metrics_generation, 1);
    assert_ne!(outcome.flags & VIEM_OUTCOME_HAS_LAYOUT, 0);
    assert!(context.saw_complete_request);
    assert!(context.shape_calls > 0);
    assert_eq!(context.reentrant_status, ViemStatus::CoreBusy as u32);

    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, 'i' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(outcome.mode, VIEM_MODE_INSERT);
    assert_eq!(outcome.command_status, VIEM_COMMAND_STATUS_COMPLETE);
    assert_ne!(outcome.flags & VIEM_OUTCOME_MODE_CHANGED, 0);

    assert_eq!(
        unsafe { test_send_text(core.handle, view, b"!".as_ptr(), 1, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(outcome.document_revision, 1);
    assert_ne!(outcome.flags & VIEM_OUTCOME_DOCUMENT_CHANGED, 0);
    assert_ne!(outcome.flags & VIEM_OUTCOME_HAS_POSITION_MAP, 0);
    core.revision = 1;
    assert_eq!(
        copy_core_bytes(viem_core_copy_source_bytes, &core, 1),
        b"!caf\xe9\r\n"
    );
    assert_eq!(
        copy_core_bytes(viem_core_copy_formatted_utf8, &core, 1),
        "!café\n".as_bytes()
    );

    assert_eq!(
        unsafe { viem_core_view_resize(core.handle, view, 120.0, 50.0, &mut outcome) },
        ViemStatus::Ok
    );
    assert_ne!(outcome.flags & VIEM_OUTCOME_LAYOUT_CHANGED, 0);
    assert_eq!(
        unsafe { viem_core_view_set_wrap(core.handle, view, 0, &mut outcome) },
        ViemStatus::Ok
    );
    assert_ne!(outcome.flags & VIEM_OUTCOME_HAS_LAYOUT, 0);

    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
    assert_eq!(
        unsafe { viem_core_view_state(core.handle, view, &mut outcome) },
        ViemStatus::InvalidView
    );
    let mut replacement_view = 0;
    assert_eq!(
        unsafe {
            viem_core_view_add(
                core.handle,
                &options,
                &provider,
                &mut replacement_view,
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(replacement_view, 2, "removed view IDs must not be reused");
    assert_eq!(
        viem_core_view_remove(core.handle, replacement_view),
        ViemStatus::Ok
    );
}

#[test]
fn core_create_detects_encoding_losslessly_and_explicit_values_remain_forced() {
    fn utf16_source(text: &str, little_endian: bool) -> Vec<u8> {
        let mut bytes = if little_endian {
            vec![0xff, 0xfe]
        } else {
            vec![0xfe, 0xff]
        };
        for unit in text.encode_utf16() {
            let encoded = if little_endian {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            };
            bytes.extend_from_slice(&encoded);
        }
        bytes
    }

    let cases = [
        (
            b"plain UTF-8 \xf0\x9f\x98\x80".to_vec(),
            VIEM_ENCODING_UTF8,
            "plain UTF-8 😀".as_bytes().to_vec(),
            false,
        ),
        (
            [vec![0xef, 0xbb, 0xbf], "BOM café".as_bytes().to_vec()].concat(),
            VIEM_ENCODING_UTF8,
            "BOM café".as_bytes().to_vec(),
            true,
        ),
        (
            utf16_source("little 😀", true),
            VIEM_ENCODING_UTF16_LE,
            "little 😀".as_bytes().to_vec(),
            true,
        ),
        (
            utf16_source("big 😀", false),
            VIEM_ENCODING_UTF16_BE,
            "big 😀".as_bytes().to_vec(),
            true,
        ),
        (
            b"caf\xe9".to_vec(),
            VIEM_ENCODING_LATIN1,
            "café".as_bytes().to_vec(),
            false,
        ),
        (
            vec![0xf0, 0x28, 0x8c, 0x28],
            VIEM_ENCODING_LATIN1,
            "ð(\u{8c}(".as_bytes().to_vec(),
            false,
        ),
        (
            vec![0xef, 0xbb, 0xbf, 0xff],
            VIEM_ENCODING_UTF8,
            "\u{fffd}".as_bytes().to_vec(),
            true,
        ),
        // Automatic detection deliberately does not guess BOM-less UTF-16.
        // These bytes are valid UTF-8 containing NUL scalars, so UTF-8 wins.
        (
            vec![b'A', 0, b'B', 0],
            VIEM_ENCODING_UTF8,
            vec![b'A', 0, b'B', 0],
            false,
        ),
    ];

    for (source, expected_encoding, expected_text, expected_bom) in cases {
        let core = create_core(
            &source,
            ViemDocumentOptions {
                encoding: VIEM_ENCODING_DETECT,
                file_format: VIEM_FILE_FORMAT_UNIX,
                ..ViemDocumentOptions::default()
            },
        );
        let state = document_state(&core);
        assert_eq!(state.encoding, expected_encoding, "source {source:?}");
        assert_eq!(
            state.flags & VIEM_DOCUMENT_STATE_HAS_BOM != 0,
            expected_bom,
            "source {source:?}"
        );
        assert_eq!(
            copy_core_bytes(viem_core_copy_source_bytes, &core, 0),
            source,
            "detection must not rewrite authoritative bytes"
        );
        assert_eq!(
            copy_core_bytes(viem_core_copy_formatted_utf8, &core, 0),
            expected_text,
            "source {source:?}"
        );
    }

    // The historical Rust convenience default remains an explicit UTF-8
    // request. Invalid UTF-8 is therefore preserved as an opaque diagnostic
    // projection instead of silently switching that existing caller to
    // Latin-1.
    assert_eq!(ViemDocumentOptions::default().encoding, VIEM_ENCODING_UTF8);
    let forced_utf8_source = b"caf\xe9";
    let forced_utf8 = create_core(
        forced_utf8_source,
        ViemDocumentOptions {
            encoding: VIEM_ENCODING_UTF8,
            ..ViemDocumentOptions::default()
        },
    );
    assert_eq!(document_state(&forced_utf8).encoding, VIEM_ENCODING_UTF8);
    assert_eq!(
        copy_core_bytes(viem_core_copy_source_bytes, &forced_utf8, 0),
        forced_utf8_source
    );
    assert_eq!(
        copy_core_bytes(viem_core_copy_formatted_utf8, &forced_utf8, 0),
        "caf\u{fffd}".as_bytes()
    );

    // Explicit Latin-1 also remains authoritative even when the bytes begin
    // with a signature another policy would recognize as a UTF-8 BOM.
    let forced_latin1_source = [0xef, 0xbb, 0xbf, b'A'];
    let forced_latin1 = create_core(
        &forced_latin1_source,
        ViemDocumentOptions {
            encoding: VIEM_ENCODING_LATIN1,
            ..ViemDocumentOptions::default()
        },
    );
    let forced_latin1_state = document_state(&forced_latin1);
    assert_eq!(forced_latin1_state.encoding, VIEM_ENCODING_LATIN1);
    assert_eq!(forced_latin1_state.flags & VIEM_DOCUMENT_STATE_HAS_BOM, 0);
    assert_eq!(
        copy_core_bytes(viem_core_copy_formatted_utf8, &forced_latin1, 0),
        "ï»¿A".as_bytes()
    );
}

#[test]
fn native_named_style_create_delete_are_sparse_revision_bound_and_undoable() {
    for (format, original, prefix) in [

        (VIEM_FORMAT_RTF, br"{\rtf1 Text}".as_slice(), "Rtf"),
    ] {
        let core = create_core(
            original,
            ViemDocumentOptions {
                format,
                ..Default::default()
            },
        );
        let mut provider = Box::new(FakeProviderContext::new(core.handle));
        let (view, mut outcome) = add_test_view(&core, provider.as_mut());
        let query = || {
            let mut info = ViemStyleSheetInfoV1::default();
            assert_eq!(
                unsafe { viem_core_style_sheet_info(core.handle, &mut info) },
                ViemStatus::Ok
            );
            info.identity
        };
        for (namespace, suffix) in [
            (VIEM_STYLE_NAMESPACE_BLOCK, "P7"),
            (VIEM_STYLE_NAMESPACE_CHARACTER, "C4"),
        ] {
            let id = format!("{prefix}{suffix}");
            let request = ViemCreateStyleV1 {
                struct_size: VIEM_CREATE_STYLE_V1_SIZE,
                namespace,
                identity: query(),
                style_id: utf8_slice(id.as_bytes()),
                display_name: utf8_slice("Sparse α".as_bytes()),
                parent_id: ViemUtf8Slice::default(),
                next_style_id: ViemUtf8Slice::default(),
            };
            assert_eq!(
                unsafe { viem_core_view_create_style(core.handle, view, &request, &mut outcome) },
                ViemStatus::Ok,
                "format {format}"
            );
            assert_eq!(
                unsafe { viem_core_view_create_style(core.handle, view, &request, &mut outcome) },
                ViemStatus::StaleRevision
            );
            let created = copy_core_bytes(
                viem_core_copy_source_bytes,
                &core,
                document_state(&core).document_revision,
            );
            assert_ne!(created, original);
            assert_eq!(
                copy_core_bytes(
                    viem_core_copy_formatted_utf8,
                    &core,
                    document_state(&core).document_revision
                ),
                b"Text"
            );
            let deletion = ViemDeleteStyleV1 {
                struct_size: VIEM_DELETE_STYLE_V1_SIZE,
                namespace,
                identity: query(),
                style_id: utf8_slice(id.as_bytes()),
            };
            assert_eq!(
                unsafe { viem_core_view_delete_style(core.handle, view, &deletion, &mut outcome) },
                ViemStatus::Ok,
                "format {format}"
            );
            assert_eq!(
                unsafe { viem_core_view_delete_style(core.handle, view, &deletion, &mut outcome) },
                ViemStatus::StaleRevision
            );
            assert_eq!(
                unsafe { viem_core_view_undo(core.handle, view, &mut outcome) },
                ViemStatus::Ok
            );
            assert_eq!(
                copy_core_bytes(
                    viem_core_copy_source_bytes,
                    &core,
                    document_state(&core).document_revision
                ),
                created
            );
            assert_eq!(
                unsafe { viem_core_view_undo(core.handle, view, &mut outcome) },
                ViemStatus::Ok
            );
            assert_eq!(
                copy_core_bytes(
                    viem_core_copy_source_bytes,
                    &core,
                    document_state(&core).document_revision
                ),
                original
            );
        }
    }
}

#[test]
fn native_named_style_assignment_checks_selection_sheet_and_preserves_history() {
    use viem_core::document::*;
    let mut source = Document::from_bytes(
        br"{\rtf1{\stylesheet{\s0 Base;}{\s1\sbasedon0 Heading;}}Alpha\par Beta}{\*\unknown keep}".to_vec(),
        Encoding::Utf8,
        Format::Rtf,
    )
    .unwrap();
    source
        .apply_style_request(StyleModelRequest::new(
            source.id(),
            source.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                origin: StyleDefinitionOrigin::SourceBacked,
                edit: StyleDefinitionEdit::InsertCharacter {
                    style: CharacterStyle {
                        id: "RtfC1".into(),
                        based_on: None,
                        properties: CharacterProperties {
                            underline: Some(true),
                            ..Default::default()
                        },
                    },
                    metadata: StyleDefinitionMetadata {
                        display_name: "RtfC1".into(),
                        origin: StyleDefinitionOrigin::SourceBacked,
                    },
                },
            }),
        ))
        .unwrap();
    let original = source.source_bytes();
    let core = create_core(
        &original,
        ViemDocumentOptions {
            format: VIEM_FORMAT_RTF,
            ..Default::default()
        },
    );
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, provider.as_mut());
    let query = || {
        let mut sheet = ViemStyleSheetInfoV1::default();
        let mut selection = ViemLogicalSelectionIdentityV1::default();
        assert_eq!(
            unsafe { viem_core_style_sheet_info(core.handle, &mut sheet) },
            ViemStatus::Ok
        );
        assert_eq!(
            unsafe { viem_core_view_list_selection(core.handle, view, &mut selection) },
            ViemStatus::Ok
        );
        (sheet.identity, selection)
    };
    let (identity, expected_selection) = query();
    let heading = b"RtfP1";
    let request = ViemAssignStyleV1 {
        struct_size: VIEM_ASSIGN_STYLE_V1_SIZE,
        namespace: VIEM_STYLE_NAMESPACE_BLOCK,
        identity,
        expected_selection,
        style_id: ViemUtf8Slice {
            data: heading.as_ptr(),
            length: heading.len() as u64,
        },
    };
    let mut stale = request;
    stale.identity.style_sheet_revision += 1;
    assert_eq!(
        unsafe { viem_core_view_assign_style(core.handle, view, &stale, &mut outcome) },
        ViemStatus::StaleRevision
    );
    assert_eq!(
        unsafe { viem_core_view_assign_style(core.handle, view, &request, &mut outcome) },
        ViemStatus::Ok
    );
    let state = document_state(&core);
    let rewritten = copy_core_bytes(viem_core_copy_source_bytes, &core, state.document_revision);
    let projected = Document::from_bytes(rewritten.clone(), Encoding::Utf8, Format::Rtf).unwrap();
    assert_eq!(projected.projection().blocks()[0].style.0, "RtfP1");
    assert_eq!(projected.text(), "Alpha\nBeta");
    assert_eq!(
        unsafe { viem_core_view_assign_style(core.handle, view, &request, &mut outcome) },
        ViemStatus::StaleRevision
    );
    assert_eq!(
        unsafe { viem_core_view_undo(core.handle, view, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        copy_core_bytes(
            viem_core_copy_source_bytes,
            &core,
            document_state(&core).document_revision
        ),
        original
    );

    for character in ['v', 'l'] {
        assert_eq!(
            unsafe {
                test_send_key(
                    core.handle,
                    view,
                    &key(VIEM_KEY_CHARACTER, character as u32),
                    &mut outcome,
                )
            },
            ViemStatus::Ok
        );
    }
    let (identity, expected_selection) = query();
    let accent = b"RtfC1";
    let request = ViemAssignStyleV1 {
        namespace: VIEM_STYLE_NAMESPACE_CHARACTER,
        identity,
        expected_selection,
        style_id: ViemUtf8Slice {
            data: accent.as_ptr(),
            length: accent.len() as u64,
        },
        ..request
    };
    assert_eq!(
        unsafe {
            test_send_key(core.handle, view, &key(VIEM_KEY_CHARACTER, 'l' as u32), &mut outcome)
        },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_assign_style(core.handle, view, &request, &mut outcome) },
        ViemStatus::StaleRevision
    );
    let (_, expected_selection) = query();
    let request = ViemAssignStyleV1 {
        expected_selection,
        ..request
    };
    assert_eq!(
        unsafe { viem_core_view_assign_style(core.handle, view, &request, &mut outcome) },
        ViemStatus::Ok
    );
    let rewritten = copy_core_bytes(
        viem_core_copy_source_bytes,
        &core,
        document_state(&core).document_revision,
    );
    let projected = Document::from_bytes(rewritten, Encoding::Utf8, Format::Rtf).unwrap();
    assert!(projected.projection().style_spans().iter().any(|span|
        span.range == (0..3) && span.application == StyleApplication::Named("RtfC1".into())));
    assert_eq!(
        unsafe { viem_core_view_undo(core.handle, view, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        copy_core_bytes(
            viem_core_copy_source_bytes,
            &core,
            document_state(&core).document_revision
        ),
        original
    );
}

#[test]
fn native_paragraph_style_request_is_revision_and_selection_checked() {
    let core = create_core(
        b"Heading\nBody",
        ViemDocumentOptions {
            format: VIEM_FORMAT_MARKDOWN_SOURCE,
            ..Default::default()
        },
    );
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, _) = add_test_view(&core, provider.as_mut());
    let mut selection = ViemLogicalSelectionIdentityV1::default();
    assert_eq!(
        unsafe { viem_core_view_list_selection(core.handle, view, &mut selection) },
        ViemStatus::Ok
    );
    let mut outcome = ViemCoreOutcomeV1::default();
    let request = ViemSetParagraphStyleV1 {
        struct_size: VIEM_SET_PARAGRAPH_STYLE_V1_SIZE,
        level: 2,
        expected_selection: selection,
    };
    assert_eq!(
        unsafe { viem_core_view_set_paragraph_style(core.handle, view, &request, &mut outcome) },
        ViemStatus::Ok
    );
    let state = document_state(&core);
    assert_eq!(
        copy_core_bytes(
            viem_core_copy_formatted_utf8,
            &core,
            state.document_revision
        ),
        b"## Heading\nBody"
    );
    assert_eq!(
        unsafe { viem_core_view_set_paragraph_style(core.handle, view, &request, &mut outcome) },
        ViemStatus::StaleRevision
    );
    let invalid = ViemSetParagraphStyleV1 {
        level: 7,
        ..request
    };
    assert_eq!(
        unsafe { viem_core_view_set_paragraph_style(core.handle, view, &invalid, &mut outcome) },
        ViemStatus::InvalidArgument
    );
}

#[test]
fn native_format_encoding_and_list_requests_validate_exact_identity() {
    let core = create_core(b"__alpha__\n\nbeta", ViemDocumentOptions::default());
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, _) = add_test_view(&core, provider.as_mut());
    let state = document_state(&core);
    let mut outcome = ViemCoreOutcomeV1::default();
    let format = ViemSetFormatV1 {
        struct_size: VIEM_SET_FORMAT_V1_SIZE,
        format: VIEM_FORMAT_MARKDOWN,
        operation: VIEM_FORMAT_OPERATION_REINTERPRET,
        reserved: 0,
        document_id: state.document_id,
        document_revision: state.document_revision,
    };
    assert_eq!(
        unsafe { test_set_format(core.handle, view, &format, &mut outcome) },
        ViemStatus::Ok
    );
    let changed = document_state(&core);
    assert_eq!(changed.format, VIEM_FORMAT_MARKDOWN);
    assert_eq!(
        copy_core_bytes(
            viem_core_copy_source_bytes,
            &core,
            changed.document_revision
        ),
        b"__alpha__\n\nbeta"
    );
    assert_eq!(
        copy_core_bytes(
            viem_core_copy_formatted_utf8,
            &core,
            changed.document_revision
        ),
        b"alpha\nbeta"
    );
    assert_eq!(
        unsafe { test_set_format(core.handle, view, &format, &mut outcome) },
        ViemStatus::StaleRevision
    );
    let encoding = ViemSetEncodingV1 {
        struct_size: VIEM_SET_ENCODING_V1_SIZE,
        encoding: VIEM_ENCODING_UTF16_LE,
        document_id: changed.document_id,
        document_revision: changed.document_revision,
    };
    assert_eq!(
        unsafe { test_set_encoding(core.handle, view, &encoding, &mut outcome) },
        ViemStatus::Ok
    );
    let changed = document_state(&core);
    assert_eq!(changed.encoding, VIEM_ENCODING_UTF16_LE);
    let mut selection = ViemLogicalSelectionIdentityV1::default();
    assert_eq!(
        unsafe { viem_core_view_list_selection(core.handle, view, &mut selection) },
        ViemStatus::Ok
    );
    assert_eq!(selection.kind, VIEM_LOGICAL_SELECTION_KIND_NONE);
    let list = ViemSetListStyleV1 {
        struct_size: VIEM_SET_LIST_STYLE_V1_SIZE,
        style: VIEM_LIST_STYLE_BULLET,
        expected_selection: selection,
    };
    assert_eq!(
        unsafe { viem_core_view_set_list_style(core.handle, view, &list, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_set_list_style(core.handle, view, &list, &mut outcome) },
        ViemStatus::StaleRevision
    );
    assert_eq!(
        unsafe { test_set_format(core.handle, view, ptr::null(), &mut outcome) },
        ViemStatus::NullPointer
    );
}

#[test]
fn document_state_tracks_pipeline_history_file_format_and_native_save_point() {
    let source = "\u{feff}**a**\r\n"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let core = create_core(
        &source,
        ViemDocumentOptions {
            encoding: VIEM_ENCODING_UTF16_LE,
            format: VIEM_FORMAT_MARKDOWN,
            file_format: VIEM_FILE_FORMAT_DETECT,
            ..ViemDocumentOptions::default()
        },
    );
    let initial = document_state(&core);
    assert_eq!(initial.struct_size, VIEM_DOCUMENT_STATE_V1_SIZE);
    assert_ne!(initial.document_id, 0);
    assert_eq!(initial.document_revision, 0);
    assert_eq!(initial.source_byte_count, source.len() as u64);
    assert_eq!(initial.encoding, VIEM_ENCODING_UTF16_LE);
    assert_eq!(initial.format, VIEM_FORMAT_MARKDOWN);
    assert_eq!(initial.file_format, VIEM_FILE_FORMAT_DOS);
    assert_eq!(initial.file_format_origin, VIEM_FILE_FORMAT_ORIGIN_DETECTED);
    assert_ne!(initial.flags & VIEM_DOCUMENT_STATE_HAS_BOM, 0);
    assert_eq!(
        initial.flags
            & (VIEM_DOCUMENT_STATE_CAN_UNDO
                | VIEM_DOCUMENT_STATE_CAN_REDO
                | VIEM_DOCUMENT_STATE_IS_DIRTY),
        0
    );
    assert_eq!(
        initial.undo_action_category,
        VIEM_HISTORY_ACTION_CATEGORY_NONE
    );
    assert_eq!(
        initial.redo_action_category,
        VIEM_HISTORY_ACTION_CATEGORY_NONE
    );
    assert_eq!(initial.reserved, [0; 2]);

    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);
    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, 'i' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { test_send_text(core.handle, view, b"x".as_ptr(), 1, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe {
            test_send_key(core.handle, view, &key(VIEM_KEY_ESCAPE, 0), &mut outcome)
        },
        ViemStatus::Ok
    );
    let edited = document_state(&core);
    assert_eq!(edited.document_revision, 1);
    assert_eq!(edited.source_byte_count, source.len() as u64 + 2);
    assert_eq!(edited.style_sheet_revision, initial.style_sheet_revision);
    assert_eq!(edited.encoding, initial.encoding);
    assert_eq!(edited.format, initial.format);
    assert_ne!(edited.flags & VIEM_DOCUMENT_STATE_HAS_BOM, 0);
    assert_ne!(edited.flags & VIEM_DOCUMENT_STATE_CAN_UNDO, 0);
    assert_ne!(edited.flags & VIEM_DOCUMENT_STATE_IS_DIRTY, 0);
    assert_eq!(
        edited.undo_action_category,
        VIEM_HISTORY_ACTION_CATEGORY_TEXT
    );
    let mut viewport_before_save = ViemViewportStateV1::default();
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, view, &mut viewport_before_save) },
        ViemStatus::Ok
    );

    let saved = ViemMarkSavedV1 {
        document_id: edited.document_id,
        document_revision: edited.document_revision,
        ..ViemMarkSavedV1::default()
    };
    assert_eq!(
        unsafe { viem_core_mark_saved(core.handle, &saved) },
        ViemStatus::Ok
    );
    let clean = document_state(&core);
    let mut viewport_after_save = ViemViewportStateV1::default();
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, view, &mut viewport_after_save) },
        ViemStatus::Ok
    );
    assert_eq!(clean.document_revision, edited.document_revision);
    assert_eq!(viewport_after_save, viewport_before_save);
    assert_eq!(clean.flags & VIEM_DOCUMENT_STATE_IS_DIRTY, 0);
    assert_ne!(clean.flags & VIEM_DOCUMENT_STATE_CAN_UNDO, 0);

    let format_request = ViemSetFileFormatV1 {
        file_format: VIEM_FILE_FORMAT_UNIX,
        document_id: clean.document_id,
        document_revision: clean.document_revision,
        ..ViemSetFileFormatV1::default()
    };
    assert_eq!(
        unsafe { viem_core_view_set_file_format(core.handle, view, &format_request, &mut outcome) },
        ViemStatus::Ok
    );
    let converted = document_state(&core);
    assert_eq!(converted.style_sheet_revision, initial.style_sheet_revision);
    assert_eq!(converted.encoding, initial.encoding);
    assert_eq!(converted.format, initial.format);
    assert_ne!(converted.flags & VIEM_DOCUMENT_STATE_HAS_BOM, 0);
    assert_eq!(converted.file_format, VIEM_FILE_FORMAT_UNIX);
    assert_eq!(converted.file_format_origin, VIEM_FILE_FORMAT_ORIGIN_FORCED);
    assert_ne!(converted.flags & VIEM_DOCUMENT_STATE_IS_DIRTY, 0);
    assert_eq!(
        converted.undo_action_category,
        VIEM_HISTORY_ACTION_CATEGORY_FILE_FORMAT
    );

    assert_eq!(
        unsafe { viem_core_view_undo(core.handle, view, &mut outcome) },
        ViemStatus::Ok
    );
    let undone = document_state(&core);
    assert_eq!(undone.style_sheet_revision, initial.style_sheet_revision);
    assert_eq!(undone.file_format, VIEM_FILE_FORMAT_DOS);
    assert_eq!(undone.file_format_origin, VIEM_FILE_FORMAT_ORIGIN_DETECTED);
    assert_eq!(undone.flags & VIEM_DOCUMENT_STATE_IS_DIRTY, 0);
    assert_ne!(undone.flags & VIEM_DOCUMENT_STATE_CAN_REDO, 0);
    assert_eq!(
        undone.redo_action_category,
        VIEM_HISTORY_ACTION_CATEGORY_FILE_FORMAT
    );
    assert_eq!(
        unsafe { viem_core_view_redo(core.handle, view, &mut outcome) },
        ViemStatus::Ok
    );

    let stale_save = ViemMarkSavedV1 {
        document_id: converted.document_id,
        document_revision: converted.document_revision,
        ..ViemMarkSavedV1::default()
    };
    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, 'i' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { test_send_text(core.handle, view, b"y".as_ptr(), 1, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_mark_saved(core.handle, &stale_save) },
        ViemStatus::StaleRevision
    );
    assert_eq!(
        unsafe { test_send_text(core.handle, view, b"z".as_ptr(), 1, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe {
            test_send_key(core.handle, view, &key(VIEM_KEY_ESCAPE, 0), &mut outcome)
        },
        ViemStatus::Ok
    );
    let grouped = document_state(&core);
    assert_eq!(
        grouped.undo_action_category,
        VIEM_HISTORY_ACTION_CATEGORY_TEXT
    );
    assert_ne!(grouped.flags & VIEM_DOCUMENT_STATE_IS_DIRTY, 0);
    assert_eq!(
        unsafe { viem_core_view_undo(core.handle, view, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_redo(core.handle, view, &mut outcome) },
        ViemStatus::Ok
    );
    let current = document_state(&core);
    let exact_save = ViemMarkSavedV1 {
        document_id: current.document_id,
        document_revision: current.document_revision,
        ..ViemMarkSavedV1::default()
    };
    assert_eq!(
        unsafe { viem_core_mark_saved(core.handle, &exact_save) },
        ViemStatus::Ok
    );
    assert_eq!(
        document_state(&core).flags & VIEM_DOCUMENT_STATE_IS_DIRTY,
        0
    );

    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
}

#[test]
fn format_operations_validate_policy_and_keep_reinterpretation_byte_exact() {
    let original = b"**First**\n\nSecond";
    for operation in [
        VIEM_FORMAT_OPERATION_REINTERPRET,
        VIEM_FORMAT_OPERATION_CONVERT,
    ] {
        let core = create_core(
            original,
            ViemDocumentOptions {
                format: VIEM_FORMAT_MARKDOWN_SOURCE,
                ..ViemDocumentOptions::default()
            },
        );
        let mut provider = Box::new(FakeProviderContext::new(core.handle));
        let (view, _) = add_test_view(&core, provider.as_mut());
        let before = document_state(&core);
        let request = ViemSetFormatV1 {
            struct_size: VIEM_SET_FORMAT_V1_SIZE,
            format: VIEM_FORMAT_PLAIN_TEXT,
            operation,
            reserved: 0,
            document_id: before.document_id,
            document_revision: before.document_revision,
        };
        let mut outcome = ViemCoreOutcomeV1::default();
        for invalid in [
            ViemSetFormatV1 {
                operation: 99,
                ..request
            },
            ViemSetFormatV1 {
                reserved: 1,
                ..request
            },
            ViemSetFormatV1 {
                struct_size: VIEM_SET_FORMAT_V1_SIZE - 1,
                ..request
            },
        ] {
            assert_eq!(
                unsafe { test_set_format(core.handle, view, &invalid, &mut outcome) },
                ViemStatus::InvalidArgument
            );
            assert_eq!(document_state(&core), before);
            assert_eq!(
                copy_core_bytes(viem_core_copy_source_bytes, &core, before.document_revision),
                original
            );
        }
        assert_eq!(
            unsafe { test_set_format(core.handle, view, &request, &mut outcome) },
            ViemStatus::Ok
        );
        let after = document_state(&core);
        assert_eq!(after.format, VIEM_FORMAT_PLAIN_TEXT);
        let expected: &[u8] = if operation == VIEM_FORMAT_OPERATION_REINTERPRET {
            original
        } else {
            b"First\n\nSecond"
        };
        assert_eq!(
            copy_core_bytes(viem_core_copy_source_bytes, &core, after.document_revision),
            expected
        );
        assert_eq!(
            copy_core_bytes(
                viem_core_copy_formatted_utf8,
                &core,
                after.document_revision
            ),
            expected
        );
        assert_eq!(
            unsafe { viem_core_view_undo(core.handle, view, &mut outcome) },
            ViemStatus::Ok
        );
        let restored = document_state(&core);
        assert_eq!(restored.format, before.format);
        assert_eq!(
            copy_core_bytes(
                viem_core_copy_source_bytes,
                &core,
                restored.document_revision
            ),
            original
        );
        assert_eq!(
            unsafe { viem_core_view_redo(core.handle, view, &mut outcome) },
            ViemStatus::Ok
        );
        let redone = document_state(&core);
        assert_eq!(redone.format, after.format);
        assert_eq!(
            copy_core_bytes(viem_core_copy_source_bytes, &core, redone.document_revision),
            expected
        );
    }
}

#[test]
fn typed_view_options_are_local_or_shared_and_fail_atomically() {
    let core = create_core(
        b"a\rb\n",
        ViemDocumentOptions {
            file_format: VIEM_FILE_FORMAT_UNIX,
            ..ViemDocumentOptions::default()
        },
    );
    let mut first_context = Box::new(FakeProviderContext::new(core.handle));
    let mut second_context = Box::new(FakeProviderContext::new(core.handle));
    let (first, mut outcome) = add_test_view(&core, &mut *first_context);
    let (second, _) = add_test_view(&core, &mut *second_context);
    assert_eq!(
        unsafe { viem_core_view_set_wrap(core.handle, first, 0, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_set_wrap(core.handle, second, 0, &mut outcome) },
        ViemStatus::Ok
    );

    let mut first_state = ViemViewportStateV1::default();
    let mut second_state = ViemViewportStateV1::default();
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, first, &mut first_state) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, second, &mut second_state) },
        ViemStatus::Ok
    );
    assert_ne!(first_state.flags & VIEM_VIEWPORT_STATE_LINEBREAK, 0);
    assert_ne!(second_state.flags & VIEM_VIEWPORT_STATE_LINEBREAK, 0);
    assert_eq!(first_state.scale, 1.0);
    assert_eq!(second_state.scale, 1.0);

    let document_revision = outcome.document_revision;
    let first_configuration = first_state.configuration_generation;
    let second_configuration = second_state.configuration_generation;
    let first_shape_calls = first_context.shape_calls;
    let second_shape_calls = second_context.shape_calls;
    assert_eq!(
        unsafe { viem_core_view_set_scale(core.handle, first, 1.5, &mut outcome) },
        ViemStatus::Ok
    );
    assert_ne!(outcome.flags & VIEM_OUTCOME_LAYOUT_CHANGED, 0);
    assert_eq!(outcome.document_revision, document_revision);
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, first, &mut first_state) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, second, &mut second_state) },
        ViemStatus::Ok
    );
    assert_eq!(first_state.scale, 1.5);
    assert_eq!(second_state.scale, 1.0);
    assert!(first_state.configuration_generation > first_configuration);
    assert_eq!(second_state.configuration_generation, second_configuration);
    assert_eq!(first_context.last_requested_scale, 1.5);
    assert_eq!(second_context.last_requested_scale, 1.0);
    assert!(first_context.shape_calls > first_shape_calls);
    assert_eq!(second_context.shape_calls, second_shape_calls);

    let scaled_state = first_state;
    for invalid in [0.0, -1.0, 0.249, 5.001, f32::NAN, f32::INFINITY] {
        outcome = ViemCoreOutcomeV1 {
            flags: u32::MAX,
            ..ViemCoreOutcomeV1::default()
        };
        assert_eq!(
            unsafe { viem_core_view_set_scale(core.handle, first, invalid, &mut outcome) },
            ViemStatus::InvalidArgument
        );
        assert_eq!(outcome, ViemCoreOutcomeV1::default());
        assert_eq!(
            unsafe { viem_core_view_viewport_state(core.handle, first, &mut first_state) },
            ViemStatus::Ok
        );
        assert_eq!(first_state, scaled_state);
    }

    let before_compatibility_call = first_state;
    assert_eq!(
        unsafe { viem_core_view_set_linebreak(core.handle, first, 1, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(outcome.flags & VIEM_OUTCOME_LAYOUT_CHANGED, 0);
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, first, &mut first_state) },
        ViemStatus::Ok
    );
    assert_eq!(first_state, before_compatibility_call);
    assert_eq!(
        unsafe { viem_core_view_set_linebreak(core.handle, first, 0, &mut outcome) },
        ViemStatus::InvalidArgument
    );
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, first, &mut first_state) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, second, &mut second_state) },
        ViemStatus::Ok
    );
    assert_ne!(first_state.flags & VIEM_VIEWPORT_STATE_LINEBREAK, 0);
    assert_ne!(second_state.flags & VIEM_VIEWPORT_STATE_LINEBREAK, 0);
    assert_eq!(
        unsafe { viem_core_view_set_linebreak(core.handle, first, 2, &mut outcome) },
        ViemStatus::InvalidArgument
    );
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, first, &mut first_state) },
        ViemStatus::Ok
    );
    assert_ne!(first_state.flags & VIEM_VIEWPORT_STATE_LINEBREAK, 0);

    let before = document_state(&core);
    let invalid = ViemSetFileFormatV1 {
        file_format: VIEM_FILE_FORMAT_DETECT,
        document_id: before.document_id,
        document_revision: before.document_revision,
        ..ViemSetFileFormatV1::default()
    };
    assert_eq!(
        unsafe { viem_core_view_set_file_format(core.handle, first, &invalid, &mut outcome) },
        ViemStatus::InvalidFileFormat
    );
    let stale = ViemSetFileFormatV1 {
        file_format: VIEM_FILE_FORMAT_DOS,
        document_id: before.document_id,
        document_revision: before.document_revision.saturating_add(1),
        ..ViemSetFileFormatV1::default()
    };
    assert_eq!(
        unsafe { viem_core_view_set_file_format(core.handle, first, &stale, &mut outcome) },
        ViemStatus::StaleRevision
    );
    let wrong_document = ViemSetFileFormatV1 {
        file_format: VIEM_FILE_FORMAT_DOS,
        document_id: before.document_id.saturating_add(1),
        document_revision: before.document_revision,
        ..ViemSetFileFormatV1::default()
    };
    assert_eq!(
        unsafe {
            viem_core_view_set_file_format(core.handle, first, &wrong_document, &mut outcome)
        },
        ViemStatus::InvalidArgument
    );
    let rejected = ViemSetFileFormatV1 {
        file_format: VIEM_FILE_FORMAT_MAC,
        document_id: before.document_id,
        document_revision: before.document_revision,
        ..ViemSetFileFormatV1::default()
    };
    assert_eq!(
        unsafe { viem_core_view_set_file_format(core.handle, second, &rejected, &mut outcome) },
        ViemStatus::PolicyRequired
    );
    assert_eq!(document_state(&core), before);

    let valid = ViemSetFileFormatV1 {
        file_format: VIEM_FILE_FORMAT_DOS,
        document_id: before.document_id,
        document_revision: before.document_revision,
        ..ViemSetFileFormatV1::default()
    };
    assert_eq!(
        unsafe { viem_core_view_set_file_format(core.handle, second, &valid, &mut outcome) },
        ViemStatus::Ok
    );
    assert_ne!(outcome.flags & VIEM_OUTCOME_DOCUMENT_CHANGED, 0);
    let shared = document_state(&core);
    assert_eq!(shared.file_format, VIEM_FILE_FORMAT_DOS);
    assert_eq!(
        shared.undo_action_category,
        VIEM_HISTORY_ACTION_CATEGORY_FILE_FORMAT
    );
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, first, &mut first_state) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, second, &mut second_state) },
        ViemStatus::Ok
    );
    assert_eq!(first_state.document_revision, shared.document_revision);
    assert_eq!(second_state.document_revision, shared.document_revision);
    assert_ne!(first_state.flags & VIEM_VIEWPORT_STATE_LINEBREAK, 0);
    assert_ne!(second_state.flags & VIEM_VIEWPORT_STATE_LINEBREAK, 0);

    assert_eq!(viem_core_view_remove(core.handle, first), ViemStatus::Ok);
    assert_eq!(viem_core_view_remove(core.handle, second), ViemStatus::Ok);
}

#[test]
fn zoom_reflows_only_a_bounded_viewport_of_a_large_document() {
    let source = "short proportional line\n".repeat(100_000);
    let core = create_core(source.as_bytes(), ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);
    let mut before = ViemViewportStateV1::default();
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, view, &mut before) },
        ViemStatus::Ok
    );

    context.shaped_bytes = 0;
    context.minimum_request_start = u64::MAX;
    context.maximum_request_end = 0;
    assert_eq!(
        unsafe { viem_core_view_set_scale(core.handle, view, 1.25, &mut outcome) },
        ViemStatus::Ok
    );

    let mut after = ViemViewportStateV1::default();
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, view, &mut after) },
        ViemStatus::Ok
    );
    assert_eq!(after.scale, 1.25);
    assert!(after.configuration_generation > before.configuration_generation);
    assert_eq!(after.document_revision, before.document_revision);
    assert!(context.shaped_bytes < 10_000);
    assert!(context.maximum_request_end < source.len() as u64);

    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
}

#[test]
fn layout_snapshot_export_geometry_hit_testing_and_pointer_placement_are_revision_bound() {
    let core = create_core(b"abc\nfi", ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);

    let mut info = ViemLayoutSnapshotInfoV1::default();
    assert_eq!(
        unsafe { viem_core_view_layout_snapshot_info(core.handle, view, &mut info) },
        ViemStatus::Ok
    );
    assert_eq!(info.struct_size, VIEM_LAYOUT_SNAPSHOT_INFO_V1_SIZE);
    assert_eq!(
        info.identity.struct_size,
        VIEM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE
    );
    assert_eq!(info.identity.view_id, view);
    assert_eq!(info.identity.document_revision, 0);
    assert_ne!(info.identity.layout_revision, 0);
    assert!(info.row_count >= 2);
    assert!(info.cluster_count >= 4, "fi may be one shaping cluster");
    assert!(info.caret_count >= info.cluster_count);
    assert!(info.total_height > 0.0);
    assert!(info.coverage_y_end > info.coverage_y_start);

    let mut queried = ViemLayoutSnapshotInfoV1::default();
    assert_eq!(
        unsafe {
            viem_core_view_copy_layout_snapshot(
                core.handle,
                view,
                &info.identity,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                0,
                &mut queried,
            )
        },
        ViemStatus::BufferTooSmall
    );
    assert_eq!(queried, info);

    let mut wrong_view = info.identity;
    wrong_view.view_id += 1;
    assert_eq!(
        unsafe {
            viem_core_view_copy_layout_snapshot(
                core.handle,
                view,
                &wrong_view,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                0,
                &mut queried,
            )
        },
        ViemStatus::StaleRevision
    );

    let mut rows = vec![ViemVisualRowV1::default(); info.row_count as usize];
    let mut clusters = vec![ViemPositionedClusterV1::default(); info.cluster_count as usize];
    let mut carets = vec![ViemPositionedCaretV1::default(); info.caret_count as usize];
    let mut copied = ViemLayoutSnapshotInfoV1::default();
    rows[0].row_index = u64::MAX;
    clusters[0].text_start = u64::MAX;
    carets[0].text_offset = u64::MAX;
    assert_eq!(
        unsafe {
            viem_core_view_copy_layout_snapshot(
                core.handle,
                view,
                &ViemLayoutSnapshotIdentityV1 {
                    struct_size: VIEM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE + 8,
                    ..info.identity
                },
                rows.as_mut_ptr(),
                rows.len().saturating_sub(1) as u64,
                clusters.as_mut_ptr(),
                clusters.len() as u64,
                carets.as_mut_ptr(),
                carets.len() as u64,
                &mut copied,
            )
        },
        ViemStatus::BufferTooSmall
    );
    assert_eq!(copied, info);
    assert_eq!(rows[0].row_index, u64::MAX, "rows must not be partial");
    assert_eq!(
        clusters[0].text_start,
        u64::MAX,
        "clusters must not be partial"
    );
    assert_eq!(
        carets[0].text_offset,
        u64::MAX,
        "carets must not be partial"
    );
    assert_eq!(
        unsafe {
            viem_core_view_copy_layout_snapshot(
                core.handle,
                view,
                &info.identity,
                rows.as_mut_ptr(),
                rows.len() as u64,
                clusters.as_mut_ptr(),
                clusters.len() as u64,
                carets.as_mut_ptr(),
                carets.len() as u64,
                &mut copied,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(copied, info);
    for (row_index, row) in rows.iter().enumerate() {
        assert_eq!(row.struct_size, VIEM_VISUAL_ROW_V1_SIZE);
        assert_eq!(row.row_index, row_index as u64);
        assert!(row.text_start <= row.text_end);
        assert!(row.hard_line_start <= row.text_start);
        assert!(row.text_end <= row.hard_line_end);
        assert!(row.baseline >= row.y);
        assert_eq!(row.line_advance, row.ascent + row.descent + row.leading);
        assert!((row.first_cluster + row.cluster_count) <= info.cluster_count);
        assert!((row.first_caret + row.caret_count) <= info.caret_count);
    }
    for cluster in &clusters {
        assert_eq!(cluster.struct_size, VIEM_POSITIONED_CLUSTER_V1_SIZE);
        assert!(cluster.text_start < cluster.text_end);
        assert!(cluster.advance > 0.0);
        assert_ne!(cluster.flags & VIEM_POSITIONED_CLUSTER_HAS_RENDER_RUN, 0);
        assert_eq!(cluster.render_run.owner, 0xf00d);
        assert_eq!(
            cluster.render_run.metrics_generation,
            info.identity.metrics_generation
        );
    }

    let row = rows.first().unwrap();
    let logical = carets
        .iter()
        .find(|caret| caret.text_offset == 1 && caret.affinity == VIEM_BOUNDARY_AFFINITY_DOWNSTREAM)
        .copied()
        .unwrap();
    let caret_request = ViemLayoutCaretRequestV1 {
        struct_size: VIEM_LAYOUT_CARET_REQUEST_V1_SIZE,
        affinity: logical.affinity,
        identity: info.identity,
        text_offset: logical.text_offset,
    };
    let mut geometry = ViemLayoutCaretGeometryV1::default();
    assert_eq!(
        unsafe { viem_core_view_caret_geometry(core.handle, view, &caret_request, &mut geometry) },
        ViemStatus::Ok
    );
    assert_eq!(geometry.struct_size, VIEM_LAYOUT_CARET_GEOMETRY_V1_SIZE);
    assert_eq!(geometry.point.text_offset, logical.text_offset);
    assert_eq!(geometry.point.affinity, logical.affinity);
    assert_eq!(geometry.rect.x, logical.x);
    assert_eq!(geometry.rect.y, row.y);
    assert_eq!(geometry.rect.height, row.ascent + row.descent);

    let hit_request = ViemLayoutHitTestRequestV1 {
        struct_size: VIEM_LAYOUT_HIT_TEST_REQUEST_V1_SIZE,
        reserved: 0,
        identity: info.identity,
        x: logical.x,
        y: row.y + 1.0,
    };
    let mut hit = ViemLayoutCaretPointV1::default();
    assert_eq!(
        unsafe { viem_core_view_layout_hit_test(core.handle, view, &hit_request, &mut hit) },
        ViemStatus::Ok
    );
    assert_eq!(hit.struct_size, VIEM_LAYOUT_CARET_POINT_V1_SIZE);
    assert_eq!(hit.document_id, info.identity.document_id);
    assert_eq!(hit.document_revision, info.identity.document_revision);
    assert_eq!(hit.layout_revision, info.identity.layout_revision);

    let place = ViemPlaceCursorV1 {
        struct_size: VIEM_PLACE_CURSOR_V1_SIZE,
        flags: 0,
        document_revision: hit.document_revision,
        text_offset: hit.text_offset,
        affinity: hit.affinity,
        reserved: 0,
    };
    assert_eq!(
        unsafe { viem_core_view_place_cursor(core.handle, view, &place, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(outcome.cursor_utf8_offset, hit.text_offset);
    let mut presentation = ViemViewPresentationV1::default();
    assert_eq!(
        unsafe { viem_core_view_presentation(core.handle, view, &mut presentation) },
        ViemStatus::Ok
    );
    assert_eq!(presentation.cursor_utf8_offset, hit.text_offset);
    assert_eq!(presentation.cursor_affinity, hit.affinity);

    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, 'v' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    let extend = ViemPlaceCursorV1 {
        flags: VIEM_PLACE_CURSOR_EXTEND_SELECTION,
        text_offset: 2,
        affinity: VIEM_BOUNDARY_AFFINITY_DOWNSTREAM,
        ..place
    };
    assert_eq!(
        unsafe { viem_core_view_place_cursor(core.handle, view, &extend, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_presentation(core.handle, view, &mut presentation) },
        ViemStatus::Ok
    );
    assert_eq!(presentation.mode, VIEM_MODE_VISUAL_CHARACTER);
    assert_ne!(
        presentation.flags & VIEM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR,
        0
    );
    assert_eq!(presentation.visual_anchor_utf8_offset, hit.text_offset);
    assert_eq!(presentation.cursor_utf8_offset, 2);

    let stale_identity = info.identity;
    assert_eq!(
        unsafe { viem_core_view_resize(core.handle, view, 300.0, 120.0, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe {
            viem_core_view_copy_layout_snapshot(
                core.handle,
                view,
                &stale_identity,
                rows.as_mut_ptr(),
                rows.len() as u64,
                clusters.as_mut_ptr(),
                clusters.len() as u64,
                carets.as_mut_ptr(),
                carets.len() as u64,
                &mut copied,
            )
        },
        ViemStatus::StaleRevision
    );

    context.metrics_generation += 1;
    assert_eq!(
        unsafe { viem_core_view_layout_snapshot_info(core.handle, view, &mut info) },
        ViemStatus::LayoutUnavailable
    );

    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
}

#[test]
fn layout_paint_export_is_exact_revision_bound_and_uses_explicit_rgba_flags() {
    let core = create_core(b"paint", ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);

    let mut paint = ViemLayoutPaintInfoV1::default();
    assert_eq!(
        unsafe { viem_core_view_layout_paint_info(core.handle, view, &mut paint) },
        ViemStatus::Ok
    );
    assert_eq!(paint.struct_size, VIEM_LAYOUT_PAINT_INFO_V1_SIZE);
    assert_eq!(
        paint.identity.struct_size,
        VIEM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE
    );
    assert_eq!(paint.identity.view_id, view);
    assert_eq!(
        paint.canvas_background,
        ViemRgbaV1 {
            red: 1.0,
            green: 1.0,
            blue: 1.0,
            alpha: 1.0,
        }
    );
    assert_eq!(paint.default_paint.struct_size, VIEM_TEXT_PAINT_V1_SIZE);
    assert_eq!(
        paint.default_paint.flags,
        VIEM_TEXT_PAINT_DEFAULT_FOREGROUND
    );
    assert_eq!(paint.flags, VIEM_LAYOUT_PAINT_DEFAULT_CANVAS);
    assert_eq!(
        paint.default_paint.foreground,
        ViemRgbaV1 {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 1.0,
        }
    );
    assert_eq!(paint.default_paint.background, ViemRgbaV1::default());
    assert_eq!(paint.paint_run_count, 0);

    let mut copied = ViemLayoutPaintInfoV1::default();
    assert_eq!(
        unsafe {
            viem_core_view_copy_layout_paint(
                core.handle,
                view,
                &paint.identity,
                ptr::null_mut(),
                0,
                &mut copied,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(copied, paint);

    let mut wrong_view = paint.identity;
    wrong_view.view_id += 1;
    assert_eq!(
        unsafe {
            viem_core_view_copy_layout_paint(
                core.handle,
                view,
                &wrong_view,
                ptr::null_mut(),
                0,
                &mut copied,
            )
        },
        ViemStatus::StaleRevision
    );
    assert_eq!(copied, ViemLayoutPaintInfoV1::default());

    let stale = paint.identity;
    assert_eq!(
        unsafe { viem_core_view_resize(core.handle, view, 320.0, 180.0, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe {
            viem_core_view_copy_layout_paint(
                core.handle,
                view,
                &stale,
                ptr::null_mut(),
                0,
                &mut copied,
            )
        },
        ViemStatus::StaleRevision
    );

    context.metrics_generation += 1;
    assert_eq!(
        unsafe { viem_core_view_layout_paint_info(core.handle, view, &mut paint) },
        ViemStatus::LayoutUnavailable
    );
    assert_eq!(paint, ViemLayoutPaintInfoV1::default());

    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
}

#[test]
fn command_line_export_is_exact_kind_cursor_and_utf8_state() {
    let core = create_core(b"text", ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);

    let mut info = ViemCommandLineInfoV1::default();
    assert_eq!(
        unsafe { viem_core_view_command_line_info(core.handle, view, &mut info) },
        ViemStatus::Ok
    );
    assert_eq!(info.struct_size, VIEM_COMMAND_LINE_INFO_V1_SIZE);
    assert_eq!(info.identity.kind, VIEM_COMMAND_LINE_KIND_NONE);
    assert_eq!(info.identity.view_id, view);
    assert_eq!(info.identity.document_revision, 0);
    assert_eq!(info.utf8_length, 0);
    let mut copied_info = ViemCommandLineInfoV1::default();
    assert_eq!(
        unsafe {
            viem_core_view_copy_command_line(
                core.handle,
                view,
                &info.identity,
                ptr::null_mut(),
                0,
                &mut copied_info,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(copied_info, info);

    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, ':' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    let command = "set cafés";
    assert_eq!(
        unsafe {
            test_send_text(
                core.handle,
                view,
                command.as_ptr(),
                command.len() as u64,
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_command_line_info(core.handle, view, &mut info) },
        ViemStatus::Ok
    );
    assert_eq!(info.identity.kind, VIEM_COMMAND_LINE_KIND_EX);
    assert_eq!(info.utf8_length, command.len() as u64);
    assert_eq!(info.cursor_utf8_offset, command.len() as u64);

    let mut bytes = vec![0xa5; command.len()];
    assert_eq!(
        unsafe {
            viem_core_view_copy_command_line(
                core.handle,
                view,
                &info.identity,
                bytes.as_mut_ptr(),
                bytes.len().saturating_sub(1) as u64,
                &mut copied_info,
            )
        },
        ViemStatus::BufferTooSmall
    );
    assert_eq!(copied_info, info);
    assert!(bytes.iter().all(|byte| *byte == 0xa5));
    assert_eq!(
        unsafe {
            viem_core_view_copy_command_line(
                core.handle,
                view,
                &info.identity,
                bytes.as_mut_ptr(),
                bytes.len() as u64,
                &mut copied_info,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(bytes, command.as_bytes());

    let stale = info.identity;
    assert_eq!(
        unsafe { test_send_key(core.handle, view, &key(VIEM_KEY_LEFT, 0), &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe {
            viem_core_view_copy_command_line(
                core.handle,
                view,
                &stale,
                bytes.as_mut_ptr(),
                bytes.len() as u64,
                &mut copied_info,
            )
        },
        ViemStatus::StaleRevision,
        "command cursor movement must stale an otherwise unchanged export"
    );
    assert_eq!(copied_info, ViemCommandLineInfoV1::default());

    for (prefix, expected_kind) in [
        ('/', VIEM_COMMAND_LINE_KIND_SEARCH_FORWARD),
        ('?', VIEM_COMMAND_LINE_KIND_SEARCH_BACKWARD),
    ] {
        assert_eq!(
            unsafe {
                test_send_key(core.handle, view, &key(VIEM_KEY_ESCAPE, 0), &mut outcome)
            },
            ViemStatus::Ok
        );
        assert_eq!(
            unsafe {
                test_send_key(
                    core.handle,
                    view,
                    &key(VIEM_KEY_CHARACTER, prefix as u32),
                    &mut outcome,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(
            unsafe { viem_core_view_command_line_info(core.handle, view, &mut info) },
            ViemStatus::Ok
        );
        assert_eq!(info.identity.kind, expected_kind);
        assert_eq!(info.utf8_length, 0);
        assert_eq!(info.cursor_utf8_offset, 0);
    }

    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
}

#[test]
fn visual_line_export_uses_wrapped_or_flowed_rows_and_the_physical_line_policy() {
    for (source, flow) in [
        ("abcdefgh ijklmnop qrstuvwxyz\nTail", false),
        ("ab\ncdefgh ijklmnop qrstuvwxyz\n\nTail", true),
    ] {
        let core = create_core(
            source.as_bytes(),
            ViemDocumentOptions {
                format: VIEM_FORMAT_MARKDOWN_SOURCE,
                ..ViemDocumentOptions::default()
            },
        );
        let mut context = Box::new(FakeProviderContext::new(core.handle));
        let (view, mut outcome) = add_test_view(&core, &mut *context);
        assert_eq!(
            unsafe { viem_core_view_resize(core.handle, view, 90., 250., &mut outcome) },
            ViemStatus::Ok
        );
        assert_eq!(
            unsafe {
                viem_core_view_set_paragraph_flow(core.handle, view, u32::from(flow), &mut outcome)
            },
            ViemStatus::Ok
        );
        let mut layout = ViemLayoutSnapshotInfoV1::default();
        assert_eq!(
            unsafe { viem_core_view_layout_snapshot_info(core.handle, view, &mut layout) },
            ViemStatus::Ok
        );
        let mut rows = vec![ViemVisualRowV1::default(); layout.row_count as usize];
        let mut clusters = vec![ViemPositionedClusterV1::default(); layout.cluster_count as usize];
        let mut carets = vec![ViemPositionedCaretV1::default(); layout.caret_count as usize];
        assert_eq!(
            unsafe {
                viem_core_view_copy_layout_snapshot(
                    core.handle,
                    view,
                    &layout.identity,
                    rows.as_mut_ptr(),
                    rows.len() as u64,
                    clusters.as_mut_ptr(),
                    clusters.len() as u64,
                    carets.as_mut_ptr(),
                    carets.len() as u64,
                    &mut ViemLayoutSnapshotInfoV1::default(),
                )
            },
            ViemStatus::Ok
        );
        let visual_end = rows[0].text_end;
        let physical_end = source.find('\n').unwrap() as u64 + 1;
        assert_ne!(
            visual_end, physical_end,
            "fixture must distinguish row and source line"
        );
        for (mode, expected_end) in [(0, visual_end), (1, physical_end)] {
            assert_eq!(
                unsafe { viem_core_view_set_line_mode(core.handle, view, mode, &mut outcome) },
                ViemStatus::Ok
            );
            assert_eq!(
                unsafe {
                    test_send_key(
                        core.handle,
                        view,
                        &key(VIEM_KEY_CHARACTER, 'V' as u32),
                        &mut outcome,
                    )
                },
                ViemStatus::Ok
            );
            let mut info = ViemVisualSelectionInfoV1::default();
            assert_eq!(
                unsafe { viem_core_view_visual_selection_info(core.handle, view, &mut info) },
                ViemStatus::Ok
            );
            let mut segments =
                vec![ViemVisualSelectionSegmentV1::default(); info.segment_count as usize];
            let mut rectangles =
                vec![ViemVisualSelectionRectangleV1::default(); info.rectangle_count as usize];
            assert_eq!(
                unsafe {
                    viem_core_view_copy_visual_selection(
                        core.handle,
                        view,
                        &info.identity,
                        segments.as_mut_ptr(),
                        segments.len() as u64,
                        rectangles.as_mut_ptr(),
                        rectangles.len() as u64,
                        &mut ViemVisualSelectionInfoV1::default(),
                    )
                },
                ViemStatus::Ok
            );
            assert_eq!(segments.len(), 1);
            assert_eq!(segments[0].text_start, 0);
            assert_eq!(
                segments[0].text_end, expected_end,
                "flow={flow}, mode={mode}"
            );
            assert_eq!(
                unsafe {
                    test_send_key(
                        core.handle,
                        view,
                        &key(VIEM_KEY_ESCAPE, 0),
                        &mut outcome,
                    )
                },
                ViemStatus::Ok
            );
        }
        let mut presentation = ViemViewPresentationV1::default();
        for (kind, expected) in [
            (
                VIEM_KEY_DOCUMENT_END,
                source.len() as u64 - if flow { 2 } else { 1 },
            ),
            (VIEM_KEY_DOCUMENT_START, 0),
        ] {
            assert_eq!(
                unsafe { test_send_key(core.handle, view, &key(kind, 0), &mut outcome) },
                ViemStatus::Ok
            );
            assert_eq!(
                unsafe { viem_core_view_presentation(core.handle, view, &mut presentation) },
                ViemStatus::Ok
            );
            assert_eq!(presentation.cursor_utf8_offset, expected);
        }
        assert_eq!(document_state(&core).document_revision, 0);
    }
}

#[test]
fn visual_selection_export_preserves_character_line_and_block_semantics() {
    let core = create_core(b"ab\ncd", ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);

    let mut info = ViemVisualSelectionInfoV1::default();
    assert_eq!(
        unsafe { viem_core_view_visual_selection_info(core.handle, view, &mut info) },
        ViemStatus::Ok
    );
    assert_eq!(info.identity.kind, VIEM_VISUAL_SELECTION_KIND_NONE);
    assert_eq!(info.segment_count, 0);
    assert_eq!(info.rectangle_count, 0);

    for character in ['v', 'l'] {
        assert_eq!(
            unsafe {
                test_send_key(
                    core.handle,
                    view,
                    &key(VIEM_KEY_CHARACTER, character as u32),
                    &mut outcome,
                )
            },
            ViemStatus::Ok
        );
    }
    assert_eq!(
        unsafe { viem_core_view_visual_selection_info(core.handle, view, &mut info) },
        ViemStatus::Ok
    );
    assert_eq!(info.identity.kind, VIEM_VISUAL_SELECTION_KIND_CHARACTER);
    assert_eq!(info.segment_count, 1);
    assert!(info.rectangle_count >= 1);
    let mut character_segments =
        vec![ViemVisualSelectionSegmentV1::default(); info.segment_count as usize];
    let mut character_rectangles =
        vec![ViemVisualSelectionRectangleV1::default(); info.rectangle_count as usize];
    let mut copied = ViemVisualSelectionInfoV1::default();
    assert_eq!(
        unsafe {
            viem_core_view_copy_visual_selection(
                core.handle,
                view,
                &info.identity,
                character_segments.as_mut_ptr(),
                character_segments.len() as u64,
                character_rectangles.as_mut_ptr(),
                character_rectangles.len() as u64,
                &mut copied,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(copied, info);
    assert_eq!(character_segments[0].text_start, 0);
    assert_eq!(character_segments[0].text_end, 2);
    assert_eq!(character_segments[0].flags, 0);
    assert!(character_rectangles
        .iter()
        .all(|rectangle| rectangle.segment_index == 0));

    assert_eq!(
        unsafe {
            test_send_key(core.handle, view, &key(VIEM_KEY_ESCAPE, 0), &mut outcome)
        },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, 'V' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_visual_selection_info(core.handle, view, &mut info) },
        ViemStatus::Ok
    );
    assert_eq!(info.identity.kind, VIEM_VISUAL_SELECTION_KIND_LINE);
    let mut line_segments =
        vec![ViemVisualSelectionSegmentV1::default(); info.segment_count as usize];
    let mut line_rectangles =
        vec![ViemVisualSelectionRectangleV1::default(); info.rectangle_count as usize];
    assert_eq!(
        unsafe {
            viem_core_view_copy_visual_selection(
                core.handle,
                view,
                &info.identity,
                line_segments.as_mut_ptr(),
                line_segments.len() as u64,
                line_rectangles.as_mut_ptr(),
                line_rectangles.len() as u64,
                &mut copied,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(line_segments.len(), 1);
    assert_eq!(line_segments[0].text_start, 0);
    assert_eq!(
        line_segments[0].text_end, 3,
        "Linewise includes the hard break"
    );
    assert!(line_rectangles
        .iter()
        .any(|rectangle| rectangle.rect.width == 0.0));

    assert_eq!(
        unsafe {
            test_send_key(core.handle, view, &key(VIEM_KEY_ESCAPE, 0), &mut outcome)
        },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, '0' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    for input in [
        key(VIEM_KEY_CONTROL_CHARACTER, 'v' as u32),
        key(VIEM_KEY_CHARACTER, 'l' as u32),
        key(VIEM_KEY_CHARACTER, 'j' as u32),
    ] {
        assert_eq!(
            unsafe { test_send_key(core.handle, view, &input, &mut outcome) },
            ViemStatus::Ok
        );
    }
    assert_eq!(
        unsafe { viem_core_view_visual_selection_info(core.handle, view, &mut info) },
        ViemStatus::Ok
    );
    assert_eq!(info.identity.kind, VIEM_VISUAL_SELECTION_KIND_BLOCK);
    assert_eq!(info.segment_count, 2);
    let mut block_segments =
        vec![ViemVisualSelectionSegmentV1::default(); info.segment_count as usize];
    let mut block_rectangles =
        vec![ViemVisualSelectionRectangleV1::default(); info.rectangle_count as usize];
    block_segments[0].text_start = u64::MAX;
    block_rectangles[0].rect.x = f32::MAX;
    assert_eq!(
        unsafe {
            viem_core_view_copy_visual_selection(
                core.handle,
                view,
                &info.identity,
                block_segments.as_mut_ptr(),
                block_segments.len().saturating_sub(1) as u64,
                block_rectangles.as_mut_ptr(),
                block_rectangles.len() as u64,
                &mut copied,
            )
        },
        ViemStatus::BufferTooSmall
    );
    assert_eq!(copied, info);
    assert_eq!(block_segments[0].text_start, u64::MAX);
    assert_eq!(block_rectangles[0].rect.x, f32::MAX);
    assert_eq!(
        unsafe {
            viem_core_view_copy_visual_selection(
                core.handle,
                view,
                &info.identity,
                block_segments.as_mut_ptr(),
                block_segments.len() as u64,
                block_rectangles.as_mut_ptr(),
                block_rectangles.len() as u64,
                &mut copied,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(
        block_segments
            .iter()
            .map(|segment| segment.text_start..segment.text_end)
            .collect::<Vec<_>>(),
        vec![0..2, 3..5]
    );
    let block_flags = VIEM_VISUAL_SELECTION_SEGMENT_HAS_VISUAL_ROW
        | VIEM_VISUAL_SELECTION_SEGMENT_HAS_HARD_LINE
        | VIEM_VISUAL_SELECTION_SEGMENT_HAS_EDGE_AFFINITIES;
    for (index, segment) in block_segments.iter().enumerate() {
        assert_eq!(segment.struct_size, VIEM_VISUAL_SELECTION_SEGMENT_V1_SIZE);
        assert_eq!(segment.flags, block_flags);
        assert_eq!(segment.row_index, index as u64);
        assert_eq!(segment.hard_line_index, index as u64);
        assert!(matches!(
            segment.left_affinity,
            VIEM_BOUNDARY_AFFINITY_UPSTREAM | VIEM_BOUNDARY_AFFINITY_DOWNSTREAM
        ));
        assert!(matches!(
            segment.right_affinity,
            VIEM_BOUNDARY_AFFINITY_UPSTREAM | VIEM_BOUNDARY_AFFINITY_DOWNSTREAM
        ));
    }
    assert!(block_rectangles.windows(2).all(|pair| {
        (pair[0].row_index, pair[0].rect.x) <= (pair[1].row_index, pair[1].rect.x)
    }));
    for rectangle in &block_rectangles {
        assert_eq!(
            rectangle.struct_size,
            VIEM_VISUAL_SELECTION_RECTANGLE_V1_SIZE
        );
        assert!(rectangle.segment_index < block_segments.len() as u64);
        assert_eq!(
            rectangle.row_index,
            block_segments[rectangle.segment_index as usize].row_index
        );
    }

    let stale_selection = info.identity;
    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, 'h' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe {
            viem_core_view_copy_visual_selection(
                core.handle,
                view,
                &stale_selection,
                block_segments.as_mut_ptr(),
                block_segments.len() as u64,
                block_rectangles.as_mut_ptr(),
                block_rectangles.len() as u64,
                &mut copied,
            )
        },
        ViemStatus::StaleRevision,
        "Visual motion without relayout must stale the selection state token"
    );
    assert_eq!(copied, ViemVisualSelectionInfoV1::default());

    let stale_layout = {
        assert_eq!(
            unsafe { viem_core_view_visual_selection_info(core.handle, view, &mut info) },
            ViemStatus::Ok
        );
        info.identity
    };
    assert_eq!(
        unsafe { viem_core_view_resize(core.handle, view, 320.0, 160.0, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe {
            viem_core_view_copy_visual_selection(
                core.handle,
                view,
                &stale_layout,
                block_segments.as_mut_ptr(),
                block_segments.len() as u64,
                block_rectangles.as_mut_ptr(),
                block_rectangles.len() as u64,
                &mut copied,
            )
        },
        ViemStatus::StaleRevision
    );

    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
}

#[test]
fn native_find_selection_is_exact_literal_and_reveal_requires_visual_state() {
    let core = create_core(b"a.c abc a.c", ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);

    assert_eq!(
        unsafe { viem_core_view_reveal_selection(core.handle, view, &mut outcome) },
        ViemStatus::InvalidRange
    );
    assert_eq!(outcome, ViemCoreOutcomeV1::default());

    for input in [
        key(VIEM_KEY_CHARACTER, 'v' as u32),
        key(VIEM_KEY_CHARACTER, '2' as u32),
        key(VIEM_KEY_CHARACTER, 'l' as u32),
    ] {
        assert_eq!(
            unsafe { test_send_key(core.handle, view, &input, &mut outcome) },
            ViemStatus::Ok
        );
    }

    let mut selection = ViemVisualSelectionInfoV1::default();
    assert_eq!(
        unsafe { viem_core_view_visual_selection_info(core.handle, view, &mut selection) },
        ViemStatus::Ok
    );
    assert_eq!(
        selection.identity.kind,
        VIEM_VISUAL_SELECTION_KIND_CHARACTER
    );
    assert_eq!(
        unsafe {
            viem_core_view_use_selection_for_find(
                core.handle,
                view,
                &selection.identity,
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(outcome.cursor_utf8_offset, 2);
    assert_eq!(outcome.document_revision, 0);

    let stale = selection.identity;
    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, 'h' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_use_selection_for_find(core.handle, view, &stale, &mut outcome) },
        ViemStatus::StaleRevision
    );
    assert_eq!(outcome, ViemCoreOutcomeV1::default());

    assert_eq!(
        unsafe { viem_core_view_reveal_selection(core.handle, view, &mut outcome) },
        ViemStatus::Ok
    );
    for input in [
        key(VIEM_KEY_ESCAPE, 0),
        key(VIEM_KEY_CHARACTER, '0' as u32),
        key(VIEM_KEY_CHARACTER, 'n' as u32),
    ] {
        assert_eq!(
            unsafe { test_send_key(core.handle, view, &input, &mut outcome) },
            ViemStatus::Ok
        );
    }
    assert_eq!(
        outcome.cursor_utf8_offset, 8,
        "selection metacharacters must remain literal in the core search"
    );

    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
}

#[test]
fn visual_selection_preserves_logical_extent_outside_partial_layout_coverage() {
    let source = "row\n".repeat(3_000);
    let core = create_core(source.as_bytes(), ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);
    let mut layout = ViemLayoutSnapshotInfoV1::default();
    assert_eq!(
        unsafe { viem_core_view_layout_snapshot_info(core.handle, view, &mut layout) },
        ViemStatus::Ok
    );
    assert_eq!(layout.flags & VIEM_LAYOUT_SNAPSHOT_FULL_DOCUMENT, 0);

    for character in ['V', 'G'] {
        assert_eq!(
            unsafe {
                test_send_key(
                    core.handle,
                    view,
                    &key(VIEM_KEY_CHARACTER, character as u32),
                    &mut outcome,
                )
            },
            ViemStatus::Ok
        );
    }
    for top in [None, Some(10_000.0), Some(0.0)] {
        if let Some(top) = top {
            let mut state = ViemViewportStateV1::default();
            assert_eq!(
                unsafe { viem_core_view_viewport_state(core.handle, view, &mut state) },
                ViemStatus::Ok
            );
            let request = ViemViewportOriginV1 {
                flags: VIEM_VIEWPORT_ORIGIN_HAS_TOP,
                top,
                expected_document_id: state.document_id,
                expected_document_revision: state.document_revision,
                expected_layout_revision: state.layout_revision,
                expected_configuration_generation: state.configuration_generation,
                expected_measurement_environment_id: state.measurement_environment_id,
                expected_metrics_generation: state.metrics_generation,
                ..ViemViewportOriginV1::default()
            };
            assert_eq!(
                unsafe {
                    viem_core_view_set_viewport_origin(core.handle, view, &request, &mut outcome)
                },
                ViemStatus::Ok
            );
        }
        let mut selection = ViemVisualSelectionInfoV1::default();
        assert_eq!(
            unsafe { viem_core_view_visual_selection_info(core.handle, view, &mut selection) },
            ViemStatus::Ok
        );
        assert_eq!(selection.identity.kind, VIEM_VISUAL_SELECTION_KIND_LINE);
        assert_eq!(selection.segment_count, 1);
        assert!(selection.rectangle_count > 0);
        let mut segment = ViemVisualSelectionSegmentV1::default();
        let mut rectangles =
            vec![ViemVisualSelectionRectangleV1::default(); selection.rectangle_count as usize];
        assert_eq!(
            unsafe {
                viem_core_view_copy_visual_selection(
                    core.handle,
                    view,
                    &selection.identity,
                    &mut segment,
                    1,
                    rectangles.as_mut_ptr(),
                    rectangles.len() as u64,
                    &mut ViemVisualSelectionInfoV1::default(),
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(segment.text_start, 0);
        assert_eq!(segment.text_end, source.len() as u64);
        assert_eq!(
            unsafe { viem_core_view_layout_snapshot_info(core.handle, view, &mut layout) },
            ViemStatus::Ok
        );
        assert_eq!(selection.identity.layout, layout.identity);
        assert!(rectangles.iter().all(
            |rectangle| rectangle.row_index < layout.row_count && rectangle.segment_index == 0
        ));
        assert!(rectangles
            .iter()
            .any(|rectangle| rectangle.rect.width > 0.0));
    }

    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
}

#[test]
fn native_ffi_history_is_mode_independent_and_finalizes_insert_grouping() {
    let core = create_core(b"base", ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);

    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, 'i' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(outcome.mode, VIEM_MODE_INSERT);
    for byte in [b"x".as_slice(), b"y".as_slice()] {
        assert_eq!(
            unsafe {
                test_send_text(
                    core.handle,
                    view,
                    byte.as_ptr(),
                    byte.len() as u64,
                    &mut outcome,
                )
            },
            ViemStatus::Ok
        );
    }
    assert_eq!(
        copy_core_bytes(
            viem_core_copy_formatted_utf8,
            &core,
            outcome.document_revision,
        ),
        b"xybase"
    );

    assert_eq!(
        unsafe { viem_core_view_undo(core.handle, view, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(outcome.command_status, VIEM_COMMAND_STATUS_COMPLETE);
    assert_eq!(outcome.mode, VIEM_MODE_NORMAL);
    assert_ne!(outcome.flags & VIEM_OUTCOME_DOCUMENT_CHANGED, 0);
    assert_ne!(outcome.flags & VIEM_OUTCOME_MODE_CHANGED, 0);
    assert_eq!(
        copy_core_bytes(
            viem_core_copy_formatted_utf8,
            &core,
            outcome.document_revision,
        ),
        b"base",
        "one native undo removes the complete open Insert unit"
    );

    assert_eq!(
        unsafe { viem_core_view_redo(core.handle, view, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(outcome.mode, VIEM_MODE_NORMAL);
    assert_eq!(outcome.cursor_utf8_offset, 2);
    assert_eq!(
        copy_core_bytes(
            viem_core_copy_formatted_utf8,
            &core,
            outcome.document_revision,
        ),
        b"xybase"
    );

    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, 'v' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(outcome.mode, VIEM_MODE_VISUAL_CHARACTER);
    assert_eq!(
        unsafe { viem_core_view_undo(core.handle, view, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(outcome.mode, VIEM_MODE_NORMAL);
    assert_ne!(outcome.flags & VIEM_OUTCOME_MODE_CHANGED, 0);
    let mut presentation = ViemViewPresentationV1::default();
    assert_eq!(
        unsafe { viem_core_view_presentation(core.handle, view, &mut presentation) },
        ViemStatus::Ok
    );
    assert_eq!(
        presentation.flags & VIEM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR,
        0
    );
    assert_eq!(
        copy_core_bytes(
            viem_core_copy_formatted_utf8,
            &core,
            outcome.document_revision,
        ),
        b"base"
    );

    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
}

#[test]
fn viewport_origin_api_is_identity_bound_bounded_and_atomic() {
    let source = (0..2_000)
        .map(|line| format!("WWWWWWWWWWWWWWWW line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let core = create_core(source.as_bytes(), ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let provider = provider(
        (&mut *context as *mut FakeProviderContext).cast(),
        fake_shape_batch,
    );
    let options = ViemViewOptionsV1 {
        width: 40.0,
        height: 32.0,
        ..ViemViewOptionsV1::default()
    };
    let mut view = 0;
    let mut outcome = ViemCoreOutcomeV1::default();
    assert_eq!(
        unsafe { viem_core_view_add(core.handle, &options, &provider, &mut view, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_set_wrap(core.handle, view, 0, &mut outcome) },
        ViemStatus::Ok
    );

    let mut state = ViemViewportStateV1::default();
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, view, &mut state) },
        ViemStatus::Ok
    );
    assert_eq!(state.left, 0.0);
    assert_ne!(state.flags & VIEM_VIEWPORT_STATE_HAS_LAYOUT, 0);
    assert_ne!(state.flags & VIEM_VIEWPORT_STATE_TOP_EXACT, 0);
    assert_eq!(state.document_revision, outcome.document_revision);
    assert_eq!(state.flags & VIEM_VIEWPORT_STATE_MAXIMUM_TOP_EXACT, 0);
    assert!(state.maximum_top > options.height);
    assert_eq!(state.layout_revision, outcome.layout_revision);
    assert_eq!(
        state.configuration_generation,
        outcome.configuration_generation
    );
    assert_eq!(
        state.measurement_environment_id,
        outcome.measurement_environment_id
    );
    assert_eq!(state.metrics_generation, outcome.metrics_generation);
    assert!(state.document_id != 0);

    let layout_revision = outcome.layout_revision;
    let configuration_generation = outcome.configuration_generation;
    let shape_calls = context.shape_calls;
    let request = ViemViewportOriginV1 {
        left: 40.0,
        expected_document_id: state.document_id,
        expected_document_revision: state.document_revision,
        expected_layout_revision: state.layout_revision,
        expected_configuration_generation: state.configuration_generation,
        expected_measurement_environment_id: state.measurement_environment_id,
        expected_metrics_generation: state.metrics_generation,
        ..ViemViewportOriginV1::default()
    };
    assert_eq!(
        unsafe { viem_core_view_set_viewport_origin(core.handle, view, &request, &mut outcome) },
        ViemStatus::Ok
    );
    assert_ne!(outcome.flags & VIEM_OUTCOME_LAYOUT_CHANGED, 0);
    assert_eq!(outcome.layout_revision, layout_revision);
    assert_eq!(outcome.configuration_generation, configuration_generation);
    assert_eq!(context.shape_calls, shape_calls);
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, view, &mut state) },
        ViemStatus::Ok
    );
    assert_eq!(state.left, 40.0);

    for invalid in [
        ViemViewportOriginV1 {
            struct_size: VIEM_VIEWPORT_ORIGIN_V1_SIZE - 1,
            left: 60.0,
            ..request
        },
        ViemViewportOriginV1 {
            flags: 1 << 31,
            left: 60.0,
            ..request
        },
        ViemViewportOriginV1 {
            left: f32::NAN,
            ..request
        },
        ViemViewportOriginV1 {
            flags: VIEM_VIEWPORT_ORIGIN_HAS_TOP,
            left: 60.0,
            top: f32::INFINITY,
            ..request
        },
    ] {
        assert_eq!(
            unsafe {
                viem_core_view_set_viewport_origin(core.handle, view, &invalid, &mut outcome)
            },
            ViemStatus::InvalidArgument
        );
        assert_eq!(
            unsafe { viem_core_view_viewport_state(core.handle, view, &mut state) },
            ViemStatus::Ok
        );
        assert_eq!(state.left, 40.0);
    }

    let vertical = ViemViewportOriginV1 {
        flags: VIEM_VIEWPORT_ORIGIN_HAS_TOP,
        left: 60.0,
        top: 14_000.0,
        ..request
    };
    let shaped_bytes = context.shaped_bytes;
    context.minimum_request_start = u64::MAX;
    context.maximum_request_end = 0;
    assert_eq!(
        unsafe { viem_core_view_set_viewport_origin(core.handle, view, &vertical, &mut outcome) },
        ViemStatus::Ok
    );
    assert_ne!(outcome.flags & VIEM_OUTCOME_LAYOUT_CHANGED, 0);
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, view, &mut state) },
        ViemStatus::Ok
    );
    assert_eq!(state.left, 60.0);
    assert!(state.top > 10_000.0);
    assert_eq!(state.flags & VIEM_VIEWPORT_STATE_TOP_EXACT, 0);
    assert!(context.minimum_request_start > 1_000);
    assert_eq!(state.flags & VIEM_VIEWPORT_STATE_MAXIMUM_TOP_EXACT, 0);
    assert!(state.maximum_top > state.top);
    assert!(context.maximum_request_end < source.len() as u64);
    assert!(context.shaped_bytes - shaped_bytes < 10_000);

    let mut layout_info = ViemLayoutSnapshotInfoV1::default();
    assert_eq!(
        unsafe { viem_core_view_layout_snapshot_info(core.handle, view, &mut layout_info) },
        ViemStatus::Ok
    );
    assert!(layout_info.coverage_hard_line_start > 500);
    assert!(layout_info.coverage_hard_line_end - layout_info.coverage_hard_line_start < 100);
    assert!(layout_info.coverage_y_start <= state.top);
    assert!(state.top + options.height <= layout_info.coverage_y_end);

    let after_down = state;
    outcome = ViemCoreOutcomeV1 {
        flags: u32::MAX,
        ..ViemCoreOutcomeV1::default()
    };
    assert_eq!(
        unsafe { viem_core_view_set_viewport_origin(core.handle, view, &vertical, &mut outcome) },
        ViemStatus::StaleRevision
    );
    assert_eq!(outcome, ViemCoreOutcomeV1::default());
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, view, &mut state) },
        ViemStatus::Ok
    );
    assert_eq!(
        state, after_down,
        "a stale request changes neither axis nor layout"
    );

    let request_from_state =
        |state: ViemViewportStateV1, left: f32, top: f32| ViemViewportOriginV1 {
            flags: VIEM_VIEWPORT_ORIGIN_HAS_TOP,
            left,
            top,
            expected_document_id: state.document_id,
            expected_document_revision: state.document_revision,
            expected_layout_revision: state.layout_revision,
            expected_configuration_generation: state.configuration_generation,
            expected_measurement_environment_id: state.measurement_environment_id,
            expected_metrics_generation: state.metrics_generation,
            ..ViemViewportOriginV1::default()
        };
    let upward = request_from_state(state, 20.0, 1_000.0);
    assert_eq!(
        unsafe { viem_core_view_set_viewport_origin(core.handle, view, &upward, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, view, &mut state) },
        ViemStatus::Ok
    );
    assert_eq!(state.left, 20.0);
    assert!(state.top < after_down.top);

    let bottom = request_from_state(state, 30.0, f32::MAX);
    assert_eq!(
        unsafe { viem_core_view_set_viewport_origin(core.handle, view, &bottom, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, view, &mut state) },
        ViemStatus::Ok
    );
    assert_eq!(state.left, 30.0);
    assert_eq!(
        unsafe { viem_core_view_layout_snapshot_info(core.handle, view, &mut layout_info) },
        ViemStatus::Ok
    );
    assert_eq!(layout_info.coverage_hard_line_end, 2_000);
    assert_ne!(state.flags & VIEM_VIEWPORT_STATE_MAXIMUM_TOP_EXACT, 0);
    assert_eq!(state.maximum_top, state.top);
    assert_eq!(
        state.top,
        (layout_info.coverage_y_end - options.height).max(layout_info.coverage_y_start)
    );

    let top = request_from_state(state, 10.0, -100.0);
    assert_eq!(
        unsafe { viem_core_view_set_viewport_origin(core.handle, view, &top, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, view, &mut state) },
        ViemStatus::Ok
    );
    assert_eq!(state.left, 10.0);
    assert_eq!(state.top, 0.0);
    assert_ne!(state.flags & VIEM_VIEWPORT_STATE_TOP_EXACT, 0);

    let before_failure = state;
    context.fail_next = Some(ViemStatus::ProviderFailure);
    let failed = request_from_state(state, 70.0, 5_000.0);
    assert_eq!(
        unsafe { viem_core_view_set_viewport_origin(core.handle, view, &failed, &mut outcome) },
        ViemStatus::ProviderFailure
    );
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, view, &mut state) },
        ViemStatus::Ok
    );
    assert_eq!(
        state, before_failure,
        "provider failure publishes no staged state"
    );

    #[repr(C, align(8))]
    struct OverlapStorage([u8; 128]);
    let mut overlap = OverlapStorage([0; 128]);
    let overlap_request = overlap.0.as_mut_ptr().cast::<ViemViewportOriginV1>();
    unsafe { overlap_request.write(request) };
    assert_eq!(
        unsafe {
            viem_core_view_set_viewport_origin(
                core.handle,
                view,
                overlap_request,
                overlap_request.cast(),
            )
        },
        ViemStatus::InvalidArgument
    );

    context.metrics_generation += 1;
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, view, &mut state) },
        ViemStatus::Ok
    );
    assert_eq!(state.flags & VIEM_VIEWPORT_STATE_HAS_LAYOUT, 0);
    assert_eq!(state.flags & VIEM_VIEWPORT_STATE_TOP_EXACT, 0);
    assert_eq!(state.flags & VIEM_VIEWPORT_STATE_MAXIMUM_TOP_EXACT, 0);
    assert_eq!(state.maximum_top, state.top, "stale height is not a scrollbar extent");
    assert_eq!(
        state.flags & VIEM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT,
        0,
        "stale width is not advertised as an exact clamp"
    );
    assert_eq!(state.metrics_generation, 2);
    let unavailable_state = state;
    assert_eq!(
        unsafe { viem_core_view_set_viewport_origin(core.handle, view, &failed, &mut outcome) },
        ViemStatus::LayoutUnavailable
    );
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, view, &mut state) },
        ViemStatus::Ok
    );
    assert_eq!(state, unavailable_state);

    assert_eq!(
        unsafe { viem_core_view_set_wrap(core.handle, view, 1, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, view, &mut state) },
        ViemStatus::Ok
    );
    assert_eq!(state.left, 0.0);
    assert_ne!(state.flags & VIEM_VIEWPORT_STATE_WRAP, 0);
    assert_ne!(state.flags & VIEM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT, 0);
    assert!(state.maximum_left > 0.0, "an unbreakable word overflows while wrapping");

    assert_eq!(
        unsafe { viem_core_view_viewport_state(core.handle, view, ptr::null_mut()) },
        ViemStatus::NullPointer
    );
    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
}

#[test]
fn provider_key_utf8_pointer_and_enum_failures_are_rejected_without_state_change() {
    let core = create_core(b"text", ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let context_pointer = (&mut *context as *mut FakeProviderContext).cast();
    let options = ViemViewOptionsV1::default();
    let mut view = 99;
    let mut outcome = ViemCoreOutcomeV1::default();

    let mut invalid_provider = provider(context_pointer, fake_shape_batch);
    invalid_provider.struct_size = VIEM_TEXT_MEASUREMENT_PROVIDER_V1_SIZE - 1;
    assert_eq!(
        unsafe {
            viem_core_view_add(
                core.handle,
                &options,
                &invalid_provider,
                &mut view,
                &mut outcome,
            )
        },
        ViemStatus::InvalidProvider
    );
    assert_eq!(view, 0);

    invalid_provider = provider(context_pointer, fake_shape_batch);
    for version in [0, 1, 2, VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION + 1] {
        invalid_provider.abi_version = version;
        assert_eq!(
            unsafe {
                viem_core_view_add(
                    core.handle,
                    &options,
                    &invalid_provider,
                    &mut view,
                    &mut outcome,
                )
            },
            ViemStatus::InvalidProvider
        );

    }

    for missing_retain in [true, false] {
        invalid_provider = provider(context_pointer, fake_shape_batch);
        if missing_retain { invalid_provider.retain_render_runs = None; }
        else { invalid_provider.release_render_runs = None; }
        assert_eq!(unsafe { viem_core_view_add(core.handle, &options, &invalid_provider,
            &mut view, &mut outcome) }, ViemStatus::InvalidProvider);
        assert_eq!(view, 0);
    }

    invalid_provider = provider(context_pointer, fake_shape_batch);
    invalid_provider.threading = 99;
    assert_eq!(
        unsafe {
            viem_core_view_add(
                core.handle,
                &options,
                &invalid_provider,
                &mut view,
                &mut outcome,
            )
        },
        ViemStatus::InvalidProvider
    );

    let malformed = provider(context_pointer, malformed_shape_batch);
    assert_eq!(
        unsafe { viem_core_view_add(core.handle, &options, &malformed, &mut view, &mut outcome,) },
        ViemStatus::ProviderFailure
    );
    assert_eq!(view, 0);

    let valid = provider(context_pointer, fake_shape_batch);
    assert_eq!(
        unsafe { viem_core_view_add(core.handle, &options, &valid, &mut view, &mut outcome,) },
        ViemStatus::Ok
    );
    assert_eq!(
        view, 2,
        "failed provider layout burned view 1 without reuse"
    );

    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_ESCAPE, 'x' as u32),
                &mut outcome,
            )
        },
        ViemStatus::InvalidKey
    );
    assert_eq!(outcome.mode, 0);
    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, 0x11_0000),
                &mut outcome,
            )
        },
        ViemStatus::InvalidKey
    );
    assert_eq!(
        unsafe { test_send_text(core.handle, view, [0xff].as_ptr(), 1, &mut outcome) },
        ViemStatus::InvalidUtf8
    );
    assert_eq!(
        unsafe { viem_core_view_set_wrap(core.handle, view, 2, &mut outcome) },
        ViemStatus::InvalidArgument
    );
    assert_eq!(
        unsafe { viem_core_view_resize(core.handle, view, f32::NAN, 10.0, &mut outcome) },
        ViemStatus::InvalidArgument
    );
    assert_eq!(
        unsafe { test_send_text(core.handle, view, ptr::null(), 1, &mut outcome) },
        ViemStatus::NullPointer
    );
    assert_eq!(
        unsafe { viem_core_view_state(core.handle, view, ptr::null_mut()) },
        ViemStatus::NullPointer
    );
}

#[test]
fn provider_crossing_cluster_and_unstable_context_contract_are_explicit() {
    let mut source = vec![b'x'; 4095];
    source.extend_from_slice(b"fitail");
    let core = create_core(&source, ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let context_pointer = (&mut *context as *mut FakeProviderContext).cast();
    let options = ViemViewOptionsV1 {
        width: 100_000.0,
        height: 100.0,
        ..ViemViewOptionsV1::default()
    };
    let mut view = 0;
    let mut outcome = ViemCoreOutcomeV1::default();
    let current = provider(context_pointer, fake_shape_batch);

    assert_eq!(
        unsafe { viem_core_view_add(core.handle, &options, &current, &mut view, &mut outcome) },
        ViemStatus::Ok
    );
    assert!(context.saw_crossing_cluster_tail);
    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);

    let declining = provider(context_pointer, unstable_context_shape_batch);
    assert_eq!(
        unsafe { viem_core_view_add(core.handle, &options, &declining, &mut view, &mut outcome,) },
        ViemStatus::UnstableShapingContext
    );
    assert_eq!(view, 0, "a declined batch must not install a view/layout");
}

#[test]
fn core_handles_are_final_and_never_reused() {
    let mut first = create_core(b"one", ViemDocumentOptions::default());
    let first_handle = first.handle;
    assert_eq!(viem_core_destroy(first_handle), ViemStatus::Ok);
    first.handle = 0;
    assert_eq!(viem_core_destroy(first_handle), ViemStatus::InvalidHandle);
    let second = create_core(b"two", ViemDocumentOptions::default());
    assert_ne!(first_handle, second.handle);
    let mut revision = 99;
    assert_eq!(
        unsafe { viem_core_revision(first_handle, &mut revision) },
        ViemStatus::InvalidHandle
    );
    assert_eq!(revision, 0);
    let mut outcome = ViemCoreOutcomeV1 { flags: u32::MAX, ..Default::default() };
    assert_eq!(
        unsafe { test_send_text(first_handle, 1, b"x".as_ptr(), 1, &mut outcome) },
        ViemStatus::InvalidHandle,
        "input cannot revive a destroyed core"
    );
    assert_eq!(outcome, ViemCoreOutcomeV1::default());
}

#[test]
fn busy_destroy_preserves_provider_lifetime_until_successful_retry() {
    let mut core = create_core(b"provider lifetime", ViemDocumentOptions::default());
    let first_handle = core.handle;
    let gate = Arc::new(CallbackGate::default());
    let mut context = Box::new(BlockingProviderContext {
        provider: FakeProviderContext::new(first_handle),
        gate: Arc::clone(&gate),
    });
    let context_address = (&mut *context as *mut BlockingProviderContext) as usize;

    let worker = std::thread::spawn(move || {
        let mut provider = provider(context_address as *mut c_void, blocking_shape_batch);
        provider.metrics_generation = Some(blocking_metrics_generation);
        let mut view = 0;
        let mut outcome = ViemCoreOutcomeV1::default();
        let status = unsafe {
            viem_core_view_add(
                first_handle,
                &ViemViewOptionsV1::default(),
                &provider,
                &mut view,
                &mut outcome,
            )
        };
        (status, view)
    });

    gate.wait_until_entered();
    assert_eq!(
        viem_core_destroy(first_handle),
        ViemStatus::CoreBusy,
        "an active provider callback keeps the checked-out core alive"
    );
    assert_eq!(
        viem_core_destroy(first_handle),
        ViemStatus::CoreBusy,
        "a failed destroy must retain the handle for a later retry"
    );

    gate.release();
    let (status, view) = worker.join().unwrap();
    assert_eq!(status, ViemStatus::Ok);
    assert_ne!(view, 0);
    assert!(context.provider.shape_calls > 0);
    assert_eq!(
        context.provider.reentrant_status,
        ViemStatus::CoreBusy as u32
    );

    assert_eq!(viem_core_destroy(first_handle), ViemStatus::Ok);
    core.handle = 0;
    drop(context);
    assert_eq!(viem_core_destroy(first_handle), ViemStatus::InvalidHandle);

    let replacement = create_core(b"replacement", ViemDocumentOptions::default());
    assert_ne!(
        replacement.handle, first_handle,
        "a successfully destroyed core handle must never be reused"
    );
}

#[test]
fn native_composition_commit_is_one_exact_undo_unit() {
    let mut core = create_core(b"", ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let context_pointer = &mut *context as *mut FakeProviderContext;
    let (view, mut outcome) = add_test_view(&core, context_pointer);

    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, 'i' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { test_send_text(core.handle, view, b"a".as_ptr(), 1, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(outcome.document_revision, 1);
    assert_eq!(outcome.cursor_utf8_offset, 1);

    let begin = ViemCompositionBeginV1 {
        struct_size: VIEM_COMPOSITION_BEGIN_V1_SIZE,
        reserved: 0,
        document_revision: 1,
        replacement_start: 1,
        replacement_end: 1,
    };
    assert_eq!(
        unsafe { viem_core_view_composition_begin(core.handle, view, &begin, &mut outcome) },
        ViemStatus::Ok
    );
    assert_ne!(outcome.flags & VIEM_OUTCOME_HAS_COMPOSITION_CHANGES, 0);
    assert_eq!(outcome.document_revision, 1);

    let update_text = "e\u{301}".as_bytes();
    let update = ViemCompositionUpdateV1 {
        struct_size: VIEM_COMPOSITION_UPDATE_V1_SIZE,
        reserved: 0,
        document_revision: 1,
        marked_text: utf8_slice(update_text),
        selected_start: update_text.len() as u64,
        selected_end: update_text.len() as u64,
    };
    assert_eq!(
        unsafe { viem_core_view_composition_update(core.handle, view, &update, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(outcome.document_revision, 1, "marked text is an overlay");

    let committed_text = "é".as_bytes();
    let commit = ViemCompositionCommitV1 {
        struct_size: VIEM_COMPOSITION_COMMIT_V1_SIZE,
        reserved: 0,
        document_revision: 1,
        committed_text: utf8_slice(committed_text),
    };
    assert_eq!(
        unsafe { viem_core_view_composition_commit(core.handle, view, &commit, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(outcome.document_revision, 2);
    assert_eq!(outcome.cursor_utf8_offset, 3);
    assert_ne!(outcome.flags & VIEM_OUTCOME_DOCUMENT_CHANGED, 0);
    assert_ne!(outcome.flags & VIEM_OUTCOME_HAS_POSITION_MAP, 0);
    assert_eq!(
        copy_core_bytes(viem_core_copy_source_bytes, &core, 2),
        "aé".as_bytes()
    );

    assert_eq!(
        unsafe {
            test_send_key(core.handle, view, &key(VIEM_KEY_ESCAPE, 0), &mut outcome)
        },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, 'u' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(
        copy_core_bytes(
            viem_core_copy_formatted_utf8,
            &core,
            outcome.document_revision,
        ),
        b"a"
    );
    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, 'u' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(
        copy_core_bytes(
            viem_core_copy_formatted_utf8,
            &core,
            outcome.document_revision,
        ),
        b""
    );
    core.revision = outcome.document_revision;
    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
}

#[test]
fn composition_overlay_reads_are_exact_revision_tagged_and_source_nonmutating() {
    let core = create_core(b"hello", ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let context_pointer = &mut *context as *mut FakeProviderContext;
    let (view, mut outcome) = add_test_view(&core, context_pointer);

    let mut inactive = ViemCompositionOverlayInfoV1::default();
    assert_eq!(
        unsafe { viem_core_view_composition_overlay_info(core.handle, view, &mut inactive) },
        ViemStatus::Ok
    );
    assert_eq!(inactive.struct_size, VIEM_COMPOSITION_OVERLAY_INFO_V1_SIZE);
    assert_eq!(inactive.flags & VIEM_COMPOSITION_OVERLAY_ACTIVE, 0);
    assert_eq!(
        inactive.identity.struct_size,
        VIEM_COMPOSITION_OVERLAY_IDENTITY_V1_SIZE
    );

    let begin = ViemCompositionBeginV1 {
        struct_size: VIEM_COMPOSITION_BEGIN_V1_SIZE,
        reserved: 0,
        document_revision: 0,
        replacement_start: 1,
        replacement_end: 4,
    };
    assert_eq!(
        unsafe { viem_core_view_composition_begin(core.handle, view, &begin, &mut outcome) },
        ViemStatus::Ok
    );
    let marked = "é界".as_bytes();
    let update = ViemCompositionUpdateV1 {
        struct_size: VIEM_COMPOSITION_UPDATE_V1_SIZE,
        reserved: 0,
        document_revision: 0,
        marked_text: utf8_slice(marked),
        selected_start: "é".len() as u64,
        selected_end: marked.len() as u64,
    };
    assert_eq!(
        unsafe { viem_core_view_composition_update(core.handle, view, &update, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        copy_core_bytes(viem_core_copy_source_bytes, &core, 0),
        b"hello",
        "queryable marked text remains a disposable projection"
    );

    let mut info = ViemCompositionOverlayInfoV1::default();
    assert_eq!(
        unsafe { viem_core_view_composition_overlay_info(core.handle, view, &mut info) },
        ViemStatus::Ok
    );
    assert_ne!(info.flags & VIEM_COMPOSITION_OVERLAY_ACTIVE, 0);
    assert_eq!(info.identity.view_id, view);
    assert_eq!(info.identity.document_revision, 0);
    assert_eq!(info.identity.generation, 1);
    assert_eq!(info.utf8_length, "hé界o".len() as u64);
    assert_eq!((info.replacement_start, info.replacement_end), (1, 4));
    assert_eq!(
        (info.marked_start, info.marked_end),
        (1, 1 + marked.len() as u64)
    );
    assert_eq!(
        (info.selected_start, info.selected_end),
        (1 + "é".len() as u64, 1 + marked.len() as u64)
    );

    let request = ViemCompositionOverlayUtf8RangeV1 {
        struct_size: VIEM_COMPOSITION_OVERLAY_UTF8_RANGE_V1_SIZE,
        reserved: 0,
        identity: info.identity,
        start: 0,
        end: info.utf8_length,
    };
    let mut required = u64::MAX;
    assert_eq!(
        unsafe {
            viem_core_view_copy_composition_utf8_range(
                core.handle,
                view,
                &request,
                ptr::null_mut(),
                0,
                &mut required,
            )
        },
        ViemStatus::BufferTooSmall
    );
    assert_eq!(required, info.utf8_length);
    let mut bytes = vec![0; required as usize];
    assert_eq!(
        unsafe {
            viem_core_view_copy_composition_utf8_range(
                core.handle,
                view,
                &request,
                bytes.as_mut_ptr(),
                bytes.len() as u64,
                &mut required,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(bytes, "hé界o".as_bytes());

    let split_scalar = ViemCompositionOverlayUtf8RangeV1 {
        start: 2,
        end: 3,
        ..request
    };
    assert_eq!(
        unsafe {
            viem_core_view_copy_composition_utf8_range(
                core.handle,
                view,
                &split_scalar,
                bytes.as_mut_ptr(),
                bytes.len() as u64,
                &mut required,
            )
        },
        ViemStatus::InvalidUtf8Boundary
    );
    assert_eq!(required, 0, "failed reads clear their byte count first");

    let next_marked = "X".as_bytes();
    let next_update = ViemCompositionUpdateV1 {
        marked_text: utf8_slice(next_marked),
        selected_start: 1,
        selected_end: 1,
        ..update
    };
    assert_eq!(
        unsafe { viem_core_view_composition_update(core.handle, view, &next_update, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe {
            viem_core_view_copy_composition_utf8_range(
                core.handle,
                view,
                &request,
                bytes.as_mut_ptr(),
                bytes.len() as u64,
                &mut required,
            )
        },
        ViemStatus::StaleRevision,
        "a previous marked-text generation must never be read as current"
    );
    assert_eq!(required, 0);

    let cancel = ViemCompositionCancelV1 {
        struct_size: VIEM_COMPOSITION_CANCEL_V1_SIZE,
        reserved: 0,
        document_revision: 0,
    };
    assert_eq!(
        unsafe { viem_core_view_composition_cancel(core.handle, view, &cancel, &mut outcome) },
        ViemStatus::Ok
    );
    let mut cancelled = ViemCompositionOverlayInfoV1::default();
    assert_eq!(
        unsafe { viem_core_view_composition_overlay_info(core.handle, view, &mut cancelled) },
        ViemStatus::Ok
    );
    assert_eq!(cancelled.flags & VIEM_COMPOSITION_OVERLAY_ACTIVE, 0);
    assert_eq!(
        copy_core_bytes(viem_core_copy_source_bytes, &core, 0),
        b"hello"
    );
    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
}

#[test]
fn composition_cancel_and_invalid_or_stale_updates_are_source_atomic() {
    let mut core = create_core(b"hello", ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let context_pointer = &mut *context as *mut FakeProviderContext;
    let (view, mut outcome) = add_test_view(&core, context_pointer);
    let begin = ViemCompositionBeginV1 {
        struct_size: VIEM_COMPOSITION_BEGIN_V1_SIZE,
        reserved: 0,
        document_revision: 0,
        replacement_start: 1,
        replacement_end: 4,
    };
    let undersized_begin = ViemCompositionBeginV1 {
        struct_size: VIEM_COMPOSITION_BEGIN_V1_SIZE - 1,
        ..begin
    };
    assert_eq!(
        unsafe {
            viem_core_view_composition_begin(core.handle, view, &undersized_begin, &mut outcome)
        },
        ViemStatus::InvalidArgument
    );
    let inverted_begin = ViemCompositionBeginV1 {
        replacement_start: 4,
        replacement_end: 1,
        ..begin
    };
    assert_eq!(
        unsafe {
            viem_core_view_composition_begin(core.handle, view, &inverted_begin, &mut outcome)
        },
        ViemStatus::InvalidRange
    );
    assert_eq!(
        unsafe { viem_core_view_composition_begin(core.handle, view, ptr::null(), &mut outcome) },
        ViemStatus::NullPointer
    );
    let aliased = &mut outcome as *mut ViemCoreOutcomeV1;
    assert_eq!(
        unsafe { viem_core_view_composition_begin(core.handle, view, aliased.cast(), aliased) },
        ViemStatus::InvalidArgument
    );
    assert_eq!(
        unsafe { viem_core_view_composition_begin(core.handle, view, &begin, &mut outcome) },
        ViemStatus::Ok
    );
    let marked = "é".as_bytes();
    let valid_update = ViemCompositionUpdateV1 {
        struct_size: VIEM_COMPOSITION_UPDATE_V1_SIZE,
        reserved: 0,
        document_revision: 0,
        marked_text: utf8_slice(marked),
        selected_start: 2,
        selected_end: 2,
    };
    assert_eq!(
        unsafe {
            viem_core_view_composition_update(core.handle, view, &valid_update, &mut outcome)
        },
        ViemStatus::Ok
    );

    let split_update = ViemCompositionUpdateV1 {
        selected_start: 1,
        selected_end: 1,
        ..valid_update
    };
    assert_eq!(
        unsafe {
            viem_core_view_composition_update(core.handle, view, &split_update, &mut outcome)
        },
        ViemStatus::NotGraphemeBoundary
    );
    let out_of_range_update = ViemCompositionUpdateV1 {
        selected_start: 3,
        selected_end: 3,
        ..valid_update
    };
    assert_eq!(
        unsafe {
            viem_core_view_composition_update(core.handle, view, &out_of_range_update, &mut outcome)
        },
        ViemStatus::InvalidRange
    );
    let invalid_utf8 = [0xff];
    let malformed_update = ViemCompositionUpdateV1 {
        marked_text: utf8_slice(&invalid_utf8),
        selected_start: 0,
        selected_end: 0,
        ..valid_update
    };
    assert_eq!(
        unsafe {
            viem_core_view_composition_update(core.handle, view, &malformed_update, &mut outcome)
        },
        ViemStatus::InvalidUtf8
    );
    let null_text_update = ViemCompositionUpdateV1 {
        marked_text: ViemUtf8Slice {
            data: ptr::null(),
            length: 1,
        },
        selected_start: 0,
        selected_end: 0,
        ..valid_update
    };
    assert_eq!(
        unsafe {
            viem_core_view_composition_update(core.handle, view, &null_text_update, &mut outcome)
        },
        ViemStatus::NullPointer
    );
    let invalid_commit = ViemCompositionCommitV1 {
        struct_size: VIEM_COMPOSITION_COMMIT_V1_SIZE,
        reserved: 0,
        document_revision: 0,
        committed_text: utf8_slice(&invalid_utf8),
    };
    assert_eq!(
        unsafe {
            viem_core_view_composition_commit(core.handle, view, &invalid_commit, &mut outcome)
        },
        ViemStatus::InvalidUtf8
    );
    assert_eq!(
        copy_core_bytes(viem_core_copy_source_bytes, &core, 0),
        b"hello"
    );
    let cancel = ViemCompositionCancelV1 {
        struct_size: VIEM_COMPOSITION_CANCEL_V1_SIZE,
        reserved: 0,
        document_revision: 0,
    };
    assert_eq!(
        unsafe { viem_core_view_composition_cancel(core.handle, view, &cancel, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(outcome.document_revision, 0);
    assert_eq!(
        copy_core_bytes(viem_core_copy_source_bytes, &core, 0),
        b"hello"
    );

    assert_eq!(
        unsafe { viem_core_view_composition_begin(core.handle, view, &begin, &mut outcome) },
        ViemStatus::Ok
    );
    let (writer, _) = add_test_view(&core, context_pointer);
    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                writer,
                &key(VIEM_KEY_CHARACTER, 'i' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { test_send_text(core.handle, writer, b"X".as_ptr(), 1, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(outcome.document_revision, 1);
    let source_after_other_view_edit = copy_core_bytes(
        viem_core_copy_source_bytes,
        &core,
        outcome.document_revision,
    );
    assert_eq!(source_after_other_view_edit, b"Xhello");
    assert_eq!(
        unsafe {
            viem_core_view_composition_update(core.handle, view, &valid_update, &mut outcome)
        },
        ViemStatus::StaleRevision
    );
    assert_eq!(
        copy_core_bytes(viem_core_copy_source_bytes, &core, 1),
        source_after_other_view_edit
    );
    core.revision = 1;
    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
    assert_eq!(viem_core_view_remove(core.handle, writer), ViemStatus::Ok);
}

#[test]
fn latin1_unrepresentable_composition_commit_is_typed_and_non_destructive() {
    let core = create_core(
        b"caf\xe9",
        ViemDocumentOptions {
            encoding: VIEM_ENCODING_LATIN1,
            ..ViemDocumentOptions::default()
        },
    );
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let context_pointer = &mut *context as *mut FakeProviderContext;
    let (view, mut outcome) = add_test_view(&core, context_pointer);
    let begin = ViemCompositionBeginV1 {
        struct_size: VIEM_COMPOSITION_BEGIN_V1_SIZE,
        reserved: 0,
        document_revision: 0,
        replacement_start: "café".len() as u64,
        replacement_end: "café".len() as u64,
    };
    assert_eq!(
        unsafe { viem_core_view_composition_begin(core.handle, view, &begin, &mut outcome) },
        ViemStatus::Ok
    );
    let emoji = "😀".as_bytes();
    let commit = ViemCompositionCommitV1 {
        struct_size: VIEM_COMPOSITION_COMMIT_V1_SIZE,
        reserved: 0,
        document_revision: 0,
        committed_text: utf8_slice(emoji),
    };
    assert_eq!(
        unsafe { viem_core_view_composition_commit(core.handle, view, &commit, &mut outcome) },
        ViemStatus::UnrepresentableCharacter
    );
    assert_eq!(
        copy_core_bytes(viem_core_copy_source_bytes, &core, 0),
        b"caf\xe9"
    );
    assert_eq!(
        copy_core_bytes(viem_core_copy_formatted_utf8, &core, 0),
        "café".as_bytes()
    );
    let cancel = ViemCompositionCancelV1 {
        struct_size: VIEM_COMPOSITION_CANCEL_V1_SIZE,
        reserved: 0,
        document_revision: 0,
    };
    assert_eq!(
        unsafe { viem_core_view_composition_cancel(core.handle, view, &cancel, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
}

#[test]
fn native_direct_properties_and_decoration_queries_are_typed_exact_and_undoable() {
    use viem_core::document::{Document, Encoding, Format, ParagraphAlignment};
    for (format, source, adapter) in [
        (VIEM_FORMAT_RTF, r"{\rtf1 Alpha\par Beta}", Format::Rtf),
    ] {
        let core = create_core(
            source.as_bytes(),
            ViemDocumentOptions {
                format,
                ..Default::default()
            },
        );
        let mut provider = Box::new(FakeProviderContext::new(core.handle));
        let (view, mut outcome) = add_test_view(&core, provider.as_mut());
        let selection = || {
            let mut value = ViemLogicalSelectionIdentityV1::default();
            assert_eq!(
                unsafe { viem_core_view_list_selection(core.handle, view, &mut value) },
                ViemStatus::Ok
            );
            value
        };
        let bytes = || {
            copy_core_bytes(
                viem_core_copy_source_bytes,
                &core,
                document_state(&core).document_revision,
            )
        };
        let mut request = ViemDirectStyleEditV1 {
            struct_size: VIEM_DIRECT_STYLE_EDIT_V1_SIZE,
            operation: VIEM_STYLE_EDIT_SET_DECLARATION,
            property: VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT,
            expected_selection: selection(),
            value: ViemStyleEditValueV1 {
                kind: VIEM_STYLE_VALUE_PARAGRAPH_ALIGNMENT,
                enum_value: VIEM_STYLE_PARAGRAPH_ALIGNMENT_CENTER,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut invalid = request;
        invalid.value.kind = VIEM_STYLE_VALUE_BOOLEAN;
        assert_eq!(
            unsafe { viem_core_view_edit_direct_style(core.handle, view, &invalid, &mut outcome) },
            ViemStatus::InvalidStyleValue
        );
        invalid = request;
        invalid.property = u32::MAX;
        assert_eq!(
            unsafe { viem_core_view_edit_direct_style(core.handle, view, &invalid, &mut outcome) },
            ViemStatus::InvalidStyleValue
        );
        invalid = request;
        invalid.reserved = 1;
        assert_eq!(
            unsafe { viem_core_view_edit_direct_style(core.handle, view, &invalid, &mut outcome) },
            ViemStatus::InvalidArgument
        );
        assert_eq!(bytes(), source.as_bytes());
        assert_eq!(
            unsafe { viem_core_view_edit_direct_style(core.handle, view, &request, &mut outcome) },
            ViemStatus::Ok,
            "format {format}"
        );
        let reopened = Document::from_bytes(bytes(), Encoding::Utf8, adapter).unwrap();
        assert_eq!(
            reopened.projection().blocks()[0].direct_paragraph.alignment,
            Some(ParagraphAlignment::Center)
        );
        assert_eq!(
            reopened.projection().blocks()[1].direct_paragraph.alignment,
            None
        );
        assert_eq!(
            unsafe { viem_core_view_edit_direct_style(core.handle, view, &request, &mut outcome) },
            ViemStatus::StaleRevision
        );
        assert_eq!(
            unsafe { viem_core_view_undo(core.handle, view, &mut outcome) },
            ViemStatus::Ok
        );
        assert_eq!(bytes(), source.as_bytes());
        for character in ['v', 'l'] {
            assert_eq!(
                unsafe {
                    test_send_key(
                        core.handle,
                        view,
                        &key(VIEM_KEY_CHARACTER, character as u32),
                        &mut outcome,
                    )
                },
                ViemStatus::Ok
            );
        }
        request.expected_selection = selection();
        request.property = VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE;
        request.value = ViemStyleEditValueV1 {
            kind: VIEM_STYLE_VALUE_BOOLEAN,
            enum_value: 1,
            ..Default::default()
        };
        assert_eq!(
            unsafe {
                test_send_key(core.handle, view, &key(VIEM_KEY_CHARACTER, 'l' as u32), &mut outcome)
            },
            ViemStatus::Ok
        );
        assert_eq!(
            unsafe { viem_core_view_edit_direct_style(core.handle, view, &request, &mut outcome) },
            ViemStatus::StaleRevision
        );
        assert_eq!(bytes(), source.as_bytes());
        for property in [
            VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE,
            VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH,
        ] {
            assert_eq!(
                unsafe {
                    test_send_key(
                        core.handle,
                        view,
                        &key(VIEM_KEY_ESCAPE, 0),
                        &mut outcome,
                    )
                },
                ViemStatus::Ok
            );
            for character in ['0', 'v', 'l', 'l'] {
                assert_eq!(
                    unsafe {
                        test_send_key(
                            core.handle,
                            view,
                            &key(VIEM_KEY_CHARACTER, character as u32),
                            &mut outcome,
                        )
                    },
                    ViemStatus::Ok
                );
            }
            request.expected_selection = selection();
            request.property = property;
            request.value.enum_value = 1;
            let mut state = u32::MAX;
            assert_eq!(
                unsafe { viem_core_view_decoration_state(core.handle, view, property, &mut state) },
                ViemStatus::Ok
            );
            assert_eq!(state, VIEM_SEMANTIC_STYLE_STATE_OFF);
            assert_eq!(
                unsafe {
                    viem_core_view_edit_direct_style(core.handle, view, &request, &mut outcome)
                },
                ViemStatus::Ok,
                "format {format} property {property}"
            );
            assert_eq!(
                unsafe { viem_core_view_decoration_state(core.handle, view, property, &mut state) },
                ViemStatus::Ok
            );
            assert_eq!(state, VIEM_SEMANTIC_STYLE_STATE_ON);
            assert_eq!(
                unsafe {
                    test_send_key(
                        core.handle,
                        view,
                        &key(VIEM_KEY_CHARACTER, 'l' as u32),
                        &mut outcome,
                    )
                },
                ViemStatus::Ok
            );
            assert_eq!(
                unsafe { viem_core_view_decoration_state(core.handle, view, property, &mut state) },
                ViemStatus::Ok
            );
            assert_eq!(
                state,
                VIEM_SEMANTIC_STYLE_STATE_MIXED,
                "format {format} property {property} selection {:?} source {}",
                selection(),
                String::from_utf8_lossy(&bytes())
            );
            assert_eq!(
                unsafe {
                    test_send_key(core.handle, view, &key(VIEM_KEY_CHARACTER, 'h' as u32), &mut outcome)
                },
                ViemStatus::Ok
            );
            request.expected_selection = selection();
            request.value.enum_value = 0;
            assert_eq!(
                unsafe {
                    viem_core_view_edit_direct_style(core.handle, view, &request, &mut outcome)
                },
                ViemStatus::Ok
            );
            assert_eq!(
                unsafe { viem_core_view_decoration_state(core.handle, view, property, &mut state) },
                ViemStatus::Ok
            );
            assert_eq!(state, VIEM_SEMANTIC_STYLE_STATE_OFF);
            assert_eq!(
                unsafe { viem_core_view_undo(core.handle, view, &mut outcome) },
                ViemStatus::Ok
            );
            for character in ['0', 'v', 'l', 'l'] {
                assert_eq!(
                    unsafe {
                        test_send_key(
                            core.handle,
                            view,
                            &key(VIEM_KEY_CHARACTER, character as u32),
                            &mut outcome,
                        )
                    },
                    ViemStatus::Ok
                );
            }
            assert_eq!(
                unsafe { viem_core_view_decoration_state(core.handle, view, property, &mut state) },
                ViemStatus::Ok
            );
            assert_eq!(state, VIEM_SEMANTIC_STYLE_STATE_ON);
            assert_eq!(
                unsafe { viem_core_view_undo(core.handle, view, &mut outcome) },
                ViemStatus::Ok
            );
            assert_eq!(bytes(), source.as_bytes());
        }
        let mut sentinel = 919u32;
        assert_eq!(
            unsafe {
                viem_core_view_decoration_state(
                    core.handle,
                    view,
                    VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT,
                    &mut sentinel,
                )
            },
            ViemStatus::InvalidArgument
        );
        assert_eq!(sentinel, 919);
        assert_eq!(
            unsafe {
                viem_core_view_decoration_state(
                    core.handle,
                    view,
                    VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE,
                    std::ptr::null_mut(),
                )
            },
            ViemStatus::NullPointer
        );
    }
}

#[test]
fn rich_bold_and_italic_toggle_states_include_default_gaps() {
    for (format, source) in [

        (VIEM_FORMAT_RTF, r"{\rtf1{\b\i A}B}"),
    ] {
        let core = create_core(
            source.as_bytes(),
            ViemDocumentOptions {
                format,
                ..Default::default()
            },
        );
        let mut provider = Box::new(FakeProviderContext::new(core.handle));
        let (view, mut outcome) = add_test_view(&core, provider.as_mut());
        for character in ['v', 'l'] {
            assert_eq!(
                unsafe {
                    test_send_key(
                        core.handle,
                        view,
                        &key(VIEM_KEY_CHARACTER, character as u32),
                        &mut outcome,
                    )
                },
                ViemStatus::Ok
            );
        }
        for style in [VIEM_SEMANTIC_STYLE_STRONG, VIEM_SEMANTIC_STYLE_EMPHASIS] {
            let mut presentation = ViemSemanticStylePresentationV1::default();
            assert_eq!(
                unsafe {
                    viem_core_view_semantic_style_presentation(
                        core.handle,
                        view,
                        style,
                        &mut presentation,
                    )
                },
                ViemStatus::Ok
            );
            assert_eq!(
                presentation.state, VIEM_SEMANTIC_STYLE_STATE_MIXED,
                "format {format} style {style}"
            );
        }
    }
}

#[test]
fn rich_decoration_state_remains_on_after_select_all_linewise_toggle() {
    for (format, source) in [

        (VIEM_FORMAT_RTF, r"{\rtf1{\b Words}{\*\opaque keep}}"),
    ] {
        let core = create_core(
            source.as_bytes(),
            ViemDocumentOptions {
                format,
                ..Default::default()
            },
        );
        let mut provider = Box::new(FakeProviderContext::new(core.handle));
        let (view, mut outcome) = add_test_view(&core, provider.as_mut());
        for character in ['g', 'g', 'V', 'G'] {
            assert_eq!(
                unsafe {
                    test_send_key(
                        core.handle,
                        view,
                        &key(VIEM_KEY_CHARACTER, character as u32),
                        &mut outcome,
                    )
                },
                ViemStatus::Ok
            );
        }
        let mut expected = ViemLogicalSelectionIdentityV1::default();
        assert_eq!(
            unsafe { viem_core_view_list_selection(core.handle, view, &mut expected) },
            ViemStatus::Ok
        );
        let request = ViemDirectStyleEditV1 {
            struct_size: VIEM_DIRECT_STYLE_EDIT_V1_SIZE,
            operation: VIEM_STYLE_EDIT_SET_DECLARATION,
            property: VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE,
            expected_selection: expected,
            value: ViemStyleEditValueV1 {
                kind: VIEM_STYLE_VALUE_BOOLEAN,
                enum_value: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            unsafe { viem_core_view_edit_direct_style(core.handle, view, &request, &mut outcome) },
            ViemStatus::Ok
        );
        let mut state = u32::MAX;
        assert_eq!(
            unsafe {
                viem_core_view_decoration_state(core.handle, view, request.property, &mut state)
            },
            ViemStatus::Ok
        );
        assert_eq!(state, VIEM_SEMANTIC_STYLE_STATE_ON, "format {format}");
    }
}

#[test]
fn checked_line_mode_and_location_queries_are_view_local_and_do_not_edit() {
    assert_eq!(std::mem::size_of::<ViemViewLineLocationV1>(), 48);
    let core = create_core(b"abcdef\nsecond", ViemDocumentOptions::default());
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *provider);
    let (second, _) = add_test_view(&core, &mut *provider);
    let mut mode = 99;
    assert_eq!(
        unsafe { viem_core_view_line_mode(core.handle, view, &mut mode) },
        ViemStatus::Ok
    );
    assert_eq!(mode, 0);
    let mut location = ViemViewLineLocationV1::default();
    assert_eq!(
        unsafe { viem_core_view_line_location(core.handle, view, &mut location) },
        ViemStatus::Ok
    );
    assert_eq!(
        (
            location.line,
            location.column,
            location.hard_line,
            location.fragment
        ),
        (1, 1, 1, 1)
    );
    assert_eq!(
        location.flags,
        VIEM_LINE_LOCATION_GLOBAL_LINE_EXACT | VIEM_LINE_LOCATION_FRAGMENT_EXACT
    );
    assert_eq!(
        unsafe { viem_core_view_set_line_mode(core.handle, view, 1, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_line_mode(core.handle, second, &mut mode) },
        ViemStatus::Ok
    );
    assert_eq!(mode, 0);
    assert_eq!(
        unsafe { viem_core_view_line_location(core.handle, view, &mut location) },
        ViemStatus::Ok
    );
    assert_eq!(location.mode, 1);
    let revision = document_state(&core).document_revision;
    assert_eq!(
        unsafe { viem_core_view_set_line_mode(core.handle, view, 77, &mut outcome) },
        ViemStatus::InvalidArgument
    );
    assert_eq!(
        unsafe { viem_core_view_line_location(core.handle, u64::MAX, &mut location) },
        ViemStatus::InvalidView
    );
    assert_eq!(
        unsafe { viem_core_view_line_location(core.handle, view, ptr::null_mut()) },
        ViemStatus::InvalidArgument
    );
    assert_eq!(document_state(&core).document_revision, revision);
    assert_eq!(
        copy_core_bytes(viem_core_copy_source_bytes, &core, revision),
        b"abcdef\nsecond"
    );
}

#[test]
fn typography_export_is_exact_batched_stale_checked_and_includes_mixed_default_gaps() {
    let core = create_core(
        br"{\rtf1{\fonttbl{\f0 Arial;}}\f0\fs40{\b A}B}",
        ViemDocumentOptions {
            format: VIEM_FORMAT_RTF,
            ..Default::default()
        },
    );
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, provider.as_mut());
    for character in ['v', 'l'] {
        assert_eq!(
            unsafe {
                test_send_key(
                    core.handle,
                    view,
                    &key(VIEM_KEY_CHARACTER, character as u32),
                    &mut outcome,
                )
            },
            ViemStatus::Ok
        );
    }
    let mut info = ViemTypographyInfoV1::default();
    assert_eq!(
        unsafe {
            viem_core_view_typography_export(
                core.handle,
                view,
                outcome.document_revision,
                &mut info,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                0,
            )
        },
        ViemStatus::BufferTooSmall
    );
    assert_eq!(info.flags & 3, 3);
    assert_eq!(info.size, 20.0);
    assert_eq!(info.base_weight, 400);
    assert_eq!(info.weight, 700);
    let mut family = vec![0; info.font_family_bytes as usize];
    let mut features = vec![ViemOpenTypeFeatureV1::default(); info.feature_count as usize];
    assert_eq!(
        unsafe {
            viem_core_view_typography_export(
                core.handle,
                view,
                outcome.document_revision,
                &mut info,
                family.as_mut_ptr(),
                family.len() as u64,
                features.as_mut_ptr(),
                features.len() as u64,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(family, b"Arial");
    family.fill(0xFF);
    assert_eq!(
        unsafe {
            viem_core_view_typography_export(
                core.handle,
                view,
                outcome.document_revision + 1,
                &mut info,
                family.as_mut_ptr(),
                family.len() as u64,
                features.as_mut_ptr(),
                features.len() as u64,
            )
        },
        ViemStatus::StaleRevision
    );
    assert!(family.iter().all(|byte| *byte == 0xFF));
    assert_eq!(
        unsafe {
            viem_core_view_typography_export(
                core.handle,
                view,
                outcome.document_revision,
                &mut info,
                (&mut info as *mut ViemTypographyInfoV1).cast(),
                1,
                ptr::null_mut(),
                0,
            )
        },
        ViemStatus::InvalidArgument
    );
}

#[test]
fn direct_character_batch_is_atomic_and_rejects_duplicate_stale_or_overlapping_requests() {
    let original = br"{\rtf1 Text}{\*\unknown keep}";
    let core = create_core(
        original,
        ViemDocumentOptions {
            format: VIEM_FORMAT_RTF,
            ..Default::default()
        },
    );
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, provider.as_mut());
    for character in ['v', 'e'] {
        assert_eq!(
            unsafe {
                test_send_key(
                    core.handle,
                    view,
                    &key(VIEM_KEY_CHARACTER, character as u32),
                    &mut outcome,
                )
            },
            ViemStatus::Ok
        );
    }
    let mut selection = ViemLogicalSelectionIdentityV1::default();
    assert_eq!(
        unsafe { viem_core_view_list_selection(core.handle, view, &mut selection) },
        ViemStatus::Ok
    );
    let make = |property, value| ViemDirectStyleEditV1 {
        struct_size: VIEM_DIRECT_STYLE_EDIT_V1_SIZE,
        operation: VIEM_STYLE_EDIT_SET_DECLARATION,
        property,
        value,
        expected_selection: selection,
        ..Default::default()
    };
    let requests = [
        make(
            VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT,
            ViemStyleEditValueV1 {
                kind: VIEM_STYLE_VALUE_UNSIGNED,
                enum_value: 200,
                ..Default::default()
            },
        ),
        make(
            VIEM_STYLE_PROPERTY_CHARACTER_BOLD,
            ViemStyleEditValueV1 {
                kind: VIEM_STYLE_VALUE_BOOLEAN,
                enum_value: 1,
                ..Default::default()
            },
        ),
        make(
            VIEM_STYLE_PROPERTY_CHARACTER_SIZE,
            ViemStyleEditValueV1 {
                kind: VIEM_STYLE_VALUE_FLOAT,
                number: 24.0,
                ..Default::default()
            },
        ),
    ];
    let duplicate = [requests[0], requests[0]];
    assert_eq!(
        unsafe {
            viem_core_view_edit_direct_character_batch(
                core.handle,
                view,
                duplicate.as_ptr(),
                2,
                &mut outcome,
            )
        },
        ViemStatus::InvalidArgument
    );
    assert_eq!(
        copy_core_bytes(
            viem_core_copy_source_bytes,
            &core,
            document_state(&core).document_revision
        ),
        original
    );
    assert_eq!(
        unsafe {
            viem_core_view_edit_direct_character_batch(
                core.handle,
                view,
                requests.as_ptr(),
                3,
                requests.as_ptr().cast_mut().cast(),
            )
        },
        ViemStatus::InvalidArgument
    );
    assert_eq!(
        unsafe {
            viem_core_view_edit_direct_character_batch(
                core.handle,
                view,
                requests.as_ptr(),
                3,
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    let changed_revision = outcome.document_revision;
    assert_eq!(
        unsafe {
            viem_core_view_edit_direct_character_batch(
                core.handle,
                view,
                requests.as_ptr(),
                3,
                &mut outcome,
            )
        },
        ViemStatus::StaleRevision
    );
    let mut info = ViemTypographyInfoV1::default();
    assert_eq!(
        unsafe {
            viem_core_view_typography_export(
                core.handle,
                view,
                changed_revision,
                &mut info,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                0,
            )
        },
        ViemStatus::BufferTooSmall
    );
    assert_eq!((info.base_weight, info.weight, info.size), (200, 500, 24.0));
    assert_eq!(
        unsafe {
            test_send_key(core.handle, view, &key(VIEM_KEY_ESCAPE, 0), &mut outcome)
        },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, 'u' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(
        copy_core_bytes(
            viem_core_copy_source_bytes,
            &core,
            outcome.document_revision
        ),
        original
    );
}

#[test]
fn readonly_and_recovered_flags_are_exact_buffer_policies_and_save_clears_recovery() {
    let core = create_core(b"recovered", ViemDocumentOptions::default());
    let initial = document_state(&core);
    assert_eq!(
        viem_core_set_read_only(
            core.handle,
            initial.document_id,
            initial.document_revision,
            2
        ),
        ViemStatus::InvalidArgument
    );
    assert_eq!(
        viem_core_set_read_only(
            core.handle,
            initial.document_id + 1,
            initial.document_revision,
            1
        ),
        ViemStatus::InvalidArgument
    );
    assert_eq!(
        viem_core_mark_recovered(
            core.handle,
            initial.document_id,
            initial.document_revision + 1
        ),
        ViemStatus::StaleRevision
    );
    assert_eq!(document_state(&core), initial);
    assert_eq!(
        viem_core_set_read_only(
            core.handle,
            initial.document_id,
            initial.document_revision,
            1
        ),
        ViemStatus::Ok
    );
    let readonly = document_state(&core);
    assert_eq!(readonly.document_revision, initial.document_revision);
    assert_eq!(readonly.flags & VIEM_DOCUMENT_STATE_IS_DIRTY, 0);
    assert_ne!(readonly.flags & VIEM_DOCUMENT_STATE_READ_ONLY, 0);
    assert_eq!(
        viem_core_mark_recovered(core.handle, initial.document_id, initial.document_revision),
        ViemStatus::Ok
    );
    let recovered = document_state(&core);
    assert_ne!(recovered.flags & VIEM_DOCUMENT_STATE_IS_DIRTY, 0);
    assert_ne!(recovered.flags & VIEM_DOCUMENT_STATE_RECOVERED, 0);
    assert_eq!(recovered.flags & VIEM_DOCUMENT_STATE_CAN_UNDO, 0);
    let saved = ViemMarkSavedV1 {
        struct_size: VIEM_MARK_SAVED_V1_SIZE,
        document_id: initial.document_id,
        document_revision: initial.document_revision,
        ..ViemMarkSavedV1::default()
    };
    assert_eq!(
        unsafe { viem_core_mark_saved(core.handle, &saved) },
        ViemStatus::Ok
    );
    let result = document_state(&core);
    assert_eq!(
        result.flags & (VIEM_DOCUMENT_STATE_IS_DIRTY | VIEM_DOCUMENT_STATE_RECOVERED),
        0
    );
    assert_ne!(result.flags & VIEM_DOCUMENT_STATE_READ_ONLY, 0);
    assert_eq!(
        copy_core_bytes(
            viem_core_copy_source_bytes,
            &core,
            initial.document_revision
        ),
        b"recovered"
    );
}

#[test]
fn readonly_ex_error_has_a_distinct_abi_status_and_no_host_write_effect() {
    let core = create_core(b"Text", ViemDocumentOptions::default());
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, _) = add_test_view(&core, &mut *provider);
    let state = document_state(&core);
    assert_eq!(
        viem_core_set_read_only(core.handle, state.document_id, state.document_revision, 1),
        ViemStatus::Ok
    );
    host_chars(&core, view, ":w");
    let (status, outcome, effects) = host_key(&core, view, key(VIEM_KEY_ENTER, 0), &[]);
    assert_eq!(status, ViemStatus::Ok);
    assert_eq!(outcome.command_status, VIEM_COMMAND_STATUS_READ_ONLY);
    assert_ne!(effects, 0);
    let diagnostic = copy_effect_batch(effects);
    assert_eq!(diagnostic.ex_requests.len(), 1);
    assert_eq!(diagnostic.ex_requests[0].kind, VIEM_EX_FRONTEND_MESSAGE);
    assert!(diagnostic
        .text(diagnostic.ex_requests[0].text)
        .contains("E45"));
    assert!(diagnostic.clipboard_writes.is_empty());
    assert_eq!(viem_effect_batch_release(effects), ViemStatus::Ok);
    host_chars(&core, view, ":w!");
    let (status, outcome, effects) = host_key(&core, view, key(VIEM_KEY_ENTER, 0), &[]);
    assert_eq!(status, ViemStatus::Ok);
    assert_eq!(outcome.command_status, VIEM_COMMAND_STATUS_COMPLETE);
    assert_ne!(effects, 0);
    assert_eq!(viem_effect_batch_release(effects), ViemStatus::Ok);
}

#[test]
fn ranged_source_export_preserves_delimiters_and_rejects_stale_identity() {
    let core = create_core(
        b"first\r\n\r\n**second**\r\n\r\nlast",
        ViemDocumentOptions {
            format: VIEM_FORMAT_MARKDOWN,
            ..ViemDocumentOptions::default()
        },
    );
    let state = document_state(&core);
    let mut required = 0;
    let mut complete = 99;
    assert_eq!(
        unsafe {
            viem_core_copy_hard_line_source_bytes(
                core.handle,
                state.document_id,
                state.document_revision,
                1,
                2,
                ptr::null_mut(),
                0,
                &mut required,
                &mut complete,
            )
        },
        ViemStatus::BufferTooSmall
    );
    assert_eq!(required, b"**second**\r\n\r\n".len() as u64);
    assert_eq!(complete, 0);
    let mut bytes = vec![0; required as usize];
    assert_eq!(
        unsafe {
            viem_core_copy_hard_line_source_bytes(
                core.handle,
                state.document_id,
                state.document_revision,
                1,
                2,
                bytes.as_mut_ptr(),
                bytes.len() as u64,
                &mut required,
                &mut complete,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(bytes, b"**second**\r\n\r\n");
    assert_eq!(
        unsafe {
            viem_core_copy_hard_line_source_bytes(
                core.handle,
                state.document_id + 1,
                state.document_revision,
                1,
                2,
                ptr::null_mut(),
                0,
                &mut required,
                &mut complete,
            )
        },
        ViemStatus::InvalidArgument
    );
    assert_eq!(
        unsafe {
            viem_core_copy_hard_line_source_bytes(
                core.handle,
                state.document_id,
                state.document_revision + 1,
                1,
                2,
                ptr::null_mut(),
                0,
                &mut required,
                &mut complete,
            )
        },
        ViemStatus::StaleRevision
    );
    assert_eq!(
        unsafe {
            viem_core_copy_hard_line_source_bytes(
                core.handle,
                state.document_id,
                state.document_revision,
                2,
                1,
                ptr::null_mut(),
                0,
                &mut required,
                &mut complete,
            )
        },
        ViemStatus::PolicyRequired
    );
    assert_eq!(document_state(&core), state);
}

#[test]
fn native_format_setter_returns_owned_loss_warning_and_stale_retry_is_inert() {
    let core = create_core(
        br"{\rtf1 Body}{\*\unknown preserved until conversion}",
        ViemDocumentOptions {
            format: VIEM_FORMAT_RTF,
            ..ViemDocumentOptions::default()
        },
    );
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, _) = add_test_view(&core, &mut *provider);
    let state = document_state(&core);
    let request = ViemSetFormatV1 {
        struct_size: VIEM_SET_FORMAT_V1_SIZE,
        format: VIEM_FORMAT_MARKDOWN,
        operation: VIEM_FORMAT_OPERATION_CONVERT,
        reserved: 0,
        document_id: state.document_id,
        document_revision: state.document_revision,
    };
    let mut outcome = ViemCoreOutcomeV1::default();
    let mut effects = 0;
    assert_eq!(
        unsafe {
            viem_core_view_set_format_with_effects(
                core.handle,
                view,
                &request,
                &mut outcome,
                &mut effects,
            )
        },
        ViemStatus::Ok
    );
    assert_ne!(effects, 0);
    let batch = copy_effect_batch(effects);
    assert_eq!(batch.ex_requests.len(), 1);
    assert_eq!(batch.ex_requests[0].kind, VIEM_EX_FRONTEND_MESSAGE);
    assert!(batch
        .text(batch.ex_requests[0].text)
        .contains("information was lost"));
    assert_eq!(viem_effect_batch_release(effects), ViemStatus::Ok);
    let committed = document_state(&core);
    assert_eq!(
        unsafe {
            viem_core_view_set_format_with_effects(
                core.handle,
                view,
                &request,
                &mut outcome,
                &mut effects,
            )
        },
        ViemStatus::StaleRevision
    );
    assert_eq!(effects, 0);
    assert_eq!(document_state(&core), committed);
}

#[test]
fn current_core_replacement_checks_graphemes_and_payloads_before_mutating_source() {
    let source = "ae\u{301}z";
    let core = create_core(source.as_bytes(), ViemDocumentOptions::default());
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, provider.as_mut());
    let begin = ViemCompositionBeginV1 {
        struct_size: VIEM_COMPOSITION_BEGIN_V1_SIZE,
        reserved: 0,
        document_revision: 0,
        replacement_start: 1,
        replacement_end: 4,
    };
    for (start, end, expected) in [
        (2, 2, ViemStatus::NotGraphemeBoundary),
        (4, 1, ViemStatus::InvalidRange),
    ] {
        let invalid = ViemCompositionBeginV1 {
            replacement_start: start,
            replacement_end: end,
            ..begin
        };
        assert_eq!(
            unsafe { viem_core_view_composition_begin(core.handle, view, &invalid, &mut outcome) },
            expected
        );
        assert_eq!(
            copy_core_bytes(viem_core_copy_source_bytes, &core, 0),
            source.as_bytes()
        );
    }
    assert_eq!(
        unsafe { viem_core_view_composition_begin(core.handle, view, &begin, &mut outcome) },
        ViemStatus::Ok
    );
    for (text, expected) in [
        (utf8_slice(&[0xff]), ViemStatus::InvalidUtf8),
        (
            ViemUtf8Slice {
                data: ptr::null(),
                length: 1,
            },
            ViemStatus::NullPointer,
        ),
    ] {
        let invalid = ViemCompositionCommitV1 {
            struct_size: VIEM_COMPOSITION_COMMIT_V1_SIZE,
            reserved: 0,
            document_revision: 0,
            committed_text: text,
        };
        assert_eq!(
            unsafe { viem_core_view_composition_commit(core.handle, view, &invalid, &mut outcome) },
            expected
        );
        assert_eq!(
            copy_core_bytes(viem_core_copy_source_bytes, &core, 0),
            source.as_bytes()
        );
    }
    let valid = ViemCompositionCommitV1 {
        struct_size: VIEM_COMPOSITION_COMMIT_V1_SIZE,
        reserved: 0,
        document_revision: 0,
        committed_text: utf8_slice("ø".as_bytes()),
    };
    assert_eq!(
        unsafe { viem_core_view_composition_commit(core.handle, view, &valid, &mut outcome) },
        ViemStatus::Ok
    );
    assert_eq!(outcome.document_revision, 1);
    assert_eq!(
        copy_core_bytes(viem_core_copy_source_bytes, &core, 1),
        "aøz".as_bytes()
    );
    assert_eq!(
        unsafe { viem_core_view_composition_begin(core.handle, view, &begin, &mut outcome) },
        ViemStatus::StaleRevision
    );
    assert_eq!(
        copy_core_bytes(viem_core_copy_source_bytes, &core, 1),
        "aøz".as_bytes()
    );
    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
}

#[test]
fn current_host_context_checks_targets_counts_and_nested_output_aliases() {
    let core = create_core(b"text", ViemDocumentOptions::default());
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, _) = add_test_view(&core, provider.as_mut());
    let entry = ViemClipboardTurnEntryV2 {
        target: VIEM_CLIPBOARD_TARGET_CLIPBOARD,
        flags: VIEM_CLIPBOARD_TURN_HAS_READ,
        generation: 1,
        plain_text: utf8_slice(b"clipboard"),
        ..ViemClipboardTurnEntryV2::default()
    };
    let (status, _, effects) = host_key(
        &core,
        view,
        key(VIEM_KEY_CHARACTER, 'x' as u32),
        &[entry, entry],
    );
    assert_eq!(status, ViemStatus::InvalidArgument);
    assert_eq!(effects, 0);
    for count in [3, u64::MAX] {
        // Invalid counts must be rejected before following the null array.
        let context = ViemCommandTurnContextV2 {
            clipboard_count: count,
            ..ViemCommandTurnContextV2::default()
        };
        let mut outcome = ViemCoreOutcomeV1::default();
        let mut effects = u64::MAX;
        assert_eq!(
            unsafe {
                viem_core_view_send_text_with_host_context_v2(
                    core.handle,
                    view,
                    b"x".as_ptr(),
                    1,
                    &context,
                    &mut outcome,
                    &mut effects,
                )
            },
            ViemStatus::InvalidArgument
        );
        assert_eq!(effects, 0);
    }
    for alias_fragment in [false, true] {
        let mut outcome = ViemCoreOutcomeV1::default();
        let alias = ViemUtf8Slice {
            data: (&outcome as *const ViemCoreOutcomeV1).cast(),
            length: 1,
        };
        let entry = if alias_fragment {
            ViemClipboardTurnEntryV2 {
                fragment_json: alias,
                ..entry
            }
        } else {
            ViemClipboardTurnEntryV2 {
                plain_text: alias,
                ..entry
            }
        };
        let context = ViemCommandTurnContextV2 {
            clipboards: &entry,
            clipboard_count: 1,
            ..ViemCommandTurnContextV2::default()
        };
        let mut effects = 0;
        // Deliberately overlap a nested readable field with writable output;
        // the ABI validates the relation before interpreting its bytes.
        assert_eq!(
            unsafe {
                viem_core_view_send_key_with_host_context_v2(
                    core.handle,
                    view,
                    &key(VIEM_KEY_CHARACTER, 'x' as u32),
                    &context,
                    &mut outcome,
                    &mut effects,
                )
            },
            ViemStatus::InvalidArgument
        );
        assert_eq!(effects, 0);
    }
    assert_eq!(document_state(&core).document_revision, 0);
    assert_eq!(
        copy_core_bytes(viem_core_copy_source_bytes, &core, 0),
        b"text"
    );
    assert_eq!(viem_core_view_remove(core.handle, view), ViemStatus::Ok);
}

/// The caret's meaning is exported, not re-derived by a frontend. A drawing
/// layer that picked a character from the cursor offset plus the boundary
/// affinity drew Normal mode's block one grapheme early after `$`.
#[test]
fn view_presentation_exports_what_the_caret_occupies() {
    let core = create_core("abcde\u{301}\nnext".as_bytes(), ViemDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, _) = add_test_view(&core, &mut *context);
    let mut outcome = ViemCoreOutcomeV1::default();
    let mut presentation = ViemViewPresentationV1::default();
    let read = |presentation: &mut ViemViewPresentationV1| {
        assert_eq!(
            unsafe { viem_core_view_presentation(core.handle, view, presentation) },
            ViemStatus::Ok
        );
    };
    read(&mut presentation);
    assert_eq!(presentation.caret_shape, VIEM_CARET_SHAPE_CELL);
    assert_eq!(presentation.caret_utf8_start, 0);
    assert_eq!(presentation.caret_utf8_end, 1);

    for input in [key(VIEM_KEY_CHARACTER, '$' as u32), key(VIEM_KEY_END, 0)] {
        assert_eq!(
            unsafe { test_send_key(core.handle, view, &key(VIEM_KEY_CHARACTER, '0' as u32), &mut outcome) },
            ViemStatus::Ok
        );
        assert_eq!(
            unsafe { test_send_key(core.handle, view, &input, &mut outcome) },
            ViemStatus::Ok
        );
        read(&mut presentation);
        // The model keeps its upstream boundary affinity, and the exported
        // cell still covers the last grapheme rather than the one before it.
        assert_eq!(presentation.cursor_affinity, VIEM_BOUNDARY_AFFINITY_UPSTREAM);
        assert_eq!(presentation.caret_shape, VIEM_CARET_SHAPE_CELL);
        assert_eq!(presentation.caret_utf8_start, presentation.cursor_utf8_offset);
        assert_eq!(presentation.caret_utf8_start, 4);
        assert_eq!(presentation.caret_utf8_end, 7, "the combining cluster is whole");
    }

    // Insert mode addresses the gap before that grapheme.
    assert_eq!(
        unsafe { test_send_key(core.handle, view, &key(VIEM_KEY_CHARACTER, 'i' as u32), &mut outcome) },
        ViemStatus::Ok
    );
    read(&mut presentation);
    assert_eq!(presentation.mode, VIEM_MODE_INSERT);
    assert_eq!(presentation.caret_shape, VIEM_CARET_SHAPE_BOUNDARY);
    assert_eq!(presentation.caret_utf8_start, 4);
    assert_eq!(presentation.caret_utf8_end, presentation.caret_utf8_start);
}

#[test]
fn link_destination_ffi_checks_snapshot_boundaries_and_output_aliases() {
    let core = create_core("[café](https://example.com/é) tail".as_bytes(), ViemDocumentOptions {
        format: VIEM_FORMAT_MARKDOWN,
        ..ViemDocumentOptions::default()
    });
    let state = document_state(&core);
    let mut required = 0;
    let mut found = 0;
    let query = |offset, revision, output, capacity, required, found| unsafe {
        viem_core_copy_link_destination(core.handle, state.document_id, revision,
            offset, output, capacity, required, found)
    };
    assert_eq!(query(0, core.revision, ptr::null_mut(), 0, &mut required, &mut found), ViemStatus::BufferTooSmall);
    assert_eq!(found, 1);
    let mut bytes = vec![0; required as usize];
    assert_eq!(query(0, core.revision, bytes.as_mut_ptr(), bytes.len() as u64, &mut required, &mut found), ViemStatus::Ok);
    assert_eq!(String::from_utf8(bytes).unwrap(), "https://example.com/é");
    assert_eq!(query(5, core.revision, ptr::null_mut(), 0, &mut required, &mut found), ViemStatus::Ok);
    assert_eq!((required, found), (0, 0));
    assert_eq!(query(4, core.revision, ptr::null_mut(), 0, &mut required, &mut found), ViemStatus::NotGraphemeBoundary);
    assert_eq!(query(0, core.revision + 1, ptr::null_mut(), 0, &mut required, &mut found), ViemStatus::StaleRevision);
    let alias = &mut required as *mut u64;
    assert_eq!(query(0, core.revision, alias.cast(), 8, alias, &mut found), ViemStatus::InvalidArgument);
    assert_eq!(query(0, core.revision, ptr::null_mut(), 0, ptr::null_mut(), &mut found), ViemStatus::NullPointer);
    assert_eq!(unsafe { viem_core_copy_link_destination(core.handle, state.document_id + 1,
        core.revision, 0, ptr::null_mut(), 0, &mut required, &mut found) }, ViemStatus::InvalidArgument);
}

#[test]
fn script_position_abi_uses_an_enum_and_typography_exports_current_colors() {
    let source = br"{\rtf1{\colortbl;\red18\green52\blue86;}{\highlight1\super A}B}";
    let core = create_core(source, ViemDocumentOptions { format: VIEM_FORMAT_RTF, ..Default::default() });
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, provider.as_mut());
    let mut info = ViemTypographyInfoV1::default();
    assert_eq!(unsafe { viem_core_view_typography_export(core.handle, view, outcome.document_revision, &mut info, ptr::null_mut(), 0, ptr::null_mut(), 0) }, ViemStatus::BufferTooSmall);
    assert_eq!(info.script_position, VIEM_SCRIPT_POSITION_SUPERSCRIPT);
    assert_eq!(info.has_background, 1);
    assert_eq!(info.background.red, 0x12 as f32 / 255.0);
    assert_eq!(info.background.blue, 0x56 as f32 / 255.0);
    assert_eq!(unsafe { test_send_key(core.handle, view, &key(VIEM_KEY_CHARACTER, 'v' as u32), &mut outcome) }, ViemStatus::Ok);
    let mut selection = ViemLogicalSelectionIdentityV1::default();
    assert_eq!(unsafe { viem_core_view_list_selection(core.handle, view, &mut selection) }, ViemStatus::Ok);
    let mut request = ViemDirectStyleEditV1 {
        struct_size: VIEM_DIRECT_STYLE_EDIT_V1_SIZE,
        operation: VIEM_STYLE_EDIT_SET_DECLARATION,
        property: VIEM_STYLE_PROPERTY_CHARACTER_SCRIPT_POSITION,
        expected_selection: selection,
        value: ViemStyleEditValueV1 { kind: VIEM_STYLE_VALUE_SCRIPT_POSITION, enum_value: 99, ..Default::default() },
        ..Default::default()
    };
    assert_eq!(unsafe { viem_core_view_edit_direct_style(core.handle, view, &request, &mut outcome) }, ViemStatus::InvalidStyleValue);
    request.value.kind = VIEM_STYLE_VALUE_FLOAT;
    request.value.number = 4.0;
    assert_eq!(unsafe { viem_core_view_edit_direct_style(core.handle, view, &request, &mut outcome) }, ViemStatus::InvalidStyleValue);
    assert_eq!(copy_core_bytes(viem_core_copy_source_bytes, &core, document_state(&core).document_revision), source);
    request.value = ViemStyleEditValueV1 { kind: VIEM_STYLE_VALUE_SCRIPT_POSITION, enum_value: VIEM_SCRIPT_POSITION_SUBSCRIPT, ..Default::default() };
    assert_eq!(unsafe { viem_core_view_edit_direct_style(core.handle, view, &request, &mut outcome) }, ViemStatus::Ok);
    assert_eq!(unsafe { viem_core_view_typography_export(core.handle, view, outcome.document_revision, &mut info, ptr::null_mut(), 0, ptr::null_mut(), 0) }, ViemStatus::BufferTooSmall);
    assert_eq!(info.script_position, VIEM_SCRIPT_POSITION_SUBSCRIPT);
    assert_eq!(info.has_background, 1);
    assert_eq!(unsafe { viem_core_view_undo(core.handle, view, &mut outcome) }, ViemStatus::Ok);
    assert_eq!(copy_core_bytes(viem_core_copy_source_bytes, &core, outcome.document_revision), source);
}

#[test]
fn formatting_batch_and_snapshot_check_sizes_nested_aliases_and_exact_two_pass_identity() {
    let source = br"{\rtf1 A}";
    let core = create_core(source, ViemDocumentOptions { format: VIEM_FORMAT_RTF, ..Default::default() });
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, provider.as_mut());
    assert_eq!(unsafe { test_send_key(core.handle, view, &key(VIEM_KEY_CHARACTER, 'v' as u32), &mut outcome) }, ViemStatus::Ok);
    let mut selection = ViemLogicalSelectionIdentityV1::default();
    assert_eq!(unsafe { viem_core_view_list_selection(core.handle, view, &mut selection) }, ViemStatus::Ok);
    let valid = ViemDirectStyleEditV1 { struct_size: VIEM_DIRECT_STYLE_EDIT_V1_SIZE,
        operation: VIEM_STYLE_EDIT_SET_DECLARATION, property: VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE,
        expected_selection: selection, value: ViemStyleEditValueV1 { kind: VIEM_STYLE_VALUE_BOOLEAN, enum_value: 1, ..Default::default() }, ..Default::default() };
    for field in 0..3 {
        let mut bad = valid;
        match field { 0 => bad.struct_size = 0, 1 => bad.value.struct_size = 0, _ => bad.expected_selection.struct_size = 0 }
        outcome.flags = 0xA5;
        assert_eq!(unsafe { viem_core_view_edit_direct_properties(core.handle, view, &bad, 1, &mut outcome) }, ViemStatus::InvalidArgument);
        assert_eq!(outcome.flags, 0xA5);
    }
    let duplicates = [valid, valid];
    assert_eq!(unsafe { viem_core_view_edit_direct_properties(core.handle, view, duplicates.as_ptr(), 2, &mut outcome) }, ViemStatus::InvalidArgument);
    assert_eq!(unsafe { viem_core_view_edit_direct_properties(core.handle, view, &valid, 1, (&valid as *const ViemDirectStyleEditV1).cast_mut().cast()) }, ViemStatus::InvalidArgument);
    let mut nested = valid;
    nested.property = VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES;
    nested.value = ViemStyleEditValueV1 { kind: VIEM_STYLE_VALUE_STRING_LIST, item_count: 1,
        items: (&mut outcome as *mut ViemCoreOutcomeV1).cast(), ..Default::default() };
    assert_eq!(unsafe { viem_core_view_edit_direct_properties(core.handle, view, &nested, 1, &mut outcome) }, ViemStatus::InvalidArgument);
    let item = ViemStyleEditValueItemV1 { kind: VIEM_STYLE_VALUE_ITEM_STRING,
        text: ViemUtf8Slice { data: (&outcome as *const ViemCoreOutcomeV1).cast(), length: 1 }, ..Default::default() };
    nested.value.items = &item;
    assert_eq!(unsafe { viem_core_view_edit_direct_properties(core.handle, view, &nested, 1, &mut outcome) }, ViemStatus::InvalidArgument);
    assert_eq!(outcome.flags, 0xA5);
    assert_eq!(copy_core_bytes(viem_core_copy_source_bytes, &core, document_state(&core).document_revision), source);

    let mut info = ViemStyleSheetInfoV1::default();
    assert_eq!(unsafe { viem_core_view_copy_formatting(core.handle, view, &selection, &mut info, ptr::null_mut(), 0, ptr::null_mut(), 0, ptr::null_mut(), 0) }, ViemStatus::BufferTooSmall);
    let sentinel = ViemStylePropertyV1 { property: u32::MAX, ..Default::default() };
    let mut properties = vec![sentinel; info.property_count as usize];
    let mut items = vec![ViemStyleValueItemV1::default(); info.value_item_count as usize];
    let mut strings = vec![0xA5; info.string_bytes as usize];
    assert_eq!(unsafe { viem_core_view_copy_formatting(core.handle, view, &selection, &mut info, properties.as_mut_ptr(), properties.len() as u64 - 1, items.as_mut_ptr(), items.len() as u64, strings.as_mut_ptr(), strings.len() as u64) }, ViemStatus::BufferTooSmall);
    assert!(properties.iter().all(|property| *property == sentinel));
    assert!(strings.iter().all(|byte| *byte == 0xA5));
    let before_info = info;
    let mut stale = selection; stale.document_revision += 1;
    assert_eq!(unsafe { viem_core_view_copy_formatting(core.handle, view, &stale, &mut info, properties.as_mut_ptr(), properties.len() as u64, items.as_mut_ptr(), items.len() as u64, strings.as_mut_ptr(), strings.len() as u64) }, ViemStatus::StaleRevision);
    assert_eq!(info, before_info);
    assert!(properties.iter().all(|property| *property == sentinel));
    let mut malformed = selection; malformed.struct_size = 0;
    assert_eq!(unsafe { viem_core_view_copy_formatting(core.handle, view, &malformed, &mut info, ptr::null_mut(), 0, ptr::null_mut(), 0, ptr::null_mut(), 0) }, ViemStatus::InvalidArgument);
    assert_eq!(info, before_info);
    assert_eq!(unsafe { viem_core_view_copy_formatting(core.handle, view, &selection, &mut info, (&mut info as *mut ViemStyleSheetInfoV1).cast(), 1, ptr::null_mut(), 0, ptr::null_mut(), 0) }, ViemStatus::InvalidArgument);
    assert_eq!(info, before_info);
    assert_eq!(unsafe { viem_core_view_copy_formatting(core.handle, view, &selection, &mut info, properties.as_mut_ptr(), properties.len() as u64, items.as_mut_ptr(), items.len() as u64, strings.as_mut_ptr(), strings.len() as u64) }, ViemStatus::Ok);
    assert!(properties.iter().all(|property| property.property != u32::MAX));
}

#[test]
fn formatting_snapshot_handles_inline_style_boundary_inside_selected_grapheme() {
    use viem_core::document::{Document, Encoding, Format};

    let source = br"{\rtf1\qc {\b A}\u769?B}";
    let projected = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    assert!(projected.projection().style_spans().iter().any(|span| {
        span.range.start == 1 || span.range.end == 1
    }));
    let core = create_core(
        source,
        ViemDocumentOptions {
            format: VIEM_FORMAT_RTF,
            ..Default::default()
        },
    );
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, provider.as_mut());
    assert_eq!(
        unsafe {
            test_send_key(
                core.handle,
                view,
                &key(VIEM_KEY_CHARACTER, 'v' as u32),
                &mut outcome,
            )
        },
        ViemStatus::Ok
    );
    let mut selection = ViemLogicalSelectionIdentityV1::default();
    assert_eq!(
        unsafe { viem_core_view_list_selection(core.handle, view, &mut selection) },
        ViemStatus::Ok
    );
    // The bold declaration ends at byte 1, inside this selected grapheme.
    assert_eq!((selection.text_start, selection.text_end), (0, 3));
    let mut info = ViemStyleSheetInfoV1::default();
    assert_eq!(
        unsafe {
            viem_core_view_copy_formatting(
                core.handle, view, &selection, &mut info,
                ptr::null_mut(), 0, ptr::null_mut(), 0, ptr::null_mut(), 0,
            )
        },
        ViemStatus::BufferTooSmall
    );
    let mut properties = vec![ViemStylePropertyV1::default(); info.property_count as usize];
    let mut items = vec![ViemStyleValueItemV1::default(); info.value_item_count as usize];
    let mut strings = vec![0; info.string_bytes as usize];
    assert_eq!(
        unsafe {
            viem_core_view_copy_formatting(
                core.handle, view, &selection, &mut info,
                properties.as_mut_ptr(), properties.len() as u64,
                items.as_mut_ptr(), items.len() as u64,
                strings.as_mut_ptr(), strings.len() as u64,
            )
        },
        ViemStatus::Ok
    );
    let property = |id| properties.iter().find(|value| value.property == id).unwrap();
    let alignment = property(VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT);
    assert_eq!(alignment.effective.enum_value, VIEM_STYLE_PARAGRAPH_ALIGNMENT_CENTER);
    assert_eq!(alignment.flags & VIEM_STYLE_PROPERTY_MIXED, 0);
    assert_eq!(
        property(VIEM_STYLE_PROPERTY_CHARACTER_SCRIPT_POSITION).flags & VIEM_STYLE_PROPERTY_MIXED,
        0
    );
    assert_eq!(
        copy_core_bytes(viem_core_copy_source_bytes, &core, document_state(&core).document_revision),
        source
    );
}

use evim_core::ffi::*;
use std::ffi::c_void;
use std::ptr;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;

const FAKE_FONT: &[u8] = b"FFI Fake Sans";

struct FakeResponseStorage {
    _carets: Vec<Vec<EvimClusterCaretStopV1>>,
    clusters: Vec<EvimShapedClusterV1>,
    visual_order: Vec<u64>,
}

struct FakeProviderContext {
    core: EvimCoreHandle,
    metrics_generation: u64,
    expected_environment: u64,
    expected_owner: u64,
    shape_calls: usize,
    shaped_bytes: usize,
    minimum_request_start: u64,
    maximum_request_end: u64,
    fail_next: Option<EvimStatus>,
    saw_complete_request: bool,
    saw_crossing_cluster_tail: bool,
    last_requested_scale: f32,
    reentrant_status: u32,
    responses: Vec<FakeResponseStorage>,
}

impl FakeProviderContext {
    fn new(core: EvimCoreHandle) -> Self {
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

fn response_storage(request: &EvimShapeRequestV1) -> Option<FakeResponseStorage> {
    fn owned_utf8(value: EvimUtf8Slice) -> Option<String> {
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
    let metrics = EvimTextMetricsV1 {
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
                EvimClusterCaretStopV1 {
                    text_offset: text_start,
                    inline_offset: 0.0,
                    affinity: EVIM_BOUNDARY_AFFINITY_DOWNSTREAM,
                },
                EvimClusterCaretStopV1 {
                    text_offset: text_end,
                    inline_offset: advance,
                    affinity: EVIM_BOUNDARY_AFFINITY_UPSTREAM,
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
            |(index, ((text_start, text_end, advance), carets))| EvimShapedClusterV1 {
                struct_size: EVIM_SHAPED_CLUSTER_V1_SIZE,
                reserved: 0,
                text_start: *text_start,
                text_end: *text_end,
                advance: *advance,
                metrics,
                typographic_bounds: EvimShapedBoundsV1 {
                    x: 0.0,
                    y: -10.0,
                    width: *advance,
                    height: 13.0,
                },
                ink_bounds: EvimShapedBoundsV1 {
                    x: 0.0,
                    y: -10.0,
                    width: *advance,
                    height: 13.0,
                },
                bidi_level: 0,
                has_render_run: 1,
                fallback_font: EvimUtf8Slice {
                    data: FAKE_FONT.as_ptr(),
                    length: FAKE_FONT.len() as u64,
                },
                caret_stops: carets.as_ptr(),
                caret_stop_count: carets.len() as u64,
                render_run: EvimRenderRunHandleV1 {
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
    requests: *const EvimShapeRequestV1,
    request_count: u64,
    responses: *mut EvimShapeResponseV1,
    response_capacity: u64,
) -> u32 {
    if context.is_null()
        || request_count != response_capacity
        || request_count > usize::MAX as u64
        || (request_count != 0 && (requests.is_null() || responses.is_null()))
    {
        return EvimStatus::InvalidArgument as u32;
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
    context.reentrant_status = unsafe { evim_core_revision(context.core, &mut revision) } as u32;
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
            >= EVIM_RESOLVED_TEXT_STYLE_V1_SIZE
            && request.default_style.font_family_count > 0
            && !request.default_style.font_families.is_null()
            && request.default_style.size > 0.0;
        context.saw_complete_request |= request.struct_size >= EVIM_SHAPE_REQUEST_V1_SIZE
            && request.measurement_environment_id == context.expected_environment
            && request.metrics_generation == context.metrics_generation
            && request.purpose == EVIM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA
            && request.has_render_run_policy == 1
            && request.render_run_owner == context.expected_owner
            && request.render_run_threading == EVIM_RENDER_THREADING_ANY
            && request.paragraph_base_direction == EVIM_TEXT_DIRECTION_AUTO
            && style_is_complete;
        let Some(storage) = response_storage(request) else {
            return EvimStatus::ProviderFailure as u32;
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
        *output = EvimShapeResponseV1 {
            struct_size: EVIM_SHAPE_RESPONSE_V1_SIZE,
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
            default_metrics: EvimTextMetricsV1 {
                ascent: 10.0,
                descent: 3.0,
                leading: 1.0,
            },
            diagnostics: ptr::null(),
            diagnostic_count: 0,
        };
    }
    EvimStatus::Ok as u32
}

unsafe extern "C" fn blocking_shape_batch(
    context: *mut c_void,
    requests: *const EvimShapeRequestV1,
    request_count: u64,
    responses: *mut EvimShapeResponseV1,
    response_capacity: u64,
) -> u32 {
    if context.is_null() {
        return EvimStatus::InvalidArgument as u32;
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
    requests: *const EvimShapeRequestV1,
    request_count: u64,
    responses: *mut EvimShapeResponseV1,
    response_capacity: u64,
) -> u32 {
    if request_count != response_capacity
        || request_count > usize::MAX as u64
        || (request_count != 0 && (requests.is_null() || responses.is_null()))
    {
        return EvimStatus::InvalidArgument as u32;
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
        *response = EvimShapeResponseV1 {
            struct_size: EVIM_SHAPE_RESPONSE_V1_SIZE,
            document_id: request.document_id,
            document_revision: request.document_revision,
            measurement_environment_id: request.measurement_environment_id,
            metrics_generation: request.metrics_generation,
            text_start: request.text_start,
            text_end: request.text_end,
            clusters: ptr::null(),
            cluster_count: 1,
            default_metrics: EvimTextMetricsV1 {
                ascent: 10.0,
                descent: 3.0,
                leading: 1.0,
            },
            ..EvimShapeResponseV1::default()
        };
    }
    EvimStatus::Ok as u32
}

unsafe extern "C" fn unstable_context_shape_batch(
    _context: *mut c_void,
    _requests: *const EvimShapeRequestV1,
    _request_count: u64,
    _responses: *mut EvimShapeResponseV1,
    _response_capacity: u64,
) -> u32 {
    EvimStatus::UnstableShapingContext as u32
}

fn provider(
    context: *mut c_void,
    shape_batch: EvimShapeBatchCallback,
) -> EvimTextMeasurementProviderV1 {
    EvimTextMeasurementProviderV1 {
        struct_size: EVIM_TEXT_MEASUREMENT_PROVIDER_V1_SIZE,
        abi_version: EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION,
        context,
        measurement_environment_id: 0xa11c_e001,
        threading: EVIM_PROVIDER_THREADING_ANY_WORKER,
        has_render_run_policy: 1,
        render_run_owner: 0xf00d,
        render_run_threading: EVIM_RENDER_THREADING_ANY,
        reserved: 0,
        metrics_generation: Some(fake_metrics_generation),
        shape_batch: Some(shape_batch),
    }
}

struct TestCore {
    handle: EvimCoreHandle,
    revision: u64,
}

impl Drop for TestCore {
    fn drop(&mut self) {
        if self.handle != 0 {
            let _ = evim_core_destroy(self.handle);
        }
    }
}

fn create_core(source: &[u8], options: EvimDocumentOptions) -> TestCore {
    let mut handle = 0;
    let mut revision = u64::MAX;
    let status = unsafe {
        evim_core_create(
            source.as_ptr(),
            source.len() as u64,
            &options,
            &mut handle,
            &mut revision,
        )
    };
    assert_eq!(status, EvimStatus::Ok);
    assert_ne!(handle, 0);
    TestCore { handle, revision }
}

fn document_state(core: &TestCore) -> EvimDocumentStateV1 {
    let mut state = EvimDocumentStateV1::default();
    assert_eq!(
        unsafe { evim_core_document_state(core.handle, &mut state) },
        EvimStatus::Ok
    );
    state
}

type CoreCopy = unsafe extern "C" fn(EvimCoreHandle, u64, *mut u8, u64, *mut u64) -> EvimStatus;

fn copy_core_bytes(function: CoreCopy, core: &TestCore, revision: u64) -> Vec<u8> {
    let mut required = 0;
    let status = unsafe { function(core.handle, revision, ptr::null_mut(), 0, &mut required) };
    assert_eq!(
        status,
        if required == 0 {
            EvimStatus::Ok
        } else {
            EvimStatus::BufferTooSmall
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
        EvimStatus::Ok
    );
    bytes
}

fn key(kind: u32, codepoint: u32) -> EvimKeyInputV1 {
    EvimKeyInputV1 {
        struct_size: EVIM_KEY_INPUT_V1_SIZE,
        kind,
        codepoint,
        reserved: 0,
    }
}

fn utf8_slice(bytes: &[u8]) -> EvimUtf8Slice {
    EvimUtf8Slice {
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
) -> (EvimViewId, EvimCoreOutcomeV1) {
    let provider = provider(context.cast(), fake_shape_batch);
    let mut view = 0;
    let mut outcome = EvimCoreOutcomeV1::default();
    assert_eq!(
        unsafe {
            evim_core_view_add(
                core.handle,
                &EvimViewOptionsV1::default(),
                &provider,
                &mut view,
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    (view, outcome)
}

#[allow(dead_code)]
struct CopiedEffectBatch {
    info: EvimEffectBatchInfoV1,
    clipboard_writes: Vec<EvimClipboardWriteV1>,
    ex_requests: Vec<EvimExFrontendRequestV1>,
    ex_options: Vec<EvimExOptionDisplayV1>,
    ex_marks: Vec<EvimExMarkV1>,
    ex_registers: Vec<EvimExRegisterV1>,
    ex_jumps: Vec<EvimExJumpV1>,
    ex_text_lines: Vec<EvimExTextLineV1>,
    file_formats: Vec<u32>,
    hard_breaks: Vec<u64>,
    strings: Vec<u8>,
}

impl CopiedEffectBatch {
    fn text(&self, reference: EvimEffectBytesRefV1) -> &str {
        let start = usize::try_from(reference.offset).unwrap();
        let length = usize::try_from(reference.length).unwrap();
        std::str::from_utf8(&self.strings[start..start + length]).unwrap()
    }
}

fn copy_effect_batch(handle: EvimEffectBatchHandle) -> CopiedEffectBatch {
    let mut info = EvimEffectBatchInfoV1::default();
    assert_eq!(
        unsafe { evim_effect_batch_info(handle, &mut info) },
        EvimStatus::Ok
    );
    let mut clipboard_writes =
        vec![EvimClipboardWriteV1::default(); info.clipboard_write_count as usize];
    let mut ex_requests = vec![EvimExFrontendRequestV1::default(); info.ex_request_count as usize];
    let mut ex_options = vec![EvimExOptionDisplayV1::default(); info.ex_option_count as usize];
    let mut ex_marks = vec![EvimExMarkV1::default(); info.ex_mark_count as usize];
    let mut ex_registers = vec![EvimExRegisterV1::default(); info.ex_register_count as usize];
    let mut ex_jumps = vec![EvimExJumpV1::default(); info.ex_jump_count as usize];
    let mut ex_text_lines = vec![EvimExTextLineV1::default(); info.ex_text_line_count as usize];
    let mut file_formats = vec![0; info.file_format_count as usize];
    let mut hard_breaks = vec![0; info.hard_break_count as usize];
    let mut strings = vec![0; info.string_bytes as usize];
    let mut copied_info = EvimEffectBatchInfoV1::default();
    assert_eq!(
        unsafe {
            evim_effect_batch_copy(
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
        EvimStatus::Ok
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
    view: EvimViewId,
    input: EvimKeyInputV1,
    entries: &[EvimClipboardTurnEntryV1],
) -> (EvimStatus, EvimCoreOutcomeV1, EvimEffectBatchHandle) {
    let context = EvimCommandTurnContextV1 {
        clipboards: if entries.is_empty() {
            ptr::null()
        } else {
            entries.as_ptr()
        },
        clipboard_count: entries.len() as u64,
        ..EvimCommandTurnContextV1::default()
    };
    let mut outcome = EvimCoreOutcomeV1::default();
    let mut effects = 0;
    let status = unsafe {
        evim_core_view_send_key_with_host_context(
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
    view: EvimViewId,
    text: &str,
) -> (EvimStatus, EvimCoreOutcomeV1, EvimEffectBatchHandle) {
    let context = EvimCommandTurnContextV1::default();
    let mut outcome = EvimCoreOutcomeV1::default();
    let mut effects = 0;
    let status = unsafe {
        evim_core_view_send_text_with_host_context(
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

fn host_chars(core: &TestCore, view: EvimViewId, chars: &str) {
    for character in chars.chars() {
        let (status, _, effects) = host_key(
            core,
            view,
            key(EVIM_KEY_CHARACTER, u32::from(character)),
            &[],
        );
        assert_eq!(status, EvimStatus::Ok);
        assert_eq!(effects, 0);
    }
}

fn host_ex(core: &TestCore, view: EvimViewId, command: &str) -> EvimEffectBatchHandle {
    host_chars(core, view, ":");
    let (status, _, effects) = host_text(core, view, command);
    assert_eq!(status, EvimStatus::Ok);
    assert_eq!(effects, 0);
    let (status, outcome, effects) = host_key(core, view, key(EVIM_KEY_ENTER, 0), &[]);
    assert_eq!(status, EvimStatus::Ok, "executing :{command}");
    assert_ne!(
        outcome.flags & EVIM_OUTCOME_HAS_EXTERNAL_EFFECTS,
        0,
        ":{command} did not publish external effects (status {})",
        outcome.command_status
    );
    assert_ne!(effects, 0, ":{command} did not publish an effect batch");
    effects
}

#[test]
fn host_context_turns_exchange_clipboards_and_own_raw_ex_effects() {
    let mut core = create_core("a".as_bytes(), EvimDocumentOptions::default());
    let mut provider_context = Box::new(FakeProviderContext::new(core.handle));
    let (view, _) = add_test_view(&core, &mut *provider_context);

    let invalid = [0xff_u8];
    let bad_entry = EvimClipboardTurnEntryV1 {
        flags: EVIM_CLIPBOARD_TURN_HAS_READ,
        target: EVIM_CLIPBOARD_TARGET_CLIPBOARD,
        generation: 4,
        plain_text: utf8_slice(&invalid),
        ..EvimClipboardTurnEntryV1::default()
    };
    let mut rejected_outcome = EvimCoreOutcomeV1 {
        flags: u32::MAX,
        ..EvimCoreOutcomeV1::default()
    };
    let mut rejected_effect = u64::MAX;
    let bad_context = EvimCommandTurnContextV1 {
        clipboards: &bad_entry,
        clipboard_count: 1,
        ..EvimCommandTurnContextV1::default()
    };
    assert_eq!(
        unsafe {
            evim_core_view_send_key_with_host_context(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, u32::from('x')),
                &bad_context,
                &mut rejected_outcome,
                &mut rejected_effect,
            )
        },
        EvimStatus::InvalidUtf8
    );
    assert_eq!(rejected_outcome, EvimCoreOutcomeV1::default());
    assert_eq!(rejected_effect, 0);
    assert_eq!(document_state(&core).document_revision, 0);

    host_chars(&core, view, "\"+");
    let pasted = "世界👋";
    let read_entry = EvimClipboardTurnEntryV1 {
        flags: EVIM_CLIPBOARD_TURN_HAS_READ,
        target: EVIM_CLIPBOARD_TARGET_CLIPBOARD,
        generation: 77,
        plain_text: utf8_slice(pasted.as_bytes()),
        ..EvimClipboardTurnEntryV1::default()
    };
    let (status, paste_outcome, paste_effects) = host_key(
        &core,
        view,
        key(EVIM_KEY_CHARACTER, u32::from('p')),
        &[read_entry],
    );
    assert_eq!(status, EvimStatus::Ok);
    assert_ne!(paste_outcome.flags & EVIM_OUTCOME_DOCUMENT_CHANGED, 0);
    assert_eq!(paste_effects, 0, "a clipboard read emits no host write");
    let revision = paste_outcome.document_revision;
    assert_eq!(
        copy_core_bytes(evim_core_copy_formatted_utf8, &core, revision),
        "a世界👋".as_bytes()
    );

    host_chars(&core, view, "\"+y");
    let writable = EvimClipboardTurnEntryV1 {
        flags: EVIM_CLIPBOARD_TURN_WRITABLE,
        target: EVIM_CLIPBOARD_TARGET_CLIPBOARD,
        ..EvimClipboardTurnEntryV1::default()
    };
    let (status, yank_outcome, yank_effects) = host_key(
        &core,
        view,
        key(EVIM_KEY_CHARACTER, u32::from('y')),
        &[writable],
    );
    assert_eq!(status, EvimStatus::Ok);
    assert_ne!(yank_outcome.flags & EVIM_OUTCOME_HAS_EXTERNAL_EFFECTS, 0);
    assert_ne!(yank_effects, 0);
    let yank = copy_effect_batch(yank_effects);
    assert_eq!(yank.clipboard_writes.len(), 1);
    let write = yank.clipboard_writes[0];
    assert_eq!(write.target, EVIM_CLIPBOARD_TARGET_CLIPBOARD);
    assert_eq!(write.register_kind, EVIM_REGISTER_KIND_LINE);
    assert_ne!(write.flags & EVIM_CLIPBOARD_WRITE_HAS_PORTABLE_REGISTER, 0);
    assert_eq!(yank.text(write.plain_text), "a世界👋\n");
    assert_eq!(evim_effect_batch_release(yank_effects), EvimStatus::Ok);

    let write_effects = host_ex(&core, view, "w");
    let captured = copy_effect_batch(write_effects);
    assert_eq!(captured.ex_requests.len(), 1);
    assert_eq!(captured.ex_requests[0].kind, EVIM_EX_FRONTEND_WRITE);
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
    assert_eq!(evim_core_destroy(core.handle), EvimStatus::Ok);
    core.handle = 0;
    let after_destroy = copy_effect_batch(write_effects);
    assert_eq!(after_destroy.info, captured.info);
    assert_eq!(after_destroy.ex_requests, captured.ex_requests);
    assert_eq!(evim_effect_batch_release(write_effects), EvimStatus::Ok);
    assert_eq!(
        evim_effect_batch_release(write_effects),
        EvimStatus::InvalidHandle
    );
    let mut stale_info = EvimEffectBatchInfoV1 {
        batch_handle: u64::MAX,
        ..EvimEffectBatchInfoV1::default()
    };
    assert_eq!(
        unsafe { evim_effect_batch_info(write_effects, &mut stale_info) },
        EvimStatus::InvalidHandle
    );
    assert_eq!(stale_info, EvimEffectBatchInfoV1::default());
}

#[test]
fn ex_info_effects_capture_marks_registers_jumps_and_printed_text() {
    let core = create_core(
        "α one\nβ two\nγ three".as_bytes(),
        EvimDocumentOptions::default(),
    );
    let mut provider_context = Box::new(FakeProviderContext::new(core.handle));
    let (view, _) = add_test_view(&core, &mut *provider_context);

    host_chars(&core, view, "maG\"ayy");

    let marks_handle = host_ex(&core, view, "marks a");
    let marks = copy_effect_batch(marks_handle);
    assert_eq!(marks.ex_requests[0].kind, EVIM_EX_FRONTEND_MARKS);
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
    assert_eq!(evim_effect_batch_release(marks_handle), EvimStatus::Ok);

    let registers_handle = host_ex(&core, view, "registers a");
    let registers = copy_effect_batch(registers_handle);
    assert_eq!(registers.ex_requests[0].kind, EVIM_EX_FRONTEND_REGISTERS);
    assert_eq!(registers.ex_registers.len(), 1);
    assert_eq!(registers.ex_registers[0].name, u32::from('a'));
    assert_eq!(
        registers.ex_registers[0].register_kind,
        EVIM_REGISTER_KIND_LINE
    );
    assert_eq!(registers.text(registers.ex_registers[0].text), "γ three\n");
    assert_eq!(evim_effect_batch_release(registers_handle), EvimStatus::Ok);

    let jumps_handle = host_ex(&core, view, "jumps");
    let jumps = copy_effect_batch(jumps_handle);
    assert_eq!(jumps.ex_requests[0].kind, EVIM_EX_FRONTEND_JUMPS);
    assert_eq!(jumps.ex_jumps.len(), 2);
    assert_eq!(jumps.ex_jumps[0].hard_line_index, 0);
    assert_eq!(jumps.ex_jumps[1].hard_line_index, 2);
    assert_eq!(jumps.ex_jumps[1].flags, EVIM_EX_JUMP_CURRENT);
    assert_eq!(jumps.text(jumps.ex_jumps[1].line_text), "γ three");
    assert_eq!(evim_effect_batch_release(jumps_handle), EvimStatus::Ok);

    let print_handle = host_ex(&core, view, "1,2s/o/o/p");
    let printed = copy_effect_batch(print_handle);
    assert_eq!(printed.ex_requests[0].kind, EVIM_EX_FRONTEND_PRINT_LINES);
    assert_eq!(printed.ex_requests[0].flags, EVIM_EX_FRONTEND_HAS_RANGE);
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
    assert_eq!(evim_effect_batch_release(print_handle), EvimStatus::Ok);
}

#[test]
fn callback_backed_core_round_trips_controller_view_and_exact_source() {
    let mut core = create_core(
        b"caf\xe9\r\n",
        EvimDocumentOptions {
            encoding: EVIM_ENCODING_LATIN1,
            file_format: EVIM_FILE_FORMAT_DOS,
            ..EvimDocumentOptions::default()
        },
    );
    assert_eq!(core.revision, 0);
    assert_eq!(
        copy_core_bytes(evim_core_copy_source_bytes, &core, 0),
        b"caf\xe9\r\n"
    );
    assert_eq!(
        copy_core_bytes(evim_core_copy_formatted_utf8, &core, 0),
        "café\n".as_bytes()
    );

    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let provider = provider(
        (&mut *context as *mut FakeProviderContext).cast(),
        fake_shape_batch,
    );
    let options = EvimViewOptionsV1 {
        width: 240.0,
        height: 100.0,
        ..EvimViewOptionsV1::default()
    };
    let mut view = 0;
    let mut outcome = EvimCoreOutcomeV1::default();
    assert_eq!(
        unsafe { evim_core_view_add(core.handle, &options, &provider, &mut view, &mut outcome,) },
        EvimStatus::Ok
    );
    assert_eq!(view, 1);
    assert_eq!(outcome.document_revision, 0);
    assert_eq!(outcome.mode, EVIM_MODE_NORMAL);
    assert_eq!(outcome.view_id, view);
    assert_eq!(outcome.measurement_environment_id, 0xa11c_e001);
    assert_eq!(outcome.metrics_generation, 1);
    assert_ne!(outcome.flags & EVIM_OUTCOME_HAS_LAYOUT, 0);
    assert!(context.saw_complete_request);
    assert!(context.shape_calls > 0);
    assert_eq!(context.reentrant_status, EvimStatus::CoreBusy as u32);

    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, 'i' as u32),
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(outcome.mode, EVIM_MODE_INSERT);
    assert_eq!(outcome.command_status, EVIM_COMMAND_STATUS_COMPLETE);
    assert_ne!(outcome.flags & EVIM_OUTCOME_MODE_CHANGED, 0);

    assert_eq!(
        unsafe { evim_core_view_send_text(core.handle, view, b"!".as_ptr(), 1, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(outcome.document_revision, 1);
    assert_ne!(outcome.flags & EVIM_OUTCOME_DOCUMENT_CHANGED, 0);
    assert_ne!(outcome.flags & EVIM_OUTCOME_HAS_POSITION_MAP, 0);
    core.revision = 1;
    assert_eq!(
        copy_core_bytes(evim_core_copy_source_bytes, &core, 1),
        b"!caf\xe9\r\n"
    );
    assert_eq!(
        copy_core_bytes(evim_core_copy_formatted_utf8, &core, 1),
        "!café\n".as_bytes()
    );

    assert_eq!(
        unsafe { evim_core_view_resize(core.handle, view, 120.0, 50.0, &mut outcome) },
        EvimStatus::Ok
    );
    assert_ne!(outcome.flags & EVIM_OUTCOME_LAYOUT_CHANGED, 0);
    assert_eq!(
        unsafe { evim_core_view_set_wrap(core.handle, view, 0, &mut outcome) },
        EvimStatus::Ok
    );
    assert_ne!(outcome.flags & EVIM_OUTCOME_HAS_LAYOUT, 0);

    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);
    assert_eq!(
        unsafe { evim_core_view_state(core.handle, view, &mut outcome) },
        EvimStatus::InvalidView
    );
    let mut replacement_view = 0;
    assert_eq!(
        unsafe {
            evim_core_view_add(
                core.handle,
                &options,
                &provider,
                &mut replacement_view,
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(replacement_view, 2, "removed view IDs must not be reused");
    assert_eq!(
        evim_core_view_remove(core.handle, replacement_view),
        EvimStatus::Ok
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
            EVIM_ENCODING_UTF8,
            "plain UTF-8 😀".as_bytes().to_vec(),
            false,
        ),
        (
            [vec![0xef, 0xbb, 0xbf], "BOM café".as_bytes().to_vec()].concat(),
            EVIM_ENCODING_UTF8,
            "BOM café".as_bytes().to_vec(),
            true,
        ),
        (
            utf16_source("little 😀", true),
            EVIM_ENCODING_UTF16_LE,
            "little 😀".as_bytes().to_vec(),
            true,
        ),
        (
            utf16_source("big 😀", false),
            EVIM_ENCODING_UTF16_BE,
            "big 😀".as_bytes().to_vec(),
            true,
        ),
        (
            b"caf\xe9".to_vec(),
            EVIM_ENCODING_LATIN1,
            "café".as_bytes().to_vec(),
            false,
        ),
        (
            vec![0xf0, 0x28, 0x8c, 0x28],
            EVIM_ENCODING_LATIN1,
            "ð(\u{8c}(".as_bytes().to_vec(),
            false,
        ),
        (
            vec![0xef, 0xbb, 0xbf, 0xff],
            EVIM_ENCODING_UTF8,
            "\u{fffd}".as_bytes().to_vec(),
            true,
        ),
        // Automatic detection deliberately does not guess BOM-less UTF-16.
        // These bytes are valid UTF-8 containing NUL scalars, so UTF-8 wins.
        (
            vec![b'A', 0, b'B', 0],
            EVIM_ENCODING_UTF8,
            vec![b'A', 0, b'B', 0],
            false,
        ),
    ];

    for (source, expected_encoding, expected_text, expected_bom) in cases {
        let core = create_core(
            &source,
            EvimDocumentOptions {
                encoding: EVIM_ENCODING_DETECT,
                file_format: EVIM_FILE_FORMAT_UNIX,
                ..EvimDocumentOptions::default()
            },
        );
        let state = document_state(&core);
        assert_eq!(state.encoding, expected_encoding, "source {source:?}");
        assert_eq!(
            state.flags & EVIM_DOCUMENT_STATE_HAS_BOM != 0,
            expected_bom,
            "source {source:?}"
        );
        assert_eq!(
            copy_core_bytes(evim_core_copy_source_bytes, &core, 0),
            source,
            "detection must not rewrite authoritative bytes"
        );
        assert_eq!(
            copy_core_bytes(evim_core_copy_formatted_utf8, &core, 0),
            expected_text,
            "source {source:?}"
        );
    }

    // The historical Rust convenience default remains an explicit UTF-8
    // request. Invalid UTF-8 is therefore preserved as an opaque diagnostic
    // projection instead of silently switching that existing caller to
    // Latin-1.
    assert_eq!(EvimDocumentOptions::default().encoding, EVIM_ENCODING_UTF8);
    let forced_utf8_source = b"caf\xe9";
    let forced_utf8 = create_core(
        forced_utf8_source,
        EvimDocumentOptions {
            encoding: EVIM_ENCODING_UTF8,
            ..EvimDocumentOptions::default()
        },
    );
    assert_eq!(document_state(&forced_utf8).encoding, EVIM_ENCODING_UTF8);
    assert_eq!(
        copy_core_bytes(evim_core_copy_source_bytes, &forced_utf8, 0),
        forced_utf8_source
    );
    assert_eq!(
        copy_core_bytes(evim_core_copy_formatted_utf8, &forced_utf8, 0),
        "caf\u{fffd}".as_bytes()
    );

    // Explicit Latin-1 also remains authoritative even when the bytes begin
    // with a signature another policy would recognize as a UTF-8 BOM.
    let forced_latin1_source = [0xef, 0xbb, 0xbf, b'A'];
    let forced_latin1 = create_core(
        &forced_latin1_source,
        EvimDocumentOptions {
            encoding: EVIM_ENCODING_LATIN1,
            ..EvimDocumentOptions::default()
        },
    );
    let forced_latin1_state = document_state(&forced_latin1);
    assert_eq!(forced_latin1_state.encoding, EVIM_ENCODING_LATIN1);
    assert_eq!(forced_latin1_state.flags & EVIM_DOCUMENT_STATE_HAS_BOM, 0);
    assert_eq!(
        copy_core_bytes(evim_core_copy_formatted_utf8, &forced_latin1, 0),
        "ï»¿A".as_bytes()
    );
}

#[test]
fn native_named_style_create_delete_are_sparse_revision_bound_and_undoable() {
    for (format, original, prefix) in [
        (
            EVIM_FORMAT_HTML,
            b"<p data-x='keep'>Text</p>".as_slice(),
            "Custom",
        ),
        (EVIM_FORMAT_RTF, br"{\rtf1 Text}".as_slice(), "Rtf"),
    ] {
        let core = create_core(
            original,
            EvimDocumentOptions {
                format,
                ..Default::default()
            },
        );
        let mut provider = Box::new(FakeProviderContext::new(core.handle));
        let (view, mut outcome) = add_test_view(&core, provider.as_mut());
        let query = || {
            let mut info = EvimStyleSheetInfoV1::default();
            assert_eq!(
                unsafe { evim_core_style_sheet_info(core.handle, &mut info) },
                EvimStatus::Ok
            );
            info.identity
        };
        for (namespace, suffix) in [
            (EVIM_STYLE_NAMESPACE_BLOCK, "P7"),
            (EVIM_STYLE_NAMESPACE_CHARACTER, "C4"),
        ] {
            let id = format!("{prefix}{suffix}");
            let request = EvimCreateStyleV1 {
                struct_size: EVIM_CREATE_STYLE_V1_SIZE,
                namespace,
                identity: query(),
                style_id: utf8_slice(id.as_bytes()),
                display_name: utf8_slice("Sparse α".as_bytes()),
                parent_id: EvimUtf8Slice::default(),
                next_style_id: EvimUtf8Slice::default(),
            };
            assert_eq!(
                unsafe { evim_core_view_create_style(core.handle, view, &request, &mut outcome) },
                EvimStatus::Ok,
                "format {format}"
            );
            assert_eq!(
                unsafe { evim_core_view_create_style(core.handle, view, &request, &mut outcome) },
                EvimStatus::StaleRevision
            );
            let created = copy_core_bytes(
                evim_core_copy_source_bytes,
                &core,
                document_state(&core).document_revision,
            );
            assert_ne!(created, original);
            assert_eq!(
                copy_core_bytes(
                    evim_core_copy_formatted_utf8,
                    &core,
                    document_state(&core).document_revision
                ),
                b"Text"
            );
            let deletion = EvimDeleteStyleV1 {
                struct_size: EVIM_DELETE_STYLE_V1_SIZE,
                namespace,
                identity: query(),
                style_id: utf8_slice(id.as_bytes()),
            };
            assert_eq!(
                unsafe { evim_core_view_delete_style(core.handle, view, &deletion, &mut outcome) },
                EvimStatus::Ok,
                "format {format}"
            );
            assert_eq!(
                unsafe { evim_core_view_delete_style(core.handle, view, &deletion, &mut outcome) },
                EvimStatus::StaleRevision
            );
            assert_eq!(
                unsafe { evim_core_view_undo(core.handle, view, &mut outcome) },
                EvimStatus::Ok
            );
            assert_eq!(
                copy_core_bytes(
                    evim_core_copy_source_bytes,
                    &core,
                    document_state(&core).document_revision
                ),
                created
            );
            assert_eq!(
                unsafe { evim_core_view_undo(core.handle, view, &mut outcome) },
                EvimStatus::Ok
            );
            assert_eq!(
                copy_core_bytes(
                    evim_core_copy_source_bytes,
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
    use evim_core::document::*;
    let mut source = Document::from_bytes(
        b"<p data-x='keep'>Alpha</p><p>Beta</p>".to_vec(),
        Encoding::Utf8,
        Format::Html,
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
                        id: "Accent".into(),
                        based_on: Some("Character".into()),
                        properties: CharacterProperties {
                            underline: Some(true),
                            ..Default::default()
                        },
                    },
                    metadata: StyleDefinitionMetadata {
                        display_name: "Accent".into(),
                        origin: StyleDefinitionOrigin::SourceBacked,
                    },
                },
            }),
        ))
        .unwrap();
    let original = source.source_bytes();
    let core = create_core(
        &original,
        EvimDocumentOptions {
            format: EVIM_FORMAT_HTML,
            ..Default::default()
        },
    );
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, provider.as_mut());
    let query = || {
        let mut sheet = EvimStyleSheetInfoV1::default();
        let mut selection = EvimLogicalSelectionIdentityV1::default();
        assert_eq!(
            unsafe { evim_core_style_sheet_info(core.handle, &mut sheet) },
            EvimStatus::Ok
        );
        assert_eq!(
            unsafe { evim_core_view_list_selection(core.handle, view, &mut selection) },
            EvimStatus::Ok
        );
        (sheet.identity, selection)
    };
    let (identity, expected_selection) = query();
    let heading = b"Heading2";
    let request = EvimAssignStyleV1 {
        struct_size: EVIM_ASSIGN_STYLE_V1_SIZE,
        namespace: EVIM_STYLE_NAMESPACE_BLOCK,
        identity,
        expected_selection,
        style_id: EvimUtf8Slice {
            data: heading.as_ptr(),
            length: heading.len() as u64,
        },
    };
    let mut stale = request;
    stale.identity.style_sheet_revision += 1;
    assert_eq!(
        unsafe { evim_core_view_assign_style(core.handle, view, &stale, &mut outcome) },
        EvimStatus::StaleRevision
    );
    assert_eq!(
        unsafe { evim_core_view_assign_style(core.handle, view, &request, &mut outcome) },
        EvimStatus::Ok
    );
    let state = document_state(&core);
    let rewritten = copy_core_bytes(evim_core_copy_source_bytes, &core, state.document_revision);
    assert!(String::from_utf8(rewritten.clone())
        .unwrap()
        .contains("<h2 data-x='keep'>Alpha</h2><p>Beta</p>"));
    assert_eq!(
        unsafe { evim_core_view_assign_style(core.handle, view, &request, &mut outcome) },
        EvimStatus::StaleRevision
    );
    assert_eq!(
        unsafe { evim_core_view_undo(core.handle, view, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        copy_core_bytes(
            evim_core_copy_source_bytes,
            &core,
            document_state(&core).document_revision
        ),
        original
    );

    for character in ['v', 'l'] {
        assert_eq!(
            unsafe {
                evim_core_view_send_key(
                    core.handle,
                    view,
                    &key(EVIM_KEY_CHARACTER, character as u32),
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
    }
    let (identity, expected_selection) = query();
    let accent = b"Accent";
    let request = EvimAssignStyleV1 {
        namespace: EVIM_STYLE_NAMESPACE_CHARACTER,
        identity,
        expected_selection,
        style_id: EvimUtf8Slice {
            data: accent.as_ptr(),
            length: accent.len() as u64,
        },
        ..request
    };
    assert_eq!(
        unsafe {
            evim_core_view_send_key(core.handle, view, &key(EVIM_KEY_RIGHT, 0), &mut outcome)
        },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_assign_style(core.handle, view, &request, &mut outcome) },
        EvimStatus::StaleRevision
    );
    let (_, expected_selection) = query();
    let request = EvimAssignStyleV1 {
        expected_selection,
        ..request
    };
    assert_eq!(
        unsafe { evim_core_view_assign_style(core.handle, view, &request, &mut outcome) },
        EvimStatus::Ok
    );
    let rewritten = copy_core_bytes(
        evim_core_copy_source_bytes,
        &core,
        document_state(&core).document_revision,
    );
    assert!(String::from_utf8(rewritten)
        .unwrap()
        .contains("<span class=\"evim-c-416363656e74\">Alp</span>ha"));
    assert_eq!(
        unsafe { evim_core_view_undo(core.handle, view, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        copy_core_bytes(
            evim_core_copy_source_bytes,
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
        EvimDocumentOptions {
            format: EVIM_FORMAT_MARKDOWN_SOURCE,
            ..Default::default()
        },
    );
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, _) = add_test_view(&core, provider.as_mut());
    let mut selection = EvimLogicalSelectionIdentityV1::default();
    assert_eq!(
        unsafe { evim_core_view_list_selection(core.handle, view, &mut selection) },
        EvimStatus::Ok
    );
    let mut outcome = EvimCoreOutcomeV1::default();
    let request = EvimSetParagraphStyleV1 {
        struct_size: EVIM_SET_PARAGRAPH_STYLE_V1_SIZE,
        level: 2,
        expected_selection: selection,
    };
    assert_eq!(
        unsafe { evim_core_view_set_paragraph_style(core.handle, view, &request, &mut outcome) },
        EvimStatus::Ok
    );
    let state = document_state(&core);
    assert_eq!(
        copy_core_bytes(
            evim_core_copy_formatted_utf8,
            &core,
            state.document_revision
        ),
        b"## Heading\nBody"
    );
    assert_eq!(
        unsafe { evim_core_view_set_paragraph_style(core.handle, view, &request, &mut outcome) },
        EvimStatus::StaleRevision
    );
    let invalid = EvimSetParagraphStyleV1 {
        level: 7,
        ..request
    };
    assert_eq!(
        unsafe { evim_core_view_set_paragraph_style(core.handle, view, &invalid, &mut outcome) },
        EvimStatus::InvalidArgument
    );
}

#[test]
fn native_format_encoding_and_list_requests_validate_exact_identity() {
    let core = create_core(b"__alpha__\nbeta", EvimDocumentOptions::default());
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, _) = add_test_view(&core, provider.as_mut());
    let state = document_state(&core);
    let mut outcome = EvimCoreOutcomeV1::default();
    let format = EvimSetFormatV1 {
        struct_size: EVIM_SET_FORMAT_V1_SIZE,
        format: EVIM_FORMAT_MARKDOWN,
        document_id: state.document_id,
        document_revision: state.document_revision,
    };
    assert_eq!(
        unsafe { evim_core_view_set_format(core.handle, view, &format, &mut outcome) },
        EvimStatus::Ok
    );
    let changed = document_state(&core);
    assert_eq!(changed.format, EVIM_FORMAT_MARKDOWN);
    assert_eq!(
        copy_core_bytes(
            evim_core_copy_source_bytes,
            &core,
            changed.document_revision
        ),
        b"__alpha__\nbeta"
    );
    assert_eq!(
        copy_core_bytes(
            evim_core_copy_formatted_utf8,
            &core,
            changed.document_revision
        ),
        b"alpha\nbeta"
    );
    assert_eq!(
        unsafe { evim_core_view_set_format(core.handle, view, &format, &mut outcome) },
        EvimStatus::StaleRevision
    );
    let encoding = EvimSetEncodingV1 {
        struct_size: EVIM_SET_ENCODING_V1_SIZE,
        encoding: EVIM_ENCODING_UTF16_LE,
        document_id: changed.document_id,
        document_revision: changed.document_revision,
    };
    assert_eq!(
        unsafe { evim_core_view_set_encoding(core.handle, view, &encoding, &mut outcome) },
        EvimStatus::Ok
    );
    let changed = document_state(&core);
    assert_eq!(changed.encoding, EVIM_ENCODING_UTF16_LE);
    let mut selection = EvimLogicalSelectionIdentityV1::default();
    assert_eq!(
        unsafe { evim_core_view_list_selection(core.handle, view, &mut selection) },
        EvimStatus::Ok
    );
    assert_eq!(selection.kind, EVIM_LOGICAL_SELECTION_KIND_NONE);
    let list = EvimSetListStyleV1 {
        struct_size: EVIM_SET_LIST_STYLE_V1_SIZE,
        style: EVIM_LIST_STYLE_BULLET,
        expected_selection: selection,
    };
    assert_eq!(
        unsafe { evim_core_view_set_list_style(core.handle, view, &list, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_set_list_style(core.handle, view, &list, &mut outcome) },
        EvimStatus::StaleRevision
    );
    assert_eq!(
        unsafe { evim_core_view_set_format(core.handle, view, ptr::null(), &mut outcome) },
        EvimStatus::NullPointer
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
        EvimDocumentOptions {
            encoding: EVIM_ENCODING_UTF16_LE,
            format: EVIM_FORMAT_MARKDOWN,
            file_format: EVIM_FILE_FORMAT_DETECT,
            ..EvimDocumentOptions::default()
        },
    );
    let initial = document_state(&core);
    assert_eq!(initial.struct_size, EVIM_DOCUMENT_STATE_V1_SIZE);
    assert_ne!(initial.document_id, 0);
    assert_eq!(initial.document_revision, 0);
    assert_eq!(initial.encoding, EVIM_ENCODING_UTF16_LE);
    assert_eq!(initial.format, EVIM_FORMAT_MARKDOWN);
    assert_eq!(initial.file_format, EVIM_FILE_FORMAT_DOS);
    assert_eq!(initial.file_format_origin, EVIM_FILE_FORMAT_ORIGIN_DETECTED);
    assert_ne!(initial.flags & EVIM_DOCUMENT_STATE_HAS_BOM, 0);
    assert_eq!(
        initial.flags
            & (EVIM_DOCUMENT_STATE_CAN_UNDO
                | EVIM_DOCUMENT_STATE_CAN_REDO
                | EVIM_DOCUMENT_STATE_IS_DIRTY),
        0
    );
    assert_eq!(
        initial.undo_action_category,
        EVIM_HISTORY_ACTION_CATEGORY_NONE
    );
    assert_eq!(
        initial.redo_action_category,
        EVIM_HISTORY_ACTION_CATEGORY_NONE
    );
    assert_eq!(initial.reserved, [0; 2]);

    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);
    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, 'i' as u32),
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_send_text(core.handle, view, b"x".as_ptr(), 1, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe {
            evim_core_view_send_key(core.handle, view, &key(EVIM_KEY_ESCAPE, 0), &mut outcome)
        },
        EvimStatus::Ok
    );
    let edited = document_state(&core);
    assert_eq!(edited.document_revision, 1);
    assert_eq!(edited.style_sheet_revision, initial.style_sheet_revision);
    assert_eq!(edited.encoding, initial.encoding);
    assert_eq!(edited.format, initial.format);
    assert_ne!(edited.flags & EVIM_DOCUMENT_STATE_HAS_BOM, 0);
    assert_ne!(edited.flags & EVIM_DOCUMENT_STATE_CAN_UNDO, 0);
    assert_ne!(edited.flags & EVIM_DOCUMENT_STATE_IS_DIRTY, 0);
    assert_eq!(
        edited.undo_action_category,
        EVIM_HISTORY_ACTION_CATEGORY_TEXT
    );
    let mut viewport_before_save = EvimViewportStateV1::default();
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, &mut viewport_before_save) },
        EvimStatus::Ok
    );

    let saved = EvimMarkSavedV1 {
        document_id: edited.document_id,
        document_revision: edited.document_revision,
        ..EvimMarkSavedV1::default()
    };
    assert_eq!(
        unsafe { evim_core_mark_saved(core.handle, &saved) },
        EvimStatus::Ok
    );
    let clean = document_state(&core);
    let mut viewport_after_save = EvimViewportStateV1::default();
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, &mut viewport_after_save) },
        EvimStatus::Ok
    );
    assert_eq!(clean.document_revision, edited.document_revision);
    assert_eq!(viewport_after_save, viewport_before_save);
    assert_eq!(clean.flags & EVIM_DOCUMENT_STATE_IS_DIRTY, 0);
    assert_ne!(clean.flags & EVIM_DOCUMENT_STATE_CAN_UNDO, 0);

    let format_request = EvimSetFileFormatV1 {
        file_format: EVIM_FILE_FORMAT_UNIX,
        document_id: clean.document_id,
        document_revision: clean.document_revision,
        ..EvimSetFileFormatV1::default()
    };
    assert_eq!(
        unsafe { evim_core_view_set_file_format(core.handle, view, &format_request, &mut outcome) },
        EvimStatus::Ok
    );
    let converted = document_state(&core);
    assert_eq!(converted.style_sheet_revision, initial.style_sheet_revision);
    assert_eq!(converted.encoding, initial.encoding);
    assert_eq!(converted.format, initial.format);
    assert_ne!(converted.flags & EVIM_DOCUMENT_STATE_HAS_BOM, 0);
    assert_eq!(converted.file_format, EVIM_FILE_FORMAT_UNIX);
    assert_eq!(converted.file_format_origin, EVIM_FILE_FORMAT_ORIGIN_FORCED);
    assert_ne!(converted.flags & EVIM_DOCUMENT_STATE_IS_DIRTY, 0);
    assert_eq!(
        converted.undo_action_category,
        EVIM_HISTORY_ACTION_CATEGORY_FILE_FORMAT
    );

    assert_eq!(
        unsafe { evim_core_view_undo(core.handle, view, &mut outcome) },
        EvimStatus::Ok
    );
    let undone = document_state(&core);
    assert_eq!(undone.style_sheet_revision, initial.style_sheet_revision);
    assert_eq!(undone.file_format, EVIM_FILE_FORMAT_DOS);
    assert_eq!(undone.file_format_origin, EVIM_FILE_FORMAT_ORIGIN_DETECTED);
    assert_eq!(undone.flags & EVIM_DOCUMENT_STATE_IS_DIRTY, 0);
    assert_ne!(undone.flags & EVIM_DOCUMENT_STATE_CAN_REDO, 0);
    assert_eq!(
        undone.redo_action_category,
        EVIM_HISTORY_ACTION_CATEGORY_FILE_FORMAT
    );
    assert_eq!(
        unsafe { evim_core_view_redo(core.handle, view, &mut outcome) },
        EvimStatus::Ok
    );

    let stale_save = EvimMarkSavedV1 {
        document_id: converted.document_id,
        document_revision: converted.document_revision,
        ..EvimMarkSavedV1::default()
    };
    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, 'i' as u32),
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_send_text(core.handle, view, b"y".as_ptr(), 1, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_mark_saved(core.handle, &stale_save) },
        EvimStatus::StaleRevision
    );
    assert_eq!(
        unsafe { evim_core_view_send_text(core.handle, view, b"z".as_ptr(), 1, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe {
            evim_core_view_send_key(core.handle, view, &key(EVIM_KEY_ESCAPE, 0), &mut outcome)
        },
        EvimStatus::Ok
    );
    let grouped = document_state(&core);
    assert_eq!(
        grouped.undo_action_category,
        EVIM_HISTORY_ACTION_CATEGORY_TEXT
    );
    assert_ne!(grouped.flags & EVIM_DOCUMENT_STATE_IS_DIRTY, 0);
    assert_eq!(
        unsafe { evim_core_view_undo(core.handle, view, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_redo(core.handle, view, &mut outcome) },
        EvimStatus::Ok
    );
    let current = document_state(&core);
    let exact_save = EvimMarkSavedV1 {
        document_id: current.document_id,
        document_revision: current.document_revision,
        ..EvimMarkSavedV1::default()
    };
    assert_eq!(
        unsafe { evim_core_mark_saved(core.handle, &exact_save) },
        EvimStatus::Ok
    );
    assert_eq!(
        document_state(&core).flags & EVIM_DOCUMENT_STATE_IS_DIRTY,
        0
    );

    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);
}

#[test]
fn typed_view_options_are_local_or_shared_and_fail_atomically() {
    let core = create_core(
        b"a\rb\n",
        EvimDocumentOptions {
            file_format: EVIM_FILE_FORMAT_UNIX,
            ..EvimDocumentOptions::default()
        },
    );
    let mut first_context = Box::new(FakeProviderContext::new(core.handle));
    let mut second_context = Box::new(FakeProviderContext::new(core.handle));
    let (first, mut outcome) = add_test_view(&core, &mut *first_context);
    let (second, _) = add_test_view(&core, &mut *second_context);
    assert_eq!(
        unsafe { evim_core_view_set_wrap(core.handle, first, 0, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_set_wrap(core.handle, second, 0, &mut outcome) },
        EvimStatus::Ok
    );

    let mut first_state = EvimViewportStateV1::default();
    let mut second_state = EvimViewportStateV1::default();
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, first, &mut first_state) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, second, &mut second_state) },
        EvimStatus::Ok
    );
    assert_ne!(first_state.flags & EVIM_VIEWPORT_STATE_LINEBREAK, 0);
    assert_ne!(second_state.flags & EVIM_VIEWPORT_STATE_LINEBREAK, 0);
    assert_eq!(first_state.scale, 1.0);
    assert_eq!(second_state.scale, 1.0);

    let document_revision = outcome.document_revision;
    let first_configuration = first_state.configuration_generation;
    let second_configuration = second_state.configuration_generation;
    let first_shape_calls = first_context.shape_calls;
    let second_shape_calls = second_context.shape_calls;
    assert_eq!(
        unsafe { evim_core_view_set_scale(core.handle, first, 1.5, &mut outcome) },
        EvimStatus::Ok
    );
    assert_ne!(outcome.flags & EVIM_OUTCOME_LAYOUT_CHANGED, 0);
    assert_eq!(outcome.document_revision, document_revision);
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, first, &mut first_state) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, second, &mut second_state) },
        EvimStatus::Ok
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
    for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        outcome = EvimCoreOutcomeV1 {
            flags: u32::MAX,
            ..EvimCoreOutcomeV1::default()
        };
        assert_eq!(
            unsafe { evim_core_view_set_scale(core.handle, first, invalid, &mut outcome) },
            EvimStatus::InvalidArgument
        );
        assert_eq!(outcome, EvimCoreOutcomeV1::default());
        assert_eq!(
            unsafe { evim_core_view_viewport_state(core.handle, first, &mut first_state) },
            EvimStatus::Ok
        );
        assert_eq!(first_state, scaled_state);
    }

    assert_eq!(
        unsafe { evim_core_view_set_linebreak(core.handle, first, 0, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, first, &mut first_state) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, second, &mut second_state) },
        EvimStatus::Ok
    );
    assert_eq!(first_state.flags & EVIM_VIEWPORT_STATE_LINEBREAK, 0);
    assert_ne!(second_state.flags & EVIM_VIEWPORT_STATE_LINEBREAK, 0);
    assert_eq!(
        unsafe { evim_core_view_set_linebreak(core.handle, first, 2, &mut outcome) },
        EvimStatus::InvalidArgument
    );
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, first, &mut first_state) },
        EvimStatus::Ok
    );
    assert_eq!(first_state.flags & EVIM_VIEWPORT_STATE_LINEBREAK, 0);

    let before = document_state(&core);
    let invalid = EvimSetFileFormatV1 {
        file_format: EVIM_FILE_FORMAT_DETECT,
        document_id: before.document_id,
        document_revision: before.document_revision,
        ..EvimSetFileFormatV1::default()
    };
    assert_eq!(
        unsafe { evim_core_view_set_file_format(core.handle, first, &invalid, &mut outcome) },
        EvimStatus::InvalidFileFormat
    );
    let stale = EvimSetFileFormatV1 {
        file_format: EVIM_FILE_FORMAT_DOS,
        document_id: before.document_id,
        document_revision: before.document_revision.saturating_add(1),
        ..EvimSetFileFormatV1::default()
    };
    assert_eq!(
        unsafe { evim_core_view_set_file_format(core.handle, first, &stale, &mut outcome) },
        EvimStatus::StaleRevision
    );
    let wrong_document = EvimSetFileFormatV1 {
        file_format: EVIM_FILE_FORMAT_DOS,
        document_id: before.document_id.saturating_add(1),
        document_revision: before.document_revision,
        ..EvimSetFileFormatV1::default()
    };
    assert_eq!(
        unsafe {
            evim_core_view_set_file_format(core.handle, first, &wrong_document, &mut outcome)
        },
        EvimStatus::InvalidArgument
    );
    let rejected = EvimSetFileFormatV1 {
        file_format: EVIM_FILE_FORMAT_MAC,
        document_id: before.document_id,
        document_revision: before.document_revision,
        ..EvimSetFileFormatV1::default()
    };
    assert_eq!(
        unsafe { evim_core_view_set_file_format(core.handle, second, &rejected, &mut outcome) },
        EvimStatus::PolicyRequired
    );
    assert_eq!(document_state(&core), before);

    let valid = EvimSetFileFormatV1 {
        file_format: EVIM_FILE_FORMAT_DOS,
        document_id: before.document_id,
        document_revision: before.document_revision,
        ..EvimSetFileFormatV1::default()
    };
    assert_eq!(
        unsafe { evim_core_view_set_file_format(core.handle, second, &valid, &mut outcome) },
        EvimStatus::Ok
    );
    assert_ne!(outcome.flags & EVIM_OUTCOME_DOCUMENT_CHANGED, 0);
    let shared = document_state(&core);
    assert_eq!(shared.file_format, EVIM_FILE_FORMAT_DOS);
    assert_eq!(
        shared.undo_action_category,
        EVIM_HISTORY_ACTION_CATEGORY_FILE_FORMAT
    );
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, first, &mut first_state) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, second, &mut second_state) },
        EvimStatus::Ok
    );
    assert_eq!(first_state.document_revision, shared.document_revision);
    assert_eq!(second_state.document_revision, shared.document_revision);
    assert_eq!(first_state.flags & EVIM_VIEWPORT_STATE_LINEBREAK, 0);
    assert_ne!(second_state.flags & EVIM_VIEWPORT_STATE_LINEBREAK, 0);

    assert_eq!(evim_core_view_remove(core.handle, first), EvimStatus::Ok);
    assert_eq!(evim_core_view_remove(core.handle, second), EvimStatus::Ok);
}

#[test]
fn zoom_reflows_only_a_bounded_viewport_of_a_large_document() {
    let source = "short proportional line\n".repeat(100_000);
    let core = create_core(source.as_bytes(), EvimDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);
    let mut before = EvimViewportStateV1::default();
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, &mut before) },
        EvimStatus::Ok
    );

    context.shaped_bytes = 0;
    context.minimum_request_start = u64::MAX;
    context.maximum_request_end = 0;
    assert_eq!(
        unsafe { evim_core_view_set_scale(core.handle, view, 1.25, &mut outcome) },
        EvimStatus::Ok
    );

    let mut after = EvimViewportStateV1::default();
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, &mut after) },
        EvimStatus::Ok
    );
    assert_eq!(after.scale, 1.25);
    assert!(after.configuration_generation > before.configuration_generation);
    assert_eq!(after.document_revision, before.document_revision);
    assert!(context.shaped_bytes < 10_000);
    assert!(context.maximum_request_end < source.len() as u64);

    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);
}

#[test]
fn layout_snapshot_export_geometry_hit_testing_and_pointer_placement_are_revision_bound() {
    let core = create_core(b"abc\nfi", EvimDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);

    let mut info = EvimLayoutSnapshotInfoV1::default();
    assert_eq!(
        unsafe { evim_core_view_layout_snapshot_info(core.handle, view, &mut info) },
        EvimStatus::Ok
    );
    assert_eq!(info.struct_size, EVIM_LAYOUT_SNAPSHOT_INFO_V1_SIZE);
    assert_eq!(
        info.identity.struct_size,
        EVIM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE
    );
    assert_eq!(info.identity.view_id, view);
    assert_eq!(info.identity.document_revision, 0);
    assert_ne!(info.identity.layout_revision, 0);
    assert!(info.row_count >= 2);
    assert!(info.cluster_count >= 4, "fi may be one shaping cluster");
    assert!(info.caret_count >= info.cluster_count);
    assert!(info.total_height > 0.0);
    assert!(info.coverage_y_end > info.coverage_y_start);

    let mut queried = EvimLayoutSnapshotInfoV1::default();
    assert_eq!(
        unsafe {
            evim_core_view_copy_layout_snapshot(
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
        EvimStatus::BufferTooSmall
    );
    assert_eq!(queried, info);

    let mut wrong_view = info.identity;
    wrong_view.view_id += 1;
    assert_eq!(
        unsafe {
            evim_core_view_copy_layout_snapshot(
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
        EvimStatus::StaleRevision
    );

    let mut rows = vec![EvimVisualRowV1::default(); info.row_count as usize];
    let mut clusters = vec![EvimPositionedClusterV1::default(); info.cluster_count as usize];
    let mut carets = vec![EvimPositionedCaretV1::default(); info.caret_count as usize];
    let mut copied = EvimLayoutSnapshotInfoV1::default();
    rows[0].row_index = u64::MAX;
    clusters[0].text_start = u64::MAX;
    carets[0].text_offset = u64::MAX;
    assert_eq!(
        unsafe {
            evim_core_view_copy_layout_snapshot(
                core.handle,
                view,
                &EvimLayoutSnapshotIdentityV1 {
                    struct_size: EVIM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE + 8,
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
        EvimStatus::BufferTooSmall
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
            evim_core_view_copy_layout_snapshot(
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
        EvimStatus::Ok
    );
    assert_eq!(copied, info);
    for (row_index, row) in rows.iter().enumerate() {
        assert_eq!(row.struct_size, EVIM_VISUAL_ROW_V1_SIZE);
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
        assert_eq!(cluster.struct_size, EVIM_POSITIONED_CLUSTER_V1_SIZE);
        assert!(cluster.text_start < cluster.text_end);
        assert!(cluster.advance > 0.0);
        assert_ne!(cluster.flags & EVIM_POSITIONED_CLUSTER_HAS_RENDER_RUN, 0);
        assert_eq!(cluster.render_run.owner, 0xf00d);
        assert_eq!(
            cluster.render_run.metrics_generation,
            info.identity.metrics_generation
        );
    }

    let row = rows.first().unwrap();
    let logical = carets
        .iter()
        .find(|caret| caret.text_offset == 1 && caret.affinity == EVIM_BOUNDARY_AFFINITY_DOWNSTREAM)
        .copied()
        .unwrap();
    let caret_request = EvimLayoutCaretRequestV1 {
        struct_size: EVIM_LAYOUT_CARET_REQUEST_V1_SIZE,
        affinity: logical.affinity,
        identity: info.identity,
        text_offset: logical.text_offset,
    };
    let mut geometry = EvimLayoutCaretGeometryV1::default();
    assert_eq!(
        unsafe { evim_core_view_caret_geometry(core.handle, view, &caret_request, &mut geometry) },
        EvimStatus::Ok
    );
    assert_eq!(geometry.struct_size, EVIM_LAYOUT_CARET_GEOMETRY_V1_SIZE);
    assert_eq!(geometry.point.text_offset, logical.text_offset);
    assert_eq!(geometry.point.affinity, logical.affinity);
    assert_eq!(geometry.rect.x, logical.x);
    assert_eq!(geometry.rect.y, row.y);
    assert_eq!(geometry.rect.height, row.ascent + row.descent);

    let hit_request = EvimLayoutHitTestRequestV1 {
        struct_size: EVIM_LAYOUT_HIT_TEST_REQUEST_V1_SIZE,
        reserved: 0,
        identity: info.identity,
        x: logical.x,
        y: row.y + 1.0,
    };
    let mut hit = EvimLayoutCaretPointV1::default();
    assert_eq!(
        unsafe { evim_core_view_layout_hit_test(core.handle, view, &hit_request, &mut hit) },
        EvimStatus::Ok
    );
    assert_eq!(hit.struct_size, EVIM_LAYOUT_CARET_POINT_V1_SIZE);
    assert_eq!(hit.document_id, info.identity.document_id);
    assert_eq!(hit.document_revision, info.identity.document_revision);
    assert_eq!(hit.layout_revision, info.identity.layout_revision);

    let place = EvimPlaceCursorV1 {
        struct_size: EVIM_PLACE_CURSOR_V1_SIZE,
        flags: 0,
        document_revision: hit.document_revision,
        text_offset: hit.text_offset,
        affinity: hit.affinity,
        reserved: 0,
    };
    assert_eq!(
        unsafe { evim_core_view_place_cursor(core.handle, view, &place, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(outcome.cursor_utf8_offset, hit.text_offset);
    let mut presentation = EvimViewPresentationV1::default();
    assert_eq!(
        unsafe { evim_core_view_presentation(core.handle, view, &mut presentation) },
        EvimStatus::Ok
    );
    assert_eq!(presentation.cursor_utf8_offset, hit.text_offset);
    assert_eq!(presentation.cursor_affinity, hit.affinity);

    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, 'v' as u32),
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    let extend = EvimPlaceCursorV1 {
        flags: EVIM_PLACE_CURSOR_EXTEND_SELECTION,
        text_offset: 2,
        affinity: EVIM_BOUNDARY_AFFINITY_DOWNSTREAM,
        ..place
    };
    assert_eq!(
        unsafe { evim_core_view_place_cursor(core.handle, view, &extend, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_presentation(core.handle, view, &mut presentation) },
        EvimStatus::Ok
    );
    assert_eq!(presentation.mode, EVIM_MODE_VISUAL_CHARACTER);
    assert_ne!(
        presentation.flags & EVIM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR,
        0
    );
    assert_eq!(presentation.visual_anchor_utf8_offset, hit.text_offset);
    assert_eq!(presentation.cursor_utf8_offset, 2);

    let stale_identity = info.identity;
    assert_eq!(
        unsafe { evim_core_view_resize(core.handle, view, 300.0, 120.0, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe {
            evim_core_view_copy_layout_snapshot(
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
        EvimStatus::StaleRevision
    );

    context.metrics_generation += 1;
    assert_eq!(
        unsafe { evim_core_view_layout_snapshot_info(core.handle, view, &mut info) },
        EvimStatus::LayoutUnavailable
    );

    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);
}

#[test]
fn layout_paint_export_is_exact_revision_bound_and_uses_explicit_rgba_flags() {
    let core = create_core(b"paint", EvimDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);

    let mut paint = EvimLayoutPaintInfoV1::default();
    assert_eq!(
        unsafe { evim_core_view_layout_paint_info(core.handle, view, &mut paint) },
        EvimStatus::Ok
    );
    assert_eq!(paint.struct_size, EVIM_LAYOUT_PAINT_INFO_V1_SIZE);
    assert_eq!(
        paint.identity.struct_size,
        EVIM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE
    );
    assert_eq!(paint.identity.view_id, view);
    assert_eq!(
        paint.canvas_background,
        EvimRgbaV1 {
            red: 1.0,
            green: 1.0,
            blue: 1.0,
            alpha: 1.0,
        }
    );
    assert_eq!(paint.default_paint.struct_size, EVIM_TEXT_PAINT_V1_SIZE);
    assert_eq!(
        paint.default_paint.flags,
        EVIM_TEXT_PAINT_DEFAULT_FOREGROUND
    );
    assert_eq!(paint.flags, EVIM_LAYOUT_PAINT_DEFAULT_CANVAS);
    assert_eq!(
        paint.default_paint.foreground,
        EvimRgbaV1 {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 1.0,
        }
    );
    assert_eq!(paint.default_paint.background, EvimRgbaV1::default());
    assert_eq!(paint.paint_run_count, 0);

    let mut copied = EvimLayoutPaintInfoV1::default();
    assert_eq!(
        unsafe {
            evim_core_view_copy_layout_paint(
                core.handle,
                view,
                &paint.identity,
                ptr::null_mut(),
                0,
                &mut copied,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(copied, paint);

    let mut wrong_view = paint.identity;
    wrong_view.view_id += 1;
    assert_eq!(
        unsafe {
            evim_core_view_copy_layout_paint(
                core.handle,
                view,
                &wrong_view,
                ptr::null_mut(),
                0,
                &mut copied,
            )
        },
        EvimStatus::StaleRevision
    );
    assert_eq!(copied, EvimLayoutPaintInfoV1::default());

    let stale = paint.identity;
    assert_eq!(
        unsafe { evim_core_view_resize(core.handle, view, 320.0, 180.0, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe {
            evim_core_view_copy_layout_paint(
                core.handle,
                view,
                &stale,
                ptr::null_mut(),
                0,
                &mut copied,
            )
        },
        EvimStatus::StaleRevision
    );

    context.metrics_generation += 1;
    assert_eq!(
        unsafe { evim_core_view_layout_paint_info(core.handle, view, &mut paint) },
        EvimStatus::LayoutUnavailable
    );
    assert_eq!(paint, EvimLayoutPaintInfoV1::default());

    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);
}

#[test]
fn command_line_export_is_exact_kind_cursor_and_utf8_state() {
    let core = create_core(b"text", EvimDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);

    let mut info = EvimCommandLineInfoV1::default();
    assert_eq!(
        unsafe { evim_core_view_command_line_info(core.handle, view, &mut info) },
        EvimStatus::Ok
    );
    assert_eq!(info.struct_size, EVIM_COMMAND_LINE_INFO_V1_SIZE);
    assert_eq!(info.identity.kind, EVIM_COMMAND_LINE_KIND_NONE);
    assert_eq!(info.identity.view_id, view);
    assert_eq!(info.identity.document_revision, 0);
    assert_eq!(info.utf8_length, 0);
    let mut copied_info = EvimCommandLineInfoV1::default();
    assert_eq!(
        unsafe {
            evim_core_view_copy_command_line(
                core.handle,
                view,
                &info.identity,
                ptr::null_mut(),
                0,
                &mut copied_info,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(copied_info, info);

    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, ':' as u32),
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    let command = "set cafés";
    assert_eq!(
        unsafe {
            evim_core_view_send_text(
                core.handle,
                view,
                command.as_ptr(),
                command.len() as u64,
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_command_line_info(core.handle, view, &mut info) },
        EvimStatus::Ok
    );
    assert_eq!(info.identity.kind, EVIM_COMMAND_LINE_KIND_EX);
    assert_eq!(info.utf8_length, command.len() as u64);
    assert_eq!(info.cursor_utf8_offset, command.len() as u64);

    let mut bytes = vec![0xa5; command.len()];
    assert_eq!(
        unsafe {
            evim_core_view_copy_command_line(
                core.handle,
                view,
                &info.identity,
                bytes.as_mut_ptr(),
                bytes.len().saturating_sub(1) as u64,
                &mut copied_info,
            )
        },
        EvimStatus::BufferTooSmall
    );
    assert_eq!(copied_info, info);
    assert!(bytes.iter().all(|byte| *byte == 0xa5));
    assert_eq!(
        unsafe {
            evim_core_view_copy_command_line(
                core.handle,
                view,
                &info.identity,
                bytes.as_mut_ptr(),
                bytes.len() as u64,
                &mut copied_info,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(bytes, command.as_bytes());

    let stale = info.identity;
    assert_eq!(
        unsafe { evim_core_view_send_key(core.handle, view, &key(EVIM_KEY_LEFT, 0), &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe {
            evim_core_view_copy_command_line(
                core.handle,
                view,
                &stale,
                bytes.as_mut_ptr(),
                bytes.len() as u64,
                &mut copied_info,
            )
        },
        EvimStatus::StaleRevision,
        "command cursor movement must stale an otherwise unchanged export"
    );
    assert_eq!(copied_info, EvimCommandLineInfoV1::default());

    for (prefix, expected_kind) in [
        ('/', EVIM_COMMAND_LINE_KIND_SEARCH_FORWARD),
        ('?', EVIM_COMMAND_LINE_KIND_SEARCH_BACKWARD),
    ] {
        assert_eq!(
            unsafe {
                evim_core_view_send_key(core.handle, view, &key(EVIM_KEY_ESCAPE, 0), &mut outcome)
            },
            EvimStatus::Ok
        );
        assert_eq!(
            unsafe {
                evim_core_view_send_key(
                    core.handle,
                    view,
                    &key(EVIM_KEY_CHARACTER, prefix as u32),
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
        assert_eq!(
            unsafe { evim_core_view_command_line_info(core.handle, view, &mut info) },
            EvimStatus::Ok
        );
        assert_eq!(info.identity.kind, expected_kind);
        assert_eq!(info.utf8_length, 0);
        assert_eq!(info.cursor_utf8_offset, 0);
    }

    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);
}

#[test]
fn visual_selection_export_preserves_character_line_and_block_semantics() {
    let core = create_core(b"ab\ncd", EvimDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);

    let mut info = EvimVisualSelectionInfoV1::default();
    assert_eq!(
        unsafe { evim_core_view_visual_selection_info(core.handle, view, &mut info) },
        EvimStatus::Ok
    );
    assert_eq!(info.identity.kind, EVIM_VISUAL_SELECTION_KIND_NONE);
    assert_eq!(info.segment_count, 0);
    assert_eq!(info.rectangle_count, 0);

    for character in ['v', 'l'] {
        assert_eq!(
            unsafe {
                evim_core_view_send_key(
                    core.handle,
                    view,
                    &key(EVIM_KEY_CHARACTER, character as u32),
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
    }
    assert_eq!(
        unsafe { evim_core_view_visual_selection_info(core.handle, view, &mut info) },
        EvimStatus::Ok
    );
    assert_eq!(info.identity.kind, EVIM_VISUAL_SELECTION_KIND_CHARACTER);
    assert_eq!(info.segment_count, 1);
    assert!(info.rectangle_count >= 1);
    let mut character_segments =
        vec![EvimVisualSelectionSegmentV1::default(); info.segment_count as usize];
    let mut character_rectangles =
        vec![EvimVisualSelectionRectangleV1::default(); info.rectangle_count as usize];
    let mut copied = EvimVisualSelectionInfoV1::default();
    assert_eq!(
        unsafe {
            evim_core_view_copy_visual_selection(
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
        EvimStatus::Ok
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
            evim_core_view_send_key(core.handle, view, &key(EVIM_KEY_ESCAPE, 0), &mut outcome)
        },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, 'V' as u32),
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_visual_selection_info(core.handle, view, &mut info) },
        EvimStatus::Ok
    );
    assert_eq!(info.identity.kind, EVIM_VISUAL_SELECTION_KIND_LINE);
    let mut line_segments =
        vec![EvimVisualSelectionSegmentV1::default(); info.segment_count as usize];
    let mut line_rectangles =
        vec![EvimVisualSelectionRectangleV1::default(); info.rectangle_count as usize];
    assert_eq!(
        unsafe {
            evim_core_view_copy_visual_selection(
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
        EvimStatus::Ok
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
            evim_core_view_send_key(core.handle, view, &key(EVIM_KEY_ESCAPE, 0), &mut outcome)
        },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, '0' as u32),
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    for input in [
        key(EVIM_KEY_CONTROL_CHARACTER, 'v' as u32),
        key(EVIM_KEY_CHARACTER, 'l' as u32),
        key(EVIM_KEY_CHARACTER, 'j' as u32),
    ] {
        assert_eq!(
            unsafe { evim_core_view_send_key(core.handle, view, &input, &mut outcome) },
            EvimStatus::Ok
        );
    }
    assert_eq!(
        unsafe { evim_core_view_visual_selection_info(core.handle, view, &mut info) },
        EvimStatus::Ok
    );
    assert_eq!(info.identity.kind, EVIM_VISUAL_SELECTION_KIND_BLOCK);
    assert_eq!(info.segment_count, 2);
    let mut block_segments =
        vec![EvimVisualSelectionSegmentV1::default(); info.segment_count as usize];
    let mut block_rectangles =
        vec![EvimVisualSelectionRectangleV1::default(); info.rectangle_count as usize];
    block_segments[0].text_start = u64::MAX;
    block_rectangles[0].rect.x = f32::MAX;
    assert_eq!(
        unsafe {
            evim_core_view_copy_visual_selection(
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
        EvimStatus::BufferTooSmall
    );
    assert_eq!(copied, info);
    assert_eq!(block_segments[0].text_start, u64::MAX);
    assert_eq!(block_rectangles[0].rect.x, f32::MAX);
    assert_eq!(
        unsafe {
            evim_core_view_copy_visual_selection(
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
        EvimStatus::Ok
    );
    assert_eq!(
        block_segments
            .iter()
            .map(|segment| segment.text_start..segment.text_end)
            .collect::<Vec<_>>(),
        vec![0..2, 3..5]
    );
    let block_flags = EVIM_VISUAL_SELECTION_SEGMENT_HAS_VISUAL_ROW
        | EVIM_VISUAL_SELECTION_SEGMENT_HAS_HARD_LINE
        | EVIM_VISUAL_SELECTION_SEGMENT_HAS_EDGE_AFFINITIES;
    for (index, segment) in block_segments.iter().enumerate() {
        assert_eq!(segment.struct_size, EVIM_VISUAL_SELECTION_SEGMENT_V1_SIZE);
        assert_eq!(segment.flags, block_flags);
        assert_eq!(segment.row_index, index as u64);
        assert_eq!(segment.hard_line_index, index as u64);
        assert!(matches!(
            segment.left_affinity,
            EVIM_BOUNDARY_AFFINITY_UPSTREAM | EVIM_BOUNDARY_AFFINITY_DOWNSTREAM
        ));
        assert!(matches!(
            segment.right_affinity,
            EVIM_BOUNDARY_AFFINITY_UPSTREAM | EVIM_BOUNDARY_AFFINITY_DOWNSTREAM
        ));
    }
    assert!(block_rectangles.windows(2).all(|pair| {
        (pair[0].row_index, pair[0].rect.x) <= (pair[1].row_index, pair[1].rect.x)
    }));
    for rectangle in &block_rectangles {
        assert_eq!(
            rectangle.struct_size,
            EVIM_VISUAL_SELECTION_RECTANGLE_V1_SIZE
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
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, 'h' as u32),
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe {
            evim_core_view_copy_visual_selection(
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
        EvimStatus::StaleRevision,
        "Visual motion without relayout must stale the selection state token"
    );
    assert_eq!(copied, EvimVisualSelectionInfoV1::default());

    let stale_layout = {
        assert_eq!(
            unsafe { evim_core_view_visual_selection_info(core.handle, view, &mut info) },
            EvimStatus::Ok
        );
        info.identity
    };
    assert_eq!(
        unsafe { evim_core_view_resize(core.handle, view, 320.0, 160.0, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe {
            evim_core_view_copy_visual_selection(
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
        EvimStatus::StaleRevision
    );

    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);
}

#[test]
fn native_find_selection_is_exact_literal_and_reveal_requires_visual_state() {
    let core = create_core(b"a.c abc a.c", EvimDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);

    assert_eq!(
        unsafe { evim_core_view_reveal_selection(core.handle, view, &mut outcome) },
        EvimStatus::InvalidRange
    );
    assert_eq!(outcome, EvimCoreOutcomeV1::default());

    for input in [
        key(EVIM_KEY_CHARACTER, 'v' as u32),
        key(EVIM_KEY_CHARACTER, '2' as u32),
        key(EVIM_KEY_CHARACTER, 'l' as u32),
    ] {
        assert_eq!(
            unsafe { evim_core_view_send_key(core.handle, view, &input, &mut outcome) },
            EvimStatus::Ok
        );
    }

    let mut selection = EvimVisualSelectionInfoV1::default();
    assert_eq!(
        unsafe { evim_core_view_visual_selection_info(core.handle, view, &mut selection) },
        EvimStatus::Ok
    );
    assert_eq!(
        selection.identity.kind,
        EVIM_VISUAL_SELECTION_KIND_CHARACTER
    );
    assert_eq!(
        unsafe {
            evim_core_view_use_selection_for_find(
                core.handle,
                view,
                &selection.identity,
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(outcome.cursor_utf8_offset, 2);
    assert_eq!(outcome.document_revision, 0);

    let stale = selection.identity;
    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, 'h' as u32),
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_use_selection_for_find(core.handle, view, &stale, &mut outcome) },
        EvimStatus::StaleRevision
    );
    assert_eq!(outcome, EvimCoreOutcomeV1::default());

    assert_eq!(
        unsafe { evim_core_view_reveal_selection(core.handle, view, &mut outcome) },
        EvimStatus::Ok
    );
    for input in [
        key(EVIM_KEY_ESCAPE, 0),
        key(EVIM_KEY_CHARACTER, '0' as u32),
        key(EVIM_KEY_CHARACTER, 'n' as u32),
    ] {
        assert_eq!(
            unsafe { evim_core_view_send_key(core.handle, view, &input, &mut outcome) },
            EvimStatus::Ok
        );
    }
    assert_eq!(
        outcome.cursor_utf8_offset, 8,
        "selection metacharacters must remain literal in the core search"
    );

    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);
}

#[test]
fn visual_selection_reports_outside_partial_layout_coverage() {
    let source = "row\n".repeat(3_000);
    let core = create_core(source.as_bytes(), EvimDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);
    let mut layout = EvimLayoutSnapshotInfoV1::default();
    assert_eq!(
        unsafe { evim_core_view_layout_snapshot_info(core.handle, view, &mut layout) },
        EvimStatus::Ok
    );
    assert_eq!(layout.flags & EVIM_LAYOUT_SNAPSHOT_FULL_DOCUMENT, 0);

    for character in ['V', 'G'] {
        assert_eq!(
            unsafe {
                evim_core_view_send_key(
                    core.handle,
                    view,
                    &key(EVIM_KEY_CHARACTER, character as u32),
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
    }
    let mut selection = EvimVisualSelectionInfoV1::default();
    assert_eq!(
        unsafe { evim_core_view_visual_selection_info(core.handle, view, &mut selection) },
        EvimStatus::OutsideLayoutCoverage
    );
    assert_eq!(selection, EvimVisualSelectionInfoV1::default());

    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);
}

#[test]
fn native_ffi_history_is_mode_independent_and_finalizes_insert_grouping() {
    let core = create_core(b"base", EvimDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *context);

    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, 'i' as u32),
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(outcome.mode, EVIM_MODE_INSERT);
    for byte in [b"x".as_slice(), b"y".as_slice()] {
        assert_eq!(
            unsafe {
                evim_core_view_send_text(
                    core.handle,
                    view,
                    byte.as_ptr(),
                    byte.len() as u64,
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
    }
    assert_eq!(
        copy_core_bytes(
            evim_core_copy_formatted_utf8,
            &core,
            outcome.document_revision,
        ),
        b"xybase"
    );

    assert_eq!(
        unsafe { evim_core_view_undo(core.handle, view, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(outcome.command_status, EVIM_COMMAND_STATUS_COMPLETE);
    assert_eq!(outcome.mode, EVIM_MODE_NORMAL);
    assert_ne!(outcome.flags & EVIM_OUTCOME_DOCUMENT_CHANGED, 0);
    assert_ne!(outcome.flags & EVIM_OUTCOME_MODE_CHANGED, 0);
    assert_eq!(
        copy_core_bytes(
            evim_core_copy_formatted_utf8,
            &core,
            outcome.document_revision,
        ),
        b"base",
        "one native undo removes the complete open Insert unit"
    );

    assert_eq!(
        unsafe { evim_core_view_redo(core.handle, view, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(outcome.mode, EVIM_MODE_NORMAL);
    assert_eq!(outcome.cursor_utf8_offset, 2);
    assert_eq!(
        copy_core_bytes(
            evim_core_copy_formatted_utf8,
            &core,
            outcome.document_revision,
        ),
        b"xybase"
    );

    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, 'v' as u32),
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(outcome.mode, EVIM_MODE_VISUAL_CHARACTER);
    assert_eq!(
        unsafe { evim_core_view_undo(core.handle, view, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(outcome.mode, EVIM_MODE_NORMAL);
    assert_ne!(outcome.flags & EVIM_OUTCOME_MODE_CHANGED, 0);
    let mut presentation = EvimViewPresentationV1::default();
    assert_eq!(
        unsafe { evim_core_view_presentation(core.handle, view, &mut presentation) },
        EvimStatus::Ok
    );
    assert_eq!(
        presentation.flags & EVIM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR,
        0
    );
    assert_eq!(
        copy_core_bytes(
            evim_core_copy_formatted_utf8,
            &core,
            outcome.document_revision,
        ),
        b"base"
    );

    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);
}

#[test]
fn viewport_origin_api_is_identity_bound_bounded_and_atomic() {
    let source = (0..2_000)
        .map(|line| format!("WWWWWWWWWWWWWWWW line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let core = create_core(source.as_bytes(), EvimDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let provider = provider(
        (&mut *context as *mut FakeProviderContext).cast(),
        fake_shape_batch,
    );
    let options = EvimViewOptionsV1 {
        width: 40.0,
        height: 32.0,
        ..EvimViewOptionsV1::default()
    };
    let mut view = 0;
    let mut outcome = EvimCoreOutcomeV1::default();
    assert_eq!(
        unsafe { evim_core_view_add(core.handle, &options, &provider, &mut view, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_set_wrap(core.handle, view, 0, &mut outcome) },
        EvimStatus::Ok
    );

    let mut state = EvimViewportStateV1::default();
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, &mut state) },
        EvimStatus::Ok
    );
    assert_eq!(state.left, 0.0);
    assert_ne!(state.flags & EVIM_VIEWPORT_STATE_HAS_LAYOUT, 0);
    assert_ne!(state.flags & EVIM_VIEWPORT_STATE_TOP_EXACT, 0);
    assert_eq!(state.document_revision, outcome.document_revision);
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
    let request = EvimViewportOriginV1 {
        left: 40.0,
        expected_document_id: state.document_id,
        expected_document_revision: state.document_revision,
        expected_layout_revision: state.layout_revision,
        expected_configuration_generation: state.configuration_generation,
        expected_measurement_environment_id: state.measurement_environment_id,
        expected_metrics_generation: state.metrics_generation,
        ..EvimViewportOriginV1::default()
    };
    assert_eq!(
        unsafe { evim_core_view_set_viewport_origin(core.handle, view, &request, &mut outcome) },
        EvimStatus::Ok
    );
    assert_ne!(outcome.flags & EVIM_OUTCOME_LAYOUT_CHANGED, 0);
    assert_eq!(outcome.layout_revision, layout_revision);
    assert_eq!(outcome.configuration_generation, configuration_generation);
    assert_eq!(context.shape_calls, shape_calls);
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, &mut state) },
        EvimStatus::Ok
    );
    assert_eq!(state.left, 40.0);

    for invalid in [
        EvimViewportOriginV1 {
            struct_size: EVIM_VIEWPORT_ORIGIN_V1_SIZE - 1,
            left: 60.0,
            ..request
        },
        EvimViewportOriginV1 {
            flags: 1 << 31,
            left: 60.0,
            ..request
        },
        EvimViewportOriginV1 {
            left: f32::NAN,
            ..request
        },
        EvimViewportOriginV1 {
            flags: EVIM_VIEWPORT_ORIGIN_HAS_TOP,
            left: 60.0,
            top: f32::INFINITY,
            ..request
        },
    ] {
        assert_eq!(
            unsafe {
                evim_core_view_set_viewport_origin(core.handle, view, &invalid, &mut outcome)
            },
            EvimStatus::InvalidArgument
        );
        assert_eq!(
            unsafe { evim_core_view_viewport_state(core.handle, view, &mut state) },
            EvimStatus::Ok
        );
        assert_eq!(state.left, 40.0);
    }

    let vertical = EvimViewportOriginV1 {
        flags: EVIM_VIEWPORT_ORIGIN_HAS_TOP,
        left: 60.0,
        top: 14_000.0,
        ..request
    };
    let shaped_bytes = context.shaped_bytes;
    context.minimum_request_start = u64::MAX;
    context.maximum_request_end = 0;
    assert_eq!(
        unsafe { evim_core_view_set_viewport_origin(core.handle, view, &vertical, &mut outcome) },
        EvimStatus::Ok
    );
    assert_ne!(outcome.flags & EVIM_OUTCOME_LAYOUT_CHANGED, 0);
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, &mut state) },
        EvimStatus::Ok
    );
    assert_eq!(state.left, 60.0);
    assert!(state.top > 10_000.0);
    assert_eq!(state.flags & EVIM_VIEWPORT_STATE_TOP_EXACT, 0);
    assert!(context.minimum_request_start > 1_000);
    assert!(context.maximum_request_end < source.len() as u64);
    assert!(context.shaped_bytes - shaped_bytes < 10_000);

    let mut layout_info = EvimLayoutSnapshotInfoV1::default();
    assert_eq!(
        unsafe { evim_core_view_layout_snapshot_info(core.handle, view, &mut layout_info) },
        EvimStatus::Ok
    );
    assert!(layout_info.coverage_hard_line_start > 500);
    assert!(layout_info.coverage_hard_line_end - layout_info.coverage_hard_line_start < 100);
    assert!(layout_info.coverage_y_start <= state.top);
    assert!(state.top + options.height <= layout_info.coverage_y_end);

    let after_down = state;
    outcome = EvimCoreOutcomeV1 {
        flags: u32::MAX,
        ..EvimCoreOutcomeV1::default()
    };
    assert_eq!(
        unsafe { evim_core_view_set_viewport_origin(core.handle, view, &vertical, &mut outcome) },
        EvimStatus::StaleRevision
    );
    assert_eq!(outcome, EvimCoreOutcomeV1::default());
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, &mut state) },
        EvimStatus::Ok
    );
    assert_eq!(
        state, after_down,
        "a stale request changes neither axis nor layout"
    );

    let request_from_state =
        |state: EvimViewportStateV1, left: f32, top: f32| EvimViewportOriginV1 {
            flags: EVIM_VIEWPORT_ORIGIN_HAS_TOP,
            left,
            top,
            expected_document_id: state.document_id,
            expected_document_revision: state.document_revision,
            expected_layout_revision: state.layout_revision,
            expected_configuration_generation: state.configuration_generation,
            expected_measurement_environment_id: state.measurement_environment_id,
            expected_metrics_generation: state.metrics_generation,
            ..EvimViewportOriginV1::default()
        };
    let upward = request_from_state(state, 20.0, 1_000.0);
    assert_eq!(
        unsafe { evim_core_view_set_viewport_origin(core.handle, view, &upward, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, &mut state) },
        EvimStatus::Ok
    );
    assert_eq!(state.left, 20.0);
    assert!(state.top < after_down.top);

    let bottom = request_from_state(state, 30.0, f32::MAX);
    assert_eq!(
        unsafe { evim_core_view_set_viewport_origin(core.handle, view, &bottom, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, &mut state) },
        EvimStatus::Ok
    );
    assert_eq!(state.left, 30.0);
    assert_eq!(
        unsafe { evim_core_view_layout_snapshot_info(core.handle, view, &mut layout_info) },
        EvimStatus::Ok
    );
    assert_eq!(layout_info.coverage_hard_line_end, 2_000);
    assert_eq!(
        state.top,
        (layout_info.coverage_y_end - options.height).max(layout_info.coverage_y_start)
    );

    let top = request_from_state(state, 10.0, -100.0);
    assert_eq!(
        unsafe { evim_core_view_set_viewport_origin(core.handle, view, &top, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, &mut state) },
        EvimStatus::Ok
    );
    assert_eq!(state.left, 10.0);
    assert_eq!(state.top, 0.0);
    assert_ne!(state.flags & EVIM_VIEWPORT_STATE_TOP_EXACT, 0);

    let before_failure = state;
    context.fail_next = Some(EvimStatus::ProviderFailure);
    let failed = request_from_state(state, 70.0, 5_000.0);
    assert_eq!(
        unsafe { evim_core_view_set_viewport_origin(core.handle, view, &failed, &mut outcome) },
        EvimStatus::ProviderFailure
    );
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, &mut state) },
        EvimStatus::Ok
    );
    assert_eq!(
        state, before_failure,
        "provider failure publishes no staged state"
    );

    #[repr(C, align(8))]
    struct OverlapStorage([u8; 128]);
    let mut overlap = OverlapStorage([0; 128]);
    let overlap_request = overlap.0.as_mut_ptr().cast::<EvimViewportOriginV1>();
    unsafe { overlap_request.write(request) };
    assert_eq!(
        unsafe {
            evim_core_view_set_viewport_origin(
                core.handle,
                view,
                overlap_request,
                overlap_request.cast(),
            )
        },
        EvimStatus::InvalidArgument
    );

    context.metrics_generation += 1;
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, &mut state) },
        EvimStatus::Ok
    );
    assert_eq!(state.flags & EVIM_VIEWPORT_STATE_HAS_LAYOUT, 0);
    assert_eq!(state.flags & EVIM_VIEWPORT_STATE_TOP_EXACT, 0);
    assert_eq!(
        state.flags & EVIM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT,
        0,
        "stale width is not advertised as an exact clamp"
    );
    assert_eq!(state.metrics_generation, 2);
    let unavailable_state = state;
    assert_eq!(
        unsafe { evim_core_view_set_viewport_origin(core.handle, view, &failed, &mut outcome) },
        EvimStatus::LayoutUnavailable
    );
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, &mut state) },
        EvimStatus::Ok
    );
    assert_eq!(state, unavailable_state);

    assert_eq!(
        unsafe { evim_core_view_set_wrap(core.handle, view, 1, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, &mut state) },
        EvimStatus::Ok
    );
    assert_eq!(state.left, 0.0);
    assert_ne!(state.flags & EVIM_VIEWPORT_STATE_WRAP, 0);
    assert_ne!(state.flags & EVIM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT, 0);
    assert_eq!(state.maximum_left, 0.0);

    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, ptr::null_mut()) },
        EvimStatus::NullPointer
    );
    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);
}

#[test]
fn provider_key_utf8_pointer_and_enum_failures_are_rejected_without_state_change() {
    let core = create_core(b"text", EvimDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let context_pointer = (&mut *context as *mut FakeProviderContext).cast();
    let options = EvimViewOptionsV1::default();
    let mut view = 99;
    let mut outcome = EvimCoreOutcomeV1::default();

    let mut invalid_provider = provider(context_pointer, fake_shape_batch);
    invalid_provider.struct_size = EVIM_TEXT_MEASUREMENT_PROVIDER_V1_SIZE - 1;
    assert_eq!(
        unsafe {
            evim_core_view_add(
                core.handle,
                &options,
                &invalid_provider,
                &mut view,
                &mut outcome,
            )
        },
        EvimStatus::InvalidProvider
    );
    assert_eq!(view, 0);

    invalid_provider = provider(context_pointer, fake_shape_batch);
    invalid_provider.abi_version = EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION + 1;
    assert_eq!(
        unsafe {
            evim_core_view_add(
                core.handle,
                &options,
                &invalid_provider,
                &mut view,
                &mut outcome,
            )
        },
        EvimStatus::InvalidProvider
    );

    invalid_provider = provider(context_pointer, fake_shape_batch);
    invalid_provider.threading = 99;
    assert_eq!(
        unsafe {
            evim_core_view_add(
                core.handle,
                &options,
                &invalid_provider,
                &mut view,
                &mut outcome,
            )
        },
        EvimStatus::InvalidProvider
    );

    let malformed = provider(context_pointer, malformed_shape_batch);
    assert_eq!(
        unsafe { evim_core_view_add(core.handle, &options, &malformed, &mut view, &mut outcome,) },
        EvimStatus::ProviderFailure
    );
    assert_eq!(view, 0);

    let valid = provider(context_pointer, fake_shape_batch);
    assert_eq!(
        unsafe { evim_core_view_add(core.handle, &options, &valid, &mut view, &mut outcome,) },
        EvimStatus::Ok
    );
    assert_eq!(
        view, 2,
        "failed provider layout burned view 1 without reuse"
    );

    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_ESCAPE, 'x' as u32),
                &mut outcome,
            )
        },
        EvimStatus::InvalidKey
    );
    assert_eq!(outcome.mode, 0);
    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, 0x11_0000),
                &mut outcome,
            )
        },
        EvimStatus::InvalidKey
    );
    assert_eq!(
        unsafe { evim_core_view_send_text(core.handle, view, [0xff].as_ptr(), 1, &mut outcome) },
        EvimStatus::InvalidUtf8
    );
    assert_eq!(
        unsafe { evim_core_view_set_wrap(core.handle, view, 2, &mut outcome) },
        EvimStatus::InvalidArgument
    );
    assert_eq!(
        unsafe { evim_core_view_resize(core.handle, view, f32::NAN, 10.0, &mut outcome) },
        EvimStatus::InvalidArgument
    );
    assert_eq!(
        unsafe { evim_core_view_send_text(core.handle, view, ptr::null(), 1, &mut outcome) },
        EvimStatus::NullPointer
    );
    assert_eq!(
        unsafe { evim_core_view_state(core.handle, view, ptr::null_mut()) },
        EvimStatus::NullPointer
    );
}

#[test]
fn legacy_v1_measurement_provider_remains_accepted() {
    let core = create_core(b"text", EvimDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let context_pointer = (&mut *context as *mut FakeProviderContext).cast();
    let options = EvimViewOptionsV1::default();
    let mut legacy = provider(context_pointer, fake_shape_batch);
    legacy.abi_version = EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V1;
    let mut view = 0;
    let mut outcome = EvimCoreOutcomeV1::default();

    assert_eq!(
        unsafe { evim_core_view_add(core.handle, &options, &legacy, &mut view, &mut outcome,) },
        EvimStatus::Ok
    );
    assert_ne!(view, 0);
    assert!(context.saw_complete_request);
    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);
}

#[test]
fn abi_v2_crossing_cluster_and_unstable_context_contract_are_explicit() {
    let mut source = vec![b'x'; 4095];
    source.extend_from_slice(b"fitail");
    let core = create_core(&source, EvimDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let context_pointer = (&mut *context as *mut FakeProviderContext).cast();
    let options = EvimViewOptionsV1 {
        width: 100_000.0,
        height: 100.0,
        ..EvimViewOptionsV1::default()
    };
    let mut view = 0;
    let mut outcome = EvimCoreOutcomeV1::default();
    let v2 = provider(context_pointer, fake_shape_batch);

    assert_eq!(
        unsafe { evim_core_view_add(core.handle, &options, &v2, &mut view, &mut outcome) },
        EvimStatus::Ok
    );
    assert!(context.saw_crossing_cluster_tail);
    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);

    let declining = provider(context_pointer, unstable_context_shape_batch);
    assert_eq!(
        unsafe { evim_core_view_add(core.handle, &options, &declining, &mut view, &mut outcome,) },
        EvimStatus::UnstableShapingContext
    );
    assert_eq!(view, 0, "a declined batch must not install a view/layout");
}

#[test]
fn core_handles_are_final_and_never_reused() {
    let mut first = create_core(b"one", EvimDocumentOptions::default());
    let first_handle = first.handle;
    assert_eq!(evim_core_destroy(first_handle), EvimStatus::Ok);
    first.handle = 0;
    assert_eq!(evim_core_destroy(first_handle), EvimStatus::InvalidHandle);
    let second = create_core(b"two", EvimDocumentOptions::default());
    assert_ne!(first_handle, second.handle);
    let mut revision = 99;
    assert_eq!(
        unsafe { evim_core_revision(first_handle, &mut revision) },
        EvimStatus::InvalidHandle
    );
    assert_eq!(revision, 0);
}

#[test]
fn busy_destroy_preserves_provider_lifetime_until_successful_retry() {
    let mut core = create_core(b"provider lifetime", EvimDocumentOptions::default());
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
        let mut outcome = EvimCoreOutcomeV1::default();
        let status = unsafe {
            evim_core_view_add(
                first_handle,
                &EvimViewOptionsV1::default(),
                &provider,
                &mut view,
                &mut outcome,
            )
        };
        (status, view)
    });

    gate.wait_until_entered();
    assert_eq!(
        evim_core_destroy(first_handle),
        EvimStatus::CoreBusy,
        "an active provider callback keeps the checked-out core alive"
    );
    assert_eq!(
        evim_core_destroy(first_handle),
        EvimStatus::CoreBusy,
        "a failed destroy must retain the handle for a later retry"
    );

    gate.release();
    let (status, view) = worker.join().unwrap();
    assert_eq!(status, EvimStatus::Ok);
    assert_ne!(view, 0);
    assert!(context.provider.shape_calls > 0);
    assert_eq!(
        context.provider.reentrant_status,
        EvimStatus::CoreBusy as u32
    );

    assert_eq!(evim_core_destroy(first_handle), EvimStatus::Ok);
    core.handle = 0;
    drop(context);
    assert_eq!(evim_core_destroy(first_handle), EvimStatus::InvalidHandle);

    let replacement = create_core(b"replacement", EvimDocumentOptions::default());
    assert_ne!(
        replacement.handle, first_handle,
        "a successfully destroyed core handle must never be reused"
    );
}

#[test]
fn native_composition_commit_is_one_exact_undo_unit() {
    let mut core = create_core(b"", EvimDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let context_pointer = &mut *context as *mut FakeProviderContext;
    let (view, mut outcome) = add_test_view(&core, context_pointer);

    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, 'i' as u32),
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_send_text(core.handle, view, b"a".as_ptr(), 1, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(outcome.document_revision, 1);
    assert_eq!(outcome.cursor_utf8_offset, 1);

    let begin = EvimCompositionBeginV1 {
        struct_size: EVIM_COMPOSITION_BEGIN_V1_SIZE,
        reserved: 0,
        document_revision: 1,
        replacement_start: 1,
        replacement_end: 1,
    };
    assert_eq!(
        unsafe { evim_core_view_composition_begin(core.handle, view, &begin, &mut outcome) },
        EvimStatus::Ok
    );
    assert_ne!(outcome.flags & EVIM_OUTCOME_HAS_COMPOSITION_CHANGES, 0);
    assert_eq!(outcome.document_revision, 1);

    let update_text = "e\u{301}".as_bytes();
    let update = EvimCompositionUpdateV1 {
        struct_size: EVIM_COMPOSITION_UPDATE_V1_SIZE,
        reserved: 0,
        document_revision: 1,
        marked_text: utf8_slice(update_text),
        selected_start: update_text.len() as u64,
        selected_end: update_text.len() as u64,
    };
    assert_eq!(
        unsafe { evim_core_view_composition_update(core.handle, view, &update, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(outcome.document_revision, 1, "marked text is an overlay");

    let committed_text = "é".as_bytes();
    let commit = EvimCompositionCommitV1 {
        struct_size: EVIM_COMPOSITION_COMMIT_V1_SIZE,
        reserved: 0,
        document_revision: 1,
        committed_text: utf8_slice(committed_text),
    };
    assert_eq!(
        unsafe { evim_core_view_composition_commit(core.handle, view, &commit, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(outcome.document_revision, 2);
    assert_eq!(outcome.cursor_utf8_offset, 3);
    assert_ne!(outcome.flags & EVIM_OUTCOME_DOCUMENT_CHANGED, 0);
    assert_ne!(outcome.flags & EVIM_OUTCOME_HAS_POSITION_MAP, 0);
    assert_eq!(
        copy_core_bytes(evim_core_copy_source_bytes, &core, 2),
        "aé".as_bytes()
    );

    assert_eq!(
        unsafe {
            evim_core_view_send_key(core.handle, view, &key(EVIM_KEY_ESCAPE, 0), &mut outcome)
        },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, 'u' as u32),
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(
        copy_core_bytes(
            evim_core_copy_formatted_utf8,
            &core,
            outcome.document_revision,
        ),
        b"a"
    );
    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, 'u' as u32),
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(
        copy_core_bytes(
            evim_core_copy_formatted_utf8,
            &core,
            outcome.document_revision,
        ),
        b""
    );
    core.revision = outcome.document_revision;
    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);
}

#[test]
fn composition_overlay_reads_are_exact_revision_tagged_and_source_nonmutating() {
    let core = create_core(b"hello", EvimDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let context_pointer = &mut *context as *mut FakeProviderContext;
    let (view, mut outcome) = add_test_view(&core, context_pointer);

    let mut inactive = EvimCompositionOverlayInfoV1::default();
    assert_eq!(
        unsafe { evim_core_view_composition_overlay_info(core.handle, view, &mut inactive) },
        EvimStatus::Ok
    );
    assert_eq!(inactive.struct_size, EVIM_COMPOSITION_OVERLAY_INFO_V1_SIZE);
    assert_eq!(inactive.flags & EVIM_COMPOSITION_OVERLAY_ACTIVE, 0);
    assert_eq!(
        inactive.identity.struct_size,
        EVIM_COMPOSITION_OVERLAY_IDENTITY_V1_SIZE
    );

    let begin = EvimCompositionBeginV1 {
        struct_size: EVIM_COMPOSITION_BEGIN_V1_SIZE,
        reserved: 0,
        document_revision: 0,
        replacement_start: 1,
        replacement_end: 4,
    };
    assert_eq!(
        unsafe { evim_core_view_composition_begin(core.handle, view, &begin, &mut outcome) },
        EvimStatus::Ok
    );
    let marked = "é界".as_bytes();
    let update = EvimCompositionUpdateV1 {
        struct_size: EVIM_COMPOSITION_UPDATE_V1_SIZE,
        reserved: 0,
        document_revision: 0,
        marked_text: utf8_slice(marked),
        selected_start: "é".len() as u64,
        selected_end: marked.len() as u64,
    };
    assert_eq!(
        unsafe { evim_core_view_composition_update(core.handle, view, &update, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        copy_core_bytes(evim_core_copy_source_bytes, &core, 0),
        b"hello",
        "queryable marked text remains a disposable projection"
    );

    let mut info = EvimCompositionOverlayInfoV1::default();
    assert_eq!(
        unsafe { evim_core_view_composition_overlay_info(core.handle, view, &mut info) },
        EvimStatus::Ok
    );
    assert_ne!(info.flags & EVIM_COMPOSITION_OVERLAY_ACTIVE, 0);
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

    let request = EvimCompositionOverlayUtf8RangeV1 {
        struct_size: EVIM_COMPOSITION_OVERLAY_UTF8_RANGE_V1_SIZE,
        reserved: 0,
        identity: info.identity,
        start: 0,
        end: info.utf8_length,
    };
    let mut required = u64::MAX;
    assert_eq!(
        unsafe {
            evim_core_view_copy_composition_utf8_range(
                core.handle,
                view,
                &request,
                ptr::null_mut(),
                0,
                &mut required,
            )
        },
        EvimStatus::BufferTooSmall
    );
    assert_eq!(required, info.utf8_length);
    let mut bytes = vec![0; required as usize];
    assert_eq!(
        unsafe {
            evim_core_view_copy_composition_utf8_range(
                core.handle,
                view,
                &request,
                bytes.as_mut_ptr(),
                bytes.len() as u64,
                &mut required,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(bytes, "hé界o".as_bytes());

    let split_scalar = EvimCompositionOverlayUtf8RangeV1 {
        start: 2,
        end: 3,
        ..request
    };
    assert_eq!(
        unsafe {
            evim_core_view_copy_composition_utf8_range(
                core.handle,
                view,
                &split_scalar,
                bytes.as_mut_ptr(),
                bytes.len() as u64,
                &mut required,
            )
        },
        EvimStatus::InvalidUtf8Boundary
    );
    assert_eq!(required, 0, "failed reads clear their byte count first");

    let next_marked = "X".as_bytes();
    let next_update = EvimCompositionUpdateV1 {
        marked_text: utf8_slice(next_marked),
        selected_start: 1,
        selected_end: 1,
        ..update
    };
    assert_eq!(
        unsafe { evim_core_view_composition_update(core.handle, view, &next_update, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe {
            evim_core_view_copy_composition_utf8_range(
                core.handle,
                view,
                &request,
                bytes.as_mut_ptr(),
                bytes.len() as u64,
                &mut required,
            )
        },
        EvimStatus::StaleRevision,
        "a previous marked-text generation must never be read as current"
    );
    assert_eq!(required, 0);

    let cancel = EvimCompositionCancelV1 {
        struct_size: EVIM_COMPOSITION_CANCEL_V1_SIZE,
        reserved: 0,
        document_revision: 0,
    };
    assert_eq!(
        unsafe { evim_core_view_composition_cancel(core.handle, view, &cancel, &mut outcome) },
        EvimStatus::Ok
    );
    let mut cancelled = EvimCompositionOverlayInfoV1::default();
    assert_eq!(
        unsafe { evim_core_view_composition_overlay_info(core.handle, view, &mut cancelled) },
        EvimStatus::Ok
    );
    assert_eq!(cancelled.flags & EVIM_COMPOSITION_OVERLAY_ACTIVE, 0);
    assert_eq!(
        copy_core_bytes(evim_core_copy_source_bytes, &core, 0),
        b"hello"
    );
    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);
}

#[test]
fn composition_cancel_and_invalid_or_stale_updates_are_source_atomic() {
    let mut core = create_core(b"hello", EvimDocumentOptions::default());
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let context_pointer = &mut *context as *mut FakeProviderContext;
    let (view, mut outcome) = add_test_view(&core, context_pointer);
    let begin = EvimCompositionBeginV1 {
        struct_size: EVIM_COMPOSITION_BEGIN_V1_SIZE,
        reserved: 0,
        document_revision: 0,
        replacement_start: 1,
        replacement_end: 4,
    };
    let undersized_begin = EvimCompositionBeginV1 {
        struct_size: EVIM_COMPOSITION_BEGIN_V1_SIZE - 1,
        ..begin
    };
    assert_eq!(
        unsafe {
            evim_core_view_composition_begin(core.handle, view, &undersized_begin, &mut outcome)
        },
        EvimStatus::InvalidArgument
    );
    let inverted_begin = EvimCompositionBeginV1 {
        replacement_start: 4,
        replacement_end: 1,
        ..begin
    };
    assert_eq!(
        unsafe {
            evim_core_view_composition_begin(core.handle, view, &inverted_begin, &mut outcome)
        },
        EvimStatus::InvalidRange
    );
    assert_eq!(
        unsafe { evim_core_view_composition_begin(core.handle, view, ptr::null(), &mut outcome) },
        EvimStatus::NullPointer
    );
    let aliased = &mut outcome as *mut EvimCoreOutcomeV1;
    assert_eq!(
        unsafe { evim_core_view_composition_begin(core.handle, view, aliased.cast(), aliased) },
        EvimStatus::InvalidArgument
    );
    assert_eq!(
        unsafe { evim_core_view_composition_begin(core.handle, view, &begin, &mut outcome) },
        EvimStatus::Ok
    );
    let marked = "é".as_bytes();
    let valid_update = EvimCompositionUpdateV1 {
        struct_size: EVIM_COMPOSITION_UPDATE_V1_SIZE,
        reserved: 0,
        document_revision: 0,
        marked_text: utf8_slice(marked),
        selected_start: 2,
        selected_end: 2,
    };
    assert_eq!(
        unsafe {
            evim_core_view_composition_update(core.handle, view, &valid_update, &mut outcome)
        },
        EvimStatus::Ok
    );

    let split_update = EvimCompositionUpdateV1 {
        selected_start: 1,
        selected_end: 1,
        ..valid_update
    };
    assert_eq!(
        unsafe {
            evim_core_view_composition_update(core.handle, view, &split_update, &mut outcome)
        },
        EvimStatus::NotGraphemeBoundary
    );
    let out_of_range_update = EvimCompositionUpdateV1 {
        selected_start: 3,
        selected_end: 3,
        ..valid_update
    };
    assert_eq!(
        unsafe {
            evim_core_view_composition_update(core.handle, view, &out_of_range_update, &mut outcome)
        },
        EvimStatus::InvalidRange
    );
    let invalid_utf8 = [0xff];
    let malformed_update = EvimCompositionUpdateV1 {
        marked_text: utf8_slice(&invalid_utf8),
        selected_start: 0,
        selected_end: 0,
        ..valid_update
    };
    assert_eq!(
        unsafe {
            evim_core_view_composition_update(core.handle, view, &malformed_update, &mut outcome)
        },
        EvimStatus::InvalidUtf8
    );
    let null_text_update = EvimCompositionUpdateV1 {
        marked_text: EvimUtf8Slice {
            data: ptr::null(),
            length: 1,
        },
        selected_start: 0,
        selected_end: 0,
        ..valid_update
    };
    assert_eq!(
        unsafe {
            evim_core_view_composition_update(core.handle, view, &null_text_update, &mut outcome)
        },
        EvimStatus::NullPointer
    );
    let invalid_commit = EvimCompositionCommitV1 {
        struct_size: EVIM_COMPOSITION_COMMIT_V1_SIZE,
        reserved: 0,
        document_revision: 0,
        committed_text: utf8_slice(&invalid_utf8),
    };
    assert_eq!(
        unsafe {
            evim_core_view_composition_commit(core.handle, view, &invalid_commit, &mut outcome)
        },
        EvimStatus::InvalidUtf8
    );
    assert_eq!(
        copy_core_bytes(evim_core_copy_source_bytes, &core, 0),
        b"hello"
    );
    let cancel = EvimCompositionCancelV1 {
        struct_size: EVIM_COMPOSITION_CANCEL_V1_SIZE,
        reserved: 0,
        document_revision: 0,
    };
    assert_eq!(
        unsafe { evim_core_view_composition_cancel(core.handle, view, &cancel, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(outcome.document_revision, 0);
    assert_eq!(
        copy_core_bytes(evim_core_copy_source_bytes, &core, 0),
        b"hello"
    );

    assert_eq!(
        unsafe { evim_core_view_composition_begin(core.handle, view, &begin, &mut outcome) },
        EvimStatus::Ok
    );
    let (writer, _) = add_test_view(&core, context_pointer);
    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                writer,
                &key(EVIM_KEY_CHARACTER, 'i' as u32),
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_send_text(core.handle, writer, b"X".as_ptr(), 1, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(outcome.document_revision, 1);
    let source_after_other_view_edit = copy_core_bytes(
        evim_core_copy_source_bytes,
        &core,
        outcome.document_revision,
    );
    assert_eq!(source_after_other_view_edit, b"Xhello");
    assert_eq!(
        unsafe {
            evim_core_view_composition_update(core.handle, view, &valid_update, &mut outcome)
        },
        EvimStatus::StaleRevision
    );
    assert_eq!(
        copy_core_bytes(evim_core_copy_source_bytes, &core, 1),
        source_after_other_view_edit
    );
    core.revision = 1;
    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);
    assert_eq!(evim_core_view_remove(core.handle, writer), EvimStatus::Ok);
}

#[test]
fn latin1_unrepresentable_composition_commit_is_typed_and_non_destructive() {
    let core = create_core(
        b"caf\xe9",
        EvimDocumentOptions {
            encoding: EVIM_ENCODING_LATIN1,
            ..EvimDocumentOptions::default()
        },
    );
    let mut context = Box::new(FakeProviderContext::new(core.handle));
    let context_pointer = &mut *context as *mut FakeProviderContext;
    let (view, mut outcome) = add_test_view(&core, context_pointer);
    let begin = EvimCompositionBeginV1 {
        struct_size: EVIM_COMPOSITION_BEGIN_V1_SIZE,
        reserved: 0,
        document_revision: 0,
        replacement_start: "café".len() as u64,
        replacement_end: "café".len() as u64,
    };
    assert_eq!(
        unsafe { evim_core_view_composition_begin(core.handle, view, &begin, &mut outcome) },
        EvimStatus::Ok
    );
    let emoji = "😀".as_bytes();
    let commit = EvimCompositionCommitV1 {
        struct_size: EVIM_COMPOSITION_COMMIT_V1_SIZE,
        reserved: 0,
        document_revision: 0,
        committed_text: utf8_slice(emoji),
    };
    assert_eq!(
        unsafe { evim_core_view_composition_commit(core.handle, view, &commit, &mut outcome) },
        EvimStatus::UnrepresentableCharacter
    );
    assert_eq!(
        copy_core_bytes(evim_core_copy_source_bytes, &core, 0),
        b"caf\xe9"
    );
    assert_eq!(
        copy_core_bytes(evim_core_copy_formatted_utf8, &core, 0),
        "café".as_bytes()
    );
    let cancel = EvimCompositionCancelV1 {
        struct_size: EVIM_COMPOSITION_CANCEL_V1_SIZE,
        reserved: 0,
        document_revision: 0,
    };
    assert_eq!(
        unsafe { evim_core_view_composition_cancel(core.handle, view, &cancel, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(evim_core_view_remove(core.handle, view), EvimStatus::Ok);
}

#[test]
fn public_c_header_typechecks_every_v3_layout_against_rust() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let unique = format!(
        "evim_ffi_header_{}_{}.c",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    );
    let source_path = std::env::temp_dir().join(unique);
    let source = format!(
        r#"
#include "evim_core.h"
#include <stddef.h>
_Static_assert(EVIM_CORE_ABI_VERSION == {abi}, "ABI version");
_Static_assert(sizeof(EvimDirectStyleEditV1) == {direct_style_edit}, "direct style size");
_Static_assert(_Alignof(EvimDirectStyleEditV1) == {direct_style_align}, "direct style alignment");
_Static_assert(offsetof(EvimDirectStyleEditV1, expected_selection) == {direct_style_selection}, "direct style selection offset");
_Static_assert(offsetof(EvimDirectStyleEditV1, value) == {direct_style_value}, "direct style value offset");
static EvimStatus (*direct_style)(EvimCoreHandle, EvimViewId, const EvimDirectStyleEditV1 *, EvimCoreOutcomeV1 *) = evim_core_view_edit_direct_style;
static EvimStatus (*decoration_state)(EvimCoreHandle, EvimViewId, uint32_t, uint32_t *) = evim_core_view_decoration_state;
_Static_assert(EVIM_ENCODING_DETECT == 0u, "automatic encoding choice");
_Static_assert(EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V1 == 1u,
    "provider ABI v1");
_Static_assert(EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V2 == 2u,
    "provider ABI v2");
_Static_assert(EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION ==
    EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V2, "current provider ABI");
_Static_assert(EVIM_STATUS_UNSTABLE_SHAPING_CONTEXT == 26u,
    "bounded-context refusal status");
_Static_assert(EVIM_STATUS_VERTICAL_VIEWPORT_ORIGIN_UNSUPPORTED == 27u,
    "vertical viewport limitation status");
_Static_assert(EVIM_STATUS_LAYOUT_UNAVAILABLE == 28u,
    "layout unavailable status");
_Static_assert(EVIM_STATUS_OUTSIDE_LAYOUT_COVERAGE == 29u,
    "outside layout coverage status");
_Static_assert(EVIM_STATUS_INVALID_UTF8_BOUNDARY == 36u,
    "invalid UTF-8 boundary status");
_Static_assert(EVIM_STATUS_INVALID_UTF16_BOUNDARY == 37u,
    "invalid UTF-16 boundary status");
_Static_assert(EVIM_STATUS_STYLE_EDIT_GROUP_ACTIVE == 38u,
    "active style edit group status");
_Static_assert(EVIM_STATUS_INVALID_STYLE_EDIT_GROUP == 39u,
    "invalid style edit group status");
_Static_assert(EVIM_STATUS_STYLE_EDIT_GROUP_WRONG_OWNER == 40u,
    "style edit group owner status");
_Static_assert(EVIM_VIEWPORT_ORIGIN_HAS_TOP == (1u << 0),
    "viewport top request flag");
_Static_assert(EVIM_VIEWPORT_STATE_WRAP == (1u << 0), "viewport wrap flag");
_Static_assert(EVIM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT == (1u << 1),
    "viewport maximum-left exact flag");
_Static_assert(EVIM_VIEWPORT_STATE_TOP_EXACT == (1u << 2),
    "viewport top exact flag");
_Static_assert(EVIM_VIEWPORT_STATE_HAS_LAYOUT == (1u << 3),
    "viewport layout identity flag");
_Static_assert(EVIM_VIEWPORT_STATE_LINEBREAK == (1u << 4),
    "viewport linebreak flag");
_Static_assert(EVIM_FILE_FORMAT_ORIGIN_DETECTED == 1u,
    "detected file-format origin");
_Static_assert(EVIM_FILE_FORMAT_ORIGIN_FORCED == 2u,
    "forced file-format origin");
_Static_assert(EVIM_FILE_FORMAT_ORIGIN_DEFAULTED == 3u,
    "defaulted file-format origin");
_Static_assert(EVIM_HISTORY_ACTION_CATEGORY_NONE == 0u,
    "no history action category");
_Static_assert(EVIM_HISTORY_ACTION_CATEGORY_TEXT == 1u,
    "text history action category");
_Static_assert(EVIM_HISTORY_ACTION_CATEGORY_STYLE == 2u,
    "style history action category");
_Static_assert(EVIM_HISTORY_ACTION_CATEGORY_FILE_FORMAT == 3u,
    "file-format history action category");
_Static_assert(EVIM_HISTORY_ACTION_CATEGORY_HARD_LINE_TRANSFER == 4u,
    "hard-line transfer history action category");
_Static_assert(EVIM_HISTORY_ACTION_CATEGORY_HARD_LINE_SOURCE_RESTORATION == 5u,
    "hard-line restoration history action category");
_Static_assert(EVIM_HISTORY_ACTION_CATEGORY_SOURCE_METADATA == 6u,
    "source-metadata history action category");
_Static_assert(EVIM_HISTORY_ACTION_CATEGORY_MIXED == 7u,
    "mixed history action category");
_Static_assert(EVIM_DOCUMENT_STATE_HAS_BOM == (1u << 0),
    "document BOM flag");
_Static_assert(EVIM_DOCUMENT_STATE_CAN_UNDO == (1u << 1),
    "document can-undo flag");
_Static_assert(EVIM_DOCUMENT_STATE_CAN_REDO == (1u << 2),
    "document can-redo flag");
_Static_assert(EVIM_DOCUMENT_STATE_IS_DIRTY == (1u << 3),
    "document dirty flag");
_Static_assert(EVIM_CLIPBOARD_TARGET_CLIPBOARD == 1u,
    "clipboard target");
_Static_assert(EVIM_CLIPBOARD_TARGET_PRIMARY == 2u,
    "primary target");
_Static_assert(EVIM_CLIPBOARD_TURN_HAS_READ == (1u << 0),
    "clipboard read flag");
_Static_assert(EVIM_CLIPBOARD_TURN_WRITABLE == (1u << 1),
    "clipboard writable flag");
_Static_assert(EVIM_EFFECT_BATCH_HAS_EX_OUTCOME == (1u << 0),
    "effect Ex outcome flag");
_Static_assert(EVIM_EFFECT_BATCH_EX_DOCUMENT_CHANGED == (1u << 1),
    "effect document-changed flag");
_Static_assert(EVIM_EFFECT_BATCH_EX_HAS_NAVIGATION == (1u << 2),
    "effect navigation flag");
_Static_assert(EVIM_EFFECT_BATCH_EX_NAVIGATION_HISTORY == (1u << 3),
    "effect history-navigation flag");
_Static_assert(EVIM_CLIPBOARD_WRITE_HAS_PORTABLE_REGISTER == (1u << 0),
    "portable clipboard flag");
_Static_assert(EVIM_REGISTER_KIND_NONE == 0u, "no register shape");
_Static_assert(EVIM_REGISTER_KIND_CHARACTER == 1u, "character register");
_Static_assert(EVIM_REGISTER_KIND_LINE == 2u, "line register");
_Static_assert(EVIM_REGISTER_KIND_BLOCK == 3u, "block register");
_Static_assert(EVIM_EX_FRONTEND_EDIT == 1u, "Ex edit request");
_Static_assert(EVIM_EX_FRONTEND_NORMAL == 15u, "Ex normal request");
_Static_assert(EVIM_EX_FRONTEND_HAS_PATH == (1u << 1), "Ex path flag");
_Static_assert(EVIM_EX_FRONTEND_HAS_RANGE == (1u << 2), "Ex range flag");
_Static_assert(EVIM_EX_OPTION_WRAP == 1u, "Ex wrap option");
_Static_assert(EVIM_EX_OPTION_FILE_FORMATS == 4u, "Ex fileformats option");
_Static_assert(EVIM_EX_OPTION_VALUE_BOOLEAN == 1u, "Ex Boolean value");
_Static_assert(EVIM_EX_OPTION_VALUE_FILE_FORMATS == 3u,
    "Ex fileformats value");
_Static_assert(EVIM_EX_JUMP_CURRENT == (1u << 0), "current Ex jump");
_Static_assert(EVIM_TEXT_PAINT_HAS_BACKGROUND == (1u << 0),
    "paint background flag");
_Static_assert(EVIM_TEXT_PAINT_UNDERLINE == (1u << 1),
    "paint underline flag");
_Static_assert(EVIM_TEXT_PAINT_STRIKETHROUGH == (1u << 2),
    "paint strikethrough flag");
_Static_assert(EVIM_COMMAND_LINE_KIND_NONE == 0u, "no command line kind");
_Static_assert(EVIM_COMMAND_LINE_KIND_EX == 1u, "Ex command line kind");
_Static_assert(EVIM_COMMAND_LINE_KIND_SEARCH_FORWARD == 2u,
    "forward-search command line kind");
_Static_assert(EVIM_COMMAND_LINE_KIND_SEARCH_BACKWARD == 3u,
    "backward-search command line kind");
_Static_assert(EVIM_VISUAL_SELECTION_KIND_NONE == 0u,
    "no Visual selection kind");
_Static_assert(EVIM_VISUAL_SELECTION_KIND_CHARACTER == 1u,
    "Characterwise Visual selection kind");
_Static_assert(EVIM_VISUAL_SELECTION_KIND_LINE == 2u,
    "Linewise Visual selection kind");
_Static_assert(EVIM_VISUAL_SELECTION_KIND_BLOCK == 3u,
    "Blockwise Visual selection kind");
_Static_assert(EVIM_VISUAL_SELECTION_SEGMENT_HAS_VISUAL_ROW == (1u << 0),
    "Visual segment row flag");
_Static_assert(EVIM_VISUAL_SELECTION_SEGMENT_HAS_HARD_LINE == (1u << 1),
    "Visual segment hard-line flag");
_Static_assert(EVIM_VISUAL_SELECTION_SEGMENT_HAS_EDGE_AFFINITIES == (1u << 2),
    "Visual segment affinity flag");
_Static_assert(sizeof(EvimDocumentOptions) == {document_options}, "document options");
_Static_assert(sizeof(EvimDocumentStateV1) == {document_state}, "document state");
_Static_assert(EVIM_DOCUMENT_STATE_V1_SIZE == sizeof(EvimDocumentStateV1),
    "document state size macro");
_Static_assert(sizeof(EvimFormattedSnapshotIdentityV1) == {formatted_identity},
    "formatted snapshot identity");
_Static_assert(EVIM_FORMATTED_SNAPSHOT_IDENTITY_V1_SIZE ==
    sizeof(EvimFormattedSnapshotIdentityV1),
    "formatted snapshot identity size macro");
_Static_assert(offsetof(EvimFormattedSnapshotIdentityV1, document_id) ==
    {formatted_identity_document}, "formatted identity document offset");
_Static_assert(sizeof(EvimFormattedSnapshotInfoV1) == {formatted_info},
    "formatted snapshot info");
_Static_assert(EVIM_FORMATTED_SNAPSHOT_INFO_V1_SIZE ==
    sizeof(EvimFormattedSnapshotInfoV1), "formatted snapshot info size macro");
_Static_assert(offsetof(EvimFormattedSnapshotInfoV1, utf8_length) ==
    {formatted_info_utf8}, "formatted info UTF-8 length offset");
_Static_assert(sizeof(EvimFormattedUtf8RangeV1) == {formatted_range},
    "formatted UTF-8 range");
_Static_assert(EVIM_FORMATTED_UTF8_RANGE_V1_SIZE ==
    sizeof(EvimFormattedUtf8RangeV1), "formatted UTF-8 range size macro");
_Static_assert(offsetof(EvimFormattedUtf8RangeV1, utf8_start) ==
    {formatted_range_start}, "formatted range start offset");
_Static_assert(sizeof(EvimFormattedPointInfoV1) == {formatted_point},
    "formatted point info");
_Static_assert(EVIM_FORMATTED_POINT_INFO_V1_SIZE ==
    sizeof(EvimFormattedPointInfoV1), "formatted point info size macro");
_Static_assert(offsetof(EvimFormattedPointInfoV1, grapheme_column) ==
    {formatted_point_column}, "formatted point column offset");
_Static_assert(sizeof(EvimClipboardTurnEntryV1) == {clipboard_turn_entry},
    "clipboard turn entry");
_Static_assert(EVIM_CLIPBOARD_TURN_ENTRY_V1_SIZE ==
    sizeof(EvimClipboardTurnEntryV1), "clipboard turn entry size macro");
_Static_assert(offsetof(EvimClipboardTurnEntryV1, plain_text) ==
    {clipboard_turn_text}, "clipboard turn text offset");
_Static_assert(sizeof(EvimCommandTurnContextV1) == {command_turn_context},
    "command turn context");
_Static_assert(EVIM_COMMAND_TURN_CONTEXT_V1_SIZE ==
    sizeof(EvimCommandTurnContextV1), "command turn context size macro");
_Static_assert(sizeof(EvimEffectBytesRefV1) == {effect_bytes_ref},
    "effect byte reference");
_Static_assert(sizeof(EvimClipboardWriteV1) == {clipboard_write},
    "clipboard write");
_Static_assert(EVIM_CLIPBOARD_WRITE_V1_SIZE == sizeof(EvimClipboardWriteV1),
    "clipboard write size macro");
_Static_assert(sizeof(EvimExOptionDisplayV1) == {ex_option}, "Ex option");
_Static_assert(EVIM_EX_OPTION_DISPLAY_V1_SIZE == sizeof(EvimExOptionDisplayV1),
    "Ex option size macro");
_Static_assert(sizeof(EvimExMarkV1) == {ex_mark}, "Ex mark");
_Static_assert(EVIM_EX_MARK_V1_SIZE == sizeof(EvimExMarkV1),
    "Ex mark size macro");
_Static_assert(sizeof(EvimExRegisterV1) == {ex_register}, "Ex register");
_Static_assert(EVIM_EX_REGISTER_V1_SIZE == sizeof(EvimExRegisterV1),
    "Ex register size macro");
_Static_assert(sizeof(EvimExJumpV1) == {ex_jump}, "Ex jump");
_Static_assert(EVIM_EX_JUMP_V1_SIZE == sizeof(EvimExJumpV1),
    "Ex jump size macro");
_Static_assert(sizeof(EvimExTextLineV1) == {ex_text_line}, "Ex text line");
_Static_assert(EVIM_EX_TEXT_LINE_V1_SIZE == sizeof(EvimExTextLineV1),
    "Ex text-line size macro");
_Static_assert(sizeof(EvimExFrontendRequestV1) == {ex_request},
    "Ex frontend request");
_Static_assert(EVIM_EX_FRONTEND_REQUEST_V1_SIZE ==
    sizeof(EvimExFrontendRequestV1), "Ex frontend request size macro");
_Static_assert(offsetof(EvimExFrontendRequestV1, first_payload) ==
    {ex_request_payload}, "Ex request payload offset");
_Static_assert(sizeof(EvimEffectBatchInfoV1) == {effect_batch_info},
    "effect batch info");
_Static_assert(EVIM_EFFECT_BATCH_INFO_V1_SIZE == sizeof(EvimEffectBatchInfoV1),
    "effect batch info size macro");
_Static_assert(offsetof(EvimEffectBatchInfoV1, ex_mark_count) ==
    {effect_batch_mark_count}, "effect batch mark count offset");
_Static_assert(sizeof(EvimResolvedTextStyleV1) == {style}, "style");
_Static_assert(sizeof(EvimShapeStyleRunV1) == {style_run}, "style run");
_Static_assert(sizeof(EvimShapedClusterV1) == {cluster}, "cluster");
_Static_assert(sizeof(EvimShapingDiagnosticV1) == {diagnostic}, "diagnostic");
_Static_assert(sizeof(EvimShapeRequestV1) == {request}, "request");
_Static_assert(sizeof(EvimShapeResponseV1) == {response}, "response");
_Static_assert(sizeof(EvimTextMeasurementProviderV1) == {provider}, "provider");
_Static_assert(sizeof(EvimViewOptionsV1) == {view_options}, "view options");
_Static_assert(sizeof(EvimViewportOriginV1) == {viewport_origin}, "viewport origin");
_Static_assert(sizeof(EvimViewportStateV1) == {viewport_state}, "viewport state");
_Static_assert(sizeof(EvimLayoutSnapshotIdentityV1) == {layout_identity},
    "layout identity");
_Static_assert(sizeof(EvimLayoutInsetsV1) == {layout_insets}, "layout insets");
_Static_assert(sizeof(EvimLayoutRectV1) == {layout_rect}, "layout rect");
_Static_assert(sizeof(EvimRgbaV1) == {rgba}, "RGBA");
_Static_assert(sizeof(EvimStyleSheetIdentityV1) == {style_sheet_identity},
    "style-sheet identity");
_Static_assert(EVIM_STYLE_SHEET_IDENTITY_V1_SIZE == sizeof(EvimStyleSheetIdentityV1),
    "style-sheet identity size macro");
_Static_assert(sizeof(EvimStyleStringRefV1) == {style_string_ref},
    "style string reference");
_Static_assert(sizeof(EvimStyleSheetInfoV1) == {style_sheet_info},
    "style-sheet info");
_Static_assert(EVIM_STYLE_SHEET_INFO_V1_SIZE == sizeof(EvimStyleSheetInfoV1),
    "style-sheet info size macro");
_Static_assert(sizeof(EvimStyleValueV1) == {style_value}, "style value");
_Static_assert(EVIM_STYLE_VALUE_V1_SIZE == sizeof(EvimStyleValueV1),
    "style value size macro");
_Static_assert(sizeof(EvimStyleValueItemV1) == {style_value_item},
    "style value item");
_Static_assert(sizeof(EvimStyleDependencyV1) == {style_dependency},
    "style dependency");
_Static_assert(sizeof(EvimStyleDefinitionV1) == {style_definition},
    "style definition");
_Static_assert(sizeof(EvimStylePropertyV1) == {style_property},
    "style property");
_Static_assert(sizeof(EvimStyleEditValueItemV1) == {style_edit_value_item},
    "style edit value item");
_Static_assert(sizeof(EvimStyleEditValueV1) == {style_edit_value},
    "style edit value");
_Static_assert(sizeof(EvimStyleEditV1) == {style_edit}, "style edit");
_Static_assert(EVIM_STYLE_EDIT_V1_SIZE == sizeof(EvimStyleEditV1),
    "style edit size macro");
_Static_assert(sizeof(EvimStyleEditGroupV1) == {style_edit_group},
    "style edit group");
_Static_assert(EVIM_STYLE_EDIT_GROUP_V1_SIZE == sizeof(EvimStyleEditGroupV1),
    "style edit group size macro");
_Static_assert(offsetof(EvimStyleEditGroupV1, token) == {style_edit_group_token},
    "style edit group token offset");
_Static_assert(EVIM_STYLE_EDIT_SET_DISPLAY_NAME == 7u,
    "style display-name operation");
_Static_assert(EVIM_STYLE_CAPABILITY_EDIT_DISPLAY_NAME == (1u << 3),
    "style display-name capability");
_Static_assert(sizeof(EvimTextPaintV1) == {text_paint}, "text paint");
_Static_assert(EVIM_TEXT_PAINT_V1_SIZE == sizeof(EvimTextPaintV1),
    "text paint size macro");
_Static_assert(sizeof(EvimLayoutPaintInfoV1) == {layout_paint_info},
    "layout paint info");
_Static_assert(EVIM_LAYOUT_PAINT_INFO_V1_SIZE == sizeof(EvimLayoutPaintInfoV1),
    "layout paint info size macro");
_Static_assert(sizeof(EvimPaintStyleRunV1) == {paint_style_run},
    "paint style run");
_Static_assert(EVIM_PAINT_STYLE_RUN_V1_SIZE == sizeof(EvimPaintStyleRunV1),
    "paint style run size macro");
_Static_assert(sizeof(EvimLayoutSnapshotInfoV1) == {layout_info}, "layout info");
_Static_assert(sizeof(EvimVisualRowV1) == {visual_row}, "visual row");
_Static_assert(sizeof(EvimPositionedClusterV1) == {positioned_cluster},
    "positioned cluster");
_Static_assert(sizeof(EvimPositionedCaretV1) == {positioned_caret},
    "positioned caret");
_Static_assert(sizeof(EvimLayoutCaretRequestV1) == {caret_request},
    "caret request");
_Static_assert(sizeof(EvimLayoutHitTestRequestV1) == {hit_test_request},
    "hit-test request");
_Static_assert(sizeof(EvimLayoutCaretPointV1) == {caret_point}, "caret point");
_Static_assert(sizeof(EvimLayoutCaretGeometryV1) == {caret_geometry},
    "caret geometry");
_Static_assert(sizeof(EvimViewPresentationV1) == {presentation},
    "view presentation");
_Static_assert(sizeof(EvimCommandLineIdentityV1) == {command_line_identity},
    "command-line identity");
_Static_assert(EVIM_COMMAND_LINE_IDENTITY_V1_SIZE == sizeof(EvimCommandLineIdentityV1),
    "command-line identity size macro");
_Static_assert(sizeof(EvimCommandLineInfoV1) == {command_line_info},
    "command-line info");
_Static_assert(EVIM_COMMAND_LINE_INFO_V1_SIZE == sizeof(EvimCommandLineInfoV1),
    "command-line info size macro");
_Static_assert(sizeof(EvimVisualSelectionIdentityV1) == {visual_selection_identity},
    "Visual-selection identity");
_Static_assert(EVIM_VISUAL_SELECTION_IDENTITY_V1_SIZE == sizeof(EvimVisualSelectionIdentityV1),
    "Visual-selection identity size macro");
_Static_assert(sizeof(EvimVisualSelectionInfoV1) == {visual_selection_info},
    "Visual-selection info");
_Static_assert(EVIM_VISUAL_SELECTION_INFO_V1_SIZE == sizeof(EvimVisualSelectionInfoV1),
    "Visual-selection info size macro");
_Static_assert(sizeof(EvimVisualSelectionSegmentV1) == {visual_selection_segment},
    "Visual-selection segment");
_Static_assert(EVIM_VISUAL_SELECTION_SEGMENT_V1_SIZE == sizeof(EvimVisualSelectionSegmentV1),
    "Visual-selection segment size macro");
_Static_assert(sizeof(EvimVisualSelectionRectangleV1) == {visual_selection_rectangle},
    "Visual-selection rectangle");
_Static_assert(EVIM_VISUAL_SELECTION_RECTANGLE_V1_SIZE == sizeof(EvimVisualSelectionRectangleV1),
    "Visual-selection rectangle size macro");
_Static_assert(sizeof(EvimPlaceCursorV1) == {place_cursor}, "place cursor");
_Static_assert(sizeof(EvimAssignStyleV1) == {assign_style}, "assign style");
_Static_assert(EVIM_ASSIGN_STYLE_V1_SIZE == sizeof(EvimAssignStyleV1), "assign style size macro");
_Static_assert(sizeof(EvimCreateStyleV1) == {create_style}, "create style");
_Static_assert(EVIM_CREATE_STYLE_V1_SIZE == sizeof(EvimCreateStyleV1), "create style size macro");
_Static_assert(sizeof(EvimDeleteStyleV1) == {delete_style}, "delete style");
_Static_assert(EVIM_DELETE_STYLE_V1_SIZE == sizeof(EvimDeleteStyleV1), "delete style size macro");
_Static_assert(sizeof(EvimSetFileFormatV1) == {set_file_format},
    "set file format");
_Static_assert(EVIM_SET_FILE_FORMAT_V1_SIZE == sizeof(EvimSetFileFormatV1),
    "set file format size macro");
_Static_assert(sizeof(EvimMarkSavedV1) == {mark_saved}, "mark saved");
_Static_assert(EVIM_MARK_SAVED_V1_SIZE == sizeof(EvimMarkSavedV1),
    "mark saved size macro");
_Static_assert(sizeof(EvimKeyInputV1) == {key}, "key");
_Static_assert(sizeof(EvimCompositionBeginV1) == {composition_begin}, "composition begin");
_Static_assert(sizeof(EvimCompositionUpdateV1) == {composition_update}, "composition update");
_Static_assert(sizeof(EvimCompositionCommitV1) == {composition_commit}, "composition commit");
_Static_assert(sizeof(EvimCompositionCancelV1) == {composition_cancel}, "composition cancel");
_Static_assert(sizeof(EvimCompositionOverlayIdentityV1) == {composition_overlay_identity},
    "composition overlay identity");
_Static_assert(EVIM_COMPOSITION_OVERLAY_IDENTITY_V1_SIZE ==
    sizeof(EvimCompositionOverlayIdentityV1), "composition overlay identity size macro");
_Static_assert(sizeof(EvimCompositionOverlayInfoV1) == {composition_overlay_info},
    "composition overlay info");
_Static_assert(EVIM_COMPOSITION_OVERLAY_INFO_V1_SIZE ==
    sizeof(EvimCompositionOverlayInfoV1), "composition overlay info size macro");
_Static_assert(sizeof(EvimCompositionOverlayUtf8RangeV1) == {composition_overlay_range},
    "composition overlay range");
_Static_assert(EVIM_COMPOSITION_OVERLAY_UTF8_RANGE_V1_SIZE ==
    sizeof(EvimCompositionOverlayUtf8RangeV1), "composition overlay range size macro");
_Static_assert(sizeof(EvimCoreOutcomeV1) == {outcome}, "outcome");
static void typecheck(void) {{
  EvimShapeRequestV1 request = {{0}};
  request.reserved = 0;
  request.paragraph_base_direction = EVIM_TEXT_DIRECTION_AUTO;
  EvimStatus (*create_core)(const uint8_t *, uint64_t,
      const EvimDocumentOptions *, EvimCoreHandle *, uint64_t *) = evim_core_create;
  EvimStatus (*document_state)(EvimCoreHandle, EvimDocumentStateV1 *) =
      evim_core_document_state;
  EvimStatus (*formatted_info)(EvimCoreHandle,
      EvimFormattedSnapshotInfoV1 *) = evim_core_formatted_snapshot_info;
  EvimStatus (*copy_formatted_range)(EvimCoreHandle,
      const EvimFormattedUtf8RangeV1 *, uint8_t *, uint64_t, uint64_t *) =
      evim_core_copy_formatted_utf8_range;
  EvimStatus (*map_utf8_to_utf16)(EvimCoreHandle,
      const EvimFormattedSnapshotIdentityV1 *, const uint64_t *, uint64_t,
      uint64_t *, uint64_t, uint64_t *) =
      evim_core_map_formatted_utf8_to_utf16;
  EvimStatus (*map_utf16_to_utf8)(EvimCoreHandle,
      const EvimFormattedSnapshotIdentityV1 *, const uint64_t *, uint64_t,
      uint64_t *, uint64_t, uint64_t *) =
      evim_core_map_formatted_utf16_to_utf8;
  EvimStatus (*formatted_point)(EvimCoreHandle,
      const EvimFormattedSnapshotIdentityV1 *, uint64_t,
      EvimFormattedPointInfoV1 *) = evim_core_formatted_point_info;
  EvimStatus (*mark_saved)(EvimCoreHandle, const EvimMarkSavedV1 *) =
      evim_core_mark_saved;
  EvimStatus (*style_sheet_info)(EvimCoreHandle, EvimStyleSheetInfoV1 *) =
      evim_core_style_sheet_info;
  EvimStatus (*copy_style_sheet)(EvimCoreHandle,
      const EvimStyleSheetIdentityV1 *, EvimStyleDefinitionV1 *, uint64_t,
      EvimStylePropertyV1 *, uint64_t, EvimStyleValueItemV1 *, uint64_t,
      EvimStyleDependencyV1 *, uint64_t, uint8_t *, uint64_t,
      EvimStyleSheetInfoV1 *) = evim_core_copy_style_sheet;
  EvimStatus (*add_view)(EvimCoreHandle, const EvimViewOptionsV1 *,
      const EvimTextMeasurementProviderV1 *, EvimViewId *,
      EvimCoreOutcomeV1 *) = evim_core_view_add;
  EvimStatus (*viewport_state)(EvimCoreHandle, EvimViewId,
      EvimViewportStateV1 *) = evim_core_view_viewport_state;
  EvimStatus (*layout_info)(EvimCoreHandle, EvimViewId,
      EvimLayoutSnapshotInfoV1 *) = evim_core_view_layout_snapshot_info;
  EvimStatus (*copy_layout)(EvimCoreHandle, EvimViewId,
      const EvimLayoutSnapshotIdentityV1 *, EvimVisualRowV1 *, uint64_t,
      EvimPositionedClusterV1 *, uint64_t, EvimPositionedCaretV1 *, uint64_t,
      EvimLayoutSnapshotInfoV1 *) = evim_core_view_copy_layout_snapshot;
  EvimStatus (*layout_paint_info)(EvimCoreHandle, EvimViewId,
      EvimLayoutPaintInfoV1 *) = evim_core_view_layout_paint_info;
  EvimStatus (*copy_layout_paint)(EvimCoreHandle, EvimViewId,
      const EvimLayoutSnapshotIdentityV1 *, EvimPaintStyleRunV1 *, uint64_t,
      EvimLayoutPaintInfoV1 *) = evim_core_view_copy_layout_paint;
  EvimStatus (*caret_geometry)(EvimCoreHandle, EvimViewId,
      const EvimLayoutCaretRequestV1 *, EvimLayoutCaretGeometryV1 *) =
      evim_core_view_caret_geometry;
  EvimStatus (*hit_test)(EvimCoreHandle, EvimViewId,
      const EvimLayoutHitTestRequestV1 *, EvimLayoutCaretPointV1 *) =
      evim_core_view_layout_hit_test;
  EvimStatus (*presentation)(EvimCoreHandle, EvimViewId,
      EvimViewPresentationV1 *) = evim_core_view_presentation;
  EvimStatus (*command_line_info)(EvimCoreHandle, EvimViewId,
      EvimCommandLineInfoV1 *) = evim_core_view_command_line_info;
  EvimStatus (*copy_command_line)(EvimCoreHandle, EvimViewId,
      const EvimCommandLineIdentityV1 *, uint8_t *, uint64_t,
      EvimCommandLineInfoV1 *) = evim_core_view_copy_command_line;
  EvimStatus (*visual_selection_info)(EvimCoreHandle, EvimViewId,
      EvimVisualSelectionInfoV1 *) = evim_core_view_visual_selection_info;
  EvimStatus (*copy_visual_selection)(EvimCoreHandle, EvimViewId,
      const EvimVisualSelectionIdentityV1 *,
      EvimVisualSelectionSegmentV1 *, uint64_t,
      EvimVisualSelectionRectangleV1 *, uint64_t,
      EvimVisualSelectionInfoV1 *) = evim_core_view_copy_visual_selection;
  EvimStatus (*use_selection_for_find)(EvimCoreHandle, EvimViewId,
      const EvimVisualSelectionIdentityV1 *, EvimCoreOutcomeV1 *) =
      evim_core_view_use_selection_for_find;
  EvimStatus (*reveal_selection)(EvimCoreHandle, EvimViewId,
      EvimCoreOutcomeV1 *) = evim_core_view_reveal_selection;
  EvimStatus (*set_viewport_origin)(EvimCoreHandle, EvimViewId,
      const EvimViewportOriginV1 *, EvimCoreOutcomeV1 *) =
      evim_core_view_set_viewport_origin;
  EvimStatus (*set_scale)(EvimCoreHandle, EvimViewId, float,
      EvimCoreOutcomeV1 *) = evim_core_view_set_scale;
  EvimStatus (*set_linebreak)(EvimCoreHandle, EvimViewId, uint32_t,
      EvimCoreOutcomeV1 *) = evim_core_view_set_linebreak;
  EvimStatus (*set_file_format)(EvimCoreHandle, EvimViewId,
      const EvimSetFileFormatV1 *, EvimCoreOutcomeV1 *) =
      evim_core_view_set_file_format;
  EvimStatus (*edit_style)(EvimCoreHandle, EvimViewId,
      const EvimStyleEditV1 *, EvimCoreOutcomeV1 *) =
      evim_core_view_edit_style;
  EvimStatus (*assign_style)(EvimCoreHandle, EvimViewId,
      const EvimAssignStyleV1 *, EvimCoreOutcomeV1 *) = evim_core_view_assign_style;
  EvimStatus (*create_style)(EvimCoreHandle, EvimViewId,
      const EvimCreateStyleV1 *, EvimCoreOutcomeV1 *) = evim_core_view_create_style;
  EvimStatus (*delete_style)(EvimCoreHandle, EvimViewId,
      const EvimDeleteStyleV1 *, EvimCoreOutcomeV1 *) = evim_core_view_delete_style;
  EvimStatus (*begin_style_group)(EvimCoreHandle, EvimViewId,
      const EvimStyleSheetIdentityV1 *, EvimStyleEditGroupV1 *) =
      evim_core_view_begin_style_edit_group;
  EvimStatus (*edit_style_in_group)(EvimCoreHandle, EvimViewId,
      const EvimStyleEditGroupV1 *, const EvimStyleEditV1 *,
      EvimCoreOutcomeV1 *) = evim_core_view_edit_style_in_group;
  EvimStatus (*end_style_group)(EvimCoreHandle, EvimViewId,
      const EvimStyleEditGroupV1 *) = evim_core_view_end_style_edit_group;
  EvimStatus (*send_key)(EvimCoreHandle, EvimViewId,
      const EvimKeyInputV1 *, EvimCoreOutcomeV1 *) = evim_core_view_send_key;
  EvimStatus (*send_text)(EvimCoreHandle, EvimViewId, const uint8_t *,
      uint64_t, EvimCoreOutcomeV1 *) = evim_core_view_send_text;
  EvimStatus (*send_key_with_host_context)(EvimCoreHandle, EvimViewId,
      const EvimKeyInputV1 *, const EvimCommandTurnContextV1 *,
      EvimCoreOutcomeV1 *, EvimEffectBatchHandle *) =
      evim_core_view_send_key_with_host_context;
  EvimStatus (*send_text_with_host_context)(EvimCoreHandle, EvimViewId,
      const uint8_t *, uint64_t, const EvimCommandTurnContextV1 *,
      EvimCoreOutcomeV1 *, EvimEffectBatchHandle *) =
      evim_core_view_send_text_with_host_context;
  EvimStatus (*effect_info)(EvimEffectBatchHandle,
      EvimEffectBatchInfoV1 *) = evim_effect_batch_info;
  EvimStatus (*effect_copy)(EvimEffectBatchHandle,
      EvimClipboardWriteV1 *, uint64_t,
      EvimExFrontendRequestV1 *, uint64_t,
      EvimExOptionDisplayV1 *, uint64_t,
      EvimExMarkV1 *, uint64_t,
      EvimExRegisterV1 *, uint64_t,
      EvimExJumpV1 *, uint64_t,
      EvimExTextLineV1 *, uint64_t,
      uint32_t *, uint64_t, uint64_t *, uint64_t,
      uint8_t *, uint64_t, EvimEffectBatchInfoV1 *) =
      evim_effect_batch_copy;
  EvimStatus (*effect_release)(EvimEffectBatchHandle) =
      evim_effect_batch_release;
  EvimStatus (*place_cursor)(EvimCoreHandle, EvimViewId,
      const EvimPlaceCursorV1 *, EvimCoreOutcomeV1 *) =
      evim_core_view_place_cursor;
  EvimStatus (*undo)(EvimCoreHandle, EvimViewId, EvimCoreOutcomeV1 *) =
      evim_core_view_undo;
  EvimStatus (*redo)(EvimCoreHandle, EvimViewId, EvimCoreOutcomeV1 *) =
      evim_core_view_redo;
  EvimStatus (*composition_begin)(EvimCoreHandle, EvimViewId,
      const EvimCompositionBeginV1 *, EvimCoreOutcomeV1 *) =
      evim_core_view_composition_begin;
  EvimStatus (*composition_update)(EvimCoreHandle, EvimViewId,
      const EvimCompositionUpdateV1 *, EvimCoreOutcomeV1 *) =
      evim_core_view_composition_update;
  EvimStatus (*composition_overlay_info)(EvimCoreHandle, EvimViewId,
      EvimCompositionOverlayInfoV1 *) =
      evim_core_view_composition_overlay_info;
  EvimStatus (*copy_composition_range)(EvimCoreHandle, EvimViewId,
      const EvimCompositionOverlayUtf8RangeV1 *, uint8_t *, uint64_t,
      uint64_t *) = evim_core_view_copy_composition_utf8_range;
  EvimStatus (*composition_commit)(EvimCoreHandle, EvimViewId,
      const EvimCompositionCommitV1 *, EvimCoreOutcomeV1 *) =
      evim_core_view_composition_commit;
  EvimStatus (*composition_cancel)(EvimCoreHandle, EvimViewId,
      const EvimCompositionCancelV1 *, EvimCoreOutcomeV1 *) =
      evim_core_view_composition_cancel;
  (void)request; (void)create_core; (void)document_state;
  (void)formatted_info; (void)copy_formatted_range;
  (void)map_utf8_to_utf16; (void)map_utf16_to_utf8;
  (void)formatted_point; (void)mark_saved;
  (void)style_sheet_info; (void)copy_style_sheet;
  (void)add_view; (void)viewport_state;
  (void)layout_info; (void)copy_layout; (void)layout_paint_info;
  (void)copy_layout_paint; (void)caret_geometry; (void)hit_test;
  (void)presentation; (void)command_line_info; (void)copy_command_line;
  (void)visual_selection_info; (void)copy_visual_selection;
  (void)use_selection_for_find; (void)reveal_selection;
  (void)set_viewport_origin; (void)set_scale;
  (void)set_linebreak; (void)set_file_format;
  (void)edit_style; (void)begin_style_group; (void)edit_style_in_group;
  (void)assign_style;
  (void)create_style; (void)delete_style;
  (void)end_style_group;
  (void)send_key;
  (void)send_text; (void)send_key_with_host_context;
  (void)send_text_with_host_context; (void)effect_info;
  (void)effect_copy; (void)effect_release;
  (void)place_cursor; (void)undo; (void)redo;
  (void)composition_begin; (void)composition_update;
  (void)composition_overlay_info; (void)copy_composition_range;
  (void)composition_commit; (void)composition_cancel;
}}
"#,
        abi = EVIM_CORE_ABI_VERSION,
        direct_style_edit = std::mem::size_of::<EvimDirectStyleEditV1>(),
        direct_style_align = std::mem::align_of::<EvimDirectStyleEditV1>(),
        direct_style_selection = std::mem::offset_of!(EvimDirectStyleEditV1, expected_selection),
        direct_style_value = std::mem::offset_of!(EvimDirectStyleEditV1, value),
        document_options = std::mem::size_of::<EvimDocumentOptions>(),
        document_state = std::mem::size_of::<EvimDocumentStateV1>(),
        formatted_identity = std::mem::size_of::<EvimFormattedSnapshotIdentityV1>(),
        formatted_identity_document =
            std::mem::offset_of!(EvimFormattedSnapshotIdentityV1, document_id),
        formatted_info = std::mem::size_of::<EvimFormattedSnapshotInfoV1>(),
        formatted_info_utf8 = std::mem::offset_of!(EvimFormattedSnapshotInfoV1, utf8_length),
        formatted_range = std::mem::size_of::<EvimFormattedUtf8RangeV1>(),
        formatted_range_start = std::mem::offset_of!(EvimFormattedUtf8RangeV1, utf8_start),
        formatted_point = std::mem::size_of::<EvimFormattedPointInfoV1>(),
        formatted_point_column = std::mem::offset_of!(EvimFormattedPointInfoV1, grapheme_column),
        clipboard_turn_entry = std::mem::size_of::<EvimClipboardTurnEntryV1>(),
        clipboard_turn_text = std::mem::offset_of!(EvimClipboardTurnEntryV1, plain_text),
        command_turn_context = std::mem::size_of::<EvimCommandTurnContextV1>(),
        effect_bytes_ref = std::mem::size_of::<EvimEffectBytesRefV1>(),
        clipboard_write = std::mem::size_of::<EvimClipboardWriteV1>(),
        ex_option = std::mem::size_of::<EvimExOptionDisplayV1>(),
        ex_mark = std::mem::size_of::<EvimExMarkV1>(),
        ex_register = std::mem::size_of::<EvimExRegisterV1>(),
        ex_jump = std::mem::size_of::<EvimExJumpV1>(),
        ex_text_line = std::mem::size_of::<EvimExTextLineV1>(),
        ex_request = std::mem::size_of::<EvimExFrontendRequestV1>(),
        ex_request_payload = std::mem::offset_of!(EvimExFrontendRequestV1, first_payload),
        effect_batch_info = std::mem::size_of::<EvimEffectBatchInfoV1>(),
        effect_batch_mark_count = std::mem::offset_of!(EvimEffectBatchInfoV1, ex_mark_count),
        style = std::mem::size_of::<EvimResolvedTextStyleV1>(),
        style_run = std::mem::size_of::<EvimShapeStyleRunV1>(),
        cluster = std::mem::size_of::<EvimShapedClusterV1>(),
        diagnostic = std::mem::size_of::<EvimShapingDiagnosticV1>(),
        request = std::mem::size_of::<EvimShapeRequestV1>(),
        response = std::mem::size_of::<EvimShapeResponseV1>(),
        provider = std::mem::size_of::<EvimTextMeasurementProviderV1>(),
        view_options = std::mem::size_of::<EvimViewOptionsV1>(),
        viewport_origin = std::mem::size_of::<EvimViewportOriginV1>(),
        viewport_state = std::mem::size_of::<EvimViewportStateV1>(),
        layout_identity = std::mem::size_of::<EvimLayoutSnapshotIdentityV1>(),
        layout_insets = std::mem::size_of::<EvimLayoutInsetsV1>(),
        layout_rect = std::mem::size_of::<EvimLayoutRectV1>(),
        rgba = std::mem::size_of::<EvimRgbaV1>(),
        style_sheet_identity = std::mem::size_of::<EvimStyleSheetIdentityV1>(),
        style_string_ref = std::mem::size_of::<EvimStyleStringRefV1>(),
        style_sheet_info = std::mem::size_of::<EvimStyleSheetInfoV1>(),
        style_value = std::mem::size_of::<EvimStyleValueV1>(),
        style_value_item = std::mem::size_of::<EvimStyleValueItemV1>(),
        style_dependency = std::mem::size_of::<EvimStyleDependencyV1>(),
        style_definition = std::mem::size_of::<EvimStyleDefinitionV1>(),
        style_property = std::mem::size_of::<EvimStylePropertyV1>(),
        style_edit_value_item = std::mem::size_of::<EvimStyleEditValueItemV1>(),
        style_edit_value = std::mem::size_of::<EvimStyleEditValueV1>(),
        style_edit = std::mem::size_of::<EvimStyleEditV1>(),
        style_edit_group = std::mem::size_of::<EvimStyleEditGroupV1>(),
        style_edit_group_token = std::mem::offset_of!(EvimStyleEditGroupV1, token),
        text_paint = std::mem::size_of::<EvimTextPaintV1>(),
        layout_paint_info = std::mem::size_of::<EvimLayoutPaintInfoV1>(),
        paint_style_run = std::mem::size_of::<EvimPaintStyleRunV1>(),
        layout_info = std::mem::size_of::<EvimLayoutSnapshotInfoV1>(),
        visual_row = std::mem::size_of::<EvimVisualRowV1>(),
        positioned_cluster = std::mem::size_of::<EvimPositionedClusterV1>(),
        positioned_caret = std::mem::size_of::<EvimPositionedCaretV1>(),
        caret_request = std::mem::size_of::<EvimLayoutCaretRequestV1>(),
        hit_test_request = std::mem::size_of::<EvimLayoutHitTestRequestV1>(),
        caret_point = std::mem::size_of::<EvimLayoutCaretPointV1>(),
        caret_geometry = std::mem::size_of::<EvimLayoutCaretGeometryV1>(),
        presentation = std::mem::size_of::<EvimViewPresentationV1>(),
        command_line_identity = std::mem::size_of::<EvimCommandLineIdentityV1>(),
        command_line_info = std::mem::size_of::<EvimCommandLineInfoV1>(),
        visual_selection_identity = std::mem::size_of::<EvimVisualSelectionIdentityV1>(),
        visual_selection_info = std::mem::size_of::<EvimVisualSelectionInfoV1>(),
        visual_selection_segment = std::mem::size_of::<EvimVisualSelectionSegmentV1>(),
        visual_selection_rectangle = std::mem::size_of::<EvimVisualSelectionRectangleV1>(),
        place_cursor = std::mem::size_of::<EvimPlaceCursorV1>(),
        assign_style = std::mem::size_of::<EvimAssignStyleV1>(),
        create_style = std::mem::size_of::<EvimCreateStyleV1>(),
        delete_style = std::mem::size_of::<EvimDeleteStyleV1>(),
        set_file_format = std::mem::size_of::<EvimSetFileFormatV1>(),
        mark_saved = std::mem::size_of::<EvimMarkSavedV1>(),
        key = std::mem::size_of::<EvimKeyInputV1>(),
        composition_begin = std::mem::size_of::<EvimCompositionBeginV1>(),
        composition_update = std::mem::size_of::<EvimCompositionUpdateV1>(),
        composition_commit = std::mem::size_of::<EvimCompositionCommitV1>(),
        composition_cancel = std::mem::size_of::<EvimCompositionCancelV1>(),
        composition_overlay_identity = std::mem::size_of::<EvimCompositionOverlayIdentityV1>(),
        composition_overlay_info = std::mem::size_of::<EvimCompositionOverlayInfoV1>(),
        composition_overlay_range = std::mem::size_of::<EvimCompositionOverlayUtf8RangeV1>(),
        outcome = std::mem::size_of::<EvimCoreOutcomeV1>(),
    );
    std::fs::write(&source_path, source).unwrap();
    let output = std::process::Command::new("cc")
        .args(["-std=c11", "-Werror", "-fsyntax-only", "-I"])
        .arg(manifest.join("include"))
        .arg(&source_path)
        .output()
        .expect("a C compiler is required to verify the public header");
    let _ = std::fs::remove_file(&source_path);
    assert!(
        output.status.success(),
        "header did not match Rust ABI:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn native_direct_properties_and_decoration_queries_are_typed_exact_and_undoable() {
    use evim_core::document::{Document, Encoding, Format, ParagraphAlignment};
    for (format, source, adapter) in [
        (
            EVIM_FORMAT_HTML,
            "<p data-x='keep'>Alpha</p><p>Beta</p>",
            Format::Html,
        ),
        (EVIM_FORMAT_RTF, r"{\rtf1 Alpha\par Beta}", Format::Rtf),
    ] {
        let core = create_core(
            source.as_bytes(),
            EvimDocumentOptions {
                format,
                ..Default::default()
            },
        );
        let mut provider = Box::new(FakeProviderContext::new(core.handle));
        let (view, mut outcome) = add_test_view(&core, provider.as_mut());
        let selection = || {
            let mut value = EvimLogicalSelectionIdentityV1::default();
            assert_eq!(
                unsafe { evim_core_view_list_selection(core.handle, view, &mut value) },
                EvimStatus::Ok
            );
            value
        };
        let bytes = || {
            copy_core_bytes(
                evim_core_copy_source_bytes,
                &core,
                document_state(&core).document_revision,
            )
        };
        let mut request = EvimDirectStyleEditV1 {
            struct_size: EVIM_DIRECT_STYLE_EDIT_V1_SIZE,
            operation: EVIM_STYLE_EDIT_SET_DECLARATION,
            property: EVIM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT,
            expected_selection: selection(),
            value: EvimStyleEditValueV1 {
                kind: EVIM_STYLE_VALUE_PARAGRAPH_ALIGNMENT,
                enum_value: EVIM_STYLE_PARAGRAPH_ALIGNMENT_CENTER,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut invalid = request;
        invalid.value.kind = EVIM_STYLE_VALUE_BOOLEAN;
        assert_eq!(
            unsafe { evim_core_view_edit_direct_style(core.handle, view, &invalid, &mut outcome) },
            EvimStatus::InvalidStyleValue
        );
        invalid = request;
        invalid.property = u32::MAX;
        assert_eq!(
            unsafe { evim_core_view_edit_direct_style(core.handle, view, &invalid, &mut outcome) },
            EvimStatus::InvalidStyleValue
        );
        invalid = request;
        invalid.reserved = 1;
        assert_eq!(
            unsafe { evim_core_view_edit_direct_style(core.handle, view, &invalid, &mut outcome) },
            EvimStatus::InvalidArgument
        );
        assert_eq!(bytes(), source.as_bytes());
        assert_eq!(
            unsafe { evim_core_view_edit_direct_style(core.handle, view, &request, &mut outcome) },
            EvimStatus::Ok,
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
            unsafe { evim_core_view_edit_direct_style(core.handle, view, &request, &mut outcome) },
            EvimStatus::StaleRevision
        );
        assert_eq!(
            unsafe { evim_core_view_undo(core.handle, view, &mut outcome) },
            EvimStatus::Ok
        );
        assert_eq!(bytes(), source.as_bytes());
        for character in ['v', 'l'] {
            assert_eq!(
                unsafe {
                    evim_core_view_send_key(
                        core.handle,
                        view,
                        &key(EVIM_KEY_CHARACTER, character as u32),
                        &mut outcome,
                    )
                },
                EvimStatus::Ok
            );
        }
        request.expected_selection = selection();
        request.property = EVIM_STYLE_PROPERTY_CHARACTER_UNDERLINE;
        request.value = EvimStyleEditValueV1 {
            kind: EVIM_STYLE_VALUE_BOOLEAN,
            enum_value: 1,
            ..Default::default()
        };
        assert_eq!(
            unsafe {
                evim_core_view_send_key(core.handle, view, &key(EVIM_KEY_RIGHT, 0), &mut outcome)
            },
            EvimStatus::Ok
        );
        assert_eq!(
            unsafe { evim_core_view_edit_direct_style(core.handle, view, &request, &mut outcome) },
            EvimStatus::StaleRevision
        );
        assert_eq!(bytes(), source.as_bytes());
        for property in [
            EVIM_STYLE_PROPERTY_CHARACTER_UNDERLINE,
            EVIM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH,
        ] {
            assert_eq!(
                unsafe {
                    evim_core_view_send_key(
                        core.handle,
                        view,
                        &key(EVIM_KEY_ESCAPE, 0),
                        &mut outcome,
                    )
                },
                EvimStatus::Ok
            );
            for character in ['0', 'v', 'l', 'l'] {
                assert_eq!(
                    unsafe {
                        evim_core_view_send_key(
                            core.handle,
                            view,
                            &key(EVIM_KEY_CHARACTER, character as u32),
                            &mut outcome,
                        )
                    },
                    EvimStatus::Ok
                );
            }
            request.expected_selection = selection();
            request.property = property;
            request.value.enum_value = 1;
            let mut state = u32::MAX;
            assert_eq!(
                unsafe { evim_core_view_decoration_state(core.handle, view, property, &mut state) },
                EvimStatus::Ok
            );
            assert_eq!(state, EVIM_SEMANTIC_STYLE_STATE_OFF);
            assert_eq!(
                unsafe {
                    evim_core_view_edit_direct_style(core.handle, view, &request, &mut outcome)
                },
                EvimStatus::Ok,
                "format {format} property {property}"
            );
            assert_eq!(
                unsafe { evim_core_view_decoration_state(core.handle, view, property, &mut state) },
                EvimStatus::Ok
            );
            assert_eq!(state, EVIM_SEMANTIC_STYLE_STATE_ON);
            assert_eq!(
                unsafe {
                    evim_core_view_send_key(
                        core.handle,
                        view,
                        &key(EVIM_KEY_RIGHT, 0),
                        &mut outcome,
                    )
                },
                EvimStatus::Ok
            );
            assert_eq!(
                unsafe { evim_core_view_decoration_state(core.handle, view, property, &mut state) },
                EvimStatus::Ok
            );
            assert_eq!(
                state,
                EVIM_SEMANTIC_STYLE_STATE_MIXED,
                "format {format} property {property} selection {:?} source {}",
                selection(),
                String::from_utf8_lossy(&bytes())
            );
            assert_eq!(
                unsafe {
                    evim_core_view_send_key(core.handle, view, &key(EVIM_KEY_LEFT, 0), &mut outcome)
                },
                EvimStatus::Ok
            );
            request.expected_selection = selection();
            request.value.enum_value = 0;
            assert_eq!(
                unsafe {
                    evim_core_view_edit_direct_style(core.handle, view, &request, &mut outcome)
                },
                EvimStatus::Ok
            );
            assert_eq!(
                unsafe { evim_core_view_decoration_state(core.handle, view, property, &mut state) },
                EvimStatus::Ok
            );
            assert_eq!(state, EVIM_SEMANTIC_STYLE_STATE_OFF);
            assert_eq!(
                unsafe { evim_core_view_undo(core.handle, view, &mut outcome) },
                EvimStatus::Ok
            );
            for character in ['0', 'v', 'l', 'l'] {
                assert_eq!(
                    unsafe {
                        evim_core_view_send_key(
                            core.handle,
                            view,
                            &key(EVIM_KEY_CHARACTER, character as u32),
                            &mut outcome,
                        )
                    },
                    EvimStatus::Ok
                );
            }
            assert_eq!(
                unsafe { evim_core_view_decoration_state(core.handle, view, property, &mut state) },
                EvimStatus::Ok
            );
            assert_eq!(state, EVIM_SEMANTIC_STYLE_STATE_ON);
            assert_eq!(
                unsafe { evim_core_view_undo(core.handle, view, &mut outcome) },
                EvimStatus::Ok
            );
            assert_eq!(bytes(), source.as_bytes());
        }
        let mut sentinel = 919u32;
        assert_eq!(
            unsafe {
                evim_core_view_decoration_state(
                    core.handle,
                    view,
                    EVIM_STYLE_PROPERTY_CHARACTER_WEIGHT,
                    &mut sentinel,
                )
            },
            EvimStatus::InvalidArgument
        );
        assert_eq!(sentinel, 919);
        assert_eq!(
            unsafe {
                evim_core_view_decoration_state(
                    core.handle,
                    view,
                    EVIM_STYLE_PROPERTY_CHARACTER_UNDERLINE,
                    std::ptr::null_mut(),
                )
            },
            EvimStatus::NullPointer
        );
    }
}

#[test]
fn rich_bold_and_italic_toggle_states_include_default_gaps() {
    for (format, source) in [
        (EVIM_FORMAT_HTML, "<p><b><i>A</i></b>B</p>"),
        (EVIM_FORMAT_RTF, r"{\rtf1{\b\i A}B}"),
    ] {
        let core = create_core(
            source.as_bytes(),
            EvimDocumentOptions {
                format,
                ..Default::default()
            },
        );
        let mut provider = Box::new(FakeProviderContext::new(core.handle));
        let (view, mut outcome) = add_test_view(&core, provider.as_mut());
        for character in ['v', 'l'] {
            assert_eq!(
                unsafe {
                    evim_core_view_send_key(
                        core.handle,
                        view,
                        &key(EVIM_KEY_CHARACTER, character as u32),
                        &mut outcome,
                    )
                },
                EvimStatus::Ok
            );
        }
        for style in [EVIM_SEMANTIC_STYLE_STRONG, EVIM_SEMANTIC_STYLE_EMPHASIS] {
            let mut presentation = EvimSemanticStylePresentationV1::default();
            assert_eq!(
                unsafe {
                    evim_core_view_semantic_style_presentation(
                        core.handle,
                        view,
                        style,
                        &mut presentation,
                    )
                },
                EvimStatus::Ok
            );
            assert_eq!(
                presentation.state, EVIM_SEMANTIC_STYLE_STATE_MIXED,
                "format {format} style {style}"
            );
        }
    }
}

#[test]
fn native_select_all_nested_html_decoration_round_trips_query_state() {
    let source = "<p><b data-keep='yes'>Words</b></p><!--keep-->";
    let core = create_core(
        source.as_bytes(),
        EvimDocumentOptions {
            format: EVIM_FORMAT_HTML,
            ..Default::default()
        },
    );
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, provider.as_mut());
    for character in ['g', 'g', 'V', 'G'] {
        assert_eq!(
            unsafe {
                evim_core_view_send_key(
                    core.handle,
                    view,
                    &key(EVIM_KEY_CHARACTER, character as u32),
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
    }
    for property in [
        EVIM_STYLE_PROPERTY_CHARACTER_UNDERLINE,
        EVIM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH,
    ] {
        let mut selection = EvimLogicalSelectionIdentityV1::default();
        assert_eq!(
            unsafe { evim_core_view_list_selection(core.handle, view, &mut selection) },
            EvimStatus::Ok
        );
        let request = EvimDirectStyleEditV1 {
            struct_size: EVIM_DIRECT_STYLE_EDIT_V1_SIZE,
            operation: EVIM_STYLE_EDIT_SET_DECLARATION,
            property,
            expected_selection: selection,
            value: EvimStyleEditValueV1 {
                kind: EVIM_STYLE_VALUE_BOOLEAN,
                enum_value: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            unsafe { evim_core_view_edit_direct_style(core.handle, view, &request, &mut outcome) },
            EvimStatus::Ok
        );
        let mut state = u32::MAX;
        assert_eq!(
            unsafe { evim_core_view_decoration_state(core.handle, view, property, &mut state) },
            EvimStatus::Ok
        );
        assert_eq!(state, EVIM_SEMANTIC_STYLE_STATE_ON);
    }
}

#[test]
fn rich_decoration_state_remains_on_after_select_all_linewise_toggle() {
    for (format, source) in [
        (
            EVIM_FORMAT_HTML,
            "<p><b data-keep='yes'>Words</b></p><!--keep-->",
        ),
        (EVIM_FORMAT_RTF, r"{\rtf1{\b Words}{\*\opaque keep}}"),
    ] {
        let core = create_core(
            source.as_bytes(),
            EvimDocumentOptions {
                format,
                ..Default::default()
            },
        );
        let mut provider = Box::new(FakeProviderContext::new(core.handle));
        let (view, mut outcome) = add_test_view(&core, provider.as_mut());
        for character in ['g', 'g', 'V', 'G'] {
            assert_eq!(
                unsafe {
                    evim_core_view_send_key(
                        core.handle,
                        view,
                        &key(EVIM_KEY_CHARACTER, character as u32),
                        &mut outcome,
                    )
                },
                EvimStatus::Ok
            );
        }
        let mut expected = EvimLogicalSelectionIdentityV1::default();
        assert_eq!(
            unsafe { evim_core_view_list_selection(core.handle, view, &mut expected) },
            EvimStatus::Ok
        );
        let request = EvimDirectStyleEditV1 {
            struct_size: EVIM_DIRECT_STYLE_EDIT_V1_SIZE,
            operation: EVIM_STYLE_EDIT_SET_DECLARATION,
            property: EVIM_STYLE_PROPERTY_CHARACTER_UNDERLINE,
            expected_selection: expected,
            value: EvimStyleEditValueV1 {
                kind: EVIM_STYLE_VALUE_BOOLEAN,
                enum_value: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            unsafe { evim_core_view_edit_direct_style(core.handle, view, &request, &mut outcome) },
            EvimStatus::Ok
        );
        let mut state = u32::MAX;
        assert_eq!(
            unsafe {
                evim_core_view_decoration_state(core.handle, view, request.property, &mut state)
            },
            EvimStatus::Ok
        );
        assert_eq!(state, EVIM_SEMANTIC_STYLE_STATE_ON, "format {format}");
    }
}

#[test]
fn native_scalar_visual_change_keeps_the_entire_html_inline_style() {
    let source = "<p>Bold <b foo='keep'>words</b> and &#x26; text.</p><!--keep-->";
    let core = create_core(
        source.as_bytes(),
        EvimDocumentOptions {
            format: EVIM_FORMAT_HTML,
            ..Default::default()
        },
    );
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, provider.as_mut());
    for character in "wvecORDS".chars() {
        let text = character.to_string();
        assert_eq!(
            unsafe {
                evim_core_view_send_text(
                    core.handle,
                    view,
                    text.as_ptr(),
                    text.len() as u64,
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
    }
    assert_eq!(
        unsafe {
            evim_core_view_send_key(core.handle, view, &key(EVIM_KEY_ESCAPE, 0), &mut outcome)
        },
        EvimStatus::Ok
    );
    let bytes = copy_core_bytes(
        evim_core_copy_source_bytes,
        &core,
        document_state(&core).document_revision,
    );
    assert!(
        String::from_utf8_lossy(&bytes).contains("<b foo='keep'>ORDS</b>"),
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(
        unsafe { evim_core_view_undo(core.handle, view, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        copy_core_bytes(
            evim_core_copy_source_bytes,
            &core,
            document_state(&core).document_revision
        ),
        source.as_bytes()
    );
}

#[test]
fn native_scalar_typing_after_html_heading_enter_uses_the_new_paragraph() {
    let source = "<h2>Heading</h2><p>Tail</p>";
    let core = create_core(
        source.as_bytes(),
        EvimDocumentOptions {
            format: EVIM_FORMAT_HTML,
            ..Default::default()
        },
    );
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, provider.as_mut());
    assert_eq!(
        unsafe { evim_core_view_send_text(core.handle, view, b"A".as_ptr(), 1, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe {
            evim_core_view_send_key(core.handle, view, &key(EVIM_KEY_ENTER, 0), &mut outcome)
        },
        EvimStatus::Ok
    );
    for character in "Body".chars() {
        let text = character.to_string();
        assert_eq!(
            unsafe {
                evim_core_view_send_text(
                    core.handle,
                    view,
                    text.as_ptr(),
                    text.len() as u64,
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
    }
    assert_eq!(
        unsafe {
            evim_core_view_send_key(core.handle, view, &key(EVIM_KEY_ESCAPE, 0), &mut outcome)
        },
        EvimStatus::Ok
    );
    let bytes = copy_core_bytes(
        evim_core_copy_source_bytes,
        &core,
        document_state(&core).document_revision,
    );
    assert!(
        String::from_utf8_lossy(&bytes).contains("</h2><p>Body</p>"),
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(
        unsafe { evim_core_view_undo(core.handle, view, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        copy_core_bytes(
            evim_core_copy_source_bytes,
            &core,
            document_state(&core).document_revision
        ),
        source.as_bytes()
    );
}

#[test]
fn checked_line_mode_and_location_queries_are_view_local_and_do_not_edit() {
    assert_eq!(std::mem::size_of::<EvimViewLineLocationV1>(), 48);
    let core = create_core(b"abcdef\nsecond", EvimDocumentOptions::default());
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, &mut *provider);
    let (second, _) = add_test_view(&core, &mut *provider);
    let mut mode = 99;
    assert_eq!(
        unsafe { evim_core_view_line_mode(core.handle, view, &mut mode) },
        EvimStatus::Ok
    );
    assert_eq!(mode, 0);
    let mut location = EvimViewLineLocationV1::default();
    assert_eq!(
        unsafe { evim_core_view_line_location(core.handle, view, &mut location) },
        EvimStatus::Ok
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
        EVIM_LINE_LOCATION_GLOBAL_LINE_EXACT | EVIM_LINE_LOCATION_FRAGMENT_EXACT
    );
    assert_eq!(
        unsafe { evim_core_view_set_line_mode(core.handle, view, 1, &mut outcome) },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe { evim_core_view_line_mode(core.handle, second, &mut mode) },
        EvimStatus::Ok
    );
    assert_eq!(mode, 0);
    assert_eq!(
        unsafe { evim_core_view_line_location(core.handle, view, &mut location) },
        EvimStatus::Ok
    );
    assert_eq!(location.mode, 1);
    let revision = document_state(&core).document_revision;
    assert_eq!(
        unsafe { evim_core_view_set_line_mode(core.handle, view, 77, &mut outcome) },
        EvimStatus::InvalidArgument
    );
    assert_eq!(
        unsafe { evim_core_view_line_location(core.handle, u64::MAX, &mut location) },
        EvimStatus::InvalidView
    );
    assert_eq!(
        unsafe { evim_core_view_line_location(core.handle, view, ptr::null_mut()) },
        EvimStatus::InvalidArgument
    );
    assert_eq!(document_state(&core).document_revision, revision);
    assert_eq!(
        copy_core_bytes(evim_core_copy_source_bytes, &core, revision),
        b"abcdef\nsecond"
    );
}

#[test]
fn typography_export_is_exact_batched_stale_checked_and_includes_mixed_default_gaps() {
    let core = create_core(
        b"<p style='font-family:Arial;font-size:20pt'><b>A</b>B</p>",
        EvimDocumentOptions {
            format: EVIM_FORMAT_HTML,
            ..Default::default()
        },
    );
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, provider.as_mut());
    for character in ['v', 'l'] {
        assert_eq!(
            unsafe {
                evim_core_view_send_key(
                    core.handle,
                    view,
                    &key(EVIM_KEY_CHARACTER, character as u32),
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
    }
    let mut info = EvimTypographyInfoV1::default();
    assert_eq!(
        unsafe {
            evim_core_view_typography_export(
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
        EvimStatus::BufferTooSmall
    );
    assert_eq!(info.flags & 3, 3);
    assert_eq!(info.size, 20.0);
    assert_eq!(info.base_weight, 400);
    assert_eq!(info.weight, 700);
    let mut family = vec![0; info.font_family_bytes as usize];
    let mut features = vec![EvimOpenTypeFeatureV1::default(); info.feature_count as usize];
    assert_eq!(
        unsafe {
            evim_core_view_typography_export(
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
        EvimStatus::Ok
    );
    assert_eq!(family, b"Arial");
    family.fill(0xFF);
    assert_eq!(
        unsafe {
            evim_core_view_typography_export(
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
        EvimStatus::StaleRevision
    );
    assert!(family.iter().all(|byte| *byte == 0xFF));
    assert_eq!(
        unsafe {
            evim_core_view_typography_export(
                core.handle,
                view,
                outcome.document_revision,
                &mut info,
                (&mut info as *mut EvimTypographyInfoV1).cast(),
                1,
                ptr::null_mut(),
                0,
            )
        },
        EvimStatus::InvalidArgument
    );
}

#[test]
fn direct_character_batch_is_atomic_and_rejects_duplicate_stale_or_overlapping_requests() {
    let original = b"<p>Text</p><!--keep-->";
    let core = create_core(
        original,
        EvimDocumentOptions {
            format: EVIM_FORMAT_HTML,
            ..Default::default()
        },
    );
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, mut outcome) = add_test_view(&core, provider.as_mut());
    for character in ['v', 'e'] {
        assert_eq!(
            unsafe {
                evim_core_view_send_key(
                    core.handle,
                    view,
                    &key(EVIM_KEY_CHARACTER, character as u32),
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
    }
    let mut selection = EvimLogicalSelectionIdentityV1::default();
    assert_eq!(
        unsafe { evim_core_view_list_selection(core.handle, view, &mut selection) },
        EvimStatus::Ok
    );
    let make = |property, value| EvimDirectStyleEditV1 {
        struct_size: EVIM_DIRECT_STYLE_EDIT_V1_SIZE,
        operation: EVIM_STYLE_EDIT_SET_DECLARATION,
        property,
        value,
        expected_selection: selection,
        ..Default::default()
    };
    let requests = [
        make(
            EVIM_STYLE_PROPERTY_CHARACTER_WEIGHT,
            EvimStyleEditValueV1 {
                kind: EVIM_STYLE_VALUE_UNSIGNED,
                enum_value: 200,
                ..Default::default()
            },
        ),
        make(
            EVIM_STYLE_PROPERTY_CHARACTER_BOLD,
            EvimStyleEditValueV1 {
                kind: EVIM_STYLE_VALUE_BOOLEAN,
                enum_value: 1,
                ..Default::default()
            },
        ),
        make(
            EVIM_STYLE_PROPERTY_CHARACTER_SIZE,
            EvimStyleEditValueV1 {
                kind: EVIM_STYLE_VALUE_FLOAT,
                number: 24.0,
                ..Default::default()
            },
        ),
    ];
    let duplicate = [requests[0], requests[0]];
    assert_eq!(
        unsafe {
            evim_core_view_edit_direct_character_batch(
                core.handle,
                view,
                duplicate.as_ptr(),
                2,
                &mut outcome,
            )
        },
        EvimStatus::InvalidArgument
    );
    assert_eq!(
        copy_core_bytes(
            evim_core_copy_source_bytes,
            &core,
            document_state(&core).document_revision
        ),
        original
    );
    assert_eq!(
        unsafe {
            evim_core_view_edit_direct_character_batch(
                core.handle,
                view,
                requests.as_ptr(),
                3,
                requests.as_ptr().cast_mut().cast(),
            )
        },
        EvimStatus::InvalidArgument
    );
    assert_eq!(
        unsafe {
            evim_core_view_edit_direct_character_batch(
                core.handle,
                view,
                requests.as_ptr(),
                3,
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    let changed_revision = outcome.document_revision;
    assert_eq!(
        unsafe {
            evim_core_view_edit_direct_character_batch(
                core.handle,
                view,
                requests.as_ptr(),
                3,
                &mut outcome,
            )
        },
        EvimStatus::StaleRevision
    );
    let mut info = EvimTypographyInfoV1::default();
    assert_eq!(
        unsafe {
            evim_core_view_typography_export(
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
        EvimStatus::BufferTooSmall
    );
    assert_eq!((info.base_weight, info.weight, info.size), (200, 500, 24.0));
    assert_eq!(
        unsafe {
            evim_core_view_send_key(core.handle, view, &key(EVIM_KEY_ESCAPE, 0), &mut outcome)
        },
        EvimStatus::Ok
    );
    assert_eq!(
        unsafe {
            evim_core_view_send_key(
                core.handle,
                view,
                &key(EVIM_KEY_CHARACTER, 'u' as u32),
                &mut outcome,
            )
        },
        EvimStatus::Ok
    );
    assert_eq!(
        copy_core_bytes(
            evim_core_copy_source_bytes,
            &core,
            outcome.document_revision
        ),
        original
    );
}

#[test]
fn readonly_and_recovered_flags_are_exact_buffer_policies_and_save_clears_recovery() {
    let core = create_core(b"recovered", EvimDocumentOptions::default());
    let initial = document_state(&core);
    assert_eq!(
        evim_core_set_read_only(
            core.handle,
            initial.document_id,
            initial.document_revision,
            2
        ),
        EvimStatus::InvalidArgument
    );
    assert_eq!(
        evim_core_set_read_only(
            core.handle,
            initial.document_id + 1,
            initial.document_revision,
            1
        ),
        EvimStatus::InvalidArgument
    );
    assert_eq!(
        evim_core_mark_recovered(
            core.handle,
            initial.document_id,
            initial.document_revision + 1
        ),
        EvimStatus::StaleRevision
    );
    assert_eq!(document_state(&core), initial);
    assert_eq!(
        evim_core_set_read_only(
            core.handle,
            initial.document_id,
            initial.document_revision,
            1
        ),
        EvimStatus::Ok
    );
    let readonly = document_state(&core);
    assert_eq!(readonly.document_revision, initial.document_revision);
    assert_eq!(readonly.flags & EVIM_DOCUMENT_STATE_IS_DIRTY, 0);
    assert_ne!(readonly.flags & EVIM_DOCUMENT_STATE_READ_ONLY, 0);
    assert_eq!(
        evim_core_mark_recovered(core.handle, initial.document_id, initial.document_revision),
        EvimStatus::Ok
    );
    let recovered = document_state(&core);
    assert_ne!(recovered.flags & EVIM_DOCUMENT_STATE_IS_DIRTY, 0);
    assert_ne!(recovered.flags & EVIM_DOCUMENT_STATE_RECOVERED, 0);
    assert_eq!(recovered.flags & EVIM_DOCUMENT_STATE_CAN_UNDO, 0);
    let saved = EvimMarkSavedV1 {
        struct_size: EVIM_MARK_SAVED_V1_SIZE,
        document_id: initial.document_id,
        document_revision: initial.document_revision,
        ..EvimMarkSavedV1::default()
    };
    assert_eq!(
        unsafe { evim_core_mark_saved(core.handle, &saved) },
        EvimStatus::Ok
    );
    let result = document_state(&core);
    assert_eq!(
        result.flags & (EVIM_DOCUMENT_STATE_IS_DIRTY | EVIM_DOCUMENT_STATE_RECOVERED),
        0
    );
    assert_ne!(result.flags & EVIM_DOCUMENT_STATE_READ_ONLY, 0);
    assert_eq!(
        copy_core_bytes(
            evim_core_copy_source_bytes,
            &core,
            initial.document_revision
        ),
        b"recovered"
    );
}

#[test]
fn readonly_ex_error_has_a_distinct_abi_status_and_no_host_write_effect() {
    let core = create_core(b"Text", EvimDocumentOptions::default());
    let mut provider = Box::new(FakeProviderContext::new(core.handle));
    let (view, _) = add_test_view(&core, &mut *provider);
    let state = document_state(&core);
    assert_eq!(
        evim_core_set_read_only(core.handle, state.document_id, state.document_revision, 1),
        EvimStatus::Ok
    );
    host_chars(&core, view, ":w");
    let (status, outcome, effects) = host_key(&core, view, key(EVIM_KEY_ENTER, 0), &[]);
    assert_eq!(status, EvimStatus::Ok);
    assert_eq!(outcome.command_status, EVIM_COMMAND_STATUS_READ_ONLY);
    assert_eq!(effects, 0);
    host_chars(&core, view, ":w!");
    let (status, outcome, effects) = host_key(&core, view, key(EVIM_KEY_ENTER, 0), &[]);
    assert_eq!(status, EvimStatus::Ok);
    assert_eq!(outcome.command_status, EVIM_COMMAND_STATUS_COMPLETE);
    assert_ne!(effects, 0);
    assert_eq!(evim_effect_batch_release(effects), EvimStatus::Ok);
}

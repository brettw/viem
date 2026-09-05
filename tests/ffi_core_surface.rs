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
    saw_complete_request: bool,
    saw_crossing_cluster_tail: bool,
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
            saw_complete_request: false,
            saw_crossing_cluster_tail: false,
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
    let mut revision = u64::MAX;
    // A same-core call must acquire the registry immediately and report Busy;
    // deadlock here would prove the callback was made under the registry lock.
    context.reentrant_status = unsafe { evim_core_revision(context.core, &mut revision) } as u32;
    context.responses.clear();
    for request in requests {
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
fn viewport_origin_api_is_horizontal_exact_and_vertical_atomic_limitation() {
    let core = create_core(b"WWWWWWWWWWWWWWWWWWWWWWWW", EvimDocumentOptions::default());
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
    assert_ne!(state.flags & EVIM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT, 0);
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
    assert!(state.maximum_left >= 40.0);

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
        top: 10.0,
        ..request
    };
    assert_eq!(
        unsafe { evim_core_view_set_viewport_origin(core.handle, view, &vertical, &mut outcome) },
        EvimStatus::VerticalViewportOriginUnsupported
    );
    assert_eq!(
        unsafe { evim_core_view_viewport_state(core.handle, view, &mut state) },
        EvimStatus::Ok
    );
    assert_eq!(state.left, 40.0, "combined failure changes neither axis");
    assert_eq!(state.top, 0.0);
    assert_eq!(context.shape_calls, shape_calls);

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
_Static_assert(EVIM_CORE_ABI_VERSION == {abi}, "ABI version");
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
_Static_assert(EVIM_VIEWPORT_ORIGIN_HAS_TOP == (1u << 0),
    "viewport top request flag");
_Static_assert(EVIM_VIEWPORT_STATE_WRAP == (1u << 0), "viewport wrap flag");
_Static_assert(EVIM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT == (1u << 1),
    "viewport maximum-left exact flag");
_Static_assert(EVIM_VIEWPORT_STATE_TOP_EXACT == (1u << 2),
    "viewport top exact flag");
_Static_assert(EVIM_VIEWPORT_STATE_HAS_LAYOUT == (1u << 3),
    "viewport layout identity flag");
_Static_assert(sizeof(EvimDocumentOptions) == {document_options}, "document options");
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
_Static_assert(sizeof(EvimKeyInputV1) == {key}, "key");
_Static_assert(sizeof(EvimCompositionBeginV1) == {composition_begin}, "composition begin");
_Static_assert(sizeof(EvimCompositionUpdateV1) == {composition_update}, "composition update");
_Static_assert(sizeof(EvimCompositionCommitV1) == {composition_commit}, "composition commit");
_Static_assert(sizeof(EvimCompositionCancelV1) == {composition_cancel}, "composition cancel");
_Static_assert(sizeof(EvimCoreOutcomeV1) == {outcome}, "outcome");
static void typecheck(void) {{
  EvimShapeRequestV1 request = {{0}};
  request.reserved = 0;
  request.paragraph_base_direction = EVIM_TEXT_DIRECTION_AUTO;
  EvimStatus (*create_core)(const uint8_t *, uint64_t,
      const EvimDocumentOptions *, EvimCoreHandle *, uint64_t *) = evim_core_create;
  EvimStatus (*add_view)(EvimCoreHandle, const EvimViewOptionsV1 *,
      const EvimTextMeasurementProviderV1 *, EvimViewId *,
      EvimCoreOutcomeV1 *) = evim_core_view_add;
  EvimStatus (*viewport_state)(EvimCoreHandle, EvimViewId,
      EvimViewportStateV1 *) = evim_core_view_viewport_state;
  EvimStatus (*set_viewport_origin)(EvimCoreHandle, EvimViewId,
      const EvimViewportOriginV1 *, EvimCoreOutcomeV1 *) =
      evim_core_view_set_viewport_origin;
  EvimStatus (*send_key)(EvimCoreHandle, EvimViewId,
      const EvimKeyInputV1 *, EvimCoreOutcomeV1 *) = evim_core_view_send_key;
  EvimStatus (*send_text)(EvimCoreHandle, EvimViewId, const uint8_t *,
      uint64_t, EvimCoreOutcomeV1 *) = evim_core_view_send_text;
  EvimStatus (*composition_begin)(EvimCoreHandle, EvimViewId,
      const EvimCompositionBeginV1 *, EvimCoreOutcomeV1 *) =
      evim_core_view_composition_begin;
  EvimStatus (*composition_update)(EvimCoreHandle, EvimViewId,
      const EvimCompositionUpdateV1 *, EvimCoreOutcomeV1 *) =
      evim_core_view_composition_update;
  EvimStatus (*composition_commit)(EvimCoreHandle, EvimViewId,
      const EvimCompositionCommitV1 *, EvimCoreOutcomeV1 *) =
      evim_core_view_composition_commit;
  EvimStatus (*composition_cancel)(EvimCoreHandle, EvimViewId,
      const EvimCompositionCancelV1 *, EvimCoreOutcomeV1 *) =
      evim_core_view_composition_cancel;
  (void)request; (void)create_core; (void)add_view; (void)viewport_state;
  (void)set_viewport_origin; (void)send_key; (void)send_text;
  (void)composition_begin; (void)composition_update;
  (void)composition_commit; (void)composition_cancel;
}}
"#,
        abi = EVIM_CORE_ABI_VERSION,
        document_options = std::mem::size_of::<EvimDocumentOptions>(),
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
        key = std::mem::size_of::<EvimKeyInputV1>(),
        composition_begin = std::mem::size_of::<EvimCompositionBeginV1>(),
        composition_update = std::mem::size_of::<EvimCompositionUpdateV1>(),
        composition_commit = std::mem::size_of::<EvimCompositionCommitV1>(),
        composition_cancel = std::mem::size_of::<EvimCompositionCancelV1>(),
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

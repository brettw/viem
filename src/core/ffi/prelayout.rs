//! Detached immutable layout work. Registry locks never span provider calls.
use super::*;
use crate::layout::{compute_layout_job, LayoutEngine, LayoutJobCandidate, LayoutJobRequest};
use std::sync::atomic::{AtomicBool, Ordering};

struct Request {
    core: ViemCoreHandle,
    view: ViewId,
    input: LayoutJobRequest,
    started: AtomicBool,
}
enum Work {
    Request(Arc<Request>),
    Result { core: ViemCoreHandle, view: ViewId, candidate: LayoutJobCandidate },
}
struct Registry { next: u64, entries: HashMap<u64, Work> }
static WORK: OnceLock<Mutex<Registry>> = OnceLock::new();
fn registry() -> &'static Mutex<Registry> {
    WORK.get_or_init(|| Mutex::new(Registry { next: 1, entries: HashMap::new() }))
}
fn register(work: Work) -> Result<u64, ViemStatus> {
    let mut registry = registry().lock().map_err(|_| ViemStatus::InternalError)?;
    let id = registry.next;
    if id == 0 { return Err(ViemStatus::ResourceExhausted); }
    registry.next = id.checked_add(1).unwrap_or(0);
    registry.entries.insert(id, work);
    Ok(id)
}
fn request(id: u64) -> Result<Arc<Request>, ViemStatus> {
    let registry = registry().lock().map_err(|_| ViemStatus::InternalError)?;
    match registry.entries.get(&id) {
        Some(Work::Request(request)) => Ok(Arc::clone(request)),
        _ => Err(ViemStatus::InvalidHandle),
    }
}

/// UI/coordinator turn. Zero output means the bounded pre-layout band is ready.
/// # Safety
/// `out_request` must identify one writable u64; direction is -1 or +1.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_prepare_prelayout(
    core: ViemCoreHandle, view: ViemViewId, direction: i32, out_request: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_request, 1)?;
        unsafe { out_request.write(0) };
        if !matches!(direction, -1 | 1) { return Err(ViemStatus::InvalidArgument); }
        let input = with_core_mut(core, |core| core.prepare_view_prelayout(ViewId(view), direction > 0).map_err(core_status))?;
        if let Some(input) = input {
            let cancellation = input.cancellation_token();
            let result = register(Work::Request(Arc::new(Request { core, view: ViewId(view), input, started: AtomicBool::new(false) })));
            if result.is_err() { cancellation.cancel(); }
            unsafe { out_request.write(result?) };
        }
        Ok(())
    })
}

/// Worker-only computation with a compatible independently owned provider.
/// No mutable core/view is accessed. A request can be computed once. Zero output
/// means it was cancelled. Release both request and returned result explicitly.
/// # Safety
/// Provider callbacks/context remain alive for this call; out_result is writable.
#[no_mangle]
pub unsafe extern "C" fn viem_layout_work_compute(
    id: u64, provider: *const ViemTextMeasurementProviderV1, out_result: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_result, 1)?;
        unsafe { out_result.write(0) };
        let provider = unsafe { CTextMeasurementProvider::from_ffi(provider) }?;
        let request = request(id)?;
        if provider.threading != ProviderThreading::AnyWorker { return Err(ViemStatus::InvalidProvider); }
        if request.started.swap(true, Ordering::AcqRel) { return Err(ViemStatus::InvalidArgument); }
        let mut engine = LayoutEngine::new(provider);
        // Installed geometry owns its leases; speculative work needs no second
        // long-lived shaping cache alongside the view's existing bounded cache.
        engine.set_cache_capacity(0);
        let candidate = match compute_layout_job(&mut engine, &request.input, LayoutExecutionContext::WorkerPool) {
            Ok(candidate) => candidate,
            Err(LayoutJobError::Cancelled) => return Ok(()),
            Err(error) => return Err(core_status(CoreError::LayoutJob(error))),
        };
        let result = register(Work::Result { core: request.core, view: request.view, candidate })?;
        unsafe { out_result.write(result) };
        Ok(())
    })
}

/// UI/coordinator installation; consumes a valid result even on stale rejection.
/// out_installed is zero when obsolete work was discarded, without UI changes.
/// # Safety
/// out_installed must identify one writable byte.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_install_prelayout(
    core: ViemCoreHandle, view: ViemViewId, direction: i32, id: u64, out_installed: *mut u8,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_installed, 1)?;
        unsafe { out_installed.write(0) };
        if !matches!(direction, -1 | 1) { return Err(ViemStatus::InvalidArgument); }
        let work = {
            let mut registry = registry().lock().map_err(|_| ViemStatus::InternalError)?;
            match registry.entries.get(&id) {
                Some(Work::Result { core: owner, view: owner_view, .. }) if *owner == core && owner_view.0 == view => (),
                _ => return Err(ViemStatus::InvalidHandle),
            }
            registry.entries.remove(&id).expect("validated result")
        };
        let Work::Result { candidate, .. } = work else { unreachable!() };
        let installed = with_core_mut(core, |core| core.install_view_prelayout(ViewId(view), direction > 0, candidate).map_err(core_status))?;
        unsafe { out_installed.write(u8::from(installed)) };
        Ok(())
    })
}

/// Cancellation is nonblocking and may race with worker computation.
#[no_mangle]
pub extern "C" fn viem_layout_work_cancel(id: u64) -> ViemStatus {
    ffi_boundary(|| { request(id)?.input.cancellation_token().cancel(); Ok(()) })
}

/// Release from any thread. Releasing a request also cancels it. An in-flight
/// computation retains only its own immutable inputs until its next checkpoint.
#[no_mangle]
pub extern "C" fn viem_layout_work_release(id: u64) -> ViemStatus {
    ffi_boundary(|| {
        let work = registry().lock().map_err(|_| ViemStatus::InternalError)?.entries.remove(&id)
            .ok_or(ViemStatus::InvalidHandle)?;
        if let Work::Request(request) = &work { request.input.cancellation_token().cancel(); }
        // Native lease release occurs after leaving the registry lock.
        drop(work);
        Ok(())
    })
}

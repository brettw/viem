use super::*;
use crate::ffi::*;

fn table(storage: &mut PaintTestProviderStorage) -> ViemTextMeasurementProviderV1 {
    ViemTextMeasurementProviderV1 {
        struct_size: VIEM_TEXT_MEASUREMENT_PROVIDER_V1_SIZE,
        abi_version: VIEM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION,
        context: storage as *mut _ as *mut c_void,
        measurement_environment_id: 501,
        threading: VIEM_PROVIDER_THREADING_ANY_WORKER,
        has_render_run_policy: 1, render_run_owner: 601,
        render_run_threading: VIEM_RENDER_THREADING_ANY, reserved: 0,
        metrics_generation: Some(paint_test_metrics_generation),
        shape_batch: Some(paint_test_shape_batch),
        retain_render_runs: Some(paint_test_retain_render_runs),
        release_render_runs: Some(paint_test_release_render_runs),
    }
}

#[test]
fn prelayout_ffi_validates_ownership_and_detaches_work_from_busy_core() {
    let mut storage = PaintTestProviderStorage::default();
    let mut core = Core::new(Document::new("paragraph\n".repeat(10_000)));
    // Leave more than one bounded chunk in the speculative band so the
    // cancellation checks below always receive another pending request.
    let view = core.add_view(paint_test_provider(&mut storage, 501, 601), 240.0, 480.0);
    let handle = register_core(core).unwrap();
    let mut request = 99;
    unsafe {
        assert_eq!(viem_core_view_prepare_prelayout(handle, view.0, 0, &mut request), ViemStatus::InvalidArgument);
        assert_eq!(request, 0);
        assert_eq!(viem_core_view_prepare_prelayout(handle, view.0, 1, ptr::null_mut()), ViemStatus::NullPointer);
        assert_eq!(viem_core_view_prepare_prelayout(handle, view.0, 1, &mut request), ViemStatus::Ok);
    }
    assert_ne!(request, 0);
    // A checked-out core remains busy during computation: the worker must use
    // its immutable request, not look up mutable document/view state.
    let busy = checkout_core(handle).unwrap();
    let result = std::thread::spawn(move || {
        let mut storage = PaintTestProviderStorage::default();
        let mut provider = table(&mut storage);
        let mut result = 0;
        unsafe {
            provider.threading = VIEM_PROVIDER_THREADING_FRONTEND_MAIN;
            assert_eq!(viem_layout_work_compute(request, &provider, &mut result), ViemStatus::InvalidProvider);
            provider.threading = VIEM_PROVIDER_THREADING_ANY_WORKER;
            assert_eq!(viem_layout_work_compute(request, &provider, &mut result), ViemStatus::Ok);
            let mut duplicate = 99;
            assert_eq!(viem_layout_work_compute(request, &provider, &mut duplicate), ViemStatus::InvalidArgument);
            assert_eq!(duplicate, 0);
        }
        result
    }).join().unwrap();
    drop(busy);
    assert_ne!(result, 0);
    let mut installed = 99;
    unsafe {
        assert_eq!(viem_core_view_install_prelayout(handle, view.0 + 1, 1, result, &mut installed), ViemStatus::InvalidHandle);
        assert_eq!(installed, 0);
        assert_eq!(viem_core_view_install_prelayout(handle, view.0, 1, result, &mut installed), ViemStatus::Ok);
        assert_eq!(installed, 1);
    }
    assert_eq!(viem_layout_work_release(result), ViemStatus::InvalidHandle);
    assert_eq!(viem_layout_work_release(request), ViemStatus::Ok);
    unsafe { assert_eq!(viem_core_view_prepare_prelayout(handle, view.0, 1, &mut request), ViemStatus::Ok); }
    assert_ne!(request, 0);
    assert_eq!(viem_layout_work_cancel(request), ViemStatus::Ok);
    let mut worker_storage = PaintTestProviderStorage::default();
    let provider = table(&mut worker_storage);
    let mut cancelled = 99;
    unsafe { assert_eq!(viem_layout_work_compute(request, &provider, &mut cancelled), ViemStatus::Ok); }
    assert_eq!(cancelled, 0);
    assert_eq!(viem_layout_work_release(request), ViemStatus::Ok);
    assert_eq!(viem_layout_work_release(request), ViemStatus::InvalidHandle);
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
}

use super::*;
use crate::ffi::{viem_core_view_capture_restoration, viem_core_view_restore,
    ViemViewRestorationV1, VIEM_VIEW_RESTORATION_V1_SIZE};

#[test]
fn restoration_abi_validates_identity_geometry_and_memory_before_mutation() {
    let mut storage = PaintTestProviderStorage::default();
    let mut core = Core::new(Document::new("first\nsecond"));
    let view = core.add_view(paint_test_provider(&mut storage, 591, 691), 240.0, 80.0);
    let document = core.document().id().0;
    let revision = core.document().revision().0;
    let handle = register_core(core).unwrap();
    let mut original = ViemViewRestorationV1::default();
    assert_eq!(unsafe { viem_core_view_capture_restoration(handle, view.0, &mut original) }, ViemStatus::Ok);
    assert_eq!(original.struct_size, VIEM_VIEW_RESTORATION_V1_SIZE);
    assert_eq!(original.cursor_affinity, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM);
    let mut outcome = ViemCoreOutcomeV1::default();
    let mut request = original;
    request.cursor_line = 1;
    request.cursor_column = u64::MAX;
    for (requested_document, requested_revision, expected) in [
        (document + 1, revision, ViemStatus::InvalidArgument),
        (document, revision + 1, ViemStatus::StaleRevision),
    ] {
        assert_eq!(unsafe { viem_core_view_restore(handle, view.0, requested_document,
            requested_revision, &request, &mut outcome) }, expected);
    }
    for invalid in [
        ViemViewRestorationV1 { cursor_affinity: 999, ..request },
        ViemViewRestorationV1 { row_fraction: f32::NAN, ..request },
        ViemViewRestorationV1 { left: f32::INFINITY, ..request },
        ViemViewRestorationV1 { struct_size: 0, ..request },
    ] {
        assert_ne!(unsafe { viem_core_view_restore(handle, view.0, document, revision,
            &invalid, &mut outcome) }, ViemStatus::Ok);
    }
    assert_eq!(unsafe { viem_core_view_restore(handle, view.0, document, revision,
        &request, ptr::null_mut()) }, ViemStatus::NullPointer);
    let mut overlapping = [0u64; 64];
    let state = overlapping.as_mut_ptr().cast::<ViemViewRestorationV1>();
    unsafe { state.write(request) };
    assert_eq!(unsafe { viem_core_view_restore(handle, view.0, document, revision,
        state, state.cast::<ViemCoreOutcomeV1>()) }, ViemStatus::InvalidArgument);
    let mut after = ViemViewRestorationV1::default();
    assert_eq!(unsafe { viem_core_view_capture_restoration(handle, view.0, &mut after) }, ViemStatus::Ok);
    assert_eq!((after.cursor_line, after.cursor_column, after.viewport_line, after.row_fraction),
        (original.cursor_line, original.cursor_column, original.viewport_line, original.row_fraction));
    assert_eq!(unsafe { viem_core_view_restore(handle, view.0, document, revision,
        &request, &mut outcome) }, ViemStatus::Ok);
    assert_eq!(checkout_core(handle).unwrap().core().command_state(view).unwrap().cursor(), 11);
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
}

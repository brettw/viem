//! Bounded, view-local search presentation work driven by the native run loop.
use super::*;

/// Advance one cooperative slice before reading the resulting presentation.
/// This never edits source or accepts an incremental-search prompt.
///
/// # Safety
/// `out_changed` must identify one writable byte.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_poll_search(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_changed: *mut u8,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_changed, 1)?;
        unsafe { out_changed.write(0) };
        let changed = with_core_mut(handle, |core| {
            core.poll_search(ViewId(view)).map_err(core_status)
        })?;
        unsafe { out_changed.write(u8::from(changed)) };
        Ok(())
    })
}

/// Query whether another bounded search slice is needed without advancing it.
///
/// # Safety
/// `out_pending` must identify one writable byte.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_search_work_pending(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_pending: *mut u8,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_pending, 1)?;
        unsafe { out_pending.write(0) };
        let pending = with_core(handle, |core| {
            core.search_work_pending(ViewId(view)).map_err(core_status)
        })?;
        unsafe { out_pending.write(u8::from(pending)) };
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_polling_validates_outputs_and_clears_failure_results() {
        let core =
            register_core(Core::<CTextMeasurementProvider>::new(Document::new("text"))).unwrap();
        for api in [
            viem_core_view_poll_search,
            viem_core_view_search_work_pending,
        ] {
            assert_eq!(
                unsafe { api(core, 0, std::ptr::null_mut()) },
                ViemStatus::NullPointer
            );
            let mut value = 99;
            assert_eq!(unsafe { api(0, 0, &mut value) }, ViemStatus::InvalidHandle);
            assert_eq!(value, 0);
            value = 99;
            assert_eq!(unsafe { api(core, 0, &mut value) }, ViemStatus::InvalidView);
            assert_eq!(value, 0);
        }
        assert_eq!(viem_core_destroy(core), ViemStatus::Ok);
    }
}

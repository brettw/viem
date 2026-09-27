//! Owned export bytes decouple long-running serialization from the live editor.
use super::*;

pub type ViemHtmlExportHandle = u64;

struct ExportRegistry {
    next_handle: ViemHtmlExportHandle,
    exports: HashMap<ViemHtmlExportHandle, ExportState>,
}

enum ExportState {
    Prepared(crate::document::HtmlExport),
    Rendering,
    Ready(Arc<[u8]>),
    Failed,
}

fn registry() -> &'static Mutex<ExportRegistry> {
    static EXPORTS: OnceLock<Mutex<ExportRegistry>> = OnceLock::new();
    EXPORTS.get_or_init(|| {
        Mutex::new(ExportRegistry {
            next_handle: 1,
            exports: HashMap::new(),
        })
    })
}

/// Capture without modifying the editing session or invoking a provider. Call
/// alongside other frontend core operations, then render the handle on a worker.
///
/// # Safety
/// `out_export` must point to one aligned, writable handle.
#[no_mangle]
pub unsafe extern "C" fn viem_core_prepare_html_export(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected_revision: u64,
    out_export: *mut ViemHtmlExportHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_export, 1)?;
        unsafe { out_export.write(0) };
        let export = with_core(handle, |core| {
            validate_revision(core.document(), expected_revision)?;
            core.prepare_html_export(ViewId(view)).map_err(core_status)
        })?;
        let mut registry = registry().lock().map_err(|_| ViemStatus::InternalError)?;
        let handle = registry.next_handle;
        if handle == 0 {
            return Err(ViemStatus::ResourceExhausted);
        }
        registry.next_handle = handle.checked_add(1).unwrap_or(0);
        registry
            .exports
            .insert(handle, ExportState::Prepared(export));
        unsafe { out_export.write(handle) };
        Ok(())
    })
}

/// Render a prepared immutable snapshot once. Never accesses the live core or
/// its measurement provider, so editing can continue while syntax work runs.
/// The handle must be released after success or failure.
#[no_mangle]
pub extern "C" fn viem_html_export_render(handle: ViemHtmlExportHandle) -> ViemStatus {
    ffi_boundary(|| {
        let export = {
            let mut registry = registry().lock().map_err(|_| ViemStatus::InternalError)?;
            let entry = registry
                .exports
                .get_mut(&handle)
                .ok_or(ViemStatus::InvalidHandle)?;
            if !matches!(entry, ExportState::Prepared(_)) {
                return Err(ViemStatus::InvalidArgument);
            }
            match std::mem::replace(entry, ExportState::Rendering) {
                ExportState::Prepared(export) => export,
                _ => unreachable!(),
            }
        };
        let result = export.render().map_err(|error| match error {
            crate::document::HtmlExportError::Document(error) => core_status(error.into()),
            crate::document::HtmlExportError::Style(error) => {
                core_status(LayoutError::from(error).into())
            }
        });
        let mut registry = registry().lock().map_err(|_| ViemStatus::InternalError)?;
        let entry = registry
            .exports
            .get_mut(&handle)
            .ok_or(ViemStatus::InvalidHandle)?;
        match result {
            Ok(bytes) => {
                *entry = ExportState::Ready(bytes.into());
                Ok(())
            }
            Err(error) => {
                *entry = ExportState::Failed;
                Err(error)
            }
        }
    })
}

/// Copy immutable export bytes. Short buffers are not partially written.
///
/// # Safety
/// `out_required` is aligned and writable. Nonzero capacity requires writable
/// output storage of that size, disjoint from `out_required`.
#[no_mangle]
pub unsafe extern "C" fn viem_html_export_copy_utf8(
    handle: ViemHtmlExportHandle,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        let capacity = checked_length(output_capacity)?;
        validate_disjoint_regions(&[
            typed_pointer_region(output, output_capacity)?,
            typed_pointer_region(out_required, 1)?,
        ])?;
        unsafe { out_required.write(0) };
        let bytes = {
            let registry = registry().lock().map_err(|_| ViemStatus::InternalError)?;
            match registry.exports.get(&handle) {
                Some(ExportState::Ready(bytes)) => bytes.clone(),
                Some(_) => return Err(ViemStatus::InvalidArgument),
                None => return Err(ViemStatus::InvalidHandle),
            }
        };
        unsafe { out_required.write(checked_export_count(bytes.len())?) };
        if capacity < bytes.len() {
            return Err(ViemStatus::BufferTooSmall);
        }
        if !bytes.is_empty() {
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len()) };
        }
        Ok(())
    })
}

/// Release an owned result. Concurrent copies retain their own immutable bytes.
#[no_mangle]
pub extern "C" fn viem_html_export_release(handle: ViemHtmlExportHandle) -> ViemStatus {
    ffi_boundary(|| {
        let bytes = registry()
            .lock()
            .map_err(|_| ViemStatus::InternalError)?
            .exports
            .remove(&handle)
            .ok_or(ViemStatus::InvalidHandle)?;
        // Large results are dropped after releasing the registry lock.
        drop(bytes);
        Ok(())
    })
}

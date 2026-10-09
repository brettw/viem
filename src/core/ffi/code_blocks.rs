//! Code-block language authoring names an exact snapshot and paragraph.
use super::*;

/// # Safety
/// Input text and outcome storage must be valid, aligned and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_set_code_block_language(
    handle: ViemCoreHandle,
    view: ViemViewId,
    document: u64,
    revision: u64,
    offset: u64,
    language: ViemUtf8Slice,
    output: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(language.data, language.length)?,
            typed_pointer_region(output, 1)?,
        ])?;
        if language.length > 128 {
            return Err(ViemStatus::InvalidArgument);
        }
        let language = str::from_utf8(unsafe { input_bytes(language.data, language.length)? })
            .map_err(|_| ViemStatus::InvalidUtf8)?;
        let offset = usize::try_from(offset).map_err(|_| ViemStatus::InvalidArgument)?;
        unsafe {
            clear_outcome(output)?;
        }
        let result = with_core_mut(handle, |core| {
            validate_revision(core.document(), revision)?;
            let outcome = core
                .set_markdown_code_language(
                    ViewId(view),
                    DocumentId(document),
                    Revision(revision),
                    offset,
                    (!language.is_empty()).then_some(language),
                )
                .map_err(core_status)?;
            summarize_core_outcome(core, ViewId(view), Some(&outcome))
        })?;
        unsafe {
            output.write(result);
        }
        Ok(())
    })
}

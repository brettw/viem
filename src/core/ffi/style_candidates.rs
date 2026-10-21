use super::*;
/// Return UTF-8 JSON stable IDs for legal local parents in this exact stylesheet.
/// # Safety
/// Input record/string and output regions are valid and pairwise disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_style_parent_candidates(
    handle: ViemCoreHandle,
    expected: *const ViemStyleSheetIdentityV1,
    namespace: u32,
    child: ViemUtf8Slice,
    output: *mut u8,
    capacity: u64,
    required: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(expected, 1)?,
            typed_pointer_region(child.data, child.length)?,
            typed_pointer_region(output, capacity)?,
            typed_pointer_region(required, 1)?,
        ])?;
        unsafe { required.write(0) };
        let expected = unsafe { *expected };
        let namespace = parse_style_namespace(namespace)?;
        let child = StyleId(
            std::str::from_utf8(unsafe { input_bytes(child.data, child.length)? })
                .map_err(|_| ViemStatus::InvalidUtf8)?
                .into(),
        );
        let parents = if expected.document_id == 0 {
            let sheet = crate::document::code_style::snapshot();
            validate_code_style_identity(expected, &sheet)?;
            sheet.local_parent_candidates(namespace, &child)
        } else {
            with_core_mut(handle, |core| {
                validate_style_sheet_identity(expected, core.document())?;
                Ok(core
                    .document()
                    .projection()
                    .style_sheet()
                    .local_parent_candidates(namespace, &child))
            })?
        };
        let bytes = serde_json::to_vec(&parents.iter().map(|id| id.0.as_str()).collect::<Vec<_>>())
            .map_err(|_| ViemStatus::InternalError)?;
        unsafe { copy_clipboard_json_bytes(&bytes, output, capacity, required) }
    })
}

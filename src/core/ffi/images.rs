//! Bounded image popup queries and exact-selection authoring.
use super::*;
use crate::document::ImageEditIntent;

/// # Safety
/// Output and required-length storage must be valid, aligned and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_copy_image_context(
    handle: ViemCoreHandle,
    view: ViemViewId,
    output: *mut u8,
    capacity: u64,
    required: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(output, capacity)?,
            typed_pointer_region(required, 1)?,
        ])?;
        let bytes = with_core(handle, |core| {
            let selection = core
                .list_selection_identity(ViewId(view))
                .map_err(core_status)?;
            let identity = logical_selection_identity_to_ffi(&selection)?;
            let document = core.document();
            let range = selection.range();
            let cursor = core
                .command_state(ViewId(view))
                .ok_or(ViemStatus::InvalidView)?
                .cursor();
            let image = document
                // Native selections are half-open and may leave their active
                // caret just after the image. Sample selected content so both
                // selection directions identify the same atomic image.
                .image_snapshot_at(document.text_point(if range.is_empty() { cursor } else { range.start })
                    .map_err(document_status)?)
                .map_err(document_status)?
                .filter(|image| {
                    range.is_empty()
                        || (image.range.start <= range.start && range.end <= image.range.end)
                });
            let linear = matches!(
                selection.kind(),
                LogicalSelectionKind::None
                    | LogicalSelectionKind::Character
                    | LogicalSelectionKind::Line
            );
            let text = if linear && range.len() <= 32 * 1024 {
                document
                    .projection()
                    .text_tree()
                    .slice(range.clone())
                    .map_err(|_| ViemStatus::CoreFailure)?
            } else {
                String::new()
            };
            serde_json::to_vec(&serde_json::json!({
                "selection": { "viewId": view, "documentId": document.id().0, "revision": document.revision().0,
                    "start": range.start, "end": range.end, "kind": identity.kind, "anchor": selection.anchor(), "active": selection.active(),
                    "affinity": if selection.active_affinity() == BoundaryAffinity::Upstream { 0 } else { 1 } },
                "canInsert": linear && document.can_insert_image(range), "text": text,
                "image": image.map(|image| serde_json::json!({"start":image.range.start,"end":image.range.end,"text":image.text,"destination":image.destination,"editable":image.editable && !document.is_read_only()}))
            })).map_err(|_| ViemStatus::CoreFailure)
        })?;
        unsafe {
            required.write(bytes.len() as u64);
        }
        if capacity < bytes.len() as u64 {
            return Err(ViemStatus::BufferTooSmall);
        }
        unsafe {
            copy_output(&bytes, output);
        }
        Ok(())
    })
}

/// # Safety
/// Input slices, expected selection and output must be valid and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_edit_image(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemLogicalSelectionIdentityV1,
    action: u32,
    image_start: u64,
    image_end: u64,
    text: ViemUtf8Slice,
    destination: ViemUtf8Slice,
    output: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(expected, 1)?,
            typed_pointer_region(output, 1)?,
            typed_pointer_region(text.data, text.length)?,
            typed_pointer_region(destination.data, destination.length)?,
        ])?;
        let expected = unsafe { expected.read() };
        if action > 2 || text.length.saturating_add(destination.length) > 32 * 1024 {
            return Err(ViemStatus::InvalidArgument);
        }
        let text = str::from_utf8(unsafe { input_bytes(text.data, text.length)? })
            .map_err(|_| ViemStatus::InvalidUtf8)?
            .to_owned();
        let destination =
            str::from_utf8(unsafe { input_bytes(destination.data, destination.length)? })
                .map_err(|_| ViemStatus::InvalidUtf8)?
                .to_owned();
        unsafe {
            clear_outcome(output)?;
        }
        let result = with_core_mut(handle, |core| {
            let selection = core
                .list_selection_identity(ViewId(view))
                .map_err(core_status)?;
            validate_logical_selection_identity(
                expected,
                logical_selection_identity_to_ffi(&selection)?,
            )?;
            if !matches!(
                selection.kind(),
                LogicalSelectionKind::None
                    | LogicalSelectionKind::Character
                    | LogicalSelectionKind::Line
            ) {
                return Err(ViemStatus::UnsupportedOperation);
            }
            let range = usize::try_from(image_start).map_err(|_| ViemStatus::LengthOverflow)?
                ..usize::try_from(image_end).map_err(|_| ViemStatus::LengthOverflow)?;
            let intent = match action {
                0 => ImageEditIntent::Insert {
                    range: selection.range(),
                    text,
                    destination,
                },
                1 => ImageEditIntent::Edit {
                    range,
                    text,
                    destination,
                },
                _ => ImageEditIntent::Remove { range },
            };
            let outcome = core
                .edit_image(ViewId(view), selection, intent)
                .map_err(core_status)?;
            summarize_core_outcome(core, ViewId(view), Some(&outcome))
        })?;
        unsafe {
            output.write(result);
        }
        Ok(())
    })
}

/// # Safety
/// `output` must be valid, aligned writable outcome storage.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_select_image(
    handle: ViemCoreHandle,
    view: ViemViewId,
    document: u64,
    revision: u64,
    offset: u64,
    output: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        unsafe {
            clear_outcome(output)?;
        }
        let result = with_core_mut(handle, |core| {
            let outcome = core
                .select_image(
                    ViewId(view),
                    DocumentId(document),
                    Revision(revision),
                    checked_length(offset)?,
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

/// Retire image render resources while preserving learned scroll extents.
///
/// # Safety
/// `destinations` supplies `count` readable UTF-8 slices until return. Empty
/// means pixels/status changed while every intrinsic dimension stayed equal.
#[no_mangle]
pub unsafe extern "C" fn viem_core_image_resources_changed(
    handle: ViemCoreHandle,
    view: ViemViewId,
    previous_metrics_generation: u64,
    destinations: *const ViemUtf8Slice,
    count: u64,
) -> ViemStatus {
    ffi_boundary(|| {
        if count > 256 { return Err(ViemStatus::InvalidArgument); }
        typed_pointer_region(destinations, count)?;
        let values = if count == 0 { &[][..] } else {
            unsafe { slice::from_raw_parts(destinations, checked_length(count)?) }
        };
        let mut total = 0u64;
        for value in values {
            total = total.checked_add(value.length).ok_or(ViemStatus::InvalidArgument)?;
            if total > 1024 * 1024 { return Err(ViemStatus::InvalidArgument); }
            str::from_utf8(unsafe { input_bytes(value.data, value.length)? })
                .map_err(|_| ViemStatus::InvalidUtf8)?;
        }
        with_core_mut(handle, |core| core.image_resources_changed(
            ViewId(view), MetricsGeneration(previous_metrics_generation), !values.is_empty(),
        ).map_err(core_status))
    })
}

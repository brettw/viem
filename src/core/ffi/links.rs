//! Bounded link popup queries and exact-selection authoring.
use super::*;
use crate::document::{LinkEditIntent, StyleApplication};

/// # Safety
/// Output and required-length storage must be valid, aligned and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_copy_link_context(
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
            let commands = core
                .command_state(ViewId(view))
                .ok_or(ViemStatus::InvalidView)?;
            let affinity = if commands.mode() == Mode::Normal {
                BoundaryAffinity::Downstream
            } else {
                commands.insertion_boundary_affinity()
            };
            let sample = if range.is_empty() && affinity == BoundaryAffinity::Upstream && cursor > 0
            {
                document
                    .hard_line_snapshot()
                    .previous_grapheme_boundary(cursor)
                    .unwrap_or(cursor)
            } else if range.is_empty() {
                cursor
            } else {
                range.start
            };
            let linked = document
                .projection()
                .style_spans_for_region(&(sample..sample.saturating_add(1)))
                .iter()
                .any(|span| {
                    span.application == StyleApplication::Automatic("Link".into())
                        && span.range.contains(&sample)
                        && (range.is_empty() || range.end <= span.range.end)
                });
            let link = document
                .link_at_boundary(
                    if range.is_empty() {
                        cursor
                    } else {
                        range.start
                    },
                    if range.is_empty() {
                        affinity
                    } else {
                        BoundaryAffinity::Downstream
                    },
                )
                .map_err(document_status)?
                .filter(|link| {
                    range.is_empty()
                        || (link.range.start <= range.start && range.end <= link.range.end)
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
                "canInsert": linear && (document.can_insert_link(range.clone()) || range.is_empty() && commands.typing_link_disabled()
                    && commands.typing_named_style().is_none_or(|style| !document.character_style_is_code(style)) && document.can_exit_link_typing(cursor, affinity)), "text": text,
                "linked": linked && (!range.is_empty() || !commands.typing_link_disabled()),
                "canExitLink": range.is_empty() && !commands.typing_link_disabled() && document.can_exit_link_typing(cursor, affinity),
                "canRemoveSelection": linear && document.can_remove_link_selection(range),
                "link": link.map(|link| serde_json::json!({"start":link.range.start,"end":link.range.end,"text":link.text,"destination":link.destination,"editable":link.editable && !document.is_read_only()}))
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
pub unsafe extern "C" fn viem_core_view_edit_link(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemLogicalSelectionIdentityV1,
    action: u32,
    link_start: u64,
    link_end: u64,
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
        if action > 4 || text.length.saturating_add(destination.length) > 32 * 1024 {
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
            let range = usize::try_from(link_start).map_err(|_| ViemStatus::LengthOverflow)?
                ..usize::try_from(link_end).map_err(|_| ViemStatus::LengthOverflow)?;
            if action == 3 {
                let outcome = core
                    .exit_link_typing(ViewId(view), selection)
                    .map_err(core_status)?;
                return summarize_core_outcome(core, ViewId(view), Some(&outcome));
            }
            let intent = match action {
                0 => LinkEditIntent::Insert {
                    range: selection.range(),
                    text,
                    destination,
                },
                1 => LinkEditIntent::Edit {
                    range,
                    text,
                    destination,
                },
                2 => LinkEditIntent::Remove { range },
                _ => LinkEditIntent::RemoveSelection {
                    range: selection.range(),
                },
            };
            let outcome = core
                .edit_link(ViewId(view), selection, intent)
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
/// All input and output regions must be valid, aligned and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_find_link_fragment(
    handle: ViemCoreHandle,
    document: u64,
    revision: u64,
    fragment: ViemUtf8Slice,
    offset: *mut u64,
    found: *mut u8,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(fragment.data, fragment.length)?,
            typed_pointer_region(offset, 1)?,
            typed_pointer_region(found, 1)?,
        ])?;
        let fragment = str::from_utf8(unsafe { input_bytes(fragment.data, fragment.length)? })
            .map_err(|_| ViemStatus::InvalidUtf8)?;
        let result = with_core(handle, |core| {
            if core.document().id().0 != document {
                return Err(ViemStatus::InvalidArgument);
            }
            validate_revision(core.document(), revision)?;
            core.document()
                .find_link_fragment(fragment)
                .map_err(document_status)
        })?;
        unsafe {
            offset.write(result.unwrap_or(0) as u64);
            found.write(u8::from(result.is_some()));
        }
        Ok(())
    })
}

//! Task-list controls use semantic context for availability and exact layout
//! identities for pointer actions. Neither path reparses source for presentation.
use super::*;

/// # Safety
/// Snapshot identity and outcome must be aligned, valid, and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_toggle_checkbox(
    handle: ViemCoreHandle, view: ViemViewId,
    expected: *const ViemLayoutSnapshotIdentityV1, paragraph_id: u64,
    output: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(expected, output)? };
        unsafe { clear_outcome(output)?; }
        let outcome = with_core_mut(handle, |core| {
            let view_id = ViewId(view);
            // IME and completion overlays have presentation coordinates; their
            // row offsets must never become base-document source-edit targets.
            if core.composition_overlay(view_id).map_err(core_status)?.is_some() {
                return Err(ViemStatus::InvalidArgument);
            }
            let snapshot = current_ffi_layout_snapshot(core, view_id)?;
            validate_snapshot_identity(request, snapshot, view_id)?;
            let at = snapshot.rows.iter().find(|row| row.decorations.iter().any(|decoration|
                matches!(decoration.kind, crate::layout::DecorationKind::Checkbox { .. })
                    && decoration.owner == Some(crate::layout::DecorationOwner::Paragraph(paragraph_id))))
                .map(|row| row.text_range.start).ok_or(ViemStatus::InvalidArgument)?;
            dispatch_event(core, view, CoreEvent::ToggleCheckbox {
                document: core.document().id(), revision: core.document().revision(), at, block_id: paragraph_id,
            })
        })?;
        unsafe { output.write(outcome); }
        Ok(())
    })
}

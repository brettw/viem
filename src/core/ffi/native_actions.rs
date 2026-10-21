use super::*;
use crate::command::{NativeAction, NativeActionKind, SelectionOrigin};
/// Bit N enables action N. Input precedence and search availability are core policy.
/// # Safety
/// `out_actions` is aligned writable storage for one u64.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_native_actions(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_actions: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_actions, 1)?;
        unsafe { out_actions.write(0) };
        let actions = with_core_mut(handle, |core| {
            let view = ViewId(view);
            let state = core.command_state(view).ok_or(ViemStatus::InvalidView)?;
            if core
                .composition_overlay(view)
                .map_err(core_status)?
                .is_some()
            {
                return Ok(0);
            }
            Ok((1..=10)
                .filter(|id| state.native_action_available(NativeActionKind::from_id(*id).unwrap()))
                .fold(0, |mask, id| mask | (1u64 << id)))
        })?;
        unsafe { out_actions.write(actions) };
        Ok(())
    })
}
/// Execute a semantic menu action. Zero return_mode uses the current mode.
/// # Safety
/// Outcome and effect-handle outputs are aligned, writable and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_native_action(
    handle: ViemCoreHandle,
    view: ViemViewId,
    kind: u32,
    origin: u32,
    return_mode: u32,
    out_outcome: *mut ViemCoreOutcomeV1,
    out_effects: *mut ViemEffectBatchHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(out_outcome, 1)?,
            typed_pointer_region(out_effects, 1)?,
        ])?;
        unsafe {
            out_outcome.write(Default::default());
            out_effects.write(0);
        }
        let kind = NativeActionKind::from_id(kind).ok_or(ViemStatus::InvalidArgument)?;
        let origin = match origin {
            1 => SelectionOrigin::Mouse,
            2 => SelectionOrigin::Key,
            3 => SelectionOrigin::Command,
            _ => return Err(ViemStatus::InvalidArgument),
        };
        let return_mode = match return_mode {
            0 => None,
            VIEM_MODE_NORMAL
            | VIEM_MODE_VISUAL_CHARACTER
            | VIEM_MODE_VISUAL_LINE
            | VIEM_MODE_VISUAL_BLOCK
            | VIEM_MODE_COMMAND_LINE
            | VIEM_MODE_SELECT_CHARACTER
            | VIEM_MODE_SELECT_LINE
            | VIEM_MODE_SELECT_BLOCK
            | VIEM_MODE_SELECTION_CHARACTER
            | VIEM_MODE_SELECTION_LINE
            | VIEM_MODE_SELECTION_BLOCK => Some(Mode::Normal),
            VIEM_MODE_INSERT => Some(Mode::Insert),
            VIEM_MODE_REPLACE => Some(Mode::Replace),
            _ => return Err(ViemStatus::InvalidArgument),
        };
        let reservation = reserve_effect_batch()?;
        let (outcome, effects) = publish_input_turn(
            handle,
            view,
            InputEvent::Key(Key::NativeAction(NativeAction {
                kind,
                origin,
                return_mode,
            })),
            ClipboardCommandContext::default(),
            reservation,
        )?;
        unsafe {
            out_outcome.write(outcome);
            out_effects.write(effects);
        }
        Ok(())
    })
}

pub const VIEM_NATIVE_SELECT_WORD: u32 = 1;
pub const VIEM_NATIVE_SELECT_SENTENCE: u32 = 2;
pub const VIEM_NATIVE_SELECT_PARAGRAPH: u32 = 3;
pub const VIEM_NATIVE_SELECT_LINE: u32 = 4;
pub const VIEM_NATIVE_SELECT_VISUAL_ROW: u32 = 5;
pub const VIEM_NATIVE_SELECT_BLOCK: u32 = 6;
pub const VIEM_NATIVE_FIND: u32 = 7;
pub const VIEM_NATIVE_FIND_REPLACE: u32 = 8;
pub const VIEM_NATIVE_FIND_NEXT: u32 = 9;
pub const VIEM_NATIVE_FIND_PREVIOUS: u32 = 10;

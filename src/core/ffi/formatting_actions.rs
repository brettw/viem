//! One shared decision for the native formatting actions that toolbars,
//! menus and accelerators offer, so frontends never derive their own.
use super::*;
use crate::FormattingActionSet;

pub const VIEM_FORMATTING_BOLD: u32 = 1;
pub const VIEM_FORMATTING_ITALIC: u32 = 2;
pub const VIEM_FORMATTING_UNDERLINE: u32 = 3;
pub const VIEM_FORMATTING_STRIKETHROUGH: u32 = 4;
pub const VIEM_FORMATTING_SUPERSCRIPT: u32 = 5;
pub const VIEM_FORMATTING_SUBSCRIPT: u32 = 6;
pub const VIEM_FORMATTING_CODE: u32 = 7;
pub const VIEM_FORMATTING_CODE_BLOCK: u32 = 8;
pub const VIEM_FORMATTING_BLOCK_QUOTE: u32 = 9;
pub const VIEM_FORMATTING_ALERT_NOTE: u32 = 10;
pub const VIEM_FORMATTING_ALERT_TIP: u32 = 11;
pub const VIEM_FORMATTING_ALERT_IMPORTANT: u32 = 12;
pub const VIEM_FORMATTING_ALERT_WARNING: u32 = 13;
pub const VIEM_FORMATTING_ALERT_CAUTION: u32 = 14;
pub const VIEM_FORMATTING_BULLETED_LIST: u32 = 15;
pub const VIEM_FORMATTING_NUMBERED_LIST: u32 = 16;
pub const VIEM_FORMATTING_TASK_LIST: u32 = 17;
pub const VIEM_FORMATTING_REMOVE_LIST: u32 = 18;
pub const VIEM_FORMATTING_INDENT: u32 = 19;
pub const VIEM_FORMATTING_UNINDENT: u32 = 20;
pub const VIEM_FORMATTING_LIST_PARAGRAPH: u32 = 21;
pub const VIEM_FORMATTING_HEADING_0: u32 = 22;
pub const VIEM_FORMATTING_HEADING_1: u32 = 23;
pub const VIEM_FORMATTING_HEADING_2: u32 = 24;
pub const VIEM_FORMATTING_HEADING_3: u32 = 25;
pub const VIEM_FORMATTING_HEADING_4: u32 = 26;
pub const VIEM_FORMATTING_HEADING_5: u32 = 27;
pub const VIEM_FORMATTING_HEADING_6: u32 = 28;
pub const VIEM_FORMATTING_LINK: u32 = 29;
pub const VIEM_FORMATTING_REFERENCE: u32 = 30;
pub const VIEM_FORMATTING_IMAGE: u32 = 31;
pub const VIEM_FORMATTING_TABLE: u32 = 32;
pub const VIEM_FORMATTING_PARAGRAPH_STYLES: u32 = 33;
pub const VIEM_FORMATTING_CHARACTER_STYLES: u32 = 34;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ViemFormattingActions {
    pub enabled: u64,
    pub on: u64,
    pub mixed: u64,
}

/// # Safety
/// `out_actions` is aligned writable storage for one value.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_formatting_actions(
    handle: ViemCoreHandle,
    view: ViemViewId,
    requested: u64,
    out_actions: *mut ViemFormattingActions,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_actions, 1)?;
        unsafe { out_actions.write(ViemFormattingActions::default()) };
        let actions = with_core(handle, |core| {
            core.formatting_actions(ViewId(view), FormattingActionSet::from_bits(requested)).map_err(core_status)
        })?;
        unsafe {
            out_actions.write(ViemFormattingActions {
                enabled: actions.enabled().bits(),
                on: actions.on().bits(),
                mixed: actions.mixed().bits(),
            })
        };
        Ok(())
    })
}

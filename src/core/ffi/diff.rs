//! Diff mode: comparison sessions between a read-only base core and a live
//! core, and base sources (Last saved refresh, chosen files, git). See
//! docs/diff_mode.md.
//!
//! Base loads and file lists may block on disk or a `git` process; hosts
//! call them on worker threads. They return an owned, immutable result handle
//! even when the base is unavailable, so the failure can be shown without an
//! error dialog; only invalid arguments fail the call itself.
use super::*;
use crate::diff::base::{
    read_file_base, BaseError, BaseKind, BaseText, CancellationToken, ChangedFile, FileChange,
    GitRepository, ProcessLimits,
};
use std::path::PathBuf;

pub type ViemDiffSessionHandle = u64;
pub type ViemDiffResultHandle = u64;
pub type ViemDiffCancellationHandle = u64;

pub const VIEM_DIFF_SESSION_ALIGNED: u32 = 1 << 0;
/// Unchanged runs collapse behind a divider.
pub const VIEM_DIFF_SESSION_COLLAPSED: u32 = 1 << 1;

pub const VIEM_DIFF_SYNC_UNCHANGED: u32 = 0;
pub const VIEM_DIFF_SYNC_INCREMENTAL: u32 = 1;
pub const VIEM_DIFF_SYNC_FULL: u32 = 2;

pub const VIEM_DIFF_NAVIGATE_NEXT: u32 = 0;
pub const VIEM_DIFF_NAVIGATE_PREVIOUS: u32 = 1;

/// A change was found; otherwise the sides are identical and nothing moved.
pub const VIEM_DIFF_NAVIGATION_FOUND: u32 = 1 << 0;
/// The views were repositioned, not only the caret.
pub const VIEM_DIFF_NAVIGATION_SCROLLED: u32 = 1 << 1;

/// The mark is the new side's added lines; otherwise removed lines.
pub const VIEM_DIFF_MARK_ADDED: u32 = 1 << 0;

pub const VIEM_DIFF_BASE_LAST_SAVED: u32 = 0;
pub const VIEM_DIFF_BASE_UNSTAGED: u32 = 1;
pub const VIEM_DIFF_BASE_HEAD: u32 = 2;
pub const VIEM_DIFF_BASE_BRANCH: u32 = 3;
pub const VIEM_DIFF_BASE_FILE: u32 = 4;
pub const VIEM_DIFF_BASE_COMMIT: u32 = 5;

pub const VIEM_DIFF_RESULT_OK: u32 = 0;
pub const VIEM_DIFF_RESULT_UNAVAILABLE: u32 = 1;
pub const VIEM_DIFF_RESULT_CANCELLED: u32 = 2;
pub const VIEM_DIFF_RESULT_TIMED_OUT: u32 = 3;
pub const VIEM_DIFF_RESULT_TOO_LARGE: u32 = 4;
pub const VIEM_DIFF_RESULT_IO_ERROR: u32 = 5;
pub const VIEM_DIFF_RESULT_COMMAND_FAILED: u32 = 6;

/// The base exists in that version; otherwise its bytes are empty.
pub const VIEM_DIFF_RESULT_BASE_EXISTS: u32 = 1 << 0;

pub const VIEM_DIFF_FILE_MODIFIED: u32 = 0;
pub const VIEM_DIFF_FILE_ADDED: u32 = 1;
pub const VIEM_DIFF_FILE_DELETED: u32 = 2;
pub const VIEM_DIFF_FILE_RENAMED: u32 = 3;
pub const VIEM_DIFF_FILE_UNSAVED: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ViemDiffBaseRequestV1 {
    pub struct_size: u32,
    pub kind: u32,
    /// The document's absolute path; for VIEM_DIFF_BASE_FILE, the chosen
    /// base file. For a renamed file's git base, its base path.
    pub file_path: ViemUtf8Slice,
    /// Branch name or commit hash, according to the base kind.
    pub revision: ViemUtf8Slice,
    /// The git executable; empty means `git` found through PATH.
    pub git_program: ViemUtf8Slice,
    /// Zero for none.
    pub cancellation: ViemDiffCancellationHandle,
    /// File lists only: absolute paths of open buffers with unsaved
    /// changes, which join the list. May be null when the count is zero.
    pub unsaved_paths: *const ViemUtf8Slice,
    pub unsaved_path_count: u64,
    /// File lists only: an absolute folder whose files alone are listed, as
    /// `git diff -- <folder>` does. Empty lists the whole repository.
    pub scope_path: ViemUtf8Slice,
}
pub const VIEM_DIFF_BASE_REQUEST_V1_SIZE: u32 = size_of::<ViemDiffBaseRequestV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemDiffResultInfoV1 {
    pub struct_size: u32,
    pub flags: u32,
    /// A VIEM_DIFF_RESULT_* code.
    pub result: u32,
    pub reserved: u32,
    /// Base bytes for a base load.
    pub byte_count: u64,
    /// Files or branches for a list.
    pub item_count: u64,
    /// UTF-8 bytes of the failure message, when there is one.
    pub message_bytes: u64,
}
pub const VIEM_DIFF_RESULT_INFO_V1_SIZE: u32 = size_of::<ViemDiffResultInfoV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemDiffListItemV1 {
    pub struct_size: u32,
    /// A VIEM_DIFF_FILE_* code; zero for branches.
    pub change: u32,
    /// UTF-8 path (or branch name) bytes, then base path bytes, are copied
    /// back to back into the caller's text buffer.
    pub path_bytes: u64,
    pub base_path_bytes: u64,
    pub added: u64,
    pub removed: u64,
}
pub const VIEM_DIFF_LIST_ITEM_V1_SIZE: u32 = size_of::<ViemDiffListItemV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemDiffSessionInfoV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub hunk_count: u64,
    pub added: u64,
    pub removed: u64,
    pub base_line_count: u64,
    pub new_line_count: u64,
}
pub const VIEM_DIFF_SESSION_INFO_V1_SIZE: u32 = size_of::<ViemDiffSessionInfoV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemDiffNavigationV1 {
    pub struct_size: u32,
    /// VIEM_DIFF_NAVIGATION_* flags.
    pub flags: u32,
    pub change_index: u64,
    pub change_count: u64,
    /// The new side's zero-based hard line that received the caret.
    pub caret_line: u64,
}
pub const VIEM_DIFF_NAVIGATION_V1_SIZE: u32 = size_of::<ViemDiffNavigationV1>() as u32;

/// One stretch of a scrollbar track, in layout units from the document top.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemDiffScrollbarMarkV1 {
    pub top: f32,
    pub bottom: f32,
    /// VIEM_DIFF_MARK_* flags.
    pub flags: u32,
    pub reserved: u32,
}

enum DiffResult {
    Base(Result<BaseText, BaseError>),
    Files(Result<Vec<ChangedFile>, BaseError>),
    Branches(Result<Vec<String>, BaseError>),
    Commits(Result<Vec<u8>, BaseError>),
}

struct DiffRegistry {
    next_handle: u64,
    sessions: HashMap<ViemDiffSessionHandle, Option<crate::coordinator::DiffSession>>,
    results: HashMap<ViemDiffResultHandle, Arc<DiffResult>>,
    cancellations: HashMap<ViemDiffCancellationHandle, CancellationToken>,
}

fn registry() -> &'static Mutex<DiffRegistry> {
    static REGISTRY: OnceLock<Mutex<DiffRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(DiffRegistry {
            next_handle: 1,
            sessions: HashMap::new(),
            results: HashMap::new(),
            cancellations: HashMap::new(),
        })
    })
}

fn allocate(registry: &mut DiffRegistry) -> Result<u64, ViemStatus> {
    let handle = registry.next_handle;
    if handle == 0 {
        return Err(ViemStatus::ResourceExhausted);
    }
    registry.next_handle = handle.checked_add(1).unwrap_or(0);
    Ok(handle)
}

fn lock() -> Result<std::sync::MutexGuard<'static, DiffRegistry>, ViemStatus> {
    registry().lock().map_err(|_| ViemStatus::InternalError)
}

/// Compare a base core's document with a live core's. The sessions are not
/// tied to views; [`viem_diff_session_sync`] names the views to update.
///
/// # Safety
/// `out_session` must point to one aligned, writable handle.
#[no_mangle]
pub unsafe extern "C" fn viem_diff_session_create(
    base_core: ViemCoreHandle,
    new_core: ViemCoreHandle,
    flags: u32,
    out_session: *mut ViemDiffSessionHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_session, 1)?;
        unsafe { out_session.write(0) };
        if flags & !(VIEM_DIFF_SESSION_ALIGNED | VIEM_DIFF_SESSION_COLLAPSED) != 0 || base_core == new_core {
            return Err(ViemStatus::InvalidArgument);
        }
        let base = checkout_core(base_core)?;
        let new = checkout_core(new_core)?;
        let mut session = crate::coordinator::DiffSession::new(
            base.core().document(),
            new.core().document(),
            flags & VIEM_DIFF_SESSION_ALIGNED != 0,
        );
        session.set_collapsed(flags & VIEM_DIFF_SESSION_COLLAPSED != 0);
        drop((base, new));
        let mut registry = lock()?;
        let handle = allocate(&mut registry)?;
        registry.sessions.insert(handle, Some(session));
        unsafe { out_session.write(handle) };
        Ok(())
    })
}

#[no_mangle]
pub extern "C" fn viem_diff_session_destroy(session: ViemDiffSessionHandle) -> ViemStatus {
    ffi_boundary(|| {
        let removed = lock()?.sessions.remove(&session).ok_or(ViemStatus::InvalidHandle)?;
        if removed.is_none() {
            return Err(ViemStatus::CoreBusy);
        }
        Ok(())
    })
}

/// Run one operation with the session checked out of the registry, so the
/// registry lock is never held while cores lay out.
fn with_session<R>(
    handle: ViemDiffSessionHandle,
    operation: impl FnOnce(&mut crate::coordinator::DiffSession) -> Result<R, ViemStatus>,
) -> Result<R, ViemStatus> {
    let mut session = {
        let mut registry = lock()?;
        let slot = registry.sessions.get_mut(&handle).ok_or(ViemStatus::InvalidHandle)?;
        slot.take().ok_or(ViemStatus::CoreBusy)?
    };
    let result = catch_unwind(AssertUnwindSafe(|| operation(&mut session)));
    if let Ok(mut registry) = lock() {
        if let Some(slot) = registry.sessions.get_mut(&handle) {
            *slot = Some(session);
        }
    }
    match result {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[no_mangle]
pub extern "C" fn viem_diff_session_set_aligned(session: ViemDiffSessionHandle, aligned: u32) -> ViemStatus {
    ffi_boundary(|| {
        if aligned > 1 {
            return Err(ViemStatus::InvalidArgument);
        }
        with_session(session, |session| {
            session.set_aligned(aligned != 0);
            Ok(())
        })
    })
}

#[no_mangle]
pub extern "C" fn viem_diff_session_set_collapsed(session: ViemDiffSessionHandle, collapsed: u32) -> ViemStatus {
    ffi_boundary(|| {
        if collapsed > 1 {
            return Err(ViemStatus::InvalidArgument);
        }
        with_session(session, |session| {
            session.set_collapsed(collapsed != 0);
            Ok(())
        })
    })
}

/// Open the collapsed run whose divider borders `line` on one side. The next
/// sync shows it on both sides.
///
/// # Safety
/// `out_opened` must point to one aligned, writable value.
#[no_mangle]
pub unsafe extern "C" fn viem_diff_session_open_collapsed(
    session: ViemDiffSessionHandle,
    from_new: u32,
    line: u64,
    out_opened: *mut u32,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_opened, 1)?;
        unsafe { out_opened.write(0) };
        let side = side_from_new(from_new)?;
        let line = usize::try_from(line).map_err(|_| ViemStatus::InvalidArgument)?;
        let opened = with_session(session, |session| Ok(session.open_collapsed_beside(side, line)))?;
        unsafe { out_opened.write(u32::from(opened)) };
        Ok(())
    })
}

/// Update the diff after any change to either core and install both sides'
/// presentations. Call after each event handled by the live core.
///
/// # Safety
/// `out_sync` must point to one aligned, writable value.
#[no_mangle]
pub unsafe extern "C" fn viem_diff_session_sync(
    session: ViemDiffSessionHandle,
    base_core: ViemCoreHandle,
    base_view: ViemViewId,
    new_core: ViemCoreHandle,
    new_view: ViemViewId,
    out_sync: *mut u32,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_sync, 1)?;
        unsafe { out_sync.write(VIEM_DIFF_SYNC_UNCHANGED) };
        if base_core == new_core {
            return Err(ViemStatus::InvalidArgument);
        }
        let mut base = checkout_core(base_core)?;
        let mut new = checkout_core(new_core)?;
        let sync = with_session(session, |session| {
            session
                .sync(base.core_mut(), ViewId(base_view), new.core_mut(), ViewId(new_view))
                .map_err(core_status)
        })?;
        unsafe {
            out_sync.write(match sync {
                crate::coordinator::DiffSync::Unchanged => VIEM_DIFF_SYNC_UNCHANGED,
                crate::coordinator::DiffSync::Incremental => VIEM_DIFF_SYNC_INCREMENTAL,
                crate::coordinator::DiffSync::Full => VIEM_DIFF_SYNC_FULL,
            })
        };
        Ok(())
    })
}

/// The zero-based hard line on the other side corresponding to `line`, for
/// keeping the sides' scroll positions together.
///
/// # Safety
/// `out_line` must point to one aligned, writable value.
#[no_mangle]
pub unsafe extern "C" fn viem_diff_session_corresponding_line(
    session: ViemDiffSessionHandle,
    from_new: u32,
    line: u64,
    out_line: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_line, 1)?;
        unsafe { out_line.write(0) };
        let side = side_from_new(from_new)?;
        let line = usize::try_from(line).map_err(|_| ViemStatus::LengthOverflow)?;
        let other = with_session(session, |session| Ok(session.corresponding_line(side, line)))?;
        unsafe { out_line.write(other as u64) };
        Ok(())
    })
}

/// The side a `from_new` argument names: 1 for the new side, 0 for the base.
fn side_from_new(from_new: u32) -> Result<crate::diff::Side, ViemStatus> {
    match from_new {
        0 => Ok(crate::diff::Side::Base),
        1 => Ok(crate::diff::Side::New),
        _ => Err(ViemStatus::InvalidArgument),
    }
}

pub const VIEM_DIFF_SCROLL_BASE_CHANGED: u32 = 1;
pub const VIEM_DIFF_SCROLL_NEW_CHANGED: u32 = 2;

/// Update the aligned pair's common scroll range and vertical position.
///
/// # Safety
/// `out_changed` must point to one aligned, writable value.
#[no_mangle]
pub unsafe extern "C" fn viem_diff_session_sync_scroll(
    session: ViemDiffSessionHandle,
    base_core: ViemCoreHandle,
    base_view: ViemViewId,
    new_core: ViemCoreHandle,
    new_view: ViemViewId,
    from_new: u32,
    out_changed: *mut u32,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_changed, 1)?;
        unsafe { out_changed.write(0) };
        let side = side_from_new(from_new)?;
        if base_core == new_core {
            return Err(ViemStatus::InvalidArgument);
        }
        let mut base = checkout_core(base_core)?;
        let mut new = checkout_core(new_core)?;
        let (left, right) = with_session(session, |session| {
            session.sync_scroll(base.core_mut(), ViewId(base_view), new.core_mut(), ViewId(new_view), side).map_err(core_status)
        })?;
        let changed = (if left { VIEM_DIFF_SCROLL_BASE_CHANGED } else { 0 }) | (if right { VIEM_DIFF_SCROLL_NEW_CHANGED } else { 0 });
        unsafe { out_changed.write(changed) };
        Ok(())
    })
}

/// Move both sides to the next or previous change, wrapping within the file:
/// the new view's caret moves to the change, and both views scroll unless it
/// is already visible on both. Hosts then refresh both views.
///
/// # Safety
/// `out_navigation` must point to an aligned, writable value whose
/// `struct_size` is at least `VIEM_DIFF_NAVIGATION_V1_SIZE`.
#[no_mangle]
pub unsafe extern "C" fn viem_diff_session_navigate(
    session: ViemDiffSessionHandle,
    base_core: ViemCoreHandle,
    base_view: ViemViewId,
    new_core: ViemCoreHandle,
    new_view: ViemViewId,
    direction: u32,
    out_navigation: *mut ViemDiffNavigationV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_navigation, 1)?;
        if unsafe { (*out_navigation).struct_size } < VIEM_DIFF_NAVIGATION_V1_SIZE {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe { out_navigation.write(ViemDiffNavigationV1 { struct_size: VIEM_DIFF_NAVIGATION_V1_SIZE, ..Default::default() }) };
        let direction = match direction {
            VIEM_DIFF_NAVIGATE_NEXT => crate::coordinator::ChangeDirection::Next,
            VIEM_DIFF_NAVIGATE_PREVIOUS => crate::coordinator::ChangeDirection::Previous,
            _ => return Err(ViemStatus::InvalidArgument),
        };
        if base_core == new_core {
            return Err(ViemStatus::InvalidArgument);
        }
        let mut base = checkout_core(base_core)?;
        let mut new = checkout_core(new_core)?;
        let navigation = with_session(session, |session| {
            session
                .navigate_change(base.core_mut(), ViewId(base_view), new.core_mut(), ViewId(new_view), direction)
                .map_err(core_status)
        })?;
        if let Some(navigation) = navigation {
            let flags = VIEM_DIFF_NAVIGATION_FOUND | if navigation.scrolled { VIEM_DIFF_NAVIGATION_SCROLLED } else { 0 };
            unsafe {
                out_navigation.write(ViemDiffNavigationV1 {
                    struct_size: VIEM_DIFF_NAVIGATION_V1_SIZE,
                    flags,
                    change_index: navigation.change as u64,
                    change_count: navigation.change_count as u64,
                    caret_line: navigation.caret_line as u64,
                })
            };
        }
        Ok(())
    })
}

/// Two-pass copy of one side's scrollbar marks, sorted by top, and the
/// document extent they are measured against. `is_new` is one for the live
/// side. A short buffer reports the required count with
/// `VIEM_STATUS_BUFFER_TOO_SMALL`.
///
/// # Safety
/// `out_count` and `out_extent` are aligned and writable. Nonzero capacity
/// requires writable mark storage of that size, disjoint from them.
#[no_mangle]
pub unsafe extern "C" fn viem_diff_session_scrollbar_marks(
    session: ViemDiffSessionHandle,
    core: ViemCoreHandle,
    view: ViemViewId,
    is_new: u32,
    marks: *mut ViemDiffScrollbarMarkV1,
    capacity: u64,
    out_count: *mut u64,
    out_extent: *mut f32,
) -> ViemStatus {
    ffi_boundary(|| {
        let capacity = checked_length(capacity)?;
        validate_disjoint_regions(&[
            typed_pointer_region(marks, capacity as u64)?,
            typed_pointer_region(out_count, 1)?,
            typed_pointer_region(out_extent, 1)?,
        ])?;
        unsafe {
            out_count.write(0);
            out_extent.write(0.0);
        }
        if is_new > 1 {
            return Err(ViemStatus::InvalidArgument);
        }
        let side = if is_new != 0 { crate::diff::Side::New } else { crate::diff::Side::Base };
        let mut core = checkout_core(core)?;
        let (found, extent) = with_session(session, |session| {
            session.scrollbar_marks(core.core_mut(), ViewId(view), side).map_err(core_status)
        })?;
        unsafe {
            out_count.write(found.len() as u64);
            out_extent.write(extent);
        }
        if capacity < found.len() {
            return Err(ViemStatus::BufferTooSmall);
        }
        for (index, mark) in found.iter().enumerate() {
            let value = ViemDiffScrollbarMarkV1 {
                top: mark.top,
                bottom: mark.bottom,
                flags: if mark.added { VIEM_DIFF_MARK_ADDED } else { 0 },
                reserved: 0,
            };
            unsafe { marks.add(index).write(value) };
        }
        Ok(())
    })
}

/// The backgrounds of the Added line and Removed line Code styles, which
/// scrollbar marks use; the built-in defaults when a style declares none.
///
/// # Safety
/// Both outputs are aligned, writable and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_diff_line_colors(out_added: *mut ViemRgbaV1, out_removed: *mut ViemRgbaV1) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[typed_pointer_region(out_added, 1)?, typed_pointer_region(out_removed, 1)?])?;
        let color = |added| {
            let color = crate::document::code_style::diff_line_background(added);
            ViemRgbaV1 { red: color.red, green: color.green, blue: color.blue, alpha: color.alpha }
        };
        unsafe {
            out_added.write(color(true));
            out_removed.write(color(false));
        }
        Ok(())
    })
}

/// # Safety
/// `out_info` must point to an aligned, writable value whose `struct_size`
/// is at least `VIEM_DIFF_SESSION_INFO_V1_SIZE`.
#[no_mangle]
pub unsafe extern "C" fn viem_diff_session_info(
    session: ViemDiffSessionHandle,
    out_info: *mut ViemDiffSessionInfoV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        if unsafe { (*out_info).struct_size } < VIEM_DIFF_SESSION_INFO_V1_SIZE {
            return Err(ViemStatus::InvalidArgument);
        }
        let info = with_session(session, |session| {
            let (added, removed) = session.line_totals();
            Ok(ViemDiffSessionInfoV1 {
                added: added as u64, removed: removed as u64,
                struct_size: VIEM_DIFF_SESSION_INFO_V1_SIZE,
                flags: if session.aligned() { VIEM_DIFF_SESSION_ALIGNED } else { 0 }
                    | if session.collapsed() { VIEM_DIFF_SESSION_COLLAPSED } else { 0 },
                hunk_count: session.state().hunk_count() as u64,
                base_line_count: session.state().base_line_count() as u64,
                new_line_count: session.state().new_line_count() as u64,
            })
        })?;
        unsafe { out_info.write(info) };
        Ok(())
    })
}

/// Set the buffer's `modifiable` policy; zero rejects every edit path,
/// including undo. Diff mode's base buffer is not modifiable.
#[no_mangle]
pub extern "C" fn viem_core_set_modifiable(core: ViemCoreHandle, modifiable: u32) -> ViemStatus {
    ffi_boundary(|| {
        if modifiable > 1 {
            return Err(ViemStatus::InvalidArgument);
        }
        with_core_mut(core, |core| {
            core.set_modifiable(modifiable != 0);
            Ok(())
        })
    })
}

/// Whether the view's caret opened collapsed context since the last sync, so
/// the host syncs to open it on the other side too.
///
/// # Safety
/// `out_opened` must point to one aligned, writable value.
#[no_mangle]
pub unsafe extern "C" fn viem_diff_view_opened_context(core: ViemCoreHandle, view: ViemViewId,
    out_opened: *mut u32) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_opened, 1)?;
        unsafe { out_opened.write(0) };
        let opened = with_core(core, |core| core.has_opened_collapsed(ViewId(view)).map_err(core_status))?;
        unsafe { out_opened.write(u32::from(opened)) };
        Ok(())
    })
}

/// Remove diff presentation from one view, for leaving diff mode.
#[no_mangle]
pub extern "C" fn viem_diff_clear_view(core: ViemCoreHandle, view: ViemViewId) -> ViemStatus {
    ffi_boundary(|| {
        with_core_mut(core, |core| crate::coordinator::DiffSession::clear(core, ViewId(view)).map_err(core_status))
    })
}

/// Copy the source as last loaded or saved, the Last saved base. Two-pass:
/// BUFFER_TOO_SMALL reports the exact size. `out_available` is zero when the
/// saved source is unknown (after recovery); hosts then read the file.
///
/// # Safety
/// `out_required` and `out_available` are aligned and writable. Nonzero
/// capacity requires writable output storage of that size, disjoint from them.
#[no_mangle]
pub unsafe extern "C" fn viem_core_copy_saved_source(
    core: ViemCoreHandle,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
    out_available: *mut u32,
) -> ViemStatus {
    ffi_boundary(|| {
        let capacity = checked_length(output_capacity)?;
        validate_disjoint_regions(&[
            typed_pointer_region(output, output_capacity)?,
            typed_pointer_region(out_required, 1)?,
            typed_pointer_region(out_available, 1)?,
        ])?;
        unsafe {
            out_required.write(0);
            out_available.write(0);
        }
        let Some(bytes) = with_core(core, |core| Ok(core.document().saved_source_bytes()))? else {
            return Ok(());
        };
        unsafe {
            out_available.write(1);
            out_required.write(bytes.len() as u64);
        }
        copy_out(&bytes, output, capacity)
    })
}

fn copy_out(bytes: &[u8], output: *mut u8, capacity: usize) -> Result<(), ViemStatus> {
    if capacity < bytes.len() {
        return Err(ViemStatus::BufferTooSmall);
    }
    if !bytes.is_empty() {
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len()) };
    }
    Ok(())
}

#[no_mangle]
pub unsafe extern "C" fn viem_diff_cancellation_create(out_cancellation: *mut ViemDiffCancellationHandle) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_cancellation, 1)?;
        unsafe { out_cancellation.write(0) };
        let mut registry = lock()?;
        let handle = allocate(&mut registry)?;
        registry.cancellations.insert(handle, CancellationToken::new());
        unsafe { out_cancellation.write(handle) };
        Ok(())
    })
}

/// Cancel any load using this token, from any thread. Loads started later
/// with it fail immediately as cancelled.
#[no_mangle]
pub extern "C" fn viem_diff_cancellation_cancel(cancellation: ViemDiffCancellationHandle) -> ViemStatus {
    ffi_boundary(|| {
        let token = lock()?.cancellations.get(&cancellation).cloned().ok_or(ViemStatus::InvalidHandle)?;
        token.cancel();
        Ok(())
    })
}

#[no_mangle]
pub extern "C" fn viem_diff_cancellation_release(cancellation: ViemDiffCancellationHandle) -> ViemStatus {
    ffi_boundary(|| lock()?.cancellations.remove(&cancellation).map(|_| ()).ok_or(ViemStatus::InvalidHandle))
}

struct BaseRequest {
    kind: BaseKind,
    path: PathBuf,
    scope: Option<PathBuf>,
    git_program: PathBuf,
    cancellation: CancellationToken,
    unsaved: Vec<PathBuf>,
}

unsafe fn utf8_input<'a>(slice: ViemUtf8Slice) -> Result<&'a str, ViemStatus> {
    let bytes = unsafe { input_bytes(slice.data, slice.length)? };
    std::str::from_utf8(bytes).map_err(|_| ViemStatus::InvalidUtf8)
}

unsafe fn base_request(request: *const ViemDiffBaseRequestV1) -> Result<BaseRequest, ViemStatus> {
    if request.is_null() {
        return Err(ViemStatus::NullPointer);
    }
    typed_pointer_region(request, 1)?;
    let request = unsafe { *request };
    if request.struct_size < VIEM_DIFF_BASE_REQUEST_V1_SIZE {
        return Err(ViemStatus::InvalidArgument);
    }
    let path = PathBuf::from(unsafe { utf8_input(request.file_path)? });
    if !path.is_absolute() {
        return Err(ViemStatus::InvalidArgument);
    }
    let kind = match request.kind {
        VIEM_DIFF_BASE_LAST_SAVED => BaseKind::LastSaved,
        VIEM_DIFF_BASE_UNSTAGED => BaseKind::Unstaged,
        VIEM_DIFF_BASE_HEAD => BaseKind::Head,
        VIEM_DIFF_BASE_BRANCH => BaseKind::Branch(unsafe { utf8_input(request.revision)? }.to_owned()),
        VIEM_DIFF_BASE_COMMIT => BaseKind::Commit(unsafe { utf8_input(request.revision)? }.to_owned()),
        VIEM_DIFF_BASE_FILE => BaseKind::File(path.clone()),
        _ => return Err(ViemStatus::InvalidArgument),
    };
    let program = unsafe { utf8_input(request.git_program)? };
    let cancellation = if request.cancellation == 0 {
        CancellationToken::new()
    } else {
        lock()?.cancellations.get(&request.cancellation).cloned().ok_or(ViemStatus::InvalidHandle)?
    };
    let count = checked_length(request.unsaved_path_count)?;
    let mut unsaved = Vec::with_capacity(count.min(4096));
    if count > 0 {
        typed_pointer_region(request.unsaved_paths, request.unsaved_path_count)?;
        for index in 0..count {
            let slice = unsafe { *request.unsaved_paths.add(index) };
            unsaved.push(PathBuf::from(unsafe { utf8_input(slice)? }));
        }
    }
    let scope = unsafe { utf8_input(request.scope_path)? };
    let scope = (!scope.is_empty()).then(|| PathBuf::from(scope));
    if scope.as_ref().is_some_and(|scope| !scope.is_absolute()) {
        return Err(ViemStatus::InvalidArgument);
    }
    Ok(BaseRequest {
        kind,
        path,
        scope,
        git_program: PathBuf::from(if program.is_empty() { "git" } else { program }),
        cancellation,
        unsaved,
    })
}

fn repository(request: &BaseRequest) -> Result<GitRepository, BaseError> {
    GitRepository::discover(&request.git_program, &request.path, ProcessLimits::default(), &request.cancellation)
}

fn publish(result: DiffResult, out_result: *mut ViemDiffResultHandle) -> Result<(), ViemStatus> {
    let mut registry = lock()?;
    let handle = allocate(&mut registry)?;
    registry.results.insert(handle, Arc::new(result));
    unsafe { out_result.write(handle) };
    Ok(())
}

/// Load a base version of a file. Last saved here means the file on disk
/// (its Refresh); the in-memory copy comes from `viem_core_copy_saved_source`.
///
/// # Safety
/// `request` points to a readable request; `out_result` to one writable handle.
#[no_mangle]
pub unsafe extern "C" fn viem_diff_load_base(
    request: *const ViemDiffBaseRequestV1,
    out_result: *mut ViemDiffResultHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_result, 1)?;
        unsafe { out_result.write(0) };
        let request = unsafe { base_request(request)? };
        let result = match &request.kind {
            BaseKind::LastSaved | BaseKind::File(_) => read_file_base(&request.path),
            kind => GitRepository::load_file(&request.git_program, kind, &request.path, ProcessLimits::default(), &request.cancellation),
        };
        publish(DiffResult::Base(result), out_result)
    })
}

/// List the files that differ from a base, sorted by path. Git bases list
/// tracked changes as `git diff` does plus the request's unsaved buffers
/// inside the repository; Last saved lists only the document; a chosen file
/// lists nothing.
///
/// # Safety
/// As for [`viem_diff_load_base`].
#[no_mangle]
pub unsafe extern "C" fn viem_diff_list_files(
    request: *const ViemDiffBaseRequestV1,
    out_result: *mut ViemDiffResultHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_result, 1)?;
        unsafe { out_result.write(0) };
        let request = unsafe { base_request(request)? };
        let result = match &request.kind {
            BaseKind::LastSaved => Ok(vec![ChangedFile {
                path: request.path.clone(),
                base_path: request.path.clone(),
                change: FileChange::Modified, added: 0, removed: 0,
            }]),
            BaseKind::File(_) => Ok(Vec::new()),
            kind => {
                let scope = request.scope.as_deref();
                GitRepository::discover(&request.git_program, scope.unwrap_or(&request.path), ProcessLimits::default(), &request.cancellation)
                    .and_then(|repo| repo.files_for_document(kind, &request.path, &request.unsaved, scope, &request.cancellation))
            }
        };
        publish(DiffResult::Files(result), out_result)
    })
}

/// List the repository's local branches for the Branch base.
///
/// # Safety
/// As for [`viem_diff_load_base`].
#[no_mangle]
pub unsafe extern "C" fn viem_diff_list_branches(
    request: *const ViemDiffBaseRequestV1,
    out_result: *mut ViemDiffResultHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_result, 1)?;
        unsafe { out_result.write(0) };
        let request = unsafe { base_request(request)? };
        let result = repository(&request).and_then(|repository| repository.branches(&request.cancellation));
        publish(DiffResult::Branches(result), out_result)
    })
}

fn result(handle: ViemDiffResultHandle) -> Result<Arc<DiffResult>, ViemStatus> {
    lock()?.results.get(&handle).cloned().ok_or(ViemStatus::InvalidHandle)
}

/// Return recent commits for HEAD, or resolve a commit hash for COMMIT.
/// # Safety
/// As for [`viem_diff_load_base`].
#[no_mangle]
pub unsafe extern "C" fn viem_diff_query_commits(
    request: *const ViemDiffBaseRequestV1,
    out_result: *mut ViemDiffResultHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_result, 1)?;
        unsafe { out_result.write(0) };
        let request = unsafe { base_request(request)? };
        let hash = match &request.kind {
            BaseKind::Head => None,
            BaseKind::Commit(hash) => Some(hash.as_str()),
            _ => return Err(ViemStatus::InvalidArgument),
        };
        let commits = repository(&request)
            .and_then(|repo| repo.commits(hash, &request.cancellation))
            .and_then(|rows| serde_json::to_vec(&rows).map_err(|e| BaseError::Io(e.to_string())));
        publish(DiffResult::Commits(commits), out_result)
    })
}

fn error_code(error: &BaseError) -> u32 {
    match error {
        BaseError::Unavailable(_) => VIEM_DIFF_RESULT_UNAVAILABLE,
        BaseError::Cancelled => VIEM_DIFF_RESULT_CANCELLED,
        BaseError::TimedOut => VIEM_DIFF_RESULT_TIMED_OUT,
        BaseError::TooLarge => VIEM_DIFF_RESULT_TOO_LARGE,
        BaseError::Io(_) => VIEM_DIFF_RESULT_IO_ERROR,
        BaseError::Command { .. } => VIEM_DIFF_RESULT_COMMAND_FAILED,
    }
}

fn result_error(result: &DiffResult) -> Option<&BaseError> {
    match result {
        DiffResult::Base(Err(error)) => Some(error),
        DiffResult::Files(Err(error)) => Some(error),
        DiffResult::Branches(Err(error)) => Some(error),
        DiffResult::Commits(Err(error)) => Some(error),
        _ => None,
    }
}

/// # Safety
/// `out_info` points to an aligned, writable value whose `struct_size` is at
/// least `VIEM_DIFF_RESULT_INFO_V1_SIZE`.
#[no_mangle]
pub unsafe extern "C" fn viem_diff_result_info(handle: ViemDiffResultHandle, out_info: *mut ViemDiffResultInfoV1) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        if unsafe { (*out_info).struct_size } < VIEM_DIFF_RESULT_INFO_V1_SIZE {
            return Err(ViemStatus::InvalidArgument);
        }
        let result = result(handle)?;
        let mut info = ViemDiffResultInfoV1 { struct_size: VIEM_DIFF_RESULT_INFO_V1_SIZE, ..Default::default() };
        if let Some(error) = result_error(&result) {
            info.result = error_code(error);
            info.message_bytes = error.to_string().len() as u64;
        }
        match result.as_ref() {
            DiffResult::Base(Ok(base)) => {
                info.byte_count = base.bytes.len() as u64;
                if base.exists {
                    info.flags |= VIEM_DIFF_RESULT_BASE_EXISTS;
                }
            }
            DiffResult::Files(Ok(files)) => info.item_count = files.len() as u64,
            DiffResult::Branches(Ok(branches)) => info.item_count = branches.len() as u64,
            DiffResult::Commits(Ok(json)) => info.byte_count = json.len() as u64,
            _ => {}
        }
        unsafe { out_info.write(info) };
        Ok(())
    })
}

/// Copy a base load's bytes. Two-pass, as for source copies.
///
/// # Safety
/// `out_required` is aligned and writable; nonzero capacity requires
/// writable, disjoint output storage of that size.
#[no_mangle]
pub unsafe extern "C" fn viem_diff_result_copy_bytes(
    handle: ViemDiffResultHandle,
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
        let result = result(handle)?;
        let bytes = match result.as_ref() {
            DiffResult::Base(Ok(base)) => &base.bytes,
            DiffResult::Commits(Ok(json)) => json,
            _ => return Err(ViemStatus::InvalidArgument),
        };
        unsafe { out_required.write(bytes.len() as u64) };
        copy_out(bytes, output, capacity)
    })
}

/// Copy a failed result's message as UTF-8. Two-pass.
///
/// # Safety
/// As for [`viem_diff_result_copy_bytes`].
#[no_mangle]
pub unsafe extern "C" fn viem_diff_result_copy_message(
    handle: ViemDiffResultHandle,
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
        let result = result(handle)?;
        let message = result_error(&result).map(ToString::to_string).unwrap_or_default();
        unsafe { out_required.write(message.len() as u64) };
        copy_out(message.as_bytes(), output, capacity)
    })
}

/// Copy one list item: its descriptor, then its path followed by its base
/// path (for files) into `text`. Two-pass on `text`.
///
/// # Safety
/// `out_item` points to an aligned, writable value whose `struct_size` is at
/// least `VIEM_DIFF_LIST_ITEM_V1_SIZE`; `out_required` is aligned and
/// writable; nonzero `text_capacity` requires writable storage of that size.
/// All three are disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_diff_result_copy_item(
    handle: ViemDiffResultHandle,
    index: u64,
    out_item: *mut ViemDiffListItemV1,
    text: *mut u8,
    text_capacity: u64,
    out_required: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        let capacity = checked_length(text_capacity)?;
        validate_disjoint_regions(&[
            typed_pointer_region(out_item, 1)?,
            typed_pointer_region(text, text_capacity)?,
            typed_pointer_region(out_required, 1)?,
        ])?;
        if unsafe { (*out_item).struct_size } < VIEM_DIFF_LIST_ITEM_V1_SIZE {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe { out_required.write(0) };
        let index = usize::try_from(index).map_err(|_| ViemStatus::InvalidRange)?;
        let result = result(handle)?;
        let (change, path, base_path, added, removed) = match result.as_ref() {
            DiffResult::Files(Ok(files)) => {
                let file = files.get(index).ok_or(ViemStatus::InvalidRange)?;
                let change = match file.change {
                    FileChange::Modified => VIEM_DIFF_FILE_MODIFIED,
                    FileChange::Added => VIEM_DIFF_FILE_ADDED,
                    FileChange::Deleted => VIEM_DIFF_FILE_DELETED,
                    FileChange::Renamed => VIEM_DIFF_FILE_RENAMED,
                    FileChange::Unsaved => VIEM_DIFF_FILE_UNSAVED,
                };
                let path = file.path.to_str().ok_or(ViemStatus::InvalidUtf8)?.to_owned();
                let base_path = file.base_path.to_str().ok_or(ViemStatus::InvalidUtf8)?.to_owned();
                (change, path, base_path, file.added, file.removed)
            }
            DiffResult::Branches(Ok(branches)) => {
                (0, branches.get(index).ok_or(ViemStatus::InvalidRange)?.clone(), String::new(), 0, 0)
            }
            _ => return Err(ViemStatus::InvalidArgument),
        };
        unsafe {
            out_item.write(ViemDiffListItemV1 {
                struct_size: VIEM_DIFF_LIST_ITEM_V1_SIZE,
                change, added, removed,
                path_bytes: path.len() as u64,
                base_path_bytes: base_path.len() as u64,
            });
            out_required.write((path.len() + base_path.len()) as u64);
        }
        let joined = [path.as_bytes(), base_path.as_bytes()].concat();
        copy_out(&joined, text, capacity)
    })
}

#[no_mangle]
pub extern "C" fn viem_diff_result_release(handle: ViemDiffResultHandle) -> ViemStatus {
    ffi_boundary(|| {
        let removed = lock()?.results.remove(&handle).ok_or(ViemStatus::InvalidHandle)?;
        drop(removed);
        Ok(())
    })
}

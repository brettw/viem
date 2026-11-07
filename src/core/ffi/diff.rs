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
    include_unsaved, read_file_base, BaseError, BaseKind, BaseText, CancellationToken, ChangedFile, FileChange,
    GitRepository, ProcessLimits,
};
use std::path::{Path, PathBuf};

pub type ViemDiffSessionHandle = u64;
pub type ViemDiffResultHandle = u64;
pub type ViemDiffCancellationHandle = u64;

pub const VIEM_DIFF_SESSION_ALIGNED: u32 = 1 << 0;

pub const VIEM_DIFF_SYNC_UNCHANGED: u32 = 0;
pub const VIEM_DIFF_SYNC_INCREMENTAL: u32 = 1;
pub const VIEM_DIFF_SYNC_FULL: u32 = 2;

pub const VIEM_DIFF_BASE_LAST_SAVED: u32 = 0;
pub const VIEM_DIFF_BASE_UNSTAGED: u32 = 1;
pub const VIEM_DIFF_BASE_HEAD: u32 = 2;
pub const VIEM_DIFF_BASE_BRANCH: u32 = 3;
pub const VIEM_DIFF_BASE_FILE: u32 = 4;

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
    /// VIEM_DIFF_BASE_BRANCH only.
    pub branch: ViemUtf8Slice,
    /// The git executable; empty means `git` found through PATH.
    pub git_program: ViemUtf8Slice,
    /// Zero for none.
    pub cancellation: ViemDiffCancellationHandle,
    /// File lists only: absolute paths of open buffers with unsaved
    /// changes, which join the list. May be null when the count is zero.
    pub unsaved_paths: *const ViemUtf8Slice,
    pub unsaved_path_count: u64,
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
}
pub const VIEM_DIFF_LIST_ITEM_V1_SIZE: u32 = size_of::<ViemDiffListItemV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemDiffSessionInfoV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub hunk_count: u64,
    pub base_line_count: u64,
    pub new_line_count: u64,
}
pub const VIEM_DIFF_SESSION_INFO_V1_SIZE: u32 = size_of::<ViemDiffSessionInfoV1>() as u32;

enum DiffResult {
    Base(Result<BaseText, BaseError>),
    Files(Result<Vec<ChangedFile>, BaseError>),
    Branches(Result<Vec<String>, BaseError>),
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
        if flags & !VIEM_DIFF_SESSION_ALIGNED != 0 || base_core == new_core {
            return Err(ViemStatus::InvalidArgument);
        }
        let base = checkout_core(base_core)?;
        let new = checkout_core(new_core)?;
        let session = crate::coordinator::DiffSession::new(
            base.core().document(),
            new.core().document(),
            flags & VIEM_DIFF_SESSION_ALIGNED != 0,
        );
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
        if from_new > 1 {
            return Err(ViemStatus::InvalidArgument);
        }
        let line = usize::try_from(line).map_err(|_| ViemStatus::LengthOverflow)?;
        let other = with_session(session, |session| Ok(session.corresponding_line(from_new != 0, line)))?;
        unsafe { out_line.write(other as u64) };
        Ok(())
    })
}

/// The viewport top for `to_view` showing the text that corresponds to the
/// top of `from_view`, for scrolling aligned sides together. `from_new` is
/// one when `from_view` shows the live document.
///
/// # Safety
/// `out_top` must point to one aligned, writable value.
#[no_mangle]
pub unsafe extern "C" fn viem_diff_session_corresponding_top(
    session: ViemDiffSessionHandle,
    from_core: ViemCoreHandle,
    from_view: ViemViewId,
    from_new: u32,
    to_core: ViemCoreHandle,
    to_view: ViemViewId,
    out_top: *mut f32,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_top, 1)?;
        unsafe { out_top.write(0.0) };
        if from_new > 1 || from_core == to_core {
            return Err(ViemStatus::InvalidArgument);
        }
        let from = checkout_core(from_core)?;
        let to = checkout_core(to_core)?;
        let top = with_session(session, |session| {
            session
                .corresponding_top(from.core(), ViewId(from_view), from_new != 0, to.core(), ViewId(to_view))
                .map_err(core_status)
        })?;
        unsafe { out_top.write(top) };
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
            Ok(ViemDiffSessionInfoV1 {
                struct_size: VIEM_DIFF_SESSION_INFO_V1_SIZE,
                flags: if session.aligned() { VIEM_DIFF_SESSION_ALIGNED } else { 0 },
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
        VIEM_DIFF_BASE_BRANCH => BaseKind::Branch(unsafe { utf8_input(request.branch)? }.to_owned()),
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
    Ok(BaseRequest {
        kind,
        path,
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
            kind => repository(&request).and_then(|repository| repository.load(kind, &request.path, &request.cancellation)),
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
                change: FileChange::Modified,
            }]),
            BaseKind::File(_) => Ok(Vec::new()),
            kind => repository(&request).and_then(|repository| {
                let mut files = repository.changed_files(kind, &request.cancellation)?;
                let root = std::fs::canonicalize(repository.root()).unwrap_or_else(|_| repository.root().to_owned());
                let inside = |path: &Path| {
                    std::fs::canonicalize(path.parent().unwrap_or(path)).is_ok_and(|parent| parent.starts_with(&root))
                };
                include_unsaved(&mut files, request.unsaved.iter().filter(|path| inside(path)).cloned());
                Ok(files)
            }),
        };
        publish(DiffResult::Files(result), out_result)
    })
}

/// List the repository's local and remote branches for the Branch base.
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
        let DiffResult::Base(Ok(base)) = result.as_ref() else {
            return Err(ViemStatus::InvalidArgument);
        };
        unsafe { out_required.write(base.bytes.len() as u64) };
        copy_out(&base.bytes, output, capacity)
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
        let (change, path, base_path) = match result.as_ref() {
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
                (change, path, base_path)
            }
            DiffResult::Branches(Ok(branches)) => {
                (0, branches.get(index).ok_or(ViemStatus::InvalidRange)?.clone(), String::new())
            }
            _ => return Err(ViemStatus::InvalidArgument),
        };
        unsafe {
            out_item.write(ViemDiffListItemV1 {
                struct_size: VIEM_DIFF_LIST_ITEM_V1_SIZE,
                change,
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

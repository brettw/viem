//! Owned, bounded external-file review state for platform frontends.

use super::*;
use crate::document::{ExternalFileReviewState, MAX_EXTERNAL_FILE_OBSERVATION_BYTES};

pub type ViemExternalFileReviewHandle = u64;
pub const VIEM_EXTERNAL_FILE_REVIEW_PRESENT: u32 = 1;
pub const VIEM_EXTERNAL_FILE_REVIEW_CAN_RELOAD: u32 = 1 << 1;
pub const VIEM_EXTERNAL_FILE_REVIEW_DISCARDS_UNSAVED_CHANGES: u32 = 1 << 2;

struct ReviewRegistry {
    next_handle: u64,
    // None marks an exclusive, checked-out turn for the handle.
    states: HashMap<ViemExternalFileReviewHandle, Option<ExternalFileReviewState>>,
}

fn review_registry() -> &'static Mutex<ReviewRegistry> {
    static REGISTRY: OnceLock<Mutex<ReviewRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(ReviewRegistry {
            next_handle: 1,
            states: HashMap::new(),
        })
    })
}

struct ReviewLease {
    handle: ViemExternalFileReviewHandle,
    state: Option<ExternalFileReviewState>,
}

impl Drop for ReviewLease {
    fn drop(&mut self) {
        if let Ok(mut registry) = review_registry().lock() {
            if let Some(entry @ None) = registry.states.get_mut(&self.handle) {
                *entry = self.state.take();
            }
        }
    }
}

fn checkout_review(handle: ViemExternalFileReviewHandle) -> Result<ReviewLease, ViemStatus> {
    let mut registry = review_registry()
        .lock()
        .map_err(|_| ViemStatus::InternalError)?;
    let state = registry
        .states
        .get_mut(&handle)
        .ok_or(ViemStatus::InvalidHandle)?;
    Ok(ReviewLease {
        handle,
        state: Some(state.take().ok_or(ViemStatus::CoreBusy)?),
    })
}

/// Creates an owned nonzero token. Handles are never reused in this process.
///
/// # Safety
/// `out_handle` must identify writable, aligned storage for one handle.
#[no_mangle]
pub unsafe extern "C" fn viem_external_file_review_create(
    out_handle: *mut ViemExternalFileReviewHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_handle, 1)?;
        unsafe { out_handle.write(0) };
        let handle = {
            let mut registry = review_registry()
                .lock()
                .map_err(|_| ViemStatus::InternalError)?;
            let handle = registry.next_handle;
            if handle == 0 {
                return Err(ViemStatus::ResourceExhausted);
            }
            registry.next_handle = handle.checked_add(1).unwrap_or(0);
            registry
                .states
                .insert(handle, Some(ExternalFileReviewState::default()));
            handle
        };
        unsafe { out_handle.write(handle) };
        Ok(())
    })
}

/// A successful destroy consumes the handle. A busy result retains ownership.
#[no_mangle]
pub extern "C" fn viem_external_file_review_destroy(
    handle: ViemExternalFileReviewHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        let removed = {
            let mut registry = review_registry()
                .lock()
                .map_err(|_| ViemStatus::InternalError)?;
            match registry.states.get(&handle) {
                None => return Err(ViemStatus::InvalidHandle),
                Some(None) => return Err(ViemStatus::CoreBusy),
                Some(Some(_)) => registry.states.remove(&handle),
            }
        };
        drop(removed);
        Ok(())
    })
}

#[no_mangle]
pub extern "C" fn viem_external_file_review_reset(
    handle: ViemExternalFileReviewHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        let mut lease = checkout_review(handle)?;
        lease.state.as_mut().unwrap().reset();
        Ok(())
    })
}

/// Forget the acknowledged observation while retaining any active review.
/// The host calls this after observing the saved baseline again.
#[no_mangle]
pub extern "C" fn viem_external_file_review_clear_acknowledged(
    handle: ViemExternalFileReviewHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        let mut lease = checkout_review(handle)?;
        lease.state.as_mut().unwrap().clear_acknowledged();
        Ok(())
    })
}

/// Starts a review unless its observation is acknowledged or a review is active.
/// `out_flags` is zero when no review should be presented. Boolean inputs are 0/1.
///
/// # Safety
/// Token and output regions must be valid, aligned, and mutually disjoint.
/// Tokens contain 1 through MAX_EXTERNAL_FILE_OBSERVATION_BYTES opaque bytes.
#[no_mangle]
pub unsafe extern "C" fn viem_external_file_review_begin(
    handle: ViemExternalFileReviewHandle,
    token: *const u8,
    token_length: u64,
    can_reload: u32,
    is_dirty: u32,
    out_flags: *mut u32,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(token, token_length)?,
            typed_pointer_region(out_flags, 1)?,
        ])?;
        unsafe { out_flags.write(0) };
        if token_length == 0
            || token_length > MAX_EXTERNAL_FILE_OBSERVATION_BYTES as u64
            || can_reload > 1
            || is_dirty > 1
        {
            return Err(ViemStatus::InvalidArgument);
        }
        let token = unsafe { input_bytes(token, token_length)? };
        let flags = {
            let mut lease = checkout_review(handle)?;
            match lease
                .state
                .as_mut()
                .unwrap()
                .begin(token, can_reload == 1, is_dirty == 1)
                .map_err(|_| ViemStatus::InvalidArgument)?
            {
                Some(review) => {
                    VIEM_EXTERNAL_FILE_REVIEW_PRESENT
                        | if review.can_reload {
                            VIEM_EXTERNAL_FILE_REVIEW_CAN_RELOAD
                        } else {
                            0
                        }
                        | if review.discards_unsaved_changes {
                            VIEM_EXTERNAL_FILE_REVIEW_DISCARDS_UNSAVED_CHANGES
                        } else {
                            0
                        }
                }
                None => 0,
            }
        };
        unsafe { out_flags.write(flags) };
        Ok(())
    })
}

/// Completes only a matching active review; mismatched/invalidated reviews are
/// no-ops. `acknowledge` is 0 to allow retry, or 1 to suppress this observation.
///
/// # Safety
/// `token` identifies `token_length` readable bytes for this call.
#[no_mangle]
pub unsafe extern "C" fn viem_external_file_review_finish(
    handle: ViemExternalFileReviewHandle,
    token: *const u8,
    token_length: u64,
    acknowledge: u32,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(token, token_length)?;
        if token_length == 0
            || token_length > MAX_EXTERNAL_FILE_OBSERVATION_BYTES as u64
            || acknowledge > 1
        {
            return Err(ViemStatus::InvalidArgument);
        }
        let token = unsafe { input_bytes(token, token_length)? };
        let mut lease = checkout_review(handle)?;
        lease
            .state
            .as_mut()
            .unwrap()
            .finish(token, acknowledge == 1)
            .map_err(|_| ViemStatus::InvalidArgument)?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create() -> ViemExternalFileReviewHandle {
        let mut handle = 0;
        assert_eq!(
            unsafe { viem_external_file_review_create(&mut handle) },
            ViemStatus::Ok
        );
        assert_ne!(handle, 0);
        handle
    }

    fn begin(handle: u64, token: &[u8], reload: u32, dirty: u32) -> (ViemStatus, u32) {
        let mut flags = 99;
        let status = unsafe {
            viem_external_file_review_begin(
                handle,
                token.as_ptr(),
                token.len() as u64,
                reload,
                dirty,
                &mut flags,
            )
        };
        (status, flags)
    }

    #[test]
    fn abi_lifecycle_flags_and_acknowledgement() {
        let handle = create();
        assert_eq!(begin(handle, b"disk", 1, 1), (ViemStatus::Ok, 7));
        assert_eq!(begin(handle, b"other", 1, 0), (ViemStatus::Ok, 0));
        assert_eq!(
            unsafe { viem_external_file_review_finish(handle, b"disk".as_ptr(), 4, 1) },
            ViemStatus::Ok
        );
        assert_eq!(begin(handle, b"disk", 1, 0), (ViemStatus::Ok, 0));
        assert_eq!(viem_external_file_review_reset(handle), ViemStatus::Ok);
        assert_eq!(
            begin(handle, b"disk", 0, 1),
            (ViemStatus::Ok, VIEM_EXTERNAL_FILE_REVIEW_PRESENT)
        );
        assert_eq!(viem_external_file_review_destroy(handle), ViemStatus::Ok);
        assert_eq!(
            viem_external_file_review_reset(handle),
            ViemStatus::InvalidHandle
        );
        assert_eq!(
            viem_external_file_review_destroy(handle),
            ViemStatus::InvalidHandle
        );
        let next = create();
        assert_ne!(next, handle);
        assert_eq!(viem_external_file_review_destroy(next), ViemStatus::Ok);
    }

    #[test]
    fn abi_rejects_bad_buffers_tokens_flags_and_handles_without_mutation() {
        assert_eq!(
            unsafe { viem_external_file_review_create(std::ptr::null_mut()) },
            ViemStatus::NullPointer
        );
        let handle = create();
        assert_eq!(begin(0, b"disk", 1, 0), (ViemStatus::InvalidHandle, 0));
        assert_eq!(begin(handle, &[], 1, 0), (ViemStatus::InvalidArgument, 0));
        assert_eq!(
            begin(
                handle,
                &vec![0; MAX_EXTERNAL_FILE_OBSERVATION_BYTES + 1],
                1,
                0
            ),
            (ViemStatus::InvalidArgument, 0)
        );
        assert_eq!(
            begin(handle, b"disk", 2, 0),
            (ViemStatus::InvalidArgument, 0)
        );
        let mut storage = 0u32;
        assert_eq!(
            unsafe {
                viem_external_file_review_begin(
                    handle,
                    (&storage as *const u32).cast(),
                    4,
                    1,
                    0,
                    &mut storage,
                )
            },
            ViemStatus::InvalidArgument
        );
        assert_eq!(
            unsafe {
                viem_external_file_review_begin(handle, std::ptr::null(), 4, 1, 0, &mut storage)
            },
            ViemStatus::NullPointer
        );
        assert_eq!(begin(handle, b"disk", 1, 0), (ViemStatus::Ok, 3));
        assert_eq!(viem_external_file_review_destroy(handle), ViemStatus::Ok);
    }

    #[test]
    fn abi_clear_acknowledgement_allows_repeated_external_state_and_preserves_pending() {
        let handle = create();
        assert_eq!(begin(handle, b"first", 1, 0), (ViemStatus::Ok, 3));
        assert_eq!(
            unsafe { viem_external_file_review_finish(handle, b"first".as_ptr(), 5, 1) },
            ViemStatus::Ok
        );
        assert_eq!(
            viem_external_file_review_clear_acknowledged(handle),
            ViemStatus::Ok
        );
        assert_eq!(begin(handle, b"first", 1, 0), (ViemStatus::Ok, 3));
        assert_eq!(
            viem_external_file_review_clear_acknowledged(handle),
            ViemStatus::Ok
        );
        assert_eq!(begin(handle, b"second", 1, 0), (ViemStatus::Ok, 0));
        assert_eq!(
            unsafe { viem_external_file_review_finish(handle, b"first".as_ptr(), 5, 0) },
            ViemStatus::Ok
        );
        assert_eq!(begin(handle, b"first", 1, 0), (ViemStatus::Ok, 3));
        assert_eq!(viem_external_file_review_destroy(handle), ViemStatus::Ok);
        assert_eq!(
            viem_external_file_review_clear_acknowledged(handle),
            ViemStatus::InvalidHandle
        );
    }

    #[test]
    fn abi_busy_turn_preserves_owned_handle_until_completion() {
        let handle = create();
        let lease = checkout_review(handle).unwrap();
        assert_eq!(
            viem_external_file_review_reset(handle),
            ViemStatus::CoreBusy
        );
        assert_eq!(
            viem_external_file_review_clear_acknowledged(handle),
            ViemStatus::CoreBusy
        );
        assert_eq!(
            viem_external_file_review_destroy(handle),
            ViemStatus::CoreBusy
        );
        drop(lease);
        assert_eq!(viem_external_file_review_reset(handle), ViemStatus::Ok);
        assert_eq!(viem_external_file_review_destroy(handle), ViemStatus::Ok);
    }
}

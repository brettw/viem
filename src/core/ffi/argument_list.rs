//! Batch-free portable argument selection; file identity stays with the host.

use crate::command::argument_list::{resolve_argument, ArgumentListError, ExArgumentTarget};

pub const VIEM_ARGUMENT_NEXT: u32 = 1;
pub const VIEM_ARGUMENT_PREVIOUS: u32 = 2;
pub const VIEM_ARGUMENT_FIRST: u32 = 3;
pub const VIEM_ARGUMENT_LAST: u32 = 4;
pub const VIEM_ARGUMENT_INDEX: u32 = 5;
pub const VIEM_ARGUMENT_CURRENT: u32 = 6;

pub const VIEM_ARGUMENT_RESOLVE_OK: u32 = 0;
pub const VIEM_ARGUMENT_RESOLVE_EMPTY: u32 = 1;
pub const VIEM_ARGUMENT_RESOLVE_BEFORE_FIRST: u32 = 2;
pub const VIEM_ARGUMENT_RESOLVE_AFTER_LAST: u32 = 3;
pub const VIEM_ARGUMENT_RESOLVE_INVALID_INDEX: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViemArgumentResolution {
    pub status: u32,
    pub reserved: u32,
    /// Zero-based destination, meaningful only on success.
    pub index: u64,
}

/// `u64::MAX` indicates an absent current/remembered zero-based index.
#[no_mangle]
pub extern "C" fn viem_argument_list_resolve(
    length: u64,
    current_index: u64,
    remembered_index: u64,
    command: u32,
    count: u64,
) -> ViemArgumentResolution {
    let target = match command {
        VIEM_ARGUMENT_NEXT => Some(ExArgumentTarget::Next(count)),
        VIEM_ARGUMENT_PREVIOUS => Some(ExArgumentTarget::Previous(count)),
        VIEM_ARGUMENT_FIRST => Some(ExArgumentTarget::First),
        VIEM_ARGUMENT_LAST => Some(ExArgumentTarget::Last),
        VIEM_ARGUMENT_INDEX => Some(ExArgumentTarget::Index(count)),
        VIEM_ARGUMENT_CURRENT => Some(ExArgumentTarget::Current),
        _ => None,
    };
    let result = target
        .ok_or(ArgumentListError::InvalidIndex)
        .and_then(|target| {
            resolve_argument(
                length,
                (current_index != u64::MAX).then_some(current_index),
                (remembered_index != u64::MAX).then_some(remembered_index),
                target,
            )
        });
    match result {
        Ok(index) => ViemArgumentResolution {
            status: VIEM_ARGUMENT_RESOLVE_OK,
            reserved: 0,
            index,
        },
        Err(error) => ViemArgumentResolution {
            status: match error {
                ArgumentListError::Empty => VIEM_ARGUMENT_RESOLVE_EMPTY,
                ArgumentListError::BeforeFirst => VIEM_ARGUMENT_RESOLVE_BEFORE_FIRST,
                ArgumentListError::AfterLast => VIEM_ARGUMENT_RESOLVE_AFTER_LAST,
                ArgumentListError::InvalidIndex => VIEM_ARGUMENT_RESOLVE_INVALID_INDEX,
            },
            reserved: 0,
            index: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_resolution_exports_distinct_errors_and_current_precedence() {
        assert_eq!(
            viem_argument_list_resolve(5, 1, 3, VIEM_ARGUMENT_NEXT, 2).index,
            3
        );
        assert_eq!(
            viem_argument_list_resolve(5, u64::MAX, 3, VIEM_ARGUMENT_PREVIOUS, 2).index,
            1
        );
        assert_eq!(
            viem_argument_list_resolve(0, u64::MAX, u64::MAX, VIEM_ARGUMENT_FIRST, 1).status,
            VIEM_ARGUMENT_RESOLVE_EMPTY
        );
        assert_eq!(
            viem_argument_list_resolve(2, 0, 0, VIEM_ARGUMENT_PREVIOUS, 1).status,
            VIEM_ARGUMENT_RESOLVE_BEFORE_FIRST
        );
        assert_eq!(
            viem_argument_list_resolve(2, 1, 1, VIEM_ARGUMENT_NEXT, u64::MAX).status,
            VIEM_ARGUMENT_RESOLVE_AFTER_LAST
        );
        assert_eq!(
            viem_argument_list_resolve(2, 0, 0, 999, 1).status,
            VIEM_ARGUMENT_RESOLVE_INVALID_INDEX
        );
    }
}

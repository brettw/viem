//! Accounted Tree-sitter allocation hooks. Limits are observed at cooperative
//! checkpoints, never implemented by returning null into a native parser.
//! Scanner allocations that bypass Tree-sitter's allocator remain outside this
//! account and require process isolation for enforceable limits.
use std::{
    alloc::{alloc, dealloc, realloc, Layout},
    cell::Cell,
    ffi::c_void,
    marker::PhantomData,
    rc::Rc,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Once,
    },
};

static INITIALIZE: Once = Once::new();
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
const GLOBAL_SOFT_BYTES: usize = 1024 * 1024 * 1024;
thread_local! { static CURRENT: Cell<*const Account> = const { Cell::new(std::ptr::null()) }; }

#[derive(Default, Debug)]
pub struct Account {
    live: AtomicUsize,
    peak: AtomicUsize,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllocationMetrics {
    pub retained_bytes: usize,
    pub peak_bytes: usize,
}
impl Account {
    pub fn metrics(&self) -> AllocationMetrics {
        AllocationMetrics {
            retained_bytes: self.live.load(Ordering::Relaxed),
            peak_bytes: self.peak.load(Ordering::Relaxed),
        }
    }
    pub fn exceeded(&self, limit: usize) -> bool {
        self.live.load(Ordering::Relaxed) > limit
            || LIVE.load(Ordering::Relaxed) > GLOBAL_SOFT_BYTES
    }
}
pub fn global_metrics() -> AllocationMetrics {
    AllocationMetrics {
        retained_bytes: LIVE.load(Ordering::Relaxed),
        peak_bytes: PEAK.load(Ordering::Relaxed),
    }
}
/// Must run before any native Tree-sitter allocation in this process.
/// All provider entry points call this; platform integrations that use native
/// Tree-sitter directly must call it before creating any such object.
pub fn initialize() {
    INITIALIZE.call_once(|| unsafe {
        tree_sitter::set_allocator(Some(malloc), Some(calloc), Some(resize), Some(free));
    });
}
pub struct Scope {
    previous: *const Account,
    _owner: Arc<Account>,
    _thread: PhantomData<Rc<()>>,
}
impl Scope {
    pub fn new(account: &Arc<Account>) -> Self {
        initialize();
        let previous = CURRENT.with(|current| current.replace(Arc::as_ptr(account)));
        Self {
            previous,
            _owner: account.clone(),
            _thread: PhantomData,
        }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|current| current.set(self.previous));
    }
}

// The maximum alignment required by Tree-sitter's C objects is 16 bytes on the
// supported ABIs. Keeping an allocation header avoids an unbounded side map.
#[repr(C, align(16))]
struct Header {
    bytes: usize,
    owner: *const Account,
}
fn layout(bytes: usize) -> Option<Layout> {
    Layout::from_size_align(bytes.checked_add(std::mem::size_of::<Header>())?, 16).ok()
}
unsafe fn charge(owner: *const Account, bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
    if !owner.is_null() {
        let live = (*owner).live.fetch_add(bytes, Ordering::Relaxed) + bytes;
        (*owner).peak.fetch_max(live, Ordering::Relaxed);
    }
}
unsafe fn uncharge(owner: *const Account, bytes: usize) {
    LIVE.fetch_sub(bytes, Ordering::Relaxed);
    if !owner.is_null() {
        (*owner).live.fetch_sub(bytes, Ordering::Relaxed);
    }
}
unsafe extern "C" fn malloc(bytes: usize) -> *mut c_void {
    let Some(layout) = layout(bytes) else {
        return std::ptr::null_mut();
    };
    let header = alloc(layout).cast::<Header>();
    if header.is_null() {
        return std::ptr::null_mut();
    }
    let owner = CURRENT.with(Cell::get);
    if !owner.is_null() {
        Arc::increment_strong_count(owner);
    }
    header.write(Header { bytes, owner });
    charge(owner, layout.size());
    header.add(1).cast()
}
unsafe extern "C" fn calloc(count: usize, bytes: usize) -> *mut c_void {
    let Some(bytes) = count.checked_mul(bytes) else {
        return std::ptr::null_mut();
    };
    let pointer = malloc(bytes);
    if !pointer.is_null() {
        pointer.cast::<u8>().write_bytes(0, bytes);
    }
    pointer
}
unsafe extern "C" fn resize(pointer: *mut c_void, bytes: usize) -> *mut c_void {
    if pointer.is_null() {
        return malloc(bytes);
    }
    if bytes == 0 {
        free(pointer);
        return std::ptr::null_mut();
    }
    let old = pointer.cast::<Header>().sub(1);
    let Some(new_layout) = layout(bytes) else {
        return std::ptr::null_mut();
    };
    let old_layout = layout((*old).bytes).expect("existing native allocation");
    let owner = (*old).owner;
    let next = realloc(old.cast(), old_layout, new_layout.size()).cast::<Header>();
    if next.is_null() {
        return std::ptr::null_mut();
    }
    uncharge(owner, old_layout.size());
    (*next).bytes = bytes;
    charge(owner, new_layout.size());
    next.add(1).cast()
}
unsafe extern "C" fn free(pointer: *mut c_void) {
    if pointer.is_null() {
        return;
    }
    let header = pointer.cast::<Header>().sub(1);
    let layout = layout((*header).bytes).expect("existing native allocation");
    let owner = (*header).owner;
    uncharge(owner, layout.size());
    dealloc(header.cast(), layout);
    if !owner.is_null() {
        drop(Arc::from_raw(owner));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocations_reallocation_and_cross_thread_free_keep_owners_alive() {
        initialize();
        let owner = Arc::new(Account::default());
        let weak = Arc::downgrade(&owner);
        let pointer = {
            let _scope = Scope::new(&owner);
            unsafe {
                let pointer = calloc(4, 8);
                assert_eq!(*pointer.cast::<u64>(), 0);
                let pointer = resize(pointer, 1024);
                assert_eq!(
                    owner.metrics().retained_bytes,
                    1024 + std::mem::size_of::<Header>()
                );
                pointer as usize
            }
        };
        drop(owner);
        assert!(weak.upgrade().is_some());
        std::thread::spawn(move || unsafe {
            free(pointer as *mut c_void);
        })
        .join()
        .unwrap();
        assert!(weak.upgrade().is_none());
    }
}

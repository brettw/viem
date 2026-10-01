//! Frontend-owned pane geometry. Registry locks only lease serial ownership;
//! tree operations and copying snapshots happen after releasing the lock.
use super::*;
use crate::layout::panes::{Axis, Error, PaneLayout, MAX_PANES};
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ViemPaneChrome {
    pub id: u64,
    pub status_height: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ViemPaneFrame {
    pub id: u64,
    pub kind: u32,
    pub flags: u32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ViemPaneSnapshot {
    pub count: u64,
    pub minimum_width: f64,
    pub minimum_height: f64,
}
struct Registry {
    next: u64,
    states: HashMap<u64, Option<PaneLayout>>,
}
fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| {
        Mutex::new(Registry {
            next: 1,
            states: HashMap::new(),
        })
    })
}
struct Lease {
    id: u64,
    state: Option<PaneLayout>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut r) = registry().lock() {
            if let Some(entry @ None) = r.states.get_mut(&self.id) {
                *entry = self.state.take();
            }
        }
    }
}
fn checkout(id: u64) -> Result<Lease, ViemStatus> {
    let mut r = registry().lock().map_err(|_| ViemStatus::InternalError)?;
    let entry = r.states.get_mut(&id).ok_or(ViemStatus::InvalidHandle)?;
    Ok(Lease {
        id,
        state: Some(entry.take().ok_or(ViemStatus::CoreBusy)?),
    })
}
fn error(e: Error) -> ViemStatus {
    match e {
        Error::InvalidPane => ViemStatus::InvalidArgument,
        Error::NoRoom => ViemStatus::PolicyRequired,
        Error::TooManyPanes => ViemStatus::ResourceExhausted,
        Error::Unsupported => ViemStatus::UnsupportedOperation,
    }
}
fn axis(value: u32) -> Result<Axis, ViemStatus> {
    match value {
        0 => Ok(Axis::Height),
        1 => Ok(Axis::Width),
        _ => Err(ViemStatus::InvalidArgument),
    }
}
/// # Safety
/// Output must be aligned and writable for one handle.
#[no_mangle]
pub unsafe extern "C" fn viem_pane_layout_create(out: *mut u64) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out, 1)?;
        let id = {
            let mut r = registry().lock().map_err(|_| ViemStatus::InternalError)?;
            let id = r.next;
            if id == 0 {
                return Err(ViemStatus::ResourceExhausted);
            }
            r.next = id.checked_add(1).unwrap_or(0);
            r.states.insert(id, Some(PaneLayout::default()));
            id
        };
        unsafe { out.write(id) };
        Ok(())
    })
}
#[no_mangle]
pub extern "C" fn viem_pane_layout_destroy(handle: u64) -> ViemStatus {
    ffi_boundary(|| {
        let removed = {
            let mut r = registry().lock().map_err(|_| ViemStatus::InternalError)?;
            match r.states.get(&handle) {
                None => return Err(ViemStatus::InvalidHandle),
                Some(None) => return Err(ViemStatus::CoreBusy),
                Some(Some(_)) => r.states.remove(&handle),
            }
        };
        drop(removed);
        Ok(())
    })
}
/// # Safety
/// Chrome identifies count readable entries. Count must equal the live pane count.
#[no_mangle]
pub unsafe extern "C" fn viem_pane_layout_update(
    handle: u64,
    width: f64,
    height: f64,
    chrome: *const ViemPaneChrome,
    count: u64,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(chrome, count)?;
        if count == 0 || count > MAX_PANES as u64 {
            return Err(ViemStatus::InvalidArgument);
        }
        let input = unsafe { std::slice::from_raw_parts(chrome, count as usize) };
        let next: Vec<_> = input.iter().map(|c| (c.id, c.status_height)).collect();
        checkout(handle)?
            .state
            .as_mut()
            .unwrap()
            .update(width, height, &next)
            .map_err(error)
    })
}
/// Copies one complete snapshot. On insufficient capacity only out_snapshot is
/// written. Frames and out_snapshot must be disjoint. No retained snapshot pointers.
/// # Safety
/// Outputs identify aligned writable storage for their stated capacities.
#[no_mangle]
pub unsafe extern "C" fn viem_pane_layout_copy(
    handle: u64,
    frames: *mut ViemPaneFrame,
    capacity: u64,
    out_snapshot: *mut ViemPaneSnapshot,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(frames, capacity)?,
            typed_pointer_region(out_snapshot, 1)?,
        ])?;
        let lease = checkout(handle)?;
        let state = lease.state.as_ref().unwrap();
        let result = state.frames();
        let (w, h) = state.minimum();
        unsafe {
            out_snapshot.write(ViemPaneSnapshot {
                count: result.len() as u64,
                minimum_width: w,
                minimum_height: h,
            })
        };
        if capacity < result.len() as u64 {
            return Err(ViemStatus::BufferTooSmall);
        }
        for (i, f) in result.iter().enumerate() {
            unsafe {
                frames.add(i).write(ViemPaneFrame {
                    id: f.id,
                    kind: u32::from(f.splitter),
                    flags: u32::from(f.status_draggable),
                    x: f.rect.x,
                    y: f.rect.y,
                    width: f.rect.width,
                    height: f.rect.height,
                })
            }
        }
        Ok(())
    })
}
#[no_mangle]
pub extern "C" fn viem_pane_layout_can_split(
    handle: u64,
    pane: u64,
    orientation: u32,
    status_height: f64,
) -> ViemStatus {
    ffi_boundary(|| {
        checkout(handle)?
            .state
            .as_ref()
            .unwrap()
            .can_split(pane, axis(orientation)?, status_height)
            .map_err(error)
    })
}
/// # Safety
/// out_pane is aligned writable storage. Failed splits leave it and geometry unchanged.
#[no_mangle]
pub unsafe extern "C" fn viem_pane_layout_split(
    handle: u64,
    pane: u64,
    orientation: u32,
    status_height: f64,
    out_pane: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_pane, 1)?;
        let id = checkout(handle)?
            .state
            .as_mut()
            .unwrap()
            .split(pane, axis(orientation)?, status_height)
            .map_err(error)?;
        unsafe { out_pane.write(id) };
        Ok(())
    })
}
#[no_mangle]
pub extern "C" fn viem_pane_layout_remove(handle: u64, pane: u64) -> ViemStatus {
    ffi_boundary(|| {
        checkout(handle)?
            .state
            .as_mut()
            .unwrap()
            .remove(pane)
            .map_err(error)
    })
}
/// Each delta is incremental, including deltas blocked at an edge. kind=0 for
/// a pane's status bar, kind=1 for a vertical splitter from the current snapshot.
#[no_mangle]
pub extern "C" fn viem_pane_layout_drag(handle: u64, id: u64, kind: u32, delta: f64) -> ViemStatus {
    ffi_boundary(|| {
        if kind > 1 {
            return Err(ViemStatus::InvalidArgument);
        }
        checkout(handle)?
            .state
            .as_mut()
            .unwrap()
            .drag(id, kind == 0, delta)
            .map_err(error)
    })
}
/// Geometry operations: 1 focus(direction in count, steps in value, caret x/y);
/// 2 rotate(steps in count, flags:0 forward/1 backward); 3 exchange(one-based count, zero=next);
/// 4 move to edge(direction in count); 5 resize(axis in count:0 height/1 width,
/// value in DIPs; flags 1=relative,2=maximize); 6 equalize(count:0 both/1 height/2 width).
/// Direction codes:0 down,1 up,2 left,3 right. No document or focus state is owned here.
/// # Safety
/// out_focus is aligned writable storage; only written on success.
#[no_mangle]
pub unsafe extern "C" fn viem_pane_layout_action(
    handle: u64,
    pane: u64,
    operation: u32,
    count: u64,
    value: f64,
    flags: u32,
    x: f64,
    y: f64,
    out_focus: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_focus, 1)?;
        if !value.is_finite() || !x.is_finite() || !y.is_finite() || flags > 3 {
            return Err(ViemStatus::InvalidArgument);
        }
        let mut lease = checkout(handle)?;
        let p = lease.state.as_mut().unwrap();
        p.pane_rect(pane).map_err(error)?;
        let focus = match operation {
            1 if count <= 3 => p
                .focus(pane, count as u32, (value.max(0.0) as usize).max(1), (x, y))
                .map_err(error)?,
            2 if flags <= 1 => {
                p.rotate(pane, count, flags == 0).map_err(error)?;
                pane
            }
            3 => {
                p.reorder(
                    pane,
                    None,
                    (count > 0)
                        .then_some(usize::try_from(count).map_err(|_| ViemStatus::LengthOverflow)?),
                )
                .map_err(error)?;
                pane
            }
            4 if count <= 3 => {
                p.move_edge(pane, count as u32).map_err(error)?;
                pane
            }
            5 => {
                p.resize(
                    pane,
                    axis(u32::try_from(count).map_err(|_| ViemStatus::InvalidArgument)?)?,
                    if flags & 2 != 0 { None } else { Some(value) },
                    flags & 1 != 0,
                )
                .map_err(error)?;
                pane
            }
            6 if count <= 2 => {
                p.equalize(match count {
                    1 => Some(Axis::Height),
                    2 => Some(Axis::Width),
                    _ => None,
                });
                pane
            }
            _ => return Err(ViemStatus::InvalidArgument),
        };
        unsafe { out_focus.write(focus) };
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pane_abi_ownership_admission_and_snapshot_buffers() {
        let mut handle = 0;
        assert_eq!(
            unsafe { viem_pane_layout_create(&mut handle) },
            ViemStatus::Ok
        );
        let input = ViemPaneChrome {
            id: 1,
            status_height: 25.0,
        };
        assert_eq!(
            unsafe { viem_pane_layout_update(handle, 204.0, 600.0, &input, 1) },
            ViemStatus::Ok
        );
        let mut next = 999;
        assert_eq!(
            unsafe { viem_pane_layout_split(handle, 1, 1, 25.0, &mut next) },
            ViemStatus::PolicyRequired
        );
        assert_eq!(next, 999);
        assert_eq!(
            unsafe { viem_pane_layout_update(handle, 600.0, 600.0, &input, 1) },
            ViemStatus::Ok
        );
        assert_eq!(
            unsafe { viem_pane_layout_split(handle, 1, 1, 25.0, &mut next) },
            ViemStatus::Ok
        );
        let mut info = ViemPaneSnapshot::default();
        assert_eq!(
            unsafe { viem_pane_layout_copy(handle, std::ptr::null_mut(), 0, &mut info) },
            ViemStatus::BufferTooSmall
        );
        assert_eq!(info.count, 3);
        assert_eq!(info.minimum_width, 205.0);
        let mut frames = [ViemPaneFrame::default(); 3];
        assert_eq!(
            unsafe { viem_pane_layout_copy(handle, frames.as_mut_ptr(), 3, &mut info) },
            ViemStatus::Ok
        );
        let before = frames.map(|f| f.width);
        let invalid = [ViemPaneChrome {
            id: 1,
            status_height: 25.0,
        }; 2];
        assert_eq!(
            unsafe { viem_pane_layout_update(handle, 600.0, 600.0, invalid.as_ptr(), 2) },
            ViemStatus::InvalidArgument
        );
        assert_eq!(
            unsafe { viem_pane_layout_copy(handle, frames.as_mut_ptr(), 3, &mut info) },
            ViemStatus::Ok
        );
        assert_eq!(frames.map(|f| f.width), before);
        let lease = checkout(handle).unwrap();
        assert_eq!(viem_pane_layout_destroy(handle), ViemStatus::CoreBusy);
        drop(lease);
        assert_eq!(viem_pane_layout_destroy(handle), ViemStatus::Ok);
        assert_eq!(
            viem_pane_layout_drag(handle, 1, 0, 12.0),
            ViemStatus::InvalidHandle
        );
        let mut other = 0;
        assert_eq!(
            unsafe { viem_pane_layout_create(&mut other) },
            ViemStatus::Ok
        );
        assert_ne!(other, handle);
        assert_eq!(viem_pane_layout_destroy(other), ViemStatus::Ok);
    }
}

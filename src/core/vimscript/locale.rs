//! Owned collation state: never change the process locale while scripts run.
use std::{
    cmp::Ordering,
    ffi::{c_char, c_int, c_void, CString},
};

type Handle = *mut c_void;

#[cfg(unix)]
unsafe extern "C" {
    fn newlocale(mask: c_int, name: *const c_char, base: Handle) -> Handle;
    fn strcoll_l(a: *const c_char, b: *const c_char, locale: Handle) -> c_int;
    fn freelocale(locale: Handle);
}
#[cfg(windows)]
unsafe extern "C" {
    fn _create_locale(category: c_int, name: *const c_char) -> Handle;
    fn _strcoll_l(a: *const c_char, b: *const c_char, locale: Handle) -> c_int;
    fn _free_locale(locale: Handle);
}

pub(super) struct Collation(Handle);
impl Collation {
    pub fn new(name: &str) -> Self {
        let name = CString::new(name).unwrap_or_else(|_| CString::new("C").unwrap());
        // LC_CTYPE and LC_COLLATE have different category numbers on Darwin
        // and glibc. Other categories retain their default C behavior.
        #[cfg(unix)]
        let handle = unsafe {
            newlocale(
                if cfg!(target_vendor = "apple") { 3 } else { 9 },
                name.as_ptr(),
                std::ptr::null_mut(),
            )
        };
        #[cfg(windows)]
        let handle = unsafe { _create_locale(0, name.as_ptr()) };
        Self(handle)
    }
    pub fn compare(&self, a: &[u8], b: &[u8]) -> Ordering {
        if self.0.is_null() {
            return a.cmp(b);
        }
        let terminated =
            |s: &[u8]| CString::new(s.split(|b| *b == 0).next().unwrap_or_default()).unwrap();
        let (a, b) = (terminated(a), terminated(b));
        #[cfg(unix)]
        let result = unsafe { strcoll_l(a.as_ptr(), b.as_ptr(), self.0) };
        #[cfg(windows)]
        let result = unsafe { _strcoll_l(a.as_ptr(), b.as_ptr(), self.0) };
        result.cmp(&0)
    }
}
impl Drop for Collation {
    fn drop(&mut self) {
        if !self.0.is_null() {
            #[cfg(unix)]
            unsafe {
                freelocale(self.0);
            }
            #[cfg(windows)]
            unsafe {
                _free_locale(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn c_collation_is_byte_order() {
        let locale = Collation::new("C");
        assert!(!locale.0.is_null());
        assert_eq!(locale.compare(b"z", "é".as_bytes()), Ordering::Less);
        assert_eq!(locale.compare(b"a", b"a"), Ordering::Equal);
    }
    #[cfg(target_vendor = "apple")]
    #[test]
    fn utf8_collation_uses_locale_order() {
        let locale = Collation::new("en_US.UTF-8");
        assert!(!locale.0.is_null());
        assert_eq!(locale.compare(b"a", b"B"), Ordering::Less);
    }
    #[test]
    fn unavailable_locale_falls_back_to_c() {
        let locale = Collation::new("viem-missing-locale");
        assert!(locale.0.is_null());
        assert_eq!(locale.compare(b"Z", b"a"), Ordering::Less);
    }
}

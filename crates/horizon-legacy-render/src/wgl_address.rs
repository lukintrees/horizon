//! WGL may return sentinel values instead of null for unavailable symbols.
use std::ffi::c_void;

pub fn resolve(
    candidate: *const c_void,
    fallback: impl FnOnce() -> *const c_void,
) -> Option<*const c_void> {
    let valid = |address: *const c_void| !matches!(address as usize, 0..=3 | usize::MAX);
    if valid(candidate) {
        Some(candidate)
    } else {
        let address = fallback();
        valid(address).then_some(address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_sentinel_extension_uses_dll_export() {
        let exported = 0x1000usize as *const c_void;
        for invalid in [0, 1, 2, 3, usize::MAX] {
            assert_eq!(
                resolve(invalid as *const c_void, || exported),
                Some(exported)
            );
        }
    }

    #[test]
    fn valid_extension_does_not_need_dll_fallback() {
        let extension = 0x2000usize as *const c_void;
        assert_eq!(resolve(extension, || panic!("not needed")), Some(extension));
    }

    #[test]
    fn no_valid_address_is_not_callable() {
        assert_eq!(resolve(std::ptr::null(), std::ptr::null), None);
        assert_eq!(
            resolve(std::ptr::without_provenance(1), || {
                std::ptr::without_provenance(3)
            }),
            None
        );
    }
}

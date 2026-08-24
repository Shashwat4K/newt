//! C ABI over `newt-core`.
//!
//! Keep this surface narrow and byte-oriented: plain data across the boundary,
//! no callbacks on the hot path, no object graphs, no platform types in
//! signatures. `cbindgen` generates the header from this file.

use std::ffi::c_char;

/// Version of the core, as a NUL-terminated static string.
///
/// The returned pointer is valid for the lifetime of the process and must not
/// be freed by the caller.
#[no_mangle]
pub extern "C" fn newt_version() -> *const c_char {
    // Built at compile time so there is nothing to allocate or free.
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn version_is_valid_c_string() {
        let ptr = newt_version();
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(s, env!("CARGO_PKG_VERSION"));
    }
}

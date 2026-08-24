//! Generates `newt.h` from the `#[no_mangle] extern "C"` surface in `src/`.
//!
//! The header is written straight into the Swift package so there is exactly
//! one source of truth for the ABI and no hand-edited copy to drift.

use std::path::PathBuf;

fn main() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let header = crate_dir
        .join("../../macos/Sources/CNewt/include/newt.h")
        .canonicalize()
        .unwrap_or_else(|_| crate_dir.join("../../macos/Sources/CNewt/include/newt.h"));

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");

    match cbindgen::generate(&crate_dir) {
        Ok(bindings) => {
            bindings.write_to_file(&header);
        }
        // A header-generation failure must not break `cargo build` for people
        // working only on the core; the Swift build is where it actually bites.
        Err(e) => println!("cargo:warning=cbindgen failed to generate {header:?}: {e}"),
    }
}

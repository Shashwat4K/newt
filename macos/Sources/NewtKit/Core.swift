import CNewt

/// Entry point to the Rust core.
///
/// Everything crossing the ABI is wrapped here so the rest of the app never
/// touches raw pointers. Phase 2 grows this into session handles and grid
/// snapshots.
public enum Core {
    /// Version reported by the linked core library.
    public static var version: String {
        // Static storage owned by the core; valid for the process lifetime and
        // must not be freed, so a copying initializer is the correct read.
        String(cString: newt_version())
    }
}

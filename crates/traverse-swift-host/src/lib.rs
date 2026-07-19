//! Minimal C-ABI boundary for the Apple `wasmi` feasibility proof.
//!
//! The production surface remains deliberately small: Swift configures limits
//! and invokes only governed runtime operations through a later safe wrapper.

#![allow(unsafe_code)] // Temporary, scoped C-ABI exception tracked by #771.

use wasmi::StoreLimitsBuilder;

/// Builds the engine-owned memory ceiling used by the feasibility fixture.
///
/// This function proves that the engine limit API is available to a static
/// Apple host without relying on SPI or a watchdog.
#[must_use]
pub fn configured_memory_limit(bytes: usize) -> usize {
    let _limits = StoreLimitsBuilder::new().memory_size(bytes).build();
    bytes
}

/// C-compatible build marker for Swift/XCFramework integration checks.
#[unsafe(no_mangle)]
pub extern "C" fn traverse_swift_host_abi_version() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::configured_memory_limit;

    #[test]
    fn exposes_the_configured_memory_limit() {
        assert_eq!(configured_memory_limit(65_536), 65_536);
    }
}

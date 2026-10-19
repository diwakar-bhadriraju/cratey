//! Cratey — a native Linux AppImage manager.
//!
//! Library crate exposing the domain model, persistence, AppImage inspection,
//! desktop integration and update providers. The binary (`src/main.rs`) is a
//! thin relm4 entry point.

pub mod appimage;
pub mod db;
pub mod error;
pub mod integration;
pub mod model;
pub mod paths;
pub mod ui;
pub mod update;

pub use error::{AppError, Result};

/// Refuse to run as root. AppImages are untrusted input and this application
/// manipulates user application directories; running with elevated privileges
/// would give a malicious AppImage or `.desktop` file an unnecessary attack
/// surface.
///
/// `getuid` is a function pointer (callers pass `|| unsafe { libc::geteuid() }`)
/// so tests can exercise the root and non-root paths without a syscall.
pub fn assert_not_root(getuid: fn() -> u32) -> Result<()> {
    if getuid() == 0 {
        Err(AppError::Config(
            "Cratey must not be run as root.".to_string(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assert_not_root_rejects_zero() {
        assert!(assert_not_root(|| 0).is_err());
    }

    #[test]
    fn assert_not_root_accepts_1000() {
        assert!(assert_not_root(|| 1000).is_ok());
    }
}

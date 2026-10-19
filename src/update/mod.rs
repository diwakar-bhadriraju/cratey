//! Update providers (GitHub Releases primary; manual URL; zsync deferred).

pub mod apply;
pub mod github;
pub mod manual;
pub mod zsync;

use crate::error::Result;
use crate::model::ManagedApp;

/// Result of an update check. Synchronous (no async runtime in v1).
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateOutcome {
    NoUpdate,
    Available(NewRelease),
    Failed(String),
}

/// A concrete downloadable release candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct NewRelease {
    pub version: String,
    pub download_url: String,
    pub size_bytes: u64,
    pub sha256: Option<String>,
    pub sig: Option<SignatureInfo>,
}

/// Optional detached signature for the binary (future verification).
#[derive(Debug, Clone, PartialEq)]
pub struct SignatureInfo {
    pub kind: SignatureKind,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SignatureKind {
    GpgAsc,
    Embedded,
    Unknown,
}

/// Phase-1 provider abstraction.
pub trait UpdateProvider {
    /// Checks for a newer release. Blocks (network + disk); returns an outcome.
    fn check(&self, app: &ManagedApp) -> Result<UpdateOutcome>;
    /// Applies `new` in place for `app`. Blocks until done.
    fn apply(&self, app: &mut ManagedApp, new: NewRelease) -> Result<()>;
}

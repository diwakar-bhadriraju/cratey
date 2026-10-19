//! Zsync-based delta update provider — Phase 2 deferred.
//!
//! The trait impl exists as a stable hook; v1 always returns "not implemented".

use crate::error::{AppError, Result};
use crate::model::ManagedApp;
use crate::update::{NewRelease, UpdateOutcome, UpdateProvider};

/// Phase-2 zsync provider. Not wired up in v1.
pub struct ZsyncProvider;

impl UpdateProvider for ZsyncProvider {
    fn check(&self, _app: &ManagedApp) -> Result<UpdateOutcome> {
        Ok(UpdateOutcome::Failed(
            "Zsync-based updates are not yet implemented in this build".to_string(),
        ))
    }

    fn apply(&self, _app: &mut ManagedApp, _new: NewRelease) -> Result<()> {
        Err(AppError::Update(
            "Zsync-based updates are not yet implemented in this build".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_not_implemented() {
        let (dir, app) = crate::update::apply::tests::test_app();
        let _ = dir;
        match ZsyncProvider.check(&app) {
            Ok(UpdateOutcome::Failed(m)) => assert!(m.contains("not yet implemented")),
            other => panic!("unexpected {other:?}"),
        }
    }
}

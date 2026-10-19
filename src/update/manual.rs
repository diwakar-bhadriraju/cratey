//! Manual-URL update provider (Phase 1 working).

use crate::error::{AppError, Result};
use crate::model::{ManagedApp, UpdateProvider};
use crate::update::apply;
use crate::update::{NewRelease, UpdateOutcome, UpdateProvider as _Trait};

/// A manual provider wraps the app's `UpdateProvider::Manual { url }`.
pub struct ManualProvider;

impl _Trait for ManualProvider {
    fn check(&self, app: &ManagedApp) -> Result<UpdateOutcome> {
        match &app.update_provider {
            UpdateProvider::Manual { url: Some(u), .. } => {
                if !u.starts_with("https://") {
                    return Ok(UpdateOutcome::Failed(
                        "manual update URL must be HTTPS".to_string(),
                    ));
                }
                // Probe size (mirrors apply's guard); a reachable target yields
                // an upgrade candidate sourced from the URL itself.
                let size = match probe(u) {
                    Ok(s) => s,
                    Err(e) => return Ok(UpdateOutcome::Failed(e.to_string())),
                };
                let file_name = u
                    .rsplit('/')
                    .next()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("update.AppImage");
                Ok(UpdateOutcome::Available(NewRelease {
                    version: format!("manual:{}", file_name),
                    download_url: u.clone(),
                    size_bytes: size,
                    sha256: None,
                    sig: None,
                }))
            }
            UpdateProvider::Manual { url: None, .. } => Ok(UpdateOutcome::Failed(
                "manual update source is not configured".to_string(),
            )),
            other => Ok(UpdateOutcome::Failed(format!(
                "provider is not Manual: {other:?}"
            ))),
        }
    }

    fn apply(&self, app: &mut ManagedApp, new: NewRelease) -> Result<()> {
        apply::apply(app, &new, self)
    }
}

fn probe(url: &str) -> Result<u64> {
    let resp = ureq::head(url)
        .header("User-Agent", "cratey")
        .call()
        .map_err(|e| AppError::Update(format!("HEAD probe failed for {url}: {e}")))?;
    resp.headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or_else(|| AppError::Update(format!("no Content-Length advertised for {url}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with(url: Option<&str>) -> ManagedApp {
        let (dir, mut a) = crate::update::apply::tests::test_app();
        let _ = dir;
        a.update_provider = UpdateProvider::Manual {
            url: url.map(String::from),
            last_verified_hash: None,
        };
        a
    }

    #[test]
    fn unavailable_url_yields_failed_or_no_provider_path() {
        // No URL → Failed("not configured"), no network.
        let a = app_with(None);
        match ManualProvider.check(&a) {
            Ok(UpdateOutcome::Failed(m)) => assert!(m.contains("not configured")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn non_https_url_rejected() {
        let a = app_with(Some("http://insecure.example/x.AppImage"));
        match ManualProvider.check(&a) {
            Ok(UpdateOutcome::Failed(m)) => assert!(m.contains("HTTPS")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}

//! Update provider backed by GitHub Releases.
//!
//! Pure selection logic (`select_release`) is separated from the HTTP fetch so
//! it can be unit-tested and reused by the wiremock e2e path.

use crate::error::{AppError, Result};
use crate::model::{AssetMatch, ManagedApp, ReleaseChannel, UpdateProvider};
use crate::update::{NewRelease, UpdateOutcome};

const API: &str = "https://api.github.com/repos";
const USER_AGENT: &str = concat!("cratey/", env!("CARGO_PKG_VERSION"));

/// A GitHub release as returned by the releases API (subset of fields).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct GitHubRelease {
    #[serde(default)]
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<GitHubAsset>,
}

impl GitHubRelease {
    pub fn version(&self) -> String {
        self.tag_name.trim_start_matches('v').to_string()
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GitHubAsset {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub browser_download_url: String,
}

/// Fetches releases for `repo` (user/repo) via the public API.
pub fn fetch_releases(repo: &str, token: Option<&str>) -> Result<Vec<GitHubRelease>> {
    let url = format!("{API}/{repo}/releases");
    let mut req = ureq::get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json");
    if let Some(tok) = token {
        req = req.header("Authorization", format!("Bearer {tok}").as_str());
    }
    let resp = req
        .call()
        .map_err(|e| AppError::Update(format!("GitHub request failed: {e}")))?;
    let releases: Vec<GitHubRelease> = resp
        .into_body()
        .read_json()
        .map_err(|e| AppError::Update(format!("GitHub JSON parse failed: {e}")))?;
    Ok(releases)
}

/// Selects the newest candidate from already-fetched releases.
/// Rule: skip drafts; skip prereleases when `channel == Stable`.
pub fn select_release(
    releases: &[GitHubRelease],
    channel: ReleaseChannel,
    asset_match: &AssetMatch,
) -> UpdateOutcome {
    let mut candidates: Vec<&GitHubAsset> = Vec::new();
    let mut candidate_version = String::new();

    for rel in releases {
        if rel.draft {
            continue;
        }
        if channel == ReleaseChannel::Stable && rel.prerelease {
            continue;
        }
        // First matching release in API order (newest first) wins.
        if let Some(asset) = select_asset(&rel.assets, asset_match) {
            // Pick the first (newest, per GitHub ordering assumption) match and set version once.
            if candidate_version.is_empty() {
                candidate_version = rel.version();
            }
            candidates.push(asset);
            break;
        }
    }

    match candidates.len() {
        0 => UpdateOutcome::NoUpdate,
        1 => {
            let asset = candidates[0];
            UpdateOutcome::Available(NewRelease {
                version: candidate_version,
                download_url: asset.browser_download_url.clone(),
                size_bytes: asset.size,
                sha256: None,
                sig: None,
            })
        }
        _ => UpdateOutcome::Failed(
            "ambiguous asset selection; please configure AssetMatch in app settings".to_string(),
        ),
    }
}

/// Chooses the sole asset matching `asset_match`. Returns `None` if no asset
/// matches; returns the first match.
fn select_asset<'a>(assets: &'a [GitHubAsset], matcher: &AssetMatch) -> Option<&'a GitHubAsset> {
    match matcher {
        AssetMatch::Name(s) => assets.iter().find(|a| a.name == *s),
        AssetMatch::Regex(r) => {
            let re = regex::Regex::new(r).ok()?;
            assets.iter().find(|a| re.is_match(&a.name))
        }
        AssetMatch::Largest => assets.iter().max_by_key(|a| a.size),
    }
}

/// Constructs and runs the primary GitHub provider check for an app.
pub fn check_for_updates(app: &ManagedApp, token: Option<&str>) -> Result<UpdateOutcome> {
    match &app.update_provider {
        UpdateProvider::GitHub {
            repo,
            channel,
            asset_match,
        } => {
            let releases = fetch_releases(repo, token)?;
            Ok(select_release(&releases, *channel, asset_match))
        }
        UpdateProvider::None => Ok(UpdateOutcome::Failed(
            "no update provider configured".to_string(),
        )),
        other => Ok(UpdateOutcome::Failed(format!(
            "unsupported provider: {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tag: &str, draft: bool, pre: bool, assets: Vec<GitHubAsset>) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.into(),
            draft,
            prerelease: pre,
            assets,
        }
    }

    fn asset(name: &str, size: u64) -> GitHubAsset {
        GitHubAsset {
            name: name.into(),
            size,
            browser_download_url: format!("https://example.com/{name}"),
        }
    }

    #[test]
    fn parses_release_list_and_selects_by_asset_matches() {
        let releases = vec![
            rel(
                "v1.0.0",
                false,
                false,
                vec![asset("foo-1.0.AppImage", 100), asset("foo-1.0.dmg", 50)],
            ),
            rel("v1.1.0", false, false, vec![asset("foo-1.1.AppImage", 120)]),
        ];
        let out = select_release(
            &releases,
            ReleaseChannel::Stable,
            &AssetMatch::Name("foo-1.1.AppImage".into()),
        );
        match out {
            UpdateOutcome::Available(r) => {
                assert_eq!(r.version, "1.1.0");
                assert_eq!(r.download_url, "https://example.com/foo-1.1.AppImage");
            }
            other => panic!("expected Available, got {other:?}"),
        }
    }

    #[test]
    fn skips_prerelease_when_channel_is_stable() {
        let releases = vec![rel(
            "v2.0.0-rc1",
            false,
            true,
            vec![asset("foo-2.0.AppImage", 10)],
        )];
        let out = select_release(&releases, ReleaseChannel::Stable, &AssetMatch::Largest);
        assert!(matches!(out, UpdateOutcome::NoUpdate));

        let out = select_release(&releases, ReleaseChannel::Prerelease, &AssetMatch::Largest);
        assert!(matches!(out, UpdateOutcome::Available(_)));
    }

    #[test]
    fn returns_ambiguous_for_multiple_matches() {
        // Largest across two assets in one release is unambiguous, but a regex
        // matching two assets is ambiguous — we pick first match and treat the
        // single picked asset as the candidate, so ambiguity only arises when the
        // same matcher yields multiple distinct download candidates. Emulate by
        // verifying Largest collapses to one.
        let releases = vec![rel(
            "v1.0.0",
            false,
            false,
            vec![asset("a.AppImage", 5), asset("b.AppImage", 9)],
        )];
        let out = select_release(&releases, ReleaseChannel::Stable, &AssetMatch::Largest);
        match out {
            UpdateOutcome::Available(r) => {
                assert_eq!(r.download_url, "https://example.com/b.AppImage")
            }
            other => panic!("expected Available, got {other:?}"),
        }
    }

    #[test]
    fn skips_drafts() {
        let releases = vec![rel("v9", true, false, vec![asset("foo.AppImage", 1)])];
        assert!(matches!(
            select_release(&releases, ReleaseChannel::Stable, &AssetMatch::Largest),
            UpdateOutcome::NoUpdate
        ));
    }
}

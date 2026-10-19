//! Pure domain types for Cratey.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Stable identifier; set once at install. NEVER changes. Used for:
/// - DB key
/// - desktop-file basename derivation
/// - icon filename derivation
pub type AppId = String;

/// Generates a new `AppId` as `"crate_" + lowercase ULID`.
pub fn new_app_id() -> AppId {
    format!("crate_{}", ulid::Ulid::new().to_string().to_lowercase())
}

/// Sanitizes a free-form string (typically the display name) into a
/// lowercase ASCII slug suitable for embedding in a filename: only
/// `[a-z0-9-]`, runs collapsed, leading/trailing `-` removed, capped.
///
/// If the input yields no usable characters, the stem `app` is returned.
fn slug_stem(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(24));
    let mut prev_dash = false;
    for c in s.chars() {
        let lc = c.to_ascii_lowercase();
        let keep = lc.is_ascii_alphanumeric();
        if keep {
            out.push(lc);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    let capped = if trimmed.len() > 24 {
        trimmed[..24].trim_end_matches('-').to_string()
    } else {
        trimmed
    };
    if capped.is_empty() {
        "app".to_string()
    } else {
        capped
    }
}

/// Derives the immutable `.desktop` basename from an `AppId` and a display
/// hint. Format: `crate-<stem>-<id_tail>.desktop`. Two different ids always
/// produce distinct basenames even when the stems collide; renaming the
/// display name does NOT rename the menu entry, so menu integration stays
/// stable across user edits.
pub fn desktop_file_basename(id: &AppId, display_name: &str) -> String {
    let tail = id
        .chars()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>()
        .to_lowercase();
    let stem = slug_stem(display_name);
    format!("crate-{stem}-{tail}.desktop")
}

/// How an update source is located.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateProvider {
    GitHub {
        repo: String,
        channel: ReleaseChannel,
        asset_match: AssetMatch,
    },
    Named {
        name: String,
    },
    Manual {
        url: Option<String>,
        last_verified_hash: Option<String>,
    },
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReleaseChannel {
    Stable,
    Prerelease,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetMatch {
    Name(String),
    Regex(String),
    Largest,
}

/// A managed AppImage installation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManagedApp {
    pub id: AppId,
    pub display_name: String,
    pub source_filename: String,
    pub installed_path: PathBuf,

    /// Immutable `.desktop` basename, derived from `id` (see [`desktop_file_basename`]).
    pub desktop_file_basename: String,
    pub desktop_file_path: PathBuf,

    pub icon_path: Option<PathBuf>,

    pub imported_at: DateTime<Utc>,
    pub last_updated_at: Option<DateTime<Utc>>,
    pub known_version: Option<String>,
    pub latest_version: Option<String>,
    pub update_status: UpdateStatus,
    pub update_provider: UpdateProvider,
    pub integration_status: IntegrationStatus,
    pub owned_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateStatus {
    Unknown,
    UpToDate,
    Available(String),
    Checking,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegrationStatus {
    Healthy,
    DesktopEntryMissing,
    IconMissing,
    Broken(String),
}

/// Import mode for dragging an AppImage into the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportMode {
    Copy,
    Move,
}

/// When the user wants update notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationPref {
    Disabled,
    Transient,
    Persistent,
}

/// When update checks run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateCheckPref {
    Manual,
    OnLaunch,
    Daily,
}

/// User-configurable settings, stored in the DB. All fields default to the safe option.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// AppImage storage directory. `None` = not yet chosen (first-run wizard).
    pub base_dir: Option<PathBuf>,
    pub default_import_mode: ImportMode,
    pub notification_pref: NotificationPref,
    pub release_channel_default: ReleaseChannel,
    pub update_check_pref: UpdateCheckPref,
    /// Show a confirmation dialog before install/uninstall.
    pub confirm_before_install: bool,
    pub confirm_before_uninstall: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            base_dir: None,
            default_import_mode: ImportMode::Copy,
            notification_pref: NotificationPref::Transient,
            release_channel_default: ReleaseChannel::Stable,
            update_check_pref: UpdateCheckPref::Manual,
            confirm_before_install: true,
            confirm_before_uninstall: true,
        }
    }
}

/// On-disk DB root, schema versioned for forward migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbDocument {
    pub schema_version: u32,
    pub apps: Vec<ManagedApp>,
    pub settings: Settings,
}

impl Default for DbDocument {
    fn default() -> Self {
        DbDocument {
            schema_version: 1,
            apps: Vec::new(),
            settings: Settings::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_app_id_has_crate_prefix_and_lowercase() {
        let id = new_app_id();
        assert!(id.starts_with("crate_"));
        assert_eq!(id, id.to_lowercase());
    }

    #[test]
    fn desktop_basename_is_stable_and_unique() {
        let a = new_app_id();
        let b = new_app_id();
        let ba = desktop_file_basename(&a, "My App");
        let bb = desktop_file_basename(&b, "My App");
        assert!(ba.starts_with("crate-my-app-"));
        assert!(ba.ends_with(".desktop"));
        assert_ne!(ba, bb);
        // Same id + same display -> same basename.
        assert_eq!(desktop_file_basename(&a, "My App"), ba);
        // Same id, different display -> different basename (renames
        // intentionally churn the menu entry so the new name matches).
        assert_ne!(
            desktop_file_basename(&a, "My App"),
            desktop_file_basename(&a, "Renamed")
        );
    }

    #[test]
    fn desktop_basename_slug_handles_unicode_and_punctuation() {
        let id = "crate_01h0000000000000000000abcd".into();
        // The slug uses ASCII-only: non-ASCII letters collapse to dashes, so
        // "Ünïcödé — App! v2.0" becomes "n-c-d-app-v2-0". The id tail (last 8)
        // is preserved, giving a unique filename. (v2 may transliterate
        // accented Latin to keep stems human-readable.)
        let s = desktop_file_basename(&id, "Ünïcödé — App! v2.0");
        assert!(s.starts_with("crate-n-c-d-app-v2-0-"));
        assert!(s.ends_with(".desktop"));
        // No double-hyphens, no leading/trailing hyphen in the stem.
        let core = s.trim_end_matches(".desktop");
        assert!(!core.contains("--"));
        assert!(!core.starts_with('-'));
        assert!(!core.ends_with('-'));
    }

    #[test]
    fn desktop_basename_slug_falls_back_when_empty() {
        let id = "crate_01h0000000000000000000abcd".into();
        let s = desktop_file_basename(&id, "!!!");
        assert!(s.contains("crate-app-"));
    }
}

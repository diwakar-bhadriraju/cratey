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

/// Derives the immutable `.desktop` file basename from an `AppId`:
/// `crate-<last 8 lowercase chars of id>.desktop`. Stable across renames; two
/// distinct ids always produce distinct basenames.
pub fn desktop_file_basename(id: &AppId) -> String {
    let tail = id
        .chars()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>()
        .to_lowercase();
    format!("crate-{tail}.desktop")
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
        let ba = desktop_file_basename(&a);
        let bb = desktop_file_basename(&b);
        assert!(ba.starts_with("crate-"));
        assert!(ba.ends_with(".desktop"));
        assert_ne!(ba, bb);
        // Same id -> same basename.
        assert_eq!(desktop_file_basename(&a), ba);
    }
}

//! XDG locations used by Cratey.
//!
//! All paths follow `${XDG_DATA_HOME:-$HOME/.local/share}/cratey/...`.
//! No escaping to system-wide directories; Cratey is purely per-user.

use std::path::PathBuf;

use directories::BaseDirs;

/// Returns the data root `cratey/` directory, creating it if needed.
pub fn data_dir() -> PathBuf {
    let base = BaseDirs::new()
        .map(|b| b.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("cratey");
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// `${XDG_DATA_HOME}/cratey/apps.json` — the DB document.
pub fn db_file() -> PathBuf {
    data_dir().join("apps.json")
}

/// `${XDG_DATA_HOME}/cratey/.lock` — flock serializes writers.
pub fn lock_file() -> PathBuf {
    data_dir().join(".lock")
}

/// `${XDG_DATA_HOME}/cratey/icons/` — synthesized PNG icons.
pub fn icons_dir() -> PathBuf {
    let dir = data_dir().join("icons");
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// `${XDG_DATA_HOME}/applications/` — `.desktop` files live here.
pub fn applications_dir() -> PathBuf {
    let base = BaseDirs::new()
        .map(|b| b.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("applications");
    std::fs::create_dir_all(&dir).ok();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_dirs_resolve() {
        // db_file must live inside <data>/cratey/apps.json.
        let db = db_file();
        assert_eq!(db.file_name().unwrap(), "apps.json");
        assert!(db.to_string_lossy().contains("/cratey/"));
    }

    #[test]
    fn applications_dir_named_applications() {
        let ad = applications_dir();
        assert_eq!(ad.file_name().unwrap(), "applications");
    }
}

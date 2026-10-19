//! Atomic JSON-on-disk persistence with an exclusive process lock.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use fs2::FileExt;

use crate::error::{AppError, Result};
use crate::model::{DbDocument, ManagedApp};
use crate::paths::{db_file, lock_file};

/// A handle on the process-wide DB lock and in-memory document.
pub struct Db {
    doc: DbDocument,
    _lock: File,
}

impl Db {
    /// Opens (or creates) the DB, taking the exclusive lock. Returns
    /// `Err(Config)` if the lock is already held by another process.
    pub fn open() -> Result<Db> {
        let data_dir = crate::paths::data_dir();
        std::fs::create_dir_all(&data_dir)?;
        let lock_path = lock_file();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        lock.try_lock_exclusive()
            .map_err(|_| AppError::Config("another instance is running".to_string()))?;

        let doc = if db_file().exists() {
            load_document(&db_file())?
        } else {
            DbDocument::default()
        };
        Ok(Db { doc, _lock: lock })
    }

    pub fn document(&self) -> &DbDocument {
        &self.doc
    }

    pub fn document_mut(&mut self) -> &mut DbDocument {
        &mut self.doc
    }

    pub fn apps(&self) -> &[ManagedApp] {
        &self.doc.apps
    }

    pub fn apps_mut(&mut self) -> &mut Vec<ManagedApp> {
        &mut self.doc.apps
    }

    /// Finds an app by id.
    pub fn find(&self, id: &str) -> Option<&ManagedApp> {
        self.doc.apps.iter().find(|a| a.id == id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut ManagedApp> {
        self.doc.apps.iter_mut().find(|a| a.id == id)
    }

    pub fn update<F: FnOnce(&mut ManagedApp)>(&mut self, id: &str, f: F) -> Result<()> {
        let app = self
            .find_mut(id)
            .ok_or_else(|| AppError::Config(format!("app {id} not found")))?;
        f(app);
        self.commit()
    }

    /// Persists the in-memory document to disk atomically: write `apps.json.tmp`
    /// then `rename()` over `apps.json` (atomic on POSIX).
    pub fn commit(&self) -> Result<()> {
        let target = db_file();
        let tmp_path = target.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(&self.doc)?;
        {
            let mut f = File::create(&tmp_path)?;
            f.write_all(json.as_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp_path, &target)?;
        // Best-effort dir sync so the rename is durable.
        if let Ok(d) = File::open(target.parent().unwrap_or(Path::new("."))) {
            let _ = d.sync_all();
        }
        Ok(())
    }
}

fn load_document(path: &Path) -> Result<DbDocument> {
    let mut f = File::open(path)?;
    let mut s = String::new();
    f.read_to_string(&mut s)?;
    Ok(serde_json::from_str(&s)?)
}

/// Writes a document to disk and validates it loads back correctly.
pub fn roundtrip(doc: &DbDocument, path: &Path) -> Result<()> {
    let json = serde_json::to_vec_pretty(doc)?;
    let tmp_path = path.with_extension("tmp");
    {
        let mut f = File::create(&tmp_path)?;
        f.write_all(&json)?;
    }
    std::fs::rename(&tmp_path, path)?;
    let reloaded = load_document(path)?;
    assert_eq!(reloaded, *doc, "roundtrip changed the document");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ImportMode, ManagedApp, Settings};

    fn sample_app(name: &str) -> ManagedApp {
        ManagedApp {
            id: crate::model::new_app_id(),
            display_name: name.to_string(),
            source_filename: format!("{name}.AppImage"),
            installed_path: std::path::PathBuf::from(format!("/tmp/{name}.AppImage")),
            desktop_file_basename: crate::model::desktop_file_basename(
                &crate::model::new_app_id(),
                name,
            ),
            desktop_file_path: std::path::PathBuf::from("/tmp/a.desktop"),
            icon_path: None,
            imported_at: chrono::Utc::now(),
            last_updated_at: None,
            known_version: None,
            latest_version: None,
            update_status: crate::model::UpdateStatus::Unknown,
            update_provider: crate::model::UpdateProvider::None,
            integration_status: crate::model::IntegrationStatus::Healthy,
            owned_files: vec![crate::model::new_app_id().into()],
        }
    }

    #[test]
    fn atomic_write_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps.json");
        let mut doc = DbDocument::default();
        doc.apps.push(sample_app("Alpha"));
        doc.apps.push(sample_app("Beta"));
        doc.settings.base_dir = Some("/tmp/cratey-test-apps".into());
        roundtrip(&doc, &path).unwrap();
        let reloaded = load_document(&path).unwrap();
        assert_eq!(reloaded, doc);
        assert_eq!(reloaded.apps.len(), 2);
    }

    #[test]
    fn concurrent_lock_blocks_second_writer() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join(".lock");
        let f1 = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .unwrap();
        f1.lock_exclusive().unwrap();
        let f2 = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .unwrap();
        assert!(f2.try_lock_exclusive().is_err());
    }

    #[test]
    fn settings_defaults_are_safe() {
        let s = Settings::default();
        assert_eq!(s.default_import_mode, ImportMode::Copy);
        assert!(s.confirm_before_install);
        assert!(s.confirm_before_uninstall);
    }
}

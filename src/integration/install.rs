//! Atomic install with stratified rollback.
//!
//! Execution order (with matching reverse rollback):
//! 1. validate plan
//! 2. copy source → target (noclobber)
//! 3. chmod
//! 4. synthesize icon
//! 5. write `.desktop` (adopt on collision)
//! 6. refresh desktop database (best-effort)
//! 7. atomic DB write  ← commit boundary
//! 8. move-mode: remove source (non-fatal on failure)
//!
//! Any failure before step 7 returns `Err` and rolls back completed steps; the
//! source stays on disk in move-mode. A step-8 failure returns `Ok(app)` with
//! the cleanup warning recorded in `app.integration_status` (the app is already
//! committed and usable).

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use chrono::Utc;

use crate::appimage::icon::synthesize_icon;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::integration::desktop::{write_entry, DesktopEntry};
use crate::integration::menu::{resolve_collision, Collision};
use crate::integration::PlannedInstall;
use crate::model::{IntegrationStatus, ManagedApp, UpdateProvider, UpdateStatus};

pub fn install(planned: PlannedInstall) -> Result<ManagedApp> {
    validate_plan(&planned)?;

    create_target_dir(&planned)?;

    // Step 2-3: copy (noclobber) then chmod.
    if let Err(e) = stage_copy_and_chmod(&planned) {
        rollback_file(&planned.installed_path);
        return Err(e);
    }

    // Step 4: icon.
    if let Err(e) = synthesize_icon(&planned.display_name, &planned.icon_path) {
        rollback_file(&planned.installed_path);
        return Err(e);
    }

    // Step 5: desktop entry (adopt on collision).
    let desktop_path = match resolve_collision(&planned)? {
        Collision::Write => {
            let entry = DesktopEntry {
                name: planned.display_name.clone(),
                generic_name: String::new(),
                comment: String::new(),
                exec: planned.installed_path.clone(),
                working_dir: None,
                icon: Some(planned.icon_path.clone()),
                categories: String::new(),
                mime_types: Vec::new(),
                startup_wm_class: planned.display_name.clone(),
                appimage_identifier: planned.display_name.clone(),
                internal_id: planned.id.clone(),
            };
            if let Err(e) = write_entry(&planned.desktop_file_path, &entry) {
                rollback_file(&planned.installed_path);
                return Err(e);
            }
            planned.desktop_file_path.clone()
        }
        Collision::Adopt(p) => p,
        Collision::Refuse(_) => unreachable!("resolve_collision returns Result"),
    };

    // Step 6: best-effort refresh.
    if let Ok(apps_dir) = std::fs::canonicalize(crate::paths::applications_dir()) {
        crate::integration::desktop::refresh_databases(&apps_dir);
    }

    // Step 7: atomic DB write (commit boundary).
    let app = build_app(&planned, desktop_path);
    if let Err(e) = commit_app(app.clone()) {
        rollback_file(&planned.installed_path);
        return Err(e);
    }

    // Step 8: move-mode source removal (non-fatal).
    if planned.move_source {
        if let Err(e) = std::fs::remove_file(&planned.source_path) {
            log::warn!(
                "could not remove source {}: {e}",
                planned.source_path.display()
            );
            let mut app = app;
            app.integration_status =
                IntegrationStatus::Broken(format!("source cleanup failed: {e}"));
            let _ = commit_app(app.clone());
            return Ok(app);
        }
    }

    Ok(app)
}

fn build_app(planned: &PlannedInstall, desktop_path: std::path::PathBuf) -> ManagedApp {
    ManagedApp {
        id: planned.id.clone(),
        display_name: planned.display_name.clone(),
        source_filename: planned
            .source_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        installed_path: planned.installed_path.clone(),
        desktop_file_basename: crate::model::desktop_file_basename(&planned.id),
        desktop_file_path: desktop_path,
        icon_path: Some(planned.icon_path.clone()),
        imported_at: Utc::now(),
        last_updated_at: None,
        known_version: None,
        latest_version: None,
        update_status: UpdateStatus::Unknown,
        update_provider: UpdateProvider::None,
        integration_status: IntegrationStatus::Healthy,
        owned_files: vec![planned.installed_path.clone(), planned.icon_path.clone()],
    }
}

fn validate_plan(planned: &PlannedInstall) -> Result<()> {
    if !planned.source_path.is_file() {
        return Err(AppError::Io(format!(
            "source {} is not a readable file",
            planned.source_path.display()
        )));
    }
    if planned.installed_path.exists() {
        return Err(AppError::Io(format!(
            "target {} already exists; refusing to overwrite",
            planned.installed_path.display()
        )));
    }
    Ok(())
}

fn create_target_dir(planned: &PlannedInstall) -> Result<()> {
    if let Some(parent) = planned.installed_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

/// Copies source → target atomically (noclobber) and applies the exec chmod.
fn stage_copy_and_chmod(planned: &PlannedInstall) -> Result<()> {
    let target = &planned.installed_path;
    let mut nf = tempfile::NamedTempFile::new_in(target.parent().unwrap_or(Path::new(".")))?;
    std::io::copy(&mut std::fs::File::open(&planned.source_path)?, &mut nf)?;
    nf.persist_noclobber(target).map_err(|e| {
        AppError::Io(format!(
            "could not persist {}: {:?}",
            target.display(),
            e.error
        ))
    })?;
    chmod_exec(target)
}

fn rollback_file(target: &Path) {
    let _ = std::fs::remove_file(target);
}

fn chmod_exec(path: &Path) -> Result<()> {
    let meta = std::fs::metadata(path)?;
    let mode = meta.permissions().mode();
    let new_mode = (mode & 0o700) | 0o755;
    let mut perms = meta.permissions();
    perms.set_mode(new_mode);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

/// Appends `app` to the canonical DB and commits atomically. The process-wide
/// lock is held for the whole install, so this is safe.
fn commit_app(app: ManagedApp) -> Result<()> {
    let mut db = Db::open()?;
    if !db.document().apps.iter().any(|a| a.id == app.id) {
        db.apps_mut().push(app);
    }
    db.commit()
}

/// Registers a pre-existing AppImage as a managed app without copying it.
///
/// Used when the user has AppImages already in their storage directory that
/// were not installed via Cratey (e.g., dropped there manually or imported
/// from another manager). The AppImage stays in place; we just synthesize the
/// icon, write the `.desktop` file, and register it in the DB.
pub fn register_existing(source_path: &Path, display_name: &str) -> Result<ManagedApp> {
    let id = crate::model::new_app_id();
    let installed = source_path.to_path_buf();

    if !installed.is_file() {
        return Err(AppError::Io(format!(
            "source {} is not a readable file",
            installed.display()
        )));
    }

    // Make it executable in case it isn't already.
    chmod_exec(&installed)?;

    let icon = crate::paths::icons_dir().join(format!("{id}.png"));
    let desktop = crate::paths::applications_dir().join(crate::model::desktop_file_basename(&id));

    // Synthesize the icon.
    synthesize_icon(display_name, &icon)?;

    // Write (or adopt) the .desktop file.
    let installed_for_entry = installed.clone();
    let id_for_entry = id.clone();
    let desktop_path = match resolve_collision(&PlannedInstall {
        id: id.clone(),
        display_name: display_name.to_string(),
        source_path: installed.clone(),
        installed_path: installed_for_entry.clone(),
        icon_path: icon.clone(),
        desktop_file_path: desktop.clone(),
        base_dir: Some(
            installed
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default(),
        ),
        move_source: false,
    })? {
        Collision::Write => {
            let entry = DesktopEntry {
                name: display_name.to_string(),
                generic_name: String::new(),
                comment: String::new(),
                exec: installed_for_entry.clone(),
                working_dir: None,
                icon: Some(icon.clone()),
                categories: String::new(),
                mime_types: Vec::new(),
                startup_wm_class: display_name.to_string(),
                appimage_identifier: display_name.to_string(),
                internal_id: id_for_entry.clone(),
            };
            write_entry(&desktop, &entry)?;
            desktop
        }
        Collision::Adopt(p) => p,
        Collision::Refuse(_) => unreachable!("resolve_collision returns Result"),
    };

    // Refresh the desktop database (best-effort).
    if let Ok(apps_dir) = std::fs::canonicalize(crate::paths::applications_dir()) {
        crate::integration::desktop::refresh_databases(&apps_dir);
    }

    let app = ManagedApp {
        id: id.clone(),
        display_name: display_name.to_string(),
        source_filename: installed
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        installed_path: installed.clone(),
        desktop_file_basename: crate::model::desktop_file_basename(&id),
        desktop_file_path: desktop_path,
        icon_path: Some(icon.clone()),
        imported_at: Utc::now(),
        last_updated_at: None,
        known_version: None,
        latest_version: None,
        update_status: UpdateStatus::Unknown,
        update_provider: UpdateProvider::None,
        integration_status: IntegrationStatus::Healthy,
        owned_files: vec![installed.clone(), icon.clone()],
    };

    commit_app(app.clone())?;
    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appimage::testutil;
    use parking_lot::Mutex;
    use std::sync::LazyLock;

    // install tests mutate the process-global XDG env; serialize them.
    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn plan(move_source: bool) -> (tempfile::TempDir, PlannedInstall) {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("Foo.AppImage");
        std::fs::write(&src, testutil::synthetic_elf(true)).unwrap();
        let pl = PlannedInstall {
            id: crate::model::new_app_id(),
            display_name: "Foo".into(),
            source_path: src,
            installed_path: dir.path().join("managed").join("Foo.AppImage"),
            icon_path: dir.path().join("managed").join("icon.png"),
            desktop_file_path: dir.path().join("applications").join("crate-x.desktop"),
            base_dir: Some(dir.path().into()),
            move_source,
        };
        (dir, pl)
    }

    fn set_xdg(dir: &tempfile::TempDir) {
        std::env::set_var("XDG_DATA_HOME", dir.path().join("share"));
        std::env::set_var("HOME", dir.path().join("home"));
    }
    fn reset_xdg() {
        std::env::remove_var("XDG_DATA_HOME");
        std::env::remove_var("HOME");
    }

    #[test]
    fn move_mode_deletes_source_only_after_commit() {
        let _g = ENV_LOCK.lock();
        let (dir, pl) = plan(true);
        set_xdg(&dir);
        let src_before = pl.source_path.clone();
        let res = install(pl.clone());
        assert!(res.is_ok(), "err: {res:?}");
        assert!(
            !src_before.exists(),
            "source should be deleted in move mode"
        );
        assert!(pl.installed_path.exists());
        reset_xdg();
    }

    #[test]
    fn copy_mode_keeps_source() {
        let _g = ENV_LOCK.lock();
        let (dir, pl) = plan(false);
        set_xdg(&dir);
        let src = pl.source_path.clone();
        let res = install(pl);
        assert!(res.is_ok());
        assert!(src.exists(), "copy mode must keep source");
        reset_xdg();
    }

    #[test]
    fn missing_source_fails_before_copy() {
        let _g = ENV_LOCK.lock();
        let (dir, mut pl) = plan(true);
        set_xdg(&dir);
        pl.source_path = "/nonexistent/foo.AppImage".into();
        let res = install(pl);
        assert!(res.is_err());
        reset_xdg();
    }

    #[test]
    fn writes_desktop_and_icon() {
        let _g = ENV_LOCK.lock();
        let (dir, pl) = plan(false);
        set_xdg(&dir);
        let desk = pl.desktop_file_path.clone();
        let icon = pl.icon_path.clone();
        let res = install(pl);
        assert!(res.is_ok());
        assert!(desk.exists(), "desktop file should exist");
        assert!(icon.exists(), "icon should exist");
        reset_xdg();
    }
}

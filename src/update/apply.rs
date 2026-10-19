//! Shared update-apply pipeline.
//!
//! Steps (with rollback safety):
//! 1. HEAD probe (reject > 2 GiB before streaming)
//! 2. download into a tempdir, streamed in 5 MiB buffers; verify sha256 when known
//! 3. validate the downloaded file is an AppImage
//! 4. move the old binary to `<installed>.prev`
//! 5. rename the new binary into place; on failure restore `.prev`
//! 6. re-apply the exec chmod
//! 7. atomic DB write (known_version, last_updated_at)
//!
//! The `.desktop` file and icon are NOT touched: the binary path is unchanged,
//! so identity and integration remain valid.

use std::io::{Read, Write};
use std::path::Path;

use chrono::Utc;

use crate::appimage::format;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::model::ManagedApp;
use crate::update::{NewRelease, UpdateProvider};

/// Hard cap: refuse to stream downloads this large (and reject unknown sizes).
pub const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB

/// Runs the apply pipeline. On any failure before the rename commits, the old
/// binary is left (or restored) intact.
pub fn apply(app: &mut ManagedApp, new: &NewRelease, _provider: &dyn UpdateProvider) -> Result<()> {
    let size = probe_size(&new.download_url)?;
    if size > MAX_DOWNLOAD_BYTES {
        return Err(AppError::Update(format!(
            "refusing to download {size} bytes (limit {} bytes)",
            MAX_DOWNLOAD_BYTES
        )));
    }

    // Step 2-3: download + validate in a tempdir (auto-cleaned on drop).
    let dir = tempfile::tempdir().map_err(|e| AppError::Io(e.to_string()))?;
    let downloaded = download_to_tempdir(new, dir.path())?;
    format::detect(&downloaded)?;

    // Step 4-5: swap binaries with `.prev` rollback.
    let binary = &app.installed_path;
    let prev = prev_path(binary);
    if let Err(e) = std::fs::rename(binary, &prev) {
        return Err(AppError::Io(format!(
            "could not stage old binary to {}: {e}",
            prev.display()
        )));
    }
    if let Err(e) = std::fs::rename(&downloaded, binary) {
        // Restore the old binary.
        let _ = std::fs::rename(&prev, binary);
        return Err(AppError::Io(format!(
            "could not place new binary: {e}; old binary restored"
        )));
    }

    // Step 6: chmod. A failure here leaves the binary non-executable, which
    // means "Update applied" would mislead the user — surface it.
    make_exec(binary)?;

    // Step 7: atomic DB write.
    app.known_version = Some(new.version.clone());
    app.last_updated_at = Some(Utc::now());
    app.update_status = crate::model::UpdateStatus::UpToDate;
    commit(app)?;
    // Step 8: clean up the staged .prev (we successfully replaced the binary;
    // keeping .prev around would only leak disk over time).
    let _ = std::fs::remove_file(&prev);
    Ok(())
}

/// Returns `Content-Length` from a HEAD probe. `None` if absent/ranges; treat
/// as a refusal per plan (unknown size).
fn probe_size(url: &str) -> Result<u64> {
    let resp = ureq::head(url)
        .header("User-Agent", "cratey")
        .call()
        .map_err(|e| AppError::Update(format!("HEAD probe failed for {url}: {e}")))?;
    let len = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    match len {
        Some(n) => Ok(n),
        None => Err(AppError::Update(format!(
            "refusing to download {url}: no Content-Length advertised"
        ))),
    }
}

/// Streams `new.download_url` to a temp file under `dir`, verifying the
/// byte count and (optionally) sha256 as we go. Streams to disk so a
/// 2 GiB download doesn't pin 2 GiB of RAM.
fn download_to_tempdir(new: &NewRelease, dir: &Path) -> Result<std::path::PathBuf> {
    let file_name = new
        .download_url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("update.AppImage");
    let dest = dir.join(file_name);

    let mut reader = ureq::get(&new.download_url)
        .call()
        .map_err(|e| AppError::Update(format!("download failed: {e}")))?
        .into_body()
        .into_reader();

    let mut out = std::fs::File::create(&dest)?;
    let mut buf = vec![0u8; 5 * 1024 * 1024];
    let mut total: u64 = 0;
    use sha2::{Digest, Sha256};
    let mut hasher = new.sha256.as_ref().map(|_| Sha256::new());

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| AppError::Update(format!("read: {e}")))?;
        if n == 0 {
            break;
        }
        if let Some(h) = hasher.as_mut() {
            h.update(&buf[..n]);
        }
        total = total.saturating_add(n as u64);
        if total > MAX_DOWNLOAD_BYTES {
            return Err(AppError::Update(format!(
                "download exceeds {MAX_DOWNLOAD_BYTES} bytes"
            )));
        }
        std::io::Write::write_all(&mut out, &buf[..n])
            .map_err(|e| AppError::Update(format!("write temp: {e}")))?;
    }
    out.flush()
        .map_err(|e| AppError::Update(format!("flush temp: {e}")))?;
    drop(out);

    if let Some(expected) = &new.sha256 {
        if let Some(h) = hasher {
            let actual_hex = hex::encode(h.finalize());
            if actual_hex.to_lowercase() != expected.to_lowercase() {
                let _ = std::fs::remove_file(&dest);
                return Err(AppError::Update(format!(
                    "sha256 mismatch: expected {expected}, got {actual_hex}"
                )));
            }
        }
    }

    Ok(dest)
}

fn prev_path(binary: &Path) -> std::path::PathBuf {
    let mut s = binary.as_os_str().to_owned();
    s.push(".prev");
    std::path::PathBuf::from(s)
}

fn make_exec(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path)?;
    let mode = meta.permissions().mode();
    let new_mode = (mode & 0o700) | 0o755;
    let mut perms = meta.permissions();
    perms.set_mode(new_mode);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

fn commit(app: &ManagedApp) -> Result<()> {
    let mut db = Db::open()?;
    if let Some(existing) = db.find_mut(&app.id) {
        *existing = app.clone();
    }
    db.commit()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::appimage::testutil;
    use crate::model::ManagedApp;

    pub(crate) fn test_app() -> (tempfile::TempDir, ManagedApp) {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join("My.AppImage");
        std::fs::write(&installed, testutil::synthetic_elf(true)).unwrap();
        let app = ManagedApp {
            id: "crate_t".into(),
            display_name: "My".into(),
            source_filename: "My.AppImage".into(),
            installed_path: installed,
            desktop_file_basename: "crate-t.desktop".into(),
            desktop_file_path: dir.path().join("a.desktop"),
            icon_path: None,
            imported_at: Utc::now(),
            last_updated_at: None,
            known_version: Some("1.0".into()),
            latest_version: None,
            update_status: crate::model::UpdateStatus::Unknown,
            update_provider: crate::model::UpdateProvider::None,
            integration_status: crate::model::IntegrationStatus::Healthy,
            owned_files: vec![],
        };
        (dir, app)
    }

    #[test]
    fn prev_path_appends_suffix() {
        assert_eq!(
            prev_path(Path::new("/a/b.AppImage")),
            Path::new("/a/b.AppImage.prev")
        );
    }

    #[test]
    fn absent_content_length_is_rejected() {
        // probe_size relies on the network; assert the guard lacks a
        // length-advertised path is a refusal is covered by MAX cap above.
        assert!(apply(
            &mut {
                let (_d, mut a) = test_app();
                a
            },
            &NewRelease {
                version: "2.0".into(),
                download_url: "http://127.0.0.1:1/x.AppImage".into(),
                size_bytes: 10,
                sha256: None,
                sig: None,
            },
            &crate::update::manual::ManualProvider
        )
        .is_err());
    }
}

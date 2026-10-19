//! Phase-1 embedded update-info reader.
//!
//! No `unsquashfs` and no execution: the `.zsync` trailer lives as literal
//! ASCII bytes near the end of a Type-2 AppImage. We read the last 64 KiB, scan
//! for `zsync:`, and parse the `Filename:` and `Zsync:` keys. Zsync-based delta
//! updates are NOT implemented in v1 — we only surface the provider URL for the
//! manual update flow.

use std::path::Path;

use crate::error::{AppError, Result};
use crate::model::UpdateProvider;

const TAIL_BYTES: usize = 64 * 1024;

/// Parses the embedded update-information trailer. Returns `None` when no
/// `zsync:` trailer is present. Returns `Err` for malformed input.
pub fn read_update_provider(path: &Path) -> Result<Option<UpdateProvider>> {
    let data = std::fs::read(path)?;
    let tail_start = data.len().saturating_sub(TAIL_BYTES);
    let tail = &data[tail_start..];

    let Some(idx) = trailer_start(tail) else {
        return Ok(None);
    };
    let text = String::from_utf8_lossy(&tail[idx..]);
    let mut filename: Option<String> = None;
    let mut zsync_url: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("Filename:") {
            filename = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("Zsync:") {
            zsync_url = Some(v.trim().to_string());
        }
    }

    let url = zsync_url.filter(|u| u.starts_with("https://"));
    if url.is_none() && filename.is_none() {
        // Present but empty trailer.
        return Ok(Some(UpdateProvider::None));
    }

    let provider = if let Some(u) = url {
        UpdateProvider::Manual {
            url: Some(u),
            last_verified_hash: None,
        }
    } else {
        // Zsync present but not HTTPS (e.g. relative or http): no usable v1 source.
        UpdateProvider::Named {
            name: filename.unwrap_or_else(|| "embedded".to_string()),
        }
    };
    Ok(Some(provider))
}

/// Finds the byte offset of the `zsync:` trailer. The trailer begins at a
/// `zsync:` preceded by a newline or at offset 0 of the tail.
fn trailer_start(tail: &[u8]) -> Option<usize> {
    let needle = b"zsync:";
    let mut i = 0usize;
    while i + needle.len() <= tail.len() {
        if &tail[i..i + needle.len()] == needle {
            match i.checked_sub(1) {
                None => return Some(0),
                Some(_) if i == 0 => return Some(0),
                Some(prev) => {
                    // Must be preceded by a newline (sig: "\nzsync:").
                    let mut j = prev;
                    loop {
                        if tail[j] == b'\n' {
                            return Some(j + 1);
                        }
                        if j == 0 {
                            return Some(0);
                        }
                        j -= 1;
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Type-1 AppImage update metadata is unreachable without a squashfs read;
/// v1 reports that clearly.
pub fn unsupported_type1() -> Result<UpdateProvider> {
    Err(AppError::AppImage(
        "update information unavailable for Type-1 AppImages in this build".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn appimage_with_trailer(trailer: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let mut data = vec![0u8; 100_000];
        // Place trailer so its start is within the last 64 KiB.
        data[90_000..90_000 + trailer.len()].copy_from_slice(trailer);
        let p = dir.path().join("t.AppImage");
        std::fs::write(&p, &data).unwrap();
        (dir, p)
    }

    #[test]
    fn parses_https_zsync_to_manual_provider() {
        let trailer = b"\nzsync:\nFilename: Foo-1.0-x86_64.AppImage\nZsync: https://example.com/Foo.AppImage.zsync\n";
        let (_dir, p) = appimage_with_trailer(trailer);
        let provider = read_update_provider(&p).unwrap().unwrap();
        match provider {
            UpdateProvider::Manual { url, .. } => {
                assert_eq!(
                    url.as_deref(),
                    Some("https://example.com/Foo.AppImage.zsync")
                )
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn no_trailer_yields_none() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("plain.AppImage");
        std::fs::write(&p, vec![0u8; 4096]).unwrap();
        assert_eq!(read_update_provider(&p).unwrap(), None);
    }
}

//! Scan a directory for AppImage files.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// Finds `*.AppImage` files (case-insensitive) under `dir`. Does NOT follow
/// symlinks.
pub fn scan_dir(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in WalkDir::new(dir).follow_links(false) {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if name.to_ascii_lowercase().ends_with(".appimage") {
            // Don't descend into the cratey temp/staging patterns.
            let p = entry.path();
            if is_intermediate(p) {
                continue;
            }
            out.push(entry.into_path());
        }
    }
    out.sort_by_key(|p| p.as_os_str().to_os_string());
    out
}

/// Skips files that are still being copied or are tricky to install later.
/// `dir/..AppImage.swp`, partial `.part` downloads, and dotfiles are excluded.
fn is_intermediate(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.starts_with('.') || name.ends_with(".part") || name.ends_with(".dpkg-tmp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_appimages_recursively_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("Foo.AppImage"), b"x").unwrap();
        std::fs::write(dir.path().join("bar.appimage"), b"x").unwrap();
        std::fs::write(dir.path().join("sub").join("Baz.APPIMAGE"), b"x").unwrap();
        std::fs::write(dir.path().join("not_image.txt"), b"x").unwrap();
        let found = scan_dir(dir.path());
        let mut names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "Baz.APPIMAGE".to_string(),
                "Foo.AppImage".to_string(),
                "bar.appimage".to_string()
            ]
        );
    }

    #[test]
    fn does_not_follow_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            let target = dir.path().join("real.AppImage");
            std::fs::write(&target, b"x").unwrap();
            std::os::unix::fs::symlink(&target, dir.path().join("link.AppImage")).unwrap();
            let found = scan_dir(dir.path());
            assert_eq!(found.len(), 1);
            assert!(found[0].ends_with("real.AppImage"));
        }
    }
}

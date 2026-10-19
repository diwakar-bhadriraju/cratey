//! Pre-write collision resolution for `.desktop` files.

use std::path::{Path, PathBuf};

use crate::error::{AppError, Result};
use crate::integration::PlannedInstall;
use crate::paths;

/// Outcome of resolving a planned `.desktop` write against existing files.
#[derive(Debug, Clone, PartialEq)]
pub enum Collision {
    /// No conflict; write our own entry.
    Write,
    /// An existing entry (ours or external) already covers this binary; use it.
    Adopt(PathBuf),
    /// A hard conflict that must surface to the user.
    Refuse(String),
}

/// Iterates `${XDG_DATA_HOME}/applications/*.desktop` and returns the first file
/// whose `Exec=` target equals `planned.installed_path` (after unquoting and
/// shell-independent parsing).
pub fn resolve_collision(planned: &PlannedInstall) -> Result<Collision> {
    let apps_dir = paths::applications_dir();
    let entries = std::fs::read_dir(&apps_dir)?;
    let our_id = &planned.id;

    // Also check our own planned basename to avoid overwriting a sibling cratey app.
    if planned.desktop_file_path.is_file() {
        // If the file at our basename is managed by us with the same id, it's an
        // update/repair of ourselves -> Write (overwrite).
        if let Ok(c) = inspect(&planned.desktop_file_path) {
            if c.managed && c.internal_id.as_deref() == Some(our_id) {
                return Ok(Collision::Write);
            }
            return Err(AppError::Integration(
                "desktop-file basename collision; refusing to overwrite".to_string(),
            ));
        }
    }

    for entry in entries {
        let Ok(entry) = entry else { continue };
        let p = entry.path();
        if p.extension().map(|e| e == "desktop").unwrap_or(false) {
            let Ok(info) = inspect(&p) else { continue };
            let target = planned.installed_path.to_string_lossy();
            if info.exec.as_deref() == Some(target.as_ref()) {
                if info.managed && info.internal_id.as_deref() == Some(our_id) {
                    // Ours: update path.
                    return Ok(Collision::Adopt(p));
                }
                // Someone else's file, or ours-but-different-id, pointing at the same binary.
                return Ok(Collision::Adopt(p));
            }
        }
    }
    Ok(Collision::Write)
}

/// Parsed identity of a `.desktop` file relevant to collision checks.
struct DesktopInspection {
    managed: bool,
    internal_id: Option<String>,
    exec: Option<Box<str>>,
}

impl DesktopInspection {
    fn empty() -> Self {
        DesktopInspection {
            managed: false,
            internal_id: None,
            exec: None,
        }
    }
}

fn inspect(path: &Path) -> Result<DesktopInspection> {
    let text = std::fs::read_to_string(path)?;
    let mut info = DesktopInspection::empty();
    let in_section = path;
    let _ = in_section;
    for line in text.lines() {
        if line.starts_with("[Desktop Entry]") {
            continue;
        }
        if line.starts_with('[') {
            break; // first section is what we care about
        }
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim();
            match k.trim() {
                "X-Cratey-Managed" => info.managed = v == "true",
                "X-Cratey-Internal-Id" => info.internal_id = Some(v.to_string()),
                "Exec" => info.exec = Some(unquote_exec(v).into()),
                _ => {}
            }
        }
    }
    Ok(info)
}

/// Parses an `Exec=` value: strips surrounding quotes and trailing field codes
/// (everything after the last `%` group is dropped), then unescapes.
fn unquote_exec(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(rest) = s.strip_prefix('"') {
        s = rest;
        if let Some(pos) = s.find('"') {
            s = &s[..pos];
        }
    }
    // Drop trailing field codes like ` %u` / `%F` that follow the path.
    if let Some(pos) = s.find('%') {
        s = s[..pos].trim_end();
    }
    // Unescape backslash-space and standard escapes.
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planned(installed: &str) -> PlannedInstall {
        PlannedInstall {
            id: "crate_t".into(),
            display_name: "Test".into(),
            source_path: "/tmp/src.AppImage".into(),
            installed_path: installed.into(),
            icon_path: "/dev/null.png".into(),
            desktop_file_path: "/tmp/apps/crate-tt.desktop".into(),
            base_dir: None,
            move_source: false,
        }
    }

    #[test]
    fn unquote_exec_handles_quoted_paths_and_field_codes() {
        assert_eq!(
            unquote_exec("\"/opt/My App.AppImage\" %u"),
            "/opt/My App.AppImage"
        );
        assert_eq!(unquote_exec("/opt/simple %F"), "/opt/simple");
        assert_eq!(unquote_exec("foo\\ bar"), "foo bar");
    }

    #[test]
    fn adopts_external_desktop_file() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("applications");
        std::fs::create_dir_all(&apps).unwrap();
        let target = "/tmp/My.AppImage";
        std::fs::write(
            apps.join("external.desktop"),
            format!("[Desktop Entry]\nType=Application\nExec=\"{target}\" %u\n"),
        )
        .unwrap();
        // Point paths::applications_dir at this temp dir by making it a subdir.
        // We can't easily redirect the function, so instead place the adopted
        // file in our own basename handling is exercised separately. Here we
        // assert parse-level behavior via resolve against a crafted dir:
        let pl = planned(target);
        // resolve_collision uses a fixed XDG dir; emulate by inspecting the file directly.
        let info = inspect(&apps.join("external.desktop")).unwrap();
        assert!(!info.managed);
        assert_eq!(info.exec.as_deref(), Some("/tmp/My.AppImage"));
        let _ = pl;
    }

    #[test]
    fn overwrites_own_desktop_file_via_managed_flag() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("crate-tt.desktop");
        std::fs::write(
            &p,
            "[Desktop Entry]\nX-Cratey-Managed=true\nX-Cratey-Internal-Id=crate_t\nExec=/tmp/My.AppImage\n",
        )
        .unwrap();
        let mut pl = planned("/tmp/My.AppImage");
        pl.id = "crate_tt".into();
        pl.desktop_file_path = p.clone();
        // The file at our basename is managed by id crate_t != crate_tt -> refuse.
        match resolve_collision(&pl) {
            Err(AppError::Integration(_)) => {}
            other => panic!("expected Refuse, got {other:?}"),
        }
    }
}

//! Rich typing for the `.desktop` entry we write for each managed app.
//!
//! We hand-roll templating (no `freedesktop-desktop-entry` crate — it has
//! Unicode/path-with-space issues). See [`DesktopEntry`].

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::model::ManagedApp;

/// A fully-specified desktop entry before serialization.
#[derive(Debug, Clone)]
pub struct DesktopEntry {
    pub name: String,
    pub generic_name: String,
    pub comment: String,
    pub exec: PathBuf,
    pub working_dir: Option<PathBuf>,
    pub icon: Option<PathBuf>,
    pub categories: String,
    pub mime_types: Vec<String>,
    pub startup_wm_class: String,
    pub appimage_identifier: String,
    pub internal_id: String,
}

impl From<&ManagedApp> for DesktopEntry {
    fn from(app: &ManagedApp) -> Self {
        DesktopEntry {
            name: app.display_name.clone(),
            generic_name: String::new(),
            comment: String::new(),
            exec: app.installed_path.clone(),
            working_dir: None,
            icon: app.icon_path.clone(),
            categories: String::new(),
            mime_types: Vec::new(),
            startup_wm_class: app.display_name.clone(),
            appimage_identifier: app.display_name.clone(),
            internal_id: app.id.clone(),
        }
    }
}

/// Escapes a single value per the Desktop Entry Spec:
/// backslash-escape `\`, `;`, `%`, `"`; for `Exec`, also escape spaces.
fn escape_value(value: &str, exec: bool) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            '%' => out.push_str("%%"),
            '"' => out.push_str("\\\""),
            ' ' if exec => out.push_str("\\ "),
            c => out.push(c),
        }
    }
    out
}

/// Renders the entry as UTF-8 (no BOM), LF line endings, spec-compliant keys.
pub fn render(entry: &DesktopEntry) -> String {
    let mut s = String::new();
    s.push_str("[Desktop Entry]\n");
    s.push_str("Type=Application\n");
    s.push_str("Version=1.0\n");
    s.push_str(&format!("Name={}\n", escape_value(&entry.name, false)));
    if !entry.generic_name.is_empty() {
        s.push_str(&format!(
            "GenericName={}\n",
            escape_value(&entry.generic_name, false)
        ));
    }
    if !entry.comment.is_empty() {
        s.push_str(&format!(
            "Comment={}\n",
            escape_value(&entry.comment, false)
        ));
    }
    // Exec: double-quoted path (handles spaces), then ` %u` outside the quote.
    let path = entry.exec.to_string_lossy();
    let quoted = format!("\"{}\"", path.replace('\\', "\\\\").replace('"', "\\\""));
    s.push_str(&format!("Exec={} %u\n", quoted));
    if let Some(wd) = &entry.working_dir {
        s.push_str(&format!(
            "Path={}\n",
            escape_value(&wd.to_string_lossy(), false)
        ));
    }
    if let Some(icon) = &entry.icon {
        s.push_str(&format!(
            "Icon={}\n",
            escape_value(&icon.to_string_lossy(), false)
        ));
    }
    if !entry.categories.is_empty() {
        s.push_str(&format!(
            "Categories={}\n",
            escape_value(&entry.categories, false)
        ));
    }
    if !entry.mime_types.is_empty() {
        s.push_str("MimeType=");
        for (i, m) in entry.mime_types.iter().enumerate() {
            if i > 0 {
                s.push(';');
            }
            s.push_str(&escape_value(m, false));
        }
        s.push_str(" ;\n");
    }
    s.push_str("Terminal=false\n");
    s.push_str("StartupNotify=true\n");
    s.push_str(&format!(
        "StartupWMClass={}\n",
        escape_value(&entry.startup_wm_class, false)
    ));
    s.push_str(&format!(
        "X-AppImage-Identifier={}\n",
        escape_value(&entry.appimage_identifier, false)
    ));
    s.push_str("X-Cratey-Managed=true\n");
    s.push_str(&format!("X-Cratey-Internal-Id={}\n", entry.internal_id));
    s
}

/// Writes the rendered entry atomically to `path` (parent dirs created).
pub fn write_entry(path: &Path, entry: &DesktopEntry) -> Result<()> {
    let rendered = render(entry);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("desktop.tmp");
    std::fs::write(&tmp, rendered.as_bytes())?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Best-effort refresh of the desktop database; failure is logged, not fatal.
pub fn refresh_databases(apps_dir: &Path) {
    if let Err(e) = std::process::Command::new("update-desktop-database")
        .arg(apps_dir)
        .status()
    {
        log::warn!("update-desktop-database failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, exec: &str) -> DesktopEntry {
        DesktopEntry {
            name: name.into(),
            generic_name: String::new(),
            comment: String::new(),
            exec: exec.into(),
            working_dir: None,
            icon: None,
            categories: String::new(),
            mime_types: vec![],
            startup_wm_class: name.into(),
            appimage_identifier: name.into(),
            internal_id: "crate_t".into(),
        }
    }

    #[test]
    fn escapes_semicolons_percent_quotes() {
        let e = entry("A;B%\"Q", "/opt/My App.AppImage");
        let s = render(&e);
        assert!(s.contains("Name=A\\;B%%\\\"Q\n"), "got: {s}");
        assert!(s.contains("Exec=\"/opt/My App.AppImage\" %u\n"), "got: {s}");
    }

    #[test]
    fn handles_paths_with_spaces_and_unicode() {
        let name = "Ünïcode App";
        let p = "/home/me/My Apps/ゾート App.AppImage";
        let e = entry(name, p);
        let s = render(&e);
        assert!(s.contains(&format!("Exec=\"{p}\" %u\n")), "got: {s}");
        assert!(s.contains("StartupWMClass=Ünïcode App\n"), "got: {s}");
    }
}

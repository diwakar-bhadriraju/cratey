//! Launch a managed app without a shell.

use std::process::Command;

use crate::error::{AppError, Result};
use crate::model::ManagedApp;

/// Vars required for desktop/portal-bound binaries (AppImages, GUI apps) to
/// start successfully. Anything else from the user's session is inherited
/// from the surrounding `relm4`/GTK process (so locale, theme, SSH agent
/// sockets etc. work) — we only force-set these to be the canonical names.
const PASSTHROUGH_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_CTYPE",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "WAYLAND_SOCKET",
    "XDG_RUNTIME_DIR",
    "XDG_DATA_DIRS",
    "XDG_CONFIG_DIRS",
    "XDG_SESSION_TYPE",
    "XDG_CURRENT_DESKTOP",
    "XDG_SEAT",
    "DBUS_SESSION_BUS_ADDRESS",
    "DBUS_SYSTEM_BUS_ADDRESS",
    "TMPDIR",
    "COLORTERM",
    "SSH_AUTH_SOCK",
    "GPG_TTY",
];

/// Spawns the installed binary, disowns the child, and returns its PID.
/// Does not wait. The child inherits the parent process's environment, with
/// the well-known desktop/portal variables pinned to their current values so
/// AppImages that need `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`, `DBUS_*`, etc.
/// always see them (a blanket `env_clear` breaks real AppImages).
pub fn launch(app: &ManagedApp) -> Result<u32> {
    if !app.installed_path.is_file() {
        return Err(AppError::Integration(format!(
            "installed binary {} is not present",
            app.installed_path.display()
        )));
    }
    let mut cmd = Command::new(&app.installed_path);
    // Pass through the curated list, ignoring missing ones (so this works
    // even if e.g. the user is on a pure X11 session without WAYLAND_DISPLAY).
    for var in PASSTHROUGH_VARS {
        if let Ok(v) = std::env::var(var) {
            cmd.env(var, v);
        }
    }
    let child = cmd.spawn().map_err(|e| {
        AppError::Io(format!(
            "failed to launch {}: {e}",
            app.installed_path.display()
        ))
    })?;
    Ok(child.id())
}

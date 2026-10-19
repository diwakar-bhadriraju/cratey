//! Launch a managed app without a shell.

use std::process::Command;

use crate::error::{AppError, Result};
use crate::model::ManagedApp;

/// Spawns the installed binary, disowns the child, and returns its PID.
/// Does not wait. `env_clear` + PATH only: no inherited shell aliases/functions.
pub fn launch(app: &ManagedApp) -> Result<u32> {
    if !app.installed_path.is_file() {
        return Err(AppError::Integration(format!(
            "installed binary {} is not present",
            app.installed_path.display()
        )));
    }
    let path = std::env::var("PATH").unwrap_or_default();
    let child = Command::new(&app.installed_path)
        .env_clear()
        .env("PATH", path)
        .spawn()
        .map_err(|e| {
            AppError::Io(format!(
                "failed to launch {}: {e}",
                app.installed_path.display()
            ))
        })?;
    Ok(child.id())
}

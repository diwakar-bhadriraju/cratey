//! Cratey — AppImage manager binary entry point.
//!
//! Two modes:
//! - default: launches the relm4 GUI.
//! - `--bin-import <file-or-url>`: headless import (usable from a file-manager
//!   `.desktop` MimeType handler). Never opens a window.

use clap::Parser;
use relm4::RelmApp;

use cratey::error::Result;

#[derive(Parser, Debug)]
#[command(name = "cratey", version, about = "Native Linux AppImage manager")]
struct Cli {
    /// Headless import of an AppImage file, then exit.
    #[arg(long, value_name = "PATH")]
    bin_import: Option<std::path::PathBuf>,
}

fn main() -> glib::ExitCode {
    // Root guard: refuse to run as root before any state is touched.
    if let Err(e) = cratey::assert_not_root(|| unsafe { libc::geteuid() }) {
        eprintln!("{e}");
        return glib::ExitCode::FAILURE;
    }

    let cli = Cli::parse();
    if let Some(path) = cli.bin_import {
        match headless_import(&path) {
            Ok(app) => {
                eprintln!(
                    "Installed {} → {}",
                    app.display_name,
                    app.installed_path.display()
                );
                glib::ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                glib::ExitCode::FAILURE
            }
        }
    } else {
        let app = RelmApp::new("org.diwakar.Cratey");
        app.run::<cratey::ui::app::App>(());
        glib::ExitCode::SUCCESS
    }
}

/// Runs the shared install flow headless (no UI).
fn headless_import(source: &std::path::Path) -> Result<cratey::model::ManagedApp> {
    cratey::appimage::format::detect(source)?;
    let db = cratey::db::Db::open()?;
    let base_dir =
        db.document().settings.base_dir.clone().ok_or_else(|| {
            cratey::error::AppError::Config("no storage directory configured".into())
        })?;
    let base_dir = base_dir.clone();
    drop(db);

    let id = cratey::model::new_app_id();
    let installed = base_dir.join(
        source
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "app.AppImage".into()),
    );
    let display = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled App".into());
    let plan = cratey::integration::PlannedInstall {
        id: id.clone(),
        display_name: display,
        source_path: source.to_path_buf(),
        installed_path: installed,
        icon_path: cratey::paths::icons_dir().join(format!("{id}.png")),
        desktop_file_path: cratey::paths::applications_dir()
            .join(cratey::model::desktop_file_basename(&id)),
        base_dir: Some(base_dir),
        move_source: false,
    };
    cratey::integration::install::install(plan)
}

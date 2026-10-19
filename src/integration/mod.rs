//! Desktop integration: `.desktop` files, menu collision resolution, install
//! (with stratified rollback), and launching.

pub mod desktop;
pub mod install;
pub mod launch;
pub mod menu;

use std::path::PathBuf;

use crate::model::AppId;

/// A fully-resolved install request from the UI/CLI.
#[derive(Debug, Clone)]
pub struct PlannedInstall {
    pub id: AppId,
    pub display_name: String,
    pub source_path: PathBuf,
    pub installed_path: PathBuf,
    pub icon_path: PathBuf,
    pub desktop_file_path: PathBuf,
    /// `None` = base_dir is empty / first run.
    pub base_dir: Option<PathBuf>,
    pub move_source: bool,
}

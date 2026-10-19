//! Main application model and UI (built imperatively for reliability).
//!
//! This module implements `SimpleComponent` by hand (no `view!` macro) so the
//! whole window can be constructed and rebuilt imperatively.

use std::path::PathBuf;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::gdk;
use relm4::{ComponentParts, ComponentSender, RelmWidgetExt, SimpleComponent};

use crate::db::Db;
use crate::error::Result;
use crate::integration::PlannedInstall;
use crate::model::{
    ImportMode, IntegrationStatus, ManagedApp, NotificationPref, ReleaseChannel, Settings,
    UpdateCheckPref, UpdateProvider, UpdateStatus,
};
use crate::paths;
use crate::update::UpdateProvider as _;

/// Top-level model; stores widget handles so `update` can rebuild the dynamic
/// sections (app list + details).
pub struct App {
    apps: Vec<ManagedApp>,
    selected: Option<usize>,
    settings: Settings,
    search_text: String,

    root: adw::Window,
    list_box: gtk::ListBox,
    details_box: gtk::Box,
    details_stack: gtk::Stack,
    sidebar_info: gtk::Label,
    toast_overlay: adw::ToastOverlay,
    /// apps indices currently shown in the list (search-filtered order).
    visible: Vec<usize>,
    /// While true, the row-selected handler ignores events (programmatic rebuild).
    suppress_selected: std::rc::Rc<std::cell::Cell<bool>>,
    /// Files still waiting for their import-preview dialog (dir imports / drops).
    pending_imports: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum AppMsg {
    Refresh,
    Select(usize),
    ImportFiles(Vec<PathBuf>),
    DoInstall {
        source: PathBuf,
        base_dir: PathBuf,
        display: String,
        mode: ImportMode,
    },
    Launch,
    OpenLocation,
    Rename(String),
    Repair,
    Uninstall,
    CheckUpdates,
    Preferences,
    SetBaseDir(PathBuf),
    SetImportMode(ImportMode),
    SetReleaseChannel(ReleaseChannel),
    SetUpdateCheckPref(UpdateCheckPref),
    SetNotificationPref(NotificationPref),
    SetConfirmInstall(bool),
    SetConfirmUninstall(bool),
    Rescan,
    About,
    Quit,
    SearchChanged(String),
    ShowToast(String),
    ClosePreview,
}

impl SimpleComponent for App {
    type Init = ();
    type Input = AppMsg;
    type Output = ();
    type Root = adw::Window;
    type Widgets = ();

    fn init_root() -> Self::Root {
        adw::Window::new()
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (apps, settings) = load_state();

        let toast_overlay = adw::ToastOverlay::new();
        root.set_content(Some(&toast_overlay));

        // Whole-window file drag-and-drop.
        let drop_target = gtk::DropTarget::builder()
            .formats(&gdk::ContentFormats::new(&["text/uri-list"]))
            .actions(gdk::DragAction::COPY)
            .build();
        let s = sender.clone();
        drop_target.connect_drop(move |_w, value, _x, _y| {
            if let Ok(list) = value.get::<gdk::FileList>() {
                let paths: Vec<PathBuf> = list
                    .files()
                    .iter()
                    .filter_map(|f| f.path().map(|p| p.to_path_buf()))
                    .collect();
                if !paths.is_empty() {
                    s.input(AppMsg::ImportFiles(paths));
                    return true;
                }
            }
            false
        });
        root.add_controller(drop_target);

        // --- Sidebar: search + list + guidance ---
        let list_box = gtk::ListBox::new();
        list_box.set_selection_mode(gtk::SelectionMode::Single);
        list_box.set_css_classes(&["boxed-list"]);
        let suppress = std::rc::Rc::new(std::cell::Cell::new(false));
        let s = sender.clone();
        let sp = suppress.clone();
        list_box.connect_row_selected(move |_, row| {
            if sp.get() {
                return;
            }
            if let Some(row) = row.as_ref() {
                s.input(AppMsg::Select(row.index() as usize));
            }
        });

        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search apps"));
        let s = sender.clone();
        search.connect_changed(move |e| s.input(AppMsg::SearchChanged(e.text().to_string())));

        let browse = gtk::Button::with_label("Browse…");
        browse.set_halign(gtk::Align::Center);
        let s = sender.clone();
        let w = root.clone();
        browse.connect_clicked(move |_| open_file_picker(&w, s.clone()));

        let sidebar_info = gtk::Label::new(Some(
            "Drop AppImages here, or pick a storage directory in Preferences.",
        ));
        sidebar_info.set_wrap(true);
        sidebar_info.set_justify(gtk::Justification::Center);
        sidebar_info.add_css_class("dim-label");

        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 6);
        sidebar.set_width_request(300);
        sidebar.append(&search);
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&list_box));
        sidebar.append(&scrolled);
        sidebar.append(&browse);
        sidebar.append(&sidebar_info);

        // --- Right pane: empty state / details stack ---
        let details_stack = gtk::Stack::new();
        details_stack.set_vexpand(true);
        details_stack.set_hexpand(true);

        let empty_page = adw::StatusPage::new();
        empty_page.set_icon_name(Some("application-x-executable-symbolic"));
        empty_page.set_title("No app selected");
        empty_page.set_description(Some("Pick an app from the list on the left."));
        details_stack.add_named(&empty_page, Some("empty"));

        let details_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        details_stack.add_named(&details_box, Some("details"));

        let split = adw::OverlaySplitView::new();
        split.set_show_sidebar(true);
        split.set_sidebar(Some(&sidebar));
        split.set_content(Some(&details_stack));

        // --- Header bar + app menu ---
        let header = adw::HeaderBar::new();

        let title = gtk::Label::new(Some("Cratey"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));

        // Primary "Import" action.
        let import_btn = gtk::Button::new();
        import_btn.set_icon_name("document-open-symbolic");
        import_btn.set_label("Import…");
        import_btn.set_tooltip_text(Some("Import an AppImage file"));
        import_btn.add_css_class("suggested-action");
        let s = sender.clone();
        let w = root.clone();
        import_btn.connect_clicked(move |_| open_file_picker(&w, s.clone()));
        header.pack_start(&import_btn);

        // Hamburger app menu.
        let menu = build_app_menu(root.clone(), sender.clone());
        header.pack_end(&menu);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&split));

        toast_overlay.set_child(Some(&toolbar));
        root.set_title(Some("Cratey"));
        root.set_default_size(1160, 760);

        let mut model = App {
            apps,
            selected: None,
            settings,
            search_text: String::new(),
            root: root.clone(),
            list_box,
            details_box,
            details_stack,
            sidebar_info,
            toast_overlay,
            visible: Vec::new(),
            suppress_selected: suppress,
            pending_imports: Vec::new(),
        };
        model.rebuild_all(&sender);

        if model.settings.base_dir.is_none() {
            sender.input(AppMsg::ShowToast(
                "Pick a storage directory to begin".into(),
            ));
            open_folder_picker(&root, sender.clone());
        }

        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            AppMsg::Refresh => self.rebuild_all(&sender),
            AppMsg::Select(pos) => {
                if let Some(real) = self.visible.get(pos) {
                    self.selected = Some(*real);
                    self.rebuild_all(&sender);
                }
            }
            AppMsg::ImportFiles(paths) => self.import_files(paths, &sender),
            AppMsg::DoInstall {
                source,
                base_dir,
                display,
                mode,
            } => {
                self.do_install(source, base_dir, display, mode, &sender);
                sender.input(AppMsg::Refresh);
                self.show_next_preview(&sender);
            }
            AppMsg::Launch => {
                if let Some(app) = self.selected_app() {
                    match crate::integration::launch::launch(app) {
                        Ok(pid) => sender.input(AppMsg::ShowToast(format!("Launched (pid {pid})"))),
                        Err(e) => sender.input(AppMsg::ShowToast(format!("Launch failed: {e}"))),
                    }
                }
            }
            AppMsg::OpenLocation => {
                if let Some(app) = self.selected_app() {
                    open_in_file_manager(&app.installed_path);
                }
            }
            AppMsg::Rename(name) => {
                if let Some(idx) = self.selected {
                    if !name.trim().is_empty() {
                        self.apps[idx].display_name = name.trim().to_string();
                        self.commit_state();
                        sender.input(AppMsg::Refresh);
                    }
                }
            }
            AppMsg::Repair => {
                if let Some(app) = self.selected_app().cloned() {
                    match repair_app(&app) {
                        Ok(()) => self.toast("Integration repaired"),
                        Err(e) => self.toast(format!("Repair failed: {e}")),
                    }
                    sender.input(AppMsg::Refresh);
                }
            }
            AppMsg::Uninstall => self.confirm_uninstall(&sender),
            AppMsg::CheckUpdates => {
                if let Some(app) = self.selected_app() {
                    self.check_updates(app.clone(), &sender);
                }
            }
            AppMsg::Preferences => self.show_preferences_window(&sender),
            AppMsg::SetBaseDir(dir) => {
                self.settings.base_dir = Some(dir);
                self.commit_settings();
                self.toast("Storage directory set");
                sender.input(AppMsg::Refresh);
            }
            AppMsg::SetImportMode(m) => {
                self.settings.default_import_mode = m;
                self.commit_settings();
            }
            AppMsg::SetReleaseChannel(c) => {
                self.settings.release_channel_default = c;
                self.commit_settings();
            }
            AppMsg::SetUpdateCheckPref(p) => {
                self.settings.update_check_pref = p;
                self.commit_settings();
            }
            AppMsg::SetNotificationPref(p) => {
                self.settings.notification_pref = p;
                self.commit_settings();
            }
            AppMsg::SetConfirmInstall(v) => {
                self.settings.confirm_before_install = v;
                self.commit_settings();
            }
            AppMsg::SetConfirmUninstall(v) => {
                self.settings.confirm_before_uninstall = v;
                self.commit_settings();
            }
            AppMsg::Rescan => {
                // Re-list AppImages already present in the storage dir.
                if let Some(base) = &self.settings.base_dir {
                    let found = crate::appimage::scan::scan_dir(base);
                    if found.is_empty() {
                        self.toast("No AppImages found in the storage directory");
                    } else {
                        self.toast(format!("Found {} AppImage(s) (import to add)", found.len()));
                    }
                    let _ = found;
                } else {
                    self.toast("No storage directory set");
                }
                sender.input(AppMsg::Refresh);
            }
            AppMsg::About => show_about(&self.root),
            AppMsg::Quit => self.root.close(),
            AppMsg::SearchChanged(s) => {
                self.search_text = s;
                self.rebuild_all(&sender);
            }
            AppMsg::ShowToast(text) => self.toast(text),
            AppMsg::ClosePreview => self.show_next_preview(&sender),
        }
    }
}

impl App {
    fn toast(&self, text: impl Into<String>) {
        let toast = adw::Toast::new(&text.into());
        self.toast_overlay.add_toast(toast);
    }

    fn selected_app(&self) -> Option<&ManagedApp> {
        self.selected.and_then(|i| self.apps.get(i))
    }

    /// Clears the app list. Must use `ListBox::remove` (not `unparent`) so the
    /// ListBox keeps its internal row bookkeeping consistent.
    fn clear_list(&mut self) {
        while let Some(row) = self.list_box.row_at_index(0) {
            self.list_box.remove(&row);
        }
    }

    /// Clears a plain container.
    fn clear_box(b: &gtk::Box) {
        while let Some(child) = b.first_child() {
            b.remove(&child);
        }
    }

    fn rebuild_all(&mut self, sender: &ComponentSender<Self>) {
        self.clear_list();
        self.suppress_selected.set(true);
        let query = self.search_text.to_lowercase();
        self.visible.clear();
        for (i, app) in self.apps.iter().enumerate() {
            if !query.is_empty()
                && !app.display_name.to_lowercase().contains(&query)
                && !app.source_filename.to_lowercase().contains(&query)
            {
                continue;
            }
            self.visible.push(i);
            let action = adw::ActionRow::new();
            action.add_prefix(&app_icon_at(app, 32));
            action.set_title(&app.display_name);
            action.set_subtitle(&app.installed_path.to_string_lossy());
            self.list_box.append(&action);
            if self.selected == Some(i) {
                self.list_box.select_row(Some(&action));
            }
        }
        self.suppress_selected.set(false);
        self.list_box.set_visible(!self.apps.is_empty());
        self.sidebar_info.set_visible(self.apps.is_empty());

        self.rebuild_details(sender);
    }

    fn rebuild_details(&mut self, sender: &ComponentSender<Self>) {
        Self::clear_box(&self.details_box);
        let Some(app) = self.selected_app() else {
            self.details_stack.set_visible_child_name("empty");
            return;
        };
        self.details_stack.set_visible_child_name("details");

        let page = adw::PreferencesPage::new();

        let id_group = adw::PreferencesGroup::new();
        id_group.set_title("Identity");
        add_info_row(&id_group, "Name", &app.display_name);
        add_info_row(
            &id_group,
            "Version",
            app.known_version.as_deref().unwrap_or("unknown"),
        );
        add_info_row(&id_group, "Update", &update_status_text(&app.update_status));
        add_info_row(
            &id_group,
            "Integration",
            &integration_status_text(&app.integration_status),
        );
        page.add(&id_group);

        let path_group = adw::PreferencesGroup::new();
        path_group.set_title("Paths");
        add_info_row(&path_group, "Binary", &app.installed_path.to_string_lossy());
        add_info_row(
            &path_group,
            "Desktop file",
            &app.desktop_file_path.to_string_lossy(),
        );
        page.add(&path_group);

        let act_group = adw::PreferencesGroup::new();
        act_group.set_title("Actions");

        let launch = adw::ActionRow::new();
        launch.set_title("Launch");
        launch.set_activatable(true);
        {
            let s = sender.clone();
            launch.connect_activated(move |_| s.input(AppMsg::Launch));
        }
        act_group.add(&launch);

        let open = adw::ActionRow::new();
        open.set_title("Open location");
        open.set_activatable(true);
        {
            let s = sender.clone();
            open.connect_activated(move |_| s.input(AppMsg::OpenLocation));
        }
        act_group.add(&open);

        let rename = adw::ActionRow::new();
        rename.set_title("Rename…");
        rename.set_activatable(true);
        {
            let s = sender.clone();
            let current = app.display_name.clone();
            rename.connect_activated(move |_| prompt_rename(current.clone(), s.clone()));
        }
        act_group.add(&rename);

        if matches!(app.integration_status, IntegrationStatus::Healthy) {
            let check = adw::ActionRow::new();
            check.set_title("Check for updates");
            check.set_subtitle("Queries the configured update source");
            check.set_activatable(true);
            {
                let s = sender.clone();
                check.connect_activated(move |_| s.input(AppMsg::CheckUpdates));
            }
            act_group.add(&check);
        } else {
            let repair = adw::ActionRow::new();
            repair.set_title("Repair integration");
            repair.set_activatable(true);
            {
                let s = sender.clone();
                repair.connect_activated(move |_| s.input(AppMsg::Repair));
            }
            act_group.add(&repair);
        }

        let uninstall = adw::ActionRow::new();
        uninstall.set_title("Uninstall…");
        uninstall.set_subtitle("Remove integration and/or the binary");
        uninstall.set_activatable(true);
        {
            let s = sender.clone();
            uninstall.connect_activated(move |_| s.input(AppMsg::Uninstall));
        }
        act_group.add(&uninstall);

        page.add(&act_group);

        let clamp = adw::Clamp::new();
        clamp.set_child(Some(&page));
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&clamp));

        // Header: large app icon + display name.
        let header = gtk::Box::new(gtk::Orientation::Vertical, 6);
        header.set_margin_top(16);
        header.set_margin_bottom(8);
        header.set_halign(gtk::Align::Center);
        header.append(&app_icon_at(app, 64));
        let name_label = gtk::Label::new(Some(&app.display_name));
        name_label.add_css_class("title-1");
        header.append(&name_label);

        self.details_box.append(&header);
        self.details_box.append(&scrolled);
    }

    fn import_files(&mut self, paths: Vec<PathBuf>, sender: &ComponentSender<Self>) {
        if self.settings.base_dir.is_none() {
            self.toast("Choose a storage directory first");
            open_folder_picker(&self.root, sender.clone());
            return;
        }
        for path in paths {
            if path.is_dir() {
                // Importing a folder: scan it for AppImages (recursively).
                let found = crate::appimage::scan::scan_dir(&path);
                if found.is_empty() {
                    self.toast(format!("No AppImages found in {}", path.display()));
                }
                for f in found {
                    self.enqueue_import(f);
                }
            } else if path.is_file() {
                self.enqueue_import(path);
            } else {
                self.toast(format!("Not found: {}", path.display()));
            }
        }
        self.show_next_preview(sender);
    }

    /// Validates a single file and queues it for the preview dialog, skipping
    /// files already installed to the same target to avoid duplicates.
    fn enqueue_import(&mut self, path: PathBuf) {
        if let Err(e) = crate::appimage::format::detect(&path) {
            self.toast(format!("{}: not a valid AppImage ({e})", path.display()));
            return;
        }
        let target = self
            .settings
            .base_dir
            .as_ref()
            .map(|b| b.join(path.file_name().unwrap_or_default()));
        if let Some(t) = &target {
            if self.apps.iter().any(|a| &a.installed_path == t) {
                self.toast(format!("{} already installed", path.display()));
                return;
            }
        }
        self.pending_imports.push(path);
    }

    /// Opens the preview for the next queued file, if any.
    fn show_next_preview(&mut self, sender: &ComponentSender<Self>) {
        if let Some(path) = self.pending_imports.first() {
            let path = path.clone();
            self.pending_imports.remove(0);
            self.show_preview(path, sender);
        }
    }

    fn show_preview(&mut self, path: PathBuf, sender: &ComponentSender<Self>) {
        let base_dir = self.settings.base_dir.clone().unwrap();

        let window = adw::Window::new();
        window.set_title(Some("Import Preview"));
        window.set_modal(true);
        window.set_resizable(false);
        window.set_default_width(500);
        window.set_transient_for(Some(&self.root));

        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.set_margin_all(16);

        let suggested = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled App".into());

        let entry_row = adw::EntryRow::new();
        entry_row.set_title("Display name");
        entry_row.set_text(&suggested);
        body.append(&entry_row);

        let mode_combo = adw::ComboRow::new();
        mode_combo.set_title("Import mode");
        let m = gtk::StringList::new(&["Copy", "Move"]);
        mode_combo.set_model(Some(&m));
        mode_combo.set_selected(0);
        body.append(&mode_combo);

        let src_row = adw::ActionRow::new();
        src_row.set_title("Source");
        src_row.set_subtitle(&path.to_string_lossy());
        body.append(&src_row);

        let btns = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        btns.set_halign(gtk::Align::End);

        let cancel = gtk::Button::with_label("Cancel");
        let s = sender.clone();
        let w = window.clone();
        cancel.connect_clicked(move |_| {
            s.input(AppMsg::ClosePreview);
            w.close();
        });
        btns.append(&cancel);

        let install = gtk::Button::with_label("Install");
        install.add_css_class("suggested-action");
        let s = sender.clone();
        let w = window.clone();
        let path2 = path.clone();
        let base2 = base_dir.clone();
        let combo = mode_combo.clone();
        let entry2 = entry_row.clone();
        install.connect_clicked(move |_| {
            let mode = if combo.selected() == 1 {
                ImportMode::Move
            } else {
                ImportMode::Copy
            };
            s.input(AppMsg::DoInstall {
                source: path2.clone(),
                base_dir: base2.clone(),
                display: entry2.text().to_string(),
                mode,
            });
            w.close();
        });
        btns.append(&install);
        body.append(&btns);

        let tbv = adw::ToolbarView::new();
        tbv.add_top_bar(&adw::HeaderBar::new());
        tbv.set_content(Some(&body));
        window.set_content(Some(&tbv));
        window.present();
    }

    fn do_install(
        &mut self,
        source: PathBuf,
        base_dir: PathBuf,
        display: String,
        mode: ImportMode,
        _sender: &ComponentSender<Self>,
    ) {
        let id = crate::model::new_app_id();
        let installed = base_dir.join(
            source
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "app.AppImage".into()),
        );
        let icon = paths::icons_dir().join(format!("{id}.png"));
        let desktop = paths::applications_dir().join(crate::model::desktop_file_basename(&id));
        let plan = PlannedInstall {
            id: id.clone(),
            display_name: display,
            source_path: source,
            installed_path: installed,
            icon_path: icon,
            desktop_file_path: desktop,
            base_dir: Some(base_dir),
            move_source: mode == ImportMode::Move,
        };
        match crate::integration::install::install(plan) {
            Ok(app) => {
                if let Some(existing) = self.apps.iter_mut().find(|a| a.id == app.id) {
                    *existing = app;
                } else {
                    self.apps.push(app);
                }
                self.commit_state();
                self.toast("Installed");
            }
            Err(e) => self.toast(format!("Install failed: {e}")),
        }
    }

    fn commit_state(&self) {
        if let Ok(mut db) = Db::open() {
            db.document_mut().settings = self.settings.clone();
            for a in &self.apps {
                if let Some(existing) = db.find_mut(&a.id) {
                    *existing = a.clone();
                } else {
                    db.apps_mut().push(a.clone());
                }
            }
            let _ = db.commit();
        }
    }

    fn commit_settings(&self) {
        if let Ok(mut db) = Db::open() {
            db.document_mut().settings = self.settings.clone();
            let _ = db.commit();
        }
    }

    fn check_updates(&self, app: ManagedApp, sender: &ComponentSender<Self>) {
        if matches!(app.update_provider, UpdateProvider::None) {
            self.toast("No update source configured for this app");
            return;
        }
        let s = sender.clone();
        run_blocking(
            move || match crate::update::github::check_for_updates(&app, None) {
                Ok(crate::update::UpdateOutcome::Available(new)) => {
                    let mut updated = app.clone();
                    updated.update_status = UpdateStatus::Checking;
                    let provider = crate::update::manual::ManualProvider;
                    match provider.apply(&mut updated, new) {
                        Ok(()) => s.input(AppMsg::ShowToast("Update applied".into())),
                        Err(e) => s.input(AppMsg::ShowToast(format!("Update failed: {e}"))),
                    }
                }
                Ok(crate::update::UpdateOutcome::NoUpdate) => {
                    s.input(AppMsg::ShowToast("No update available".into()))
                }
                Ok(crate::update::UpdateOutcome::Failed(m)) => {
                    s.input(AppMsg::ShowToast(format!("Check failed: {m}")))
                }
                Err(e) => s.input(AppMsg::ShowToast(format!("Check failed: {e}"))),
            },
        );
    }

    fn confirm_uninstall(&mut self, sender: &ComponentSender<Self>) {
        let Some(app) = self.selected_app().cloned() else {
            return;
        };
        let dialog = adw::AlertDialog::builder()
            .heading("Uninstall")
            .body(format!(
                "Remove {}? Choosing 'integration and binary' deletes the AppImage, its icon and its menu entry.",
                app.display_name
            ))
            .close_response("keep")
            .default_response("keep")
            .build();
        dialog.add_response("keep", "Keep");
        dialog.add_response("int", "Remove integration only");
        dialog.add_response("bin", "Remove integration and binary");
        let s = sender.clone();
        let id = app.id.clone();
        let owned = app.owned_files.clone();
        let desktop = app.desktop_file_path.clone();
        dialog.choose(
            Some(&self.root),
            None::<&gtk::gio::Cancellable>,
            move |response| {
                let resp = response.to_string();
                if resp != "keep" {
                    for p in &owned {
                        let _ = std::fs::remove_file(p);
                    }
                    if resp == "int" {
                        let _ = std::fs::remove_file(&desktop);
                    }
                    if let Ok(mut db) = Db::open() {
                        db.apps_mut().retain(|a| a.id != id);
                        let _ = db.commit();
                    }
                    s.input(AppMsg::Refresh);
                }
            },
        );
    }

    fn show_preferences_window(&self, sender: &ComponentSender<Self>) {
        let dialog = adw::PreferencesDialog::new();
        dialog.set_content_width(620);
        // Local helper: a ComboRow with a label list.
        let mkcombo = |title: &str, options: &[&str]| -> adw::ComboRow {
            let combo = adw::ComboRow::new();
            combo.set_title(title);
            let list = gtk::StringList::new(options);
            combo.set_model(Some(&list));
            combo
        };

        // ---- Storage ----
        let storage = adw::PreferencesPage::new();
        storage.set_title("Storage");
        let group = adw::PreferencesGroup::new();
        group.set_title("Storage directory");
        group.set_description(Some("AppImages are copied here when you install them."));

        let dir_row = adw::ActionRow::new();
        dir_row.set_title("Directory");
        dir_row.set_subtitle(
            self.settings
                .base_dir
                .as_ref()
                .map(|b| b.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Not set".into())
                .as_str(),
        );
        group.add(&dir_row);

        let btn_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        btn_row.set_halign(gtk::Align::Start);
        btn_row.set_margin_top(6);

        let change = gtk::Button::with_label("Change…");
        let s = sender.clone();
        let w = self.root.clone();
        change.connect_clicked(move |_| open_folder_picker(&w, s.clone()));
        btn_row.append(&change);

        let open = gtk::Button::with_label("Open");
        let s = sender.clone();
        let base = self.settings.base_dir.clone();
        open.connect_clicked(move |_| {
            if let Some(dir) = &base {
                open_in_file_manager(dir);
            } else {
                s.input(AppMsg::ShowToast("No storage directory set".into()));
            }
        });
        btn_row.append(&open);

        let rescan = gtk::Button::with_label("Rescan");
        let s = sender.clone();
        rescan.connect_clicked(move |_| s.input(AppMsg::Rescan));
        btn_row.append(&rescan);

        group.add(&btn_row);
        storage.add(&group);

        // ---- Import ----
        let import = adw::PreferencesPage::new();
        import.set_title("Import");
        let igroup = adw::PreferencesGroup::new();
        igroup.set_title("Import behaviour");
        let mode = mkcombo("Default import mode", &["Copy", "Move"]);
        mode.set_selected(if self.settings.default_import_mode == ImportMode::Move {
            1
        } else {
            0
        });
        let s = sender.clone();
        mode.connect_selected_notify(move |combo| {
            s.input(AppMsg::SetImportMode(if combo.selected() == 1 {
                ImportMode::Move
            } else {
                ImportMode::Copy
            }));
        });
        igroup.add(&mode);
        import.add(&igroup);

        // ---- Updates ----
        let updates = adw::PreferencesPage::new();
        updates.set_title("Updates");
        let ugroup = adw::PreferencesGroup::new();
        ugroup.set_title("Update checks");

        let channel = mkcombo("Release channel", &["Stable", "Prerelease"]);
        channel.set_selected(
            if self.settings.release_channel_default == ReleaseChannel::Prerelease {
                1
            } else {
                0
            },
        );
        let s = sender.clone();
        channel.connect_selected_notify(move |combo| {
            s.input(AppMsg::SetReleaseChannel(if combo.selected() == 1 {
                ReleaseChannel::Prerelease
            } else {
                ReleaseChannel::Stable
            }));
        });
        ugroup.add(&channel);

        let freq = mkcombo("Check frequency", &["Manual", "On launch", "Daily"]);
        freq.set_selected(match self.settings.update_check_pref {
            UpdateCheckPref::Manual => 0,
            UpdateCheckPref::OnLaunch => 1,
            UpdateCheckPref::Daily => 2,
        });
        freq.set_subtitle("Daily checks arrive in a later release");
        let s = sender.clone();
        freq.connect_selected_notify(move |combo| {
            s.input(AppMsg::SetUpdateCheckPref(match combo.selected() {
                1 => UpdateCheckPref::OnLaunch,
                2 => UpdateCheckPref::Daily,
                _ => UpdateCheckPref::Manual,
            }));
        });
        ugroup.add(&freq);
        updates.add(&ugroup);

        // ---- Notifications ----
        let notif = adw::PreferencesPage::new();
        notif.set_title("Notifications");
        let ng = adw::PreferencesGroup::new();
        ng.set_title("Notification style");
        let notif_combo = mkcombo("Style", &["Disabled", "Transient", "Persistent"]);
        notif_combo.set_selected(match self.settings.notification_pref {
            NotificationPref::Disabled => 0,
            NotificationPref::Persistent => 2,
            NotificationPref::Transient => 1,
        });
        let s = sender.clone();
        notif_combo.connect_selected_notify(move |combo| {
            s.input(AppMsg::SetNotificationPref(match combo.selected() {
                0 => NotificationPref::Disabled,
                2 => NotificationPref::Persistent,
                _ => NotificationPref::Transient,
            }));
        });
        ng.add(&notif_combo);
        notif.add(&ng);

        // ---- Safety ----
        let safety = adw::PreferencesPage::new();
        safety.set_title("Safety");
        let sg = adw::PreferencesGroup::new();
        sg.set_title("Confirmations");

        let cfgi = adw::SwitchRow::new();
        cfgi.set_title("Confirm before installing");
        cfgi.set_active(self.settings.confirm_before_install);
        let s = sender.clone();
        cfgi.connect_active_notify(move |sw| s.input(AppMsg::SetConfirmInstall(sw.is_active())));
        sg.add(&cfgi);

        let cfgu = adw::SwitchRow::new();
        cfgu.set_title("Confirm before uninstalling");
        cfgu.set_active(self.settings.confirm_before_uninstall);
        let s = sender.clone();
        cfgu.connect_active_notify(move |sw| s.input(AppMsg::SetConfirmUninstall(sw.is_active())));
        sg.add(&cfgu);
        safety.add(&sg);

        dialog.add(&storage);
        dialog.add(&import);
        dialog.add(&updates);
        dialog.add(&notif);
        dialog.add(&safety);
        dialog.present(Some(&self.root));
    }
}

// ---------- free helpers ----------

fn add_info_row(group: &adw::PreferencesGroup, title: &str, subtitle: &str) {
    let row = adw::ActionRow::new();
    row.set_title(title);
    row.set_subtitle(subtitle);
    group.add(&row);
}

/// Returns the app's real (synthesized) icon scaled to `size` px, falling back
/// to a symbolic placeholder when the icon file is missing.
fn app_icon_at(app: &ManagedApp, size: i32) -> gtk::Image {
    if let Some(icon) = app.icon_path.as_ref() {
        if icon.is_file() {
            if let Ok(pb) = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(icon, size, size, true) {
                let texture = gdk::Texture::for_pixbuf(&pb);
                let img = gtk::Image::from_paintable(Some(&texture));
                img.set_pixel_size(size);
                return img;
            }
        }
    }
    let img = gtk::Image::from_icon_name("application-x-executable-symbolic");
    img.set_pixel_size(size);
    img
}

fn update_status_text(s: &UpdateStatus) -> String {
    match s {
        UpdateStatus::Unknown => "Unknown".into(),
        UpdateStatus::UpToDate => "Up to date".into(),
        UpdateStatus::Available(v) => format!("Update available ({v})"),
        UpdateStatus::Checking => "Checking…".into(),
        UpdateStatus::Failed(e) => format!("Failed ({e})"),
    }
}

fn integration_status_text(s: &IntegrationStatus) -> String {
    match s {
        IntegrationStatus::Healthy => "Healthy".into(),
        IntegrationStatus::DesktopEntryMissing => "Desktop entry missing".into(),
        IntegrationStatus::IconMissing => "Icon missing".into(),
        IntegrationStatus::Broken(e) => format!("Broken ({e})"),
    }
}

fn repair_app(app: &ManagedApp) -> Result<()> {
    if let Some(icon) = &app.icon_path {
        crate::appimage::icon::synthesize_icon(&app.display_name, icon)?;
    }
    let entry = crate::integration::desktop::DesktopEntry::from(app);
    crate::integration::desktop::write_entry(&app.desktop_file_path, &entry)?;
    Ok(())
}

fn load_state() -> (Vec<ManagedApp>, Settings) {
    match Db::open() {
        Ok(db) => {
            let mut apps = db.apps().to_vec();
            let settings = db.document().settings.clone();
            drop(db);
            // Dedup by installed path (keep the first entry) so a previously
            // re-imported AppImage doesn't surface twice.
            let mut seen = std::collections::HashSet::new();
            apps.retain(|a| seen.insert(a.installed_path.clone()));
            (apps, settings)
        }
        Err(_) => (Vec::new(), Settings::default()),
    }
}

fn run_blocking<F: FnOnce() + Send + 'static>(f: F) {
    std::thread::spawn(f);
}

fn open_in_file_manager(path: &std::path::Path) {
    if let Some(parent) = path.parent() {
        let _ = std::process::Command::new("xdg-open").arg(parent).spawn();
    }
}

fn open_file_picker(parent: &impl IsA<gtk::Window>, sender: ComponentSender<App>) {
    let dialog = gtk::FileDialog::builder()
        .title("Choose AppImage file")
        .modal(true)
        .build();
    let s = sender.clone();
    dialog.open(Some(parent), None::<&gtk::gio::Cancellable>, move |res| {
        if let Ok(file) = res {
            if let Some(p) = file.path() {
                s.input(AppMsg::ImportFiles(vec![p]));
            }
        }
    });
}

fn open_folder_picker(parent: &impl IsA<gtk::Window>, sender: ComponentSender<App>) {
    let dialog = gtk::FileDialog::builder()
        .title("Choose AppImage storage directory")
        .modal(true)
        .build();
    dialog.select_folder(Some(parent), None::<&gtk::gio::Cancellable>, move |res| {
        if let Ok(file) = res {
            if let Some(p) = file.path() {
                sender.input(AppMsg::SetBaseDir(p));
            }
        }
    });
}

fn prompt_rename(current: String, sender: ComponentSender<App>) {
    let window = adw::Window::new();
    window.set_title(Some("Rename"));
    window.set_modal(true);
    window.set_resizable(false);
    window.set_default_width(360);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_all(16);
    let entry = gtk::Entry::new();
    entry.set_text(&current);
    body.append(&entry);
    let btns = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    btns.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let w = window.clone();
    cancel.connect_clicked(move |_| w.close());
    btns.append(&cancel);
    let ok = gtk::Button::with_label("Rename");
    ok.add_css_class("suggested-action");
    let s = sender.clone();
    let w = window.clone();
    let e2 = entry.clone();
    ok.connect_clicked(move |_| {
        s.input(AppMsg::Rename(e2.text().to_string()));
        w.close();
    });
    btns.append(&ok);
    body.append(&btns);

    let tbv = adw::ToolbarView::new();
    tbv.add_top_bar(&adw::HeaderBar::new());
    tbv.set_content(Some(&body));
    window.set_content(Some(&tbv));
    window.present();
}

/// Builds the hamburger app menu (Import / Preferences / About / Quit).
fn build_app_menu(parent: adw::Window, sender: ComponentSender<App>) -> gtk::MenuButton {
    let btn = gtk::MenuButton::new();
    btn.set_icon_name("open-menu-symbolic");
    btn.set_tooltip_text(Some("Menu"));

    let pop = gtk::Popover::new();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.set_margin_top(6);
    list.set_margin_bottom(6);
    list.set_margin_start(6);
    list.set_margin_end(6);

    let mk = |label: &str| -> gtk::Button {
        let b = gtk::Button::with_label(label);
        b.set_halign(gtk::Align::Start);
        b.add_css_class("flat");
        b
    };

    let import = mk("Import AppImage…");
    let s = sender.clone();
    let p = pop.clone();
    let w = parent.clone();
    import.connect_clicked(move |_| {
        open_file_picker(&w, s.clone());
        p.popdown();
    });
    list.append(&import);

    let prefs = mk("Preferences…");
    let s = sender.clone();
    let p = pop.clone();
    prefs.connect_clicked(move |_| {
        s.input(AppMsg::Preferences);
        p.popdown();
    });
    list.append(&prefs);

    let about = mk("About Cratey");
    let s = sender.clone();
    let p = pop.clone();
    about.connect_clicked(move |_| {
        s.input(AppMsg::About);
        p.popdown();
    });
    list.append(&about);

    let quit = mk("Quit");
    let s = sender.clone();
    let p = pop.clone();
    quit.connect_clicked(move |_| {
        s.input(AppMsg::Quit);
        p.popdown();
    });
    list.append(&quit);

    pop.set_child(Some(&list));
    btn.set_popover(Some(&pop));
    btn
}

/// Shows the About dialog.
fn show_about(parent: &adw::Window) {
    let about = adw::AboutDialog::new();
    about.set_application_name("Cratey");
    about.set_version(env!("CARGO_PKG_VERSION"));
    about.set_developer_name("Diwakar Bhadriraju");
    about.set_license_type(gtk::License::MitX11);
    about.set_comments("Native Linux AppImage manager");
    about.present(Some(parent));
}

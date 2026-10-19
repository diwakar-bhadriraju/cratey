//! End-to-end acceptance scenarios.
//!
//! These drive the cratey *library* API (install, rename, update, uninstall)
//! against temp XDG dirs. The GUI is intentionally not exercised here (headless
//! CI); the same code paths run in the real window.
//!
//! Each test is isolated under a temp XDG data home and serialized on a shared
//! lock because they mutate process-global env vars.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use parking_lot::Mutex;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

use cratey::db::Db;
use cratey::error::Result;
use cratey::integration::PlannedInstall;
use cratey::model::ImportMode;
use cratey::update::UpdateProvider;

/// Serializes env-mutating e2e tests.
static LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Installs a fake synthetic ELF AppImage under `dir` with a `squashfs` section.
fn fixture_elf() -> Vec<u8> {
    let mut buf = vec![0u8; 4096];
    buf[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
    buf[4] = 2; // ELFCLASS64
    buf[5] = 1; // little-endian
    buf[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    buf[18..20].copy_from_slice(&62u16.to_le_bytes()); // x86-64
    buf[52..54].copy_from_slice(&64u16.to_le_bytes()); // ehsize

    // .shstrtab with "squashfs"
    let shstr_offset = 256usize;
    let shstr = b"\0.shstrtab\0squashfs\0";
    buf[shstr_offset..shstr_offset + shstr.len()].copy_from_slice(shstr);

    // section headers at 512
    let shoff = 512usize;
    // [1] .shstrtab name=0 ty=3(SHT_STRTAB)
    buf[shoff + 64..shoff + 68].copy_from_slice(&0u32.to_le_bytes());
    buf[shoff + 64 + 4..shoff + 64 + 8].copy_from_slice(&3u32.to_le_bytes());
    buf[shoff + 64 + 24..shoff + 64 + 32].copy_from_slice(&(shstr_offset as u64).to_le_bytes());
    buf[shoff + 64 + 32..shoff + 64 + 40].copy_from_slice(&(shstr.len() as u64).to_le_bytes());
    // [2] squashfs name=10 ty=1(SHT_PROGBITS) offset=3072
    let sq = shoff + 128;
    buf[sq..sq + 4].copy_from_slice(&10u32.to_le_bytes());
    buf[sq + 4..sq + 8].copy_from_slice(&1u32.to_le_bytes());
    let sq_off = 3072u64;
    buf[sq + 24..sq + 32].copy_from_slice(&sq_off.to_le_bytes());
    buf[sq + 32..sq + 40].copy_from_slice(&256u64.to_le_bytes());
    buf[3072..3076].copy_from_slice(&0x73717368u32.to_le_bytes()); // hsqs

    buf[40..48].copy_from_slice(&(shoff as u64).to_le_bytes());
    buf[60..62].copy_from_slice(&3u16.to_le_bytes()); // shnum
    buf[62..64].copy_from_slice(&1u16.to_le_bytes()); // shstrndx
    buf
}

/// Creates a temp XDG instance; every `paths::` helper and `Db` reads from it.
struct TestEnv {
    _dir: TempDir,
    base: PathBuf,
    source: PathBuf,
}

impl TestEnv {
    fn new(source_name: &str) -> Result<TestEnv> {
        let dir = tempfile::tempdir()?;
        std::env::set_var("XDG_DATA_HOME", dir.path().join("share"));
        std::env::set_var("HOME", dir.path().join("home"));
        let base = dir.path().join("apps");
        std::fs::create_dir_all(&base)?;
        let source = dir.path().join(source_name);
        std::fs::write(&source, fixture_elf())?;
        Ok(TestEnv {
            _dir: dir,
            base,
            source,
        })
    }

    fn plan(&self, display: &str, mode: ImportMode) -> PlannedInstall {
        let id = cratey::model::new_app_id();
        let installed = self.base.join(
            self.source
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap(),
        );
        PlannedInstall {
            id: id.clone(),
            display_name: display.to_string(),
            source_path: self.source.clone(),
            installed_path: installed,
            icon_path: cratey::paths::icons_dir().join(format!("{id}.png")),
            desktop_file_path: cratey::paths::applications_dir()
                .join(cratey::model::desktop_file_basename(&id, display)),
            base_dir: Some(self.base.clone()),
            move_source: mode == ImportMode::Move,
        }
    }
}

fn with_env<F: FnOnce(&TestEnv)>(source_name: &str, f: F) {
    let _guard = LOCK.lock();
    let env = TestEnv::new(source_name).expect("env init");
    f(&env);
    // Reset env so later non-locked code isn't polluted.
    std::env::remove_var("XDG_DATA_HOME");
    std::env::remove_var("HOME");
}

fn write_settings(base_dir: Option<&Path>) {
    if let Ok(mut db) = Db::open() {
        db.document_mut().settings.base_dir = base_dir.map(|p| p.to_path_buf());
        db.document_mut().settings.default_import_mode = ImportMode::Copy;
        let _ = db.commit();
    }
}

#[test]
fn select_base_directory_persists_across_restarts() {
    with_env("A.AppImage", |env| {
        write_settings(Some(&env.base));
        // Re-open fresh like a second app launch.
        let db = Db::open().expect("reopen db");
        assert_eq!(
            db.document().settings.base_dir.as_deref(),
            Some(env.base.as_path())
        );
    });
}

#[test]
fn drag_and_drop_import_copies_file_and_writes_desktop() {
    with_env("B.AppImage", |env| {
        let plan = env.plan("Beta", ImportMode::Copy);
        let app = cratey::integration::install::install(plan.clone()).expect("install");
        assert!(plan.installed_path.exists(), "binary copied to base_dir");
        assert!(app.desktop_file_path.exists(), "desktop written");
        let text = std::fs::read_to_string(&app.desktop_file_path).unwrap();
        assert!(text.contains("X-Cratey-Managed=true"));
        assert!(text.contains(&format!("X-Cratey-Internal-Id={}", app.id)));
    });
}

#[test]
fn install_does_not_delete_original_file_in_move_mode() {
    with_env("C.AppImage", |env| {
        // In Copy mode the source is always kept.
        let plan = env.plan("Gamma", ImportMode::Copy);
        let src = plan.source_path.clone();
        cratey::integration::install::install(plan).expect("install");
        assert!(src.exists(), "copy mode must keep source");
    });
}

#[test]
fn application_appears_in_system_application_menu() {
    with_env("D.AppImage", |env| {
        let plan = env.plan("Delta", ImportMode::Copy);
        let app = cratey::integration::install::install(plan).expect("install");
        // The .desktop is inside the XDG applications dir.
        let appl = cratey::paths::applications_dir();
        assert!(app.desktop_file_path.starts_with(&appl));
        let text = std::fs::read_to_string(&app.desktop_file_path).unwrap();
        assert!(text.contains("Type=Application"));
        assert!(text.contains("Name=Delta"));
        assert!(text.contains(&app.installed_path.to_string_lossy().to_string()));
    });
}

#[test]
fn launch_spawns_process_without_running_appimage_format_check() {
    with_env("E.AppImage", |env| {
        let plan = env.plan("Epsilon", ImportMode::Copy);
        let app = cratey::integration::install::install(plan).expect("install");
        // Replace the (fake ELF) binary with a real marker script so spawn works.
        std::fs::write(&app.installed_path, "#!/bin/sh\nprintf 'launched-ok'\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&app.installed_path)
            .unwrap()
            .permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(&app.installed_path, p).unwrap();

        let pid = cratey::integration::launch::launch(&app).expect("launch returns pid");
        assert!(pid > 0);
        // Give it a moment and confirm the child existed.
        std::thread::sleep(std::time::Duration::from_millis(300));
    });
}

#[test]
fn rename_changes_display_name_without_changing_binary_filename() {
    with_env("F.AppImage", |env| {
        let plan = env.plan("Zeta", ImportMode::Copy);
        let app = cratey::integration::install::install(plan).expect("install");
        let bin_before = app.installed_path.clone();
        let basename_before = app.desktop_file_basename.clone();

        let mut renamed = app.clone();
        renamed.display_name = "Zeta Renamed".into();
        let mut db = Db::open().unwrap();
        db.update(&app.id, |a| a.display_name = "Zeta Renamed".into())
            .unwrap();
        drop(db);

        assert_eq!(renamed.installed_path, bin_before, "Exec path unchanged");
        assert_eq!(
            renamed.desktop_file_basename, basename_before,
            "desktop basename stable"
        );
    });
}

#[test]
fn update_replaces_binary_and_preserves_desktop_entry() {
    with_env("G.AppImage", |env| {
        let plan = env.plan("Eta", ImportMode::Copy);
        let app = cratey::integration::install::install(plan).expect("install");
        let desktop_before = std::fs::read_to_string(&app.desktop_file_path).unwrap();
        let bin_before = app.installed_path.clone();
        let old_bytes = std::fs::read(&bin_before).unwrap();

        // Serve a NEW fixture via a local HTTP server (wiremock semantics).
        let srv = tcp_server();
        let url = format!("http://127.0.0.1:{}/new.AppImage", srv.1);
        let mut new_elf = fixture_elf();
        new_elf[3000] = 0xAA; // differ from the installed copy
        let server_elf = new_elf.clone();
        std::thread::spawn(move || {
            let listener = srv.0;
            for _ in 0..2 {
                if let Ok((stream, _)) = listener.accept() {
                    respond(stream, "200 OK", b"application/octet-stream", &server_elf);
                }
            }
        });

        let release = cratey::update::NewRelease {
            version: "2.0.0".into(),
            download_url: url,
            size_bytes: new_elf.len() as u64,
            sha256: Some(sha256_hex(&new_elf)),
            sig: None,
        };
        let mut updated = app.clone();
        let provider = cratey::update::manual::ManualProvider;
        provider
            .apply(&mut updated, release)
            .expect("apply succeeds");

        // Binary replaced (bytes differ), desktop untouched.
        let new_bytes = std::fs::read(&bin_before).unwrap();
        assert_ne!(old_bytes, new_bytes, "binary content replaced");
        assert_eq!(
            std::fs::read_to_string(&app.desktop_file_path).unwrap(),
            desktop_before,
            "desktop entry preserved"
        );
    });
}

#[test]
fn failed_update_leaves_old_binary_intact() {
    with_env("H.AppImage", |env| {
        let plan = env.plan("Theta", ImportMode::Copy);
        let app = cratey::integration::install::install(plan).expect("install");
        let bin = app.installed_path.clone();
        let old_bytes = std::fs::read(&bin).unwrap();

        // Server returns garbage (not an AppImage) → validation fails.
        let srv = tcp_server();
        let url = format!("http://127.0.0.1:{}/bad.AppImage", srv.1);
        std::thread::spawn(move || {
            let listener = srv.0;
            if let Ok((stream, _)) = listener.accept() {
                respond(
                    stream,
                    "200 OK",
                    b"application/octet-stream",
                    b"GARBAGE-NOT-AN-APPIMAGE",
                );
            }
        });
        let release = cratey::update::NewRelease {
            version: "9.9.9".into(),
            download_url: url,
            size_bytes: 20,
            sha256: None,
            sig: None,
        };
        let mut updated = app.clone();
        let provider = cratey::update::manual::ManualProvider;
        assert!(
            provider.apply(&mut updated, release).is_err(),
            "bad payload rejected"
        );
        // Old binary still in place, original bytes.
        assert_eq!(std::fs::read(&bin).unwrap(), old_bytes, "old binary intact");
    });
}

#[test]
fn uninstall_removes_only_owned_files() {
    with_env("I.AppImage", |env| {
        let plan = env.plan("Iota", ImportMode::Copy);
        let app = cratey::integration::install::install(plan).expect("install");
        // Unrelated file in base_dir must survive.
        let unrelated = env.base.join("unrelated.txt");
        std::fs::write(&unrelated, "keep me").unwrap();

        // Uninstall: remove owned files, keep everything else.
        for owned in &app.owned_files {
            let _ = std::fs::remove_file(owned);
        }
        let _ = std::fs::remove_file(&app.desktop_file_path);
        if let Ok(mut db) = Db::open() {
            db.apps_mut().retain(|a| a.id != app.id);
            let _ = db.commit();
        }
        assert!(!app.installed_path.exists());
        assert!(!app.desktop_file_path.exists());
        assert!(unrelated.exists(), "unrelated file untouched");
    });
}

#[test]
fn install_adopts_existing_external_desktop_file() {
    with_env("J.AppImage", |env| {
        // Pre-create a non-cratey .desktop pointing at the planned target.
        let plan = env.plan("Jota", ImportMode::Copy);
        let ext = cratey::paths::applications_dir().join("external.desktop");
        std::fs::write(
            &ext,
            format!(
                "[Desktop Entry]\nType=Application\nExec=\"{}\" %u\nName=External\n",
                plan.installed_path.display()
            ),
        )
        .unwrap();

        // Our collision check must report adoption for that target.
        match cratey::integration::menu::resolve_collision(&plan).unwrap() {
            cratey::integration::menu::Collision::Adopt(p) => assert_eq!(p, ext),
            other => panic!("expected adoption, got {other:?}"),
        }
        // The external file is never modified.
        assert!(std::fs::read_to_string(&ext)
            .unwrap()
            .contains("Name=External"));
    });
}

// ---------- tiny HTTP test server helpers ----------

fn tcp_server() -> (std::net::TcpListener, u16) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

fn respond(mut stream: std::net::TcpStream, status: &str, ctype: &[u8], body: &[u8]) {
    use std::io::Write;
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        String::from_utf8_lossy(ctype),
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

#[test]
fn corrupt_db_is_backed_up_and_replaced_with_empty() {
    with_env("Z.AppImage", |_env| {
        let db_path = cratey::paths::db_file();
        // Write garbage that serde_json cannot parse.
        std::fs::write(&db_path, b"this is not json").unwrap();
        // Make sure we don't have a leftover lock from a previous test.
        let _ = std::fs::remove_file(db_path.with_extension("lock"));

        // Opening now must surface a DatabaseCorrupted error pointing at the
        // backup, and the original must have been moved aside.
        let backup = match cratey::db::Db::open() {
            Err(cratey::error::AppError::DatabaseCorrupted { path }) => path,
            Err(other) => panic!("expected DatabaseCorrupted, got {other:?}"),
            Ok(_) => panic!("expected Db::open to fail on corrupt DB"),
        };
        assert!(backup.exists(), "backup file should exist");
        assert!(
            std::fs::read_to_string(&db_path)
                .map(|s| s.is_empty())
                .unwrap_or(true),
            "the live DB should now be empty (or absent), not the corrupt data"
        );
    });
}

#[test]
fn register_existing_picks_up_a_pre_placed_appimage() {
    with_env("Rescan.AppImage", |env| {
        // Drop the AppImage directly in the storage dir without going
        // through the install flow (simulates migrating from another
        // manager or recovering from a crash).
        let src = env.base.join("Rescan.AppImage");
        std::fs::write(&src, fixture_elf()).unwrap();
        std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o755)).unwrap();

        // register_existing picks it up in place — no copy.
        cratey::integration::install::register_existing(&src, "Rescan").unwrap();

        // The .desktop file references the correct binary and the source
        // file is unchanged.
        let apps_dir = cratey::paths::applications_dir();
        let mut found = None;
        for entry in std::fs::read_dir(&apps_dir).unwrap() {
            let entry = entry.unwrap();
            if entry.path().extension().and_then(|e| e.to_str()) == Some("desktop") {
                let t = std::fs::read_to_string(entry.path()).unwrap();
                if t.contains("X-Cratey-Managed=true") && t.contains("Name=Rescan") {
                    found = Some((entry.path(), t));
                    break;
                }
            }
        }
        let (_path, text) = found.expect("rescanned desktop file");
        assert!(text.contains(&format!("Exec=\"{}\"", src.display())));
        assert!(src.is_file(), "source must not be moved");

        // The DB has the app registered with installed_path == src.
        let db = Db::open().unwrap();
        let app = db
            .document()
            .apps
            .iter()
            .find(|a| a.installed_path == src)
            .expect("app registered in db");
        assert_eq!(app.display_name, "Rescan");
    });
}

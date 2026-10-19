# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Richer window: header bar with an Import button and a hamburger app menu
  (Import / Preferences / About / Quit).
- Full multi-page Preferences dialog: Storage (change/open/rescan directory),
  Import (default mode), Updates (channel + check frequency), Notifications
  (style), and Safety (confirmation toggles) — all persisted.

### Fixed / polish
- File pickers now pass the parent window (fixes Import doing nothing on
  Wayland, where GTK's portal-based dialog needs a parent).
- Preferences and main windows open at a usable size.
- App rows and the details pane show each app's real (synthesized) icon
  instead of a generic placeholder.

## [0.1.0] - 2026-10-19

Phase 1 initial release.

### Added
- GTK4/Libadwaita GUI (relm4) with an app list, details pane, and search.
- Drag-and-drop and `--bin-import` import of Type-2 ELF AppImages.
- Import preview dialog with editable display name and Copy/Move mode.
- `.desktop` menu integration with collision adoption (never overwrites
  foreign entries) and `update-desktop-database` refresh.
- Launch, rename, open location, repair integration, and uninstall (owned
  files only).
- Synthesized 256×256 accent-color icons (no `unsquashfs`).
- GitHub Releases update provider (manual + on-demand) with sha256
  verification and atomic binary swap with rollback.
- Atomic JSON persistence with an exclusive process lock.

### Deferred (Phase 2)
- Type-1 (ISO) AppImage support.
- Embedded `.DirIcon` extraction via `unsquashfs`.
- Scheduled update checks and desktop notifications.
- Zsync-based delta updates.

[Unreleased]: https://github.com/diwakar-bhadriraju/cratey/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/diwakar-bhadriraju/cratey/releases/tag/v0.1.0
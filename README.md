# Cratey — Native Linux AppImage Manager

Manage AppImages like native apps: import them into a storage directory,
launch them, rename them, get `.desktop` menu integration, and update them
through GitHub Releases.

**Phase 1 features**

- Drag-and-drop (or `--bin-import`) an AppImage into the library
- Import preview (editable display name, Copy/Move)
- `.desktop` integration with menu collision adoption
- Launch, open location, rename, repair integration, uninstall
- GitHub Releases update checks (manual + on-demand)
- Synthesized accent-color icons (no `unsquashfs`, no FUSE, never executes an AppImage)

## Install

Requires Rust 1.91+ and the GTK 4 / libadwaita 1 development headers.

```sh
cargo install --path . --locked --jobs 8
```

Run:

```sh
cratey
```

## Usage

1. On first launch, choose an application storage directory (e.g. `~/Applications`).
2. Drag AppImage files into the window (or use **Browse…** / `cratey --bin-import Foo.AppImage`).
3. Review the import preview and click **Install**.
4. Select an app in the list to launch, rename, check for updates, or uninstall.

### Headless import

```sh
cratey --bin-import ~/Downloads/Foo.AppImage
```

Writes the same `.desktop`, icon and DB entries as the GUI and exits 0 on
success. Useful from a file-manager `.desktop` MimeType handler.

## Troubleshooting

- **A `.desktop` file I made by hand points at an AppImage and install seems to
  ignore it.** Cratey never overwrites files it does not manage. If an existing
  entry points at the *same binary* Cratey adopts it; if it points elsewhere
  Cratey writes its own `crate-<id>.desktop` and leaves yours untouched.
- **"Not a valid AppImage"** — check the file is a Type-2 ELF AppImage (see
  limitations).
- **`update-desktop-database` is missing** — the app still works; the entry is
  a plain text file. Install `desktop-file-utils` to silence the warning.
- **"another instance is running"** — Cratey takes an exclusive lock on its
  database; a duplicate process exits.

## Limitations (Phase 1)

- **Type-1 (ISO) AppImages are not supported** — only Type-2 ELF AppImages.
  Type-1 is a planned Phase 2 item.
- **Synthesized icons only.** Embedded `.DirIcon` extraction (via
  `unsquashfs`) is deferred to v2 and is not required here. `cratey` never
  needs `squashfs-tools`, never runs FUSE, and never executes an AppImage to
  inspect it.
- **Scheduled update checks, desktop notifications, and embedded-zsync delta
  updates are Phase 2.** Updates are manual / on-demand only in v1.
- No network access is performed unless you click "Check for updates".

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## License

MIT — see [LICENSE](LICENSE).
# Contributing

## Build & test

The build box has 12 logical CPUs; the repo ships `.cargo/config.toml`
preferring **8 jobs** locally to keep the system usable. CI uses 2.

```sh
cargo fmt                 # formatting
cargo clippy --jobs 8 -- -D warnings   # lints (must be clean)
cargo test --jobs 8 --all-features
cargo build --jobs 8 --release
```

`cargo` jobs on the CLI: always pass `--jobs 8` (or `JOBS=8`) **before** any
`--` separator, because the `--` separator strips remaining args from
Cargo and forwards them to `rustc`/`clippy-driver` (which doesn't accept
`--jobs`). Never the default 12.

## Code style

- **Minimize new code; match the surrounding pattern.** Prefer the smallest
  change that fully solves the task.
- AppImages are never executed or mounted during inspection; do not introduce
  an `unsquashfs`/FUSE call in v1.
- Keep pure/fixture helpers in `#[cfg(test)]`. Tests are deterministic and
  isolated (temp XDG dirs + a shared lock in `tests/e2e.rs`).
- Don't add dependencies for a few lines; use what the project already has.
- Before editing a `pub` symbol, run an LSP/code-intelligence "references"
  query and update every caller.

## Pre-commit (optional)

A `pre-commit` hook is optional; if you use one:

```yaml
repos:
  - repo: local
    hooks:
      - id: rust-fmt
        name: cargo fmt --check
        entry: cargo fmt -- --check
        language: system
        pass_filenames: false
      - id: rust-clippy
        name: clippy
        entry: cargo clippy --jobs 2 -- -D warnings
        language: system
        pass_filenames: false
```

## Testing guidance

- Unit tests live beside the code (`src/**`).
- `tests/e2e.rs` holds the 10 acceptance scenarios.
- `tools/make_fixture.sh` regenerates the synthetic AppImage fixture.

## Release process

Tag `v*.*.*`; `.github/workflows/release.yml` builds and uploads the release
binaries and checksums. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
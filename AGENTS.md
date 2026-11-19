# Repository Guidelines

## Project Structure & Module Organization

PixClip is a native Rust desktop media toolkit for Windows and Linux. The UI uses egui/eframe; media operations use external yt-dlp, FFmpeg, ffprobe, and Streamlink executables.

- `src/main.rs`: GUI/CLI entry points and terminal menu.
- `src/model.rs`: compatible JSON history/settings, paths, time parsing and disk checks.
- `src/jobs.rs`: persistent queue, scheduling and worker threads.
- `src/process.rs`: shared subprocess runner, process-tree isolation and cancellation.
- `src/media.rs`: download/Live workflows, media command builders and preview capture.
- `src/tools.rs`: tool lookup, verified installation and hardware detection.
- `src/app.rs`: original mint/amber UI, queue/history controls, mascot and clip previews.
- Unit tests live beside their modules; `src/integration.rs` contains ignored real-tool tests.
- `assets/`: mascot PNGs and Windows icon; `docs/README-PixClip.md`: Thai user guide.
- `scripts/`: media-tool installer and platform-specific helpers under `windows/` and `linux/`.
- `tools/` and `dist/`: local executables and ZIP distributions; generated contents are excluded from Git.
- `PixClip.bat`, `PixClip`, `Install-PixClip.bat`, and `install.sh`: root Windows/Linux launchers and installers.

## Build, Test, and Development Commands

Run commands from the project root with a working Rust toolchain.

- `cargo run -- --gui` or `cargo run -- --cli`: launch the app or terminal menu.
- `cargo fmt --check`: check formatting.
- `cargo clippy --locked --all-targets -- -D warnings`: check Rust diagnostics.
- `cargo test --locked`: run offline regression tests.
- `cargo test --locked -- --ignored`: run real-tool integration tests with synthetic media and localhost.
- `scripts/build.ps1` (Windows) or `bash scripts/build.sh` (Linux): build the release executable used by the root launchers.

Commit `Cargo.lock`. Distributions include the platform executable, launchers, documentation, scripts and licenses; assets/fonts are embedded in the binary. Set `PIXCLIP_STATE_DIR` to a temporary directory for GUI/integration checks so real user history is untouched.

## Coding Style & Naming Conventions

Use rustfmt, `snake_case` functions/variables, `PascalCase` types, and `UPPER_SNAKE_CASE` constants. Prefer `Path`/`PathBuf` and subprocess argument lists. Keep GUI updates on the main thread; workers send events. Reuse the shared process runner for cancellation, deadlines and disk checks. Preserve the established mint/amber pixel theme.

## Testing Guidelines

Use Rust `#[test]` functions and `#[ignore]` for tests requiring external media tools. Cover queue transitions, cancellation, deadlines, legacy history and file preservation. Integration tests use synthetic media and localhost; they do not prove public-site compatibility. Report platform limitations and distinguish Windows GUI checks from Linux builds and real Wayland interaction.

## Commit & Pull Request Guidelines

Use concise imperative commit subjects. PRs explain behavior changes, list validation and platform limitations, and include screenshots for GUI changes. Exclude downloaded media, private history/logs, `target/`, `.cache/`, `tools/` executables, `dist/` archives and machine-specific `.lnk` files from source changes.

<p align="center"><img src="assets/pixclip-mascot-512.png" alt="PixClip pixel-art camera mascot" width="256"></p>
<h1 align="center">PixClip</h1>
<p align="center">Download / Record / Convert / Cut / Join</p>

PixClip is a native Rust media toolkit for Windows, Linux, and macOS with a dark
mint-and-amber theme and the original control layout.
The application uses yt-dlp, FFmpeg, ffprobe and Streamlink as external tools.
It does not launch Python or require a Python installation to run its UI or queue.
Streamlink and yt-dlp retain their own runtime and packaging requirements.

## Run

Get the Windows, Linux and macOS builds from [GitHub Releases](https://github.com/PeakNAtanon/PixClip/releases/latest).

- **Windows x64** — run the MSI for a per-user install with a Start menu shortcut, or download the portable EXE. Open PixClip and choose **Install tools** on first use.
- **Linux x64** — extract the archive, install FFmpeg with your distribution's package manager, and run `./PixClip --install-tools`, then `./PixClip`. Release archives are built on Ubuntu 24.04.
- **macOS** — choose `arm64` for Apple Silicon or `x64` for Intel. Extract the archive, run `brew install ffmpeg streamlink`, and then run `./PixClip --install-tools` followed by `./PixClip`.

Run `./PixClip --cli` in a terminal on Linux or macOS. The Windows MSI installs in
`%LOCALAPPDATA%\PixClipApp`, while queue history is stored in `%LOCALAPPDATA%\PixClip`.

If you downloaded source code, build the application first:

```powershell
# Windows: Rust with its matching MSVC or MinGW build tools is required.
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1
.\PixClip.bat
```

```bash
# Linux: Rust, a C compiler, pkg-config, OpenSSL and desktop development libraries.
# Debian / Ubuntu build dependencies:
sudo apt install build-essential pkg-config libssl-dev libx11-dev libxi-dev libxrandr-dev libxcursor-dev libxkbcommon-dev libwayland-dev libasound2-dev
bash scripts/build.sh
./PixClip
```

On macOS, install Rust, the Xcode Command Line Tools, and Homebrew dependencies.

```bash
xcode-select --install
brew install ffmpeg streamlink
bash scripts/build.sh
./PixClip --install-tools
./PixClip
```

The build scripts create `PixClip.exe` on Windows or `pixclip-bin` on Linux and macOS.
Use `bash scripts/package-unix.sh` after building to create the native archive in `dist/`.

To reproduce the Windows MSI, build the release executable first, then use the
WiX 5.0.2 command-line tool with its matching UI extension.

```powershell
dotnet tool install --global wix --version 5.0.2
wix extension add -g WixToolset.UI.wixext/5.0.2
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-msi.ps1
```

### Automatic release packages

The [release workflow](.github/workflows/release.yml) runs when a `v*` tag is pushed.
The tag must match the three-part version in `Cargo.toml`, such as `v0.2.22`
for version `0.2.22`. Update `Cargo.toml` and `Cargo.lock` and commit the changes
before creating the tag. This workflow does not create or push tags itself.

- Windows: the existing build scripts and WiX 5.0.2 produce an x64 MSI.
- Linux: [cargo-deb](https://github.com/kornelski/cargo-deb) and
  [cargo-generate-rpm](https://github.com/cat-in-136/cargo-generate-rpm)
  produce x64 DEB/RPM packages with a desktop launcher. FFmpeg and GTK are
  declared as runtime dependencies; DEB also detects linked system libraries.
- Arch Linux: a native Arch container runs `makepkg` to produce an x64
  `.pkg.tar.zst` package with FFmpeg, yt-dlp and Streamlink dependencies.
- Flatpak: the GNOME 49 SDK builds the app from vendored Rust sources. The x64
  `.flatpak` bundle includes FFmpeg/ffprobe with H.264 and MP3 encoding, yt-dlp
  and portable Streamlink. Downloads, Music and Videos are accessible; native
  file dialogs can grant access to other selected files through portals.
- macOS: [cargo-bundle](https://github.com/burtonageo/cargo-bundle) packages a
  Universal Binary for Apple Silicon and Intel; create-dmg creates the DMG.
  The app is signed ad hoc for executable integrity, without Developer ID
  signing or notarization. FFmpeg and Streamlink still require Homebrew.

All five builds must succeed before the workflow publishes the six installers
and a SHA-256 checksum file to the tag's GitHub Release. A new release stays a
draft until its uploads finish. The workflow uses the built-in `GITHUB_TOKEN`;
only its publishing job has `contents: write` permission.
Linux packages are built on `ubuntu-latest` and require compatible system
libraries; this does not guarantee support for older distributions.

Install the Arch package with `sudo pacman -U PixClip-X.Y.Z-linux-x64.pkg.tar.zst`.
For Flatpak, enable the Flathub remote to obtain its runtime, then run
`flatpak install --user PixClip-X.Y.Z-linux-x64.flatpak` and
`flatpak run io.github.PeakNAtanon.PixClip`. These files are published as GitHub
release assets; the workflow does not submit packages to AUR or Flathub.

Linux users install FFmpeg/ffprobe with their package manager.
The media-tool installer downloads standalone yt-dlp with checksum verification;
on Windows it also downloads verified FFmpeg/ffprobe. Streamlink uses its official
portable Windows ZIP or Linux AppImage with the release SHA-256 digest.
On macOS, PixClip uses the installed FFmpeg, ffprobe and Streamlink, plus the verified
universal yt-dlp executable. Linux AppImages require glibc 2.28 or newer.
On Linux and macOS, downloaded tools are stored in the user's state directory
under `tools/`, so installing them does not write into `/usr/bin` or the `.app`.
macOS also searches `/opt/homebrew/bin` and `/usr/local/bin` for media tools when
launched from Finder.

## Features

### App updates

PixClip 0.2.22 and later check GitHub Releases at startup and every six hours.
Use **HELP → Check PixClip updates** to check manually. When a newer stable
version is available, the app displays an update window.

On Windows x64, **Update and restart** downloads the official MSI, verifies its
size and GitHub SHA-256 digest, and waits for PixClip to exit before installing.
The installer shows progress and reopens PixClip. Queued or running jobs must
finish first; history, settings and downloaded media are kept. Portable users
are migrated to a per-user MSI installation. Failed downloads keep the app open;
installer failures show an error and attempt to reopen the previous app.

Linux and macOS show update notifications and a Releases link; install the
appropriate package with the platform package manager. Automatic installation
in this release is supported on Windows x64. Existing versions without this
feature need one manual upgrade to a build containing it.

### Media tools

Preview the current [main window](docs/screenshots/current-main.png),
[queue](docs/screenshots/current-queue.png), [playlist selection](docs/screenshots/current-playlist.png),
[clip editor](docs/screenshots/current-cut.png), [join](docs/screenshots/current-join.png),
and [help](docs/screenshots/current-help.png).
The [mascot](assets/pixclip-mascot-64.aseprite), [pixel camera icon](assets/pixclip-icon.aseprite), [button icons](assets/button-icons.aseprite), [header filmstrip](assets/pixclip-filmstrip.aseprite), [UI panels](assets/pixclip-panels.aseprite), and [dropdown arrows](assets/dropdown-arrows.aseprite) are editable in Aseprite.
See the [comparison and test report](docs/RUST-PARITY.md) for the original Python references and validation limits.

- Video, playlist, audio and MPEG-TS downloads, including direct Kick VOD playback.
- Select playlist items before queueing, download a time range, or save subtitles as SRT. Spotify Track and Playlist / Album modes let you review YouTube matches before queueing.
- Live recording with Streamlink or benchmarked CPU and hardware H.264 encoding.
- One to five parallel downloads, pause, cancel, retry, history and job logs.
- Scheduled Live recordings with a fixed end time and reconnect attempts.
- Optional Netscape `cookies.txt` files for authenticated sources.
- Convert multiple files at once, convert TS to MP4 or HEVC to H.264, compress MP4, or extract audio as MP3, M4A, FLAC or WAV.
- Frame previews, fast and accurate cuts, and ordered joins.
- For smaller video files, choose **CONVERT → Compress MP4**, select **Balanced**, **Smaller**, or **Smallest**, and optionally limit resolution to 1080p or 720p. PixClip saves a separate MP4. Results depend on the source.
- Disk-space checks during work; cancellation keeps partial media.
- Thai/English help, native file dialogs, clipboard paste and hardware status.

Scheduled jobs need PixClip open and the computer awake. Pause stops new jobs.
Delete and Clear all remove queue/history entries while preserving media files.
Fast joins require matching stream formats; Compatible mode re-encodes.
The GUI offers Compatible mode when the selected clips have different stream settings.
Pause queue holds downloads and scheduled recordings; local editing remains available.
Cut Clip has a timeline with draggable trim handles, precise time fields, and preview playback with audio. Press Space to play or pause.

To cut a downloaded video, open **Queue / History → Files** for the completed job, select the video, and click **Open in Cut clip**. The editor loads it directly onto the timeline. This workflow is also in **HELP → 06 Queue / History**.

## Auto encoder

PixClip times a short synthetic 1080p encode with each usable FFmpeg encoder.
It measures the Live and file presets separately and keeps each winner for the
current app session. The status bar displays both choices, and each job log shows
the encoder being tried. Compare them from the terminal with `--compatibility-test`.

| Platform | H.264 candidates |
| --- | --- |
| Windows | CPU `libx264`, NVIDIA NVENC, AMD AMF, Intel Quick Sync |
| Linux | CPU `libx264`, NVIDIA NVENC, AMD AMF, Intel Quick Sync, VA-API render devices |
| macOS | CPU `libx264`, Apple VideoToolbox hardware encoder |

Only candidates that complete the actual test are ranked. Auto applies to Live
GPU/CPU, H.264 conversions and compression, accurate cuts and compatible joins,
download compatibility conversions, and eligible video time-range downloads.
Auto retries a failed hardware encode using the CPU path. Time range retries
with yt-dlp's original options when a hardware encode fails, for example when
the source uses a container that cannot hold H.264.

Fast modes copy the original streams. The explicitly named NVIDIA modes keep
their NVIDIA encoders. Benchmark results depend on the workload and system load,
and the same quality level can produce different file sizes with different encoders.

## Existing history

The Rust app reads the existing `jobs.json` and `settings.json` formats:

- Windows: `%LOCALAPPDATA%/PixClip/`
- Linux and macOS: `${XDG_STATE_HOME:-~/.local/state}/PixClip/`

Before its first history rewrite it preserves an exact `jobs.pre-rust.json` copy.
Interrupted work needs manual retry. Invalid history is kept and new jobs use a
separate recovery file. A per-user lock prevents concurrent history writers.
Unknown JSON fields and legacy output-folder paths are preserved.
Invalid settings load defaults after preserving an exact `settings-invalid-*.json` backup.
Local edit retries restore the saved concat list and ask before replacing output.

For isolated development/tests, set `PIXCLIP_STATE_DIR` to a temporary directory.
Keep cookie exports private; they contain login credentials. Only their path is
saved. Resolved signed media URLs are hidden from job logs.

Downloads are stored under the chosen folder:

```text
Videos/ or Live/ or Playlists/ or Audio/
  YYYY-MM-DD/
    HHMMSS - source [job-id]/
```

## CLI

```text
PixClip.exe --cli
PixClip.exe --compatibility-test
PixClip.exe --install-tools
PixClip.exe --update-ytdlp
PixClip.exe --download-ts URL OUTPUT_DIR
PixClip.exe --download-live-ts-auto URL OUTPUT_DIR
PixClip.exe --download-live-ts-nvenc URL OUTPUT_DIR
PixClip.exe --record-live-streamlink URL OUTPUT_DIR --cookies-file cookies.txt
PixClip.exe --convert 0 INPUT.ts
PixClip.exe --convert 4 INPUT.mp4 --compression smallest --max-resolution 720
PixClip.exe --cut INPUT.mp4 OUTPUT.mp4 00:01:00 00:01:30 --accurate
PixClip.exe --join OUTPUT.mp4 FIRST.mp4 SECOND.mp4
```

On Linux replace `PixClip.exe` with `./PixClip`. Use `--help` for all commands.
Direct CLI jobs use a temporary queue, leave GUI history untouched, and refuse
to overwrite outputs. **TUI** opens the terminal menu from the desktop app.
The terminal menu includes the original choices 1–20 and adds 21–26 for time ranges, subtitles, playlist item numbers, compression, audio extraction, and cutting a clip. Direct CLI download folders
omit the queue job ID, matching the original terminal workflow.

## Code and checks

```text
src/main.rs       Entry points and terminal menu
src/model.rs      Settings, compatible history, paths and validation
src/jobs.rs       Queue transitions and worker threads
src/process.rs    Shared process-tree cancellation and output capture
src/media.rs      Download, Live, convert, cut, join and preview operations
src/tools.rs      Tool lookup, verified installation and hardware detection
src/app.rs        Native UI with the original theme and workflows
assets/           Original artwork and bundled OFL Thai font
scripts/          Build/install helpers
```

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
# FFmpeg must be discoverable for the UI wiring checks.
cargo test --locked
# Requires FFmpeg, ffprobe and yt-dlp; synthetic media and localhost only:
cargo test --locked -- --ignored
```

These checks do not prove public-site compatibility or every GPU driver/backend.
GUI smoke builds can enable `--features screenshot` and set `PIXCLIP_SCREENSHOT_TO`
to a PNG filename. Always use an isolated `PIXCLIP_STATE_DIR` for UI tests.

See [the Thai guide](docs/README-PixClip.md) and [contributor instructions](AGENTS.md).
Support development on [Ko-fi](https://ko-fi.com/peaknatanon).

## License

[MIT](LICENSE). External media tools keep their own licenses. The bundled
Noto Sans Thai font is covered by [OFL](assets/fonts/OFL.txt).
Run the executable with `--licenses` to read the app and font notices in the terminal.

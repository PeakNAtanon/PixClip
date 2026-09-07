<p align="center">
  <img src="assets/pixclip-icon.png" alt="PixClip logo" width="160">
</p>

<h1 align="center">PixClip</h1>

<p align="center">
  Universal pixel-art media toolkit for Windows and Linux
</p>

<p align="center">
  Download videos and Live streams · Convert · Cut · Join · Schedule recordings
</p>

PixClip is a Python-first desktop media toolkit with a terminal-inspired pixel-art GUI. It uses yt-dlp, FFmpeg, ffprobe, and Streamlink while keeping the workflow simple for both Windows and Linux.

## Features

- Download video, playlist, MP3, MPEG-TS (`.ts`), and Live streams.
- Kick VOD playback fallback for supported public VOD URLs.
- Record Live with Streamlink or HLS and reconnect after temporary failures.
- Use an optional Netscape `cookies.txt` file for sites that require a logged-in session.
- Detect NVIDIA/AMD hardware and choose NVENC, AMF, VA-API, or CPU encoding automatically.
- Convert TS to MP4, transcode H.265/HEVC to H.264/AAC, cut clips, and join clips.
- Queue/history with cancel, delete-without-file-removal, retry, scheduled Live recordings, disk-space checks, progress, and ETA.
- Bilingual `วิธีใช้ / HELP` popup in Thai and English.
- GUI and CLI modes using Python standard library and Tkinter.

## Quick start

### Linux / WSL

Run this after replacing nothing—the repository is already configured:

```bash
curl -fsSL https://raw.githubusercontent.com/PeakNAtanon/PixClip/main/install.sh | bash -s -- PeakNAtanon/PixClip
export PATH="$HOME/.local/bin:$PATH"
PixClip
```

The installer downloads PixClip, installs `python3-tk` on apt-based Linux when needed, installs yt-dlp and Streamlink in PixClip's private Python environment, validates FFmpeg/ffprobe, and creates the `PixClip` command. FFmpeg/ffprobe are not installed automatically on Linux.

### Windows

1. Download the repository with **Code → Download ZIP**, or clone it with Git.
2. Extract it, then double-click [`Install-PixClip.bat`](Install-PixClip.bat).
3. The installer checks Python, can install it with `winget`, installs the media tools, creates the PixClip shortcut, and opens the GUI.

For a manual launch, use [`OPEN-GUI-CMD.bat`](OPEN-GUI-CMD.bat) or [`PixClip.bat`](PixClip.bat).

## Using `cookies.txt`

Some Live pages, including TikTok Live, may require the same logged-in browser session that can play the stream. PixClip accepts a Netscape-format `cookies.txt` export:

1. Sign in to the website in your browser.
2. Export the website cookies as a Netscape `cookies.txt` file using a trusted browser cookie export tool.
3. Open PixClip and click `Choose` beside `Cookies: OFF`.
4. Select the exported file, choose a Live mode, paste the URL, and click `DOWNLOAD`.
5. Click `Clear` when you want PixClip to return to public access.

PixClip remembers only the file path and does not print cookie values in the log. Treat the file like a password: never share it or commit it to Git. The repository ignores common cookie export filenames automatically.

## Manual commands

```bash
# GUI
python3 media_toolkit.py

# Terminal mode after install.sh
PixClip --cli

# Terminal mode from the source folder
python3 media_toolkit.py --cli

# Environment report
python3 media_toolkit.py --compatibility-test

# Live recording with an exported Netscape cookies.txt file
python3 media_toolkit.py --record-live-streamlink "LIVE_URL" "$HOME/Videos" --cookies-file "$HOME/private/cookies.txt"
```

Linux desktop GUI requires a working desktop/WSLg session and Tkinter. Headless Linux should use `--cli`.
The GUI's `Choose` button beside `Cookies: OFF` remembers only the file path. Never commit or share the cookies file.

## Organized output folders

New queued downloads and direct CLI downloads are grouped under the selected folder by type and date:

```text
Videos/ or Live/ or Playlists/ or Audio/
  YYYY-MM-DD/
    HHMMSS - source [job-id]/   # Queue / History
    HHMMSS - source/            # direct CLI
```

`Open folder` opens the job's exact folder. Existing history without the new folder field continues to use the legacy `PixClip-<job-id>` layout. Convert, cut, and join outputs remain at the path you choose.

## Linux distribution support

PixClip is Python/Tkinter based and is designed to run on most mainstream Linux distributions. The easiest path is an apt-based distribution:

| Distribution | GUI | What must be installed manually |
| --- | --- | --- |
| Ubuntu, Debian, Linux Mint, Pop!_OS | Yes | `python3-venv`, `ffmpeg`, `curl`, and `tar`; `python3-tk` and `python3-venv` are installed automatically by PixClip when possible |
| Fedora | Yes | `python3`, `python3-pip`, `python3-tkinter`, `ffmpeg`, `curl`, and `tar` |
| Arch Linux, Manjaro | Yes | `python`, `python-pip`, `tk`, `ffmpeg`, `curl`, and `tar` |
| WSL2 + WSLg | Yes, with WSLg | Use the Ubuntu/Debian instructions; without WSLg run `PixClip --cli` |

For Ubuntu/Debian-family systems, the recommended preparation is:

```bash
sudo apt update
sudo apt install -y python3 python3-venv python3-tk ffmpeg curl tar
```

For Fedora:

```bash
sudo dnf install -y python3 python3-pip python3-tkinter curl tar
```

Install `ffmpeg` from an enabled Fedora repository (some installations require RPM Fusion), then confirm both `ffmpeg -version` and `ffprobe -version` work. For Arch/Manjaro:

```bash
sudo pacman -S --needed python python-pip tk ffmpeg curl tar
```

The installer downloads and verifies yt-dlp and installs Streamlink inside PixClip's private Python virtual environment, avoiding PEP 668 conflicts with system Python. On non-apt distributions, Tkinter, Python venv support, and FFmpeg must be prepared manually before running `install.sh`. NVIDIA/AMD hardware encoding additionally depends on the correct graphics driver and an FFmpeg build containing the matching encoder; CPU encoding remains available as a fallback.

## Development and tests

```bash
python3 -m py_compile media_toolkit.py pixclip_jobs.py pixclip_ui.py
python3 -m unittest -v test_pixclip_jobs
```

Integration tests use synthetic local media and are opt-in:

```bash
PIXCLIP_INTEGRATION=1 python3 -m unittest -v test_pixclip_integration
```

See [`README-PixClip.md`](README-PixClip.md) for the detailed Thai user guide and [`AGENTS.md`](AGENTS.md) for contributor instructions.

## Support PixClip

If PixClip is useful to you, you can support its continued development on Ko-fi:

[☕ Support PixClip on Ko-fi](https://ko-fi.com/peaknatanon)

## License

PixClip is released under the [MIT License](LICENSE).

yt-dlp, FFmpeg, ffprobe, Streamlink, and other external tools retain their own licenses and terms.

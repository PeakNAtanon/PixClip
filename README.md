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
- Detect NVIDIA/AMD hardware and choose NVENC, AMF, VA-API, or CPU encoding automatically.
- Convert TS to MP4, transcode H.265/HEVC to H.264/AAC, cut clips, and join clips.
- Queue, history, scheduled Live recordings, disk-space checks, progress, and ETA.
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

The installer downloads PixClip, installs `python3-tk` on apt-based Linux when needed, installs or verifies media tools, and creates the `PixClip` command.

### Windows

1. Download the repository with **Code → Download ZIP**, or clone it with Git.
2. Extract it, then double-click [`Install-PixClip.bat`](Install-PixClip.bat).
3. The installer checks Python, can install it with `winget`, installs the media tools, creates the PixClip shortcut, and opens the GUI.

For a manual launch, use [`OPEN-GUI-CMD.bat`](OPEN-GUI-CMD.bat) or [`PixClip.bat`](PixClip.bat).

## Manual commands

```bash
# GUI
python3 media_toolkit.py

# Terminal mode
python3 media_toolkit.py --cli

# Environment report
python3 media_toolkit.py --compatibility-test
```

Linux desktop GUI requires a working desktop/WSLg session and Tkinter. Headless Linux should use `--cli`.

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

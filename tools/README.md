# Local media tools

PixClip installs verified standalone yt-dlp, FFmpeg, and ffprobe here.
Streamlink can also be placed here or in `streamlink/bin/`. When missing,
PixClip installs Streamlink's official portable Windows ZIP or Linux AppImage,
verified against the SHA-256 digest in the official GitHub release metadata.
Linux still requires FFmpeg/ffprobe from the distribution's package manager.

These executables are local dependencies and are excluded from Git. PixClip
also accepts legacy executables in the project root, existing Streamlink
virtual environments, and tools on `PATH`. The Rust app does not launch Python.

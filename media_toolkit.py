#!/usr/bin/env python3
"""PixClip.

Python replacement for the original PowerShell GUI and batch menu.

The application intentionally uses only the Python standard library.  It
expects yt-dlp, FFmpeg, and Streamlink either beside this file or somewhere
on PATH.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import re
import shlex
import site
import shutil
import stat
import subprocess
import sysconfig
import sys
import tempfile
import threading
import time
import urllib.error
from urllib.parse import urlparse
import urllib.request
import zipfile
from pathlib import Path
from queue import Empty, Queue
from typing import Callable, Dict, Iterable, List, Optional, Sequence, Tuple

from pixclip_ui import WorkflowUI, ClipPreview
from pixclip_jobs import load_settings, require_space, save_settings, stop_tree


BASE_DIR = Path(__file__).resolve().parent
SCRIPT_PATH = Path(__file__).resolve()
DEFAULT_OUTPUT_DIR = Path.home() / "Downloads" / "YT-DLP"

FFMPEG_URL = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip"
FFMPEG_CHECKSUM_URL = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip.sha256"

IS_WINDOWS = os.name == "nt"
YT_DLP_ASSET_NAME = "yt-dlp.exe" if IS_WINDOWS else "yt-dlp"
YT_DLP_INSTALL_NAME = "yt-dlp.exe" if IS_WINDOWS else "yt-dlp"
YT_DLP_URL = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/" + YT_DLP_ASSET_NAME
YT_DLP_CHECKSUM_URL = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS"
CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)
CREATE_NEW_CONSOLE = getattr(subprocess, "CREATE_NEW_CONSOLE", 0)
CREATE_NEW_PROCESS_GROUP = getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
WINDOWS_APP_USER_MODEL_ID = "PixClip.UniversalMediaToolkit"

TS_DOWNLOAD_MODE = "Video/Live - MPEG-TS (.ts)"
AUTO_LIVE_TS_MODE = "Live - MPEG-TS (.ts) - Auto GPU/CPU"
LIVE_TS_NVENC_MODE = "Live - MPEG-TS (.ts) - NVIDIA NVENC"
STREAMLINK_LIVE_MODE = "Live - Streamlink - Original quality (.ts)"

DOWNLOAD_MODES = [
    "MP4 - Best quality",
    "MP4 - Up to 4K",
    "MP4 - Compatible (H.264/AAC)",
    "MP3 - Best quality",
    "Playlist - MP4 best quality",
    "Playlist - MP4 compatible (H.264/AAC)",
    "Playlist - MP3 best quality",
    TS_DOWNLOAD_MODE,
    AUTO_LIVE_TS_MODE,
    LIVE_TS_NVENC_MODE,
    STREAMLINK_LIVE_MODE,
]

CONVERT_MODES = [
    "TS to MP4 - Fast (original quality)",
    "TS to MP4 - Compatible H.264",
    "TS to MP4 - H.265 NVIDIA",
    "H.265/HEVC to H.264/AAC",
]

JOIN_MODES = [
    "Fast - no re-encoding (same format only)",
    "Compatible MP4 - re-encode H.264/AAC",
]

CLIP_MODES = [
    "Fast - original quality (keyframe cut)",
    "Compatible MP4 - accurate H.264/AAC",
]

DOWNLOAD_PROGRESS_RE = re.compile(r"\[download\]\s+(\d+(?:\.\d+)?)%")
MEDIA_DURATION_RE = re.compile(r"^media_duration=(\d+(?:\.\d+)?)$")

KICK_USER_AGENT = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
    "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36"
)
KICK_HLS_HEADERS = "Origin: https://kick.com\r\nReferer: https://kick.com/\r\n"
KICK_VOD_URL_RE = re.compile(
    r"^https?://(?:www\.)?kick\.com/[^/?#]+/videos/"
    r"(?P<video_id>[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12})"
    r"(?:[/?#]|$)",
    re.IGNORECASE,
)


def configure_output_encoding() -> None:
    """Keep Windows CLI/child-process output safe for Unicode titles and paths."""

    for stream in (getattr(sys, "stdout", None), getattr(sys, "stderr", None)):
        if stream is None or not hasattr(stream, "reconfigure"):
            continue
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except (OSError, ValueError):
            pass


def configure_windows_app_identity() -> None:
    """Give the Python process PixClip's identity for the Windows taskbar."""

    if not IS_WINDOWS:
        return
    try:
        import ctypes

        set_app_id = ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID
        set_app_id.argtypes = [ctypes.c_wchar_p]
        set_app_id.restype = ctypes.c_long
        set_app_id(WINDOWS_APP_USER_MODEL_ID)
    except (AttributeError, OSError):
        # Keep PixClip usable on unusual Windows/Python environments where
        # the optional shell identity API is unavailable.
        pass


def apply_windows_window_icon(window, icon_path: Path) -> bool:
    """Set PixClip's ICO on the native Windows window handle."""

    if not IS_WINDOWS or not icon_path.is_file():
        return False
    try:
        import ctypes

        user32 = ctypes.windll.user32
        user32.LoadImageW.argtypes = [
            ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_uint,
            ctypes.c_int, ctypes.c_int, ctypes.c_uint,
        ]
        user32.LoadImageW.restype = ctypes.c_void_p
        user32.SendMessageW.argtypes = [
            ctypes.c_void_p, ctypes.c_uint, ctypes.c_void_p, ctypes.c_void_p,
        ]
        user32.SendMessageW.restype = ctypes.c_void_p
        image_icon = 1
        load_from_file = 0x10
        load_default_size = 0x40
        icon_handle = user32.LoadImageW(
            None, str(icon_path), image_icon, 0, 0, load_from_file | load_default_size
        )
        if not icon_handle:
            return False
        hwnd = ctypes.c_void_p(int(window.winfo_id()))
        wm_seticon = 0x0080
        icon_small = 0
        icon_big = 1
        user32.SendMessageW(hwnd, wm_seticon, icon_big, icon_handle)
        user32.SendMessageW(hwnd, wm_seticon, icon_small, icon_handle)
        return True
    except (AttributeError, OSError, TypeError, ValueError):
        # Tk's iconbitmap/iconphoto fallback still handles the window when
        # native Win32 icon APIs are unavailable.
        return False


def parse_download_progress(text: str) -> Optional[float]:
    match = DOWNLOAD_PROGRESS_RE.search(text)
    if not match:
        return None
    return max(0.0, min(100.0, float(match.group(1))))


def parse_ffmpeg_progress_seconds(text: str) -> Optional[float]:
    line = text.strip()
    if line.startswith("out_time_ms=") or line.startswith("out_time_us="):
        try:
            return max(0.0, float(line.split("=", 1)[1]) / 1_000_000.0)
        except ValueError:
            return None
    if line.startswith("out_time="):
        value = line.split("=", 1)[1]
        try:
            hours, minutes, seconds = value.split(":")
            return max(0.0, (float(hours) * 3600.0) + (float(minutes) * 60.0) + float(seconds))
        except (ValueError, AttributeError):
            return None
    return None


def parse_media_duration(text: str) -> Optional[float]:
    match = MEDIA_DURATION_RE.match(text.strip())
    if not match:
        return None
    try:
        return max(0.0, float(match.group(1)))
    except ValueError:
        return None


def format_eta(seconds: float) -> str:
    """Format an estimated remaining time as HH:MM:SS or MM:SS."""

    remaining = max(0, int(round(seconds)))
    hours, remainder = divmod(remaining, 3600)
    minutes, seconds_part = divmod(remainder, 60)
    if hours:
        return f"{hours:02d}:{minutes:02d}:{seconds_part:02d}"
    return f"{minutes:02d}:{seconds_part:02d}"


def parse_timecode(value: str) -> Optional[float]:
    """Parse seconds or HH:MM:SS(.mmm) into a non-negative duration."""

    text = value.strip()
    if not text:
        return None
    try:
        parts = text.split(":")
        if len(parts) == 1:
            seconds = float(parts[0])
        elif len(parts) == 2:
            minutes = float(parts[0])
            seconds = (minutes * 60.0) + float(parts[1])
        elif len(parts) == 3:
            hours = float(parts[0])
            minutes = float(parts[1])
            seconds = (hours * 3600.0) + (minutes * 60.0) + float(parts[2])
        else:
            return None
    except ValueError:
        return None
    if seconds < 0 or not math.isfinite(seconds):
        return None
    return seconds


def format_ffmpeg_time(seconds: float) -> str:
    """Format seconds in a stable FFmpeg-compatible time format."""

    hours, remainder = divmod(max(0.0, seconds), 3600.0)
    minutes, seconds_part = divmod(remainder, 60.0)
    return f"{int(hours):02d}:{int(minutes):02d}:{seconds_part:06.3f}"


def format_clip_filename_time(seconds: float) -> str:
    """Format a time value without filename-invalid characters."""

    total_seconds = max(0, int(round(seconds)))
    hours, remainder = divmod(total_seconds, 3600)
    minutes, seconds_part = divmod(remainder, 60)
    return f"{hours:02d}h{minutes:02d}m{seconds_part:02d}s"


def extract_kick_vod_id(url: str) -> Optional[str]:
    """Return the UUID from a Kick VOD URL, if the URL is a VOD page."""

    match = KICK_VOD_URL_RE.match(url.strip())
    return match.group("video_id") if match else None


def safe_filename_component(value: str, fallback: str) -> str:
    """Make a title safe for both Windows and Unix filenames."""

    cleaned = re.sub(r'[<>:"/\\|?*\x00-\x1f]', "_", value)
    cleaned = re.sub(r"\s+", " ", cleaned).strip().rstrip(" .")
    return (cleaned[:160] or fallback).strip()


def request_kick_vod_playback(video_id: str) -> Optional[Dict[str, object]]:
    """Request the current Kick VOD playback URL without the old yt-dlp extractor."""

    endpoint = f"https://web.kick.com/api/v1/stream/{video_id}/playback"
    body = {
        "video_player": {"player": {}},
        "video_session": {},
        "user_session": {"non_personalised_ads": True},
    }
    request = urllib.request.Request(
        endpoint,
        data=json.dumps(body).encode("utf-8"),
        headers={
            "Accept": "application/json, text/plain, */*",
            "Content-Type": "application/json",
            "Origin": "https://kick.com",
            "Referer": "https://kick.com/",
            "User-Agent": KICK_USER_AGENT,
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            payload = json.load(response)
    except urllib.error.HTTPError as exc:
        print(f"ERROR: Kick playback API returned HTTP {exc.code}.", flush=True)
        return None
    except (urllib.error.URLError, OSError, TimeoutError, ValueError):
        print("ERROR: Could not contact the Kick playback API.", flush=True)
        return None
    if not isinstance(payload, dict):
        print("ERROR: Kick playback API returned an invalid response.", flush=True)
        return None
    return payload


def python_executable() -> str:
    """Return a console-capable Python executable for child processes."""

    executable = Path(sys.executable)
    if IS_WINDOWS and executable.name.lower() in {"pythonw.exe", "pyw.exe"}:
        console_executable = executable.with_name("python.exe")
        if console_executable.exists():
            return str(console_executable)
    return str(executable)


def tool_candidates(name: str) -> List[str]:
    """Return platform-neutral executable names for a requested tool."""

    raw_name = Path(name).name
    stem = raw_name[:-4] if raw_name.lower().endswith(".exe") else raw_name
    candidates = [raw_name, stem]
    if IS_WINDOWS:
        candidates.insert(0, stem + ".exe")
    return list(dict.fromkeys(candidates))

def display_tool_name(name: str) -> str:
    """Return a platform-neutral display name for a media executable."""

    raw_name = Path(name).name
    return raw_name[:-4] if raw_name.lower().endswith(".exe") else raw_name

def find_tool(name: str) -> Optional[str]:
    """Find a media tool beside the application first, then on PATH."""

    for candidate in tool_candidates(name):
        local_path = BASE_DIR / candidate
        if local_path.is_file():
            return str(local_path)
    for candidate in tool_candidates(name):
        found = shutil.which(candidate)
        if found:
            return found
    return None


def find_streamlink() -> Optional[str]:
    """Find Streamlink, including Python user-install script directories."""

    direct = find_tool("streamlink.exe")
    if direct:
        return direct

    directories: List[Path] = []
    try:
        scripts_directory = sysconfig.get_path("scripts")
        if scripts_directory:
            directories.append(Path(scripts_directory))
    except (KeyError, OSError, TypeError, ValueError):
        pass
    try:
        user_base = Path(site.getuserbase())
        directories.append(user_base / ("Scripts" if IS_WINDOWS else "bin"))
    except (OSError, TypeError, ValueError):
        pass

    for directory in directories:
        for candidate in tool_candidates("streamlink.exe"):
            path = directory / candidate
            if path.is_file():
                return str(path)
    return None


def command_text(executable: str, arguments: Sequence[str]) -> str:
    """Format a command for the log without executing shell interpolation."""

    parts = [executable, *[str(item) for item in arguments]]
    if IS_WINDOWS:
        return subprocess.list2cmdline(parts)
    return shlex.join(parts)


def open_folder(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)
    if IS_WINDOWS:
        os.startfile(str(path))  # type: ignore[attr-defined]
    elif sys.platform == "darwin":
        subprocess.Popen(["open", str(path)])
    else:
        subprocess.Popen(["xdg-open", str(path)])


def current_windows_name() -> str:
    if not IS_WINDOWS:
        return platform.system()
    try:
        build = sys.getwindowsversion().build
    except AttributeError:
        return "Windows"
    return "Windows 11" if build >= 22000 else "Windows 10"



def _run_hardware_query(arguments: Sequence[str], timeout: float = 5) -> str:
    """Run a short, read-only hardware query without opening a console window."""

    try:
        result = subprocess.run(
            [str(argument) for argument in arguments],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=timeout,
            creationflags=CREATE_NO_WINDOW,
        )
    except (OSError, subprocess.SubprocessError):
        return ""
    return result.stdout + result.stderr


def _clean_hardware_name(value: str) -> str:
    return re.sub(r"\s+", " ", value).strip(" \t\r\n")


def _usable_gpu_name(value: str) -> bool:
    lowered = value.lower()
    return bool(value) and not any(
        marker in lowered
        for marker in ("microsoft basic display", "llvmpipe", "software rasterizer", "virtual display")
    )


def classify_gpu_name(name: str) -> str:
    """Classify a GPU model into the vendors supported by auto recording."""

    lowered = name.lower()
    if "nvidia" in lowered or "geforce" in lowered or "quadro" in lowered or "tesla" in lowered:
        return "NVIDIA"
    if "amd" in lowered or "ati" in lowered or "radeon" in lowered:
        return "AMD"
    if "intel" in lowered or "arc" in lowered or "uhd graphics" in lowered:
        return "Intel"
    if "apple" in lowered or "m1" in lowered or "m2" in lowered or "m3" in lowered or "m4" in lowered:
        return "Apple"
    return "Unknown"


def _windows_gpu_names() -> List[str]:
    powershell = shutil.which("powershell.exe") or shutil.which("powershell")
    if not powershell:
        return []
    output = _run_hardware_query(
        [
            powershell,
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty Name",
        ]
    )
    return [
        name
        for name in (_clean_hardware_name(line) for line in output.splitlines())
        if _usable_gpu_name(name)
    ]


def _linux_gpu_names() -> List[str]:
    names: List[str] = []
    lspci = shutil.which("lspci")
    if lspci:
        output = _run_hardware_query([lspci, "-nn"])
        for line in output.splitlines():
            if not re.search(r"(?:VGA compatible controller|3D controller|Display controller)", line, re.I):
                continue
            name = _clean_hardware_name(line.split(":", 2)[-1])
            if _usable_gpu_name(name):
                names.append(name)

    # lspci is optional. PCI vendor IDs still let a minimal Linux install
    # choose the correct encoder and show a useful vendor label.
    vendor_ids = {"0x10de": "NVIDIA", "0x1002": "AMD", "0x8086": "Intel"}
    for device in sorted(Path("/sys/class/drm").glob("card[0-9]")):
        try:
            vendor_id = (device / "device" / "vendor").read_text(encoding="ascii").strip().lower()
        except (OSError, UnicodeError):
            continue
        vendor = vendor_ids.get(vendor_id)
        if vendor and not any(classify_gpu_name(item) == vendor for item in names):
            names.append(vendor + " GPU")
    return names


def _mac_gpu_names() -> List[str]:
    output = _run_hardware_query(["system_profiler", "SPDisplaysDataType"])
    names = []
    for line in output.splitlines():
        if ":" not in line or not re.search(r"(?:Chipset Model|Graphics/Displays)", line, re.I):
            continue
        name = _clean_hardware_name(line.split(":", 1)[1])
        if _usable_gpu_name(name):
            names.append(name)
    return names


def detect_gpu() -> Tuple[str, str]:
    """Return (vendor, model) using only local OS hardware information."""

    if IS_WINDOWS:
        names = _windows_gpu_names()
    elif sys.platform.startswith("linux"):
        names = _linux_gpu_names()
    elif sys.platform == "darwin":
        names = _mac_gpu_names()
    else:
        names = []

    # nvidia-smi is a reliable fallback when WMI/lspci is unavailable.
    if not names:
        nvidia = shutil.which("nvidia-smi")
        if nvidia:
            output = _run_hardware_query([nvidia, "--query-gpu=name", "--format=csv,noheader,nounits"], timeout=10)
            names = [
                name
                for name in (_clean_hardware_name(line) for line in output.splitlines())
                if _usable_gpu_name(name)
            ]

    unique_names = list(dict.fromkeys(names))
    for preferred in ("NVIDIA", "AMD", "Intel", "Apple"):
        matching = [name for name in unique_names if classify_gpu_name(name) == preferred]
        if matching:
            return preferred, " / ".join(matching)
    if unique_names:
        return classify_gpu_name(unique_names[0]), " / ".join(unique_names)
    return "Unknown", "GPU not detected"


def cpu_name() -> str:
    """Return a readable local CPU model on Windows, Linux, or macOS."""

    if sys.platform.startswith("linux"):
        try:
            for line in Path("/proc/cpuinfo").read_text(encoding="utf-8", errors="replace").splitlines():
                if line.lower().startswith(("model name", "hardware")) and ":" in line:
                    value = _clean_hardware_name(line.split(":", 1)[1])
                    if value:
                        return value
        except OSError:
            pass
    elif sys.platform == "darwin":
        value = _clean_hardware_name(_run_hardware_query(["sysctl", "-n", "machdep.cpu.brand_string"]))
        if value:
            return value.splitlines()[0]
    elif IS_WINDOWS:
        powershell = shutil.which("powershell.exe") or shutil.which("powershell")
        if powershell:
            output = _run_hardware_query(
                [
                    powershell,
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Get-CimInstance Win32_Processor | Select-Object -First 1 -ExpandProperty Name",
                ]
            )
            value = next((_clean_hardware_name(line) for line in output.splitlines() if line.strip()), "")
            if value:
                return value
    value = _clean_hardware_name(platform.processor())
    return value if value and value.lower() not in {"amd64", "x86_64", "arm64"} else "Unknown CPU"


def gpu_name() -> str:
    """Return a detected GPU model for the CLI and status bar."""

    return detect_gpu()[1]


def find_vaapi_device() -> Optional[str]:
    """Return the first Linux VA-API render device, if one is available."""

    if not sys.platform.startswith("linux"):
        return None
    for candidate in sorted(Path("/dev/dri").glob("renderD*")):
        if candidate.is_char_device() or candidate.exists():
            return str(candidate)
    return None


def has_encoder(ffmpeg: str, encoder: str) -> bool:
    try:
        result = subprocess.run(
            [ffmpeg, "-hide_banner", "-encoders"],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=30,
            creationflags=CREATE_NO_WINDOW,
        )
    except (OSError, subprocess.SubprocessError):
        return False
    return re.search(r"(?<![A-Za-z0-9_])" + re.escape(encoder) + r"(?![A-Za-z0-9_])", result.stdout + result.stderr, re.I) is not None


def has_nvenc_encoder(ffmpeg: str, encoder: str) -> bool:
    return has_encoder(ffmpeg, encoder)


def select_live_encoder(ffmpeg: str, vendor: Optional[str] = None, model: Optional[str] = None) -> Dict[str, str]:
    """Select NVIDIA, AMD, or CPU encoding from local hardware and FFmpeg support."""

    if not vendor or not model:
        vendor, model = detect_gpu()
    if vendor == "NVIDIA" and has_encoder(ffmpeg, "h264_nvenc"):
        return {"encoder": "h264_nvenc", "label": "NVIDIA NVENC", "vendor": vendor, "model": model}
    if vendor == "AMD":
        if IS_WINDOWS and has_encoder(ffmpeg, "h264_amf"):
            return {"encoder": "h264_amf", "label": "AMD AMF", "vendor": vendor, "model": model}
        vaapi_device = find_vaapi_device()
        if vaapi_device and has_encoder(ffmpeg, "h264_vaapi"):
            return {
                "encoder": "h264_vaapi",
                "label": "AMD VA-API",
                "vendor": vendor,
                "model": model,
                "vaapi_device": vaapi_device,
            }
    return {"encoder": "libx264", "label": "CPU (libx264)", "vendor": vendor, "model": model}


def has_nvenc(ffmpeg: str) -> bool:
    return has_nvenc_encoder(ffmpeg, "hevc_nvenc")


def get_media_duration(input_file: Path) -> Optional[float]:
    """Read media duration in seconds using ffprobe for progress reporting."""

    ffprobe = find_tool("ffprobe.exe")
    if not ffprobe:
        return None
    try:
        result = subprocess.run(
            [
                ffprobe,
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
                str(input_file),
            ],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=60,
            creationflags=CREATE_NO_WINDOW,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if result.returncode != 0:
        return None
    for line in result.stdout.splitlines():
        try:
            duration = float(line.strip())
        except ValueError:
            continue
        if duration > 0:
            return duration
    return None


def get_total_media_duration(input_files: Sequence[Path]) -> Optional[float]:
    """Return the total duration when every input can be probed."""

    durations: List[float] = []
    for input_file in input_files:
        duration = get_media_duration(Path(input_file))
        if duration is None:
            return None
        durations.append(duration)
    return sum(durations) if durations else None


JOIN_SIGNATURE_FIELDS = (
    "codec_type",
    "codec_name",
    "width",
    "height",
    "pix_fmt",
    "sample_rate",
    "channels",
    "channel_layout",
    "r_frame_rate",
    "time_base",
)


def get_media_stream_signature(input_file: Path) -> Optional[Tuple[Tuple[str, ...], ...]]:
    """Read stream properties used to decide whether fast concat is safe."""

    ffprobe = find_tool("ffprobe.exe")
    if not ffprobe:
        return None
    try:
        result = subprocess.run(
            [
                ffprobe,
                "-v",
                "error",
                "-show_entries",
                "stream=" + ",".join(JOIN_SIGNATURE_FIELDS),
                "-of",
                "json",
                str(input_file),
            ],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=60,
            creationflags=CREATE_NO_WINDOW,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if result.returncode != 0:
        return None
    try:
        payload = json.loads(result.stdout)
    except (TypeError, ValueError):
        return None
    streams = payload.get("streams") if isinstance(payload, dict) else None
    if not isinstance(streams, list) or not streams:
        return None
    signature: List[Tuple[str, ...]] = []
    for stream in streams:
        if not isinstance(stream, dict):
            return None
        signature.append(tuple(str(stream.get(field, "")) for field in JOIN_SIGNATURE_FIELDS))
    return tuple(signature)


def fast_join_compatibility(input_files: Sequence[Path]) -> Optional[bool]:
    """Return whether all inputs have matching streams, or None if probing is unavailable."""

    signatures = [get_media_stream_signature(Path(item)) for item in input_files]
    if not signatures or any(signature is None for signature in signatures):
        return None
    first = signatures[0]
    return all(signature == first for signature in signatures[1:])


def build_download_arguments(mode: str, url: str, output_dir: Path) -> List[str]:
    """Build yt-dlp arguments equivalent to the original GUI/batch modes."""

    common = [
        "--ignore-config",
        "--newline",
        "--continue",
        "--no-overwrites",
        "--windows-filenames",
        "--retries",
        "10",
        "--fragment-retries",
        "10",
        "--concurrent-fragments",
        "4",
        "-P",
        str(output_dir),
        "--embed-metadata",
    ]

    single_name = "%(title)s [%(height)sp] [%(id)s].%(ext)s"
    playlist_name = "%(playlist_title)s/%(playlist_index)03d - %(title)s [%(id)s].%(ext)s"

    if mode == "MP4 - Best quality":
        extra = [
            "--no-playlist",
            "-o",
            single_name,
            "-f",
            "bv*+ba/b",
            "--merge-output-format",
            "mp4",
            "--remux-video",
            "mp4",
        ]
    elif mode == "MP4 - Up to 4K":
        extra = [
            "--no-playlist",
            "-o",
            single_name,
            "-f",
            "bv*[height<=2160]+ba/b[height<=2160]",
            "--merge-output-format",
            "mp4",
            "--remux-video",
            "mp4",
        ]
    elif mode == "MP4 - Compatible (H.264/AAC)":
        extra = [
            "--no-playlist",
            "-o",
            single_name,
            "-f",
            "bv*+ba/b",
            "-S",
            "vcodec:h264,res,acodec:aac",
            "--merge-output-format",
            "mp4",
            "--remux-video",
            "mp4",
        ]
    elif mode == "MP3 - Best quality":
        extra = [
            "--no-playlist",
            "-o",
            "%(title)s [%(id)s].%(ext)s",
            "-f",
            "ba/b",
            "-x",
            "--audio-format",
            "mp3",
            "--audio-quality",
            "0",
            "--embed-thumbnail",
            "--convert-thumbnails",
            "jpg",
        ]
    elif mode == TS_DOWNLOAD_MODE:
        extra = [
            "--no-playlist",
            "-o",
            single_name,
            "-f",
            "bv*+ba/b",
            "-S",
            "vcodec:h264,res,acodec:aac",
            "--merge-output-format",
            "mp4",
            "--remux-video",
            "mpegts",
        ]
    elif mode in {
        "Playlist - MP4 best quality",
        "Playlist - MP4 compatible (H.264/AAC)",
    }:
        archive_name = (
            "playlist-mp4-compatible.txt"
            if "compatible" in mode.lower()
            else "playlist-mp4.txt"
        )
        extra = [
            "--yes-playlist",
            "--download-archive",
            str(output_dir / "_archives" / archive_name),
            "-o",
            playlist_name,
            "-f",
            "bv*+ba/b",
        ]
        if "compatible" in mode.lower():
            extra.extend(["-S", "vcodec:h264,res,acodec:aac"])
        extra.extend(["--merge-output-format", "mp4", "--remux-video", "mp4"])
    elif mode == "Playlist - MP3 best quality":
        extra = [
            "--yes-playlist",
            "--download-archive",
            str(output_dir / "_archives" / "playlist-mp3.txt"),
            "-o",
            playlist_name,
            "-f",
            "ba/b",
            "-x",
            "--audio-format",
            "mp3",
            "--audio-quality",
            "0",
            "--embed-thumbnail",
            "--convert-thumbnails",
            "jpg",
        ]
    else:
        raise ValueError("Unknown download mode: " + mode)

    return common + extra + ["--", url]


def build_convert_arguments(input_file: Path, mode: str, progress: bool = False) -> Tuple[Path, List[str]]:
    name = input_file.stem
    directory = input_file.parent

    if mode == CONVERT_MODES[0]:
        output_file = directory / (name + ".mp4")
        arguments = [
            "-hide_banner",
            "-y",
            "-i",
            str(input_file),
            "-map",
            "0:v?",
            "-map",
            "0:a?",
            "-map_metadata",
            "0",
            "-c",
            "copy",
            "-movflags",
            "+faststart",
            str(output_file),
        ]
    elif mode == CONVERT_MODES[1]:
        output_file = directory / (name + "_converted.mp4")
        arguments = [
            "-hide_banner",
            "-y",
            "-i",
            str(input_file),
            "-map",
            "0:v:0?",
            "-map",
            "0:a:0?",
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "20",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-ar",
            "48000",
            "-movflags",
            "+faststart",
            str(output_file),
        ]
    elif mode == CONVERT_MODES[2]:
        output_file = directory / (name + "_H265_NVENC.mp4")
        arguments = [
            "-hide_banner",
            "-y",
            "-i",
            str(input_file),
            "-map",
            "0:v:0?",
            "-map",
            "0:a:0?",
            "-map_metadata",
            "0",
            "-c:v",
            "hevc_nvenc",
            "-preset",
            "p6",
            "-tune",
            "hq",
            "-rc",
            "vbr",
            "-cq",
            "22",
            "-b:v",
            "0",
            "-spatial-aq",
            "1",
            "-aq-strength",
            "8",
            "-pix_fmt",
            "yuv420p",
            "-tag:v",
            "hvc1",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-ar",
            "48000",
            "-movflags",
            "+faststart",
            str(output_file),
        ]
    elif mode == CONVERT_MODES[3]:
        output_file = directory / (name + "_H264.mp4")
        arguments = [
            "-hide_banner",
            "-y",
            "-i",
            str(input_file),
            "-map",
            "0:v:0?",
            "-map",
            "0:a:0?",
            "-map_metadata",
            "0",
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "20",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-ar",
            "48000",
            "-movflags",
            "+faststart",
            str(output_file),
        ]
    else:
        raise ValueError("Unknown conversion mode: " + mode)

    if progress:
        arguments[-1:-1] = ["-progress", "pipe:1", "-nostats"]
    return output_file, arguments


def build_clip_arguments(
    input_file: Path,
    output_file: Path,
    start_seconds: float,
    end_seconds: float,
    mode: str,
    progress: bool = False,
) -> List[str]:
    """Build FFmpeg arguments for extracting one segment from a video."""

    if start_seconds < 0 or end_seconds <= start_seconds:
        raise ValueError("End time must be greater than start time.")
    duration = end_seconds - start_seconds
    if mode == CLIP_MODES[0]:
        arguments = [
            "-hide_banner",
            "-y",
            "-ss",
            format_ffmpeg_time(start_seconds),
            "-i",
            str(input_file),
            "-t",
            format_ffmpeg_time(duration),
            "-map",
            "0:v:0?",
            "-map",
            "0:a:0?",
            "-map_metadata",
            "0",
            "-c",
            "copy",
            "-avoid_negative_ts",
            "make_zero",
            "-movflags",
            "+faststart",
        ]
    elif mode == CLIP_MODES[1]:
        arguments = [
            "-hide_banner",
            "-y",
            "-i",
            str(input_file),
            "-ss",
            format_ffmpeg_time(start_seconds),
            "-t",
            format_ffmpeg_time(duration),
            "-map",
            "0:v:0?",
            "-map",
            "0:a:0?",
            "-map_metadata",
            "0",
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "20",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-ar",
            "48000",
            "-movflags",
            "+faststart",
        ]
    else:
        raise ValueError("Unknown clip mode: " + mode)

    if progress:
        arguments.extend(["-progress", "pipe:1", "-nostats"])
    arguments.append(str(output_file))
    return arguments


def concat_file_line(path: Path) -> str:
    """Encode one path for FFmpeg's concat demuxer list format."""

    normalized = path.resolve().as_posix().replace("'", "'\\''")
    return "file '" + normalized + "'"


def validate_join_inputs(input_files: Sequence[Path], output_file: Path) -> List[Path]:
    """Validate and normalize join paths before FFmpeg can overwrite anything."""

    if len(input_files) < 2:
        raise ValueError("Please add at least two video clips.")
    normalized: List[Path] = []
    seen = set()
    for item in input_files:
        path = Path(item).expanduser()
        if not path.is_file():
            raise ValueError(f"Input clip was not found: {path}")
        resolved = path.resolve()
        if resolved in seen:
            raise ValueError(f"The same clip was added more than once: {path.name}")
        seen.add(resolved)
        normalized.append(path)

    output = Path(output_file).expanduser()
    if output.exists() and output.is_dir():
        raise ValueError("The output path is a folder. Choose an MP4 file.")
    if output.suffix.lower() != ".mp4":
        raise ValueError("Join clips always creates an MP4 file. Use a .mp4 output name.")
    if output.resolve() in seen:
        raise ValueError("The output file must be different from every input clip.")
    return normalized


def build_join_arguments(
    list_file: Path,
    output_file: Path,
    mode: str,
    progress: bool = False,
) -> List[str]:
    arguments = [
        "-hide_banner",
        "-y",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
        str(list_file),
    ]
    if mode == JOIN_MODES[0]:
        arguments.extend(
            [
                "-c",
                "copy",
                "-map_metadata",
                "0",
                "-avoid_negative_ts",
                "make_zero",
                "-movflags",
                "+faststart",
            ]
        )
    elif mode == JOIN_MODES[1]:
        arguments.extend(
            [
                "-map",
                "0:v:0?",
                "-map",
                "0:a:0?",
                "-map_metadata",
                "0",
                "-c:v",
                "libx264",
                "-preset",
                "medium",
                "-crf",
                "20",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                "-ar",
                "48000",
                "-avoid_negative_ts",
                "make_zero",
                "-movflags",
                "+faststart",
            ]
        )
    else:
        raise ValueError("Unknown join mode: " + mode)
    if progress:
        arguments.extend(["-progress", "pipe:1", "-nostats"])
    arguments.append(str(output_file))
    return arguments



def build_live_ts_arguments(
    stream_url: str,
    output_file: Path,
    encoder: str,
    progress: bool = False,
    vaapi_device: Optional[str] = None,
) -> List[str]:
    """Build a resilient ongoing Live capture command for a selected encoder."""

    if encoder not in {"h264_nvenc", "h264_amf", "h264_vaapi", "libx264"}:
        raise ValueError("Unsupported Live encoder: " + encoder)
    arguments = ["-hide_banner", "-nostdin", "-n"]
    if encoder == "h264_vaapi":
        if not vaapi_device:
            raise ValueError("AMD VA-API device was not found.")
        arguments.extend(["-vaapi_device", vaapi_device])
    arguments.extend(
        [
            "-i",
            stream_url,
            "-map",
            "0:v:0?",
            "-map",
            "0:a:0?",
        ]
    )
    if encoder == "h264_nvenc":
        arguments.extend(
            [
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p5",
                "-tune",
                "ll",
                "-rc",
                "vbr",
                "-cq",
                "23",
                "-b:v",
                "0",
                "-pix_fmt",
                "yuv420p",
            ]
        )
    elif encoder == "h264_amf":
        arguments.extend(["-c:v", "h264_amf", "-pix_fmt", "yuv420p"])
    elif encoder == "h264_vaapi":
        arguments.extend(["-vf", "format=nv12,hwupload", "-c:v", "h264_vaapi", "-qp", "23"])
    else:
        arguments.extend(["-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-pix_fmt", "yuv420p"])
    arguments.extend(["-c:a", "aac", "-b:a", "160k", "-ar", "48000"])
    if progress:
        arguments.extend(["-progress", "pipe:1", "-nostats"])
    arguments.extend(["-f", "mpegts", str(output_file)])
    return arguments


def build_live_ts_nvenc_arguments(
    stream_url: str,
    output_file: Path,
    progress: bool = False,
) -> List[str]:
    """Build an ongoing Live capture command using NVIDIA H.264 NVENC."""

    return build_live_ts_arguments(stream_url, output_file, "h264_nvenc", progress=progress)


def build_streamlink_arguments(
    url: str,
    output_file: Path,
    ffmpeg_path: Optional[str] = None,
) -> List[str]:
    """Build a resilient Streamlink command that records the best Live stream."""

    arguments = [
        "--loglevel",
        "info",
        "--progress",
        "force",
        "--retry-streams",
        "5",
        "--retry-max",
        "0",
        "--retry-open",
        "5",
        "--stream-segment-attempts",
        "5",
    ]
    if ffmpeg_path:
        arguments.extend(
            [
                "--ffmpeg-ffmpeg",
                ffmpeg_path,
                "--ffmpeg-fout",
                "mpegts",
                "--ffmpeg-video-transcode",
                "copy",
                "--ffmpeg-audio-transcode",
                "copy",
            ]
        )
    arguments.extend(["--output", str(output_file), url, "best"])
    return arguments


class ProcessRunner:
    """Run a subprocess without blocking the GUI and stream its output."""

    def __init__(
        self,
        executable: str,
        arguments: Sequence[str],
        working_directory: Path,
        on_line: Callable[[str], None],
        on_done: Callable[[int, bool], None],
        hide_window: bool = True,
    ) -> None:
        self.command = [executable, *[str(argument) for argument in arguments]]
        self.working_directory = working_directory
        self.on_line = on_line
        self.on_done = on_done
        self.hide_window = hide_window
        self._process: Optional[subprocess.Popen[str]] = None
        self._cancel_requested = threading.Event()
        self._lock = threading.Lock()
        self._thread: Optional[threading.Thread] = None

    @property
    def is_running(self) -> bool:
        with self._lock:
            return self._process is not None and self._process.poll() is None

    def start(self) -> None:
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def _run(self) -> None:
        exit_code = 1
        try:
            flags = CREATE_NEW_PROCESS_GROUP
            if self.hide_window:
                flags |= CREATE_NO_WINDOW
            process = subprocess.Popen(
                self.command,
                cwd=str(self.working_directory),
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                encoding="utf-8",
                errors="replace",
                bufsize=1,
                creationflags=flags,
                start_new_session=not IS_WINDOWS,
            )
            with self._lock:
                self._process = process
            if self._cancel_requested.is_set():
                stop_tree(process)
            assert process.stdout is not None
            for line in process.stdout:
                self.on_line(line.rstrip("\r\n"))
            exit_code = process.wait()
        except Exception as exc:  # pragma: no cover - depends on the host OS
            self.on_line("ERROR: " + str(exc))
        finally:
            with self._lock:
                self._process = None
            self.on_done(exit_code, self._cancel_requested.is_set())

    def cancel(self) -> None:
        self._cancel_requested.set()
        with self._lock:
            process = self._process
        if process is None or process.poll() is not None:
            return
        try:
            if IS_WINDOWS:
                subprocess.run(
                    ["taskkill", "/PID", str(process.pid), "/T", "/F"],
                    capture_output=True,
                    creationflags=CREATE_NO_WINDOW,
                    timeout=10,
                )
            else:
                stop_tree(process)
        except (OSError, subprocess.SubprocessError):
            try:
                process.kill()
            except OSError:
                pass


def download_file(url: str, destination: Path, display_name: str = "file") -> None:
    """Download a file while emitting parser-friendly progress for the GUI and CLI."""

    request = urllib.request.Request(url, headers={"User-Agent": "Universal-Media-Toolkit/1.0"})
    with urllib.request.urlopen(request, timeout=120) as response, destination.open("wb") as output:
        try:
            total_size = int(response.headers.get("Content-Length", "0"))
        except (TypeError, ValueError):
            total_size = 0
        downloaded = 0
        last_percent = -1
        while True:
            chunk = response.read(1024 * 1024)
            if not chunk:
                break
            output.write(chunk)
            downloaded += len(chunk)
            if total_size > 0:
                percent = min(100.0, downloaded * 100.0 / total_size)
                whole_percent = int(percent)
                if whole_percent != last_percent or percent >= 100.0:
                    print(f"[download] {percent:5.1f}% - {display_name}", flush=True)
                    last_percent = whole_percent
        if total_size <= 0:
            print(f"[download] complete - {display_name}", flush=True)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest().upper()


def expected_yt_dlp_hash(checksum_file: Path) -> str:
    text = checksum_file.read_text(encoding="utf-8", errors="replace")
    pattern = r"(?im)^([a-f0-9]{64})\s+\*?" + re.escape(YT_DLP_ASSET_NAME) + r"\s*$"
    match = re.search(pattern, text)
    if not match:
        raise RuntimeError("yt-dlp checksum entry was not found.")
    return match.group(1).upper()


def expected_ffmpeg_hash(checksum_file: Path) -> str:
    text = checksum_file.read_text(encoding="utf-8", errors="replace")
    match = re.search(r"(?i)\b([a-f0-9]{64})\b", text)
    if not match:
        raise RuntimeError("FFmpeg checksum was not found.")
    return match.group(1).upper()


def assert_file_hash(path: Path, expected: str, display_name: str) -> None:
    actual = sha256_file(path)
    if actual != expected:
        raise RuntimeError(
            f"{display_name} failed SHA-256 verification. Expected {expected} but received {actual}."
        )
    print(f"{display_name} SHA-256 verified.", flush=True)


def test_executable(path: Path, argument: str, display_name: str) -> None:
    try:
        result = subprocess.run(
            [str(path), argument],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=60,
            creationflags=CREATE_NO_WINDOW,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        raise RuntimeError(f"{display_name} validation could not start: {exc}") from exc
    if result.returncode != 0:
        raise RuntimeError(f"{display_name} validation failed with exit code {result.returncode}.")


def safe_extract_zip(archive: Path, destination: Path) -> None:
    destination = destination.resolve()
    destination.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive) as zipped:
        for item in zipped.infolist():
            target = (destination / item.filename).resolve()
            try:
                inside = os.path.commonpath([str(destination), str(target)]) == str(destination)
            except ValueError:
                inside = False
            if not inside:
                raise RuntimeError("The FFmpeg archive contains an unsafe path.")
            if item.is_dir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            with zipped.open(item) as source, target.open("wb") as output:
                shutil.copyfileobj(source, output)


def ffmpeg_install_hint() -> str:
    if sys.platform == "darwin":
        return "brew install ffmpeg"
    if shutil.which("apt-get"):
        return "sudo apt install ffmpeg"
    if shutil.which("dnf"):
        return "sudo dnf install ffmpeg"
    if shutil.which("pacman"):
        return "sudo pacman -S ffmpeg"
    return "Install FFmpeg with your distribution package manager."


def tkinter_available() -> bool:
    """Return whether this Python installation can import Tkinter."""

    try:
        import tkinter  # noqa: F401
    except ImportError:
        return False
    return True


def ensure_linux_tkinter() -> int:
    """Install the Linux Tkinter package when the GUI dependency is missing."""

    if sys.platform != "linux":
        return 0
    if tkinter_available():
        print("[2/6] Python Tkinter is ready.", flush=True)
        return 0

    apt_get = shutil.which("apt-get")
    if not apt_get:
        print("ERROR: Python Tkinter is missing.", flush=True)
        print("Install your distro's Tk package, for example: sudo apt install python3-tk", flush=True)
        return 1

    command = [apt_get]
    if os.geteuid() != 0:
        if not shutil.which("sudo"):
            print("ERROR: sudo is required to install python3-tk.", flush=True)
            print("Run: sudo apt update && sudo apt install -y python3-tk", flush=True)
            return 1
        command.insert(0, "sudo")

    print("[2/6] Python Tkinter is missing; installing python3-tk...", flush=True)
    code, _ = run_command(command[0], [*command[1:], "update"], BASE_DIR)
    if code != 0:
        print(f"ERROR: apt package index update failed with exit code {code}.", flush=True)
        return code
    code, _ = run_command(command[0], [*command[1:], "install", "-y", "python3-tk"], BASE_DIR)
    if code != 0:
        print(f"ERROR: python3-tk installation failed with exit code {code}.", flush=True)
        return code
    if not tkinter_available():
        print("ERROR: python3-tk was installed, but this Python cannot import Tkinter.", flush=True)
        return 1
    print("Python Tkinter is ready.", flush=True)
    return 0


def install_streamlink() -> int:
    """Install Streamlink for the current Python user and validate its CLI."""

    python = python_executable()
    print("Installing Streamlink with Python pip...", flush=True)
    code, _ = run_command(
        python,
        ["-m", "pip", "install", "--user", "--upgrade", "streamlink"],
        BASE_DIR,
    )
    if code != 0:
        print("The user install failed; retrying without --user...", flush=True)
        code, _ = run_command(
            python,
            ["-m", "pip", "install", "--upgrade", "streamlink"],
            BASE_DIR,
        )
    if code != 0:
        print(f"ERROR: Streamlink installation failed with exit code {code}.", flush=True)
        return code

    streamlink = find_streamlink()
    if not streamlink:
        print("ERROR: Streamlink was installed, but its executable was not found.", flush=True)
        print("Add Python's Scripts/bin directory to PATH, then restart the toolkit.", flush=True)
        return 1
    try:
        test_executable(Path(streamlink), "--version", "Streamlink")
    except RuntimeError as exc:
        print("ERROR: " + str(exc), flush=True)
        return 1
    print(f"Streamlink is ready: {streamlink}", flush=True)
    return 0


def install_tools_unix(source_check: bool = False) -> int:
    """Install yt-dlp, validate media tools, and ensure Linux Tkinter."""

    work_directory = Path(tempfile.mkdtemp(prefix="media-toolkit-install-"))
    try:
        checksum_file = work_directory / "SHA2-256SUMS"
        print("[1/6] Downloading the official yt-dlp checksum file...", flush=True)
        download_file(YT_DLP_CHECKSUM_URL, checksum_file, "yt-dlp checksums")
        expected = expected_yt_dlp_hash(checksum_file)
        if source_check:
            print("Official yt-dlp download source and checksum format are available.", flush=True)
            return 0

        if ensure_linux_tkinter() != 0:
            return 1

        download_path = work_directory / YT_DLP_ASSET_NAME
        print("[3/6] Downloading the official yt-dlp Unix release...", flush=True)
        download_file(YT_DLP_URL, download_path, "yt-dlp")
        print("[4/6] Verifying yt-dlp...", flush=True)
        assert_file_hash(download_path, expected, YT_DLP_ASSET_NAME)
        download_path.chmod(download_path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
        test_executable(download_path, "--version", YT_DLP_ASSET_NAME)

        target = BASE_DIR / YT_DLP_INSTALL_NAME
        shutil.copy2(download_path, target)
        target.chmod(target.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

        ffmpeg = find_tool("ffmpeg")
        ffprobe = find_tool("ffprobe")
        if not ffmpeg or not ffprobe:
            print("[5/6] yt-dlp is ready, but FFmpeg/ffprobe is missing.", flush=True)
            print("Install FFmpeg with:", ffmpeg_install_hint(), flush=True)
            return 1
        test_executable(Path(ffmpeg), "-version", "ffmpeg")
        test_executable(Path(ffprobe), "-version", "ffprobe")
        print("[5/6] FFmpeg and ffprobe validated.", flush=True)
        if install_streamlink() != 0:
            return 1
        print("[6/6] Streamlink validated.", flush=True)
        print("INSTALLATION COMPLETE", flush=True)
        print("yt-dlp and Streamlink are ready; FFmpeg/ffprobe were validated.", flush=True)
        return 0
    finally:
        shutil.rmtree(work_directory, ignore_errors=True)


def install_tools(source_check: bool = False) -> int:
    """Download, verify, validate, and install yt-dlp, FFmpeg, and Streamlink."""

    if not IS_WINDOWS:
        return install_tools_unix(source_check)

    work_directory = Path(tempfile.mkdtemp(prefix="media-toolkit-install-"))
    try:
        yt_checksum_file = work_directory / "SHA2-256SUMS"
        ffmpeg_checksum_file = work_directory / "ffmpeg.sha256"

        print("[1/8] Downloading official checksum files...", flush=True)
        download_file(YT_DLP_CHECKSUM_URL, yt_checksum_file, "yt-dlp checksums")
        download_file(FFMPEG_CHECKSUM_URL, ffmpeg_checksum_file, "FFmpeg checksums")
        yt_expected = expected_yt_dlp_hash(yt_checksum_file)
        ffmpeg_expected = expected_ffmpeg_hash(ffmpeg_checksum_file)

        if source_check:
            print("Official download sources and checksum formats are available.", flush=True)
            return 0

        yt_download = work_directory / "yt-dlp.exe"
        ffmpeg_archive = work_directory / "ffmpeg.zip"

        print("[2/8] Downloading yt-dlp from the official GitHub release...", flush=True)
        download_file(YT_DLP_URL, yt_download, "yt-dlp")
        print("[3/8] Verifying yt-dlp...", flush=True)
        assert_file_hash(yt_download, yt_expected, "yt-dlp.exe")

        print("[4/8] Downloading the FFmpeg essentials build...", flush=True)
        download_file(FFMPEG_URL, ffmpeg_archive, "FFmpeg archive")
        print("[5/8] Verifying FFmpeg archive...", flush=True)
        assert_file_hash(ffmpeg_archive, ffmpeg_expected, "FFmpeg archive")

        print("[6/8] Extracting and validating executables...", flush=True)
        extract_directory = work_directory / "ffmpeg"
        safe_extract_zip(ffmpeg_archive, extract_directory)
        ffmpeg_files = list(extract_directory.rglob("ffmpeg.exe"))
        ffprobe_files = list(extract_directory.rglob("ffprobe.exe"))
        if not ffmpeg_files or not ffprobe_files:
            raise RuntimeError("The verified FFmpeg archive did not contain ffmpeg.exe and ffprobe.exe.")
        ffmpeg_file = ffmpeg_files[0]
        ffprobe_file = ffprobe_files[0]

        test_executable(yt_download, "--version", "yt-dlp.exe")
        test_executable(ffmpeg_file, "-version", "ffmpeg.exe")
        test_executable(ffprobe_file, "-version", "ffprobe.exe")

        print("[7/8] Installing yt-dlp and FFmpeg beside the Media Toolkit...", flush=True)
        shutil.copy2(yt_download, BASE_DIR / "yt-dlp.exe")
        shutil.copy2(ffmpeg_file, BASE_DIR / "ffmpeg.exe")
        shutil.copy2(ffprobe_file, BASE_DIR / "ffprobe.exe")
        if install_streamlink() != 0:
            return 1
        print("[8/8] Streamlink validated.", flush=True)
        print("INSTALLATION COMPLETE", flush=True)
        print("yt-dlp.exe, ffmpeg.exe, ffprobe.exe, and Streamlink are ready.", flush=True)
        return 0
    finally:
        shutil.rmtree(work_directory, ignore_errors=True)


def run_command(
    executable: str,
    arguments: Sequence[str],
    working_directory: Path,
    *,
    capture: bool = False,
    display_arguments: Optional[Sequence[str]] = None,
) -> Tuple[int, str]:
    """Run a command for the CLI and stream output unless capture is requested."""

    shown_arguments = arguments if display_arguments is None else display_arguments
    print("> " + command_text(executable, shown_arguments), flush=True)
    if capture:
        result = subprocess.run(
            [executable, *[str(argument) for argument in arguments]],
            cwd=str(working_directory),
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            creationflags=CREATE_NO_WINDOW,
        )
        return result.returncode, result.stdout + result.stderr

    try:
        result = subprocess.run(
            [executable, *[str(argument) for argument in arguments]],
            cwd=str(working_directory),
            creationflags=0,
        )
    except OSError as exc:
        print("ERROR: " + str(exc), flush=True)
        return 1, ""
    return result.returncode, ""


def update_ytdlp() -> int:
    """Update yt-dlp and validate the executable after the updater exits."""

    ytdlp = find_tool("yt-dlp.exe")
    if not ytdlp:
        print("ERROR: yt-dlp was not found beside this file or on PATH.", flush=True)
        return 1
    print("Updating yt-dlp...", flush=True)
    code, _ = run_command(ytdlp, ["-U"], BASE_DIR)
    if code != 0:
        print(f"ERROR: yt-dlp update failed with exit code {code}.", flush=True)
        return code
    try:
        test_executable(Path(ytdlp), "--version", "yt-dlp")
    except RuntimeError as exc:
        print("ERROR: " + str(exc), flush=True)
        return 1
    print("UPDATE COMPLETE - yt-dlp was updated and verified.", flush=True)
    return 0


def print_result(exit_code: int, output_directory: Optional[Path] = None) -> None:
    print("\n" + "=" * 60)
    if exit_code == 0:
        print("DONE")
    else:
        print(f"FAILED - exit code: {exit_code}")
    if output_directory is not None:
        print("\nOutput folder:")
        print(output_directory)
    print("=" * 60)


def run_cli_download(mode: str, url: str, output_directory: Path) -> int:
    ytdlp = find_tool("yt-dlp.exe")
    ffmpeg = find_tool("ffmpeg.exe")
    if not ytdlp:
        print("ERROR: yt-dlp was not found beside this file or on PATH.")
        return 1
    if not ffmpeg:
        print("ERROR: ffmpeg was not found beside this file or on PATH.")
        return 1
    output_directory.mkdir(parents=True, exist_ok=True)
    (output_directory / "_archives").mkdir(parents=True, exist_ok=True)
    code, _ = run_command(
        ytdlp,
        build_download_arguments(mode, url, output_directory),
        BASE_DIR,
    )
    print_result(code, output_directory)
    return code


def run_cli_convert(mode: str, input_file: Path) -> int:
    ffmpeg = find_tool("ffmpeg.exe")
    if not ffmpeg:
        print("ERROR: ffmpeg was not found beside this file or on PATH.")
        return 1
    output_file, arguments = build_convert_arguments(input_file, mode)
    code, _ = run_command(ffmpeg, arguments, input_file.parent)
    print_result(code, input_file.parent)
    return code


def run_cli_join(files: Sequence[Path], mode: str, output_file: Path) -> int:
    ffmpeg = find_tool("ffmpeg.exe")
    if not ffmpeg:
        print("ERROR: ffmpeg was not found beside this file or on PATH.")
        return 1
    output_file = Path(output_file).expanduser()
    try:
        files = validate_join_inputs(files, output_file)
    except ValueError as exc:
        print("ERROR: " + str(exc))
        return 1

    list_file: Optional[Path] = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".txt", prefix="ytdlp_join_", delete=False, encoding="utf-8", newline="\n"
        ) as stream:
            list_file = Path(stream.name)
            stream.write("\n".join(concat_file_line(item) for item in files) + "\n")
        output_file.parent.mkdir(parents=True, exist_ok=True)
        code, _ = run_command(
            ffmpeg,
            build_join_arguments(list_file, output_file, mode),
            output_file.parent,
        )
        print_result(code, output_file.parent)
        return code
    finally:
        if list_file is not None:
            try:
                list_file.unlink()
            except OSError:
                pass


def run_kick_vod_ts_download(video_id: str, output_directory: Path, ffmpeg: str) -> int:
    """Download a Kick VOD through the current playback API and mux it as MPEG-TS."""

    payload = request_kick_vod_playback(video_id)
    if payload is None:
        return 1

    playback_data = payload.get("playback_url")
    video_session = payload.get("video_session")
    playback_url = playback_data.get("vod") if isinstance(playback_data, dict) else None
    title = video_session.get("video_title") if isinstance(video_session, dict) else None
    duration_value = video_session.get("video_duration") if isinstance(video_session, dict) else None
    if not isinstance(playback_url, str) or not playback_url.strip():
        print("ERROR: Kick did not return a VOD playback URL. The VOD may be private or unavailable.", flush=True)
        return 1

    title_text = title.strip() if isinstance(title, str) else "Kick VOD"
    final_name = f"{safe_filename_component(title_text, 'Kick VOD')} [{video_id}].ts"
    duration: Optional[float]
    try:
        duration = float(duration_value) if duration_value is not None else None
    except (TypeError, ValueError):
        duration = None

    output_directory.mkdir(parents=True, exist_ok=True)
    output = output_directory / final_name
    temporary_directory = Path(tempfile.mkdtemp(prefix="kick_vod_ts_"))
    temporary_output = temporary_directory / "source.ts"
    keep_temporary = True
    try:
        print("Reading Kick VOD information...", flush=True)
        print(f"Output: {output.name}", flush=True)
        if duration is not None and duration > 0:
            # The GUI uses this line with FFmpeg's -progress output to show a percentage.
            print(f"media_duration={duration}", flush=True)
        print("Downloading Kick VOD and creating MPEG-TS...", flush=True)
        arguments = [
            "-hide_banner",
            "-y",
            "-user_agent",
            KICK_USER_AGENT,
            "-headers",
            KICK_HLS_HEADERS,
            "-i",
            playback_url,
            "-map",
            "0:v:0?",
            "-map",
            "0:a:0?",
            "-c",
            "copy",
            "-progress",
            "pipe:1",
            "-nostats",
            "-f",
            "mpegts",
            str(temporary_output),
        ]
        display_arguments = [
            "-hide_banner",
            "-y",
            "-user_agent",
            KICK_USER_AGENT,
            "-headers",
            "<Kick HLS headers>",
            "-i",
            "<Kick HLS playback URL>",
            "-map",
            "0:v:0?",
            "-map",
            "0:a:0?",
            "-c",
            "copy",
            "-progress",
            "pipe:1",
            "-nostats",
            "-f",
            "mpegts",
            str(temporary_output),
        ]
        code, _ = run_command(
            ffmpeg,
            arguments,
            BASE_DIR,
            display_arguments=display_arguments,
        )
        if code != 0:
            print("ERROR: FFmpeg could not download the Kick HLS stream.", flush=True)
            return code
        if not temporary_output.is_file() or temporary_output.stat().st_size == 0:
            print("ERROR: Kick download finished without creating a valid TS file.", flush=True)
            return 1
        try:
            os.replace(temporary_output, output)
        except OSError as exc:
            print("ERROR: Could not move the downloaded TS file: " + str(exc), flush=True)
            return 1
        print(f"Created: {output}", flush=True)
        keep_temporary = False
        return 0
    finally:
        if not keep_temporary:
            shutil.rmtree(temporary_directory, ignore_errors=True)
        else:
            print(f"Partial Kick download kept here: {temporary_directory}", flush=True)


def run_cli_ts_download(url: str, output_directory: Path) -> int:
    """Download a source and create a real MPEG-TS file, matching batch mode 5."""

    ffmpeg = find_tool("ffmpeg.exe")
    if not ffmpeg:
        print("ERROR: ffmpeg was not found beside this file or on PATH.")
        return 1

    kick_vod_id = extract_kick_vod_id(url)
    if kick_vod_id:
        return run_kick_vod_ts_download(kick_vod_id, output_directory, ffmpeg)

    ytdlp = find_tool("yt-dlp.exe")
    if not ytdlp:
        print("ERROR: yt-dlp and ffmpeg are required.")
        return 1

    output_directory.mkdir(parents=True, exist_ok=True)
    temporary_directory = Path(tempfile.mkdtemp(prefix="ytdlp_ts_"))
    keep_temporary = True
    try:
        print("Reading video information...", flush=True)
        code, printed_name = run_command(
            ytdlp,
            [
                "--ignore-config",
                "--quiet",
                "--no-warnings",
                "--no-playlist",
                "--simulate",
                "--windows-filenames",
                "--print",
                "filename",
                "-o",
                "%(title)s [%(id)s].ts",
                "--",
                url,
            ],
            BASE_DIR,
            capture=True,
        )
        if code != 0:
            print(printed_name, end="" if printed_name.endswith("\n") else "\n")
            return code
        name_lines = [line.strip() for line in printed_name.splitlines() if line.strip()]
        final_name = Path(name_lines[-1]).name if name_lines else "downloaded_video.ts"
        if not final_name.lower().endswith(".ts"):
            final_name += ".ts"

        print("Downloading source video and audio...", flush=True)
        code, _ = run_command(
            ytdlp,
            [
                "--ignore-config",
                "--newline",
                "--continue",
                "--no-overwrites",
                "--windows-filenames",
                "--retries",
                "10",
                "--fragment-retries",
                "10",
                "--concurrent-fragments",
                "4",
                "--no-playlist",
                "-P",
                str(temporary_directory),
                "-o",
                "source.%(ext)s",
                "-f",
                "bv*+ba/b",
                "-S",
                "vcodec:h264,res,acodec:aac",
                "--merge-output-format",
                "mp4",
                "--",
                url,
            ],
            BASE_DIR,
        )
        if code != 0:
            print(f"Temporary files were kept here: {temporary_directory}")
            return code

        candidates = [path for path in temporary_directory.glob("source.*") if path.is_file()]
        if not candidates:
            print(f"ERROR: Downloaded source file was not found in {temporary_directory}")
            return 1
        source = candidates[0]
        output = output_directory / final_name
        print("Creating MPEG-TS without re-encoding...", flush=True)
        code, _ = run_command(
            ffmpeg,
            [
                "-hide_banner",
                "-y",
                "-i",
                str(source),
                "-map",
                "0:v:0?",
                "-map",
                "0:a:0?",
                "-map_metadata",
                "0",
                "-c",
                "copy",
                "-f",
                "mpegts",
                str(output),
            ],
            BASE_DIR,
        )
        if code != 0:
            print("Fast remux was not compatible; falling back to H.264/AAC encoding.")
            try:
                output.unlink()
            except OSError:
                pass
            code, _ = run_command(
                ffmpeg,
                [
                    "-hide_banner",
                    "-y",
                    "-i",
                    str(source),
                    "-map",
                    "0:v:0?",
                    "-map",
                    "0:a:0?",
                    "-c:v",
                    "libx264",
                    "-preset",
                    "medium",
                    "-crf",
                    "18",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:a",
                    "aac",
                    "-b:a",
                    "192k",
                    "-ar",
                    "48000",
                    "-f",
                    "mpegts",
                    str(output),
                ],
                BASE_DIR,
            )
        if code == 0:
            print(f"Created: {output}")
            keep_temporary = False
        else:
            print(f"Temporary source was kept here: {temporary_directory}")
        return code
    finally:
        if not keep_temporary:
            shutil.rmtree(temporary_directory, ignore_errors=True)



def run_cli_live_ts_nvenc(
    url: str,
    output_directory: Path,
    encoder_override: Optional[str] = None,
) -> int:
    """Capture an ongoing Live URL as MPEG-TS with NVIDIA or selected hardware."""

    ytdlp = find_tool("yt-dlp.exe")
    ffmpeg = find_tool("ffmpeg.exe")
    if not ytdlp:
        print("ERROR: yt-dlp was not found beside this file or on PATH.")
        return 1
    if not ffmpeg:
        print("ERROR: ffmpeg was not found beside this file or on PATH.")
        return 1

    if encoder_override == "auto":
        encoder_info = select_live_encoder(ffmpeg)
        print(f"Detected GPU: {encoder_info['model']} ({encoder_info['vendor']})", flush=True)
        print(f"Detected CPU: {cpu_name()}", flush=True)
        print(f"Auto recording encoder: {encoder_info['label']} [{encoder_info['encoder']}].", flush=True)
    else:
        if not has_nvenc_encoder(ffmpeg, "h264_nvenc"):
            print("ERROR: FFmpeg does not include h264_nvenc. Install an NVENC-enabled build and NVIDIA driver.")
            return 1
        encoder_info = {
            "encoder": "h264_nvenc",
            "label": "NVIDIA NVENC",
            "vendor": "NVIDIA",
            "model": gpu_name(),
        }

    output_directory.mkdir(parents=True, exist_ok=True)
    print("Reading Live stream information...", flush=True)
    title_code, title_output = run_command(
        ytdlp,
        [
            "--ignore-config",
            "--quiet",
            "--no-warnings",
            "--no-playlist",
            "--skip-download",
            "--print",
            "%(title)s [%(id)s]",
            "--",
            url,
        ],
        BASE_DIR,
        capture=True,
    )
    if title_code != 0:
        print(title_output, end="" if title_output.endswith("\n") else "\n")
        return title_code
    title_line = next((line.strip() for line in title_output.splitlines() if line.strip()), "live_stream")
    filename = safe_filename_component(title_line, "live_stream") + ".ts"
    output = output_directory / filename
    counter = 2
    while output.exists():
        output = output_directory / f"{Path(filename).stem} ({counter}).ts"
        counter += 1

    print("Resolving the direct Live stream URL...", flush=True)
    url_code, stream_output = run_command(
        ytdlp,
        [
            "--ignore-config",
            "--quiet",
            "--no-warnings",
            "--no-playlist",
            "-f",
            "best",
            "-g",
            "--",
            url,
        ],
        BASE_DIR,
        capture=True,
        display_arguments=[
            "--ignore-config",
            "--quiet",
            "--no-warnings",
            "--no-playlist",
            "-f",
            "best",
            "-g",
            "--",
            "<Live URL>",
        ],
    )
    if url_code != 0:
        print(stream_output, end="" if stream_output.endswith("\n") else "\n")
        return url_code
    stream_url = next(
        (line.strip() for line in stream_output.splitlines() if re.match(r"^https?://", line.strip())),
        "",
    )
    if not stream_url:
        print("ERROR: yt-dlp did not return a playable Live stream URL.")
        return 1

    print(f"Starting Live capture with {encoder_info['label']} ({encoder_info['encoder']}).", flush=True)
    print("Use Cancel task or Ctrl+C to stop and finalize the .ts file.", flush=True)
    arguments = build_live_ts_arguments(
        stream_url,
        output,
        encoder_info["encoder"],
        progress=True,
        vaapi_device=encoder_info.get("vaapi_device"),
    )
    display_arguments = build_live_ts_arguments(
        "<resolved Live stream URL>",
        output,
        encoder_info["encoder"],
        progress=True,
        vaapi_device=encoder_info.get("vaapi_device"),
    )
    code, _ = run_command(
        ffmpeg,
        arguments,
        BASE_DIR,
        display_arguments=display_arguments,
    )
    print_result(code, output_directory)
    if code == 0:
        print(f"Created: {output}", flush=True)
    return code


def run_cli_live_ts_auto(url: str, output_directory: Path) -> int:
    return run_cli_live_ts_nvenc(url, output_directory, encoder_override="auto")


def streamlink_recording_filename(url: str) -> str:
    """Create a timestamped, cross-platform filename for a Live recording."""

    parsed = urlparse(url.strip())
    host = parsed.netloc.rsplit("@", 1)[-1].split(":", 1)[0].lower()
    path_parts = [part for part in parsed.path.split("/") if part]
    if host.endswith("twitch.tv") or host.endswith("kick.com"):
        label = path_parts[0] if path_parts else host
    elif "youtube" in host or host == "youtu.be":
        label = "youtube_live"
    else:
        label = host or "live_stream"
    timestamp = time.strftime("%Y%m%d-%H%M%S")
    return safe_filename_component(f"{label} [{timestamp}]", "live_stream") + ".ts"


def run_cli_streamlink(url: str, output_directory: Path) -> int:
    """Record a Live URL to MPEG-TS with Streamlink at the best quality."""

    streamlink = find_streamlink()
    ffmpeg = find_tool("ffmpeg.exe")
    if not streamlink:
        print("ERROR: Streamlink was not found beside this file, in Python's scripts folder, or on PATH.")
        print("Use Install tools first, then restart the toolkit.")
        return 1
    if not ffmpeg:
        print("ERROR: FFmpeg was not found. Streamlink needs it to mux separate video/audio streams into TS.")
        print("Use Install tools first, then restart the toolkit.")
        return 1

    output_directory.mkdir(parents=True, exist_ok=True)
    filename = streamlink_recording_filename(url)
    output = output_directory / filename
    counter = 2
    while output.exists():
        output = output_directory / f"{Path(filename).stem} ({counter}).ts"
        counter += 1

    print("Starting Live recording with Streamlink (best quality, original stream).", flush=True)
    print("The recording has no fixed end time. Use Cancel task or Ctrl+C to stop and finalize the .ts file.", flush=True)
    arguments = build_streamlink_arguments(url, output, ffmpeg)
    display_arguments = build_streamlink_arguments("<Live URL>", output, "<ffmpeg>")
    code, _ = run_command(
        streamlink,
        arguments,
        BASE_DIR,
        display_arguments=display_arguments,
    )
    if code == 0:
        if not output.is_file() or output.stat().st_size == 0:
            print("ERROR: Streamlink finished without creating a valid TS file.", flush=True)
            code = 1
    print_result(code, output_directory)
    if code == 0:
        print(f"Created: {output}", flush=True)
    return code


def parse_cli_paths(value: str) -> List[Path]:
    try:
        tokens = shlex.split(value, posix=False)
    except ValueError:
        tokens = value.split()
    paths = []
    for token in tokens:
        cleaned = token.strip().strip('"')
        if cleaned:
            paths.append(Path(cleaned).expanduser())
    return paths


def run_cli() -> int:
    output_directory = DEFAULT_OUTPUT_DIR
    while True:
        print("\n" + "=" * 64)
        print("PIXCLIP")
        print(f"yt-dlp : {find_tool('yt-dlp.exe') or 'Missing'}")
        print(f"Streamlink: {find_streamlink() or 'Missing'}")
        print(f"GPU    : {gpu_name()}")
        print(f"CPU    : {cpu_name()}")
        print(f"Output : {output_directory}")
        print("=" * 64)
        print("[ DOWNLOAD VIDEO ]")
        print("  [1] MP4 - Best quality")
        print("  [2] MP4 - 4K / 2160p High Quality")
        print("  [3] MP4 - H.264/AAC compatible")
        print("  [4] MP3 - Best quality")
        print("  [5] Video/Live - MPEG-TS (.ts)")
        print("[ LIVE / GPU ]")
        print(" [18] Live - MPEG-TS (.ts) - NVIDIA NVENC")
        print(" [19] Live - Streamlink - Original quality (.ts)")
        print(" [20] Live - MPEG-TS (.ts) - Auto GPU/CPU")
        print("[ PLAYLIST ]")
        print("  [6] Playlist MP4 - Best quality")
        print("  [7] Playlist MP4 - H.264/AAC compatible")
        print("  [8] Playlist MP3 - Best quality")
        print("[ CONVERT WITH FFMPEG ]")
        print("  [9] TS to MP4 - Fast / Original quality")
        print(" [10] TS to MP4 - H.264/AAC compatible")
        print(" [11] TS to MP4 - H.265/HEVC - RTX 2060 NVENC HQ")
        print(" [12] H.265/HEVC to H.264/AAC")
        print("[ TOOLS ]")
        print(" [13] Change download folder")
        print(" [14] Open download folder")
        print(" [15] Update yt-dlp")
        print(" [16] Join video clips with FFmpeg")
        print(" [17] Install / repair yt-dlp, FFmpeg, and Streamlink")
        print("  [0] Exit")

        choice = input("Select: ").strip()
        if choice == "0":
            return 0
        if choice in {"1", "2", "3", "4", "6", "7", "8"}:
            url = input("Paste video or playlist URL: ").strip()
            if not re.match(r"^https?://", url, re.IGNORECASE):
                print("Invalid URL.")
                continue
            mode_by_choice = {
                "1": DOWNLOAD_MODES[0],
                "2": DOWNLOAD_MODES[1],
                "3": DOWNLOAD_MODES[2],
                "4": DOWNLOAD_MODES[3],
                "6": DOWNLOAD_MODES[4],
                "7": DOWNLOAD_MODES[5],
                "8": DOWNLOAD_MODES[6],
            }
            run_cli_download(mode_by_choice[choice], url, output_directory)
            input("Press Enter to continue...")
        elif choice == "5":
            url = input("Paste video or Live URL: ").strip()
            if re.match(r"^https?://", url, re.IGNORECASE):
                run_cli_ts_download(url, output_directory)
            else:
                print("Invalid URL.")
            input("Press Enter to continue...")
        elif choice == "18":
            url = input("Paste Live URL: ").strip()
            if re.match(r"^https?://", url, re.IGNORECASE):
                run_cli_live_ts_nvenc(url, output_directory)
            else:
                print("Invalid URL.")
            input("Press Enter to continue...")
        elif choice == "19":
            url = input("Paste Live URL: ").strip()
            if re.match(r"^https?://", url, re.IGNORECASE):
                run_cli_streamlink(url, output_directory)
            else:
                print("Invalid URL.")
            input("Press Enter to continue...")
        elif choice == "20":
            url = input("Paste Live URL: ").strip()
            if re.match(r"^https?://", url, re.IGNORECASE):
                run_cli_live_ts_auto(url, output_directory)
            else:
                print("Invalid URL.")
            input("Press Enter to continue...")
        elif choice in {"9", "10", "11", "12"}:
            ffmpeg = find_tool("ffmpeg.exe")
            if not ffmpeg:
                print("ERROR: ffmpeg was not found.")
                input("Press Enter to continue...")
                continue
            mode = CONVERT_MODES[int(choice) - 9]
            if mode == CONVERT_MODES[2] and not has_nvenc(ffmpeg):
                print("ERROR: FFmpeg does not include hevc_nvenc or NVIDIA support was not detected.")
                input("Press Enter to continue...")
                continue
            raw_path = input("Video file: ").strip().strip('"')
            input_file = Path(raw_path).expanduser()
            if not input_file.is_file():
                print("ERROR: File not found.")
            else:
                run_cli_convert(mode, input_file)
            input("Press Enter to continue...")
        elif choice == "13":
            new_folder = input("New folder path (leave blank to cancel): ").strip().strip('"')
            if new_folder:
                candidate = Path(new_folder).expanduser()
                try:
                    candidate.mkdir(parents=True, exist_ok=True)
                    output_directory = candidate
                except OSError as exc:
                    print("ERROR: " + str(exc))
            input("Press Enter to continue...")
        elif choice == "14":
            try:
                open_folder(output_directory)
            except OSError as exc:
                print("ERROR: " + str(exc))
            input("Press Enter to continue...")
        elif choice == "15":
            update_ytdlp()
            input("Press Enter to continue...")
        elif choice == "16":
            raw_files = input("Clips (quote paths that contain spaces): ").strip()
            files = [path for path in parse_cli_paths(raw_files) if path.is_file()]
            if len(files) < 2:
                print("Please provide at least two valid files.")
                input("Press Enter to continue...")
                continue
            raw_mode = input("Mode [1 fast / 2 compatible]: ").strip() or "1"
            mode = JOIN_MODES[1] if raw_mode == "2" else JOIN_MODES[0]
            default_output = output_directory / ("joined_" + str(int(time.time())) + ".mp4")
            raw_output = input(f"Output MP4 (Enter for {default_output}): ").strip().strip('"')
            output = Path(raw_output).expanduser() if raw_output else default_output
            run_cli_join(files, mode, output)
            input("Press Enter to continue...")
        elif choice == "17":
            confirm = input("Continue with download and verification? [Y/N]: ").strip().lower()
            if confirm == "y":
                try:
                    install_tools()
                except Exception as exc:
                    print("ERROR: " + str(exc))
            input("Press Enter to continue...")
        else:
            print("Invalid choice.")


def make_rounded_button_class(tk):
    """Create a dependency-free rounded button using a Tk canvas."""

    class RoundedButton(tk.Canvas):
        def __init__(
            self,
            parent,
            text,
            command,
            *,
            bg,
            fg,
            activebackground,
            activeforeground,
            disabledforeground,
            width=None,
        ):
            self._button_text = text
            self._command = command
            self._base_bg = bg
            self._active_bg = activebackground
            self._base_fg = fg
            self._active_fg = activeforeground
            self._disabled_fg = disabledforeground
            self._font = ("Segoe UI", 10)
            self._state = "normal"
            self._hovered = False
            self._pressed = False
            self._radius = 11
            width_pixels = max(100, (width * 10) + 28) if width is not None else max(104, len(text) * 8 + 34)
            super().__init__(
                parent,
                width=width_pixels,
                height=40,
                bg=parent.cget("bg"),
                bd=0,
                highlightthickness=0,
                relief="flat",
                cursor="hand2",
                takefocus=True,
            )
            self.bind("<Configure>", lambda _event: self._draw())
            self.bind("<Enter>", self._on_enter)
            self.bind("<Leave>", self._on_leave)
            self.bind("<Button-1>", self._on_press)
            self.bind("<ButtonRelease-1>", self._on_release)
            self.bind("<FocusIn>", lambda _event: self._draw())
            self.bind("<FocusOut>", lambda _event: self._draw())
            self.bind("<Return>", self._on_keyboard_press)
            self.bind("<space>", self._on_keyboard_press)
            self._draw()

        @staticmethod
        def _rounded_points(x1, y1, x2, y2, radius):
            points = []
            for center_x, center_y, start_angle in (
                (x1 + radius, y1 + radius, 180),
                (x2 - radius, y1 + radius, 270),
                (x2 - radius, y2 - radius, 0),
                (x1 + radius, y2 - radius, 90),
            ):
                for step in range(7):
                    angle = (start_angle + (90 * step / 6)) * 3.141592653589793 / 180
                    points.extend((center_x + radius * __import__("math").cos(angle), center_y + radius * __import__("math").sin(angle)))
            return points

        def _draw(self):
            self.delete("all")
            width = max(1, self.winfo_width())
            height = max(1, self.winfo_height())
            if width < 8 or height < 8:
                return
            radius = min(self._radius, max(3, (height - 5) / 2), max(3, (width - 4) / 2))
            shadow = self._rounded_points(2, 3, width - 2, height - 1, radius)
            self.create_polygon(shadow, fill="#07101f", outline="")
            face = self._rounded_points(1, 1, width - 3, height - 4, radius)
            if self._state == "disabled":
                fill = "#24344d"
                text_color = self._disabled_fg
            elif self._pressed:
                fill = self._active_bg
                text_color = self._active_fg
            elif self._hovered:
                fill = self._active_bg
                text_color = self._active_fg
            else:
                fill = self._base_bg
                text_color = self._base_fg
            outline = "#60a5fa" if self.focus_get() is self else fill
            self.create_polygon(face, fill=fill, outline=outline, width=1)
            self.create_text(
                width / 2 - 1,
                (height - 3) / 2,
                text=self._button_text,
                fill=text_color,
                font=self._font,
            )

        def _on_enter(self, _event):
            self._hovered = True
            self._draw()

        def _on_leave(self, _event):
            self._hovered = False
            self._pressed = False
            self._draw()

        def _on_press(self, _event):
            if self._state != "disabled":
                self.focus_set()
                self._pressed = True
                self._draw()

        def _on_release(self, event):
            was_pressed = self._pressed
            self._pressed = False
            self._draw()
            if (
                was_pressed
                and self._state != "disabled"
                and 0 <= event.x <= self.winfo_width()
                and 0 <= event.y <= self.winfo_height()
            ):
                self._command()

        def _on_keyboard_press(self, _event):
            if self._state != "disabled":
                self._command()

        def configure(self, cnf=None, **kwargs):
            options = {}
            if cnf:
                options.update(cnf)
            options.update(kwargs)
            for key in ("bg", "background"):
                if key in options:
                    self._base_bg = options.pop(key)
            for key in ("fg", "foreground"):
                if key in options:
                    self._base_fg = options.pop(key)
            if "activebackground" in options:
                self._active_bg = options.pop("activebackground")
            if "activeforeground" in options:
                self._active_fg = options.pop("activeforeground")
            if "font" in options:
                self._font = options.pop("font")
            if "state" in options:
                self._state = options.pop("state")
                super().configure(cursor="arrow" if self._state == "disabled" else "hand2")
            if options:
                super().configure(**options)
            self._draw()

        config = configure

    return RoundedButton


def make_pixel_progress(tk, parent, colors):
    """Segmented Canvas meter with the same lifecycle used by the GUI."""
    class PixelProgress(tk.Canvas):
        def __init__(self):
            super().__init__(parent, height=16, bg=colors["input"], highlightthickness=1,
                             highlightbackground=colors["border"])
            self.mode, self.value, self.maximum = "indeterminate", 0, 100
            self.timer = None
            self.phase = 0
            self.bind("<Configure>", lambda event: self.draw())
            self.bind("<Destroy>", lambda event: self.stop())

        def configure(self, cnf=None, **kwargs):
            options = dict(cnf or {}, **kwargs)
            for key in ("mode", "value", "maximum"):
                if key in options:
                    setattr(self, key, options.pop(key))
            if options:
                super().configure(**options)
            self.draw()

        config = configure

        def draw(self):
            self.delete("all")
            count = max(1, (self.winfo_width() - 4) // 12)
            fraction = max(0, min(1, float(self.value) / max(1, float(self.maximum))))
            for index in range(count):
                active = index < int(count * fraction)
                if self.mode == "indeterminate":
                    active = (index - self.phase) % count < min(4, count)
                self.create_rectangle(3 + index * 12, 3, 12 + index * 12, 13,
                                      fill=colors["success"] if active else colors["surface_raised"], width=0)

        def start(self, interval=100):
            self.stop()
            def tick():
                self.phase += 1
                self.draw()
                self.timer = self.after(max(100, interval), tick)
            tick()

        def stop(self):
            if self.timer is not None:
                self.after_cancel(self.timer)
                self.timer = None

    return PixelProgress()


class MediaToolkitApp(WorkflowUI):
    """Tkinter GUI equivalent of the original WinForms PowerShell interface."""

    COLORS = {
        "background": "#101522",
        "surface": "#182133",
        "surface_raised": "#243149",
        "input": "#0b101b",
        "header": "#0b101b",
        "text": "#dce8df",
        "muted": "#a1b6aa",
        "disabled": "#74877d",
        "border": "#50617c",
        "primary": "#24543b",
        "primary_pressed": "#326b4b",
        "info": "#79ddc6",
        "download_surface": "#182133",
        "convert": "#204b48",
        "convert_pressed": "#2d6460",
        "convert_surface": "#182133",
        "success": "#8ae6bd",
        "warning": "#efc176",
        "danger": "#f87171",
        "log_background": "#0b101b",
        "log_text": "#c1cedc",
    }

    def __init__(self, root) -> None:
        import tkinter as tk
        from tkinter import ttk

        self.tk = tk
        self.ttk = ttk
        self.root = root
        from tkinter import font as tkfont
        families = set(tkfont.families(root))
        self.mono_font = next(
            (face for face in ("Cascadia Mono", "Consolas", "DejaVu Sans Mono", "Liberation Mono") if face in families),
            tkfont.nametofont("TkFixedFont").actual("family"),
        )
        self.settings = load_settings()
        self.output_directory = DEFAULT_OUTPUT_DIR
        remembered_folder = self.settings.get("output_directory")
        if isinstance(remembered_folder, str) and remembered_folder.strip():
            candidate_folder = Path(remembered_folder).expanduser()
            if candidate_folder.is_dir():
                self.output_directory = candidate_folder
        self.current_runner: Optional[ProcessRunner] = None
        self.current_temporary_files: List[Path] = []
        self.current_success_message = "Done - your file is ready."
        self.current_busy_message = ""
        self.live_recording = False
        self.recording_started_at = None
        self.recording_timer = None
        self.was_cancelled = False
        self.track_download_progress = False
        self.track_ffmpeg_progress = False
        self.current_progress_duration: Optional[float] = None
        self.current_progress_started_at: Optional[float] = None
        self.ytdlp_path: Optional[str] = None
        self.ffmpeg_path: Optional[str] = None
        self.ffprobe_path: Optional[str] = None
        self.streamlink_path: Optional[str] = None
        self.gpu_vendor, self.gpu_model = detect_gpu()
        self.cpu_model = cpu_name()

        self.root.title("PixClip")
        self.app_icon = None
        self.mascot_image = None
        icon_bitmap_applied = False
        icon_bitmap_path = BASE_DIR / "assets" / "pixclip.ico"
        if IS_WINDOWS and icon_bitmap_path.is_file():
            try:
                # Use the ICO as the native Windows window/taskbar icon.  The
                # PNG below remains available for child windows and fallback
                # platforms where Tk does not support ICO files.
                self.root.iconbitmap(default=str(icon_bitmap_path))
                icon_bitmap_applied = True
            except tk.TclError:
                pass
            icon_bitmap_applied = apply_windows_window_icon(self.root, icon_bitmap_path) or icon_bitmap_applied
        try:
            self.app_icon = tk.PhotoImage(master=root, file=str(BASE_DIR / "assets" / "pixclip-icon.png"))
            if not icon_bitmap_applied:
                self.root.iconphoto(True, self.app_icon)
            self.mascot_image = tk.PhotoImage(master=root, file=str(BASE_DIR / "assets" / "pixclip-header.png"))
        except tk.TclError:
            # Keep source-only copies usable when optional artwork is missing.
            pass
        self.root.geometry("1000x860")
        self.root.minsize(740, 780)
        self.root.configure(bg=self.COLORS["background"])
        self.root.protocol("WM_DELETE_WINDOW", self.on_close)
        self._configure_styles()
        self._build_ui()
        self.refresh_dependency_status()
        self.load_clipboard_url()
        self.setup_workflow(python_executable())

    def _configure_styles(self) -> None:
        style = self.ttk.Style(self.root)
        try:
            style.theme_use("clam")
        except self.tk.TclError:
            pass
        for option, value in (
            ("*TCombobox*Listbox.background", self.COLORS["input"]),
            ("*TCombobox*Listbox.foreground", self.COLORS["text"]),
            ("*TCombobox*Listbox.selectBackground", self.COLORS["primary"]),
            ("*TCombobox*Listbox.selectForeground", self.COLORS["text"]),
            # Tk parses option-database values as Tcl words.  The unbraced
            # "Segoe UI 10" value is split incorrectly on Windows, causing
            # the combobox popdown to fail with: expected integer but got UI.
            ("*TCombobox*Listbox.font", (self.mono_font, 10)),
            ("*TCombobox*Listbox.relief", "flat"),
            ("*TCombobox*Listbox.borderWidth", 0),
        ):
            self.root.option_add(option, value)
        style.configure(
            "Dark.TCombobox",
            fieldbackground=self.COLORS["input"],
            background=self.COLORS["input"],
            foreground=self.COLORS["text"],
            bordercolor=self.COLORS["border"],
            lightcolor=self.COLORS["border"],
            darkcolor=self.COLORS["border"],
            arrowcolor=self.COLORS["info"],
            padding=(12, 8),
            borderwidth=1,
            relief="flat",
        )
        style.map(
            "Dark.TCombobox",
            fieldbackground=[
                ("disabled", self.COLORS["surface"]),
                ("focus", self.COLORS["surface_raised"]),
                ("readonly", self.COLORS["input"]),
            ],
            foreground=[
                ("disabled", self.COLORS["disabled"]),
                ("readonly", self.COLORS["text"]),
            ],
            bordercolor=[("focus", self.COLORS["primary"]), ("readonly", self.COLORS["border"])],
            lightcolor=[("focus", self.COLORS["primary"]), ("readonly", self.COLORS["border"])],
            darkcolor=[("focus", self.COLORS["primary"]), ("readonly", self.COLORS["border"])],
            arrowcolor=[
                ("disabled", self.COLORS["disabled"]),
                ("active", self.COLORS["text"]),
                ("readonly", self.COLORS["info"]),
            ],
        )
        style.configure(
            "Busy.Horizontal.TProgressbar",
            troughcolor=self.COLORS["surface"],
            background=self.COLORS["primary"],
            lightcolor=self.COLORS["primary"],
            darkcolor=self.COLORS["primary"],
        )

    def pixel_icon(self, name: str):
        patterns = {
            "download": ["00011000","00011000","00011000","01111110","00111100","00011000","10000001","11111111"],
            "cut": ["11000011","11000110","00101100","00011000","00101100","11000110","11000011","00000000"],
            "convert": ["00000100","01111110","10000100","10000000","00000001","00100001","01111110","00100000"],
            "folder": ["11100000","10011110","10000001","10000001","10000001","10000001","11111111","00000000"],
            "join": ["11100111","10100101","10111101","10000001","10111101","10100101","11100111","00000000"],
            "stop": ["00000000","01111110","01111110","01111110","01111110","01111110","01111110","00000000"],
            "terminal": ["00000000","01000000","00100000","00010000","00100000","01001110","00000000","00000000"],
        }
        if not hasattr(self, "pixel_icons"):
            self.pixel_icons = {}
        if name not in self.pixel_icons:
            icon = self.tk.PhotoImage(master=self.root, width=16, height=16)
            color = self.COLORS["warning"] if name == "folder" else self.COLORS["info"]
            if name == "stop":
                color = self.COLORS["danger"]
            for y, row in enumerate(patterns[name]):
                for x, cell in enumerate(row):
                    if cell == "1":
                        icon.put(color, to=(x*2, y*2, x*2+2, y*2+2))
            self.pixel_icons[name] = icon
        return self.pixel_icons[name]

    def _button(self, parent, text: str, command, *, bg: Optional[str] = None, width: Optional[int] = None):
        label = text.lower()
        icon_name = next((name for word, name in (
            ("cancel", "stop"), ("cut", "cut"), ("join", "join"),
            ("folder", "folder"), ("download", "download"), ("video file", "convert"),
            ("update", "convert"),
        ) if word in label), "terminal")
        button = self.tk.Button(
            parent,
            text="[ " + text + " ]",
            command=command,
            image=self.pixel_icon(icon_name), compound="left",
            bg=bg or self.COLORS["surface_raised"],
            fg=self.COLORS["text"],
            activebackground=self.COLORS["border"],
            activeforeground=self.COLORS["text"],
            disabledforeground=self.COLORS["disabled"],
            font=(self.mono_font, 10),
            relief="raised",
            borderwidth=2,
            highlightthickness=1,
            highlightbackground=self.COLORS["border"],
            highlightcolor=self.COLORS["info"],
            padx=10,
            pady=9,
            takefocus=True,
        )
        button.bind("<Return>", lambda event: button.invoke())
        return button

    def _build_ui(self) -> None:
        tk = self.tk
        ttk = self.ttk

        header = tk.Frame(self.root, bg=self.COLORS["header"], height=106)
        header.pack(side="top", fill="x")
        header.pack_propagate(False)
        tk.Frame(header, bg=self.COLORS["primary"], height=3).pack(side="bottom", fill="x")
        self.help_button = self._button(header, "วิธีใช้ / HELP", self.show_help_dialog)
        self.help_button.configure(image="", font=(self.mono_font, 9), padx=6, pady=3, borderwidth=1)
        self.help_button.pack(side="right", anchor="n", padx=(8, 18), pady=(14, 0))
        if self.mascot_image is not None:
            self.mascot_canvas = tk.Canvas(header, width=80, height=100, bg=self.COLORS["header"], highlightthickness=0)
            self.mascot_canvas.pack(side="left", padx=(24, 0))
            self.mascot_canvas.create_image(40, 40, image=self.mascot_image, tags="mascot")
            self.mascot_canvas.create_text(40, 90, text="READY", fill=self.COLORS["success"], font=(self.mono_font, 8), tags="mood")
        title_block = tk.Frame(header, bg=self.COLORS["header"])
        title_block.pack(side="left", fill="both", expand=True)
        # Hand-drawn 5x7 glyphs stay pixel-sharp without an external font.
        glyphs = {
            "P":"11110/10001/10001/11110/10000/10000/10000",
            "X":"10001/10001/01010/00100/01010/10001/10001",
            "C":"01111/10000/10000/10000/10000/10000/01111",
            "U":"10001/10001/10001/10001/10001/10001/01110",
            "N":"10001/11001/11001/10101/10011/10011/10001",
            "I":"11111/00100/00100/00100/00100/00100/11111",
            "V":"10001/10001/10001/10001/10001/01010/00100",
            "E":"11111/10000/10000/11110/10000/10000/11111",
            "R":"11110/10001/10001/11110/10100/10010/10001",
            "S":"01111/10000/10000/01110/00001/00001/11110",
            "A":"01110/10001/10001/11111/10001/10001/10001",
            "L":"10000/10000/10000/10000/10000/10000/11111",
            "M":"10001/11011/10101/10101/10001/10001/10001",
            "D":"11110/10001/10001/10001/10001/10001/11110",
            "T":"11111/00100/00100/00100/00100/00100/00100",
            "O":"01110/10001/10001/10001/10001/10001/01110",
            "K":"10001/10010/10100/11000/10100/10010/10001",
        }
        title = tk.Canvas(title_block, height=36, bg=self.COLORS["header"], highlightthickness=0)
        title.pack(fill="x", padx=24, pady=(15, 3))
        def draw_title(event):
            title.delete("all")
            scale = 3 if event.width >= 430 else 2
            for index, letter in enumerate("PIXCLIP"):
                for y, row in enumerate(glyphs.get(letter, "").split("/")):
                    for x, cell in enumerate(row):
                        if cell == "1":
                            left, top = (index*6+x)*scale, y*scale
                            title.create_rectangle(left+2, top+2, left+scale+2, top+scale+2, fill=self.COLORS["primary"], width=0)
                            title.create_rectangle(left, top, left+scale, top+scale, fill=self.COLORS["success"], width=0)
        title.bind("<Configure>", draw_title)
        tk.Label(
            title_block,
            text="Download / Record / Convert / Cut / Join",
            bg=self.COLORS["header"],
            fg=self.COLORS["muted"],
            font=(self.mono_font, 10),
        ).pack(anchor="w", padx=27, pady=(3, 0))

        content = tk.Frame(self.root, bg=self.COLORS["background"])
        content.pack(fill="both", expand=True, padx=24, pady=(18, 14))
        content.grid_columnconfigure(0, weight=1)
        content.grid_columnconfigure(1, weight=0)
        content.grid_columnconfigure(2, weight=0)
        content.grid_rowconfigure(7, weight=1)

        tk.Label(
            content,
            text="[ DOWNLOAD / LIVE ]  Video / playlist URL",
            bg=self.COLORS["download_surface"],
            fg=self.COLORS["info"],
            anchor="sw",
            font=(self.mono_font, 11, "bold"),
            padx=8,
        ).grid(row=0, column=0, columnspan=3, sticky="nsew")

        self.url_var = tk.StringVar()
        self.url_entry = tk.Entry(
            content,
            textvariable=self.url_var,
            bg=self.COLORS["input"],
            fg=self.COLORS["text"],
            insertbackground=self.COLORS["text"],
            relief="flat",
            highlightthickness=1,
            highlightbackground=self.COLORS["border"],
            highlightcolor=self.COLORS["primary"],
            font=(self.mono_font, 11),
        )
        self.url_entry.grid(row=1, column=0, columnspan=2, sticky="nsew", pady=(3, 3), padx=(0, 8))
        self.paste_button = self._button(content, "Paste URL", self.paste_url)
        self.paste_button.grid(row=1, column=2, sticky="nsew", pady=(3, 3))

        self.download_mode_var = tk.StringVar(value=DOWNLOAD_MODES[0])
        self.download_mode_box = ttk.Combobox(
            content,
            textvariable=self.download_mode_var,
            values=DOWNLOAD_MODES,
            state="readonly",
            style="Dark.TCombobox",
            font=(self.mono_font, 10, "bold"),
            height=12,
        )
        self.download_mode_box.grid(row=2, column=0, columnspan=2, sticky="nsew", pady=(7, 7), padx=(0, 8))
        self.download_button = self._button(
            content,
            "1  DOWNLOAD",
            self.download,
            bg=self.COLORS["primary"],
        )
        self.download_button.configure(font=(self.mono_font, 10, "bold"), activebackground=self.COLORS["primary_pressed"])
        self.download_button.grid(row=2, column=2, sticky="nsew", pady=(5, 5))

        self.folder_var = tk.StringVar(value=str(self.output_directory))
        self.folder_entry = tk.Entry(
            content,
            textvariable=self.folder_var,
            state="readonly",
            readonlybackground=self.COLORS["surface"],
            fg=self.COLORS["muted"],
            relief="flat",
            highlightthickness=1,
            highlightbackground=self.COLORS["border"],
            font=(self.mono_font, 9),
        )
        self.folder_entry.grid(row=3, column=0, sticky="nsew", pady=(6, 5), padx=(0, 8))
        self.choose_folder_button = self._button(content, "Choose folder", self.choose_folder)
        self.choose_folder_button.grid(row=3, column=1, sticky="nsew", pady=(4, 4), padx=(0, 8))
        self.open_folder_button = self._button(content, "Open folder", self.open_output_folder)
        self.open_folder_button.grid(row=3, column=2, sticky="nsew", pady=(4, 4))

        convert_panel = tk.LabelFrame(
            content, text="[ CONVERT ]", font=(self.mono_font, 9),
            fg=self.COLORS["info"], bg=self.COLORS["convert_surface"],
            relief="solid", bd=1, padx=8, pady=4,
        )
        convert_panel.grid(row=4, column=0, columnspan=3, sticky="nsew")
        convert_panel.grid_columnconfigure(0, weight=1)
        self.convert_mode_var = tk.StringVar(value=CONVERT_MODES[0])
        self.convert_mode_box = ttk.Combobox(
            convert_panel,
            textvariable=self.convert_mode_var,
            values=CONVERT_MODES,
            state="readonly",
            style="Dark.TCombobox",
            font=(self.mono_font, 10, "bold"),
            height=8,
        )
        self.convert_mode_box.grid(row=0, column=0, sticky="nsew", padx=(0, 8), pady=9)
        self.convert_button = self._button(
            convert_panel,
            "2  CHOOSE VIDEO FILE",
            self.convert,
            bg=self.COLORS["convert"],
        )
        self.convert_button.configure(font=(self.mono_font, 10, "bold"), activebackground=self.COLORS["convert_pressed"])
        self.convert_button.grid(row=0, column=1, sticky="nsew", pady=7)

        tool_panel = tk.LabelFrame(
            content, text="[ EDIT / TOOLS ]", font=(self.mono_font, 9),
            fg=self.COLORS["muted"], bg=self.COLORS["background"],
            relief="solid", bd=1, padx=8, pady=4,
        )
        tool_panel.grid(row=5, column=0, columnspan=3, sticky="nsew")
        self.install_button = self._button(tool_panel, "Install tools", self.install_tools, width=12)
        self.install_button.grid(row=0, column=0, sticky="ew", padx=(0, 6), pady=4)
        self.join_button = self._button(tool_panel, "Join clips", self.show_join_dialog, width=9)
        self.join_button.grid(row=0, column=1, sticky="ew", padx=(0, 6), pady=4)
        self.clip_button = self._button(tool_panel, "Cut clip", self.show_clip_dialog, width=9)
        self.clip_button.grid(row=0, column=2, sticky="ew", pady=4)
        self.update_button = self._button(tool_panel, "Update yt-dlp", self.update_ytdlp, width=11)
        self.update_button.grid(row=1, column=0, sticky="ew", padx=(0, 6), pady=4)
        self.advanced_button = self._button(tool_panel, "Advanced", self.open_cli, width=9)
        self.advanced_button.grid(row=1, column=1, sticky="ew", padx=(0, 6), pady=4)
        self.cancel_button = self._button(tool_panel, "Cancel task", self.cancel_task, width=11)
        self.cancel_button.configure(fg=self.COLORS["danger"], state="disabled")
        self.cancel_button.grid(row=1, column=2, sticky="ew", pady=4)
        for column in range(3):
            tool_panel.grid_columnconfigure(column, weight=1, uniform="tools")

        status_panel = tk.Frame(content, bg=self.COLORS["background"])
        status_panel.grid(row=6, column=0, columnspan=3, sticky="nsew")
        self.recording_label = tk.Label(
            status_panel, text="", anchor="w", font=(self.mono_font, 10, "bold"),
            bg=self.COLORS["background"], fg=self.COLORS["danger"],
        )
        self.status_label = tk.Label(
            status_panel,
            text="Ready",
            bg=self.COLORS["background"],
            fg=self.COLORS["text"],
            anchor="w",
            font=(self.mono_font, 10, "bold"),
        )
        self.status_label.pack(fill="x")
        self.progress = make_pixel_progress(tk, status_panel, self.COLORS)
        self.progress.pack(side="bottom", fill="x", pady=(3, 0))
        self.progress.pack_forget()

        log_frame = tk.LabelFrame(
            content, text="[ OUTPUT / LOG ]", font=(self.mono_font, 9),
            fg=self.COLORS["muted"], bg=self.COLORS["log_background"],
            relief="solid", bd=1, padx=8, pady=6,
        )
        log_frame.grid(row=7, column=0, columnspan=3, sticky="nsew", pady=(4, 4))
        log_frame.grid_rowconfigure(0, weight=1)
        log_frame.grid_columnconfigure(0, weight=1)
        self.log_box = tk.Text(
            log_frame,
            bg=self.COLORS["log_background"],
            fg=self.COLORS["log_text"],
            insertbackground=self.COLORS["log_text"],
            relief="flat",
            bd=0,
            wrap="none",
            font=(self.mono_font, 9),
            state="disabled",
        )
        self.log_box.grid(row=0, column=0, sticky="nsew")
        for tag, color in (
            ("normal", "log_text"), ("error", "danger"),
            ("warning", "warning"), ("success", "success"), ("command", "info"),
        ):
            self.log_box.tag_configure(tag, foreground=self.COLORS[color])
        scrollbar = tk.Scrollbar(log_frame, command=self.log_box.yview)
        scrollbar.grid(row=0, column=1, sticky="ns")
        self.log_box.configure(yscrollcommand=scrollbar.set)

        status_footer = tk.Frame(
            content,
            bg=self.COLORS["surface"],
            relief="solid",
            bd=1,
            padx=6,
            pady=3,
        )
        status_footer.grid(row=8, column=0, columnspan=3, sticky="ew", pady=(2, 0))
        status_footer.grid_columnconfigure(0, weight=1)

        self.dependency_label = tk.Label(
            status_footer,
            bg=self.COLORS["surface"],
            fg=self.COLORS["muted"],
            anchor="w",
            justify="left",
            font=(self.mono_font, 8),
            padx=4,
            pady=1,
        )
        self.dependency_label.grid(row=0, column=0, sticky="ew")
        self.hardware_label = tk.Label(
            status_footer,
            bg=self.COLORS["surface"],
            fg=self.COLORS["info"],
            anchor="w",
            justify="left",
            font=(self.mono_font, 8),
            padx=4,
            pady=1,
        )
        self.hardware_label.grid(row=1, column=0, sticky="ew")

        def resize_status_footer(event):
            wraplength = max(200, event.width - 16)
            self.dependency_label.configure(wraplength=wraplength)
            self.hardware_label.configure(wraplength=wraplength)

        status_footer.bind("<Configure>", resize_status_footer)

    def post(self, callback: Callable, *arguments) -> None:
        try:
            self.root.after(0, callback, *arguments)
        except self.tk.TclError:
            pass

    def refresh_dependency_status(self) -> None:
        self.ytdlp_path = find_tool("yt-dlp.exe")
        self.ffmpeg_path = find_tool("ffmpeg.exe")
        self.ffprobe_path = find_tool("ffprobe.exe")
        self.streamlink_path = find_streamlink()
        ytdlp_status = "Ready" if self.ytdlp_path else "Missing"
        ffmpeg_status = "Ready" if self.ffmpeg_path else "Missing"
        ffprobe_status = "Ready" if self.ffprobe_path else "Missing"
        streamlink_status = "Ready" if self.streamlink_path else "Missing"
        encoder_label = "Unknown"
        if self.ffmpeg_path:
            encoder_label = select_live_encoder(
                self.ffmpeg_path,
                self.gpu_vendor,
                self.gpu_model,
            )["label"]
        tools_color = (
            self.COLORS["muted"]
            if self.ytdlp_path and self.ffmpeg_path and self.ffprobe_path and self.streamlink_path
            else self.COLORS["danger"]
        )
        self.dependency_label.configure(
            text=(
                f"SYSTEM  |  {current_windows_name()}"
                f"  |  yt-dlp: {ytdlp_status}"
                f"  |  FFmpeg: {ffmpeg_status}"
                f"  |  ffprobe: {ffprobe_status}"
                f"  |  Streamlink: {streamlink_status}"
            ),
            fg=tools_color,
        )
        self.hardware_label.configure(
            text=(
                f"HARDWARE  |  GPU: {self.gpu_model}"
                f"  |  CPU: {self.cpu_model}"
                f"  |  Auto encoder: {encoder_label}"
            ),
            fg=self.COLORS["info"],
        )
        if tools_color == self.COLORS["danger"]:
            self.set_status("Some required tools are missing. Use Install tools below.", self.COLORS["danger"])

    def load_clipboard_url(self) -> None:
        try:
            text = self.root.clipboard_get().strip()
        except self.tk.TclError:
            return
        if re.match(r"^https?://", text, re.IGNORECASE):
            self.url_var.set(text)

    def paste_url(self) -> None:
        try:
            self.url_var.set(self.root.clipboard_get().strip())
        except self.tk.TclError:
            pass

    def set_status(self, text: str, color: str) -> None:
        self.status_label.configure(text=text, fg=color)

    def append_log(self, text: str) -> None:
        # Match severity markers, not words inside filenames or URLs.
        line = text.strip()
        if re.match(r"^(?:ERROR\b|FAILED\b)", line, re.I) or re.search(r"\[(?:error|critical)\]", line, re.I):
            tag = "error"
        elif re.match(r"^(?:WARNING\b|CANCELLED\b)", line, re.I) or re.search(r"\[warning\]", line, re.I):
            tag = "warning"
        elif re.match(r"^(?:DONE\b|Created:|INSTALLATION COMPLETE|UPDATE COMPLETE)", line):
            tag = "success"
        elif line.startswith("> "):
            tag = "command"
        else:
            tag = "normal"
        self.log_box.configure(state="normal")
        self.log_box.insert("end", text + "\n", (tag,))
        self.log_box.see("end")
        self.log_box.configure(state="disabled")

    def handle_process_line(self, text: str) -> None:
        if getattr(self, "local_history", None) is not None:
            self.local_history["logs"].append(text)
            self.local_history["logs"] = self.local_history["logs"][-100:]
        self.append_log(text)
        if self.live_recording and self.recording_started_at is None:
            seconds = parse_ffmpeg_progress_seconds(text)
            if (seconds is not None and seconds > 0) or re.search(r"\bWritten\s+[1-9]", text, re.I):
                self.recording_started_at = time.monotonic()
        parsed_duration = parse_media_duration(text)
        if parsed_duration is not None:
            self.current_progress_duration = parsed_duration
        progress: Optional[float] = None
        if self.track_download_progress:
            if text.strip() == "progress=end":
                progress = 100.0
            else:
                progress = parse_download_progress(text)
                if progress is None and self.current_progress_duration:
                    seconds = parse_ffmpeg_progress_seconds(text)
                    if seconds is not None:
                        progress = (seconds / self.current_progress_duration) * 100.0
        elif self.track_ffmpeg_progress:
            if text.strip() == "progress=end":
                progress = 100.0
            else:
                seconds = parse_ffmpeg_progress_seconds(text)
                if seconds is not None and self.current_progress_duration:
                    progress = (seconds / self.current_progress_duration) * 100.0
        if progress is None:
            return
        progress = max(0.0, min(100.0, progress))
        self.progress.configure(mode="determinate", maximum=100, value=progress)
        eta_text = ""
        if (
            self.current_progress_started_at is not None
            and 0.0 < progress < 100.0
        ):
            elapsed = time.monotonic() - self.current_progress_started_at
            if elapsed >= 1.0:
                remaining = elapsed * (100.0 - progress) / progress
                eta_text = f" | ETA {format_eta(remaining)}"
        self.set_status(f"{self.current_busy_message} {progress:.1f}%{eta_text}", self.COLORS["info"])

    def update_recording_status(self) -> None:
        self.recording_timer = None
        if not self.live_recording:
            return
        if self.recording_started_at is None:
            text = "[ LIVE ] Connecting / waiting for stream..."
            color = self.COLORS["warning"]
        else:
            elapsed = format_eta(time.monotonic() - self.recording_started_at)
            text = f"■ REC  {elapsed} elapsed  |  Cancel task to stop"
            color = self.COLORS["danger"]
        self.recording_label.configure(text=text, fg=color)
        self.recording_timer = self.root.after(1000, self.update_recording_status)

    def set_busy(self, busy: bool, message: str = "") -> None:
        state = "disabled" if busy else "normal"
        for button in (
            self.convert_button,
            self.join_button,
            self.clip_button,
            self.install_button,
            self.update_button,
        ):
            button.configure(state=state)
        self.cancel_button.configure(state="normal" if busy else "disabled")
        self.download_mode_box.configure(state="readonly")
        self.convert_mode_box.configure(state="disabled" if busy else "readonly")
        if busy:
            self.set_status(message, self.COLORS["info"])
            self.progress.pack(side="bottom", fill="x", pady=(3, 0))
            if self.track_download_progress or self.track_ffmpeg_progress:
                self.progress.configure(mode="determinate", maximum=100, value=0)
            else:
                self.progress.configure(mode="indeterminate")
                self.progress.start(30)
        else:
            self.live_recording = False
            self.recording_started_at = None
            if self.recording_timer is not None:
                self.root.after_cancel(self.recording_timer)
                self.recording_timer = None
            self.recording_label.pack_forget()
            self.progress.stop()
            self.progress.configure(mode="indeterminate", value=0)
            self.progress.pack_forget()

    def clear_temporary_files(self) -> None:
        for path in self.current_temporary_files:
            try:
                path.unlink()
            except OSError:
                pass
        self.current_temporary_files = []

    def start_task(
        self,
        executable: str,
        arguments: Sequence[str],
        working_directory: Path,
        busy_message: str,
        success_message: str = "Done - your file is ready.",
        temporary_files: Optional[Sequence[Path]] = None,
        track_download_progress: bool = False,
        progress_duration: Optional[float] = None,
        live_recording: bool = False,
    ) -> None:
        if hasattr(self, "jobs") and self.jobs.active:
            from tkinter import messagebox
            messagebox.showinfo("PixClip", "Wait for downloads to finish, or cancel them in Queue / History first.", parent=self.root)
            return
        if self.current_runner is not None and self.current_runner.is_running:
            from tkinter import messagebox
            messagebox.showinfo("Media Toolkit", "A task is already running.", parent=self.root)
            return

        try:
            require_space(working_directory, self.reserve_var.get() if hasattr(self, "reserve_var") else 1)
        except (OSError, ValueError) as exc:
            from tkinter import messagebox
            messagebox.showerror("Disk space", str(exc), parent=self.root)
            return
        self.local_working_directory = working_directory
        self.last_job_state = "Working"

        self.log_box.configure(state="normal")
        self.log_box.delete("1.0", "end")
        self.log_box.configure(state="disabled")
        self.append_log("> " + command_text(executable, arguments))
        self.append_log("")
        self.current_busy_message = busy_message
        self.track_download_progress = track_download_progress
        self.track_ffmpeg_progress = progress_duration is not None
        self.current_progress_duration = progress_duration
        self.current_progress_started_at = time.monotonic()
        self.set_busy(True, busy_message)
        self.live_recording = live_recording
        self.recording_started_at = None
        if live_recording:
            self.recording_label.pack(fill="x", before=self.status_label, pady=(6, 2))
            self.update_recording_status()
        self.current_temporary_files = list(temporary_files or [])
        self.current_success_message = success_message
        self.was_cancelled = False
        if hasattr(self, "jobs") and "ffmpeg" in Path(executable).name.lower():
            self.local_history = self.jobs.record_local(executable, arguments, working_directory, busy_message, temporary_files or ())

        runner = ProcessRunner(
            executable,
            arguments,
            working_directory,
            lambda line: self.post(self.handle_process_line, line),
            lambda code, cancelled: self.post(self.task_finished, code, cancelled),
        )
        self.current_runner = runner
        runner.start()

    def task_finished(self, exit_code: int, cancelled: bool) -> None:
        if getattr(self, "local_history", None) is not None:
            job = self.local_history
            job.update(state="Cancelled" if cancelled or self.was_cancelled else "Done" if exit_code == 0 else "Failed", finished=time.time())
            job["detail"] = "Complete" if job["state"] == "Done" else "See job log; source file kept"
            output = Path(job["arguments"][-1])
            if exit_code == 0 and output.is_file():
                job["outputs"] = [str(output)]
            self.jobs.save()
            self.local_history = None
        self.current_runner = None
        self.set_busy(False)
        if cancelled or self.was_cancelled:
            self.last_job_state = "Check log"
            self.set_status("Task cancelled.", self.COLORS["warning"])
            self.append_log("")
            self.append_log("CANCELLED")
        elif exit_code == 0:
            self.last_job_state = "Done"
            self.refresh_dependency_status()
            self.set_status(self.current_success_message, self.COLORS["success"])
            self.append_log("")
            self.append_log("DONE")
        else:
            self.last_job_state = "Check log"
            self.set_status(f"Failed (exit code {exit_code}). See the log below.", self.COLORS["danger"])
            self.append_log("")
            self.append_log(f"FAILED - exit code {exit_code}")
        self.clear_temporary_files()
        self.track_download_progress = False
        self.track_ffmpeg_progress = False
        self.current_progress_duration = None
        self.current_progress_started_at = None
        self.current_busy_message = ""
        self.was_cancelled = False

    def choose_folder(self) -> None:
        from tkinter import filedialog

        selected = filedialog.askdirectory(
            parent=self.root,
            title="Choose where downloaded files will be saved.",
            initialdir=str(self.output_directory),
        )
        if selected:
            self.output_directory = Path(selected)
            self.folder_var.set(str(self.output_directory))
            self.settings["output_directory"] = str(self.output_directory)
            try:
                save_settings(self.settings)
            except OSError as exc:
                self.append_log("WARNING: Could not remember output folder: " + str(exc))

    def open_output_folder(self) -> None:
        try:
            open_folder(self.output_directory)
        except OSError as exc:
            self.set_status(str(exc), self.COLORS["danger"])

    def download(self) -> None:
        self.enqueue_download()

    def convert(self) -> None:
        from tkinter import filedialog, messagebox

        if not self.ffmpeg_path:
            messagebox.showerror("Missing FFmpeg", "ffmpeg was not found. Use Install tools first.")
            return
        selected = filedialog.askopenfilename(
            parent=self.root,
            title="Choose a video file",
            filetypes=[
                ("Video files", "*.mp4 *.mkv *.mov *.avi *.ts *.m2ts *.webm"),
                ("All files", "*.*"),
            ],
        )
        if not selected:
            return
        input_file = Path(selected)
        duration = get_media_duration(input_file)
        try:
            _, arguments = build_convert_arguments(
                input_file,
                self.convert_mode_var.get(),
                progress=duration is not None,
            )
        except ValueError as exc:
            messagebox.showerror("Convert", str(exc))
            return
        if self.convert_mode_var.get() == CONVERT_MODES[2] and not has_nvenc(self.ffmpeg_path):
            messagebox.showerror(
                "NVENC unavailable",
                "FFmpeg does not include hevc_nvenc or an NVIDIA encoder was not detected.",
            )
            return
        self.start_task(
            self.ffmpeg_path,
            arguments,
            input_file.parent,
            "Converting video...",
            progress_duration=duration,
        )

    def show_clip_dialog(self) -> None:
        from tkinter import filedialog, messagebox

        if not self.ffmpeg_path:
            messagebox.showerror("Missing FFmpeg", "ffmpeg was not found. Use Install tools first.")
            return

        tk = self.tk
        dialog = tk.Toplevel(self.root)
        dialog.title("Cut a clip segment")
        dialog.geometry("760x720")
        dialog.minsize(700, 680)
        dialog.configure(bg=self.COLORS["background"])
        dialog.transient(self.root)
        dialog.grab_set()

        tk.Label(
            dialog,
            text="Cut a clip segment",
            bg=self.COLORS["background"],
            fg=self.COLORS["text"],
            font=(self.mono_font, 18, "bold"),
        ).pack(anchor="w", padx=22, pady=(18, 0))
        tk.Label(
            dialog,
            text="Select a video and set the start/end time as HH:MM:SS or seconds.",
            bg=self.COLORS["background"],
            fg=self.COLORS["muted"],
            font=(self.mono_font, 9),
        ).pack(anchor="w", padx=24, pady=(2, 14))

        form = tk.Frame(dialog, bg=self.COLORS["background"])
        form.pack(fill="x", padx=24)
        form.grid_columnconfigure(0, weight=1)

        input_file_var = tk.StringVar()
        duration_var = tk.StringVar(value="Duration: unknown")
        start_var = tk.StringVar(value="00:00:00")
        end_var = tk.StringVar(value="00:00:30")
        clip_mode_var = tk.StringVar(value=CLIP_MODES[0])

        tk.Label(
            form,
            text="Video file",
            bg=self.COLORS["background"],
            fg=self.COLORS["info"],
            font=(self.mono_font, 10, "bold"),
        ).grid(row=0, column=0, sticky="w", pady=(0, 5))
        input_entry = tk.Entry(
            form,
            textvariable=input_file_var,
            state="readonly",
            readonlybackground=self.COLORS["surface"],
            fg=self.COLORS["muted"],
            relief="flat",
            highlightthickness=1,
            highlightbackground=self.COLORS["border"],
            font=(self.mono_font, 9),
        )
        input_entry.grid(row=1, column=0, sticky="ew", padx=(0, 8), pady=(0, 3))
        self._button(
            form,
            "Choose video",
            lambda: choose_source(),
            width=13,
        ).grid(row=1, column=1, sticky="e", pady=(0, 3))
        tk.Label(
            form,
            textvariable=duration_var,
            bg=self.COLORS["background"],
            fg=self.COLORS["muted"],
            anchor="w",
            font=(self.mono_font, 9),
        ).grid(row=2, column=0, columnspan=2, sticky="w", pady=(0, 14))

        times = tk.Frame(form, bg=self.COLORS["background"])
        times.grid(row=3, column=0, columnspan=2, sticky="ew")
        times.grid_columnconfigure(0, weight=1)
        times.grid_columnconfigure(1, weight=1)
        for column, label, variable in (
            (0, "Start time", start_var),
            (1, "End time", end_var),
        ):
            tk.Label(
                times,
                text=label,
                bg=self.COLORS["background"],
                fg=self.COLORS["info"],
                font=(self.mono_font, 10, "bold"),
            ).grid(row=0, column=column, sticky="w", padx=(0, 12) if column == 0 else (12, 0), pady=(0, 5))
            tk.Entry(
                times,
                textvariable=variable,
                bg=self.COLORS["input"],
                fg=self.COLORS["text"],
                insertbackground=self.COLORS["text"],
                relief="flat",
                highlightthickness=1,
                highlightbackground=self.COLORS["border"],
                highlightcolor=self.COLORS["primary"],
                font=(self.mono_font, 10),
            ).grid(row=1, column=column, sticky="ew", padx=(0, 12) if column == 0 else (12, 0), pady=(0, 14))

        tk.Label(
            form,
            text="Cut mode",
            bg=self.COLORS["background"],
            fg=self.COLORS["info"],
            font=(self.mono_font, 10, "bold"),
        ).grid(row=4, column=0, sticky="w", pady=(0, 5))
        self.ttk.Combobox(
            form,
            textvariable=clip_mode_var,
            values=CLIP_MODES,
            state="readonly",
            style="Dark.TCombobox",
            font=(self.mono_font, 9, "bold"),
        ).grid(row=5, column=0, columnspan=2, sticky="ew", pady=(0, 6))

        def choose_source() -> None:
            selected = filedialog.askopenfilename(
                parent=dialog,
                title="Choose a video file to cut",
                filetypes=[
                    ("Video files", "*.mp4 *.mkv *.mov *.avi *.ts *.m2ts *.webm"),
                    ("All files", "*.*"),
                ],
            )
            if not selected:
                return
            input_file = Path(selected)
            input_file_var.set(str(input_file))

        def cut_and_save() -> None:
            input_file = Path(input_file_var.get()).expanduser()
            if not input_file.is_file():
                messagebox.showerror("Cut clip", "Choose a valid video file first.", parent=dialog)
                return
            start_seconds = parse_timecode(start_var.get())
            end_seconds = parse_timecode(end_var.get())
            if start_seconds is None or end_seconds is None:
                messagebox.showerror(
                    "Cut clip",
                    "Enter valid start and end times, for example 00:01:30 or 90.",
                    parent=dialog,
                )
                return
            if end_seconds <= start_seconds:
                messagebox.showerror("Cut clip", "End time must be greater than start time.", parent=dialog)
                return
            source_duration = preview.duration or None
            if source_duration is not None:
                if start_seconds >= source_duration:
                    messagebox.showerror("Cut clip", "Start time is beyond the video duration.", parent=dialog)
                    return
                if end_seconds > source_duration + 0.05:
                    messagebox.showerror(
                        "Cut clip",
                        f"End time is beyond the video duration ({format_ffmpeg_time(source_duration)}).",
                        parent=dialog,
                    )
                    return

            default_name = (
                f"{input_file.stem}_clip_"
                f"{format_clip_filename_time(start_seconds)}-"
                f"{format_clip_filename_time(end_seconds)}.mp4"
            )
            selected_output = filedialog.asksaveasfilename(
                parent=dialog,
                title="Save clipped video as",
                initialdir=str(input_file.parent),
                initialfile=default_name,
                defaultextension=".mp4",
                filetypes=[("MP4 video", "*.mp4"), ("All files", "*.*")],
            )
            if not selected_output:
                return
            output_file = Path(selected_output).expanduser()
            try:
                if output_file.resolve() == input_file.resolve():
                    raise ValueError("Output file must be different from the input file.")
                output_file.parent.mkdir(parents=True, exist_ok=True)
            except (OSError, ValueError) as exc:
                messagebox.showerror("Cut clip", str(exc), parent=dialog)
                return
            if output_file.exists() and not messagebox.askyesno(
                "Overwrite output?",
                f"{output_file.name} already exists. Replace it?",
                parent=dialog,
            ):
                return
            try:
                arguments = build_clip_arguments(
                    input_file,
                    output_file,
                    start_seconds,
                    end_seconds,
                    clip_mode_var.get(),
                    progress=True,
                )
                self.start_task(
                    self.ffmpeg_path,
                    arguments,
                    output_file.parent,
                    "Cutting selected clip...",
                    "Clip created successfully.",
                    progress_duration=end_seconds - start_seconds,
                )
                dialog.destroy()
            except (OSError, ValueError) as exc:
                messagebox.showerror("Cut clip", str(exc), parent=dialog)

        preview = ClipPreview(self, dialog, input_file_var, start_var, end_var, duration_var)
        buttons = tk.Frame(dialog, bg=self.COLORS["background"])
        buttons.pack(fill="x", padx=24, pady=(14, 18))
        self._button(
            buttons,
            "CANCEL",
            dialog.destroy,
            bg=self.COLORS["surface_raised"],
            width=10,
        ).pack(side="right", padx=(8, 0), ipadx=8, ipady=4)
        self._button(
            buttons,
            "CUT CLIP",
            cut_and_save,
            bg=self.COLORS["convert"],
            width=12,
        ).pack(side="right", ipadx=8, ipady=4)

    def update_ytdlp(self) -> None:
        from tkinter import messagebox

        if not self.ytdlp_path:
            messagebox.showerror("Missing yt-dlp", "yt-dlp was not found. Use Install tools first.")
            return
        self.start_task(
            python_executable(),
            [str(SCRIPT_PATH), "--update-ytdlp"],
            BASE_DIR,
            "Updating yt-dlp...",
            "yt-dlp updated and verified.",
        )

    def install_tools(self) -> None:
        from tkinter import messagebox

        if not messagebox.askyesno(
            "Install or repair media tools",
            "Download official releases and verify SHA-256 checksums?\nLinux may ask for sudo to install python3-tk.",
            parent=self.root,
        ):
            return
        self.start_task(
            python_executable(),
            [str(SCRIPT_PATH), "--install-tools"],
            BASE_DIR,
            "Downloading and verifying media tools...",
            "yt-dlp, FFmpeg, and Streamlink installed and verified.",
            track_download_progress=True,
        )

    def open_cli(self) -> None:
        try:
            flags = CREATE_NEW_CONSOLE if IS_WINDOWS else 0
            kwargs = {"cwd": str(BASE_DIR), "creationflags": flags}
            if not IS_WINDOWS:
                kwargs["start_new_session"] = True
            subprocess.Popen([python_executable(), str(SCRIPT_PATH), "--cli"], **kwargs)
        except OSError as exc:
            self.set_status("Could not open Advanced mode: " + str(exc), self.COLORS["danger"])

    def cancel_task(self) -> None:
        if self.current_runner is not None and self.current_runner.is_running:
            self.was_cancelled = True
            self.current_runner.cancel()
            self.set_status("Task cancelled.", self.COLORS["warning"])

    def show_join_dialog(self) -> None:
        from tkinter import filedialog, messagebox

        if not self.ffmpeg_path:
            messagebox.showerror("Missing FFmpeg", "ffmpeg was not found. Use Install tools first.")
            return

        tk = self.tk
        dialog = tk.Toplevel(self.root)
        dialog.title("Join clips with FFmpeg")
        dialog.geometry("760x560")
        dialog.minsize(680, 480)
        dialog.configure(bg=self.COLORS["background"])
        dialog.transient(self.root)
        dialog.grab_set()

        tk.Label(
            dialog,
            text="Join video clips",
            bg=self.COLORS["background"],
            fg=self.COLORS["text"],
            font=(self.mono_font, 18, "bold"),
        ).pack(anchor="w", padx=22, pady=(18, 0))
        tk.Label(
            dialog,
            text="Add clips in playback order. Fast mode requires matching codecs and formats.",
            bg=self.COLORS["background"],
            fg=self.COLORS["muted"],
            font=(self.mono_font, 9),
        ).pack(anchor="w", padx=24, pady=(2, 10))

        body = tk.Frame(dialog, bg=self.COLORS["background"])
        body.pack(fill="both", expand=True, padx=24)
        body.grid_rowconfigure(0, weight=1)
        body.grid_columnconfigure(0, weight=1)

        listbox = tk.Listbox(
            body,
            bg=self.COLORS["input"],
            fg=self.COLORS["text"],
            selectbackground=self.COLORS["convert"],
            selectforeground=self.COLORS["text"],
            selectmode="extended",
            relief="flat",
            highlightthickness=1,
            highlightbackground=self.COLORS["border"],
            font=(self.mono_font, 10),
        )
        listbox.grid(row=0, column=0, sticky="nsew", padx=(0, 14))
        paths: List[Path] = []
        clip_count_var = tk.StringVar(value="0 clips")
        tk.Label(
            body,
            textvariable=clip_count_var,
            bg=self.COLORS["background"],
            fg=self.COLORS["muted"],
            anchor="w",
            font=(self.mono_font, 9),
        ).grid(row=1, column=0, sticky="w", padx=(0, 14), pady=(5, 0))

        side = tk.Frame(body, bg=self.COLORS["background"])
        side.grid(row=0, column=1, sticky="n")

        def redraw() -> None:
            listbox.delete(0, "end")
            for index, path in enumerate(paths, start=1):
                listbox.insert("end", f"{index}. {path.name}")
            clip_count_var.set(f"{len(paths)} clip(s) selected")

        def add_clips() -> None:
            selected = filedialog.askopenfilenames(
                parent=dialog,
                title="Select clips in the order you want them joined",
                filetypes=[
                    ("Video files", "*.mp4 *.mkv *.mov *.avi *.ts *.m2ts *.webm"),
                    ("All files", "*.*"),
                ],
            )
            for item in selected:
                candidate = Path(item)
                if candidate not in paths:
                    paths.append(candidate)
            redraw()

        def remove_selected() -> None:
            selected = list(listbox.curselection())
            for index in reversed(selected):
                del paths[index]
            if selected:
                redraw()

        def clear_clips() -> None:
            if paths:
                paths.clear()
                redraw()

        def move_selected(delta: int) -> None:
            selected = listbox.curselection()
            if not selected:
                return
            index = selected[0]
            new_index = index + delta
            if 0 <= new_index < len(paths):
                paths[index], paths[new_index] = paths[new_index], paths[index]
                redraw()
                listbox.selection_set(new_index)

        for label, callback in (
            ("Add clips", add_clips),
            ("Remove selected", remove_selected),
            ("Move up", lambda: move_selected(-1)),
            ("Move down", lambda: move_selected(1)),
            ("Clear all", clear_clips),
        ):
            self._button(side, label, callback, width=14).pack(fill="x", pady=(0, 8))

        bottom = tk.Frame(dialog, bg=self.COLORS["background"])
        bottom.pack(fill="x", padx=24, pady=(12, 18))
        bottom.grid_columnconfigure(1, weight=1)
        tk.Label(
            bottom,
            text="Join mode",
            bg=self.COLORS["background"],
            fg=self.COLORS["info"],
            font=(self.mono_font, 10, "bold"),
        ).grid(row=0, column=0, sticky="w", pady=(0, 6))
        join_mode_var = tk.StringVar(value=JOIN_MODES[0])
        join_mode_box = self.ttk.Combobox(
            bottom,
            textvariable=join_mode_var,
            values=JOIN_MODES,
            state="readonly",
            style="Dark.TCombobox",
            font=(self.mono_font, 9),
        )
        join_mode_box.grid(row=1, column=0, sticky="ew", padx=(0, 18))

        def join_and_save() -> None:
            if len(paths) < 2:
                messagebox.showinfo("Join clips", "Add at least two clips before joining.", parent=dialog)
                return
            initial_dir = str(paths[0].parent)
            selected_output = filedialog.asksaveasfilename(
                parent=dialog,
                title="Save joined video as",
                initialdir=initial_dir,
                initialfile="joined.mp4",
                defaultextension=".mp4",
                filetypes=[("MP4 video", "*.mp4")],
            )
            if not selected_output:
                return
            output_file = Path(selected_output).expanduser()
            try:
                normalized_paths = validate_join_inputs(paths, output_file)
            except ValueError as exc:
                messagebox.showerror("Join clips", str(exc), parent=dialog)
                return
            if output_file.exists() and not messagebox.askyesno(
                "Overwrite output?",
                f"{output_file.name} already exists. Replace it?",
                parent=dialog,
            ):
                return

            selected_mode = join_mode_var.get()
            if selected_mode == JOIN_MODES[0]:
                compatible = fast_join_compatibility(normalized_paths)
                if compatible is False:
                    switch_mode = messagebox.askyesno(
                        "Fast join may fail",
                        "The clips do not have identical stream settings. "
                        "Switch to Compatible MP4 mode and re-encode them?",
                        parent=dialog,
                    )
                    if not switch_mode:
                        return
                    join_mode_var.set(JOIN_MODES[1])
                    selected_mode = JOIN_MODES[1]

            total_duration = get_total_media_duration(normalized_paths)
            list_file: Optional[Path] = None
            try:
                with tempfile.NamedTemporaryFile(
                    mode="w", suffix=".txt", prefix="ytdlp_join_", delete=False, encoding="utf-8", newline="\n"
                ) as stream:
                    list_file = Path(stream.name)
                    stream.write("\n".join(concat_file_line(item) for item in normalized_paths) + "\n")
                output_file.parent.mkdir(parents=True, exist_ok=True)
                arguments = build_join_arguments(
                    list_file,
                    output_file,
                    selected_mode,
                    progress=total_duration is not None,
                )
                self.start_task(
                    self.ffmpeg_path,
                    arguments,
                    output_file.parent,
                    "Joining clips...",
                    "Clips joined successfully.",
                    [list_file],
                    progress_duration=total_duration,
                )
                dialog.destroy()
            except (OSError, ValueError) as exc:
                if list_file is not None:
                    try:
                        list_file.unlink()
                    except OSError:
                        pass
                messagebox.showerror("Join clips", str(exc), parent=dialog)

        self._button(
            bottom,
            "JOIN AND SAVE MP4",
            join_and_save,
            bg=self.COLORS["convert"],
        ).grid(row=0, column=1, rowspan=2, sticky="e", padx=(0, 0), ipadx=25, ipady=8)

    def on_close(self) -> None:
        from tkinter import messagebox

        if self.jobs.active or any(j["state"] == "Queued" for j in self.jobs.jobs):
            if not messagebox.askyesno("PixClip", "Close PixClip? Running downloads will be cancelled. Queued schedules only run while the app is open.", parent=self.root):
                return

        if self.current_runner is not None and self.current_runner.is_running:
            if not messagebox.askyesno(
                "Media Toolkit",
                "A task is still running. Close and cancel it?",
                parent=self.root,
            ):
                return
            self.was_cancelled = True
            self.current_runner.cancel()
        self.workflow_closing = True
        for job in self.jobs.jobs:
            if job["id"] in self.jobs.active:
                self.jobs.cancel(job)
                job["state"] = "Interrupted"
        self.jobs.save()
        def finish_close():
            self.jobs.tick(allow_start=False)
            if self.jobs.active or (self.current_runner is not None and self.current_runner.is_running):
                self.root.after(100, finish_close)
            else:
                self.root.destroy()
        finish_close()


def run_gui() -> int:
    try:
        import tkinter as tk
    except ImportError as exc:
        print("Tkinter is required for the GUI: " + str(exc), file=sys.stderr)
        return 1
    configure_windows_app_identity()
    root = tk.Tk()
    MediaToolkitApp(root)
    root.mainloop()
    return 0


def compatibility_report() -> int:
    report = {
        "Compatible": True,
        "Python": platform.python_version(),
        "Platform": platform.platform(),
        "GPU": gpu_name(),
        "CPU": cpu_name(),
        "Windows": current_windows_name(),
        "YtDlpFound": bool(find_tool("yt-dlp.exe")),
        "FfmpegFound": bool(find_tool("ffmpeg.exe")),
        "FfprobeFound": bool(find_tool("ffprobe.exe")),
        "StreamlinkFound": bool(find_streamlink()),
        "TkinterLoaded": tkinter_available(),
    }
    if not report["TkinterLoaded"]:
        report["Compatible"] = False
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["Compatible"] else 1


def write_error_log(exc: BaseException) -> None:
    details = [
        "Time: " + time.strftime("%Y-%m-%d %H:%M:%S"),
        "Platform: " + platform.platform(),
        "Python: " + platform.python_version(),
        "Error: " + str(exc),
    ]
    try:
        (BASE_DIR / "Media-Toolkit-Error.log").write_text("\n".join(details), encoding="utf-8")
    except OSError:
        pass


def main(argv: Optional[Sequence[str]] = None) -> int:
    configure_output_encoding()
    parser = argparse.ArgumentParser(description="PixClip")
    parser.add_argument("--cli", action="store_true", help="open the advanced command-line menu")
    parser.add_argument(
        "--install-tools",
        action="store_true",
        help="install and verify yt-dlp, FFmpeg, and Streamlink",
    )
    parser.add_argument("--update-ytdlp", action="store_true", help="update and verify yt-dlp")
    parser.add_argument(
        "--download-ts",
        nargs=2,
        metavar=("URL", "OUTPUT_DIR"),
        help="download one video or Live URL as an MPEG-TS file",
    )
    parser.add_argument(
        "--download-live-ts-nvenc",
        nargs=2,
        metavar=("URL", "OUTPUT_DIR"),
        help="capture an ongoing Live URL as MPEG-TS with NVIDIA H.264 NVENC",
    )
    parser.add_argument(
        "--download-live-ts-auto",
        nargs=2,
        metavar=("URL", "OUTPUT_DIR"),
        help="capture an ongoing Live URL as MPEG-TS with the detected GPU or CPU",
    )
    parser.add_argument(
        "--record-live-streamlink",
        nargs=2,
        metavar=("URL", "OUTPUT_DIR"),
        help="record an ongoing Live URL as an original-quality MPEG-TS with Streamlink",
    )
    parser.add_argument("--source-check", action="store_true", help="only verify that official checksum sources are reachable")
    parser.add_argument("--compatibility-test", action="store_true", help="print an environment compatibility report")
    args = parser.parse_args(argv)

    try:
        if args.compatibility_test:
            return compatibility_report()
        if args.update_ytdlp:
            return update_ytdlp()
        if args.download_ts:
            url, output_directory = args.download_ts
            return run_cli_ts_download(url, Path(output_directory).expanduser())
        if args.download_live_ts_nvenc:
            url, output_directory = args.download_live_ts_nvenc
            return run_cli_live_ts_nvenc(url, Path(output_directory).expanduser())
        if args.download_live_ts_auto:
            url, output_directory = args.download_live_ts_auto
            return run_cli_live_ts_auto(url, Path(output_directory).expanduser())
        if args.record_live_streamlink:
            url, output_directory = args.record_live_streamlink
            return run_cli_streamlink(url, Path(output_directory).expanduser())
        if args.install_tools:
            return install_tools(source_check=args.source_check)
        if args.cli:
            return run_cli()
        return run_gui()
    except KeyboardInterrupt:
        return 130
    except Exception as exc:  # keep double-click failures diagnosable
        write_error_log(exc)
        print("ERROR: " + str(exc), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

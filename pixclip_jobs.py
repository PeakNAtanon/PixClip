"""Persistent download queue and isolated workers; standard library only.

Each download owns a directory and process tree. UI state changes happen only
in tick(), on Tk's thread. A worker imports the existing CLI, never the GUI.
"""
from __future__ import annotations

import json
import math
import os
import re
import signal
import shutil
import subprocess
import sys
import threading
import time
import uuid
from pathlib import Path
from queue import Queue, Empty
from urllib.parse import urlparse

QUALITIES = ("Best available", "Up to 2160p", "Up to 1080p", "Up to 720p", "Up to 480p", "Audio only (MP3)")
TERMINAL = {"Done", "Failed", "Cancelled", "Interrupted"}
GIB = 1024 ** 3


def parse_ffmpeg_progress_seconds(text):
    """Read FFmpeg's progress markers in microseconds or timecode form."""

    line = str(text).strip()
    if line.startswith("out_time_us=") or line.startswith("out_time_ms="):
        try:
            return max(0.0, float(line.split("=", 1)[1]) / 1_000_000.0)
        except ValueError:
            return None
    if line.startswith("out_time="):
        try:
            hours, minutes, seconds = line.split("=", 1)[1].split(":")
            return max(0.0, float(hours) * 3600.0 + float(minutes) * 60.0 + float(seconds))
        except (ValueError, AttributeError):
            return None
    return None


def format_elapsed(seconds):
    total = max(0, int(seconds))
    hours, remainder = divmod(total, 3600)
    minutes, seconds = divmod(remainder, 60)
    return f"{hours:02d}:{minutes:02d}:{seconds:02d}"


def safe_storage_component(value, fallback="unknown"):
    """Make a readable folder-name component safe on Windows and Unix."""

    cleaned = re.sub(r'[<>:"/\\|?*\x00-\x1f]', "_", str(value))
    cleaned = re.sub(r"\s+", " ", cleaned).strip().rstrip(" .")
    if cleaned.upper() in {"CON", "PRN", "AUX", "NUL"}:
        cleaned += "_"
    return (cleaned[:100] or fallback).strip()


def storage_category(mode):
    """Return the top-level folder used for a download mode."""

    value = str(mode or "").lower()
    if "live" in value:
        return "Live"
    if "playlist" in value:
        return "Playlists"
    if "mp3" in value or "audio" in value:
        return "Audio"
    return "Videos"


def source_label(url):
    """Get a short source/channel label for a readable output folder."""

    parsed = urlparse(str(url or "").strip())
    host = parsed.netloc.rsplit("@", 1)[-1].split(":", 1)[0].lower()
    host = host[4:] if host.startswith("www.") else host
    parts = [part for part in parsed.path.split("/") if part]
    if host.endswith(("kick.com", "twitch.tv", "tiktok.com")) and parts:
        label = parts[0].lstrip("@")
    elif host.endswith("youtube.com") or host == "youtu.be":
        label = "youtube"
        if parts and parts[0].startswith("@"):
            label = parts[0].lstrip("@")
    else:
        label = host or "source"
    return safe_storage_component(label, "source")


def organized_output_folder(folder, mode, url, job_id=None, created=None):
    """Build a dated, readable folder for a download or recording."""

    try:
        timestamp = float(created) if created is not None else time.time()
        local_time = time.localtime(timestamp)
    except (TypeError, ValueError, OverflowError, OSError):
        local_time = time.localtime()
    date_folder = time.strftime("%Y-%m-%d", local_time)
    readable = time.strftime("%H%M%S", local_time) + " - " + source_label(url)
    if job_id:
        readable += " [" + safe_storage_component(str(job_id)[:8], "job") + "]"
    return (Path(folder).expanduser() / storage_category(mode) / date_folder /
            safe_storage_component(readable, "PixClip-job"))


def job_output_directory(job):
    """Resolve a job's output folder, including the legacy layout."""

    stored = job.get("output_folder")
    if isinstance(stored, str) and stored.strip():
        return Path(stored).expanduser()
    return (Path(job["folder"]).expanduser() /
            ("PixClip-" + str(job["id"])[:12]))


def state_path():
    base = (Path(os.environ.get("LOCALAPPDATA", Path.home())) if os.name == "nt"
            else Path(os.environ.get("XDG_STATE_HOME", Path.home() / ".local" / "state")))
    return base / "PixClip" / "jobs.json"


def settings_path():
    return state_path().with_name("settings.json")


def load_settings(path=None):
    target = Path(path) if path else settings_path()
    try:
        data = json.loads(target.read_text(encoding="utf-8"))
    except (FileNotFoundError, OSError, ValueError, TypeError):
        return {}
    return data if isinstance(data, dict) else {}


def save_settings(settings, path=None):
    target = Path(path) if path else settings_path()
    target.parent.mkdir(parents=True, exist_ok=True)
    temporary = target.with_suffix(".tmp")
    temporary.write_text(json.dumps(settings, ensure_ascii=False, indent=2), encoding="utf-8")
    os.replace(temporary, target)


def stop_tree(process, grace=1):
    if os.name == "nt":
        if process.poll() is not None:
            return
        subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                       capture_output=True, timeout=15, creationflags=subprocess.CREATE_NO_WINDOW)
    else:
        try:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=grace)
        except subprocess.TimeoutExpired:
            pass
        except ProcessLookupError:
            pass
        finally:
            # The group can outlive its leader, including children ignoring TERM.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass


def require_space(folder, reserve_gb=1, extra_bytes=0):
    if not math.isfinite(float(reserve_gb)) or float(reserve_gb) < .25:
        raise ValueError("Disk reserve must be at least 0.25 GiB")
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    free = shutil.disk_usage(folder).free
    needed = max(0.25, float(reserve_gb)) * GIB + extra_bytes
    if free < needed:
        raise OSError(f"Low disk space: {free/GIB:.2f} GiB free; need {needed/GIB:.2f} GiB. Free space or choose another folder.")
    return free


def quality_arguments(arguments, quality):
    args = list(arguments)
    if quality == QUALITIES[0] or quality == QUALITIES[-1] or "-f" not in args:
        return args
    height = int(re.search(r"\d+", quality).group())
    index = args.index("-f") + 1
    if args[index] == "ba/b":
        return args
    # Direct Live resolution needs one muxed stream, not two URLs.
    args[index] = (f"b[height<={height}]" if "-g" in args else
                   f"bv*[height<={height}]+ba/b[height<={height}]")
    return args


class JobQueue:
    def __init__(self, python, path=None):
        self.python = python
        self.path = Path(path) if path else state_path()
        self.jobs = []
        self.active = {}
        self.events = Queue()
        self.recent_lines = []
        self.limit = 1
        self.paused = False
        self.warning = ""
        try:
            data = json.loads(self.path.read_text(encoding="utf-8"))
            if not isinstance(data, dict) or not isinstance(data.get("jobs"), list):
                raise ValueError("Invalid jobs file")
            self.limit = max(1, min(5, int(data.get("limit", 1))))
            self.paused = bool(data.get("paused", False))
            for job in data["jobs"]:
                if not isinstance(job, dict) or not all(k in job for k in ("id", "url", "mode", "folder", "state")):
                    continue
                if job["state"] in ("Running", "Cancelling"):
                    job["state"] = "Interrupted"
                    job["detail"] = "App closed during this job. Retry manually; partial files were kept."
                self.jobs.append(job)
        except FileNotFoundError:
            pass
        except (OSError, ValueError, TypeError) as exc:
            self.warning = "History could not be loaded: " + str(exc)
            # Preserve malformed history rather than silently replacing it.
            self.path = self.path.with_name("jobs-recovered-" + uuid.uuid4().hex[:8] + ".json")

    def save(self):
        try:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            temporary = self.path.with_suffix(".tmp")
            temporary.write_text(json.dumps({"jobs": self.jobs, "limit": self.limit, "paused": self.paused},
                                             ensure_ascii=False, indent=2), encoding="utf-8")
            os.replace(temporary, self.path)
        except OSError as exc:
            self.warning = "History was not saved: " + str(exc)

    def add(self, url, mode, folder, quality=QUALITIES[0], auto_mp4=False,
            start_at=0, duration=0, reserve_gb=1, cookies_file=None):
        from urllib.parse import urlparse
        parsed = urlparse(url)
        if parsed.scheme not in ("http", "https") or not parsed.netloc:
            raise ValueError("Each line must be a valid http(s) URL.")
        if quality not in QUALITIES:
            raise ValueError("Unknown quality")
        if not all(math.isfinite(float(v)) and float(v) >= 0 for v in (start_at, duration, reserve_gb)) or float(reserve_gb) < .25:
            raise ValueError("Invalid schedule or disk reserve")
        cookie_path = None
        if cookies_file is not None and str(cookies_file).strip():
            cookie_path = Path(str(cookies_file)).expanduser()
            if not cookie_path.is_file():
                raise ValueError("Cookies file was not found: " + str(cookie_path))
        job_id = uuid.uuid4().hex
        created = time.time()
        folder_path = Path(folder).expanduser()
        job = dict(id=job_id, url=url, mode=mode, folder=str(folder_path),
                   output_folder=str(organized_output_folder(folder_path, mode, url, job_id, created)),
                   quality=quality, auto_mp4=auto_mp4, start_at=float(start_at), duration=float(duration),
                   reserve_gb=float(reserve_gb), state="Queued", detail="Waiting", progress=None,
                   cookies_file=str(cookie_path) if cookie_path else None,
                   created=created, outputs=[], logs=[])
        self.jobs.append(job)
        self.save()
        return job

    def retry(self, job):
        if job["state"] not in TERMINAL:
            raise ValueError("Cancel or finish this job before retrying.")
        return self.add(job["url"], job["mode"], job["folder"], job.get("quality", QUALITIES[0]),
                        job.get("auto_mp4", False), duration=job.get("duration", 0),
                        reserve_gb=job.get("reserve_gb", 1), cookies_file=job.get("cookies_file"))

    def record_local(self, executable, arguments, folder, label, temporary_files=()):
        snapshots = {str(p): Path(p).read_text(encoding="utf-8") for p in temporary_files if Path(p).is_file()}
        job = dict(id=uuid.uuid4().hex, kind="local", url=label, mode=label, folder=str(folder),
                   executable=executable, arguments=list(map(str, arguments)), temporary=snapshots,
                   state="Running", detail=label, progress=None, created=time.time(),
                   started=time.time(), outputs=[], logs=[])
        self.jobs.append(job)
        self.save()
        return job

    def cancel(self, job):
        if job["state"] == "Queued":
            job.update(state="Cancelled", detail="Cancelled before starting")
            self.save()
        elif job["id"] in self.active:
            job.update(state="Cancelling", detail="Stopping process tree; partial files kept")
            self.active[job["id"]].set()

    def delete(self, job):
        """Remove a queued or completed job from history without deleting media files."""

        if job.get("state") in {"Running", "Cancelling"} or job.get("id") in self.active:
            raise ValueError("Cancel the running job before deleting it.")
        for index, candidate in enumerate(self.jobs):
            if candidate.get("id") == job.get("id"):
                self.jobs.pop(index)
                self.save()
                return True
        return False

    def clear_all(self):
        """Clear pending jobs and history, preserving active jobs and media."""
        self.jobs[:] = [job for job in self.jobs
                       if job.get("state") in {"Running", "Cancelling"}
                       or job.get("id") in self.active]
        self.save()

    def _run(self, job, cancelled):
        process = None
        try:
            flags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
            process = subprocess.Popen([self.python, "-u", str(Path(__file__).resolve()), "--worker"],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                text=True, encoding="utf-8", errors="replace", creationflags=flags,
                start_new_session=os.name != "nt")
            process.stdin.write(json.dumps(job) + "\n")
            process.stdin.close()
            def reader():
                with process.stdout:
                    for line in process.stdout:
                        self.events.put((job["id"], "line", line.rstrip()))
            reader_thread = threading.Thread(target=reader, daemon=True)
            reader_thread.start()
            while process.poll() is None:
                if cancelled.wait(0.2):
                    stop_tree(process, grace=3)
                    break
            code = process.wait()
            reader_thread.join(timeout=3)
            self.events.put((job["id"], "done", (code, cancelled.is_set())))
        except Exception as exc:
            if process is not None:
                try:
                    stop_tree(process, grace=3)
                except (OSError, subprocess.SubprocessError) as cleanup_error:
                    self.events.put((job["id"], "line", "ERROR: Process cleanup: " + str(cleanup_error)))
            self.events.put((job["id"], "line", "ERROR: " + str(exc)))
            self.events.put((job["id"], "done", (1, cancelled.is_set())))

    def tick(self, allow_start=True):
        self.recent_lines = []
        by_id = {j["id"]: j for j in self.jobs}
        changed = False
        # Bound each UI poll, even when FFmpeg emits lots of progress lines.
        for _ in range(500):
            try:
                job_id, kind, value = self.events.get_nowait()
            except Empty:
                break
            job = by_id.get(job_id)
            if job is None:
                continue
            changed = True
            if kind == "done":
                code, cancelled = value
                job["state"] = "Cancelled" if cancelled else "Done" if code == 0 else "Failed"
                job["finished"] = time.time()
                previous_detail = job.get("detail", "")
                job["detail"] = ("Complete" if code == 0 else previous_detail if previous_detail.startswith("ERROR:") else f"Exit {code}; see job log") if not cancelled else "Cancelled; partial files kept"
                if job["state"] == "Done":
                    job["progress"] = 100
                self.active.pop(job_id, None)
                self.save()
            else:
                self.recent_lines.append((job_id, value))
                job.setdefault("logs", []).append(value)
                job["logs"] = job["logs"][-100:]
                if value.startswith("PIXCLIP_OUTPUT="):
                    path = value.split("=", 1)[1]
                    if path not in job.setdefault("outputs", []):
                        job["outputs"].append(path)
                if value.startswith("PIXCLIP_PHASE="):
                    job["detail"] = value.split("=", 1)[1]
                    job["progress"] = None
                if value.startswith("media_duration="):
                    try:
                        job["media_duration"] = float(value.split("=", 1)[1])
                    except ValueError:
                        pass
                elapsed = parse_ffmpeg_progress_seconds(value)
                if elapsed is not None:
                    job["media_elapsed"] = elapsed
                    if job.get("media_duration", 0) > 0:
                        job["progress"] = min(99, 100 * elapsed / job["media_duration"])
                    elif "live" in str(job.get("mode", "")).lower():
                        job["detail"] = f"Live recording | REC {format_elapsed(elapsed)}"
                if value.startswith("speed=") and job.get("media_duration", 0) > 0:
                    try:
                        speed = float(value.split("=", 1)[1].rstrip("x"))
                        if speed > 0:
                            eta = max(0, (job["media_duration"]-job.get("media_elapsed", 0))/speed)
                            job["detail"] = f"Downloading | ETA ~{int(eta)//60:02}:{int(eta)%60:02}"
                    except ValueError:
                        pass
                match = re.search(r"\[download\]\s+(\d+(?:\.\d+)?)%", value)
                if match:
                    job["progress"] = min(99, float(match.group(1)))
                    job["detail"] = value.strip()
                if value.startswith("ERROR:"):
                    job["detail"] = value
        now = time.time()
        if allow_start and not self.paused:
            # Scheduled recordings get first claim on the next free slot.
            waiting = sorted((j for j in self.jobs if j["state"] == "Queued"),
                             key=lambda j: (not bool(j.get("start_at")), j.get("start_at", 0), j["created"]))
            for job in waiting:
                start = job.get("start_at", 0)
                if start > now:
                    continue
                if start and job.get("duration", 0) and now >= start + job["duration"]:
                    job.update(state="Failed", detail="Scheduled window missed; retry to record now")
                    self.save()
                    changed = True
                    continue
                if len(self.active) >= self.limit:
                    continue
                job.update(state="Running", started=now, detail="Starting", progress=None)
                token = threading.Event()
                self.active[job["id"]] = token
                self.save()
                threading.Thread(target=self._run, args=(dict(job), token), daemon=True).start()
                changed = True
        return changed


def worker(job):
    import media_toolkit as media
    import tempfile
    media.configure_output_encoding()
    root = job_output_directory(job)
    reserve = job.get("reserve_gb", 1)
    require_space(root, reserve)
    work = root / ".work"
    work.mkdir(exist_ok=True)
    tempfile.tempdir = str(work)
    quality = job.get("quality", QUALITIES[0])
    cookies_file = job.get("cookies_file")
    if cookies_file is not None and str(cookies_file).strip():
        cookies_path = Path(str(cookies_file)).expanduser()
        if not cookies_path.is_file():
            raise ValueError("Cookies file was not found: " + str(cookies_path))
        cookies_file = str(cookies_path)
    else:
        cookies_file = None
    live = job["mode"] in (media.STREAMLINK_LIVE_MODE, media.LIVE_TS_NVENC_MODE, media.AUTO_LIVE_TS_MODE)
    if live and quality == QUALITIES[-1]:
        raise ValueError("Audio-only is a download mode; choose a video quality for Live recording.")
    deadline = ((job.get("start_at") or time.time()) + job["duration"]
                if live and job.get("duration", 0) else None)
    timed_out = False
    children = set()
    if os.name != "nt":
        def terminate_worker(signum, frame):
            for child in list(children):
                stop_tree(child)
            raise SystemExit(130)
        signal.signal(signal.SIGTERM, terminate_worker)
        signal.signal(signal.SIGINT, terminate_worker)

    def controlled(executable, arguments, working_directory, *, capture=False, display_arguments=None):
        nonlocal timed_out
        args = list(map(str, arguments))
        name = Path(executable).name.lower()
        is_ytdlp = "yt-dlp" in name
        recording = live and not remuxing and ("streamlink" in name or "ffmpeg" in name) and not capture
        if live and not remuxing and deadline and time.time() >= deadline:
            raise TimeoutError("Scheduled Live window ended before capture could start")
        if is_ytdlp:
            # Keep yt-dlp's normal [download] progress output. The worker
            # already discovers completed files by scanning its output folder.
            args = quality_arguments(args, quality)
        if "streamlink" in name:
            args = ["--no-config", *args]
            if quality != QUALITIES[0]:
                h = int(re.search(r"\d+", quality).group())
                args[-1] = ",".join(f"{x}p60,{x}p" for x in (2160, 1440, 1080, 720, 480, 360, 240, 160) if x <= h)
        print("PIXCLIP_PHASE=" + ("Recording / waiting for Live" if recording else "Working"), flush=True)
        # Do not log resolved signed media URLs or request headers.
        print("Running " + name, flush=True)
        process = subprocess.Popen([executable, *args], cwd=str(working_directory),
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding="utf-8", errors="replace",
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0), start_new_session=os.name != "nt")
        children.add(process)
        lines = []
        def reader():
            with process.stdout:
                for line in process.stdout:
                    if capture:
                        lines.append(line)
                    else:
                        print(line.rstrip(), flush=True)
        thread = threading.Thread(target=reader, daemon=True)
        thread.start()
        failure = None
        started = time.monotonic()
        while process.poll() is None:
            try:
                require_space(root, reserve)
                if capture and time.monotonic() - started > 90:
                    raise TimeoutError("Metadata request timed out after 90 seconds")
                if live and capture and deadline and time.time() >= deadline:
                    raise TimeoutError("Scheduled Live window ended while resolving metadata")
                if recording and deadline and time.time() >= deadline:
                    timed_out = True
                    break
            except (OSError, TimeoutError) as exc:
                failure = exc
                break
            time.sleep(0.5)
        if process.poll() is None:
            stop_tree(process)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
        code = process.wait()
        if os.name != "nt":
            stop_tree(process)
        children.discard(process)
        thread.join(timeout=3)
        if failure:
            raise failure
        return (0 if timed_out and recording else code), "".join(lines)

    remuxing = False
    media.run_command = controlled
    # Kick's fallback exposes an HLS master rather than a yt-dlp format list.
    original_kick = media.request_kick_vod_playback
    def kick_with_quality(video_id):
        payload = original_kick(video_id)
        if payload and quality not in (QUALITIES[0], QUALITIES[-1]):
            from urllib.request import Request, urlopen
            from urllib.parse import urljoin
            url = payload.get("playback_url", {}).get("vod")
            if url:
                with urlopen(Request(url, headers={"User-Agent": media.KICK_USER_AGENT,
                                                   "Referer": "https://kick.com/"}), timeout=20) as response:
                    playlist = response.read(2_000_000).decode("utf-8")
                maximum = int(re.search(r"\d+", quality).group())
                variants = []
                rows = playlist.splitlines()
                for index, row in enumerate(rows[:-1]):
                    match = re.search(r"RESOLUTION=\d+x(\d+)", row)
                    if row.startswith("#EXT-X-STREAM-INF:") and match and int(match.group(1)) <= maximum:
                        if 'AUDIO=' in row:
                            raise ValueError("This Kick stream uses separate audio. Select Best available to preserve audio.")
                        variants.append((int(match.group(1)), urljoin(url, rows[index+1].strip())))
                if not variants:
                    raise ValueError("Requested Kick resolution unavailable. Select Best available.")
                payload["playback_url"]["vod"] = max(variants)[1]
        return payload
    media.request_kick_vod_playback = kick_with_quality
    mode = job["mode"]
    cookie_kwargs = {"cookies_file": cookies_file} if cookies_file else {}
    if quality == QUALITIES[-1]:
        mode = "Playlist - MP3 best quality" if mode.startswith("Playlist") else "MP3 - Best quality"
    if live:
        if mode == media.STREAMLINK_LIVE_MODE:
            run = media.run_cli_streamlink
        elif mode == media.AUTO_LIVE_TS_MODE:
            run = media.run_cli_live_ts_auto
        else:
            run = media.run_cli_live_ts_nvenc
        code = 1
        for attempt in range(6):
            if deadline and time.time() >= deadline:
                break
            code = run(job["url"], root, **cookie_kwargs)
            if timed_out or (code == 0 and not deadline):
                break
            if attempt < 5:
                print(f"PIXCLIP_PHASE=Reconnecting ({attempt+1}/5); existing segments kept", flush=True)
                time.sleep(min(5, max(0, deadline-time.time())) if deadline else 5)
    elif mode == media.TS_DOWNLOAD_MODE:
        code = media.run_cli_ts_download(job["url"], root, **cookie_kwargs)
    else:
        code = media.run_cli_download(mode, job["url"], root, **cookie_kwargs)
    outputs = [p for p in root.rglob("*") if p.is_file() and ".work" not in p.parts
               and p.suffix.lower() in {".mp4", ".ts", ".mkv", ".mp3", ".webm", ".m4a"} and p.stat().st_size]
    for output in outputs:
        print("PIXCLIP_OUTPUT=" + str(output), flush=True)
    if code != 0:
        return code
    if not outputs:
        print("ERROR: No media file was created. The source may be offline or already archived.", flush=True)
        return 1
    if live and not all(media.get_media_duration(p) for p in outputs):
        raise ValueError("Recording is empty or unreadable. Partial files were kept.")
    if job.get("auto_mp4"):
        remuxing = True
        for source in outputs:
            if source.suffix.lower() != ".ts":
                continue
            require_space(root, reserve, int(source.stat().st_size * 1.1))
            target = source.with_suffix(".mp4")
            temporary = source.with_name(source.stem + ".remuxing.mp4")
            print("PIXCLIP_PHASE=TS to MP4 (copying; original TS kept)", flush=True)
            ffmpeg = media.find_tool("ffmpeg")
            code, _ = controlled(ffmpeg, ["-hide_banner", "-n", "-i", str(source), "-map", "0:v:0?",
                "-map", "0:a:0?", "-c", "copy", "-movflags", "+faststart", str(temporary)], root)
            if code or not temporary.exists() or not temporary.stat().st_size:
                raise ValueError("MP4 remux failed; original TS kept. Try Compatible H.264 conversion.")
            source_duration = media.get_media_duration(source)
            result_duration = media.get_media_duration(temporary)
            if not source_duration or not result_duration or abs(source_duration-result_duration) > max(2, source_duration * .02):
                raise ValueError("MP4 duration check failed; original TS and temporary MP4 kept.")
            if target.exists():
                raise FileExistsError("MP4 already exists; not overwritten: " + str(target))
            temporary.rename(target)
            print("PIXCLIP_OUTPUT=" + str(target), flush=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(worker(json.loads(sys.stdin.readline())))
    except Exception as exc:
        print("ERROR: " + str(exc), flush=True)
        raise SystemExit(1)

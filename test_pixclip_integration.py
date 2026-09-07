"""Opt-in real FFmpeg/yt-dlp/Tk tests, serving only a synthetic localhost clip.

Set PIXCLIP_INTEGRATION=1, then python -m unittest -v test_pixclip_integration.
No public website downloads and no Live recordings are made.
"""
import functools
import contextlib
import io
import http.server
import os
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import patch

import media_toolkit as media
from pixclip_jobs import JobQueue, TERMINAL, worker


@unittest.skipUnless(os.environ.get("PIXCLIP_INTEGRATION") == "1", "Opt-in media tool integration")
class IntegrationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.source = self.directory / "sample.mp4"
        self.ffmpeg = media.find_tool("ffmpeg")
        if not self.ffmpeg or not media.find_tool("yt-dlp"):
            self.skipTest("FFmpeg and yt-dlp required")
        subprocess.run([self.ffmpeg, "-hide_banner", "-loglevel", "error", "-f", "lavfi",
            "-i", "testsrc=size=320x180:rate=25", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000",
            "-t", "3", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", str(self.source)],
            check=True, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))

    def test_real_queue_ts_remux_and_history(self):
        class Handler(http.server.SimpleHTTPRequestHandler):
            def log_message(self, *args):
                pass
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Handler, directory=str(self.directory)))
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        queue = JobQueue(sys.executable, self.directory / "history.json")
        queue.limit = 2
        url = f"http://127.0.0.1:{server.server_port}/sample.mp4"
        jobs = [queue.add(url, media.TS_DOWNLOAD_MODE, self.directory / "output", auto_mp4=True),
                queue.add(url, "MP4 - Best quality", self.directory / "output")]
        deadline = time.monotonic()+100
        while time.monotonic()<deadline and not all(j["state"] in TERMINAL for j in jobs):
            queue.tick()
            time.sleep(.05)
        try:
            for job in jobs:
                self.assertEqual(job["state"], "Done", "\n".join(job.get("logs", [])))
                for output in job["outputs"]:
                    self.assertGreater(media.get_media_duration(Path(output)) or 0, 2.5)
            outputs = [Path(p) for p in jobs[0]["outputs"]]
            self.assertTrue(any(p.suffix == ".ts" for p in outputs))
            self.assertTrue(any(p.suffix == ".mp4" for p in outputs))
            self.assertEqual(len(JobQueue(sys.executable, queue.path).jobs), 2)
        finally:
            for job in jobs:
                queue.cancel(job)
            end = time.monotonic()+10
            while queue.active and time.monotonic()<end:
                queue.tick(allow_start=False)
                time.sleep(.1)

    def test_tk_preview_and_small_layout(self):
        import tkinter as tk
        from pixclip_ui import ClipPreview
        root = tk.Tk()
        self.addCleanup(root.destroy)
        with patch("pixclip_jobs.state_path", return_value=self.directory / "gui-history.json"):
            app = media.MediaToolkitApp(root)
        self.addCleanup(lambda: setattr(app, "workflow_closing", True))
        self.assertIn("SYSTEM", app.dependency_label.cget("text"))
        self.assertIn("HARDWARE", app.hardware_label.cget("text"))
        self.assertIn("GPU:", app.hardware_label.cget("text"))
        self.assertIn("CPU:", app.hardware_label.cget("text"))
        for size in ("1000x780", "740x680"):
            root.geometry(size)
            root.update()
            self.assertGreater(app.log_box.winfo_height(), 25)
            app.show_queue()
            root.update()
        app.show_help_dialog()
        root.update()
        self.assertTrue(app.help_window.winfo_exists())
        help_text = ""
        def collect(widget):
            nonlocal help_text
            for child in widget.winfo_children():
                if isinstance(child, tk.Text):
                    help_text += child.get("1.0", "end")
                collect(child)
        collect(app.help_window)
        self.assertIn("เริ่มใช้งาน", help_text)
        self.assertIn("TS → MP4", help_text)
        app.help_render("en")
        root.update()
        english_help = app.help_text_box.get("1.0", "end")
        self.assertIn("Getting started", english_help)
        self.assertEqual(app.help_language_var.get(), "en")
        app.help_render("th")
        root.update()
        self.assertIn("เริ่มใช้งาน", app.help_text_box.get("1.0", "end"))
        app.help_window.destroy()
        window = tk.Toplevel(root)
        source, start, end, duration = [tk.StringVar(master=root) for _ in range(4)]
        preview = ClipPreview(app, window, source, start, end, duration)
        before = time.monotonic()
        source.set(str(self.source))
        self.assertLess(time.monotonic()-before, .1, "Source change must not block Tk")
        deadline = time.monotonic()+10
        while not hasattr(preview, "image") and time.monotonic()<deadline:
            root.update()
            time.sleep(.03)
        self.assertTrue(hasattr(preview, "image"))
        self.assertGreater(preview.image.width(), 100)
        self.assertGreater(preview.duration, 2.5)
        self.assertIn("Duration:", duration.get())
        preview.slider.set(1.5)
        root.update()
        window.destroy()

    def test_timed_live_finalizes_and_remuxes_after_deadline(self):
        job = dict(id="timedtest", folder=str(self.directory / "recordings"), url="http://localhost/test",
                   mode=media.STREAMLINK_LIVE_MODE, duration=2, reserve_gb=.25, auto_mp4=True)
        def synthetic_live(url, folder):
            return media.run_command(self.ffmpeg, ["-hide_banner", "-loglevel", "error", "-re",
                "-stream_loop", "-1", "-i", str(self.source), "-c", "copy", "-flush_packets", "1",
                "-f", "mpegts", str(folder / "live.ts")], folder)[0]
        with patch.object(media, "run_cli_streamlink", side_effect=synthetic_live), \
             patch.object(media, "run_command", media.run_command), \
             patch.object(media, "request_kick_vod_playback", media.request_kick_vod_playback), \
             patch.object(tempfile, "tempdir", tempfile.tempdir), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(worker(job), 0)
        root = Path(job["folder"]) / "PixClip-timedtest"
        self.assertTrue((root / "live.ts").is_file())
        self.assertTrue((root / "live.mp4").is_file())
        self.assertGreater(media.get_media_duration(root / "live.mp4") or 0, 0)

    def test_live_metadata_cannot_exceed_slot(self):
        job = dict(id="metatest", folder=str(self.directory / "recordings"), url="http://localhost/test",
                   mode=media.LIVE_TS_NVENC_MODE, duration=.1, reserve_gb=.25)
        def metadata(url, folder):
            return media.run_command(sys.executable, ["-c", "import time; time.sleep(4)"], folder, capture=True)[0]
        with patch.object(media, "run_cli_live_ts_nvenc", side_effect=metadata), \
             patch.object(media, "run_command", media.run_command), \
             patch.object(media, "request_kick_vod_playback", media.request_kick_vod_playback), \
             patch.object(tempfile, "tempdir", tempfile.tempdir), contextlib.redirect_stdout(io.StringIO()):
            started = time.monotonic()
            with self.assertRaises(TimeoutError):
                worker(job)
            self.assertLess(time.monotonic()-started, 2)

    def test_live_reconnect_keeps_previous_segment(self):
        job = dict(id="retrytest", folder=str(self.directory / "recordings"), url="http://localhost/test",
                   mode=media.STREAMLINK_LIVE_MODE, duration=0, reserve_gb=.25)
        calls = []
        def reconnect(url, folder):
            calls.append(1)
            target = folder / f"segment{len(calls)}.ts"
            subprocess.run([self.ffmpeg, "-hide_banner", "-loglevel", "error", "-i", str(self.source),
                "-c", "copy", "-f", "mpegts", str(target)], check=True,
                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
            return 1 if len(calls) == 1 else 0
        with patch.object(media, "run_cli_streamlink", side_effect=reconnect), \
             patch.object(media, "run_command", media.run_command), \
             patch.object(media, "request_kick_vod_playback", media.request_kick_vod_playback), \
             patch.object(tempfile, "tempdir", tempfile.tempdir), \
             patch("pixclip_jobs.time.sleep"), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(worker(job), 0)
        root = Path(job["folder"]) / "PixClip-retrytest"
        self.assertEqual(len(list(root.glob("*.ts"))), 2)
        self.assertEqual(len(calls), 2)


if __name__ == "__main__":
    unittest.main()

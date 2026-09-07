"""Offline regression tests: python -m unittest -v test_pixclip_jobs."""
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch
from collections import namedtuple

import media_toolkit as media
from pixclip_jobs import JobQueue, QUALITIES, GIB, load_settings, require_space, quality_arguments, save_settings, stop_tree


class QueueTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / "jobs.json"
        self.queue = JobQueue(sys.executable, self.path)

    def add(self, **kwargs):
        return self.queue.add("https://example.com/video", "MP4 - Best quality", self.temp.name, **kwargs)

    def test_persist_settings_and_retry(self):
        job = self.add(quality=QUALITIES[2], auto_mp4=True, duration=60)
        job["state"] = "Failed"
        self.queue.limit = 3
        self.queue.save()
        restored = JobQueue(sys.executable, self.path)
        self.assertEqual(restored.limit, 3)
        retried = restored.retry(restored.jobs[0])
        self.assertNotEqual(job["id"], retried["id"])
        self.assertEqual(retried["quality"], QUALITIES[2])
        self.assertEqual(retried["start_at"], 0)

    def test_remember_output_folder_settings(self):
        settings = Path(self.temp.name) / "settings.json"
        save_settings({"output_directory": self.temp.name}, settings)
        self.assertEqual(load_settings(settings)["output_directory"], self.temp.name)
        settings.write_text("invalid", encoding="utf-8")
        self.assertEqual(load_settings(settings), {})

    def test_local_edit_history_preserves_retry_inputs(self):
        concat = Path(self.temp.name) / "concat.txt"
        concat.write_text("file 'one.mp4'\n", encoding="utf-8")
        job = self.queue.record_local("ffmpeg", ["-i", str(concat), "joined.mp4"], self.temp.name, "Joining", [concat])
        restored = JobQueue(sys.executable, self.path).jobs[0]
        self.assertEqual(restored["kind"], "local")
        self.assertEqual(restored["state"], "Interrupted")
        self.assertEqual(restored["temporary"][str(concat)], "file 'one.mp4'\n")

    def test_interrupted_is_not_auto_started(self):
        job = self.add()
        job["state"] = "Running"
        self.queue.save()
        restored = JobQueue(sys.executable, self.path)
        restored.tick()
        self.assertEqual(restored.jobs[0]["state"], "Interrupted")
        self.assertFalse(restored.active)

    def test_corrupt_history_preserved(self):
        self.path.write_text("broken", encoding="utf-8")
        restored = JobQueue(sys.executable, self.path)
        restored.save()
        self.assertEqual(self.path.read_text(), "broken")
        self.assertTrue(restored.warning)

    @patch("threading.Thread.start")
    def test_concurrency_pause_and_schedule(self, start):
        self.queue.limit = 2
        future = self.add(start_at=time.time()+3600, duration=60)
        now1, now2, now3 = self.add(), self.add(), self.add()
        self.queue.paused = True
        self.queue.tick()
        self.assertEqual(start.call_count, 0)
        self.queue.paused = False
        self.queue.tick()
        self.assertEqual(start.call_count, 2)
        self.assertEqual(future["state"], "Queued")
        self.assertEqual(now3["state"], "Queued")
        self.queue.events.put((now1["id"], "done", (0, False)))
        self.queue.tick()
        self.assertEqual(now1["state"], "Done")
        self.assertEqual(now3["state"], "Running")

    @patch("threading.Thread.start")
    def test_expired_schedule_and_cancel_pending(self, start):
        expired = self.add(start_at=time.time()-120, duration=30)
        pending = self.add()
        self.queue.cancel(pending)
        self.queue.tick()
        self.assertEqual(expired["state"], "Failed")
        self.assertEqual(pending["state"], "Cancelled")
        self.assertEqual(start.call_count, 0)

    @patch("threading.Thread.start")
    def test_cancel_running(self, start):
        job = self.add()
        self.queue.tick()
        self.queue.cancel(job)
        self.assertTrue(self.queue.active[job["id"]].is_set())
        self.queue.events.put((job["id"], "done", (1, True)))
        self.queue.tick()
        self.assertEqual(job["state"], "Cancelled")

    def test_progress_and_output_markers(self):
        job = self.add()
        for line in ("media_duration=100", "out_time_us=50000000", "speed=2x", "PIXCLIP_OUTPUT=/tmp/test.mp4"):
            self.queue.events.put((job["id"], "line", line))
        self.queue.tick(allow_start=False)
        self.assertEqual(job["progress"], 50)
        self.assertIn("00:25", job["detail"])
        self.assertEqual(job["outputs"], ["/tmp/test.mp4"])

    def test_input_validation(self):
        for url in ("file:///etc/passwd", "http://", "abc"):
            with self.assertRaises(ValueError):
                self.queue.add(url, "MP4", self.temp.name)
        with self.assertRaises(ValueError):
            self.add(duration=float("nan"))

    def test_space_reserve_and_remux_budget(self):
        usage = namedtuple("usage", "total used free")(10*GIB, 8*GIB, 2*GIB)
        with patch("shutil.disk_usage", return_value=usage):
            require_space(self.temp.name, 1)
            with self.assertRaises(OSError):
                require_space(self.temp.name, 1, 2*GIB)
        with self.assertRaises(ValueError):
            require_space(self.temp.name, float("nan"))

    def test_quality_cap_and_audio(self):
        args = ["-f", "bv*+ba/b", "--", "https://example.com"]
        self.assertIn("height<=720", quality_arguments(args, QUALITIES[3])[1])
        self.assertEqual(args[1], "bv*+ba/b")
        self.assertEqual(quality_arguments(["-f", "ba/b"], QUALITIES[3]), ["-f", "ba/b"])
        self.assertEqual(quality_arguments(["-f", "best", "-g"], QUALITIES[3])[1], "b[height<=720]")

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux process-group regression")
    def test_stop_tree_kills_child_that_ignores_term(self):
        child_code = "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); print('ready',flush=True); time.sleep(30)"
        parent_code = ("import subprocess,sys,time; "
            f"c=subprocess.Popen([sys.executable,'-c',{child_code!r}],stdout=subprocess.PIPE,text=True); "
            "c.stdout.readline(); print(c.pid,flush=True); time.sleep(30)")
        process = subprocess.Popen([sys.executable, "-c", parent_code], stdout=subprocess.PIPE,
                                   text=True, start_new_session=True)
        try:
            pid = int(process.stdout.readline())
            stop_tree(process)
            deadline = time.monotonic()+2
            while time.monotonic()<deadline:
                status = Path(f"/proc/{pid}/status")
                if not status.exists() or "State:\tZ" in status.read_text():
                    break
                time.sleep(.02)
            else:
                self.fail("Descendant still alive")
        finally:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
            process.stdout.close()


class InstallerTests(unittest.TestCase):
    def test_linux_tkinter_skips_when_ready(self):
        with patch.object(media.sys, "platform", "linux"), \
             patch.object(media, "tkinter_available", return_value=True), \
             patch.object(media, "run_command") as run:
            self.assertEqual(media.ensure_linux_tkinter(), 0)
        run.assert_not_called()

    def test_linux_tkinter_uses_apt_and_sudo(self):
        with patch.object(media.sys, "platform", "linux"), \
             patch.object(media, "tkinter_available", side_effect=[False, True]), \
             patch.object(media.shutil, "which", side_effect=lambda name: "/usr/bin/apt-get" if name == "apt-get" else "/usr/bin/sudo"), \
             patch.object(media.os, "geteuid", return_value=1000), \
             patch.object(media, "run_command", return_value=(0, "")) as run:
            self.assertEqual(media.ensure_linux_tkinter(), 0)
        self.assertEqual(run.call_count, 2)
        self.assertEqual(run.call_args_list[0].args, ("sudo", ["/usr/bin/apt-get", "update"], media.BASE_DIR))
        self.assertEqual(run.call_args_list[1].args, ("sudo", ["/usr/bin/apt-get", "install", "-y", "python3-tk"], media.BASE_DIR))


class HardwareTests(unittest.TestCase):
    def test_gpu_vendor_classification(self):
        self.assertEqual(media.classify_gpu_name("NVIDIA GeForce RTX 4070"), "NVIDIA")
        self.assertEqual(media.classify_gpu_name("AMD Radeon RX 7800 XT"), "AMD")
        self.assertEqual(media.classify_gpu_name("Intel Arc A770"), "Intel")

    def test_auto_selects_nvidia_encoder(self):
        def available(_ffmpeg, encoder):
            return encoder == "h264_nvenc"
        with patch.object(media, "has_encoder", side_effect=available):
            selected = media.select_live_encoder("ffmpeg", "NVIDIA", "NVIDIA GeForce RTX 4070")
        self.assertEqual(selected["encoder"], "h264_nvenc")
        self.assertEqual(selected["label"], "NVIDIA NVENC")

    def test_auto_selects_amd_amf_on_windows(self):
        def available(_ffmpeg, encoder):
            return encoder == "h264_amf"
        with patch.object(media, "IS_WINDOWS", True), patch.object(media, "has_encoder", side_effect=available):
            selected = media.select_live_encoder("ffmpeg", "AMD", "AMD Radeon RX 7800 XT")
        self.assertEqual(selected["encoder"], "h264_amf")
        self.assertEqual(selected["label"], "AMD AMF")

    def test_auto_selects_amd_vaapi_on_linux(self):
        def available(_ffmpeg, encoder):
            return encoder == "h264_vaapi"
        with patch.object(media, "IS_WINDOWS", False),              patch.object(media, "find_vaapi_device", return_value="/dev/dri/renderD128"),              patch.object(media, "has_encoder", side_effect=available):
            selected = media.select_live_encoder("ffmpeg", "AMD", "AMD Radeon RX 6600")
        self.assertEqual(selected["encoder"], "h264_vaapi")
        self.assertEqual(selected["vaapi_device"], "/dev/dri/renderD128")

    def test_auto_falls_back_to_cpu(self):
        with patch.object(media, "has_encoder", return_value=False):
            selected = media.select_live_encoder("ffmpeg", "NVIDIA", "NVIDIA GeForce GTX 1060")
        self.assertEqual(selected["encoder"], "libx264")
        self.assertEqual(selected["label"], "CPU (libx264)")

    def test_auto_live_modes_build_expected_video_encoder(self):
        amd_args = media.build_live_ts_arguments("https://stream.example/live.m3u8", Path("out.ts"), "h264_amf")
        cpu_args = media.build_live_ts_arguments("https://stream.example/live.m3u8", Path("out.ts"), "libx264")
        self.assertIn("h264_amf", amd_args)
        self.assertIn("libx264", cpu_args)


if __name__ == "__main__":
    unittest.main()

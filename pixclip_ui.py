"""Queue controls and frame-based trim preview, using PixClip's existing theme."""
import base64
import math
import subprocess
import threading
import time
from datetime import datetime
from pathlib import Path
from queue import Queue, Empty

from pixclip_jobs import JobQueue, QUALITIES, TERMINAL, require_space, GIB


class WorkflowUI:
    def setup_workflow(self, python):
        tk, ttk = self.tk, self.ttk
        self.jobs = JobQueue(python)
        self.local_history = None
        self.queue_window = None
        self.queue_tree = None
        self.workflow_closing = False
        self.last_job_state = "Ready"
        self.mascot_frame = 0
        self.workflow_active = False
        self.disk_tick = 0
        self.queue_progress_visible = False
        self.quality_var = tk.StringVar(value=QUALITIES[0])
        self.auto_mp4_var = tk.BooleanVar(value=True)
        self.reserve_var = tk.DoubleVar(value=1)
        content = self.download_mode_box.master
        for widget in content.grid_slaves():
            info = widget.grid_info()
            if int(info["row"]) >= 4:
                widget.grid_configure(row=int(info["row"]) + 1)
        content.grid_rowconfigure(7, weight=0)
        content.grid_rowconfigure(8, weight=1)
        options = tk.Frame(content, bg=self.COLORS["background"])
        options.grid(row=4, column=0, columnspan=3, sticky="ew", pady=(2, 6))
        ttk.Combobox(options, textvariable=self.quality_var, values=QUALITIES,
            state="readonly", style="Dark.TCombobox", width=20).pack(side="left")
        tk.Checkbutton(options, text="TS -> MP4 (keep TS)", variable=self.auto_mp4_var,
            bg=self.COLORS["background"], fg=self.COLORS["text"], selectcolor=self.COLORS["input"],
            activebackground=self.COLORS["background"], activeforeground=self.COLORS["text"]).pack(side="left", padx=8)
        self.cookies_var = tk.StringVar(value=str(self.cookies_file) if self.cookies_file else "")
        self.cookies_display_var = tk.StringVar(
            value=("Cookies: " + self.cookies_file.name) if self.cookies_file else "Cookies: OFF"
        )
        tk.Label(
            options,
            textvariable=self.cookies_display_var,
            width=18,
            anchor="w",
            bg=self.COLORS["background"],
            fg=self.COLORS["muted"],
            font=(self.mono_font, 8),
        ).pack(side="left", padx=(2, 4))
        self.choose_cookies_button = self._button(options, "Choose", self.choose_cookies)
        self.choose_cookies_button.configure(image="", font=(self.mono_font, 8), padx=4, pady=2, borderwidth=1)
        self.choose_cookies_button.pack(side="left", padx=(0, 3))
        self.clear_cookies_button = self._button(options, "Clear", self.clear_cookies)
        self.clear_cookies_button.configure(image="", font=(self.mono_font, 8), padx=4, pady=2, borderwidth=1)
        self.clear_cookies_button.pack(side="left")
        self._button(options, "Queue / History", self.show_queue).pack(side="right")
        self.workflow_label = tk.Label(content, text="Queue ready | Disk check enabled", anchor="w",
            bg=self.COLORS["background"], fg=self.COLORS["muted"], font=(self.mono_font, 9))
        self.workflow_label.grid(row=10, column=0, columnspan=3, sticky="ew")
        if self.jobs.warning:
            self.append_log("WARNING: " + self.jobs.warning)
        self.workflow_tick()

    def enqueue_download(self):
        from tkinter import messagebox
        try:
            url = self.url_var.get().strip()
            require_space(self.output_directory, self.reserve_var.get())
            self.jobs.add(url, self.download_mode_var.get(), self.output_directory,
                self.quality_var.get(), self.auto_mp4_var.get(), reserve_gb=self.reserve_var.get(),
                cookies_file=self.cookies_file)
            self.show_queue()
        except (OSError, ValueError, self.tk.TclError) as exc:
            messagebox.showerror("Add download", str(exc), parent=self.root)

    def workflow_tick(self):
        if self.workflow_closing:
            return
        changed = self.jobs.tick(allow_start=self.current_runner is None)
        for job_id, line in self.jobs.recent_lines:
            self.append_log(f"[{job_id[:6]}] {line}")
        if self.jobs.recent_lines:
            # Bound the visible log for long-running Live sessions.
            count = int(self.log_box.index("end-1c").split(".")[0])
            if count > 2500:
                self.log_box.configure(state="normal")
                self.log_box.delete("1.0", f"{count-2000}.0")
                self.log_box.configure(state="disabled")
        active = [j for j in self.jobs.jobs if j["state"] in ("Running", "Cancelling")]
        queued = sum(j["state"] == "Queued" for j in self.jobs.jobs)
        if active:
            self.last_job_state = "Working"
            self.workflow_active = True
        elif self.workflow_active:
            self.workflow_active = False
            completed = [j for j in self.jobs.jobs if j.get("finished")]
            last = max(completed, key=lambda j: j["finished"]) if completed else None
            self.last_job_state = "Done" if last and last["state"] == "Done" else "Check log"
        self.workflow_label.configure(text=f"{len(active)} running | {queued} queued | {self.last_job_state}" + (" | Queue paused" if self.jobs.paused else ""))
        if self.current_runner is None:
            for button in (self.install_button, self.update_button, self.convert_button, self.clip_button, self.join_button):
                button.configure(state="disabled" if active else "normal")
            if active:
                first = active[0]
                self.status_label.configure(text=f"Queue: {len(active)} running | {first['detail'][:100]}", fg=self.COLORS["info"])
                self.progress.pack(side="bottom", fill="x", pady=(3, 0))
                progress = first.get("progress")
                self.progress.stop()
                if progress is not None:
                    self.progress.configure(mode="determinate", value=progress, maximum=100)
                else:
                    self.progress.configure(mode="indeterminate")
                    self.progress.start(100)
                self.queue_progress_visible = True
            elif self.queue_progress_visible:
                self.progress.stop()
                self.progress.pack_forget()
                self.status_label.configure(text="Queue finished. See Queue / History for files and logs.", fg=self.COLORS["success"] if self.last_job_state == "Done" else self.COLORS["warning"])
                self.queue_progress_visible = False
        self.disk_tick += 1
        if self.disk_tick % 20 == 0:
            try:
                directory = getattr(self, "local_working_directory", self.output_directory) if self.current_runner else self.output_directory
                free = require_space(directory, self.reserve_var.get())
                self.disk_message = f" | {free/GIB:.1f} GiB free"
            except (OSError, ValueError, self.tk.TclError) as exc:
                self.disk_message = " | LOW DISK SPACE"
                if self.current_runner:
                    self.append_log("ERROR: " + str(exc))
                    self.cancel_task()
        self.workflow_label.configure(text=self.workflow_label.cget("text") + getattr(self, "disk_message", ""))
        self.mascot_frame += 1
        if hasattr(self, "mascot_canvas"):
            state = "Working" if self.current_runner is not None else self.last_job_state
            dy = int(math.sin(self.mascot_frame * .7) * 3) if state == "Working" else 0
            dx = (2 if self.mascot_frame % 2 else -2) if state == "Check log" and self.mascot_frame % 20 < 4 else 0
            self.mascot_canvas.coords("mascot", 40 + dx, 40 + dy)
            # One 250 ms blink every four seconds, including while idle.
            eyelid_state = "normal" if self.mascot_frame % 16 == 0 else "hidden"
            for left, right, cover, lid in self.mascot_eyelids:
                self.mascot_canvas.coords(cover, left + dx, 34 + dy, right + dx, 47 + dy)
                self.mascot_canvas.coords(lid, left + 1 + dx, 40 + dy, right - 1 + dx, 42 + dy)
                self.mascot_canvas.itemconfigure(cover, state=eyelid_state)
                self.mascot_canvas.itemconfigure(lid, state=eyelid_state)
            self.mascot_canvas.itemconfigure("mood", text={"Working": "REC / ...", "Done": "OK!", "Check log": "! CHECK", "Ready": "READY"}.get(state, "READY"),
                fill=self.COLORS["warning"] if state == "Check log" else self.COLORS["success"])
        if self.queue_window is not None and self.queue_window.winfo_exists():
            self.refresh_queue()
        if self.jobs.warning:
            self.workflow_label.configure(text=self.jobs.warning, fg=self.COLORS["warning"])
        self.root.after(250, self.workflow_tick)

    def show_queue(self):
        tk, ttk = self.tk, self.ttk
        from tkinter import messagebox
        if self.queue_window is not None and self.queue_window.winfo_exists():
            self.queue_window.lift()
            return
        window = self.queue_window = tk.Toplevel(self.root)
        window.title("PixClip - Queue / History / Schedule")
        window.geometry("1040x720")
        window.minsize(780, 650)
        window.configure(bg=self.COLORS["background"])
        def label(parent, text):
            return tk.Label(parent, text=text, bg=self.COLORS["background"], fg=self.COLORS["text"], font=(self.mono_font, 9), anchor="w")
        top = tk.Frame(window, bg=self.COLORS["background"])
        top.pack(fill="x", padx=16, pady=12)
        label(top, "Parallel downloads (1-4):").pack(side="left")
        limit_var = tk.IntVar(value=self.jobs.limit)
        def settings():
            try:
                self.jobs.limit = max(1, min(4, limit_var.get()))
                self.jobs.save()
            except tk.TclError:
                pass
        spin = tk.Spinbox(top, from_=1, to=4, textvariable=limit_var, width=3, command=settings)
        spin.pack(side="left", padx=8)
        spin.bind("<FocusOut>", lambda e: settings())
        spin.bind("<Return>", lambda e: settings())
        def pause():
            self.jobs.paused = not self.jobs.paused
            self.jobs.save()
            pause_button.configure(text="[ Resume queue ]" if self.jobs.paused else "[ Pause queue ]",
                                   image=self.pixel_icon("play" if self.jobs.paused else "pause"))
        pause_button = self._button(top, "Resume queue" if self.jobs.paused else "Pause queue", pause,
                                   icon="play" if self.jobs.paused else "pause")
        pause_button.pack(side="left", padx=8)
        self.history_only = tk.BooleanVar(value=False)
        tk.Checkbutton(top, text="History only", variable=self.history_only, command=self.refresh_queue,
            bg=self.COLORS["background"], fg=self.COLORS["text"], selectcolor=self.COLORS["input"]).pack(side="right")
        table = tk.Frame(window)
        table.pack(fill="both", expand=True, padx=16)
        style = ttk.Style(window)
        style.configure("PixClip.Treeview", background=self.COLORS["input"], fieldbackground=self.COLORS["input"], foreground=self.COLORS["text"], rowheight=27, font=(self.mono_font, 9))
        style.configure("PixClip.Treeview.Heading", background=self.COLORS["surface"], foreground=self.COLORS["text"], font=(self.mono_font, 9, "bold"))
        style.map("PixClip.Treeview", background=[("selected", self.COLORS["primary"])] )
        self.queue_tree = ttk.Treeview(table, columns=("state", "progress", "url", "detail"), show="headings", selectmode="browse", style="PixClip.Treeview")
        for name, width in (("state", 90), ("progress", 75), ("url", 280), ("detail", 360)):
            self.queue_tree.heading(name, text=name.title())
            self.queue_tree.column(name, width=width, minwidth=50)
        self.queue_tree.pack(side="left", fill="both", expand=True)
        scroll = ttk.Scrollbar(table, orient="vertical", command=self.queue_tree.yview)
        scroll.pack(side="right", fill="y")
        self.queue_tree.configure(yscrollcommand=scroll.set)
        self.queue_tree.bind("<Double-1>", lambda event: action("log"))
        controls = tk.Frame(window, bg=self.COLORS["background"])
        controls.pack(fill="x", padx=16, pady=8)
        def action(name):
            selection = self.queue_tree.selection()
            job = next((j for j in self.jobs.jobs if selection and j["id"] == selection[0]), None)
            if not job:
                return
            try:
                if name == "cancel":
                    if job.get("kind") == "local" and job["state"] == "Running":
                        self.cancel_task()
                    else:
                        self.jobs.cancel(job)
                elif name == "delete":
                    if not messagebox.askyesno(
                        "Delete job",
                        "Remove this job from Queue / History? Media files will not be deleted.",
                        parent=window,
                    ):
                        return
                    self.jobs.delete(job)
                    self.refresh_queue()
                elif name == "retry":
                    if job.get("kind") == "local":
                        self.retry_local(job)
                    else:
                        self.jobs.retry(job)
                elif name == "folder":
                    from media_toolkit import open_folder
                    directory = Path(job["folder"]) if job.get("kind") == "local" else Path(job["folder"]) / ("PixClip-" + job["id"][:12])
                    open_folder(directory if directory.exists() else Path(job["folder"]))
                elif name == "file":
                    outputs = [Path(p) for p in job.get("outputs", []) if Path(p).is_file()]
                    if not outputs:
                        raise ValueError("No completed file is available. Use Open folder to inspect partial files.")
                    self.show_outputs(outputs)
                else:
                    log = tk.Toplevel(window)
                    log.title("PixClip - " + job["state"])
                    box = tk.Text(log, bg=self.COLORS["log_background"], fg=self.COLORS["log_text"], wrap="word")
                    box.pack(fill="both", expand=True)
                    box.insert("end", job["detail"] + "\n" + "\n".join(job.get("logs", [])))
                    box.configure(state="disabled")
            except (OSError, ValueError) as exc:
                messagebox.showerror("Queue", str(exc), parent=window)
        for text, name in (("Cancel", "cancel"), ("Delete", "delete"), ("Retry", "retry"), ("Files", "file"), ("Open folder", "folder"), ("Job log", "log")):
            self._button(controls, text, lambda n=name: action(n),
                         icon="stop" if name == "cancel" else name).pack(side="left", padx=(0, 6))
        def clear_all():
            if messagebox.askyesno("Clear all",
                "Clear all pending jobs (including scheduled Live) and history?\n"
                "Running jobs and media files will be kept.", parent=window):
                self.jobs.clear_all()
                self.refresh_queue()
        self._button(controls, "Clear all", clear_all, icon="clear").pack(side="left", padx=(0, 6))
        form = tk.Frame(window, bg=self.COLORS["background"])
        form.pack(fill="x", padx=16, pady=(0, 12))
        label(form, "Add URLs (one per line). Uses the mode, quality and output folder on the main window.").pack(fill="x")
        urls = tk.Text(form, height=3, bg=self.COLORS["input"], fg=self.COLORS["text"], insertbackground=self.COLORS["text"])
        urls.pack(fill="x", pady=5)
        urls.insert("1.0", self.url_var.get())
        schedule = tk.Frame(form, bg=self.COLORS["background"])
        schedule.pack(fill="x")
        start_var, duration_var = tk.StringVar(), tk.StringVar(value="60")
        label(schedule, "Live start (local YYYY-MM-DD HH:MM):").grid(row=0, column=0, sticky="w")
        tk.Entry(schedule, textvariable=start_var, width=20).grid(row=0, column=1, padx=8)
        label(schedule, "Minutes:").grid(row=0, column=2)
        tk.Entry(schedule, textvariable=duration_var, width=6).grid(row=0, column=3, padx=8)
        label(schedule, "Reserve GiB:").grid(row=1, column=0, sticky="w", pady=6)
        tk.Spinbox(schedule, from_=0.25, to=100, increment=.25, textvariable=self.reserve_var, width=6).grid(row=1, column=1, sticky="w", padx=8)
        hint = label(form, "Leave start blank to record now. Live duration includes reconnect waits. Keep PixClip open and the computer awake.")
        hint.configure(wraplength=740, justify="left", fg=self.COLORS["warning"])
        hint.pack(fill="x", pady=4)
        def add(scheduled):
            from media_toolkit import STREAMLINK_LIVE_MODE, LIVE_TS_NVENC_MODE, AUTO_LIVE_TS_MODE
            try:
                links = [line.strip() for line in urls.get("1.0", "end").splitlines() if line.strip()]
                if not links:
                    raise ValueError("Paste at least one URL")
                start, duration = 0, 0
                if scheduled:
                    if self.download_mode_var.get() not in (STREAMLINK_LIVE_MODE, LIVE_TS_NVENC_MODE, AUTO_LIVE_TS_MODE):
                        raise ValueError("Select a Live Streamlink, Live NVENC, or Auto GPU/CPU mode on the main window first.")
                    start = datetime.strptime(start_var.get().strip(), "%Y-%m-%d %H:%M").timestamp() if start_var.get().strip() else 0
                    if start and start <= time.time():
                        raise ValueError("Start time must be in the future")
                    duration = float(duration_var.get()) * 60
                    if not math.isfinite(duration) or duration <= 0:
                        raise ValueError("Duration must be a positive number of minutes")
                from urllib.parse import urlparse
                if any(urlparse(url).scheme not in ("http", "https") or not urlparse(url).netloc for url in links):
                    raise ValueError("Every line must be a valid http(s) URL")
                require_space(self.output_directory, self.reserve_var.get())
                for url in links:
                    self.jobs.add(url, self.download_mode_var.get(), self.output_directory, self.quality_var.get(),
                        self.auto_mp4_var.get(), start_at=start, duration=duration, reserve_gb=self.reserve_var.get(),
                        cookies_file=self.cookies_file)
                urls.delete("1.0", "end")
                self.refresh_queue()
            except (ValueError, OSError, tk.TclError) as exc:
                messagebox.showerror("Add jobs", str(exc), parent=window)
        buttons = tk.Frame(form, bg=self.COLORS["background"])
        buttons.pack(fill="x")
        self._button(buttons, "Add downloads now", lambda: add(False), bg=self.COLORS["primary"]).pack(side="left")
        self._button(buttons, "Add timed Live", lambda: add(True), icon="schedule").pack(side="left", padx=8)
        self.refresh_queue()

    def retry_local(self, job):
        import tempfile
        from tkinter import messagebox
        if job["state"] not in TERMINAL or self.current_runner is not None or self.jobs.active:
            raise ValueError("Finish active work before retrying this edit")
        arguments = list(job["arguments"])
        if arguments and Path(arguments[-1]).exists() and not messagebox.askyesno("Retry edit", "The output already exists. Replace it by running this edit again?", parent=self.root):
            return
        temporary = []
        for previous, contents in job.get("temporary", {}).items():
            with tempfile.NamedTemporaryFile(mode="w", suffix=".txt", encoding="utf-8", delete=False) as file:
                file.write(contents)
                path = Path(file.name)
            temporary.append(path)
            arguments = [str(path) if arg == previous else arg for arg in arguments]
        self.start_task(job["executable"], arguments, Path(job["folder"]), job["url"], temporary_files=temporary)

    def refresh_queue(self):
        tree = self.queue_tree
        if tree is None or not tree.winfo_exists():
            return
        wanted = {j["id"] for j in self.jobs.jobs if not self.history_only.get() or j["state"] in TERMINAL}
        for item in tree.get_children():
            if item not in wanted:
                tree.delete(item)
        for job in self.jobs.jobs:
            if job["id"] not in wanted:
                continue
            progress = job.get("progress")
            detail = job.get("detail", "")
            if job["state"] == "Queued" and job.get("start_at"):
                detail = "Scheduled " + datetime.fromtimestamp(job["start_at"]).strftime("%Y-%m-%d %H:%M")
            elif job["state"] == "Running" and job.get("duration"):
                remaining = max(0, (job.get("start_at") or job["started"]) + job["duration"] - time.time())
                detail = f"Live slot remaining {int(remaining)//60:02}:{int(remaining)%60:02} | {detail}"
            values = (job["state"], "--" if progress is None else f"{progress:.1f}%", job["url"], detail)
            if tree.exists(job["id"]):
                if tree.item(job["id"], "values") != values:
                    tree.item(job["id"], values=values)
            else:
                tree.insert("", "end", iid=job["id"], values=values)

    def show_outputs(self, outputs):
        import os
        tk = self.tk
        from tkinter import messagebox
        window = tk.Toplevel(self.root)
        window.title("PixClip - Completed files")
        box = tk.Listbox(window, width=90, height=12)
        box.pack(fill="both", expand=True)
        for path in outputs:
            box.insert("end", str(path))
        def open_selected(event=None):
            if not box.curselection():
                return
            path = outputs[box.curselection()[0]]
            try:
                if os.name == "nt":
                    os.startfile(str(path))
                else:
                    subprocess.Popen(["open" if sys_platform() == "darwin" else "xdg-open", str(path)])
            except OSError as exc:
                messagebox.showerror("Open file", str(exc), parent=window)
        self._button(window, "Open selected file", open_selected).pack(pady=8)
        box.bind("<Double-1>", open_selected)

    def show_help_dialog(self):
        """Show the in-app Thai/English quick guide without blocking the main window."""
        tk = self.tk
        if getattr(self, "help_window", None) is not None and self.help_window.winfo_exists():
            self.help_window.lift()
            self.help_window.focus_force()
            return

        window = self.help_window = tk.Toplevel(self.root)
        window.title("PixClip - วิธีใช้ / Help")
        window.geometry("900x700")
        window.minsize(680, 520)
        window.configure(bg=self.COLORS["background"])
        window.transient(self.root)
        if self.app_icon is not None:
            try:
                window.iconphoto(False, self.app_icon)
            except tk.TclError:
                pass

        header = tk.Frame(window, bg=self.COLORS["header"], height=92)
        header.pack(fill="x")
        header.pack_propagate(False)
        if self.mascot_image is not None:
            tk.Label(header, image=self.mascot_image, bg=self.COLORS["header"], borderwidth=0).pack(side="left", padx=(18, 8), pady=6)

        language_panel = tk.Frame(header, bg=self.COLORS["header"])
        language_panel.pack(side="right", anchor="n", padx=(8, 18), pady=(15, 0))
        language_state_label = tk.Label(
            language_panel,
            text="LANG",
            bg=self.COLORS["header"],
            fg=self.COLORS["muted"],
            font=(self.mono_font, 8, "bold"),
        )
        language_state_label.pack(anchor="e", pady=(0, 3))
        language_buttons = tk.Frame(language_panel, bg=self.COLORS["header"])
        language_buttons.pack(anchor="e")
        language_var = tk.StringVar(value="th")
        self.help_language_var = language_var

        title = tk.Frame(header, bg=self.COLORS["header"])
        title.pack(side="left", fill="both", expand=True)
        title_label = tk.Label(
            title,
            text="PIXCLIP // QUICK GUIDE",
            bg=self.COLORS["header"],
            fg=self.COLORS["success"],
            font=(self.mono_font, 16, "bold"),
            anchor="w",
        )
        title_label.pack(fill="x", pady=(17, 0))
        subtitle_label = tk.Label(
            title,
            text="คู่มือใช้งานฉบับย่อ เปิดหน้าต่างนี้ค้างไว้ระหว่างใช้งานได้",
            bg=self.COLORS["header"],
            fg=self.COLORS["muted"],
            font=(self.mono_font, 9),
            anchor="w",
        )
        subtitle_label.pack(fill="x", pady=(4, 0))
        tk.Frame(header, bg=self.COLORS["primary"], height=3).pack(side="bottom", fill="x")

        body = tk.Frame(window, bg=self.COLORS["background"])
        body.pack(fill="both", expand=True, padx=18, pady=(14, 8))
        text_box = tk.Text(
            body,
            bg=self.COLORS["log_background"],
            fg=self.COLORS["text"],
            insertbackground=self.COLORS["text"],
            selectbackground=self.COLORS["primary"],
            relief="flat",
            bd=0,
            wrap="word",
            padx=16,
            pady=14,
            font=(self.mono_font, 10),
            spacing1=2,
            spacing3=6,
        )
        self.help_text_box = text_box
        scroll = tk.Scrollbar(body, command=text_box.yview)
        text_box.configure(yscrollcommand=scroll.set)
        text_box.pack(side="left", fill="both", expand=True)
        scroll.pack(side="right", fill="y")
        text_box.tag_configure(
            "heading",
            foreground=self.COLORS["info"],
            font=(self.mono_font, 11, "bold"),
            spacing1=8,
            spacing3=2,
        )
        text_box.tag_configure("hint", foreground=self.COLORS["warning"])

        translations = {
            "th": {
                "window_title": "PixClip - วิธีใช้ / Help",
                "subtitle": "คู่มือใช้งานฉบับย่อ เปิดหน้านี้ค้างไว้ระหว่างใช้งานได้",
                "language": "ภาษา: ไทย",
                "tip": "เคล็ดลับ: กด HELP นี้ได้ทุกเมื่อโดยไม่ต้องหยุดงานที่กำลังทำ",
                "close": "ปิดหน้าต่าง",
                "full": "เปิดคู่มือเต็ม",
                "sections": (
                    ("01  เริ่มใช้งาน", "กด Install tools เพื่อติดตั้ง yt-dlp, FFmpeg และ Streamlink และบน Linux จะติดตั้ง `python3-tk` หากยังไม่มี จากนั้นวางลิงก์วิดีโอหรือ Live ลงในช่อง URL และเลือกโฟลเดอร์ปลายทาง PixClip จะจำโฟลเดอร์ล่าสุดไว้ให้ในครั้งถัดไป หากเว็บต้องล็อกอิน ให้เลือกไฟล์ Netscape `cookies.txt` ด้วยปุ่ม Choose cookies"),
                    ("02  ดาวน์โหลดวิดีโอ", "เลือกโหมดที่ต้องการ แล้วกด 1 DOWNLOAD งานจะถูกเพิ่มในคิวทันที สามารถเลือกคุณภาพ Best available หรือจำกัดความละเอียดสูงสุด 2160p / 1080p / 720p / 480p ได้"),
                    ("03  โหลด Live และ TS", "เลือก Live - Auto GPU/CPU เพื่อให้ PixClip ตรวจ NVIDIA/AMD และเลือก NVENC/AMF/VA-API หรือ CPU ให้อัตโนมัติ หรือเลือก Live - Streamlink / Live - NVIDIA NVENC เองก็ได้ หาก TikTok มองไม่เห็น Live ให้เลือกไฟล์ Netscape `cookies.txt` ก่อนเพิ่มงาน โปรแกรมจะแสดงเปอร์เซ็นต์และเวลาโดยประมาณเมื่อคำนวณได้"),
                    ("04  Cookies สำหรับเว็บที่ต้องล็อกอิน", "1) ล็อกอินเว็บในเบราว์เซอร์ แล้ว export Cookies เป็นไฟล์ Netscape `cookies.txt` 2) กด Choose ข้าง Cookies: OFF แล้วเลือกไฟล์ 3) ตรวจชื่อไฟล์ที่แสดง จากนั้นเพิ่มงาน Live หรือดาวน์โหลด 4) กด Clear เมื่อต้องการเลิกใช้ Cookies PixClip จำเฉพาะ path และไม่แสดงค่า Cookies ใน Log ห้ามแชร์ไฟล์นี้"),
                    ("05  TS → MP4 อัตโนมัติ", "เปิดตัวเลือก TS -> MP4 (keep TS) ก่อนเพิ่มงาน โปรแกรมจะแปลงด้วยการ copy stream และเก็บไฟล์ TS ต้นฉบับไว้ หากเปิดไม่ได้ให้ใช้โหมด TS to MP4 - Compatible H.264"),
                    ("06  Queue / History", "กด Queue / History เพื่อเพิ่มหลาย URL (หนึ่งบรรทัดต่อหนึ่งลิงก์), ตั้งงานพร้อมกัน 1–4 งาน, หยุดรับงานใหม่ด้วย Pause queue, ยกเลิก, ลบรายการออกจากประวัติ (ไม่ลบไฟล์), ลองใหม่, เปิดไฟล์, เปิดโฟลเดอร์ หรือดู Job log ได้"),
                    ("07  ตั้งเวลาอัด Live", "เลือกโหมด Live ก่อน เปิด Queue / History แล้วใส่เวลาเครื่องรูปแบบ YYYY-MM-DD HH:MM และจำนวนนาที จากนั้นกด Add timed Live ต้องเปิด PixClip และให้เครื่องตื่นอยู่ตลอดช่วงเวลาอัด"),
                    ("08  ตัดคลิปแบบมีพรีวิว", "กด Cut clip → Choose video รออ่านความยาว แล้วเลื่อนแถบเพื่อดูภาพเฟรม กด Set start here และ Set end here ได้ Fast cut เร็วกว่า ส่วน Compatible MP4 ตัดตรงเวลามากกว่า"),
                    ("09  แปลงและต่อคลิป", "เลือกโหมดในแผง CONVERT แล้วกด 2 CHOOSE VIDEO FILE สำหรับต่อคลิปให้กด Join clips และเลือก Fast เมื่อไฟล์มีรูปแบบตรงกัน หรือ Compatible MP4 เมื่อต้องเข้ารหัสใหม่"),
                    ("10  ถ้างานมีปัญหา", "ตรวจ Install tools และพื้นที่ว่างก่อน ลอง Best available หากจำกัดความละเอียดแล้วไม่มีรูปแบบที่รองรับ เปิด Job log เพื่อดูสาเหตุ และกด Retry หลังแก้ปัญหาแล้ว"),
                    ("11  หมายเหตุ", "การตั้งเวลาและคิวทำงานขณะที่ PixClip เปิดอยู่เท่านั้น มาสคอตจะแสดง READY, WORKING, OK หรือ CHECK ตามสถานะงาน ไฟล์ประวัติถูกเก็บในโฟลเดอร์ข้อมูลผู้ใช้ของ Windows/Linux"),
                ),
            },
            "en": {
                "window_title": "PixClip - Help",
                "subtitle": "Quick guide. Keep this window open while you work.",
                "language": "Language: English",
                "tip": "Tip: Open HELP anytime without stopping the current task.",
                "close": "Close window",
                "full": "Open full guide",
                "sections": (
                    ("01  Getting started", "Click Install tools to install yt-dlp, FFmpeg, and Streamlink. On Linux, it also installs `python3-tk` when needed. Paste a video or Live URL, then choose the destination folder. PixClip remembers the last folder for the next launch. If a site requires login, choose a Netscape `cookies.txt` file."),
                    ("02  Download video", "Choose a mode and press 1 DOWNLOAD to add a job to the queue. Select Best available or set a maximum resolution of 2160p, 1080p, 720p, or 480p."),
                    ("03  Record Live and TS", "Choose Live - Auto GPU/CPU to detect NVIDIA or AMD and select NVENC, AMF, VA-API, or CPU automatically. You can also choose Live - Streamlink or Live - NVIDIA NVENC manually. If TikTok hides a Live stream, choose a Netscape `cookies.txt` file before adding the job. Progress and an estimated time appear when available."),
                    ("04  Cookies for login required sites", "1) Sign in to the site in your browser and export Cookies as a Netscape `cookies.txt` file. 2) Click Choose beside Cookies: OFF and select the file. 3) Confirm the file name, then add the Live or download job. 4) Click Clear to stop using Cookies. PixClip remembers only the path and never displays cookie values in the log. Never share this file."),
                    ("05  Automatic TS to MP4", "Enable TS -> MP4 (keep TS) before adding a job. PixClip remuxes with stream copy and keeps the original TS file. If the MP4 container is not compatible, use TS to MP4 - Compatible H.264."),
                    ("06  Queue / History", "Use Queue / History to add multiple URLs, set 1-4 parallel jobs, pause new jobs, cancel, delete history entries without deleting media files, retry, open files or folders, and view job logs."),
                    ("07  Schedule a Live recording", "Choose a Live mode, open Queue / History, enter local time as YYYY-MM-DD HH:MM and the number of minutes, then press Add timed Live. Keep PixClip open and keep the computer awake."),
                    ("08  Cut with preview", "Click Cut clip -> Choose video, wait for the duration, then move the slider to preview frames. Use Set start here and Set end here. Fast cut is quicker; Compatible MP4 is more precise."),
                    ("09  Convert and join", "Choose a mode in CONVERT and press 2 CHOOSE VIDEO FILE. To join clips, press Join clips and use Fast when stream settings match, or Compatible MP4 to re-encode."),
                    ("10  Troubleshooting", "Check Install tools and free disk space first. Try Best available if a resolution limit has no compatible format. Open Job log for details and press Retry after fixing the issue."),
                    ("11  Notes", "Queue and scheduled recordings run only while PixClip is open. The mascot shows READY, WORKING, OK, or CHECK. Job history is stored in the Windows/Linux user data folder."),
                ),
            },
        }

        def render_help(language):
            language = "en" if language == "en" else "th"
            data = translations[language]
            language_var.set(language)
            window.title(data["window_title"])
            subtitle_label.configure(text=data["subtitle"])
            language_state_label.configure(text=data["language"])
            text_box.configure(state="normal")
            text_box.delete("1.0", "end")
            for index, (heading, paragraph) in enumerate(data["sections"]):
                text_box.insert("end", heading + "\n", "heading")
                text_box.insert("end", paragraph + "\n")
                if index == 0:
                    text_box.insert("end", data["tip"] + "\n", "hint")
            text_box.configure(state="disabled")
            th_button.configure(
                bg=self.COLORS["primary"] if language == "th" else self.COLORS["surface_raised"],
                activebackground=self.COLORS["primary_pressed"] if language == "th" else self.COLORS["border"],
            )
            en_button.configure(
                bg=self.COLORS["primary"] if language == "en" else self.COLORS["surface_raised"],
                activebackground=self.COLORS["primary_pressed"] if language == "en" else self.COLORS["border"],
            )

        self.help_render = render_help
        th_button = self._button(language_buttons, "TH", lambda: render_help("th"), width=4)
        en_button = self._button(language_buttons, "EN", lambda: render_help("en"), width=4)
        th_button.pack(side="left", padx=(0, 4))
        en_button.pack(side="left")

        footer = tk.Frame(window, bg=self.COLORS["background"])
        footer.pack(fill="x", padx=18, pady=(0, 14))
        self._button(footer, "ปิดหน้าต่าง", window.destroy, width=12).pack(side="right")
        self._button(footer, "เปิดคู่มือเต็ม", lambda: self.open_help_file(window), width=14).pack(side="right", padx=(0, 8))
        window.protocol("WM_DELETE_WINDOW", window.destroy)
        window.bind("<Escape>", lambda event: window.destroy())
        render_help("th")
        window.focus_force()

    def open_help_file(self, parent):
        import os
        from tkinter import messagebox
        guide = BASE_DIR / "README-PixClip.md"
        try:
            if os.name == "nt":
                os.startfile(str(guide))
            else:
                subprocess.Popen(["open" if sys_platform() == "darwin" else "xdg-open", str(guide)])
        except OSError as exc:
            messagebox.showerror("วิธีใช้", f"เปิดคู่มือไม่สำเร็จ:\n{exc}", parent=parent)


def sys_platform():
    import sys
    return sys.platform


class ClipPreview:
    """Debounced still-frame preview. No Pillow or video-player dependency."""
    def __init__(self, app, parent, source_var, start_var, end_var, duration_var=None):
        self.app, self.source_var = app, source_var
        self.duration_var, self.end_var = duration_var, end_var
        self.parent, self.pending, self.process = parent, None, None
        self.generation, self.closed = 0, False
        self.source_generation = 0
        self.results = Queue()
        self.duration = 0
        tk = app.tk
        self.frame = tk.Frame(parent, bg=app.COLORS["background"])
        self.frame.pack(fill="both", expand=True, padx=24, pady=4)
        self.label = tk.Label(self.frame, text="Choose a video to preview frames", bg=app.COLORS["input"], fg=app.COLORS["muted"])
        self.label.pack(fill="both", expand=True)
        self.slider = tk.Scale(self.frame, from_=0, to=1, resolution=.1, orient="horizontal",
            bg=app.COLORS["background"], fg=app.COLORS["text"], highlightthickness=0,
            command=self.seek, state="disabled")
        self.slider.pack(fill="x")
        buttons = tk.Frame(self.frame, bg=app.COLORS["background"])
        buttons.pack(fill="x")
        from media_toolkit import format_ffmpeg_time
        app._button(buttons, "Set start here", lambda: start_var.set(format_ffmpeg_time(self.slider.get()))).pack(side="left")
        app._button(buttons, "Set end here", lambda: end_var.set(format_ffmpeg_time(self.slider.get()))).pack(side="left", padx=8)
        self.trace = source_var.trace_add("write", self.source_changed)
        self.frame.bind("<Destroy>", self.destroy)
        self.poll()

    def source_changed(self, *_):
        from media_toolkit import get_media_duration
        path = Path(self.source_var.get())
        self.source_generation += 1
        source_generation = self.source_generation
        self.generation += 1
        self.duration = 0
        self.slider.configure(state="disabled")
        self.label.configure(image="", text="Reading video duration...")
        if self.duration_var is not None:
            self.duration_var.set("Reading duration...")
        def probe():
            duration = get_media_duration(path) or 0
            self.results.put(("duration", source_generation, duration))
        threading.Thread(target=probe, daemon=True).start()

    def seek(self, value):
        self.generation += 1
        if self.pending:
            self.frame.after_cancel(self.pending)
        self.pending = self.frame.after(250, lambda: self.render(float(value), self.generation))

    def render(self, seconds, generation):
        self.pending = None
        if self.closed or not self.source_var.get():
            return
        if self.process and self.process.poll() is None:
            self.process.kill()
        path = self.source_var.get()
        self.label.configure(text="Loading frame...")
        def work():
            try:
                process = subprocess.Popen([self.app.ffmpeg_path, "-hide_banner", "-loglevel", "error",
                    "-ss", str(min(seconds, max(0, self.duration - .05))), "-i", path,
                    "-frames:v", "1", "-vf", "scale=480:180:force_original_aspect_ratio=decrease",
                    "-f", "image2pipe", "-vcodec", "png", "pipe:1"], stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
                self.process = process
                if self.closed or generation != self.generation:
                    process.kill()
                try:
                    data, err = process.communicate(timeout=15)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.communicate()
                    raise ValueError("Preview timed out; try another frame")
                if process.returncode or not data:
                    raise ValueError("Frame unavailable. You can still enter start/end times manually.")
                self.results.put((generation, data, ""))
            except (OSError, ValueError) as exc:
                self.results.put((generation, None, str(exc)))
        threading.Thread(target=work, daemon=True).start()

    def poll(self):
        if self.closed:
            return
        try:
            while True:
                generation, data, error = self.results.get_nowait()
                if generation == "duration":
                    if data != self.source_generation:
                        continue
                    from media_toolkit import format_ffmpeg_time
                    self.duration = error
                    self.slider.configure(to=max(.1, self.duration), state="normal" if self.duration else "disabled")
                    if self.duration_var is not None:
                        self.duration_var.set("Duration: " + (format_ffmpeg_time(self.duration) if self.duration else "unknown"))
                    if self.duration:
                        self.end_var.set(format_ffmpeg_time(self.duration))
                    self.slider.set(0)
                    self.seek(0)
                    continue
                if generation != self.generation:
                    continue
                if data:
                    self.image = self.app.tk.PhotoImage(master=self.frame, data=base64.b64encode(data))
                    self.label.configure(image=self.image, text="")
                else:
                    self.label.configure(image="", text=error)
        except Empty:
            pass
        self.frame.after(100, self.poll)

    def destroy(self, event):
        if event.widget != self.frame:
            return
        self.closed = True
        self.source_var.trace_remove("write", self.trace)
        if self.pending:
            self.frame.after_cancel(self.pending)
        if self.process and self.process.poll() is None:
            self.process.kill()

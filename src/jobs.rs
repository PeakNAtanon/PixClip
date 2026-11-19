use crate::{
    media,
    model::{self, History, Job, State},
    process::Context,
};
use anyhow::{Context as _, Result, ensure};
use fs2::FileExt;
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

enum Event {
    Line(String, String),
    Finished(String, std::result::Result<Vec<PathBuf>, String>),
}
struct Active {
    cancel: Arc<AtomicBool>,
    thread: thread::JoinHandle<()>,
}

pub struct Engine {
    pub history: History,
    pub log: VecDeque<String>,
    pub warning: String,
    pub directory: PathBuf,
    path: PathBuf,
    active: HashMap<String, Active>,
    sender: mpsc::Sender<Event>,
    receiver: mpsc::Receiver<Event>,
    closing: bool,
    _lock: File,
}

impl Engine {
    pub fn new(directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory)?;
        let lock = File::options()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(directory.join("queue.lock"))?;
        lock.try_lock_exclusive()
            .context("PixClip is already open for this user. Close the other window first.")?;
        let mut path = directory.join("jobs.json");
        let mut warning = String::new();
        let history = match History::load(&path) {
            Ok(history) => history,
            Err(error) => {
                warning = format!(
                    "History was not loaded: {error}. Original file kept. New jobs use a recovery file."
                );
                path = directory.join(format!(
                    "jobs-recovered-{}.json",
                    uuid::Uuid::new_v4().simple()
                ));
                History {
                    limit: 1,
                    ..History::default()
                }
            }
        };
        let (sender, receiver) = mpsc::channel();
        let mut engine = Self {
            history,
            log: VecDeque::new(),
            warning,
            directory: directory.into(),
            path,
            active: HashMap::new(),
            sender,
            receiver,
            closing: false,
            _lock: lock,
        };
        if !engine.warning.is_empty() {
            engine.log.push_back(format!("WARNING: {}", engine.warning));
        }
        Ok(engine)
    }

    pub fn save(&mut self) {
        if let Err(error) = model::atomic_json(&self.path, &self.history) {
            self.warning = format!("History was not saved: {error}");
        }
    }
    pub fn running(&self) -> bool {
        !self.active.is_empty()
    }
    pub fn active_count(&self) -> usize {
        self.active.len()
    }
    pub fn pending(&self) -> bool {
        self.running() || self.history.jobs.iter().any(|j| j.state == State::Queued)
    }
    pub fn job(&self, id: &str) -> Option<&Job> {
        self.history.jobs.iter().find(|j| j.id == id)
    }

    pub fn add(&mut self, mut job: Job) -> Result<String> {
        job.validate()?;
        if job.kind != "local" && job.kind != "tools" {
            ensure!(
                media::MODES.contains(&job.mode.as_str()),
                "Unknown download mode"
            );
            ensure!(
                !media::live(&job.mode) || job.quality != model::QUALITIES[5],
                "Choose a video quality for Live recording"
            );
            if job.output_folder.is_none() {
                job.output_folder = Some(model::organized_folder(&job)?);
            }
        }
        model::require_space(
            if job.kind == "local" || job.kind == "tools" {
                &job.folder
            } else {
                job.output_folder.as_ref().unwrap()
            },
            job.reserve_gb,
            0,
        )?;
        let id = job.id.clone();
        self.history.jobs.push(job);
        self.save();
        Ok(id)
    }

    pub fn retry(&mut self, id: &str, overwrite: bool) -> Result<String> {
        let mut job = self.job(id).context("Choose a job")?.clone();
        ensure!(
            job.state.terminal(),
            "Cancel or finish this job before retrying"
        );
        let resume_download = job.kind.is_empty()
            && (job.mode.starts_with("Playlist") || job.spotify_url.is_some())
            && job.state != State::Done;
        if resume_download && let Some(folder) = &job.output_folder {
            ensure!(
                !self.history.jobs.iter().any(|other| {
                    !other.state.terminal() && other.output_folder.as_ref() == Some(folder)
                }),
                "This download is already queued or running"
            );
        }
        job.id = uuid::Uuid::new_v4().simple().to_string();
        job.created = model::now();
        job.start_at = 0.0;
        job.started = None;
        job.finished = None;
        job.state = State::Queued;
        job.detail = "Waiting".into();
        job.progress = None;
        job.logs.clear();
        job.outputs.clear();
        if job.kind.is_empty() && !resume_download {
            job.output_folder = None;
        }
        if job.kind == "local" {
            for argument in &mut job.arguments {
                if argument == "-y" || argument == "-n" {
                    *argument = if overwrite { "-y" } else { "-n" }.into();
                }
            }
        }
        self.add(job)
    }

    pub fn cancel(&mut self, id: &str) {
        if let Some(active) = self.active.get(id) {
            active.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(job) = self.history.jobs.iter_mut().find(|j| j.id == id) {
            if job.state == State::Queued {
                job.state = State::Cancelled;
                job.detail = "Cancelled before starting".into();
            } else if job.state == State::Running {
                job.state = State::Cancelling;
                job.detail = "Stopping process tree; partial files kept".into();
            }
        }
        self.save();
    }

    pub fn delete(&mut self, id: &str) -> Result<()> {
        ensure!(
            !self.active.contains_key(id),
            "Cancel the running job before deleting it"
        );
        self.history.jobs.retain(|j| j.id != id);
        self.save();
        Ok(())
    }
    pub fn clear(&mut self) {
        self.history
            .jobs
            .retain(|j| self.active.contains_key(&j.id));
        self.save();
    }

    pub fn tick(&mut self) {
        let mut save = false;
        for event in self.receiver.try_iter().take(500) {
            match event {
                Event::Line(id, line) => {
                    self.log.push_back(format!(
                        "[{}] {line}",
                        id.chars().take(6).collect::<String>()
                    ));
                    while self.log.len() > 2000 {
                        self.log.pop_front();
                    }
                    if let Some(job) = self.history.jobs.iter_mut().find(|j| j.id == id) {
                        job.logs.push(line.clone());
                        if job.logs.len() > 100 {
                            job.logs.remove(0);
                        }
                        update_progress(job, &line);
                    }
                }
                Event::Finished(id, result) => {
                    let cancelled = self
                        .active
                        .get(&id)
                        .is_some_and(|a| a.cancel.load(Ordering::Relaxed));
                    if let Some(job) = self.history.jobs.iter_mut().find(|j| j.id == id) {
                        job.finished = Some(model::now());
                        if cancelled {
                            job.state = State::Cancelled;
                            job.detail = "Cancelled; partial files kept".into();
                        } else {
                            match result {
                                Ok(outputs) => {
                                    job.state = State::Done;
                                    job.outputs = outputs;
                                    job.progress = Some(100.0);
                                    job.detail = "Complete".into();
                                }
                                Err(error) => {
                                    job.state = State::Failed;
                                    job.detail = format!("ERROR: {error}");
                                    job.logs.push(job.detail.clone());
                                    job.detail = failed_detail(job);
                                    self.log.push_back(job.detail.clone());
                                }
                            }
                        }
                    }
                    if let Some(active) = self.active.remove(&id) {
                        let _ = active.thread.join();
                    }
                    save = true;
                }
            }
        }
        if !self.closing {
            let mut waiting: Vec<_> = self
                .history
                .jobs
                .iter()
                .filter(|j| {
                    j.state == State::Queued && (!self.history.paused || !j.kind.is_empty())
                })
                .map(|j| (j.id.clone(), j.start_at, j.created))
                .collect();
            waiting.sort_by(|a, b| {
                (a.1 == 0.0)
                    .cmp(&(b.1 == 0.0))
                    .then_with(|| a.1.total_cmp(&b.1))
                    .then_with(|| a.2.total_cmp(&b.2))
            });
            for (id, start, _) in waiting {
                let now = model::now();
                if start > now {
                    continue;
                }
                let index = self.history.jobs.iter().position(|j| j.id == id).unwrap();
                if start > 0.0
                    && self.history.jobs[index].duration > 0.0
                    && now >= start + self.history.jobs[index].duration
                {
                    let job = &mut self.history.jobs[index];
                    job.state = State::Failed;
                    job.detail = "Scheduled window missed; retry to record now".into();
                    save = true;
                    continue;
                }
                let exclusive = !self.history.jobs[index].kind.is_empty();
                let active_exclusive = self.active.keys().any(|id| {
                    self.history
                        .jobs
                        .iter()
                        .any(|j| j.id == *id && !j.kind.is_empty())
                });
                if self.active.len() >= self.history.limit.clamp(1, 5)
                    || active_exclusive
                    || (exclusive && self.running())
                {
                    continue;
                }
                let job = &mut self.history.jobs[index];
                job.state = State::Running;
                job.started = Some(now);
                job.detail = "Starting".into();
                job.progress = None;
                let job = job.clone();
                let sender = self.sender.clone();
                let id_for_log = id.clone();
                let cancel = Arc::new(AtomicBool::new(false));
                let ctx = Context {
                    cancel: cancel.clone(),
                    deadline: if media::live(&job.mode) {
                        job.deadline()
                    } else {
                        None
                    },
                    folder: if exclusive {
                        job.folder.clone()
                    } else {
                        job.directory()
                    },
                    reserve: job.reserve_gb,
                    log: Arc::new(move |line| {
                        let _ = sender.send(Event::Line(id_for_log.clone(), line));
                    }),
                };
                let sender = self.sender.clone();
                let thread = thread::spawn(move || {
                    let result = media::execute(&job, &ctx)
                        .map_err(|e| crate::process::public_line(&format!("{e:#}")));
                    let _ = sender.send(Event::Finished(job.id, result));
                });
                self.active.insert(id, Active { cancel, thread });
                save = true;
                if exclusive {
                    break;
                }
            }
        }
        if save {
            self.save();
        }
    }

    pub fn begin_close(&mut self) {
        self.closing = true;
        let ids: Vec<_> = self.active.keys().cloned().collect();
        for id in ids {
            self.cancel(&id);
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.begin_close();
        for (_, active) in self.active.drain() {
            let _ = active.thread.join();
        }
        for job in &mut self.history.jobs {
            if matches!(job.state, State::Running | State::Cancelling) {
                job.state = State::Interrupted;
                job.detail = "App closed during this job. Partial files were kept.".into();
            }
        }
        self.save();
    }
}

pub(crate) fn failed_detail(job: &Job) -> String {
    let is_youtube = url::Url::parse(&job.url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| {
            host == "youtube.com" || host.ends_with(".youtube.com") || host == "youtu.be"
        });
    if is_youtube && job.kind.is_empty() {
        for line in job
            .logs
            .iter()
            .rev()
            .filter(|line| line.starts_with("ERROR: [youtube]"))
        {
            if line.contains("Sign in to confirm your age") {
                return "ERROR: YouTube requires an age-verified account. Select fresh Cookies and add the URL again.".into();
            }
            if line.contains("not a bot") {
                return "ERROR: YouTube could not verify the session. Sign in, select fresh Cookies, then add the URL again.".into();
            }
        }
    }
    job.detail.clone()
}

fn update_progress(job: &mut Job, line: &str) {
    if let Some(path) = line.strip_prefix("PIXCLIP_OUTPUT=") {
        let path = PathBuf::from(path);
        if !job.outputs.contains(&path) {
            job.outputs.push(path);
        }
    } else if let Some(phase) = line.strip_prefix("PIXCLIP_PHASE=") {
        job.detail = phase.into();
        job.progress = None;
    } else if let Some(seconds) = line
        .strip_prefix("media_duration=")
        .and_then(|v| v.parse::<f64>().ok())
    {
        if seconds.is_finite() && seconds > 0.0 {
            job.extra.insert("media_duration".into(), seconds.into());
        }
    } else if let Some(seconds) = line
        .strip_prefix("out_time_us=")
        .or_else(|| line.strip_prefix("out_time_ms="))
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| v / 1_000_000.0)
    {
        if seconds.is_finite() {
            job.extra.insert("media_elapsed".into(), seconds.into());
            if let Some(length) = job
                .extra
                .get("media_duration")
                .and_then(|v| v.as_f64())
                .filter(|v| *v > 0.0)
            {
                job.progress = Some((100.0 * seconds / length).clamp(0.0, 99.0));
            } else if media::live(&job.mode) {
                job.detail = format!("Live recording | REC {}", model::timecode(seconds));
            }
        }
    } else if let Some(speed) = line
        .strip_prefix("speed=")
        .and_then(|s| s.trim_end_matches('x').parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
    {
        if let Some(length) = job.extra.get("media_duration").and_then(|v| v.as_f64()) {
            let elapsed = job
                .extra
                .get("media_elapsed")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            job.detail = format!(
                "Working | ETA ~{}",
                model::timecode(((length - elapsed) / speed).max(0.0))
            );
        }
    } else if line.starts_with("[download]")
        && let Some(value) = line
            .split_whitespace()
            .find_map(|s| s.strip_suffix('%').and_then(|s| s.parse::<f64>().ok()))
            .filter(|v| v.is_finite())
    {
        job.progress = Some(value.clamp(0.0, 99.0));
        job.detail = line.into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn new_job(folder: &Path) -> Job {
        Job {
            url: "https://example.com/video".into(),
            mode: media::MODES[0].into(),
            folder: folder.into(),
            ..Job::default()
        }
    }
    #[test]
    fn pause_cancel_delete_retry_and_history_preserve_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::new(dir.path()).unwrap();
        engine.history.paused = true;
        let id = engine.add(new_job(dir.path())).unwrap();
        engine.tick();
        assert!(!engine.running());
        let folder = engine.job(&id).unwrap().directory();
        fs::write(folder.join("kept.ts"), b"media").unwrap();
        engine.cancel(&id);
        assert_eq!(engine.job(&id).unwrap().state, State::Cancelled);
        let retry = engine.retry(&id, false).unwrap();
        assert_ne!(folder, engine.job(&retry).unwrap().directory());
        engine.delete(&id).unwrap();
        assert!(folder.join("kept.ts").is_file());
        assert!(Engine::new(dir.path()).is_err());
        drop(engine);
        let restored = Engine::new(dir.path()).unwrap();
        assert_eq!(restored.history.jobs.len(), 1);
    }
    #[test]
    fn retrying_an_incomplete_playlist_reuses_its_files_and_archive() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::new(dir.path()).unwrap();
        let mut job = new_job(dir.path());
        job.mode = media::MODES[6].into();
        let saved_folder = dir.path().join("Playlists/2026-11-19/143200 - youtube");
        job.output_folder = Some(saved_folder.clone());
        let id = engine.add(job).unwrap();
        let folder = engine.job(&id).unwrap().directory();
        assert_eq!(folder, saved_folder);
        engine.cancel(&id);
        let archive = folder.join("_archives").join("playlist-mp3.txt");
        fs::create_dir_all(archive.parent().unwrap()).unwrap();
        fs::write(&archive, b"youtube complete\n").unwrap();
        let retry = engine.retry(&id, false).unwrap();
        assert_eq!(engine.job(&retry).unwrap().directory(), folder);
        assert_eq!(fs::read(archive).unwrap(), b"youtube complete\n");
        assert!(engine.retry(&id, false).is_err());
    }
    #[test]
    fn expired_schedule_never_starts() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::new(dir.path()).unwrap();
        let mut job = new_job(dir.path());
        job.mode = media::STREAMLINK.into();
        job.start_at = model::now() - 100.0;
        job.duration = 10.0;
        let id = engine.add(job).unwrap();
        engine.tick();
        assert_eq!(engine.job(&id).unwrap().state, State::Failed);
        assert!(!engine.running());
    }
    #[test]
    fn pausing_downloads_does_not_pause_local_edits() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::new(dir.path()).unwrap();
        engine.history.paused = true;
        let download = engine.add(new_job(dir.path())).unwrap();
        let local = engine
            .add(Job {
                kind: "local".into(),
                mode: "Converting".into(),
                folder: dir.path().into(),
                arguments: vec!["missing-output.mp4".into()],
                ..Job::default()
            })
            .unwrap();
        engine.tick();
        assert_eq!(engine.job(&download).unwrap().state, State::Queued);
        assert_eq!(engine.job(&local).unwrap().state, State::Running);
        engine.begin_close();
    }
    #[test]
    fn ffmpeg_and_download_progress_are_understood() {
        let mut job = Job::default();
        update_progress(&mut job, "media_duration=100");
        update_progress(&mut job, "out_time_us=50000000");
        assert_eq!(job.progress, Some(50.0));
        update_progress(&mut job, "speed=2x");
        assert!(job.detail.contains("00:00:25"));
        update_progress(&mut job, "[download] 75.2% of 10MiB ETA 00:01");
        assert_eq!(job.progress, Some(75.2));
    }

    #[test]
    fn failed_youtube_jobs_explain_the_authentication_error_and_keep_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::new(dir.path()).unwrap();
        for (url, message, expected) in [
            (
                "https://www.youtube.com/watch?v=example",
                "Sign in to confirm your age.",
                "YouTube requires an age-verified account",
            ),
            (
                "https://youtu.be/example",
                "Sign in to confirm you’re not a bot.",
                "YouTube could not verify the session",
            ),
            (
                "https://example.com/video",
                "Sign in to confirm your age.",
                "Download failed",
            ),
        ] {
            let id = engine
                .add(Job {
                    url: url.into(),
                    ..new_job(dir.path())
                })
                .unwrap();
            let line = format!("ERROR: [youtube] example: {message}");
            engine
                .sender
                .send(Event::Line(id.clone(), line.clone()))
                .unwrap();
            engine
                .sender
                .send(Event::Finished(id.clone(), Err("Download failed".into())))
                .unwrap();
            engine.tick();
            let job = engine.job(&id).unwrap();
            assert_eq!(job.state, State::Failed);
            assert!(job.detail.contains(expected), "{}", job.detail);
            assert!(job.logs.contains(&line));
        }
    }
}

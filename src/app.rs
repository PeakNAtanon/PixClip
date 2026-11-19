mod clip;
mod playback;
mod playlist_review;
mod spotify_review;

use crate::{
    jobs::Engine,
    media,
    model::{self, AUDIO_BITRATES, AUDIO_FORMATS, Job, QUALITIES, Settings, State},
    process, tools,
};
use anyhow::{Context as _, Result, ensure};
use chrono::{Local, NaiveDateTime};
use clip::Clip;
use eframe::egui::{self, Color32, FontFamily, FontId, RichText, TextureHandle, Vec2};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const BG: Color32 = Color32::from_rgb(13, 23, 33);
const SURFACE: Color32 = Color32::from_rgb(23, 38, 49);
const INPUT: Color32 = Color32::from_rgb(9, 19, 27);
const HEADER: Color32 = Color32::from_rgb(19, 40, 46);
const TEXT: Color32 = Color32::from_rgb(237, 246, 242);
const MUTED: Color32 = Color32::from_rgb(169, 192, 189);
const MINT: Color32 = Color32::from_rgb(113, 225, 181);
const INFO: Color32 = Color32::from_rgb(156, 228, 211);
const AMBER: Color32 = Color32::from_rgb(255, 195, 101);
const RED: Color32 = Color32::from_rgb(248, 113, 113);
const GREEN: Color32 = Color32::from_rgb(28, 84, 70);
const TEAL: Color32 = Color32::from_rgb(25, 70, 82);
const SURFACE_RAISED: Color32 = Color32::from_rgb(34, 55, 68);
const BORDER: Color32 = Color32::from_rgb(58, 85, 95);
const BUTTON_ICON_ATLAS: &str = "pixclip-button-icons";
const PANEL_ATLAS: &str = "pixclip-panels";
const DROPDOWN_ATLAS: &str = "pixclip-dropdown-arrows";

enum UiEvent {
    Hardware(tools::Hardware),
    Preview(u64, Option<f64>, f64, std::result::Result<Vec<u8>, String>),
    JoinChecked(PathBuf, Vec<PathBuf>, std::result::Result<bool, String>),
}

pub struct App {
    engine: Engine,
    settings: Settings,
    url: String,
    mode: usize,
    quality: usize,
    audio_format: usize,
    audio_quality: usize,
    spotify_review: Option<spotify_review::Review>,
    playlist_review: Option<playlist_review::Review>,
    download_start: String,
    download_end: String,
    subtitle_language: usize,
    auto_mp4: bool,
    reserve: f64,
    convert_mode: usize,
    compress_limit: usize,
    compress_level: usize,
    extract_format: usize,
    extract_quality: usize,
    status: String,
    mascot: TextureHandle,
    filmstrip: TextureHandle,
    hardware: Option<tools::Hardware>,
    sender: mpsc::Sender<UiEvent>,
    receiver: mpsc::Receiver<UiEvent>,
    queue_open: bool,
    history_only: bool,
    selected: Option<String>,
    batch_urls: String,
    schedule: String,
    minutes: String,
    cut_open: bool,
    clip: Clip,
    join_open: bool,
    join_files: Vec<PathBuf>,
    join_selected: std::collections::BTreeSet<usize>,
    join_anchor: Option<usize>,
    join_checking: bool,
    join_mode: usize,
    help_open: bool,
    thai: bool,
    detail_open: bool,
    files_open: bool,
    file_selected: Option<PathBuf>,
    closing: bool,
    free_space: Option<u64>,
    space_checked: Instant,
    #[cfg(feature = "screenshot")]
    screenshot_frames: u8,
}

pub fn run() -> Result<()> {
    let directory = model::state_dir();
    let engine = Engine::new(&directory)?;
    let settings = Settings::load(&directory)?;
    let icon = image::load_from_memory(include_bytes!("../assets/pixclip-icon.png"))?.into_rgba8();
    let size = [1000.0, 860.0];
    #[cfg(feature = "screenshot")]
    let size = match std::env::var("PIXCLIP_SCREENSHOT_VIEW").as_deref() {
        Ok("queue") => [1040.0, 720.0],
        Ok("cut") => clip::SIZE,
        Ok("join") => [760.0, 560.0],
        Ok("help") => [900.0, 700.0],
        _ => size,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PixClip")
            .with_app_id("PixClip.UniversalMediaToolkit")
            .with_inner_size(size)
            .with_min_inner_size([740.0, size[1].min(780.0)])
            .with_icon(egui::IconData {
                width: icon.width(),
                height: icon.height(),
                rgba: icon.into_raw(),
            }),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "PixClip",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(&cc.egui_ctx, engine, settings)))),
    )
    .map_err(|e| anyhow::anyhow!("Could not open PixClip: {e}"))
}

impl App {
    fn new(ctx: &egui::Context, mut engine: Engine, settings: Settings) -> Self {
        theme(ctx);
        for (name, png) in [
            (
                BUTTON_ICON_ATLAS,
                include_bytes!("../assets/button-icons.png").as_slice(),
            ),
            (
                PANEL_ATLAS,
                include_bytes!("../assets/pixclip-panels.png").as_slice(),
            ),
            (
                DROPDOWN_ATLAS,
                include_bytes!("../assets/dropdown-arrows.png").as_slice(),
            ),
        ] {
            let image = texture(ctx, name, png, egui::TextureOptions::NEAREST)
                .expect("Bundled UI sprite sheet must decode");
            ctx.data_mut(|data| data.insert_temp(egui::Id::new(name), image));
        }
        if !settings.load_warning.is_empty() {
            engine
                .log
                .push_back(format!("WARNING: {}", settings.load_warning));
            if !engine.warning.is_empty() {
                engine.warning.push_str(" | ");
            }
            engine.warning.push_str(&settings.load_warning);
        }
        let mascot = texture(
            ctx,
            "pixclip-mascot",
            include_bytes!("../assets/pixclip-header.png"),
            egui::TextureOptions::NEAREST,
        )
        .expect("Bundled mascot must decode");
        let filmstrip = texture(
            ctx,
            "pixclip-filmstrip",
            include_bytes!("../assets/pixclip-filmstrip.png"),
            egui::TextureOptions::NEAREST,
        )
        .expect("Bundled filmstrip must decode");
        let (sender, receiver) = mpsc::channel();
        let background = sender.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let hardware = if cfg!(any(test, feature = "screenshot")) {
                tools::Hardware {
                    gpu: "Unknown".into(),
                    cpu: "Unknown".into(),
                    encoder: "libx264".into(),
                    file_encoder: "libx264".into(),
                }
            } else {
                tools::hardware()
            };
            let _ = background.send(UiEvent::Hardware(hardware));
            ctx.request_repaint();
        });
        let clipboard = if cfg!(any(test, feature = "screenshot")) {
            None
        } else {
            arboard::Clipboard::new()
                .ok()
                .and_then(|mut c| c.get_text().ok())
                .filter(|s| {
                    url::Url::parse(s.trim()).is_ok_and(|url| {
                        matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
                    })
                })
        }
        .unwrap_or_default();
        Self {
            engine,
            settings,
            url: clipboard.trim().into(),
            mode: 0,
            quality: 0,
            audio_format: 0,
            audio_quality: 0,
            spotify_review: None,
            playlist_review: None,
            download_start: "00:00".into(),
            download_end: "00:30".into(),
            subtitle_language: 0,
            auto_mp4: true,
            reserve: 1.0,
            convert_mode: 0,
            compress_limit: 0,
            compress_level: 0,
            extract_format: 0,
            extract_quality: 0,
            status: if ["yt-dlp", "ffmpeg", "ffprobe", "streamlink"]
                .iter()
                .any(|name| tools::find(name).is_none())
            {
                "Some required tools are missing. Use Install tools below."
            } else {
                "Ready"
            }
            .into(),
            mascot,
            filmstrip,
            hardware: None,
            sender,
            receiver,
            queue_open: false,
            history_only: false,
            selected: None,
            batch_urls: String::new(),
            schedule: String::new(),
            minutes: "60".into(),
            cut_open: false,
            clip: Clip::default(),
            join_open: false,
            join_files: vec![],
            join_selected: Default::default(),
            join_anchor: None,
            join_checking: false,
            join_mode: 0,
            help_open: false,
            thai: true,
            detail_open: false,
            files_open: false,
            file_selected: None,
            closing: false,
            free_space: None,
            space_checked: Instant::now() - Duration::from_secs(10),
            #[cfg(feature = "screenshot")]
            screenshot_frames: 0,
        }
    }

    fn error(&mut self, error: impl std::fmt::Display) {
        self.status = format!("ERROR: {error}");
        self.engine.log.push_back(self.status.clone());
        if !cfg!(any(test, feature = "screenshot")) {
            rfd::MessageDialog::new()
                .set_title("PixClip")
                .set_description(&self.status)
                .set_level(rfd::MessageLevel::Error)
                .show();
        }
    }
    fn save_settings(&mut self) {
        if let Err(error) = self.settings.save(&self.engine.directory) {
            self.error(error);
        }
    }
    fn download_mode(&self) -> &'static str {
        match self.mode {
            media::SPOTIFY_TRACK_MODE => media::MODES[3],
            media::SPOTIFY_PLAYLIST_MODE => media::MODES[6],
            media::SELECT_PLAYLIST_MODE => media::MODES[4],
            mode => media::MODES[mode],
        }
    }
    fn job_template(&self, url: String) -> Job {
        let mut job = Job {
            url,
            mode: self.download_mode().into(),
            folder: self.settings.output_directory.clone(),
            quality: QUALITIES[self.quality].into(),
            auto_mp4: self.auto_mp4,
            reserve_gb: self.reserve,
            cookies_file: self.settings.cookies_file.clone(),
            ..Job::default()
        };
        if media::audio_mode(&job.mode) {
            let format = AUDIO_FORMATS[self.audio_format];
            job.audio_format = format.into();
            if media::audio_encoder(format).is_some() {
                job.audio_bitrate = AUDIO_BITRATES[self.audio_quality];
            }
        }
        if matches!(self.mode, media::TIME_RANGE_MODE | media::SUBTITLES_MODE) {
            job.quality = QUALITIES[0].into();
            job.subtitle_language = media::SUBTITLE_LANGUAGES[self.subtitle_language].into();
        }
        job
    }
    fn enqueue(&mut self, urls: &str, schedule: bool) -> Result<()> {
        let urls: Vec<_> = urls
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        ensure!(!urls.is_empty(), "Paste at least one URL");
        let spotify_collection = match self.mode {
            media::SPOTIFY_TRACK_MODE => Some(false),
            media::SPOTIFY_PLAYLIST_MODE => Some(true),
            _ => None,
        };
        for &url in &urls {
            if let Some(link) = crate::spotify::Link::parse(url)? {
                ensure!(
                    !schedule,
                    "Spotify audio downloads cannot be scheduled as Live"
                );
                ensure!(urls.len() == 1, "Review one Spotify link at a time");
                ensure!(
                    spotify_collection.is_none_or(|collection| collection == link.is_collection()),
                    "Select {} for this Spotify link",
                    media::MODE_LABELS[if link.is_collection() {
                        media::SPOTIFY_PLAYLIST_MODE
                    } else {
                        media::SPOTIFY_TRACK_MODE
                    }]
                );
                return self.start_spotify_review(url, link);
            }
        }
        ensure!(
            spotify_collection.is_none(),
            "Paste a public open.spotify.com link for {}",
            media::MODE_LABELS[self.mode]
        );
        if self.mode == media::SELECT_PLAYLIST_MODE {
            ensure!(!schedule, "Choose a Live mode first");
            ensure!(urls.len() == 1, "Review one playlist at a time");
            return self.start_playlist_review(urls[0]);
        }
        let mut start = 0.0;
        let mut duration = 0.0;
        if schedule {
            ensure!(
                media::live(self.download_mode()),
                "Select a Live mode first"
            );
            let time = NaiveDateTime::parse_from_str(self.schedule.trim(), "%Y-%m-%d %H:%M")
                .context("Use Start format YYYY-MM-DD HH:MM")?;
            start = time
                .and_local_timezone(Local)
                .single()
                .context("Start time is ambiguous or does not exist")?
                .timestamp() as f64;
            duration = self
                .minutes
                .trim()
                .parse::<f64>()
                .context("Duration must be a number of minutes")?
                * 60.0;
            ensure!(
                start > model::now() && duration.is_finite() && duration > 0.0,
                "Choose a future start time and a positive duration"
            );
        }
        let mut jobs = vec![];
        for line in urls {
            let mut job = self.job_template(line.into());
            if self.mode == media::TIME_RANGE_MODE {
                job.download_range = Some([
                    model::parse_time(&self.download_start).context("Enter a valid start time")?,
                    model::parse_time(&self.download_end).context("Enter a valid end time")?,
                ]);
            }
            job.start_at = start;
            job.duration = duration;
            job.validate()?;
            jobs.push(job);
        }
        for job in jobs {
            self.selected = Some(self.engine.add(job)?);
        }
        self.status = if self.engine.history.paused {
            "Queue paused - press Resume queue to start."
        } else {
            "Added to queue"
        }
        .into();
        self.queue_open = true;
        Ok(())
    }
    fn add_edit(
        &mut self,
        label: &str,
        output: &Path,
        mut arguments: Vec<String>,
        temporary: std::collections::BTreeMap<String, String>,
        inputs: Vec<PathBuf>,
    ) -> Result<()> {
        if output.exists() {
            ensure!(
                confirm("Replace file?", &format!("Replace {}?", output.display())),
                "Cancelled; existing file kept"
            );
            for argument in &mut arguments {
                if argument == "-n" {
                    *argument = "-y".into();
                }
            }
        }
        let job = Job {
            kind: "local".into(),
            mode: label.into(),
            url: label.into(),
            folder: output
                .parent()
                .context("Choose an output directory")?
                .into(),
            executable: Some(tools::require("ffmpeg")?),
            arguments,
            inputs,
            temporary,
            reserve_gb: self.reserve,
            ..Job::default()
        };
        self.selected = Some(self.engine.add(job)?);
        self.engine.log.clear();
        self.status = label.into();
        Ok(())
    }
    fn tool_job(&mut self, update: bool) {
        if !update
            && !confirm(
                "Install or repair media tools",
                "Download official releases and verify SHA-256 checksums?",
            )
        {
            return;
        }
        let mode = if update {
            "Update yt-dlp"
        } else {
            "Install tools"
        };
        let job = Job {
            kind: "tools".into(),
            mode: mode.into(),
            url: mode.into(),
            folder: model::root(),
            reserve_gb: self.reserve,
            ..Job::default()
        };
        match self.engine.add(job) {
            Ok(id) => {
                self.selected = Some(id);
                self.status = mode.into();
                self.engine.log.clear();
            }
            Err(error) => self.error(error),
        }
    }

    fn main_ui(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG))
            .show(ctx, |ui| {
                modal_parent(ui, self.cut_open, self.join_open);
                let bounds = ui.max_rect();
                let width = bounds.width();
                let height = bounds.height();
                let content = width - 48.0;
                let right = width - 196.0;
                let idle = !self.engine.running();
                let local_busy = self.engine.history.jobs.iter().any(|j| {
                    !j.kind.is_empty() && matches!(j.state, State::Running | State::Cancelling)
                });
                ui.painter()
                    .rect_filled(rect(0.0, 0.0, width, 104.0), 0.0, HEADER);
                ui.painter()
                    .rect_filled(rect(0.0, 104.0, width, 1.0), 0.0, BORDER);
                ui.painter()
                    .rect_filled(rect(24.0, 101.0, 80.0, 4.0), 0.0, MINT);
                ui.painter()
                    .rect_filled(rect(104.0, 103.0, 20.0, 2.0), 0.0, INFO);
                if width >= 860.0 {
                    ui.put(
                        rect(width - 383.0, 16.0, 208.0, 72.0),
                        egui::Image::new(&self.filmstrip).fit_to_exact_size(Vec2::new(208.0, 72.0)),
                    );
                }
                let latest = self
                    .engine
                    .history
                    .jobs
                    .iter()
                    .filter(|j| j.finished.is_some())
                    .max_by(|a, b| a.finished.unwrap().total_cmp(&b.finished.unwrap()));
                let check = self.status.starts_with("ERROR")
                    || latest.is_some_and(|j| {
                        matches!(
                            j.state,
                            State::Failed | State::Cancelled | State::Interrupted
                        )
                    });
                let mood = if self.engine.running() {
                    "REC / ..."
                } else if check {
                    "! CHECK"
                } else if latest.is_some_and(|j| j.state == State::Done) {
                    "OK!"
                } else {
                    "READY"
                };
                let tick = if cfg!(feature = "screenshot") {
                    1
                } else {
                    (ui.input(|i| i.time) * 4.0) as u64
                };
                let dy = if self.engine.running() {
                    (tick as f32 * 0.7).sin() * 3.0
                } else {
                    0.0
                };
                let dx = if check && tick % 20 < 4 {
                    if tick.is_multiple_of(2) { -2.0 } else { 2.0 }
                } else {
                    0.0
                };
                ui.put(
                    rect(24.0 + dx, dy, 80.0, 80.0),
                    egui::Image::new(&self.mascot),
                );
                if tick > 0 && tick.is_multiple_of(16) {
                    // Eye positions on the 32 px icon, displayed at 80 points.
                    for left in [25.0, 42.5] {
                        ui.painter().rect_filled(
                            rect(24.0 + dx + left, 37.5 + dy, 7.5, 10.0),
                            0.0,
                            Color32::from_rgb(220, 248, 232),
                        );
                        ui.painter().rect_filled(
                            rect(24.0 + dx + left, 42.5 + dy, 7.5, 2.5),
                            0.0,
                            BG,
                        );
                    }
                }
                label_at(
                    ui,
                    rect(24.0, 84.0, 80.0, 16.0),
                    mood,
                    if check && !self.engine.running() {
                        AMBER
                    } else {
                        INFO
                    },
                    10.7,
                    false,
                    true,
                );
                in_rect(ui, "title", rect(128.0, 23.0, 180.0, 34.0), pixel_title);
                label_at(
                    ui,
                    rect(129.0, 62.0, 450.0, 22.0),
                    "Download, edit and convert media",
                    MUTED,
                    13.3,
                    false,
                    false,
                );
                if pixel_button(
                    ui,
                    rect(width - 137.0, 14.0, 118.0, 29.0),
                    "วิธีใช้ / HELP",
                    "help",
                    SURFACE_RAISED,
                    true,
                    12.0,
                )
                .clicked()
                {
                    self.help_open = true;
                }
                group_at(
                    ui,
                    rect(16.0, 119.0, width - 32.0, 233.0),
                    "DOWNLOAD / LIVE",
                );
                ui.put(
                    rect(24.0, 153.0, content - 180.0, 44.0),
                    egui::TextEdit::singleline(&mut self.url)
                        .hint_text(
                            RichText::new("Paste a video or playlist URL")
                                .color(MUTED.gamma_multiply(0.76)),
                        )
                        .margin(egui::Margin::symmetric(8, 10)),
                );
                if pixel_button(
                    ui,
                    rect(right, 153.0, 172.0, 44.0),
                    "Paste URL",
                    "paste",
                    SURFACE_RAISED,
                    true,
                    13.3,
                )
                .clicked()
                {
                    match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                        Ok(value) => self.url = value.trim().into(),
                        Err(error) => self.error(error),
                    }
                }
                combo_at(
                    ui,
                    "download-mode",
                    rect(24.0, 207.0, content - 180.0, 40.0),
                    &mut self.mode,
                    &media::MODE_LABELS,
                    true,
                );
                if pixel_button(
                    ui,
                    rect(right, 205.0, 172.0, 44.0),
                    "DOWNLOAD",
                    "download",
                    GREEN,
                    true,
                    13.3,
                )
                .clicked()
                    && let Err(error) = self.enqueue(&self.url.clone(), false)
                {
                    self.error(error);
                }
                ui.painter()
                    .rect_filled(rect(24.0, 260.0, content - 376.0, 42.0), 0.0, SURFACE);
                ui.painter().rect_stroke(
                    rect(24.0, 260.0, content - 376.0, 42.0),
                    0.0,
                    egui::Stroke::new(1.0_f32, BORDER),
                    egui::StrokeKind::Inside,
                );
                label_at(
                    ui,
                    rect(27.0, 260.0, content - 382.0, 42.0),
                    &self.settings.output_directory.display().to_string(),
                    MUTED,
                    12.0,
                    false,
                    false,
                );
                if pixel_button(
                    ui,
                    rect(width - 392.0, 258.0, 188.0, 44.0),
                    "Choose folder",
                    "folder",
                    SURFACE_RAISED,
                    true,
                    13.3,
                )
                .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_directory(&self.settings.output_directory)
                        .pick_folder()
                {
                    self.settings.output_directory = path;
                    self.save_settings();
                }
                if pixel_button(
                    ui,
                    rect(right, 258.0, 172.0, 44.0),
                    "Open folder",
                    "open-folder",
                    SURFACE_RAISED,
                    true,
                    13.3,
                )
                .clicked()
                    && let Err(error) = process::open(&self.settings.output_directory)
                {
                    self.error(error);
                }
                if self.mode == media::TIME_RANGE_MODE {
                    for (x, label, time) in [
                        (24.0, "Start", &mut self.download_start),
                        (178.0, "End", &mut self.download_end),
                    ] {
                        label_at(
                            ui,
                            rect(x, 312.0, 44.0, 35.0),
                            label,
                            INFO,
                            12.0,
                            false,
                            false,
                        );
                        ui.put(
                            rect(x + 45.0, 312.0, 102.0, 35.0),
                            egui::TextEdit::singleline(time),
                        );
                    }
                } else if self.mode == media::SUBTITLES_MODE {
                    combo_at(
                        ui,
                        "subtitle-language",
                        rect(24.0, 312.0, 150.0, 35.0),
                        &mut self.subtitle_language,
                        &["English", "Thai", "All languages"],
                        true,
                    );
                    label_at(
                        ui,
                        rect(184.0, 312.0, 147.0, 35.0),
                        "SRT subtitles",
                        INFO,
                        12.0,
                        false,
                        false,
                    );
                } else if self.mode == media::SELECT_PLAYLIST_MODE {
                    label_at(
                        ui,
                        rect(24.0, 312.0, 305.0, 35.0),
                        "Select items after opening the playlist",
                        INFO,
                        12.0,
                        false,
                        false,
                    );
                } else if media::audio_mode(self.download_mode()) {
                    combo_at(
                        ui,
                        "audio-format",
                        rect(24.0, 312.0, 163.0, 35.0),
                        &mut self.audio_format,
                        &AUDIO_FORMATS,
                        true,
                    );
                    if media::audio_encoder(AUDIO_FORMATS[self.audio_format]).is_some() {
                        combo_at(
                            ui,
                            "audio-quality",
                            rect(200.0, 312.0, 136.0, 35.0),
                            &mut self.audio_quality,
                            &["Best quality", "320 kbps", "192 kbps", "128 kbps"],
                            true,
                        );
                    } else {
                        label_at(
                            ui,
                            rect(200.0, 312.0, 136.0, 35.0),
                            "Lossless",
                            INFO,
                            12.0,
                            false,
                            false,
                        );
                    }
                } else {
                    combo_at(
                        ui,
                        "quality",
                        rect(24.0, 312.0, 163.0, 35.0),
                        &mut self.quality,
                        &QUALITIES,
                        true,
                    );
                    in_rect(ui, "auto-mp4", rect(200.0, 316.0, 145.0, 28.0), |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        ui.add(egui::Checkbox::new(
                            &mut self.auto_mp4,
                            RichText::new("TS -> MP4 (keep TS)").font(FontId::proportional(12.0)),
                        ));
                    });
                }
                let compact = width < 836.0;
                let (choose_width, clear_width, queue_width) = if compact {
                    (58.0, 52.0, 162.0)
                } else {
                    (72.0, 66.0, 204.0)
                };
                let queue_x = width - 24.0 - queue_width;
                let choose_x = (queue_x - choose_width - clear_width - 12.0).min(if compact {
                    460.0
                } else {
                    560.0
                });
                let cookie_name = self
                    .settings
                    .cookies_file
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy());
                let cookie = match &cookie_name {
                    Some(_) if compact => "ON",
                    Some(name) => name.as_ref(),
                    None => "OFF",
                };
                let response = ui.put(
                    rect(345.0, 317.0, choose_x - 350.0, 25.0),
                    egui::Label::new(
                        RichText::new(format!("Cookies: {cookie}"))
                            .monospace()
                            .size(10.7)
                            .color(MUTED),
                    )
                    .truncate()
                    .show_tooltip_when_elided(false),
                );
                if let Some(name) = cookie_name {
                    response.on_hover_text(name.as_ref());
                }
                if pixel_button(
                    ui,
                    rect(choose_x, 317.0, choose_width, 25.0),
                    "Choose",
                    "",
                    SURFACE_RAISED,
                    !local_busy,
                    12.0,
                )
                .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Netscape cookies", &["txt", "cookies"])
                        .add_filter("All files", &["*"])
                        .pick_file()
                {
                    self.settings.cookies_file = Some(path);
                    self.save_settings();
                }
                if pixel_button(
                    ui,
                    rect(choose_x + choose_width + 4.0, 317.0, clear_width, 25.0),
                    "Clear",
                    "",
                    SURFACE_RAISED,
                    !local_busy,
                    12.0,
                )
                .clicked()
                {
                    self.settings.cookies_file = None;
                    self.save_settings();
                }
                if pixel_button(
                    ui,
                    rect(queue_x, 308.0, queue_width, 44.0),
                    "Queue / History",
                    "history",
                    SURFACE_RAISED,
                    true,
                    13.3,
                )
                .clicked()
                {
                    self.batch_urls = self.url.clone();
                    self.queue_open = true;
                }
                group_at(ui, rect(24.0, 358.0, content, 85.0), "CONVERT");
                let options_width = match self.convert_mode {
                    media::COMPRESS_MODE => 276.0,
                    media::EXTRACT_AUDIO_MODE => 244.0,
                    _ => 0.0,
                };
                let mode_width = content - 271.0 - options_width;
                combo_at(
                    ui,
                    "convert-mode",
                    rect(33.0, 389.0, mode_width, 40.0),
                    &mut self.convert_mode,
                    &media::CONVERT_MODES,
                    !local_busy,
                );
                if options_width > 0.0 {
                    let x = 33.0 + mode_width + 8.0;
                    match self.convert_mode {
                        media::COMPRESS_MODE => {
                            combo_at(
                                ui,
                                "compress-limit",
                                rect(x, 389.0, 134.0, 40.0),
                                &mut self.compress_limit,
                                &["Keep res.", "Up to 1080p", "Up to 720p"],
                                !local_busy,
                            );
                            combo_at(
                                ui,
                                "compress-level",
                                rect(x + 142.0, 389.0, 118.0, 40.0),
                                &mut self.compress_level,
                                &media::COMPRESS_LEVELS,
                                !local_busy,
                            );
                        }
                        media::EXTRACT_AUDIO_MODE => {
                            combo_at(
                                ui,
                                "extract-format",
                                rect(x, 389.0, 86.0, 40.0),
                                &mut self.extract_format,
                                &AUDIO_FORMATS,
                                !local_busy,
                            );
                            if media::audio_encoder(AUDIO_FORMATS[self.extract_format]).is_some() {
                                combo_at(
                                    ui,
                                    "extract-quality",
                                    rect(x + 94.0, 389.0, 134.0, 40.0),
                                    &mut self.extract_quality,
                                    &["Best quality", "320 kbps", "192 kbps", "128 kbps"],
                                    !local_busy,
                                );
                            } else {
                                label_at(
                                    ui,
                                    rect(x + 94.0, 389.0, 134.0, 40.0),
                                    "Lossless",
                                    INFO,
                                    12.0,
                                    false,
                                    false,
                                );
                            }
                        }
                        _ => {}
                    }
                }
                if pixel_button(
                    ui,
                    rect(width - 278.0, 387.0, 245.0, 44.0),
                    "CHOOSE VIDEO FILES",
                    "convert",
                    TEAL,
                    idle,
                    13.3,
                )
                .clicked()
                {
                    self.convert_files();
                }
                group_at(ui, rect(24.0, 452.0, content, 121.0), "EDIT / TOOLS");
                let bw = (content - 38.0) / 3.0;
                for (i, (text, icon)) in [
                    ("Install tools", "tools"),
                    ("Join clips", "join"),
                    ("Cut clip", "cut"),
                    ("Update yt-dlp", "update"),
                    ("TUI", "terminal"),
                    ("Cancel task", "stop"),
                ]
                .iter()
                .enumerate()
                {
                    let enabled = match i {
                        4 => true,
                        5 => self.engine.history.jobs.iter().any(|j| {
                            matches!(j.kind.as_str(), "local" | "tools")
                                && matches!(j.state, State::Running | State::Cancelling)
                        }),
                        _ => idle,
                    };
                    if pixel_button(
                        ui,
                        rect(
                            33.0 + (i % 3) as f32 * (bw + 6.0),
                            469.0 + (i / 3) as f32 * 52.0,
                            if i % 3 == 2 { bw + 7.0 } else { bw },
                            44.0,
                        ),
                        text,
                        icon,
                        SURFACE_RAISED,
                        enabled,
                        13.3,
                    )
                    .clicked()
                    {
                        match i {
                            0 => self.tool_job(false),
                            1 | 2 => {
                                if let Err(error) = tools::require("ffmpeg") {
                                    self.error(error);
                                } else if i == 1 {
                                    self.join_files.clear();
                                    self.join_selected.clear();
                                    self.join_anchor = None;
                                    self.join_mode = 0;
                                    self.join_open = true;
                                } else {
                                    self.clip.reset();
                                    self.cut_open = true;
                                }
                            }
                            3 => self.tool_job(true),
                            4 => {
                                if let Err(error) = process::open_cli() {
                                    self.error(error);
                                }
                            }
                            _ => {
                                let ids: Vec<_> = self
                                    .engine
                                    .history
                                    .jobs
                                    .iter()
                                    .filter(|j| {
                                        matches!(j.kind.as_str(), "local" | "tools")
                                            && j.state == State::Running
                                    })
                                    .map(|j| j.id.clone())
                                    .collect();
                                for id in ids {
                                    self.engine.cancel(&id);
                                }
                            }
                        }
                    }
                }
                let active = self
                    .engine
                    .history
                    .jobs
                    .iter()
                    .find(|j| matches!(j.state, State::Running | State::Cancelling));
                let recent = self
                    .engine
                    .history
                    .jobs
                    .iter()
                    .filter(|j| j.finished.is_some())
                    .max_by(|a, b| a.finished.unwrap().total_cmp(&b.finished.unwrap()));
                let queued = self
                    .engine
                    .history
                    .jobs
                    .iter()
                    .filter(|j| j.state == State::Queued)
                    .count();
                let status = if let Some(job) = active {
                    if job.kind.is_empty() {
                        format!(
                            "Queue: {} running | Progress {} | {}",
                            self.engine.active_count(),
                            job.progress
                                .map(|p| format!("{p:.1}%"))
                                .unwrap_or_else(|| "--".into()),
                            job.detail
                        )
                    } else {
                        job.detail.clone()
                    }
                } else if self.engine.history.paused && queued > 0 {
                    format!("Queue paused | {queued} waiting - press Resume queue")
                } else if let Some(job) = recent {
                    completion_status(job)
                } else {
                    self.status.clone()
                };
                label_at(
                    ui,
                    rect(27.0, 577.0, content, 22.0),
                    &status,
                    if status.starts_with("ERROR")
                        || status.starts_with("Some required tools are missing")
                    {
                        RED
                    } else {
                        TEXT
                    },
                    13.3,
                    true,
                    false,
                );
                let progress_height = if active.is_some() { 20.0 } else { 0.0 };
                if let Some(job) = active {
                    in_rect(
                        ui,
                        "main-progress",
                        rect(24.0, 601.0, content, 16.0),
                        |ui| pixel_progress(ui, job.progress.map(|p| p as f32 / 100.0)),
                    );
                }
                let log_height = (height - 693.0 - progress_height).max(70.0);
                let log_rect = rect(24.0, 601.0 + progress_height, content, log_height);
                group_at(ui, log_rect, "OUTPUT / LOG");
                in_rect(
                    ui,
                    "log-area",
                    log_rect
                        .shrink2(Vec2::new(9.0, 16.0))
                        .translate(Vec2::new(0.0, 8.0)),
                    |ui| {
                        style_scrollbar(ui, self.engine.log.is_empty());
                        egui::ScrollArea::vertical()
                            .id_salt("main-log")
                            .max_height(ui.available_height())
                            .min_scrolled_height(ui.available_height())
                            .auto_shrink([false, false])
                            .scroll_bar_visibility(
                                egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                            )
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                let text = self
                                    .engine
                                    .log
                                    .iter()
                                    .map(String::as_str)
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                log_text(ui, &text, false);
                            });
                    },
                );
                let sy = 601.0 + progress_height + log_height + 6.0;
                pixel_frame(ui.painter(), rect(24.0, sy, content, 50.0), SURFACE, BORDER);
                let dependencies = [
                    ("yt-dlp", "yt-dlp"),
                    ("FFmpeg", "ffmpeg"),
                    ("ffprobe", "ffprobe"),
                    ("Streamlink", "streamlink"),
                ]
                .iter()
                .map(|(label, name)| {
                    format!(
                        "{label}: {}",
                        if tools::find(name).is_some() {
                            "Ready"
                        } else {
                            "Missing"
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("  |  ");
                label_at(
                    ui,
                    rect(37.0, sy + 5.0, content - 26.0, 20.0),
                    &format!(
                        "SYSTEM  |  {}  |  {dependencies}",
                        if cfg!(windows) { "Windows" } else { "Linux" }
                    ),
                    MUTED,
                    10.7,
                    false,
                    false,
                );
                let hardware = self
                    .hardware
                    .as_ref()
                    .map(|h| {
                        format!(
                            "Auto Live: {} | Auto H.264: {} | GPU: {} | CPU: {}",
                            h.encoder, h.file_encoder, h.gpu, h.cpu,
                        )
                    })
                    .unwrap_or_else(|| "Testing CPU and GPU encoding speed...".into());
                label_at(
                    ui,
                    rect(37.0, sy + 26.0, content - 26.0, 18.0),
                    &format!("HARDWARE  |  {hardware}"),
                    INFO,
                    10.7,
                    false,
                    false,
                );
                let free = self
                    .free_space
                    .map(|bytes| format!(" | {:.1} GiB free", bytes as f64 / model::GIB as f64))
                    .unwrap_or_default();
                let workflow = if !self.engine.warning.is_empty() {
                    self.engine.warning.clone()
                } else {
                    format!(
                        "{} running | {queued} queued | {}{}{free}",
                        self.engine.active_count(),
                        if self.engine.running() {
                            "Working"
                        } else if recent.is_some_and(|j| j.state == State::Done) {
                            "Done"
                        } else if recent.is_some() {
                            "Check log"
                        } else {
                            "Ready"
                        },
                        if self.engine.history.paused {
                            " | Queue paused"
                        } else {
                            ""
                        }
                    )
                };
                label_at(
                    ui,
                    rect(27.0, sy + 52.0, content, 19.0),
                    &workflow,
                    if self.engine.warning.is_empty() {
                        MUTED
                    } else {
                        AMBER
                    },
                    12.0,
                    false,
                    false,
                );
            });
    }

    fn convert_files(&mut self) {
        if let Err(error) = tools::require("ffmpeg") {
            self.error(error);
            return;
        }
        if let Some(inputs) = video_dialog()
            .set_title("Choose video files to convert")
            .pick_files()
            && let Err(error) = self.queue_conversions(inputs)
        {
            self.error(error);
        }
    }

    fn queue_conversions(&mut self, inputs: Vec<PathBuf>) -> Result<()> {
        let format = AUDIO_FORMATS[self.extract_format];
        let conversion = match self.convert_mode {
            media::COMPRESS_MODE => media::Conversion::Compress {
                limit: media::COMPRESS_LIMITS[self.compress_limit],
                level: self.compress_level,
            },
            media::EXTRACT_AUDIO_MODE => media::Conversion::Audio {
                format,
                bitrate: media::audio_encoder(format).and(AUDIO_BITRATES[self.extract_quality]),
            },
            mode => media::Conversion::Video(mode),
        };
        let multiple = inputs.len() > 1;
        let mut added = 0;
        let mut errors = Vec::new();
        for input in inputs {
            let result = (|| -> Result<()> {
                let (mut output, mut arguments) = media::convert(&input, conversion)?;
                let pending = |path: &Path| {
                    self.engine.history.jobs.iter().any(|job| {
                        job.kind == "local"
                            && !job.state.terminal()
                            && job.arguments.last().is_some_and(|name| {
                                if cfg!(windows) {
                                    name.to_lowercase() == path.to_string_lossy().to_lowercase()
                                } else {
                                    Path::new(name) == path
                                }
                            })
                    })
                };
                if multiple || pending(&output) {
                    let base = output.clone();
                    let name = base.file_stem().unwrap_or_default().to_string_lossy();
                    let extension = base.extension().unwrap_or_default().to_string_lossy();
                    let mut number = 2;
                    while output.try_exists()? || pending(&output) {
                        output.set_file_name(format!("{name} ({number}).{extension}"));
                        number += 1;
                    }
                    *arguments.last_mut().unwrap() = media::text(&output);
                }
                self.add_edit(
                    "Converting",
                    &output,
                    arguments,
                    Default::default(),
                    vec![input.clone()],
                )
            })();
            match result {
                Ok(()) => added += 1,
                Err(error) => errors.push(format!("{}: {error}", input.display())),
            }
        }
        if added > 0 {
            self.queue_open = true;
            self.status = format!("Added {added} files to the conversion queue");
        }
        ensure!(
            errors.is_empty(),
            "Added {added} files. Could not add:\n{}",
            errors.join("\n")
        );
        Ok(())
    }

    fn queue_action(&mut self, label: &str) {
        if let Some(job) = self
            .selected
            .as_ref()
            .and_then(|id| self.engine.job(id))
            .cloned()
        {
            let result = match label {
                "Cancel" => {
                    self.engine.cancel(&job.id);
                    Ok(())
                }
                "Delete" => {
                    if confirm(
                        "Delete job",
                        "Remove this entry from Queue / History? Media files will be kept.",
                    ) {
                        self.engine.delete(&job.id)
                    } else {
                        Ok(())
                    }
                }
                "Retry" => {
                    if job.kind == "local" && self.engine.running() {
                        self.error("Finish active work before retrying this edit");
                        return;
                    }
                    let exists = job.kind == "local"
                        && job.arguments.last().is_some_and(|p| Path::new(p).exists());
                    if !exists
                        || confirm(
                            "Retry edit",
                            "The output exists. Replace it by running this edit again?",
                        )
                    {
                        self.engine.retry(&job.id, exists).map(|id| {
                            self.selected = Some(id);
                        })
                    } else {
                        Ok(())
                    }
                }
                "Files" => {
                    if let Some(file) = job.outputs.iter().find(|p| p.is_file()) {
                        self.file_selected = Some(file.clone());
                        self.files_open = true;
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!(
                            "No completed file is available. Use Open folder to inspect partial files."
                        ))
                    }
                }
                "Open folder" => {
                    process::open(&if job.kind == "local" || !job.directory().is_dir() {
                        job.folder.clone()
                    } else {
                        job.directory()
                    })
                }
                _ => {
                    self.detail_open = true;
                    Ok(())
                }
            };

            if let Err(error) = result {
                self.error(error);
            }
        }
    }
    fn queue_ui(&mut self, ui: &mut egui::Ui) {
        modal_parent(ui, self.cut_open, self.join_open);
        let w = ui.max_rect().width();
        let h = ui.max_rect().height();
        pixel_frame(
            ui.painter(),
            rect(12.0, 7.0, w - 24.0, 54.0),
            HEADER,
            BORDER,
        );
        pixel_heading(
            ui,
            rect(447.0, 13.0, w - 567.0, 42.0),
            "Queue / History",
            "history",
            16.0,
        );
        label_at(
            ui,
            rect(19.0, 17.0, 185.0, 35.0),
            "Parallel downloads (1-5):",
            TEXT,
            12.0,
            false,
            false,
        );
        let mut parallel = self.engine.history.limit.to_string();
        if spin_at(
            ui,
            "parallel",
            rect(205.0, 24.0, 36.0, 20.0),
            &mut parallel,
            1.0..=5.0,
            1.0,
        ) && let Ok(limit) = parallel.parse::<usize>()
        {
            self.engine.history.limit = limit.clamp(1, 5);
            self.engine.save();
        }
        let paused = self.engine.history.paused;
        if pixel_button(
            ui,
            rect(256.0, 12.0, 172.0, 44.0),
            if paused {
                "Resume queue"
            } else {
                "Pause queue"
            },
            if paused { "play" } else { "pause" },
            SURFACE_RAISED,
            true,
            13.3,
        )
        .clicked()
        {
            self.engine.history.paused = !paused;
            self.engine.save();
        }
        in_rect(
            ui,
            "history-only",
            rect(w - 108.0, 22.0, 100.0, 25.0),
            |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                ui.add(egui::Checkbox::new(
                    &mut self.history_only,
                    RichText::new("History only").font(FontId::proportional(12.0)),
                ));
            },
        );
        let table = rect(16.0, 68.0, w - 32.0, (h - 420.0).max(100.0));
        pixel_frame(ui.painter(), table, INPUT, BORDER);
        in_rect(ui, "queue-table", table.shrink(2.0), |ui| {
            style_scrollbar(ui, self.engine.history.jobs.is_empty());
            let widths = [
                (w - 50.0) * 0.138,
                (w - 50.0) * 0.123,
                (w - 50.0) * 0.33,
                (w - 50.0) * 0.409,
            ];
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.horizontal(|ui| {
                for (label, width) in ["State", "Progress", "Url", "Detail"].iter().zip(widths) {
                    let (rect, _) =
                        ui.allocate_exact_size(Vec2::new(width, 25.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, SURFACE);
                    ui.painter().rect_stroke(
                        rect,
                        0.0,
                        egui::Stroke::new(1.0_f32, MUTED),
                        egui::StrokeKind::Inside,
                    );
                    label_at(ui, rect, label, TEXT, 12.0, true, true);
                }
            });
            egui::ScrollArea::vertical()
                .id_salt("queue-rows")
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    for job in &self.engine.history.jobs {
                        if self.history_only && !job.state.terminal() {
                            continue;
                        }
                        let (row, response) = ui.allocate_exact_size(
                            Vec2::new(widths.iter().sum(), 26.0),
                            egui::Sense::click(),
                        );
                        if self.selected.as_ref() == Some(&job.id) {
                            ui.painter().rect_filled(row, 0.0, GREEN);
                            ui.painter().rect_filled(
                                rect(row.left(), row.top(), 2.0, row.height()),
                                0.0,
                                MINT,
                            );
                        }
                        let values = [
                            format!("{:?}", job.state),
                            job.progress
                                .map(|p| format!("{p:.1}%"))
                                .unwrap_or_else(|| "--".into()),
                            job.url.clone(),
                            queue_detail(job),
                        ];
                        let mut x = row.left();
                        for (value, width) in values.iter().zip(widths) {
                            label_at(
                                ui,
                                rect(x + 4.0, row.top(), width - 8.0, 26.0),
                                value,
                                TEXT,
                                12.0,
                                false,
                                false,
                            );
                            x += width;
                        }
                        if response.clicked() {
                            self.selected = Some(job.id.clone());
                        }
                        if response.double_clicked() {
                            self.selected = Some(job.id.clone());
                            self.detail_open = true;
                        }
                    }
                });
        });
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) && self.selected.is_some() {
            self.detail_open = true;
        }
        let actions = [
            ("Cancel", "stop", 132.0),
            ("Delete", "delete", 132.0),
            ("Retry", "retry", 124.0),
            ("Files", "file", 124.0),
            ("Open folder", "open-folder", 172.0),
            ("Job log", "log", 140.0),
            ("Clear all", "clear", 142.0),
        ];
        let mut x = 16.0;
        let scale = (w - 32.0) / 1008.0;
        for (label, icon, width) in actions {
            if pixel_button(
                ui,
                rect(x, h - 344.0, width * scale, 44.0),
                label,
                icon,
                SURFACE_RAISED,
                true,
                13.3,
            )
            .clicked()
            {
                if label == "Clear all" {
                    if confirm(
                        "Clear all",
                        "Clear pending jobs and history? Running jobs and media files will be kept.",
                    ) {
                        self.engine.clear();
                    }
                } else {
                    self.queue_action(label);
                }
            }
            x += (width + 6.0) * scale;
        }
        label_at(
            ui,
            rect(19.0, h - 290.0, w - 38.0, 24.0),
            "Add URLs (one per line). Uses the mode, quality and output folder on the main window.",
            TEXT,
            12.0,
            false,
            false,
        );
        ui.put(
            rect(16.0, h - 265.0, w - 32.0, 52.0),
            egui::TextEdit::multiline(&mut self.batch_urls).desired_rows(1),
        );
        pixel_heading(
            ui,
            rect(16.0, h - 203.0, w - 32.0, 24.0),
            "Schedule a Live recording",
            "schedule",
            12.0,
        );
        label_at(
            ui,
            rect(19.0, h - 179.0, w - 38.0, 24.0),
            "Record now: use the main Download button  |  Schedule: enter a future time below  |  Keep PixClip open.",
            AMBER,
            12.0,
            false,
            false,
        );
        label_at(
            ui,
            rect(19.0, h - 151.0, 221.0, 23.0),
            "Start time (YYYY-MM-DD HH:MM):",
            TEXT,
            12.0,
            false,
            false,
        );
        ui.put(
            rect(240.0, h - 150.0, 138.0, 22.0),
            egui::TextEdit::singleline(&mut self.schedule),
        );
        label_at(
            ui,
            rect(389.0, h - 151.0, 400.0, 23.0),
            "Example: 2026-09-08 20:30",
            TEXT,
            12.0,
            false,
            false,
        );
        label_at(
            ui,
            rect(19.0, h - 123.0, 200.0, 23.0),
            "Duration:",
            TEXT,
            12.0,
            false,
            false,
        );
        spin_at(
            ui,
            "duration",
            rect(240.0, h - 122.0, 58.0, 22.0),
            &mut self.minutes,
            1.0..=10080.0,
            1.0,
        );
        label_at(
            ui,
            rect(389.0, h - 123.0, 300.0, 23.0),
            "minutes",
            TEXT,
            12.0,
            false,
            false,
        );
        label_at(
            ui,
            rect(19.0, h - 95.0, 200.0, 23.0),
            "Free space:",
            TEXT,
            12.0,
            false,
            false,
        );
        let mut reserve = self.reserve.to_string();
        if spin_at(
            ui,
            "disk-reserve",
            rect(240.0, h - 94.0, 58.0, 22.0),
            &mut reserve,
            0.25..=100.0,
            0.25,
        ) && let Ok(value) = reserve.parse::<f64>()
            && value.is_finite()
        {
            self.reserve = value.clamp(0.25, 100.0);
        }
        label_at(
            ui,
            rect(389.0, h - 95.0, 300.0, 23.0),
            "GiB free space",
            TEXT,
            12.0,
            false,
            false,
        );
        label_at(
            ui,
            rect(19.0, h - 65.0, w - 38.0, 24.0),
            "Format: YYYY-MM-DD HH:MM and the time must be in the future | Live duration includes reconnect waits.",
            MUTED,
            12.0,
            false,
            false,
        );
        if pixel_button(
            ui,
            rect(16.0, h - 41.0, 220.0, 29.0),
            "Add downloads now",
            "download",
            GREEN,
            true,
            13.3,
        )
        .clicked()
        {
            match self.enqueue(&self.batch_urls.clone(), false) {
                Ok(()) => self.batch_urls.clear(),
                Err(error) => self.error(error),
            }
        }
        if pixel_button(
            ui,
            rect(244.0, h - 41.0, 188.0, 29.0),
            "Schedule Live",
            "schedule",
            SURFACE_RAISED,
            true,
            13.3,
        )
        .clicked()
        {
            match self.enqueue(&self.batch_urls.clone(), true) {
                Ok(()) => self.batch_urls.clear(),
                Err(error) => self.error(error),
            }
        }
    }

    fn join_ui(&mut self, ui: &mut egui::Ui) {
        if self.join_checking {
            ui.disable();
        }
        let w = ui.max_rect().width();
        let h = ui.max_rect().height();
        pixel_heading(
            ui,
            rect(24.0, 13.0, w - 48.0, 43.0),
            "Join video clips",
            "join",
            21.0,
        );
        label_at(
            ui,
            rect(27.0, 60.0, w - 54.0, 22.0),
            "Add clips in playback order. Fast mode requires matching codecs and formats.",
            MUTED,
            12.0,
            false,
            false,
        );
        let list = rect(24.0, 90.0, w - 266.0, h - 212.0);
        pixel_frame(ui.painter(), list, INPUT, BORDER);
        in_rect(ui, "join-list", list.shrink(3.0), |ui| {
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (index, file) in self.join_files.iter().enumerate() {
                        if ui
                            .selectable_label(
                                self.join_selected.contains(&index),
                                format!(
                                    "{}. {}",
                                    index + 1,
                                    file.file_name().unwrap_or_default().to_string_lossy()
                                ),
                            )
                            .clicked()
                        {
                            let modifiers = ui.input(|i| i.modifiers);
                            select_join_row(
                                &mut self.join_selected,
                                &mut self.join_anchor,
                                index,
                                modifiers.command,
                                modifiers.shift,
                            );
                        }
                    }
                });
        });
        for (i, (label, icon)) in [
            ("Add clips", "folder"),
            ("Remove selected", "delete"),
            ("Move up", ""),
            ("Move down", ""),
            ("Clear all", "clear"),
        ]
        .iter()
        .enumerate()
        {
            if pixel_button(
                ui,
                rect(w - 228.0, 90.0 + i as f32 * 52.0, 204.0, 44.0),
                label,
                icon,
                SURFACE_RAISED,
                true,
                13.3,
            )
            .clicked()
            {
                match i {
                    0 => {
                        if let Some(files) = video_dialog()
                            .set_title("Select clips in the order you want them joined")
                            .pick_files()
                        {
                            for file in files {
                                if !self.join_files.contains(&file) {
                                    self.join_files.push(file);
                                }
                            }
                        }
                    }
                    1 => {
                        for index in self.join_selected.iter().rev() {
                            self.join_files.remove(*index);
                        }
                        self.join_selected.clear();
                        self.join_anchor = None;
                    }
                    2 => {
                        if let Some(index) = self.join_selected.first().copied().filter(|i| *i > 0)
                        {
                            self.join_files.swap(index, index - 1);
                            self.join_selected = [index - 1].into();
                            self.join_anchor = Some(index - 1);
                        }
                    }
                    3 => {
                        if let Some(index) = self
                            .join_selected
                            .first()
                            .copied()
                            .filter(|i| *i + 1 < self.join_files.len())
                        {
                            self.join_files.swap(index, index + 1);
                            self.join_selected = [index + 1].into();
                            self.join_anchor = Some(index + 1);
                        }
                    }
                    _ => {
                        self.join_files.clear();
                        self.join_selected.clear();
                        self.join_anchor = None;
                    }
                }
            }
        }
        label_at(
            ui,
            rect(27.0, h - 116.0, w - 54.0, 24.0),
            &if self.join_checking {
                "Checking clip formats...".into()
            } else {
                format!("{} clips", self.join_files.len())
            },
            MUTED,
            12.0,
            false,
            false,
        );
        label_at(
            ui,
            rect(27.0, h - 82.0, 200.0, 24.0),
            "Join mode",
            INFO,
            13.3,
            true,
            false,
        );
        combo_at(
            ui,
            "join-mode",
            rect(24.0, h - 54.0, w - 324.0, 36.0),
            &mut self.join_mode,
            &media::JOIN_MODES,
            true,
        );
        if pixel_button(
            ui,
            rect(w - 294.0, h - 81.0, 270.0, 60.0),
            "JOIN AND SAVE MP4",
            "join",
            TEAL,
            true,
            13.3,
        )
        .clicked()
        {
            if self.join_files.len() < 2 {
                self.error("Add at least two clips before joining.");
                return;
            }
            if let Some(output) = rfd::FileDialog::new()
                .set_title("Save joined video as")
                .set_directory(self.join_files[0].parent().unwrap_or(Path::new(".")))
                .add_filter("MP4", &["mp4"])
                .set_file_name("joined.mp4")
                .save_file()
            {
                let files = self.join_files.clone();
                if self.join_mode == 0 {
                    self.join_checking = true;
                    let sender = self.sender.clone();
                    let ctx = ui.ctx().clone();
                    thread::spawn(move || {
                        let result = media::fast_join_compatible(&files).map_err(|e| e.to_string());
                        let _ = sender.send(UiEvent::JoinChecked(output, files, result));
                        ctx.request_repaint();
                    });
                } else if let Err(error) = self.submit_join(&output, files) {
                    self.error(error);
                }
            }
        }
    }

    fn submit_join(&mut self, output: &Path, files: Vec<PathBuf>) -> Result<()> {
        let list = output
            .parent()
            .context("Choose an output directory")?
            .join("pixclip-concat.txt");
        let (arguments, content) = media::join(&files, output, &list, self.join_mode == 1)?;
        self.add_edit(
            "Joining clips",
            output,
            arguments,
            [(media::text(&list), content)].into(),
            files,
        )?;
        self.join_open = false;
        Ok(())
    }

    fn help_ui(&mut self, ui: &mut egui::Ui) {
        modal_parent(ui, self.cut_open, self.join_open);
        let w = ui.max_rect().width();
        let h = ui.max_rect().height();
        let translations: serde_json::Value =
            serde_json::from_str(include_str!("../docs/help-content.json")).expect("Bundled help");
        pixel_frame(ui.painter(), rect(8.0, 4.0, w - 16.0, 88.0), HEADER, BORDER);
        ui.put(rect(18.0, 6.0, 80.0, 80.0), egui::Image::new(&self.mascot));
        label_at(
            ui,
            rect(110.0, 21.0, w - 340.0, 30.0),
            "PIXCLIP // QUICK GUIDE",
            MINT,
            21.3,
            true,
            false,
        );
        if pixel_button(
            ui,
            rect(w - 222.0, 39.0, 100.0, 44.0),
            "TH",
            "",
            if self.thai { GREEN } else { SURFACE_RAISED },
            true,
            13.3,
        )
        .clicked()
        {
            self.thai = true;
        }
        if pixel_button(
            ui,
            rect(w - 118.0, 39.0, 100.0, 44.0),
            "EN",
            "",
            if self.thai { SURFACE_RAISED } else { GREEN },
            true,
            13.3,
        )
        .clicked()
        {
            self.thai = false;
        }
        let data = &translations[if self.thai { "th" } else { "en" }];
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(
            data["window_title"].as_str().unwrap().into(),
        ));
        label_at(
            ui,
            rect(w - 222.0, 14.0, 200.0, 22.0),
            data["language"].as_str().unwrap_or(""),
            MUTED,
            10.7,
            false,
            false,
        );
        label_at(
            ui,
            rect(110.0, 54.0, w - 350.0, 26.0),
            data["subtitle"].as_str().unwrap_or(""),
            MUTED,
            12.0,
            false,
            false,
        );
        let body = rect(18.0, 106.0, w - 36.0, h - 178.0);
        pixel_frame(ui.painter(), body, INPUT, BORDER);
        in_rect(
            ui,
            "help-content",
            body.shrink2(Vec2::new(14.0, 10.0)),
            |ui| {
                style_scrollbar(ui, false);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        for (index, section) in
                            data["sections"].as_array().unwrap().iter().enumerate()
                        {
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new(section[0].as_str().unwrap())
                                    .color(INFO)
                                    .strong()
                                    .size(14.7),
                            );
                            ui.label(RichText::new(section[1].as_str().unwrap()).size(13.3));
                            if index == 0 {
                                ui.label(
                                    RichText::new(data["tip"].as_str().unwrap())
                                        .color(AMBER)
                                        .size(12.0),
                                );
                            }
                        }
                    });
            },
        );
        if pixel_button(
            ui,
            rect(w - 176.0, h - 58.0, 158.0, 44.0),
            data["close"].as_str().unwrap(),
            "",
            SURFACE_RAISED,
            true,
            13.3,
        )
        .clicked()
        {
            self.help_open = false;
        }
        if pixel_button(
            ui,
            rect(w - 372.0, h - 58.0, 188.0, 44.0),
            data["full"].as_str().unwrap(),
            "log",
            SURFACE_RAISED,
            true,
            13.3,
        )
        .clicked()
            && let Err(error) = process::open(&model::root().join("docs/README-PixClip.md"))
        {
            self.error(error);
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.queue_open {
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("queue"),
                egui::ViewportBuilder::default()
                    .with_title("PixClip - Queue / History / Schedule")
                    .with_inner_size([1040.0, 720.0])
                    .with_min_inner_size([780.0, 650.0]),
                |ctx, _| {
                    if ctx.input(|i| {
                        i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)
                    }) {
                        self.queue_open = false;
                    }
                    egui::CentralPanel::default()
                        .frame(egui::Frame::new().fill(BG))
                        .show(ctx, |ui| self.queue_ui(ui));
                },
            );
        }
        if self.cut_open {
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("cut"),
                egui::ViewportBuilder::default()
                    .with_title("Cut a clip segment")
                    .with_inner_size(clip::SIZE)
                    .with_min_inner_size(clip::MIN_SIZE),
                |ctx, _| {
                    if ctx.input(|i| {
                        i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)
                    }) {
                        self.cut_open = false;
                    }
                    egui::CentralPanel::default()
                        .frame(egui::Frame::new().fill(BG))
                        .show(ctx, |ui| self.cut_ui(ui));
                },
            );
        }
        if self.join_open {
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("join"),
                egui::ViewportBuilder::default()
                    .with_title("Join clips with FFmpeg")
                    .with_inner_size([760.0, 560.0])
                    .with_min_inner_size([680.0, 480.0]),
                |ctx, _| {
                    if ctx.input(|i| {
                        i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)
                    }) {
                        self.join_open = false;
                    }
                    egui::CentralPanel::default()
                        .frame(egui::Frame::new().fill(BG))
                        .show(ctx, |ui| self.join_ui(ui));
                },
            );
        }
        if self.help_open {
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("help"),
                egui::ViewportBuilder::default()
                    .with_title("PixClip - วิธีใช้ / HELP")
                    .with_inner_size([900.0, 700.0])
                    .with_min_inner_size([680.0, 520.0]),
                |ctx, _| {
                    if ctx.input(|i| {
                        i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)
                    }) {
                        self.help_open = false;
                    }
                    egui::CentralPanel::default()
                        .frame(egui::Frame::new().fill(BG))
                        .show(ctx, |ui| self.help_ui(ui));
                },
            );
        }
        if let Some(job) = self
            .selected
            .as_ref()
            .and_then(|id| self.engine.job(id))
            .cloned()
        {
            if self.detail_open {
                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("job-log"),
                    egui::ViewportBuilder::default()
                        .with_title(format!("PixClip - {:?}", job.state))
                        .with_inner_size([700.0, 450.0]),
                    |ctx, _| {
                        if ctx.input(|i| {
                            i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)
                        }) {
                            self.detail_open = false;
                        }
                        egui::CentralPanel::default()
                            .frame(egui::Frame::new().fill(BG).inner_margin(12))
                            .show(ctx, |ui| {
                                flow_heading(ui, &format!("Job log / {:?}", job.state), "log");
                                list_panel(ui, |ui| {
                                    egui::ScrollArea::both().auto_shrink([false, false]).show(
                                        ui,
                                        |ui| {
                                            log_text(
                                                ui,
                                                &format!("{}\n{}", job.detail, job.logs.join("\n")),
                                                true,
                                            );
                                        },
                                    );
                                });
                            });
                    },
                );
            }
            if self.files_open {
                let directory = job.directory();
                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("completed-files"),
                    egui::ViewportBuilder::default()
                        .with_title("PixClip - Completed files")
                        .with_inner_size([760.0, 300.0])
                        .with_min_inner_size([460.0, 220.0]),
                    |ctx, _| {
                        if ctx.input(|i| {
                            i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)
                        }) {
                            self.files_open = false;
                        }
                        egui::CentralPanel::default()
                            .frame(egui::Frame::new().fill(BG).inner_margin(8))
                            .show(ctx, |ui| {
                                let mut open = None;
                                flow_heading(ui, "Completed files", "file");
                                let height = (ui.available_height() - 65.0).max(40.0);
                                list_panel(ui, |ui| {
                                    egui::ScrollArea::both()
                                        .auto_shrink([false, false])
                                        .max_height(height)
                                        .show(ui, |ui| {
                                            for path in job.outputs.iter().filter(|p| p.is_file()) {
                                                let response = ui.selectable_label(
                                                    self.file_selected.as_ref() == Some(path),
                                                    path.strip_prefix(&directory)
                                                        .unwrap_or(path)
                                                        .display()
                                                        .to_string(),
                                                );
                                                if response.clicked() {
                                                    self.file_selected = Some(path.clone());
                                                }
                                                if response.double_clicked() {
                                                    open = Some(path.clone());
                                                }
                                            }
                                        });
                                });
                                let selected = self
                                    .file_selected
                                    .as_ref()
                                    .filter(|path| job.outputs.contains(path) && path.is_file());
                                let is_video = selected
                                    .and_then(|path| path.extension()?.to_str())
                                    .is_some_and(|extension| {
                                        VIDEO_EXTENSIONS
                                            .iter()
                                            .any(|video| video.eq_ignore_ascii_case(extension))
                                    });
                                let mut cut = false;
                                ui.horizontal(|ui| {
                                    if flow_button(
                                        ui,
                                        "Open selected file",
                                        "file",
                                        204.0,
                                        SURFACE_RAISED,
                                        selected.is_some(),
                                    )
                                    .clicked()
                                    {
                                        open = selected.cloned();
                                    }
                                    cut = flow_button(
                                        ui,
                                        "Open in Cut clip",
                                        "cut",
                                        190.0,
                                        TEAL,
                                        is_video,
                                    )
                                    .clicked();
                                });
                                if cut && let Some(path) = selected.cloned() {
                                    match tools::require("ffmpeg") {
                                        Ok(_) => {
                                            self.load_cut_source(path);
                                            if self.cut_open {
                                                ctx.send_viewport_cmd_to(
                                                    egui::ViewportId::from_hash_of("cut"),
                                                    egui::ViewportCommand::Focus,
                                                );
                                            }
                                            self.cut_open = true;
                                            self.files_open = false;
                                        }
                                        Err(error) => self.error(error),
                                    }
                                }
                                if let Some(path) = open
                                    && let Err(error) = process::open(&path)
                                {
                                    self.error(error);
                                }
                            });
                    },
                );
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.engine.tick();
        self.show_spotify_review(ctx);
        self.show_playlist_review(ctx);
        #[cfg(feature = "screenshot")]
        {
            self.screenshot_frames = self.screenshot_frames.saturating_add(1);
            if let Some(path) = std::env::var_os("PIXCLIP_SCREENSHOT_TO") {
                let screenshot = ctx.input(|input| {
                    input.events.iter().find_map(|event| {
                        if let egui::Event::Screenshot { image, .. } = event {
                            Some(image.clone())
                        } else {
                            None
                        }
                    })
                });
                if let Some(image) = screenshot {
                    let bytes: Vec<_> = image
                        .pixels
                        .iter()
                        .flat_map(|pixel| pixel.to_array())
                        .collect();
                    image::save_buffer(
                        path,
                        &bytes,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    )
                    .expect("UI screenshot could not be saved");
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                if self.screenshot_frames == 8 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                        egui::UserData::default(),
                    ));
                }
            }
        }
        #[cfg(feature = "screenshot")]
        if let Ok(view) = std::env::var("PIXCLIP_SCREENSHOT_VIEW") {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(BG))
                .show(ctx, |ui| match view.as_str() {
                    "queue" => self.queue_ui(ui),
                    "cut" => self.cut_ui(ui),
                    "join" => self.join_ui(ui),
                    "help" => self.help_ui(ui),
                    _ => {}
                });
            ctx.request_repaint();
            return;
        }
        for event in self.receiver.try_iter().collect::<Vec<_>>() {
            match event {
                UiEvent::Hardware(hardware) => self.hardware = Some(hardware),
                UiEvent::JoinChecked(output, files, result) => {
                    self.join_checking = false;
                    if !self.join_open {
                        continue;
                    }
                    match result {
                        Ok(false) => {
                            if !confirm(
                                "Fast join may fail",
                                "The clips do not have identical stream settings. Switch to Compatible MP4 mode and re-encode them?",
                            ) {
                                continue;
                            }
                            self.join_mode = 1;
                        }
                        Err(error) => {
                            self.error(error);
                            continue;
                        }
                        _ => {}
                    }
                    if let Err(error) = self.submit_join(&output, files) {
                        self.error(error);
                    }
                }
                UiEvent::Preview(generation, duration, position, frame) => {
                    self.clip_preview_received(ctx, generation, duration, position, frame);
                }
            }
        }
        self.tick_clip(ctx);
        if self.space_checked.elapsed() > Duration::from_secs(5) {
            self.free_space = fs2::available_space(&self.settings.output_directory).ok();
            self.space_checked = Instant::now();
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.closing && self.engine.pending() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if confirm(
                "Close PixClip?",
                "Running jobs will be cancelled. Scheduled jobs only run while PixClip is open. Close now?",
            ) {
                self.closing = true;
                self.engine.begin_close();
            }
        }
        if self.closing {
            self.status = "Stopping process trees; partial files kept...".into();
            if !self.engine.running() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        self.main_ui(ctx);
        self.dialogs(ctx);
        ctx.request_repaint_after(Duration::from_millis(if self.engine.running() {
            100
        } else {
            250
        }));
    }
}

fn log_text(ui: &mut egui::Ui, text: &str, wrap: bool) {
    let mut layout = egui::text::LayoutJob::default();
    for line in text.split_inclusive('\n') {
        let message = line
            .split_once("] ")
            .map(|(_, message)| message)
            .unwrap_or(line);
        let color = if message.starts_with("ERROR:") || message.starts_with("FAILED") {
            RED
        } else if message.starts_with("WARNING:") {
            AMBER
        } else {
            Color32::from_rgb(193, 206, 220)
        };
        layout.append(
            line,
            0.0,
            egui::TextFormat {
                font_id: FontId::monospace(12.0),
                color,
                ..Default::default()
            },
        );
    }
    ui.add(
        egui::Label::new(layout)
            .selectable(true)
            .wrap_mode(if wrap {
                egui::TextWrapMode::Wrap
            } else {
                egui::TextWrapMode::Extend
            }),
    );
}

fn modal_parent(ui: &mut egui::Ui, cut: bool, join: bool) {
    let target = if cut {
        "cut"
    } else if join {
        "join"
    } else {
        return;
    };
    ui.disable();
    if ui.input(|i| i.pointer.any_pressed()) {
        ui.ctx().send_viewport_cmd_to(
            egui::ViewportId::from_hash_of(target),
            egui::ViewportCommand::Focus,
        );
    }
}

fn style_scrollbar(ui: &mut egui::Ui, empty: bool) {
    let visuals = ui.visuals_mut();
    visuals.extreme_bg_color = INPUT;
    visuals.widgets.inactive.bg_fill = if empty { INPUT } else { BORDER };
    visuals.widgets.hovered.bg_fill = MUTED;
    visuals.widgets.active.bg_fill = INFO;
}

fn completion_status(job: &Job) -> String {
    match job.state {
        State::Cancelled => "Task cancelled.".into(),
        State::Done => match job.mode.as_str() {
            "Joining clips" => "Clips joined successfully.",
            "Cutting clip" => "Clip created successfully.",
            "Converting" => "Done - your file is ready.",
            "Update yt-dlp" => "yt-dlp updated and verified.",
            "Install tools" => "yt-dlp, FFmpeg, and Streamlink installed and verified.",
            _ => "Queue finished. See Queue / History for files and logs.",
        }
        .into(),
        State::Failed => crate::jobs::failed_detail(job),
        _ => job.detail.clone(),
    }
}

fn select_join_row(
    selected: &mut std::collections::BTreeSet<usize>,
    anchor: &mut Option<usize>,
    index: usize,
    command: bool,
    shift: bool,
) {
    if shift {
        let start = anchor.unwrap_or(index);
        if !command {
            selected.clear();
        }
        selected.extend(start.min(index)..=start.max(index));
    } else if command {
        if !selected.remove(&index) {
            selected.insert(index);
        }
        *anchor = Some(index);
    } else {
        selected.clear();
        selected.insert(index);
        *anchor = Some(index);
    }
}

fn queue_detail(job: &Job) -> String {
    if job.state == State::Failed {
        return crate::jobs::failed_detail(job);
    }
    if job.state == State::Queued && job.start_at > 0.0 {
        if let Some(start) = chrono::DateTime::from_timestamp(job.start_at as i64, 0) {
            return format!(
                "Scheduled {}",
                start.with_timezone(&Local).format("%Y-%m-%d %H:%M")
            );
        }
    } else if job.state == State::Running && job.duration > 0.0 {
        let remaining = (job.deadline().unwrap_or(model::now()) - model::now()).max(0.0) as u64;
        return format!(
            "Live slot remaining {:02}:{:02} | {}",
            remaining / 60,
            remaining % 60,
            job.detail
        );
    }
    job.detail.clone()
}

fn spin_at(
    ui: &mut egui::Ui,
    id: &str,
    bounds: egui::Rect,
    value: &mut String,
    range: std::ops::RangeInclusive<f64>,
    step: f64,
) -> bool {
    let mut changed = false;
    in_rect(ui, id, bounds, |ui| {
        changed = ui
            .put(
                rect(
                    bounds.left(),
                    bounds.top(),
                    bounds.width() - 14.0,
                    bounds.height(),
                ),
                egui::TextEdit::singleline(value),
            )
            .changed();
        for (index, delta) in [step, -step].iter().enumerate() {
            let arrow = rect(
                bounds.right() - 14.0,
                bounds.top() + index as f32 * bounds.height() / 2.0,
                14.0,
                bounds.height() / 2.0,
            );
            let response = ui.interact(arrow, ui.make_persistent_id(index), egui::Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    if index == 0 { "Increase" } else { "Decrease" },
                )
            });
            ui.painter().rect_filled(arrow, 0.0, SURFACE_RAISED);
            ui.painter().rect_stroke(
                arrow,
                0.0,
                egui::Stroke::new(1.0_f32, BORDER),
                egui::StrokeKind::Inside,
            );
            let middle = arrow.center();
            let direction = if index == 0 { -1.0 } else { 1.0 };
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    middle + Vec2::new(-3.0, -direction * 2.0),
                    middle + Vec2::new(3.0, -direction * 2.0),
                    middle + Vec2::new(0.0, direction * 2.0),
                ],
                TEXT,
                egui::Stroke::NONE,
            ));
            if response.clicked() {
                let number = (value.parse::<f64>().unwrap_or(*range.start()) + delta)
                    .clamp(*range.start(), *range.end());
                *value = number.to_string();
                changed = true;
            }
        }
    });
    changed
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(x, y), Vec2::new(w, h))
}
fn in_rect<R>(
    ui: &mut egui::Ui,
    id: &str,
    rect: egui::Rect,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            content(ui)
        },
    )
    .inner
}
fn label_at(
    ui: &egui::Ui,
    rect: egui::Rect,
    text: &str,
    color: Color32,
    size: f32,
    bold: bool,
    center: bool,
) {
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let position = if center {
        rect.center()
    } else {
        egui::pos2(rect.left(), rect.center().y)
    };
    let anchor = if center {
        egui::Align2::CENTER_CENTER
    } else {
        egui::Align2::LEFT_CENTER
    };
    painter.text(
        position,
        anchor,
        text,
        if bold {
            FontId::new(size, FontFamily::Name("MonoBold".into()))
        } else {
            FontId::monospace(size)
        },
        color,
    );
}
fn panel_shape(
    painter: &egui::Painter,
    bounds: egui::Rect,
    tiles: &[(f32, Color32)],
) -> egui::Shape {
    let Some(texture) = painter
        .ctx()
        .data(|data| data.get_temp::<TextureHandle>(egui::Id::new(PANEL_ATLAS)))
    else {
        return egui::Shape::Noop;
    };
    // Aseprite tiles are 12 px square. Only the center strips stretch.
    let xs = [
        bounds.left(),
        bounds.left() + 4.0,
        bounds.right() - 4.0,
        bounds.right(),
    ];
    let ys = [
        bounds.top(),
        bounds.top() + 4.0,
        bounds.bottom() - 4.0,
        bounds.bottom(),
    ];
    let mut mesh = egui::Mesh::with_texture(texture.id());
    for &(offset, tint) in tiles {
        for y in 0..3 {
            for x in 0..3 {
                let destination = egui::Rect::from_min_max(
                    egui::pos2(xs[x], ys[y]),
                    egui::pos2(xs[x + 1], ys[y + 1]),
                );
                let uv = egui::Rect::from_min_size(
                    egui::pos2((offset + x as f32 * 4.0) / 52.0, y as f32 * 4.0 / 16.0),
                    Vec2::new(4.0 / 52.0, 4.0 / 16.0),
                );
                mesh.add_rect_with_uv(destination, uv, tint);
            }
        }
    }
    egui::Shape::mesh(mesh)
}

fn pixel_frame(painter: &egui::Painter, bounds: egui::Rect, fill: Color32, edge: Color32) {
    painter.add(panel_shape(painter, bounds, &[(0.0, edge), (12.0, fill)]));
}

fn pixel_heading(ui: &mut egui::Ui, bounds: egui::Rect, title: &str, icon: &str, size: f32) {
    pixel_frame(ui.painter(), bounds, HEADER, BORDER);
    paint_icon(
        ui,
        egui::pos2(bounds.left() + 10.0, bounds.center().y - 8.0),
        icon,
        true,
    );
    label_at(
        ui,
        rect(
            bounds.left() + 34.0,
            bounds.top(),
            bounds.width() - 44.0,
            bounds.height(),
        ),
        title,
        MINT,
        size,
        true,
        false,
    );
    ui.interact(bounds, ui.id().with(title), egui::Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, title));
}

fn flow_heading(ui: &mut egui::Ui, title: &str, icon: &str) {
    let (_, bounds) = ui.allocate_space(Vec2::new(ui.available_width(), 40.0));
    pixel_heading(ui, bounds, title, icon, 19.0);
}

fn flow_button(
    ui: &mut egui::Ui,
    text: &str,
    icon: &str,
    width: f32,
    color: Color32,
    enabled: bool,
) -> egui::Response {
    let (_, bounds) = ui.allocate_space(Vec2::new(width, 34.0));
    pixel_button(ui, bounds, text, icon, color, enabled, 13.3)
}

fn list_panel(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    let painter = ui.painter().clone();
    let background = painter.add(egui::Shape::Noop);
    let frame = egui::Frame::new().inner_margin(10).show(ui, contents);
    painter.set(
        background,
        panel_shape(
            &painter,
            frame.response.rect,
            &[(0.0, BORDER), (12.0, INPUT)],
        ),
    );
}

fn group_at(ui: &egui::Ui, rect: egui::Rect, title: &str) {
    let (fill, accent, icon) = match title {
        "DOWNLOAD / LIVE" => (SURFACE, MINT, "download"),
        "CONVERT" => (SURFACE, INFO, "convert"),
        "EDIT / TOOLS" => (SURFACE, AMBER, "tools"),
        _ => (INPUT, MUTED, "log"),
    };
    pixel_frame(ui.painter(), rect, fill, BORDER);
    paint_icon(ui, rect.min + Vec2::new(9.0, 2.0), icon, true);
    label_at(
        ui,
        self::rect(
            rect.left() + 31.0,
            rect.top() + 1.0,
            rect.width() - 42.0,
            18.0,
        ),
        title,
        accent,
        12.0,
        true,
        false,
    );
}
fn combo_at(
    ui: &mut egui::Ui,
    id: &str,
    rect: egui::Rect,
    index: &mut usize,
    choices: &[&str],
    enabled: bool,
) {
    in_rect(ui, id, rect, |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            ui.visuals_mut().widgets.inactive.bg_fill = INPUT;
            ui.visuals_mut().widgets.inactive.weak_bg_fill = INPUT;
            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
            ui.style_mut().text_styles.insert(
                egui::TextStyle::Button,
                if id == "quality" {
                    FontId::proportional(12.0)
                } else {
                    FontId::new(13.3, FontFamily::Name("MonoBold".into()))
                },
            );
            let inner_height = ui
                .text_style_height(&egui::TextStyle::Button)
                .max(ui.spacing().icon_width);
            ui.spacing_mut().button_padding =
                Vec2::new(10.0, ((rect.height() - inner_height) / 2.0).max(0.0));
            combo(ui, id, index, choices, rect.width());
        });
    });
}

fn dropdown(id: impl std::hash::Hash) -> egui::ComboBox {
    egui::ComboBox::from_id_salt(id)
        .truncate()
        .icon(paint_dropdown_arrow)
}

fn paint_dropdown_arrow(
    ui: &egui::Ui,
    bounds: egui::Rect,
    _: &egui::style::WidgetVisuals,
    open: bool,
) {
    let Some(texture) = ui
        .ctx()
        .data(|data| data.get_temp::<TextureHandle>(egui::Id::new(DROPDOWN_ATLAS)))
    else {
        return;
    };
    let size = Vec2::new(14.0, 12.0);
    let rect = egui::Rect::from_center_size(bounds.center().round(), size);
    let x = if open { 0.5 } else { 0.0 };
    let uv = egui::Rect::from_min_size(egui::pos2(x, 0.0), Vec2::new(0.5, 1.0));
    let tint = if ui.is_enabled() {
        Color32::WHITE
    } else {
        Color32::WHITE.gamma_multiply(0.44)
    };
    ui.painter().image(texture.id(), rect, uv, tint);
}

fn pixel_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    text: &str,
    icon: &str,
    color: Color32,
    enabled: bool,
    size: f32,
) -> egui::Response {
    let response = ui.interact(
        rect,
        ui.make_persistent_id((text, rect.min.x.to_bits(), rect.min.y.to_bits())),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, text));
    let primary = color == GREEN || color == TEAL;
    let base = if color == GREEN {
        MINT
    } else if color == TEAL {
        Color32::from_rgb(104, 193, 201)
    } else {
        color.lerp_to_gamma(INFO, 0.12)
    };
    let fill = if !enabled {
        SURFACE
    } else if response.is_pointer_button_down_on() {
        base.lerp_to_gamma(INPUT, 0.28)
    } else if response.hovered() {
        base.lerp_to_gamma(MINT, 0.14)
    } else {
        base
    };
    let stroke = if response.has_focus() && primary {
        INPUT
    } else if response.has_focus() || (enabled && response.hovered()) {
        INFO
    } else if primary && enabled {
        base.lerp_to_gamma(TEXT, 0.30)
    } else if enabled {
        Color32::from_rgb(98, 135, 145)
    } else {
        SURFACE_RAISED
    };
    pixel_frame(ui.painter(), rect, fill, stroke);
    let text_color = if !enabled {
        MUTED.gamma_multiply(0.65)
    } else if primary {
        INPUT
    } else {
        TEXT
    };
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::monospace(size), text_color);
    let icon_width = if icon.is_empty() { 0.0 } else { 29.0 };
    let x = rect.center().x - (galley.size().x + icon_width) / 2.0;
    if !icon.is_empty() {
        let badge = self::rect(x, rect.center().y - 11.0, 22.0, 22.0);
        ui.painter()
            .add(panel_shape(ui.painter(), badge, &[(24.0, INPUT)]));
        paint_icon(ui, badge.min + Vec2::splat(3.0), icon, enabled);
    }
    ui.painter().galley(
        egui::pos2(x + icon_width, rect.center().y - galley.size().y / 2.0),
        galley,
        text_color,
    );
    response
}
fn paint_icon(ui: &egui::Ui, position: egui::Pos2, name: &str, enabled: bool) {
    let tint = if enabled {
        Color32::WHITE
    } else {
        Color32::WHITE.gamma_multiply(0.44)
    };
    let bounds = egui::Rect::from_min_size(position, Vec2::splat(16.0));
    // 16 px cells, in the same order as the Aseprite sheet.
    const ICONS: [&str; 24] = [
        "download",
        "paste",
        "folder",
        "open-folder",
        "history",
        "convert",
        "tools",
        "join",
        "cut",
        "update",
        "terminal",
        "stop",
        "delete",
        "retry",
        "file",
        "log",
        "clear",
        "schedule",
        "play",
        "pause",
        "save",
        "help",
        "left",
        "right",
    ];
    let Some(index) = ICONS.iter().position(|icon| *icon == name) else {
        return;
    };
    let texture = ui
        .ctx()
        .data(|data| data.get_temp::<TextureHandle>(egui::Id::new(BUTTON_ICON_ATLAS)));
    if let Some(texture) = texture {
        let cell = Vec2::new(1.0 / 6.0, 1.0 / 4.0);
        let uv = egui::Rect::from_min_size(
            egui::pos2((index % 6) as f32 * cell.x, (index / 6) as f32 * cell.y),
            cell,
        );
        ui.painter().image(texture.id(), bounds, uv, tint);
    }
}

#[cfg(windows)]
fn system_mono_font(bold: bool) -> Option<Vec<u8>> {
    use windows_sys::Win32::Graphics::Gdi::*;
    // GDI can resolve packaged system fonts that have no file in Windows/Fonts.
    unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return None;
        }
        let mut result = None;
        for name in ["Cascadia Mono", "Consolas", "Courier New"] {
            let face: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let font = CreateFontW(
                -13,
                0,
                0,
                0,
                if bold { 700 } else { 400 },
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                0,
                0,
                face.as_ptr(),
            );
            if font.is_null() {
                continue;
            }
            let old = SelectObject(dc, font);
            let mut actual = [0_u16; 128];
            let count = GetTextFaceW(dc, actual.len() as i32, actual.as_mut_ptr());
            let actual = String::from_utf16_lossy(&actual[..count.saturating_sub(1) as usize]);
            if actual.eq_ignore_ascii_case(name) {
                let size = GetFontData(dc, 0, 0, std::ptr::null_mut(), 0);
                if size != u32::MAX && size > 0 {
                    let mut bytes = vec![0_u8; size as usize];
                    if GetFontData(dc, 0, 0, bytes.as_mut_ptr().cast(), size) == size {
                        result = Some(bytes);
                    }
                }
            }
            SelectObject(dc, old);
            DeleteObject(font);
            if result.is_some() {
                break;
            }
        }
        DeleteDC(dc);
        result
    }
}

fn theme(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut fonts = egui::FontDefinitions::default();
    let system_fonts = if cfg!(windows) {
        vec![
            PathBuf::from("C:/Windows/Fonts/CascadiaMono.ttf"),
            PathBuf::from("C:/Windows/Fonts/cour.ttf"),
        ]
    } else {
        vec![
            PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"),
            PathBuf::from("/usr/share/fonts/truetype/liberation2/LiberationMono-Regular.ttf"),
        ]
    };
    let normal = system_fonts
        .iter()
        .find_map(|path| std::fs::read(path).ok());
    #[cfg(windows)]
    let normal = system_mono_font(false).or(normal);
    if let Some(bytes) = normal {
        fonts.font_data.insert(
            "SystemMono".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, "SystemMono".into());
        }
    }
    if cfg!(windows)
        && let Ok(bytes) = std::fs::read("C:/Windows/Fonts/tahoma.ttf")
    {
        fonts.font_data.insert(
            "SystemThai".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("SystemThai".into());
        }
    }
    fonts.font_data.insert(
        "Thai".into(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/NotoSansThai.ttf")).into(),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("Thai".into());
    }
    let mut bold = fonts.families[&FontFamily::Monospace].clone();
    let bold_paths = if cfg!(windows) {
        vec![
            "C:/Windows/Fonts/CascadiaMono-Bold.ttf",
            "C:/Windows/Fonts/courbd.ttf",
        ]
    } else {
        vec![
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
            "/usr/share/fonts/truetype/liberation2/LiberationMono-Bold.ttf",
        ]
    };
    let bold_data = bold_paths.iter().find_map(|path| std::fs::read(path).ok());
    #[cfg(windows)]
    let bold_data = system_mono_font(true).or(bold_data);
    if let Some(bytes) = bold_data {
        fonts.font_data.insert(
            "SystemMonoBold".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        bold.insert(0, "SystemMonoBold".into());
    }
    fonts
        .families
        .insert(FontFamily::Name("MonoBold".into()), bold);
    let ui_font = if cfg!(windows) {
        "C:/Windows/Fonts/segoeui.ttf"
    } else {
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
    };
    if let Ok(bytes) = std::fs::read(ui_font) {
        fonts
            .font_data
            .insert("SystemUI".into(), egui::FontData::from_owned(bytes).into());
        fonts
            .families
            .get_mut(&FontFamily::Proportional)
            .unwrap()
            .insert(0, "SystemUI".into());
    }
    ctx.set_fonts(fonts);
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = BG;
    style.visuals.window_stroke = egui::Stroke::new(2.0_f32, BORDER);
    style.visuals.extreme_bg_color = INPUT;
    style.visuals.widgets.inactive.bg_fill = SURFACE;
    style.visuals.widgets.inactive.weak_bg_fill = SURFACE;
    style.visuals.widgets.hovered.bg_fill = SURFACE_RAISED;
    style.visuals.widgets.active.bg_fill = GREEN;
    style.visuals.widgets.open.bg_fill = HEADER;
    style.visuals.widgets.open.weak_bg_fill = HEADER;
    style.visuals.selection.bg_fill = GREEN;
    style.visuals.selection.stroke.color = MINT;
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::ZERO;
        widget.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
    }
    style.visuals.window_corner_radius = egui::CornerRadius::ZERO;
    style.spacing.item_spacing = Vec2::new(8.0, 7.0);
    style.spacing.button_padding = Vec2::new(9.0, 7.0);
    style.spacing.scroll = egui::style::ScrollStyle::solid();
    style.spacing.scroll.bar_width = 17.0;
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::monospace(13.3));
    style
        .text_styles
        .insert(egui::TextStyle::Small, FontId::monospace(12.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, FontId::monospace(24.0));
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::monospace(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, FontId::monospace(13.0));
    ctx.set_style(style);
}

fn combo(ui: &mut egui::Ui, id: &str, index: &mut usize, choices: &[&str], width: f32) {
    let mut combo = dropdown(id).selected_text(choices[*index]).width(width);
    if matches!(id, "download-mode" | "convert-mode") {
        combo = combo
            .height(360.0)
            .popup_style(egui::style::StyleModifier::new(|style| {
                style.spacing.button_padding.y = 4.0;
            }));
    }
    combo.show_ui(ui, |ui| {
        let (first, secondary) = if id == "download-mode" {
            (media::SPOTIFY_TRACK_MODE, media::TIME_RANGE_MODE)
        } else {
            (0, 0)
        };
        for value in (first..choices.len())
            .chain(secondary..first)
            .chain(0..secondary)
        {
            ui.selectable_value(index, value, choices[value]);
        }
    });
}
fn confirm(title: &str, description: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(description)
        .set_level(rfd::MessageLevel::Warning)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
}
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mkv", "mov", "avi", "ts", "m2ts", "webm"];

fn video_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new()
        .set_title("Choose a video file")
        .add_filter("Video files", VIDEO_EXTENSIONS)
        .add_filter("All files", &["*"])
}
fn texture(
    ctx: &egui::Context,
    name: &str,
    bytes: &[u8],
    options: egui::TextureOptions,
) -> Result<TextureHandle> {
    let image = image::load_from_memory(bytes)?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Ok(ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
        options,
    ))
}
fn pixel_progress(ui: &mut egui::Ui, progress: Option<f32>) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, INPUT);
    let count = (rect.width() / 9.0) as usize;
    for index in 0..count {
        let cell = egui::Rect::from_min_size(
            rect.min + Vec2::new(index as f32 * 9.0 + 2.0, 2.0),
            Vec2::new(6.0, 12.0),
        );
        ui.painter().rect_filled(
            cell,
            0.0,
            if progress
                .map(|p| index as f32 / (count as f32) < p)
                .unwrap_or_else(|| {
                    (index + (ui.input(|i| i.time) * 10.0) as usize) % count.max(1) < 10
                })
            {
                MINT
            } else {
                SURFACE
            },
        );
    }
}
fn pixel_title(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(180.0, 34.0), egui::Sense::hover());
    for (index, glyph) in [
        "11110/10001/10001/11110/10000/10000/10000",
        "11111/00100/00100/00100/00100/00100/11111",
        "10001/10001/01010/00100/01010/10001/10001",
        "01111/10000/10000/10000/10000/10000/01111",
        "10000/10000/10000/10000/10000/10000/11111",
        "11111/00100/00100/00100/00100/00100/11111",
        "11110/10001/10001/11110/10000/10000/10000",
    ]
    .iter()
    .enumerate()
    {
        for (y, row) in glyph.split('/').enumerate() {
            for (x, cell) in row.chars().enumerate() {
                if cell == '1' {
                    let min = rect.min + Vec2::new((index * 6 + x) as f32 * 4.0, y as f32 * 4.0);
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(min + Vec2::splat(2.0), Vec2::splat(4.0)),
                        0.0,
                        GREEN,
                    );
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(min, Vec2::splat(4.0)),
                        0.0,
                        MINT,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(app: &mut App, ctx: &egui::Context, view: &str, events: Vec<egui::Event>) {
        let size = if view == "queue" {
            [1040.0, 720.0]
        } else {
            [1000.0, 860.0]
        };
        let time = ctx.input(|i| i.time) + 0.05;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(rect(0.0, 0.0, size[0], size[1])),
                time: Some(time),
                events,
                focused: true,
                ..Default::default()
            },
            |ctx| {
                if view == "queue" {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::new().fill(BG))
                        .show(ctx, |ui| app.queue_ui(ui));
                } else {
                    app.main_ui(ctx);
                }
            },
        );
    }
    fn click(app: &mut App, ctx: &egui::Context, view: &str, x: f32, y: f32) {
        for pressed in [true, false] {
            draw(
                app,
                ctx,
                view,
                vec![
                    egui::Event::PointerMoved(egui::pos2(x, y)),
                    egui::Event::PointerButton {
                        pos: egui::pos2(x, y),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
    }
    #[test]
    fn original_main_button_positions_and_modal_cut_join_work() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            Engine::new(dir.path()).unwrap(),
            Settings {
                output_directory: dir.path().into(),
                ..Settings::default()
            },
        );
        for _ in 0..3 {
            draw(&mut app, &ctx, "main", vec![]);
        }
        click(&mut app, &ctx, "main", 870.0, 330.0);
        assert!(app.queue_open);
        app.queue_open = false;
        click(&mut app, &ctx, "main", 810.0, 490.0);
        assert!(app.cut_open);
        click(&mut app, &ctx, "main", 490.0, 490.0);
        assert!(!app.join_open, "Cut must capture parent input");
        app.cut_open = false;
        draw(&mut app, &ctx, "main", vec![]);
        click(&mut app, &ctx, "main", 490.0, 490.0);
        assert!(app.join_open);
        app.join_open = false;
        draw(&mut app, &ctx, "main", vec![]);
        click(&mut app, &ctx, "main", 920.0, 28.0);
        assert!(app.help_open);
        let download = Job {
            state: State::Running,
            url: "https://example.com/video".into(),
            folder: dir.path().into(),
            ..Job::default()
        };
        let local = Job {
            kind: "local".into(),
            state: State::Running,
            folder: dir.path().into(),
            ..Job::default()
        };
        let download_id = download.id.clone();
        let local_id = local.id.clone();
        app.engine.history.jobs.extend([download, local]);
        draw(&mut app, &ctx, "main", vec![]);
        click(&mut app, &ctx, "main", 810.0, 543.0);
        assert_eq!(
            app.engine.job(&download_id).unwrap().state,
            State::Running,
            "Main Cancel task must leave downloads alone"
        );
        assert_eq!(app.engine.job(&local_id).unwrap().state, State::Cancelling);
    }
    #[test]
    fn queue_rows_select_from_url_and_detail_and_double_click_opens_log() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            Engine::new(dir.path()).unwrap(),
            Settings {
                output_directory: dir.path().into(),
                ..Settings::default()
            },
        );
        for _ in 0..2 {
            app.engine.history.jobs.push(Job {
                state: State::Done,
                url: "https://example.com/video".into(),
                detail: "Complete".into(),
                folder: dir.path().into(),
                ..Job::default()
            });
        }
        let first = app.engine.history.jobs[0].id.clone();
        let second = app.engine.history.jobs[1].id.clone();
        for _ in 0..3 {
            draw(&mut app, &ctx, "queue", vec![]);
        }
        click(&mut app, &ctx, "queue", 420.0, 106.0);
        assert_eq!(app.selected, Some(first));
        click(&mut app, &ctx, "queue", 800.0, 134.0);
        assert_eq!(app.selected, Some(second));
        click(&mut app, &ctx, "queue", 800.0, 134.0);
        assert!(app.detail_open);
    }
    #[test]
    fn join_keeps_ctrl_and_shift_selection() {
        let mut selected = Default::default();
        let mut anchor = None;
        select_join_row(&mut selected, &mut anchor, 1, false, false);
        select_join_row(&mut selected, &mut anchor, 3, true, false);
        assert_eq!(selected, [1, 3].into());
        select_join_row(&mut selected, &mut anchor, 5, false, true);
        assert_eq!(selected, [3, 4, 5].into());
        select_join_row(&mut selected, &mut anchor, 4, true, false);
        assert_eq!(selected, [3, 5].into());
    }

    #[test]
    fn audio_settings_are_recorded_for_batch_playlist_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            Engine::new(dir.path()).unwrap(),
            Settings {
                output_directory: dir.path().into(),
                ..Settings::default()
            },
        );
        app.mode = 6;
        app.audio_format = 1;
        app.audio_quality = 2;
        app.enqueue("https://example.com/one\nhttps://example.com/two", false)
            .unwrap();
        assert_eq!(app.engine.history.jobs.len(), 2);
        for job in &app.engine.history.jobs {
            assert_eq!(job.audio_format, "M4A");
            assert_eq!(job.audio_bitrate, Some(192));
        }
    }

    #[test]
    fn batch_conversion_reserves_outputs_and_keeps_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(
            &egui::Context::default(),
            Engine::new(dir.path()).unwrap(),
            Settings {
                output_directory: dir.path().into(),
                ..Settings::default()
            },
        );
        let first = dir.path().join("clip.mov");
        let second = dir.path().join("clip.mkv");
        let existing = dir.path().join("clip_compressed_720p.mp4");
        for path in [&first, &second, &existing] {
            std::fs::write(path, b"original").unwrap();
        }
        app.convert_mode = media::COMPRESS_MODE;
        app.compress_limit = 2;
        app.queue_conversions(vec![first, second]).unwrap();
        let names: Vec<_> = app
            .engine
            .history
            .jobs
            .iter()
            .map(|job| {
                Path::new(job.arguments.last().unwrap())
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
            })
            .collect();
        assert_eq!(
            names,
            [
                "clip_compressed_720p (2).mp4",
                "clip_compressed_720p (3).mp4"
            ]
        );
        assert_eq!(std::fs::read(existing).unwrap(), b"original");
        assert!(
            app.engine
                .history
                .jobs
                .iter()
                .all(|job| job.arguments.iter().any(|arg| arg == "-n"))
        );
    }

    #[test]
    fn spotify_menu_entries_keep_audio_options_and_require_the_matching_link() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(
            &egui::Context::default(),
            Engine::new(dir.path()).unwrap(),
            Settings {
                output_directory: dir.path().into(),
                ..Settings::default()
            },
        );
        let track = "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC";
        let playlist = "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M";
        app.audio_format = 1;
        app.audio_quality = 2;
        for (mode, expected, wrong_link, hint) in [
            (
                media::SPOTIFY_TRACK_MODE,
                media::MODES[3],
                playlist,
                "Spotify - Playlist / Album",
            ),
            (
                media::SPOTIFY_PLAYLIST_MODE,
                media::MODES[6],
                track,
                "Spotify - Track",
            ),
        ] {
            app.mode = mode;
            let job = app.job_template(String::new());
            assert_eq!(job.mode, expected);
            assert_eq!(job.audio_format, "M4A");
            assert_eq!(job.audio_bitrate, Some(192));
            assert!(
                app.enqueue(wrong_link, false)
                    .unwrap_err()
                    .to_string()
                    .contains(hint)
            );
            assert!(app.enqueue("https://example.com/video", false).is_err());
        }
        assert!(app.engine.history.jobs.is_empty());
    }

    #[test]
    fn scheduling_requires_live_mode_and_preserves_the_entered_slot() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            Engine::new(dir.path()).unwrap(),
            Settings {
                output_directory: dir.path().into(),
                ..Settings::default()
            },
        );
        app.schedule = (Local::now() + chrono::Duration::hours(1))
            .format("%Y-%m-%d %H:%M")
            .to_string();
        app.minutes = "12.5".into();
        app.reserve = 0.25;
        assert!(app.enqueue("https://example.com/live", true).is_err());
        app.mode = 8;
        app.enqueue("https://example.com/live", true).unwrap();
        let job = &app.engine.history.jobs[0];
        assert_eq!(job.mode, media::AUTO_LIVE);
        assert_eq!(job.duration, 750.0);
        assert!(job.start_at > model::now());
        assert!(job.directory().starts_with(dir.path().join("Live")));
        app.schedule = "2000-01-01 00:00".into();
        assert!(app.enqueue("https://example.com/live", true).is_err());
        assert_eq!(app.engine.history.jobs.len(), 1);
    }
}

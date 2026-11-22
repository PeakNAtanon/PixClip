use anyhow::{Context, Result, bail, ensure};
use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub const QUALITIES: [&str; 6] = [
    "Best available",
    "Up to 2160p",
    "Up to 1080p",
    "Up to 720p",
    "Up to 480p",
    "Audio only (MP3)",
];
pub const AUDIO_FORMATS: [&str; 4] = ["MP3", "M4A", "FLAC", "WAV"];
pub const AUDIO_BITRATES: [Option<u16>; 4] = [None, Some(320), Some(192), Some(128)];
pub const GIB: u64 = 1024 * 1024 * 1024;

pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub fn home() -> PathBuf {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn root() -> PathBuf {
    let executable = env::current_exe().unwrap_or_default();
    let parent = executable.parent().unwrap_or(Path::new("."));
    for directory in parent.ancestors() {
        if directory.join("Cargo.toml").is_file() || directory.join("PixClip.bat").is_file() {
            return directory.to_path_buf();
        }
    }
    parent.to_path_buf()
}

pub fn state_dir() -> PathBuf {
    if let Some(directory) = env::var_os("PIXCLIP_STATE_DIR") {
        return directory.into();
    }
    if cfg!(windows) {
        env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home)
            .join("PixClip")
    } else {
        env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".local/state"))
            .join("PixClip")
    }
}

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("Missing settings directory")?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub output_directory: PathBuf,
    pub cookies_file: Option<PathBuf>,
    #[serde(skip)]
    pub load_warning: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            output_directory: home().join("Downloads/YT-DLP"),
            cookies_file: None,
            load_warning: String::new(),
            extra: BTreeMap::new(),
        }
    }
}

fn expand_home(path: PathBuf) -> PathBuf {
    path.strip_prefix("~")
        .map(|relative| home().join(relative))
        .unwrap_or(path)
}

impl Settings {
    pub fn load(directory: &Path) -> Result<Self> {
        let path = directory.join("settings.json");
        match fs::read(&path) {
            Ok(bytes) => {
                let mut settings: Self = match serde_json::from_slice(&bytes) {
                    Ok(settings) => settings,
                    Err(_) => {
                        let backup = directory
                            .join(format!("settings-invalid-{}.json", Uuid::new_v4().simple()));
                        let mut file = fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&backup)?;
                        file.write_all(&bytes)?;
                        file.sync_all()?;
                        return Ok(Self {
                            load_warning: format!(
                                "Settings were invalid; defaults loaded. Original backed up at {}",
                                backup.display()
                            ),
                            ..Self::default()
                        });
                    }
                };
                settings.output_directory = expand_home(settings.output_directory);
                if !settings.output_directory.is_dir() {
                    settings.output_directory = Self::default().output_directory;
                }
                settings.cookies_file = settings
                    .cookies_file
                    .map(expand_home)
                    .filter(|path| path.is_file());
                Ok(settings)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }
    pub fn save(&self, directory: &Path) -> Result<()> {
        atomic_json(&directory.join("settings.json"), self)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum State {
    #[default]
    Queued,
    Running,
    Cancelling,
    Done,
    Failed,
    Cancelled,
    Interrupted,
}

impl State {
    pub fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Done | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Job {
    pub id: String,
    pub url: String,
    pub mode: String,
    pub folder: PathBuf,
    pub output_folder: Option<PathBuf>,
    pub quality: String,
    pub audio_format: String,
    pub audio_bitrate: Option<u16>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub playlist_items: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_range: Option<[f64; 2]>,
    pub subtitle_language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spotify_url: Option<String>,
    pub auto_mp4: bool,
    pub start_at: f64,
    pub duration: f64,
    pub reserve_gb: f64,
    pub state: State,
    pub detail: String,
    pub progress: Option<f64>,
    pub cookies_file: Option<PathBuf>,
    pub created: f64,
    pub started: Option<f64>,
    pub finished: Option<f64>,
    pub outputs: Vec<PathBuf>,
    pub logs: Vec<String>,
    pub kind: String,
    pub executable: Option<PathBuf>,
    pub arguments: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<PathBuf>,
    pub temporary: BTreeMap<String, String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for Job {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4().simple().to_string(),
            url: String::new(),
            mode: String::new(),
            folder: PathBuf::new(),
            output_folder: None,
            quality: QUALITIES[0].into(),
            audio_format: AUDIO_FORMATS[0].into(),
            audio_bitrate: None,
            playlist_items: vec![],
            download_range: None,
            subtitle_language: crate::media::SUBTITLE_LANGUAGES[0].into(),
            spotify_url: None,
            auto_mp4: false,
            start_at: 0.0,
            duration: 0.0,
            reserve_gb: 1.0,
            state: State::Queued,
            detail: "Waiting".into(),
            progress: None,
            cookies_file: None,
            created: now(),
            started: None,
            finished: None,
            outputs: vec![],
            logs: vec![],
            kind: String::new(),
            executable: None,
            arguments: vec![],
            inputs: vec![],
            temporary: BTreeMap::new(),
            extra: BTreeMap::new(),
        }
    }
}

impl Job {
    pub fn validate(&self) -> Result<()> {
        if self.kind != "local" && self.kind != "tools" {
            let url = url::Url::parse(&self.url).context("Use a valid http(s) URL")?;
            ensure!(
                matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
                "Use a valid http(s) URL"
            );
            ensure!(
                QUALITIES.contains(&self.quality.as_str()),
                "Unknown quality"
            );
            if self.mode == crate::media::MODES[crate::media::TIME_RANGE_MODE]
                || self.download_range.is_some()
            {
                ensure!(
                    self.download_range.is_some_and(|[start, end]| {
                        start.is_finite() && end.is_finite() && start >= 0.0 && end > start
                    }),
                    "Enter a start time and a later end time"
                );
            }
            ensure!(
                self.playlist_items.is_empty()
                    || (self.mode.starts_with("Playlist") && !self.playlist_items.contains(&0)),
                "Playlist selections use positive item numbers"
            );
            if self.mode == crate::media::MODES[crate::media::SUBTITLES_MODE] {
                ensure!(
                    crate::media::SUBTITLE_LANGUAGES.contains(&self.subtitle_language.as_str()),
                    "Choose a subtitle language"
                );
            }
            if crate::media::audio_only(self) {
                ensure!(
                    AUDIO_FORMATS.contains(&self.audio_format.as_str()),
                    "Choose MP3, M4A, FLAC or WAV"
                );
                ensure!(
                    AUDIO_BITRATES.contains(&self.audio_bitrate),
                    "Choose Best quality, 320, 192 or 128 kbps"
                );
                ensure!(
                    self.audio_bitrate.is_none()
                        || crate::media::audio_encoder(&self.audio_format).is_some(),
                    "Bitrate is selectable for MP3 and M4A"
                );
            }
        }
        ensure!(
            [self.start_at, self.duration, self.reserve_gb]
                .iter()
                .all(|v| v.is_finite() && *v >= 0.0),
            "Invalid schedule or disk reserve"
        );
        ensure!(self.reserve_gb >= 0.25, "Keep at least 0.25 GiB free");
        ensure!(
            !self.folder.as_os_str().is_empty(),
            "Choose an output folder"
        );
        if let Some(path) = &self.cookies_file {
            ensure!(
                path.is_file(),
                "Cookies file was not found: {}",
                path.display()
            );
        }
        Ok(())
    }
    pub fn directory(&self) -> PathBuf {
        self.output_folder.clone().unwrap_or_else(|| {
            self.folder.join(format!(
                "PixClip-{}",
                self.id.chars().take(12).collect::<String>()
            ))
        })
    }
    pub fn deadline(&self) -> Option<f64> {
        (self.duration > 0.0).then(|| {
            (if self.start_at > 0.0 {
                self.start_at
            } else {
                self.started.unwrap_or_else(now)
            }) + self.duration
        })
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    pub jobs: Vec<Job>,
    pub limit: usize,
    pub paused: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl History {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self {
                limit: 1,
                ..Self::default()
            });
        }
        let bytes = fs::read(path)?;
        let value: Value = serde_json::from_slice(&bytes)
            .context("History is invalid; the original file was kept")?;
        ensure!(
            value.get("jobs").is_some_and(Value::is_array),
            "History is invalid; the original file was kept"
        );
        let mut history: Self = serde_json::from_value(value)?;
        history.limit = history.limit.clamp(1, 5);
        for job in &mut history.jobs {
            if matches!(job.state, State::Running | State::Cancelling) {
                job.state = State::Interrupted;
                job.detail =
                    "App closed during this job. Retry manually; partial files were kept.".into();
            }
        }
        // Keep the exact pre-migration file before the first Rust save.
        let backup = path.with_extension("pre-rust.json");
        if !backup.exists() {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(backup)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        Ok(history)
    }
}

pub fn require_space(folder: &Path, reserve: f64, extra: u64) -> Result<u64> {
    ensure!(
        reserve.is_finite() && reserve >= 0.25,
        "Keep at least 0.25 GiB free"
    );
    fs::create_dir_all(folder)?;
    let free = fs2::available_space(folder)?;
    let required = ((reserve * GIB as f64) as u64).saturating_add(extra);
    ensure!(
        free >= required,
        "Low disk space: {:.2} GiB free; need {:.2} GiB. Partial files were kept.",
        free as f64 / GIB as f64,
        required as f64 / GIB as f64
    );
    Ok(free)
}

pub fn safe_component(text: &str) -> String {
    let mut text: String = text
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(100)
        .collect();
    text = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches([' ', '.'])
        .to_string();
    let stem_end = text.find('.').unwrap_or(text.len());
    let stem = text[..stem_end].to_uppercase();
    if ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (1..=9).any(|n| stem == format!("COM{n}") || stem == format!("LPT{n}"))
    {
        text.insert(stem_end, '_');
    }
    if text.is_empty() {
        "source".into()
    } else {
        text
    }
}

pub fn organized_folder(job: &Job) -> Result<PathBuf> {
    let url = url::Url::parse(job.spotify_url.as_deref().unwrap_or(&job.url))?;
    let host = url
        .host_str()
        .unwrap_or("source")
        .trim_start_matches("www.");
    let platform = [
        ("youtube.com", "YouTube"),
        ("youtu.be", "YouTube"),
        ("tiktok.com", "TikTok"),
        ("twitch.tv", "Twitch"),
        ("kick.com", "Kick"),
        ("facebook.com", "Facebook"),
        ("instagram.com", "Instagram"),
        ("spotify.com", "Spotify"),
    ]
    .into_iter()
    .find_map(|(domain, name)| {
        (host == domain
            || host
                .strip_suffix(domain)
                .is_some_and(|prefix| prefix.ends_with('.')))
        .then_some(name)
    })
    .unwrap_or(host);
    let category = if crate::media::live(&job.mode) {
        "Live"
    } else if job.mode.starts_with("Playlist") {
        "Playlists"
    } else if job.mode == crate::media::MODES[crate::media::SUBTITLES_MODE] {
        "Subtitles"
    } else if crate::media::audio_only(job) {
        "Audio"
    } else {
        "Videos"
    };
    let time = Local
        .timestamp_opt(job.created as i64, 0)
        .single()
        .context("Invalid creation date")?;
    Ok(job
        .folder
        .join(category)
        .join(safe_component(platform))
        .join(time.format("%Y-%m-%d").to_string()))
}

pub fn parse_time(text: &str) -> Result<f64> {
    let parts = text.trim().split(':').collect::<Vec<_>>();
    ensure!(
        !parts.is_empty() && parts.len() <= 3,
        "Use seconds, MM:SS or HH:MM:SS"
    );
    let mut total = 0.0;
    for (index, part) in parts.iter().enumerate() {
        let value: f64 = part.parse().context("Use seconds, MM:SS or HH:MM:SS")?;
        ensure!(
            value.is_finite() && value >= 0.0 && (index == 0 || value < 60.0),
            "Invalid time"
        );
        total = total * 60.0 + value;
    }
    if !total.is_finite() {
        bail!("Invalid time");
    }
    Ok(total)
}

pub fn timecode(seconds: f64) -> String {
    let ms = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_settings_open_with_defaults_and_keep_the_original_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let bytes = b"{invalid settings";
        fs::write(&path, bytes).unwrap();
        let settings = Settings::load(dir.path()).unwrap();
        assert!(!settings.load_warning.is_empty());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let backups: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("settings-invalid-")
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(&backups[0]).unwrap(), bytes);
        settings.save(dir.path()).unwrap();
        assert!(Settings::load(dir.path()).unwrap().load_warning.is_empty());
        assert_eq!(fs::read(&backups[0]).unwrap(), bytes);
    }
    #[test]
    fn old_history_and_unknown_fields_survive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let text = r#"{"jobs":[{"id":"abcdef1234567890","url":"https://example.com/v","mode":"MP4 - Best quality","folder":"output","state":"Running","custom":42}],"limit":2,"paused":true}"#;
        fs::write(&path, text).unwrap();
        let history = History::load(&path).unwrap();
        assert_eq!(history.jobs[0].state, State::Interrupted);
        assert!(
            history.jobs[0]
                .directory()
                .ends_with("PixClip-abcdef123456")
        );
        atomic_json(&path, &history).unwrap();
        assert_eq!(
            fs::read_to_string(path.with_extension("pre-rust.json")).unwrap(),
            text
        );
        let value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(value["jobs"][0]["custom"], 42);
    }
    #[test]
    fn corrupt_history_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        fs::write(&path, "broken").unwrap();
        assert!(History::load(&path).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "broken");
    }
    #[test]
    fn validates_times_urls_and_reserved_names() {
        assert_eq!(parse_time("1:02:03.5").unwrap(), 3723.5);
        for bad in ["NaN", "inf", "-1", "1:70", "1:2:3:4"] {
            assert!(parse_time(bad).is_err());
        }
        assert_eq!(timecode(59.9999), "00:01:00.000");
        assert_eq!(safe_component("CON.txt"), "CON_.txt");
        let job = Job {
            url: "file:///secret".into(),
            folder: "output".into(),
            ..Job::default()
        };
        assert!(job.validate().is_err());
    }

    #[test]
    fn downloads_group_by_media_type_and_website() {
        use crate::media::{MODES, STREAMLINK, TS};

        for (mode, quality, url, category, platform) in [
            (
                TS,
                QUALITIES[0],
                "https://youtu.be/clip",
                "Videos",
                "YouTube",
            ),
            (
                MODES[0],
                QUALITIES[5],
                "https://m.youtube.com/watch?v=clip",
                "Audio",
                "YouTube",
            ),
            (
                MODES[6],
                QUALITIES[5],
                "https://www.youtube.com/playlist?list=music",
                "Playlists",
                "YouTube",
            ),
            (
                STREAMLINK,
                QUALITIES[0],
                "https://kick.com/channel/live",
                "Live",
                "Kick",
            ),
            (
                MODES[0],
                QUALITIES[0],
                "https://con.example/file",
                "Videos",
                "con_.example",
            ),
        ] {
            let job = Job {
                url: url.into(),
                mode: mode.into(),
                quality: quality.into(),
                folder: "output".into(),
                created: 0.0,
                ..Job::default()
            };
            let local_time = Local
                .timestamp_opt(0, 0)
                .single()
                .unwrap()
                .format("%Y-%m-%d")
                .to_string();
            assert_eq!(
                organized_folder(&job).unwrap(),
                Path::new("output")
                    .join(category)
                    .join(platform)
                    .join(local_time),
            );
            let mut later = job.clone();
            later.id = "another-job".into();
            later.created += 1.0;
            assert_eq!(
                organized_folder(&job).unwrap(),
                organized_folder(&later).unwrap()
            );
            later.created += 86400.0;
            assert_ne!(
                organized_folder(&job).unwrap(),
                organized_folder(&later).unwrap()
            );
        }
    }
}

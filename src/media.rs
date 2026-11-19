use crate::{
    encoder::{self, Encoder},
    model::{self, Job, QUALITIES},
    process::{self, Context},
    tools,
};
use anyhow::{Context as _, Result, bail, ensure};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    thread,
    time::Duration,
};

pub const TS: &str = "Video/Live - MPEG-TS (.ts)";
pub const AUTO_LIVE: &str = "Live - MPEG-TS (.ts) - Auto GPU/CPU";
pub const NVENC_LIVE: &str = "Live - MPEG-TS (.ts) - NVIDIA NVENC";
pub const STREAMLINK: &str = "Live - Streamlink - Original quality (.ts)";
pub const TIME_RANGE_MODE: usize = 11;
pub const SUBTITLES_MODE: usize = 12;
pub const SUBTITLE_LANGUAGES: [&str; 3] = ["en.*,-live_chat", "th.*,-live_chat", "all,-live_chat"];
pub const MODES: [&str; 13] = [
    "MP4 - Best quality",
    "MP4 - Up to 4K",
    "MP4 - Compatible (H.264/AAC)",
    "MP3 - Best quality",
    "Playlist - MP4 best quality",
    "Playlist - MP4 compatible (H.264/AAC)",
    "Playlist - MP3 best quality",
    TS,
    AUTO_LIVE,
    NVENC_LIVE,
    STREAMLINK,
    "Download - Time range",
    "Subtitles only",
];
pub const SPOTIFY_TRACK_MODE: usize = MODES.len();
pub const SPOTIFY_PLAYLIST_MODE: usize = MODES.len() + 1;
pub const SELECT_PLAYLIST_MODE: usize = MODES.len() + 2;
pub const MODE_LABELS: [&str; MODES.len() + 3] = {
    let mut labels = [""; MODES.len() + 3];
    let mut index = 0;
    while index < MODES.len() {
        labels[index] = MODES[index];
        index += 1;
    }
    labels[3] = "Audio - choose format";
    labels[6] = "Playlist - Audio";
    labels[SPOTIFY_TRACK_MODE] = "Spotify - Track";
    labels[SPOTIFY_PLAYLIST_MODE] = "Spotify - Playlist / Album";
    labels[SELECT_PLAYLIST_MODE] = "Playlist - Select items";
    labels
};
pub const COMPRESS_MODE: usize = 4;
pub const EXTRACT_AUDIO_MODE: usize = 5;
pub const COMPRESS_LIMITS: [Option<u32>; 3] = [None, Some(1080), Some(720)];
pub const COMPRESS_LEVELS: [&str; 3] = ["Balanced", "Smaller", "Smallest"];
pub const CONVERT_MODES: [&str; 6] = [
    "TS to MP4 - Fast (original quality)",
    "TS to MP4 - Compatible H.264",
    "TS to MP4 - H.265 NVIDIA",
    "H.265/HEVC to H.264/AAC",
    "Compress MP4",
    "Extract Audio",
];
pub const CUT_MODES: [&str; 2] = [
    "Fast - original quality (keyframe cut)",
    "Compatible MP4 - accurate H.264/AAC",
];
pub const JOIN_MODES: [&str; 2] = [
    "Fast - no re-encoding (same format only)",
    "Compatible MP4 - re-encode H.264/AAC",
];

pub fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}
pub fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
pub fn live(mode: &str) -> bool {
    [AUTO_LIVE, NVENC_LIVE, STREAMLINK].contains(&mode)
}
pub fn height(quality: &str) -> Option<u32> {
    quality
        .split_whitespace()
        .find_map(|s| s.trim_end_matches('p').parse().ok())
}

fn cookies(arguments: &mut Vec<String>, job: &Job, streamlink: bool) {
    if let Some(path) = &job.cookies_file {
        arguments.extend([
            if streamlink {
                "--http-cookies-file"
            } else {
                "--cookies"
            }
            .into(),
            text(path),
        ]);
    }
}

fn download_cap(job: &Job) -> Option<u32> {
    height(&job.quality).or_else(|| (job.mode == MODES[1]).then_some(2160))
}

fn compatible_download_mode(mode: &str) -> bool {
    [MODES[2], MODES[5]].contains(&mode)
}

pub fn audio_mode(mode: &str) -> bool {
    [MODES[3], MODES[6]].contains(&mode)
}

pub fn audio_encoder(format: &str) -> Option<&'static str> {
    match format {
        "MP3" => Some("libmp3lame"),
        "M4A" => Some("aac"),
        _ => None,
    }
}

pub fn audio_only(job: &Job) -> bool {
    job.mode != MODES[SUBTITLES_MODE] && (job.quality == QUALITIES[5] || audio_mode(&job.mode))
}

pub fn download_arguments(job: &Job, folder: &Path) -> Result<Vec<String>> {
    ensure!(
        MODES.contains(&job.mode.as_str()),
        "Unknown download mode: {}",
        job.mode
    );
    let audio = audio_only(job);
    let audio_extension = job.audio_format.to_ascii_lowercase();
    let playlist = job.mode.starts_with("Playlist");
    let mut arguments = args(&[
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
    ]);
    cookies(&mut arguments, job, false);
    arguments.extend(["-P".into(), text(folder)]);
    if let Some(ffmpeg) = tools::find("ffmpeg") {
        arguments.extend(["--ffmpeg-location".into(), text(ffmpeg.parent().unwrap())]);
    }
    if job.mode == MODES[SUBTITLES_MODE] {
        arguments.extend(args(&[
            "--no-playlist",
            "--skip-download",
            "--write-subs",
            "--write-auto-subs",
            "--sub-langs",
            &job.subtitle_language,
            "--sub-format",
            "srt/vtt/best",
            "--convert-subs",
            "srt",
            "-o",
            "%(title)s [%(id)s].%(ext)s",
            "--",
            &job.url,
        ]));
        return Ok(arguments);
    }
    arguments.push("--embed-metadata".into());
    arguments.push(
        if playlist {
            "--yes-playlist"
        } else {
            "--no-playlist"
        }
        .into(),
    );
    arguments.extend([
        "-o".into(),
        if playlist {
            "%(playlist_title)s/%(playlist_index)03d - %(title)s [%(id)s].%(ext)s"
        } else if audio {
            "%(title)s [%(id)s].%(ext)s"
        } else {
            "%(title)s [%(height)sp] [%(id)s].%(ext)s"
        }
        .into(),
    ]);
    if playlist {
        let archive = if audio {
            format!("playlist-{audio_extension}.txt")
        } else if compatible_download_mode(&job.mode) {
            "playlist-mp4-compatible.txt".into()
        } else {
            "playlist-mp4.txt".into()
        };
        fs::create_dir_all(folder.join("_archives"))?;
        arguments.extend([
            "--download-archive".into(),
            text(&folder.join("_archives").join(archive)),
        ]);
        if !job.playlist_items.is_empty() {
            let indices = job
                .playlist_items
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",");
            arguments.extend(["--playlist-items".into(), indices]);
        }
    }
    if let Some([start, end]) = job.download_range {
        arguments.extend([
            "--download-sections".into(),
            format!("*{start:.3}-{end:.3}"),
            "--force-keyframes-at-cuts".into(),
        ]);
    }
    if audio {
        let extract_format = if let Some(bitrate) = job.audio_bitrate {
            let encoder = audio_encoder(&job.audio_format)
                .context("Choose MP3 or M4A for a bitrate preset")?;
            // A lossless intermediate makes the bitrate apply even when the input already uses the selected codec.
            arguments.extend([
                "--paths".into(),
                format!("temp:{}", folder.join(".work").display()),
                "--recode-video".into(),
                audio_extension.clone(),
                "--postprocessor-args".into(),
                format!("VideoConvertor+ffmpeg_o:-c:a {encoder} -b:a {bitrate}k"),
            ]);
            "flac"
        } else {
            &audio_extension
        };
        arguments.extend(args(&[
            "-f",
            "ba/b",
            "-x",
            "--audio-format",
            extract_format,
            "--audio-quality",
            "0",
        ]));
        if audio_extension != "wav" {
            arguments.extend(args(&["--embed-thumbnail", "--convert-thumbnails", "jpg"]));
        }
    } else {
        let cap = download_cap(job);
        arguments.extend([
            "-f".into(),
            cap.map(|h| {
                format!("bv*[height<={h}]+ba/b[height<={h}]/bv*[height<=?{h}]+ba/b[height<=?{h}]")
            })
            .unwrap_or_else(|| "bv*+ba/b".into()),
        ]);
        if compatible_download_mode(&job.mode) || job.mode == TS {
            arguments.extend(args(&["-S", "vcodec:h264,res,acodec:aac"]));
        }
        arguments.extend(args(&[
            "--merge-output-format",
            "mp4",
            "--remux-video",
            "mp4",
        ]));
    }
    arguments.extend(["--".into(), job.url.clone()]);
    Ok(arguments)
}

fn compatible(arguments: &mut Vec<String>) {
    encode_mp4(arguments, "0:v:0?", "20", "192k");
}

fn encode_mp4(arguments: &mut Vec<String>, video: &str, crf: &str, audio_rate: &str) {
    arguments.extend(args(&[
        "-map",
        video,
        "-map",
        "0:a:0?",
        "-map_metadata",
        "0",
        "-c:v",
        "libx264",
        "-preset",
        "medium",
        "-crf",
        crf,
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-b:a",
        audio_rate,
        "-ar",
        "48000",
    ]));
}

fn run_auto_h264(ffmpeg: &Path, arguments: &[String], ctx: &Context) -> Result<process::Output> {
    let Some(index) = arguments.windows(8).position(|a| {
        a[..3] == ["-c:v", "libx264", "-preset"]
            && a[4] == "-crf"
            && a[6..] == ["-pix_fmt", "yuv420p"]
    }) else {
        return ctx.run(ffmpeg, arguments, false, false);
    };
    let encoder = encoder::auto(ffmpeg, false);
    ctx.emit(format!("Video encoder: {}", encoder.name));
    if encoder.name == encoder::CPU.name {
        return ctx.run(ffmpeg, arguments, false, false);
    }

    ctx.check()?;
    let output = Path::new(arguments.last().context("Missing output path")?);
    let mut accelerated = arguments.to_vec();
    let no_overwrite = accelerated.get(2).is_some_and(|flag| flag == "-n");
    let staging = if no_overwrite {
        ensure!(
            !output.exists(),
            "Output already exists; existing file kept"
        );
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(&ctx.folder);
        Some(
            tempfile::Builder::new()
                .prefix(".pixclip-encode-")
                .tempdir_in(parent)?,
        )
    } else {
        // The callers with -y already write to private temporary files.
        None
    };
    let target = staging.as_ref().map_or_else(
        || output.to_path_buf(),
        |dir| dir.path().join(output.file_name().unwrap()),
    );
    *accelerated.last_mut().unwrap() = text(&target);
    encoder.configure(
        &mut accelerated,
        index..index + 8,
        &arguments[index + 3],
        &arguments[index + 5],
    );
    let result = match ctx.run(ffmpeg, &accelerated, false, false) {
        Ok(result) => result,
        Err(error) => {
            if let Some(staging) = staging
                && target.metadata().is_ok_and(|file| file.len() > 0)
            {
                let folder = staging.keep();
                ctx.emit(format!("Partial encode kept in {}", folder.display()));
            }
            return Err(error);
        }
    };
    if result.code == 0 && valid_duration(&target, ctx).is_ok() {
        if no_overwrite {
            tempfile::TempPath::try_from_path(&target)?.persist_noclobber(output)?;
        }
        return Ok(result);
    }
    ctx.check()?;
    ctx.emit("Hardware encode failed; retrying with CPU libx264.");
    ctx.run(ffmpeg, arguments, false, false)
}

fn download_media(job: &Job, directory: &Path, ctx: &Context) -> Result<process::Output> {
    let yt_dlp = tools::require("yt-dlp")?;
    let arguments = download_arguments(job, directory)?;
    if job.download_range.is_some() && !audio_only(job) {
        let ffmpeg = tools::require("ffmpeg")?;
        let encoder = encoder::auto(&ffmpeg, false);
        ctx.emit(format!("Time range video encoder: {}", encoder.name));
        if encoder.name != encoder::CPU.name {
            let mut options = arguments.clone();
            let mut extra = args(&["--downloader-args", ""]);
            let mut video = encoder.video("medium", "20");
            if let Some(device) = &encoder.device {
                extra.extend([
                    "--downloader-args".into(),
                    format!("ffmpeg_i1:-vaapi_device {}", text(device)),
                ]);
                video.extend(args(&["-vf", "format=nv12,hwupload"]));
            }
            extra[1] = format!("ffmpeg_o:{}", video.join(" "));
            let separator = options.len() - 2;
            options.splice(separator..separator, extra);
            let result = ctx.run(&yt_dlp, &options, false, false)?;
            if result.code == 0 {
                return Ok(result);
            }
            ctx.emit("Hardware range encode failed; retrying with yt-dlp defaults.");
        }
    }
    ctx.run(&yt_dlp, &arguments, false, false)
}

fn finish(arguments: &mut Vec<String>, output: &Path) {
    if output
        .extension()
        .is_some_and(|ext| ext == "mp4" || ext == "m4a")
    {
        arguments.extend(args(&["-movflags", "+faststart"]));
    }
    arguments.extend(args(&["-progress", "pipe:1", "-nostats"]));
    arguments.push(text(output));
}

#[derive(Clone, Copy)]
pub enum Conversion<'a> {
    Video(usize),
    Compress {
        limit: Option<u32>,
        level: usize,
    },
    Audio {
        format: &'a str,
        bitrate: Option<u16>,
    },
}

pub fn convert(input: &Path, conversion: Conversion<'_>) -> Result<(PathBuf, Vec<String>)> {
    ensure!(input.is_file(), "Choose an existing video file");
    let stem = input
        .file_stem()
        .context("Invalid input filename")?
        .to_string_lossy();
    let output = match conversion {
        Conversion::Video(mode) if mode < COMPRESS_MODE => {
            let suffix = ["", "_converted", "_H265_NVENC", "_H264"][mode];
            input.with_file_name(format!("{stem}{suffix}.mp4"))
        }
        Conversion::Video(_) => bail!("Choose a valid conversion mode"),
        Conversion::Compress { limit, level } => {
            ensure!(
                COMPRESS_LIMITS.contains(&limit),
                "Choose 1080p, 720p or keep resolution"
            );
            let name = COMPRESS_LEVELS
                .get(level)
                .context("Choose Balanced, Smaller or Smallest")?;
            let mut suffix = limit.map(|limit| format!("_{limit}p")).unwrap_or_default();
            if level != 0 {
                suffix.push('_');
                suffix.push_str(&name.to_ascii_lowercase());
            }
            input.with_file_name(format!("{stem}_compressed{suffix}.mp4"))
        }
        Conversion::Audio { format, bitrate } => {
            ensure!(
                model::AUDIO_FORMATS.contains(&format),
                "Choose MP3, M4A, FLAC or WAV"
            );
            ensure!(
                model::AUDIO_BITRATES.contains(&bitrate),
                "Choose Best quality, 320, 192 or 128 kbps"
            );
            ensure!(
                bitrate.is_none() || audio_encoder(format).is_some(),
                "Bitrate is selectable for MP3 and M4A"
            );
            input.with_file_name(format!("{stem}_audio.{}", format.to_ascii_lowercase()))
        }
    };
    ensure!(
        output != input,
        "Output must be different from the input file"
    );
    if output.exists() {
        ensure!(
            input.canonicalize()? != output.canonicalize()?,
            "Output must be different from the input file"
        );
    }
    let mut arguments = args(&["-hide_banner", "-nostdin", "-n", "-i"]);
    arguments.push(text(input));
    match conversion {
        Conversion::Video(0) => arguments.extend(args(&[
            "-map",
            "0:v?",
            "-map",
            "0:a?",
            "-map_metadata",
            "0",
            "-c",
            "copy",
        ])),
        Conversion::Video(2) => arguments.extend(args(&[
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
        ])),
        Conversion::Video(_) => compatible(&mut arguments),
        Conversion::Compress { limit, level } => {
            let (crf, audio_rate) = [("27", "128k"), ("31", "96k"), ("35", "80k")][level];
            encode_mp4(&mut arguments, "0:V:0", crf, audio_rate);
            let filter = match limit {
                Some(short) => {
                    let long = short * 16 / 9;
                    format!(
                        "scale='min(iw,if(gte(iw,ih),{long},{short}))':'min(ih,if(gte(iw,ih),{short},{long}))':force_original_aspect_ratio=decrease:force_divisible_by=2"
                    )
                }
                None => "pad=ceil(iw/2)*2:ceil(ih/2)*2".into(),
            };
            arguments.extend(["-vf".into(), filter]);
        }
        Conversion::Audio { format, bitrate } => {
            let encoder = match format {
                "FLAC" => "flac",
                "WAV" => "pcm_s24le",
                format => audio_encoder(format).context("Unsupported audio format")?,
            };
            arguments.extend(args(&[
                "-map",
                "0:a:0",
                "-map_metadata",
                "0",
                "-vn",
                "-c:a",
                encoder,
            ]));
            if let Some(bitrate) = bitrate {
                arguments.extend(["-b:a".into(), format!("{bitrate}k")]);
            } else if let Some(quality) = match format {
                "MP3" => Some("0"),
                "M4A" => Some("4"),
                _ => None,
            } {
                arguments.extend(args(&["-q:a", quality]));
            }
        }
    }
    finish(&mut arguments, &output);
    Ok((output, arguments))
}

pub fn cut(
    input: &Path,
    output: &Path,
    start: f64,
    end: f64,
    accurate: bool,
) -> Result<Vec<String>> {
    ensure!(
        input.is_file()
            && input != output
            && output
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("mp4")),
        "Choose a different MP4 output file"
    );
    if output.exists() {
        ensure!(
            input.canonicalize()? != output.canonicalize()?,
            "Output must be different from the input file"
        );
    }
    ensure!(
        start.is_finite() && end.is_finite() && start >= 0.0 && end > start,
        "End time must be greater than start time"
    );
    let mut arguments = args(&["-hide_banner", "-nostdin", "-n"]);
    if !accurate {
        arguments.extend(["-ss".into(), model::timecode(start)]);
    }
    arguments.extend(["-i".into(), text(input)]);
    if accurate {
        arguments.extend(["-ss".into(), model::timecode(start)]);
    }
    arguments.extend(["-t".into(), model::timecode(end - start)]);
    if accurate {
        compatible(&mut arguments);
    } else {
        arguments.extend(args(&[
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
        ]));
    }
    finish(&mut arguments, output);
    Ok(arguments)
}

pub fn concat_line(path: &Path) -> Result<String> {
    let path = path.canonicalize()?;
    let value = text(&path)
        .trim_start_matches("\\\\?\\")
        .replace('\\', "/")
        .replace('\'', "'\\''");
    Ok(format!("file '{value}'\n"))
}

pub fn join(
    input: &[PathBuf],
    output: &Path,
    list: &Path,
    accurate: bool,
) -> Result<(Vec<String>, String)> {
    ensure!(input.len() >= 2, "Add at least two clips");
    ensure!(
        output
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("mp4")),
        "Choose an MP4 output filename"
    );
    let absolute_output = if output.exists() {
        output.canonicalize()?
    } else if output.parent().is_some_and(Path::is_dir) {
        output
            .parent()
            .context("Choose an output directory")?
            .canonicalize()?
            .join(output.file_name().unwrap())
    } else {
        std::path::absolute(output)?
    };
    let mut seen = std::collections::HashSet::new();
    let mut content = String::new();
    for path in input {
        ensure!(
            path.is_file(),
            "Input clip was not found: {}",
            path.display()
        );
        let canonical = path.canonicalize()?;
        ensure!(
            canonical != absolute_output && seen.insert(canonical),
            "Output must differ from all inputs, and clips must not be duplicated"
        );
        content.push_str(&concat_line(path)?);
    }
    let mut arguments = args(&[
        "-hide_banner",
        "-nostdin",
        "-n",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
    ]);
    arguments.push(text(list));
    if accurate {
        compatible(&mut arguments);
    } else {
        arguments.extend(args(&["-c", "copy", "-map_metadata", "0"]));
    }
    arguments.extend(args(&["-avoid_negative_ts", "make_zero"]));
    finish(&mut arguments, output);
    Ok((arguments, content))
}

pub fn probe(path: &Path) -> Result<Value> {
    let bytes = process::capture(
        &tools::require("ffprobe")?,
        &args(&[
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
            &text(path),
        ]),
        Duration::from_secs(20),
    )?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn inspect_output(path: &Path, ctx: &Context) -> Result<Value> {
    let output = ctx.run(
        &tools::require("ffprobe")?,
        &args(&[
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
            &text(path),
        ]),
        true,
        false,
    )?;
    ensure!(output.code == 0, "Could not inspect output media");
    Ok(serde_json::from_str(&output.text)?)
}

fn av_tracks(media: &Value) -> [Option<&Value>; 2] {
    let streams = media["streams"].as_array().map_or(&[][..], Vec::as_slice);
    ["video", "audio"].map(|kind| streams.iter().find(|stream| stream["codec_type"] == kind))
}

pub fn duration(path: &Path) -> Result<f64> {
    let value = probe(path)?;
    duration_value(&value)
}

fn duration_value(value: &Value) -> Result<f64> {
    let length: f64 = value["format"]["duration"]
        .as_str()
        .unwrap_or("")
        .parse()
        .context("Video duration is unavailable")?;
    ensure!(
        length.is_finite() && length > 0.0,
        "Recording is empty or unreadable. Partial files were kept."
    );
    Ok(length)
}

fn probe_duration(path: &Path, ctx: &Context) -> Result<Option<f64>> {
    let output = ctx.run(
        &tools::require("ffprobe")?,
        &args(&["-v", "error", "-show_format", "-of", "json", &text(path)]),
        true,
        false,
    )?;
    if output.code != 0 {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(&output.text)?;
    Ok(duration_value(&value).ok())
}

fn valid_duration(path: &Path, ctx: &Context) -> Result<f64> {
    probe_duration(path, ctx)?.context("Recording is empty or unreadable. Partial files were kept.")
}
fn signature(path: &Path, ctx: &Context) -> Result<Vec<Vec<Value>>> {
    let output = ctx.run(
        &tools::require("ffprobe")?,
        &args(&["-v", "error", "-show_streams", "-of", "json", &text(path)]),
        true,
        false,
    )?;
    ensure!(output.code == 0, "Could not inspect clip streams");
    let value: Value = serde_json::from_str(&output.text)?;
    stream_signature(&value)
}

fn stream_signature(value: &Value) -> Result<Vec<Vec<Value>>> {
    Ok(value["streams"]
        .as_array()
        .context("No media streams")?
        .iter()
        .filter(|s| s["codec_type"] == "video" || s["codec_type"] == "audio")
        .map(|stream| {
            [
                "codec_type",
                "codec_name",
                "profile",
                "width",
                "height",
                "pix_fmt",
                "sample_rate",
                "channels",
                "channel_layout",
                "r_frame_rate",
                "time_base",
            ]
            .iter()
            .map(|key| stream[key].clone())
            .collect()
        })
        .collect())
}

pub fn fast_join_compatible(input: &[PathBuf]) -> Result<bool> {
    ensure!(input.len() >= 2, "Add at least two clips");
    let first = stream_signature(&probe(&input[0])?)?;
    for path in &input[1..] {
        if stream_signature(&probe(path)?)? != first {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn preview(path: &Path, seconds: f64, duration: f64) -> Result<Vec<u8>> {
    let ffmpeg = tools::require("ffmpeg")?;
    let frame_at = |seconds: f64, input_seek: bool| {
        let mut arguments = args(&["-hide_banner", "-loglevel", "error"]);
        let seek = ["-ss".into(), format!("{seconds:.3}")];
        if input_seek {
            arguments.extend(seek.clone());
        }
        arguments.extend(["-i".into(), text(path)]);
        if !input_seek {
            arguments.extend(seek);
        }
        arguments.extend(args(&[
            "-frames:v",
            "1",
            "-vf",
            "scale=854:480:force_original_aspect_ratio=decrease,setsar=1",
            "-f",
            "image2pipe",
            "-vcodec",
            "png",
            "pipe:1",
        ]));
        process::capture(&ffmpeg, &arguments, Duration::from_secs(15))
    };
    let seconds = seconds.clamp(0.0, (duration - 0.05).max(0.0));
    let bytes = frame_at(seconds, true)?;
    if !bytes.is_empty() {
        return Ok(bytes);
    }
    // Some TS streams cannot seek to a preceding keyframe through the demuxer.
    let bytes = frame_at(seconds, false)?;
    let bytes = if bytes.is_empty() && seconds > 0.0 && seconds + 0.25 >= duration {
        frame_at((seconds - 1.0).max(0.0), false)?
    } else {
        bytes
    };
    ensure!(!bytes.is_empty(), "No video frame at the selected time");
    Ok(bytes)
}

pub fn outputs(folder: &Path) -> Result<Vec<PathBuf>> {
    let mut result = vec![];
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_symlink() {
            continue;
        }
        if path.is_dir()
            && path
                .file_name()
                .is_some_and(|n| n != ".work" && n != "_archives")
        {
            result.extend(outputs(&path)?);
        } else if path.is_file()
            && path.extension().is_some_and(|s| {
                [
                    "mp4", "ts", "mkv", "mp3", "webm", "m4a", "flac", "wav", "srt", "vtt",
                ]
                .contains(&s.to_string_lossy().to_lowercase().as_str())
            })
            && path.metadata()?.len() > 0
        {
            result.push(path);
        }
    }
    Ok(result)
}

pub fn execute(job: &Job, ctx: &Context) -> Result<Vec<PathBuf>> {
    job.validate()?;
    ctx.check()?;
    if job.kind == "tools" {
        if job.mode == "Update yt-dlp" {
            let output = ctx.run(&tools::require("yt-dlp")?, &args(&["-U"]), false, false)?;
            ensure!(output.code == 0, "yt-dlp update failed");
        } else {
            tools::install(ctx, job.mode == "Source check")?;
        }
        return Ok(vec![]);
    }
    if job.kind == "local" {
        return local(job, ctx);
    }
    ensure!(MODES.contains(&job.mode.as_str()), "Unknown download mode");
    let directory = job.directory();
    fs::create_dir_all(directory.join(".work"))?;
    if live(&job.mode) {
        record_live(job, ctx)?;
    } else if job.mode == TS && !audio_only(job) {
        download_ts(job, ctx)?;
    } else {
        let output = download_media(job, &directory, ctx)?;
        ensure!(
            output.code == 0,
            "Download failed (exit {}); see job log",
            output.code
        );
    }
    let mut result = outputs(&directory)?;
    if job.mode == MODES[SUBTITLES_MODE] {
        result.retain(|path| {
            path.extension()
                .is_some_and(|ext| ext == "srt" || ext == "vtt")
        });
        ensure!(
            !result.is_empty(),
            "No subtitles found for the selected language"
        );
    }
    ensure!(
        !result.is_empty(),
        "No media file was created. The source may be offline or already archived."
    );
    if live(&job.mode) {
        let mut finishing = ctx.clone();
        finishing.deadline = None;
        for path in &result {
            let info = inspect_output(path, &finishing)?;
            duration_value(&info)?;
            if job.mode == STREAMLINK && info["format"]["format_name"] != "mpegts" {
                create_ts(path, path, &finishing)?;
            }
        }
    } else if job.mode != MODES[SUBTITLES_MODE] && !audio_only(job) {
        for path in &result {
            validate_video_download(job, path, ctx)?;
        }
    }
    for path in &result {
        ctx.emit(format!("PIXCLIP_OUTPUT={}", path.display()));
    }
    if job.auto_mp4 {
        let sources = result.clone();
        for path in sources
            .iter()
            .filter(|p| p.extension().is_some_and(|s| s == "ts"))
        {
            result.push(remux(path, ctx)?);
        }
    }
    result.sort_unstable();
    let work = directory.join(".work");
    if audio_only(job)
        && job.audio_bitrate.is_some()
        && job.mode.starts_with("Playlist")
        && let Ok(entries) = fs::read_dir(&work)
    {
        // yt-dlp moves the encoded tracks out but leaves the temporary playlist folder.
        for entry in entries.flatten() {
            if entry
                .file_type()
                .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
            {
                let _ = fs::remove_dir(entry.path());
            }
        }
    }
    let _ = fs::remove_dir(work);
    Ok(result)
}

fn validate_video_download(job: &Job, path: &Path, ctx: &Context) -> Result<()> {
    let cap = download_cap(job);
    let compatible_mode = compatible_download_mode(&job.mode);
    if cap.is_none() && !compatible_mode {
        return Ok(());
    }
    let info = inspect_output(path, ctx)?;
    let [video, audio] = av_tracks(&info);
    let video = video.context("No video track in the downloaded file")?;
    if let Some(cap) = cap {
        ensure!(
            video["height"]
                .as_u64()
                .is_some_and(|height| height <= u64::from(cap)),
            "Downloaded video exceeds {cap}p or has unknown dimensions; file kept"
        );
    }
    if compatible_mode
        && (video["codec_name"] != "h264"
            || video["pix_fmt"] != "yuv420p"
            || audio.is_some_and(|track| track["codec_name"] != "aac"))
    {
        let temporary = tempfile::Builder::new()
            .suffix(".mp4")
            .tempfile_in(job.directory().join(".work"))?;
        let mut arguments = args(&["-hide_banner", "-nostdin", "-y", "-i", &text(path)]);
        compatible(&mut arguments);
        finish(&mut arguments, temporary.path());
        ctx.emit("PIXCLIP_PHASE=Encoding compatible H.264/AAC MP4");
        if let Ok(duration) = duration_value(&info) {
            ctx.emit(format!("media_duration={duration}"));
        }
        let result = run_auto_h264(&tools::require("ffmpeg")?, &arguments, ctx)?;
        ensure!(
            result.code == 0,
            "Compatible MP4 encoding failed; downloaded file kept"
        );
        valid_duration(temporary.path(), ctx)?;
        temporary.persist(path).map_err(|error| error.error)?;
    }
    Ok(())
}

fn local(job: &Job, ctx: &Context) -> Result<Vec<PathBuf>> {
    let mut arguments = job.arguments.clone();
    let work = job.folder.join(format!(".pixclip-work-{}", job.id));
    fs::create_dir_all(&work)?;
    for (index, (old, contents)) in job.temporary.iter().enumerate() {
        let path = work.join(format!("concat-{index}.txt"));
        fs::write(&path, contents)?;
        for argument in &mut arguments {
            if argument == old {
                *argument = text(&path);
            }
        }
    }
    let output = arguments
        .last()
        .map(PathBuf::from)
        .context("Missing output path")?;
    if job.mode == "Joining clips" && arguments.iter().any(|a| a == "copy") {
        let mut first = None;
        for input in &job.inputs {
            let value = signature(input, ctx)?;
            if let Some(first) = &first {
                ensure!(
                    *first == value,
                    "Clip formats differ. Select Compatible MP4 to re-encode."
                );
            } else {
                first = Some(value);
            }
        }
    }
    let source = arguments
        .iter()
        .position(|a| a == "-i")
        .and_then(|i| arguments.get(i + 1))
        .map(PathBuf::from);
    let source_duration = if job.mode == "Joining clips" && !job.inputs.is_empty() {
        let mut total = 0.0;
        for source in &job.inputs {
            total += probe_duration(source, ctx)?.unwrap_or(0.0);
        }
        (total > 0.0).then_some(total)
    } else if let Some(source) = source {
        probe_duration(&source, ctx)?
    } else {
        None
    };
    if let Some(mut duration) = source_duration {
        if job.mode == "Cutting clip" {
            let value = |flag: &str| {
                arguments
                    .iter()
                    .position(|a| a == flag)
                    .and_then(|i| arguments.get(i + 1))
                    .and_then(|s| model::parse_time(s).ok())
                    .unwrap_or(0.0)
            };
            ensure!(
                value("-ss") < duration && value("-ss") + value("-t") <= duration + 0.05,
                "Clip time is beyond the video duration"
            );
            duration = value("-t");
        }
        ctx.emit(format!("media_duration={duration}"));
    }
    let result = run_auto_h264(&tools::require("ffmpeg")?, &arguments, ctx)?;
    ensure!(
        result.code == 0,
        "Media command failed; partial files kept (exit {})",
        result.code
    );
    ensure!(
        output.is_file() && output.metadata()?.len() > 0,
        "No media output was created"
    );
    valid_duration(&output, ctx)?;
    ctx.emit(format!("PIXCLIP_OUTPUT={}", output.display()));
    for file in fs::read_dir(&work)? {
        let _ = fs::remove_file(file?.path());
    }
    let _ = fs::remove_dir(&work);
    Ok(vec![output])
}

fn kick_vod_id(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    if !matches!(url.host_str(), Some("kick.com" | "www.kick.com")) {
        return None;
    }
    let parts: Vec<_> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let id = if parts.len() >= 3 && parts[1] == "videos" {
        parts[2]
    } else if parts.len() >= 2 && matches!(parts[0], "videos" | "video") {
        parts[1]
    } else {
        return None;
    };
    uuid::Uuid::parse_str(id).ok()?;
    Some(id.into())
}

fn unique_recording_path(folder: &Path, filename: &str) -> PathBuf {
    let base = Path::new(filename);
    let stem = model::safe_component(&base.file_stem().unwrap_or_default().to_string_lossy());
    let extension = model::safe_component(&base.extension().unwrap_or_default().to_string_lossy());
    let filename = format!("{stem}.{extension}");
    let mut path = folder.join(&filename);
    let mut counter = 2;
    while path.exists() {
        path = folder.join(format!("{stem} ({counter}).{extension}"));
        counter += 1;
    }
    path
}

fn recording_title(job: &Job, ctx: &Context) -> Result<String> {
    let mut arguments = args(&[
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
    ]);
    cookies(&mut arguments, job, false);
    arguments.extend(["--".into(), job.url.clone()]);
    let output = ctx.run(&tools::require("yt-dlp")?, &arguments, true, false)?;
    if output.code != 0 {
        for line in output.text.lines() {
            ctx.emit(process::public_line(line));
        }
        bail!("Could not read video information; see job log");
    }
    let name = output
        .text
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or("downloaded_video.ts")
        .trim();
    let name = Path::new(name)
        .file_name()
        .context("Invalid recording filename")?
        .to_string_lossy();
    Ok(if name.to_lowercase().ends_with(".ts") {
        name.into_owned()
    } else {
        format!("{name}.ts")
    })
}

fn download_ts(job: &Job, ctx: &Context) -> Result<()> {
    tools::require("ffmpeg")?;
    if let Some(id) = kick_vod_id(&job.url) {
        return kick_ts(job, &id, ctx);
    }
    let directory = job.directory();
    let work = directory.join(".work");
    ctx.emit("PIXCLIP_PHASE=Reading video information");
    let filename = recording_title(job, ctx)?;
    let mut arguments = download_arguments(job, &work)?;
    if let Some(index) = arguments.iter().position(|a| a == "-o") {
        arguments[index + 1] = "source.%(ext)s".into();
    }
    let result = ctx.run(&tools::require("yt-dlp")?, &arguments, false, false)?;
    ensure!(
        result.code == 0,
        "Download failed (exit {}); temporary source kept",
        result.code
    );
    let source = outputs(&work)?
        .into_iter()
        .find(|p| p.file_stem().is_some_and(|s| s == "source"))
        .context("Downloaded source was not found; temporary files kept")?;
    if let Some(duration) = probe_duration(&source, ctx)? {
        ctx.emit(format!("media_duration={duration}"));
    }
    let target = unique_recording_path(&directory, &filename);
    create_ts(&source, &target, ctx)?;
    let _ = fs::remove_file(source);
    Ok(())
}

fn create_ts(source: &Path, target: &Path, ctx: &Context) -> Result<()> {
    let replacing_recording = source == target;
    ensure!(
        replacing_recording || !target.exists(),
        "Recording output already exists; existing file kept"
    );
    let source_tracks = av_tracks(&inspect_output(source, ctx)?).map(|track| track.is_some());
    ensure!(
        source_tracks.contains(&true),
        "Source has no audio or video"
    );
    let keeps_tracks = |path: &Path| -> Result<bool> {
        Ok(av_tracks(&inspect_output(path, ctx)?).map(|track| track.is_some()) == source_tracks)
    };
    let parent = target.parent().context("Missing output folder")?;
    let temporary = tempfile::Builder::new()
        .prefix(".pixclip-ts-")
        .suffix(".ts")
        .tempfile_in(parent)?;
    let (file, path) = temporary.keep().map_err(|e| e.error)?;
    drop(file);
    let mut arguments = args(&[
        "-hide_banner",
        "-nostdin",
        "-y",
        "-i",
        &text(source),
        "-map",
        "0:v:0?",
        "-map",
        "0:a:0?",
        "-map_metadata",
        "0",
        "-c",
        "copy",
        "-progress",
        "pipe:1",
        "-nostats",
        "-f",
        "mpegts",
        &text(&path),
    ]);
    ctx.emit("PIXCLIP_PHASE=Creating MPEG-TS without re-encoding");
    let result = ctx.run(&tools::require("ffmpeg")?, &arguments, false, false)?;
    if result.code != 0 || !keeps_tracks(&path).unwrap_or(false) {
        ctx.check()?;
        ctx.emit("Fast remux was not compatible; falling back to H.264/AAC encoding.");
        let at = arguments.iter().position(|s| s == "-c").unwrap();
        arguments.splice(
            at..at + 2,
            args(&[
                "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p",
                "-c:a", "aac", "-b:a", "192k", "-ar", "48000",
            ]),
        );
        let result = run_auto_h264(&tools::require("ffmpeg")?, &arguments, ctx)?;
        ensure!(
            result.code == 0,
            "TS conversion failed; temporary source and TS kept at {}",
            path.display()
        );
    }
    ensure!(
        keeps_tracks(&path)?,
        "TS conversion lost an audio or video track; source kept"
    );
    valid_duration(&path, ctx)?;
    let temporary = tempfile::TempPath::try_from_path(&path)?;
    if replacing_recording {
        temporary.persist(target).map_err(|e| e.error)?;
    } else {
        temporary.persist_noclobber(target).map_err(|e| e.error)?;
    }
    Ok(())
}

fn kick_ts(job: &Job, video_id: &str, ctx: &Context) -> Result<()> {
    ctx.emit("PIXCLIP_PHASE=Reading Kick VOD playback");
    ctx.check()?;
    let value = tools::kick_playback(video_id, ctx)?;
    let mut stream = value["playback_url"]["vod"]
        .as_str()
        .context("Kick did not return a VOD playback URL")?
        .to_string();
    if let Some(cap) = height(&job.quality) {
        let playlist = String::from_utf8(tools::request_bytes(&stream, None, ctx)?)?;
        let base = url::Url::parse(&stream)?;
        let lines: Vec<_> = playlist.lines().collect();
        let mut variants = vec![];
        for pair in lines.windows(2) {
            if let Some(dimensions) = pair[0].split("RESOLUTION=").nth(1) {
                let h = dimensions
                    .split(',')
                    .next()
                    .and_then(|s| s.split('x').nth(1))
                    .and_then(|s| s.parse::<u32>().ok());
                if let Some(h) = h.filter(|h| *h <= cap) {
                    ensure!(
                        !pair[0].contains("AUDIO="),
                        "This Kick stream uses separate audio. Select Best available to preserve audio."
                    );
                    variants.push((h, base.join(pair[1])?.to_string()));
                }
            }
        }
        stream = variants
            .into_iter()
            .max_by_key(|v| v.0)
            .context("Requested Kick resolution is unavailable. Select Best available.")?
            .1;
    }
    let title = model::safe_component(
        value["video_session"]["video_title"]
            .as_str()
            .unwrap_or("Kick VOD"),
    );
    let output = unique_recording_path(&job.directory(), &format!("{title} [{video_id}].ts"));
    let result = ctx.run(
        &tools::require("ffmpeg")?,
        &args(&[
            "-hide_banner",
            "-nostdin",
            "-n",
            "-user_agent",
            tools::USER_AGENT,
            "-headers",
            "Origin: https://kick.com\r\nReferer: https://kick.com/\r\n",
            "-i",
            &stream,
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
            &text(&output),
        ]),
        false,
        false,
    )?;
    ensure!(
        result.code == 0,
        "Kick VOD fallback failed; partial files kept"
    );
    Ok(())
}

fn streamlink_filename(url: &str) -> Result<String> {
    let parsed = url::Url::parse(url)?;
    let host = parsed.host_str().unwrap_or("live_stream");
    let label = if host == "twitch.tv"
        || host.ends_with(".twitch.tv")
        || host == "kick.com"
        || host.ends_with(".kick.com")
    {
        parsed
            .path_segments()
            .and_then(|mut p| p.find(|s| !s.is_empty()))
            .unwrap_or(host)
    } else if host == "youtu.be" || host == "youtube.com" || host.ends_with(".youtube.com") {
        "youtube_live"
    } else {
        host
    };
    Ok(format!(
        "{} [{}].ts",
        model::safe_component(label),
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ))
}

fn record_live(job: &Job, ctx: &Context) -> Result<()> {
    ensure!(
        job.quality != QUALITIES[5],
        "Choose a video quality for Live recording"
    );
    let ffmpeg = tools::require("ffmpeg")?;
    if job.mode == NVENC_LIVE {
        let output = ctx.run(&ffmpeg, &args(&["-hide_banner", "-encoders"]), true, false)?;
        ensure!(
            output.code == 0 && output.text.contains("h264_nvenc"),
            "FFmpeg does not include h264_nvenc. Install an NVENC-enabled build and NVIDIA driver."
        );
    }
    let mut encoder = if job.mode == AUTO_LIVE {
        encoder::auto(&ffmpeg, true).clone()
    } else {
        encoder::NVENC
    };
    let filename = if job.mode == STREAMLINK {
        streamlink_filename(&job.url)?
    } else {
        recording_title(job, ctx)?
    };
    let mut last_code = 1;
    for attempt in 0..6 {
        if ctx.deadline.is_some_and(|d| model::now() >= d) {
            break;
        }
        ctx.check()?;
        let output = unique_recording_path(&job.directory(), &filename);
        let attempt_result = (|| -> Result<process::Output> {
            let (executable, arguments) = if job.mode == STREAMLINK {
                let mut arguments = args(&[
                    "--no-config",
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
                ]);
                cookies(&mut arguments, job, true);
                arguments.extend(args(&[
                    "--ffmpeg-ffmpeg",
                    &text(&ffmpeg),
                    "--ffmpeg-fout",
                    "mpegts",
                    "--ffmpeg-video-transcode",
                    "copy",
                    "--ffmpeg-audio-transcode",
                    "copy",
                    "--output",
                    &text(&output),
                    &job.url,
                ]));
                let quality = height(&job.quality)
                    .map(|cap| {
                        [2160, 1440, 1080, 720, 480, 360, 240, 160]
                            .into_iter()
                            .filter(|h| *h <= cap)
                            .map(|h| format!("{h}p60,{h}p"))
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_else(|| "best".into());
                arguments.push(quality);
                (tools::require("streamlink")?, arguments)
            } else {
                let mut arguments = args(&["--ignore-config", "--no-playlist", "-g", "-f"]);
                arguments.push(
                    height(&job.quality)
                        .map(|h| format!("b[height<={h}]"))
                        .unwrap_or_else(|| "b".into()),
                );
                cookies(&mut arguments, job, false);
                arguments.extend(["--".into(), job.url.clone()]);
                let metadata = ctx.run(&tools::require("yt-dlp")?, &arguments, true, false)?;
                ensure!(
                    metadata.code == 0,
                    "Could not resolve a muxed Live stream; try Streamlink mode"
                );
                let stream = metadata
                    .text
                    .lines()
                    .find(|line| line.starts_with("http://") || line.starts_with("https://"))
                    .context("No playable Live URL was returned")?;
                ctx.emit(format!("Live video encoder: {}", encoder.name));
                (ffmpeg.clone(), live_arguments(stream, &output, &encoder))
            };
            ctx.emit("PIXCLIP_PHASE=Recording / waiting for Live");
            let result = ctx.run(&executable, &arguments, false, true)?;
            if result.code != 0 && job.mode == AUTO_LIVE && encoder.name != encoder::CPU.name {
                encoder = encoder::CPU;
                ctx.emit("Hardware encoder failed; retrying with the CPU encoder.");
            }
            Ok(result)
        })();
        let result = match attempt_result {
            Ok(result) => result,
            Err(error) => {
                ctx.check()?;
                if attempt == 5 || ctx.deadline.is_some_and(|end| model::now() >= end) {
                    return Err(error);
                }
                ctx.emit(format!(
                    "WARNING: {}",
                    process::public_line(&format!("{error:#}"))
                ));
                process::Output {
                    code: 1,
                    text: String::new(),
                    timed_out: false,
                }
            }
        };
        last_code = result.code;
        if result.timed_out || (last_code == 0 && ctx.deadline.is_none()) {
            break;
        }
        if attempt < 5 {
            ctx.emit(format!(
                "PIXCLIP_PHASE=Reconnecting ({}/5); existing segments kept",
                attempt + 1
            ));
            for _ in 0..50 {
                ensure!(
                    !ctx.cancel.load(Ordering::Relaxed),
                    "Cancelled; partial files kept"
                );
                if ctx.deadline.is_some_and(|d| model::now() >= d) {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    ensure!(
        last_code == 0,
        "Live recording failed; existing segments were kept"
    );
    Ok(())
}

pub fn live_arguments(url: &str, output: &Path, encoder: &Encoder) -> Vec<String> {
    let mut arguments = args(&[
        "-hide_banner",
        "-nostdin",
        "-n",
        "-i",
        url,
        "-map",
        "0:v:0?",
        "-map",
        "0:a:0?",
    ]);
    let end = arguments.len();
    encoder.configure(&mut arguments, end..end, "veryfast", "23");
    arguments.extend(args(&[
        "-c:a",
        "aac",
        "-b:a",
        "160k",
        "-ar",
        "48000",
        "-progress",
        "pipe:1",
        "-nostats",
        "-flush_packets",
        "1",
        "-f",
        "mpegts",
        &text(output),
    ]));
    arguments
}

pub fn remux(source: &Path, ctx: &Context) -> Result<PathBuf> {
    ctx.check()?;
    model::require_space(
        &ctx.folder,
        ctx.reserve,
        (source.metadata()?.len() as f64 * 1.1) as u64,
    )?;
    let target = source.with_extension("mp4");
    ensure!(!target.exists(), "MP4 already exists; original TS kept");
    let temporary = tempfile::Builder::new()
        .prefix("pixclip-")
        .suffix(".remuxing.mp4")
        .tempfile_in(source.parent().unwrap())?;
    let mut ctx = ctx.clone();
    ctx.deadline = None;
    ctx.emit("PIXCLIP_PHASE=TS to MP4 (copying; original TS kept)");
    let outcome = (|| -> Result<()> {
        let result = ctx.run(
            &tools::require("ffmpeg")?,
            &args(&[
                "-hide_banner",
                "-nostdin",
                "-y",
                "-i",
                &text(source),
                "-map",
                "0:v:0?",
                "-map",
                "0:a:0?",
                "-c",
                "copy",
                "-movflags",
                "+faststart",
                &text(temporary.path()),
            ]),
            false,
            false,
        )?;
        ensure!(
            result.code == 0 && temporary.path().metadata()?.len() > 0,
            "MP4 remux failed; original TS kept"
        );
        let original = valid_duration(source, &ctx)?;
        let converted = valid_duration(temporary.path(), &ctx)?;
        ensure!(
            (original - converted).abs() <= (original * 0.02).max(2.0),
            "MP4 duration check failed; original TS kept"
        );
        Ok(())
    })();
    if let Err(error) = outcome {
        let _ = temporary.keep();
        return Err(error);
    }
    match temporary.persist_noclobber(&target) {
        Ok(_) => {
            ctx.emit(format!("PIXCLIP_OUTPUT={}", target.display()));
            Ok(target)
        }
        Err(error) => {
            let message = error.error.to_string();
            let _ = error.file.keep();
            bail!("Could not save MP4: {message}. Original TS and temporary MP4 were kept.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recording_names_and_kick_routing_match_the_original_workflows() {
        let id = "12345678-1234-1234-1234-123456789012";
        assert_eq!(
            kick_vod_id(&format!("https://kick.com/channel/videos/{id}?x=1")),
            Some(id.into())
        );
        assert!(
            kick_vod_id(&format!("https://kick.com.example.com/channel/videos/{id}")).is_none()
        );
        assert!(kick_vod_id("https://kick.com/channel/videos/invalid").is_none());
        let dir = tempfile::tempdir().unwrap();
        let first = unique_recording_path(dir.path(), "My live [id].ts");
        assert_eq!(first.file_name().unwrap(), "My live [id].ts");
        fs::write(&first, b"keep").unwrap();
        let second = unique_recording_path(dir.path(), "My live [id].ts");
        assert_eq!(second.file_name().unwrap(), "My live [id] (2).ts");
        assert_eq!(
            unique_recording_path(dir.path(), &format!("{}.ts", "long title ".repeat(50)))
                .extension()
                .unwrap(),
            "ts"
        );
        assert!(
            streamlink_filename("https://www.twitch.tv/channel")
                .unwrap()
                .starts_with("channel [")
        );
        assert!(
            streamlink_filename("https://youtube.com/watch?v=id")
                .unwrap()
                .starts_with("youtube_live [")
        );
        assert_eq!(fs::read(first).unwrap(), b"keep");
    }
    #[test]
    fn download_quality_and_cookies_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let job = Job {
            url: "https://example.com/video".into(),
            mode: MODES[0].into(),
            quality: QUALITIES[2].into(),
            cookies_file: Some(dir.path().join("cookies.txt")),
            ..Job::default()
        };
        let values = download_arguments(&job, dir.path()).unwrap();
        assert!(
            values.iter().any(|s| s
                == "bv*[height<=1080]+ba/b[height<=1080]/bv*[height<=?1080]+ba/b[height<=?1080]")
        );
        assert!(values.contains(&"--cookies".into()));
        assert!(values.contains(&"--no-overwrites".into()));
    }

    #[test]
    fn download_options_apply_playlist_indices_time_ranges_and_caption_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut job = Job {
            url: "https://example.com/media".into(),
            folder: dir.path().into(),
            mode: MODES[4].into(),
            playlist_items: vec![1, 3],
            ..Job::default()
        };
        let options = download_arguments(&job, dir.path()).unwrap();
        assert!(
            options
                .windows(2)
                .any(|pair| pair == ["--playlist-items", "1,3"])
        );

        job.mode = MODES[TIME_RANGE_MODE].into();
        job.playlist_items.clear();
        job.download_range = Some([5.0, 8.5]);
        job.validate().unwrap();
        let options = download_arguments(&job, dir.path()).unwrap();
        assert!(
            options
                .windows(2)
                .any(|pair| pair == ["--download-sections", "*5.000-8.500"])
        );
        job.download_range = Some([8.5, 5.0]);
        assert!(job.validate().is_err());

        job.mode = MODES[SUBTITLES_MODE].into();
        job.download_range = None;
        job.subtitle_language = SUBTITLE_LANGUAGES[1].into();
        let options = download_arguments(&job, dir.path()).unwrap();
        assert!(
            options
                .windows(2)
                .any(|pair| pair == ["--sub-langs", "th.*,-live_chat"])
        );
        assert!(options.iter().any(|option| option == "--skip-download"));
        assert!(
            !options
                .iter()
                .any(|option| option == "--merge-output-format")
        );
    }
    #[test]
    fn playlist_audio_options_use_a_format_specific_archive() {
        let dir = tempfile::tempdir().unwrap();
        let mut job = Job {
            url: "https://example.com/playlist".into(),
            folder: dir.path().into(),
            mode: MODES[6].into(),
            ..Job::default()
        };
        let defaults = download_arguments(&job, dir.path()).unwrap();
        assert!(
            defaults
                .windows(2)
                .any(|pair| pair == ["--audio-format", "mp3"])
        );
        assert!(defaults.iter().any(|arg| arg.ends_with("playlist-mp3.txt")));

        job.audio_format = "M4A".into();
        job.audio_bitrate = Some(192);
        job.validate().unwrap();
        let selected = download_arguments(&job, dir.path()).unwrap();
        assert!(
            selected
                .windows(2)
                .any(|pair| pair == ["--recode-video", "m4a"])
        );
        assert!(selected.iter().any(|arg| arg.ends_with("playlist-m4a.txt")));
        assert!(
            selected
                .iter()
                .any(|arg| arg.ends_with("-c:a aac -b:a 192k"))
        );

        job.audio_format = "WAV".into();
        assert!(job.validate().is_err());
        job.audio_bitrate = None;
        job.validate().unwrap();
        let selected = download_arguments(&job, dir.path()).unwrap();
        assert!(
            selected
                .windows(2)
                .any(|pair| pair == ["--audio-format", "wav"])
        );
        assert!(!selected.iter().any(|arg| arg == "--embed-thumbnail"));
    }
    #[test]
    #[ignore = "Requires real FFmpeg and ffprobe; synthetic media only"]
    fn vp9_download_keeps_video_in_compatible_mp4_and_ts_modes() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        fs::create_dir(folder.join(".work")).unwrap();
        let ctx = Context {
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            deadline: None,
            folder: folder.into(),
            reserve: 0.25,
            log: std::sync::Arc::new(|_| {}),
        };
        let source = folder.join("vp9-opus.mp4");
        let result = ctx
            .run(
                &tools::require("ffmpeg").unwrap(),
                &args(&[
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc2=s=320x180:r=25",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440",
                    "-t",
                    "1",
                    "-c:v",
                    "libvpx-vp9",
                    "-threads",
                    "2",
                    "-c:a",
                    "libopus",
                    &text(&source),
                ]),
                true,
                false,
            )
            .unwrap();
        assert_eq!(result.code, 0, "{}", result.text);
        let assert_compatible = |path: &Path| {
            let info = probe(path).unwrap();
            let [video, audio] = av_tracks(&info);
            assert_eq!(video.unwrap()["codec_name"], "h264");
            assert_eq!(audio.unwrap()["codec_name"], "aac");
        };
        for mode in [MODES[2], MODES[5]] {
            let output = folder.join("compatible.mp4");
            fs::copy(&source, &output).unwrap();
            let job = Job {
                mode: mode.into(),
                output_folder: Some(folder.into()),
                ..Job::default()
            };
            validate_video_download(&job, &output, &ctx).unwrap();
            assert_compatible(&output);
        }
        let output = folder.join("video.ts");
        create_ts(&source, &output, &ctx).unwrap();
        assert_compatible(&output);
    }

    #[test]
    fn all_live_encoders_use_mpegts_and_do_not_overwrite() {
        for encoder in [
            "libx264",
            "h264_nvenc",
            "h264_amf",
            "h264_qsv",
            "h264_vaapi",
            "h264_videotoolbox",
        ] {
            let values = live_arguments(
                "https://example.com/live",
                Path::new("out.ts"),
                &Encoder {
                    name: encoder,
                    device: (encoder == "h264_vaapi").then(|| PathBuf::from("/dev/dri/renderD128")),
                },
            );
            assert!(values.contains(&"-n".into()));
            assert!(
                values
                    .windows(2)
                    .any(|pair| pair == ["-flush_packets", "1"])
            );
            assert!(values.contains(&encoder.into()));
            assert_eq!(&values[values.len() - 2], "mpegts");
        }
    }
}

#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod encoder;
#[cfg(test)]
mod integration;
mod jobs;
mod media;
mod model;
mod process;
mod spotify;
mod tools;

use anyhow::{Context as _, Result, bail, ensure};
use model::{Job, Settings, State};
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if !arguments.is_empty() && arguments != ["--gui"] {
        attach_console();
    }
    if let Err(error) = run(arguments) {
        eprintln!("ERROR: {error:#}");
        if std::env::args().len() == 1 || std::env::args().any(|a| a == "--gui") {
            rfd::MessageDialog::new()
                .set_title("PixClip")
                .set_description(format!("{error:#}"))
                .set_level(rfd::MessageLevel::Error)
                .show();
        }
        std::process::exit(if error.to_string().starts_with("Cancelled") {
            130
        } else {
            1
        });
    }
}

fn attach_console() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{Foundation::INVALID_HANDLE_VALUE, System::Console::*};
        let output = GetStdHandle(STD_OUTPUT_HANDLE);
        if (output.is_null() || output == INVALID_HANDLE_VALUE)
            && AttachConsole(ATTACH_PARENT_PROCESS) == 0
        {
            AllocConsole();
        }
    }
}

fn run(mut arguments: Vec<String>) -> Result<()> {
    if arguments.is_empty() || arguments == ["--gui"] {
        return app::run();
    }
    if arguments.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "PixClip - Download / Record / Convert / Cut / Join\n\nUsage: pixclip [--gui | --cli | COMMAND]\n\n  --compatibility-test\n  --install-tools [--source-check]\n  --update-ytdlp\n  --download-ts URL OUTPUT_DIR\n  --download-live-ts-auto URL OUTPUT_DIR\n  --download-live-ts-nvenc URL OUTPUT_DIR\n  --record-live-streamlink URL OUTPUT_DIR\n  --download MODE_INDEX URL OUTPUT_DIR [--audio-format FORMAT] [--audio-bitrate KBPS]\n  --convert MODE_INDEX INPUT\n  --cut INPUT OUTPUT START END [--accurate]\n  --join OUTPUT INPUT INPUT... [--accurate]\n\nDownload modes: 0-12, conversion modes: 0-5. Times: seconds, MM:SS or HH:MM:SS.\nAudio modes 3 and 6 support mp3, m4a, flac and wav. MP3/M4A accept 320, 192 or 128 kbps.\nUse --playlist-items 1,3 for playlist downloads, --start TIME --end TIME for mode 11, or --subtitle-lang en|th|all for mode 12.\nConversion mode 4 accepts --max-resolution 1080|720 and --compression balanced|smaller|smallest. Mode 5 accepts audio format and bitrate options.\nDownloads accept --cookies-file PATH. Existing outputs are kept.\nPixClip needs yt-dlp, FFmpeg, ffprobe and Streamlink; no Python runtime is required by the app."
        );
        return Ok(());
    }
    let cookies = take_option(&mut arguments, "--cookies-file")?
        .map(|path| absolute(&path))
        .transpose()?;
    let accurate = take_flag(&mut arguments, "--accurate");
    let source_check = take_flag(&mut arguments, "--source-check");
    if arguments == ["--cli"] {
        return menu(cookies);
    }
    if arguments == ["--compatibility-test"] {
        let hardware = tools::hardware();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "Compatible": true,
                "Version": env!("CARGO_PKG_VERSION"),
                "Platform": std::env::consts::OS,
                "Architecture": std::env::consts::ARCH,
                "GPU": hardware.gpu,
                "CPU": hardware.cpu,
                "Encoder": hardware.encoder,
                "FileEncoder": hardware.file_encoder,
                "YtDlpFound": tools::find("yt-dlp").is_some(),
                "FfmpegFound": tools::find("ffmpeg").is_some(),
                "FfprobeFound": tools::find("ffprobe").is_some(),
                "StreamlinkFound": tools::find("streamlink").is_some(),
                "PythonRequired": false,
            }))?
        );
        return Ok(());
    }
    let command = arguments
        .first()
        .map(String::as_str)
        .context("Choose a command; use --help")?;
    if command == "--install-tools" || command == "--update-ytdlp" {
        ensure!(arguments.len() == 1, "Unexpected arguments; use --help");
        let mode = if command == "--update-ytdlp" {
            "Update yt-dlp"
        } else if source_check {
            "Source check"
        } else {
            "Install tools"
        };
        return run_job(Job {
            kind: "tools".into(),
            mode: mode.into(),
            url: mode.into(),
            folder: model::root(),
            ..Job::default()
        });
    }
    let mode = match command {
        "--download-ts" => Some(media::TS),
        "--download-live-ts-auto" => Some(media::AUTO_LIVE),
        "--download-live-ts-nvenc" => Some(media::NVENC_LIVE),
        "--record-live-streamlink" => Some(media::STREAMLINK),
        _ => None,
    };
    if let Some(mode) = mode {
        ensure!(arguments.len() == 3, "Use {command} URL OUTPUT_DIR");
        return run_job(Job {
            url: arguments[1].clone(),
            mode: mode.into(),
            folder: absolute(&arguments[2])?,
            cookies_file: cookies,
            ..Job::default()
        });
    }
    match command {
        "--download" => {
            let audio_format = take_option(&mut arguments, "--audio-format")?;
            let audio_bitrate = take_option(&mut arguments, "--audio-bitrate")?
                .map(|value| value.parse::<u16>())
                .transpose()
                .context("Use 320, 192 or 128 for --audio-bitrate")?;
            let start = take_option(&mut arguments, "--start")?;
            let end = take_option(&mut arguments, "--end")?;
            let language = take_option(&mut arguments, "--subtitle-lang")?;
            let playlist = take_option(&mut arguments, "--playlist-items")?;
            ensure!(
                arguments.len() == 4,
                "Use --download MODE_INDEX URL OUTPUT_DIR"
            );
            let index: usize = arguments[1].parse()?;
            let mode = media::MODES
                .get(index)
                .context("Download mode must be 0-12")?;
            ensure!(
                media::audio_mode(mode) || (audio_format.is_none() && audio_bitrate.is_none()),
                "Audio options require download mode 3 (Audio) or 6 (Playlist - Audio)"
            );
            let mut job = Job {
                url: arguments[2].clone(),
                mode: (*mode).into(),
                folder: absolute(&arguments[3])?,
                cookies_file: cookies,
                ..Job::default()
            };
            if let Some(format) = audio_format {
                job.audio_format = format.to_ascii_uppercase();
            }
            job.audio_bitrate = audio_bitrate;
            if start.is_some() || end.is_some() || index == media::TIME_RANGE_MODE {
                ensure!(
                    index == media::TIME_RANGE_MODE,
                    "Use --start and --end with download mode 11"
                );
                job.download_range = Some([
                    model::parse_time(start.as_deref().context("Pass --start TIME")?)?,
                    model::parse_time(end.as_deref().context("Pass --end TIME")?)?,
                ]);
            }
            if let Some(language) = language {
                ensure!(
                    index == media::SUBTITLES_MODE,
                    "Use --subtitle-lang with download mode 12"
                );
                job.subtitle_language = subtitle_language(&language)?.into();
            }
            if let Some(items) = playlist {
                job.playlist_items = playlist_indices(&items)?;
            }
            run_job(job)
        }
        "--convert" => {
            let limit = take_option(&mut arguments, "--max-resolution")?
                .map(|value| value.parse::<u32>())
                .transpose()?;
            let level = take_option(&mut arguments, "--compression")?
                .map(|value| compression_level(&value))
                .transpose()?;
            let format = take_option(&mut arguments, "--audio-format")?
                .map(|value| value.to_ascii_uppercase());
            let bitrate = take_option(&mut arguments, "--audio-bitrate")?
                .map(|value| value.parse::<u16>())
                .transpose()?;
            ensure!(arguments.len() == 3, "Use --convert MODE_INDEX INPUT");
            let source = absolute(&arguments[2])?;
            let conversion = match arguments[1].parse::<usize>()? {
                media::COMPRESS_MODE if format.is_none() && bitrate.is_none() => {
                    media::Conversion::Compress {
                        limit,
                        level: level.unwrap_or(0),
                    }
                }
                media::EXTRACT_AUDIO_MODE if limit.is_none() && level.is_none() => {
                    media::Conversion::Audio {
                        format: format.as_deref().unwrap_or("MP3"),
                        bitrate,
                    }
                }
                mode if limit.is_none()
                    && level.is_none()
                    && format.is_none()
                    && bitrate.is_none() =>
                {
                    media::Conversion::Video(mode)
                }
                _ => bail!(
                    "Use --max-resolution or --compression with mode 4, or audio options with mode 5"
                ),
            };
            let (output, arguments) = media::convert(&source, conversion)?;
            run_edit(
                "Converting",
                output,
                arguments,
                Default::default(),
                vec![source],
            )
        }
        "--cut" => {
            ensure!(
                arguments.len() == 5,
                "Use --cut INPUT OUTPUT START END [--accurate]"
            );
            let input = absolute(&arguments[1])?;
            let output = absolute(&arguments[2])?;
            let args = media::cut(
                &input,
                &output,
                model::parse_time(&arguments[3])?,
                model::parse_time(&arguments[4])?,
                accurate,
            )?;
            run_edit(
                "Cutting clip",
                output,
                args,
                Default::default(),
                vec![input],
            )
        }
        "--join" => {
            ensure!(
                arguments.len() >= 4,
                "Use --join OUTPUT INPUT INPUT... [--accurate]"
            );
            let output = absolute(&arguments[1])?;
            let inputs = arguments[2..]
                .iter()
                .map(|s| absolute(s))
                .collect::<Result<Vec<_>>>()?;
            let list = output
                .parent()
                .context("Choose an output directory")?
                .join("pixclip-concat.txt");
            let (arguments, content) = media::join(&inputs, &output, &list, accurate)?;
            run_edit(
                "Joining clips",
                output,
                arguments,
                [(media::text(&list), content)].into(),
                inputs,
            )
        }
        _ => bail!("Unknown command {command}; use --help"),
    }
}

fn absolute(path: &str) -> Result<PathBuf> {
    let path = if path == "~" {
        model::home()
    } else if let Some(relative) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        model::home().join(relative)
    } else {
        PathBuf::from(path)
    };
    Ok(if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    })
}
fn take_flag(arguments: &mut Vec<String>, name: &str) -> bool {
    if let Some(index) = arguments.iter().position(|s| s == name) {
        arguments.remove(index);
        true
    } else {
        false
    }
}
fn take_option(arguments: &mut Vec<String>, name: &str) -> Result<Option<String>> {
    if let Some(index) = arguments.iter().position(|s| s == name) {
        ensure!(index + 1 < arguments.len(), "{name} requires a value");
        arguments.remove(index);
        Ok(Some(arguments.remove(index)))
    } else {
        Ok(None)
    }
}

fn run_edit(
    label: &str,
    output: PathBuf,
    arguments: Vec<String>,
    temporary: std::collections::BTreeMap<String, String>,
    inputs: Vec<PathBuf>,
) -> Result<()> {
    ensure!(
        !output.exists(),
        "Output already exists; choose another filename: {}",
        output.display()
    );
    run_job(Job {
        kind: "local".into(),
        mode: label.into(),
        url: label.into(),
        folder: output.parent().context("Missing output directory")?.into(),
        executable: Some(tools::require("ffmpeg")?),
        arguments,
        inputs,
        temporary,
        ..Job::default()
    })
}

fn run_job(job: Job) -> Result<()> {
    static CANCEL: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    let cancel = CANCEL
        .get_or_init(|| {
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let _ = ctrlc::set_handler(move || {
                flag.store(true, Ordering::Relaxed);
            });
            cancel
        })
        .clone();
    cancel.store(false, Ordering::Relaxed);
    let session = tempfile::tempdir()?;
    let mut engine = jobs::Engine::new(session.path())?;
    let id = engine.add(job)?;
    loop {
        if cancel.load(Ordering::Relaxed) {
            engine.begin_close();
        }
        engine.tick();
        while let Some(line) = engine.log.pop_front() {
            println!("{line}");
        }
        if engine.job(&id).is_some_and(|j| j.state.terminal()) {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    engine.save();
    let job = engine.job(&id).unwrap();
    ensure!(
        job.state != State::Cancelled,
        "Cancelled; partial files kept"
    );
    ensure!(job.state == State::Done, "{}", job.detail);
    for output in &job.outputs {
        println!("Created: {}", output.display());
    }
    Ok(())
}

fn input(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut text = String::new();
    ensure!(io::stdin().read_line(&mut text)? > 0, "Input closed");
    Ok(text.trim().trim_matches('"').into())
}

fn subtitle_language(code: &str) -> Result<&'static str> {
    let index = match code {
        "en" => 0,
        "th" => 1,
        "all" => 2,
        _ => bail!("Use en, th or all for subtitle language"),
    };
    Ok(media::SUBTITLE_LANGUAGES[index])
}

fn playlist_indices(items: &str) -> Result<Vec<usize>> {
    items
        .split(',')
        .map(|item| item.trim().parse::<usize>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Enter item numbers such as 1,3,5")
}

fn compression_level(level: &str) -> Result<usize> {
    media::COMPRESS_LEVELS
        .iter()
        .position(|name| name.eq_ignore_ascii_case(level))
        .context("Use balanced, smaller or smallest")
}

fn prompt_audio() -> Result<(String, Option<u16>)> {
    let value = input("Audio format [MP3/M4A/FLAC/WAV, Enter = MP3]: ")?;
    let format = if value.is_empty() {
        "MP3".into()
    } else {
        value.to_ascii_uppercase()
    };
    ensure!(
        model::AUDIO_FORMATS.contains(&format.as_str()),
        "Choose MP3, M4A, FLAC or WAV"
    );
    let bitrate = if media::audio_encoder(&format).is_some() {
        let value = input("Bitrate in kbps [320/192/128, Enter = best]: ")?;
        (!value.is_empty())
            .then(|| value.parse::<u16>())
            .transpose()?
    } else {
        None
    };
    Ok((format, bitrate))
}

const CLI_MENU: &str = "
[ DOWNLOAD VIDEO ]
  [1] MP4 - Best quality
  [2] MP4 - 4K / 2160p High Quality
  [3] MP4 - H.264/AAC compatible
  [4] Audio - choose format
  [5] Video/Live - MPEG-TS (.ts)
 [21] Download - Time range
 [22] Subtitles only
[ LIVE / GPU ]
 [18] Live - MPEG-TS (.ts) - NVIDIA NVENC
 [19] Live - Streamlink - Original quality (.ts)
 [20] Live - MPEG-TS (.ts) - Auto GPU/CPU
[ PLAYLIST ]
  [6] Playlist MP4 - Best quality
  [7] Playlist MP4 - H.264/AAC compatible
  [8] Playlist - Audio
 [23] Playlist - Select item numbers
[ CONVERT WITH FFMPEG ]
  [9] TS to MP4 - Fast / Original quality
 [10] TS to MP4 - H.264/AAC compatible
 [11] TS to MP4 - H.265/HEVC - RTX 2060 NVENC HQ
 [12] H.265/HEVC to H.264/AAC
 [24] Compress MP4
 [25] Extract Audio
[ TOOLS ]
 [13] Change download folder
 [14] Open download folder
 [15] Update yt-dlp
 [16] Join video clips with FFmpeg
 [17] Install / repair yt-dlp, FFmpeg, and Streamlink
 [26] Cut clip - Start and End
  [0] Exit";

fn cli_download_mode(choice: &str) -> Option<&'static str> {
    let index = match choice {
        "1" => 0,
        "2" => 1,
        "3" => 2,
        "4" => 3,
        "5" => 7,
        "6" => 4,
        "7" => 5,
        "8" => 6,
        "18" => 9,
        "19" => 10,
        "20" => 8,
        "21" => media::TIME_RANGE_MODE,
        "22" => media::SUBTITLES_MODE,
        "23" => 4,
        _ => return None,
    };
    Some(media::MODES[index])
}

fn parse_cli_paths(text: &str) -> Result<Vec<PathBuf>> {
    let mut paths = vec![];
    let mut current = String::new();
    let mut quote = None;
    for character in text.chars() {
        if Some(character) == quote {
            quote = None;
        } else if quote.is_none() && (character == '"' || character == '\'') {
            quote = Some(character);
        } else if quote.is_none() && character.is_whitespace() {
            if !current.is_empty() {
                paths.push(absolute(&current)?);
                current.clear();
            }
        } else {
            current.push(character);
        }
    }
    ensure!(quote.is_none(), "Close the quote around each file path");
    if !current.is_empty() {
        paths.push(absolute(&current)?);
    }
    Ok(paths)
}

fn menu(cookies: Option<PathBuf>) -> Result<()> {
    let mut settings = Settings::load(&model::state_dir())?;
    let hardware = tools::hardware();
    loop {
        println!("\n{}\nPIXCLIP TUI", "=".repeat(64));
        println!(
            "yt-dlp : {}",
            tools::find("yt-dlp")
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "Missing".into())
        );
        println!(
            "Streamlink: {}",
            tools::find("streamlink")
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "Missing".into())
        );
        println!(
            "GPU    : {}\nCPU    : {}\nOutput : {}\n{}",
            hardware.gpu,
            hardware.cpu,
            settings.output_directory.display(),
            "=".repeat(64)
        );
        println!("{CLI_MENU}");
        let choice = input("Select: ")?;
        if choice == "0" {
            return Ok(());
        }
        let result = (|| -> Result<()> {
            if let Some(mode) = cli_download_mode(&choice) {
                let mut job = Job {
                    url: input(if media::live(mode) {
                        "Paste Live URL: "
                    } else {
                        "Paste video or playlist URL: "
                    })?,
                    mode: mode.into(),
                    folder: settings.output_directory.clone(),
                    cookies_file: cookies.clone().or_else(|| settings.cookies_file.clone()),
                    ..Job::default()
                };
                match choice.as_str() {
                    "21" => {
                        job.download_range = Some([
                            model::parse_time(&input("Start (seconds or HH:MM:SS): ")?)?,
                            model::parse_time(&input("End (seconds or HH:MM:SS): ")?)?,
                        ]);
                    }
                    "22" => {
                        let language = input("Subtitle language [en/th/all, Enter = en]: ")?;
                        if !language.is_empty() {
                            job.subtitle_language = subtitle_language(&language)?.into();
                        }
                    }
                    "23" => {
                        job.playlist_items = playlist_indices(&input("Items (e.g. 1,3,5): ")?)?;
                        job.mode = match input(
                            "Output [1 MP4 / 2 Compatible MP4 / 3 Audio, Enter = 1]: ",
                        )?
                        .as_str()
                        {
                            "" | "1" => media::MODES[4],
                            "2" => media::MODES[5],
                            "3" => media::MODES[6],
                            _ => bail!("Choose MP4, Compatible MP4 or Audio"),
                        }
                        .into();
                    }
                    _ => {}
                }
                if media::audio_mode(&job.mode) {
                    (job.audio_format, job.audio_bitrate) = prompt_audio()?;
                }
                return run_job(job);
            }
            match choice.as_str() {
                "9" | "10" | "11" | "12" => {
                    let mode = choice.parse::<usize>()? - 9;
                    let source = absolute(&input("Video file: ")?)?;
                    let (output, args) = media::convert(&source, media::Conversion::Video(mode))?;
                    run_edit("Converting", output, args, Default::default(), vec![source])
                }
                "24" | "25" => {
                    let source = absolute(&input("Video file: ")?)?;
                    let (output, args) = if choice == "24" {
                        let limit = input("Maximum resolution [1080/720, Enter = keep]: ")?;
                        let level =
                            input("Compression [Balanced/Smaller/Smallest, Enter = Balanced]: ")?;
                        media::convert(
                            &source,
                            media::Conversion::Compress {
                                limit: (!limit.is_empty())
                                    .then(|| limit.parse::<u32>())
                                    .transpose()?,
                                level: if level.is_empty() {
                                    0
                                } else {
                                    compression_level(&level)?
                                },
                            },
                        )?
                    } else {
                        let (format, bitrate) = prompt_audio()?;
                        media::convert(
                            &source,
                            media::Conversion::Audio {
                                format: &format,
                                bitrate,
                            },
                        )?
                    };
                    run_edit("Converting", output, args, Default::default(), vec![source])
                }
                "26" => {
                    let source = absolute(&input("Video file: ")?)?;
                    let start = model::parse_time(&input("Start (seconds or HH:MM:SS): ")?)?;
                    let end = model::parse_time(&input("End (seconds or HH:MM:SS): ")?)?;
                    let name = input("Output MP4 path: ")?;
                    ensure!(!name.is_empty(), "Choose an output MP4 path");
                    let output = absolute(&name)?;
                    let accurate = input("Mode [1 fast / 2 accurate, Enter = 2]: ")? != "1";
                    let args = media::cut(&source, &output, start, end, accurate)?;
                    run_edit(
                        "Cutting clip",
                        output,
                        args,
                        Default::default(),
                        vec![source],
                    )
                }
                "13" => {
                    let path = input("New folder path (leave blank to cancel): ")?;
                    if !path.is_empty() {
                        let path = absolute(&path)?;
                        std::fs::create_dir_all(&path)?;
                        settings.output_directory = path;
                        settings.save(&model::state_dir())?;
                    }
                    Ok(())
                }
                "14" => process::open(&settings.output_directory),
                "15" | "17" => {
                    if choice == "17"
                        && !input("Continue with download and verification? [Y/N]: ")?
                            .eq_ignore_ascii_case("y")
                    {
                        return Ok(());
                    }
                    let label = if choice == "15" {
                        "Update yt-dlp"
                    } else {
                        "Install tools"
                    };
                    run_job(Job {
                        kind: "tools".into(),
                        mode: label.into(),
                        url: label.into(),
                        folder: model::root(),
                        ..Job::default()
                    })
                }
                "16" => {
                    print!("Clips (quote paths that contain spaces): ");
                    io::stdout().flush()?;
                    let mut line = String::new();
                    ensure!(io::stdin().read_line(&mut line)? > 0, "Input closed");
                    let files = parse_cli_paths(line.trim())?;
                    ensure!(
                        files.len() >= 2 && files.iter().all(|p| p.is_file()),
                        "Please provide at least two valid files."
                    );
                    let accurate = input("Mode [1 fast / 2 compatible]: ")? == "2";
                    let default = settings
                        .output_directory
                        .join(format!("joined_{}.mp4", model::now() as i64));
                    let value = input(&format!("Output MP4 (Enter for {}): ", default.display()))?;
                    let output = if value.is_empty() {
                        default
                    } else {
                        absolute(&value)?
                    };
                    let list = output.with_extension("concat.txt");
                    let (args, content) = media::join(&files, &output, &list, accurate)?;
                    run_edit(
                        "Joining clips",
                        output,
                        args,
                        [(media::text(&list), content)].into(),
                        files,
                    )
                }
                _ => {
                    println!("Invalid choice.");
                    Ok(())
                }
            }
        })();
        if let Err(error) = result {
            eprintln!("ERROR: {error:#}");
        }
        input("Press Enter to continue...")?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_cli_choices_and_quoted_paths_are_preserved() {
        let modes: Vec<_> = (1..=20)
            .filter_map(|n| cli_download_mode(&n.to_string()))
            .collect();
        let original_modes = &media::MODES[..media::TIME_RANGE_MODE];
        assert_eq!(modes.len(), original_modes.len());
        for mode in original_modes {
            assert!(modes.contains(mode));
        }
        assert_eq!(cli_download_mode("5"), Some(media::TS));
        assert_eq!(cli_download_mode("18"), Some(media::NVENC_LIVE));
        assert_eq!(cli_download_mode("19"), Some(media::STREAMLINK));
        assert_eq!(cli_download_mode("20"), Some(media::AUTO_LIVE));
        let paths = parse_cli_paths("\"first clip.mp4\" \"second clip.mp4\"").unwrap();
        assert_eq!(
            paths,
            vec![
                absolute("first clip.mp4").unwrap(),
                absolute("second clip.mp4").unwrap()
            ]
        );
        assert!(parse_cli_paths("\"unfinished.mp4").is_err());
        for n in 0..=26 {
            assert!(CLI_MENU.contains(&format!("[{n}]")));
        }
        assert_eq!(
            cli_download_mode("21"),
            Some(media::MODES[media::TIME_RANGE_MODE])
        );
        assert_eq!(
            cli_download_mode("22"),
            Some(media::MODES[media::SUBTITLES_MODE])
        );
        assert_eq!(cli_download_mode("23"), Some(media::MODES[4]));
    }
    #[test]
    fn relative_cookie_and_home_paths_are_resolved_before_changing_directory() {
        let mut arguments = vec!["--cookies-file".into(), "cookies.txt".into()];
        let path = take_option(&mut arguments, "--cookies-file")
            .unwrap()
            .unwrap();
        assert_eq!(
            absolute(&path).unwrap(),
            std::env::current_dir().unwrap().join("cookies.txt")
        );
        assert_eq!(absolute("~/Videos").unwrap(), model::home().join("Videos"));
        assert!(take_option(&mut vec!["--cookies-file".into()], "--cookies-file").is_err());
    }
}

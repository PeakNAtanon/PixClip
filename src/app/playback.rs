use crate::{media, process, tools};
use anyhow::{Context as _, Result, ensure};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use eframe::egui::{self, ColorImage};
use serde_json::Value;
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

const SIZE: [usize; 2] = [640, 360];
const FPS: f64 = 30.0;

#[derive(Default)]
pub(super) struct Updates {
    pub frame: Option<(f64, ColorImage)>,
    pub finished: Option<Result<f64, String>>,
    pub sound: Option<&'static str>,
}

pub(super) struct Playback {
    stop: Arc<AtomicBool>,
    updates: Arc<Mutex<Updates>>,
}

impl Playback {
    pub fn start(source: PathBuf, start: f64, end: f64, ctx: egui::Context) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let updates = Arc::new(Mutex::new(Updates::default()));
        let worker_stop = stop.clone();
        let worker_updates = updates.clone();
        thread::spawn(move || {
            let result = play(&source, start, end, &worker_stop, &worker_updates, &ctx)
                .map_err(|error| error.to_string());
            worker_updates.lock().unwrap().finished = Some(result);
            ctx.request_repaint();
        });
        Self { stop, updates }
    }

    pub fn take_updates(&self) -> Updates {
        std::mem::take(&mut *self.updates.lock().unwrap())
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

type AudioChunk = Vec<f32>;

struct AudioOutput {
    stream: cpal::Stream,
    format: (u32, u16),
    sender: SyncSender<AudioChunk>,
}

impl AudioOutput {
    fn open(stop: &Arc<AtomicBool>) -> Result<Self> {
        let device = cpal::default_host()
            .default_output_device()
            .context("No audio device")?;
        let supported = device.default_output_config()?;
        let config = supported.config();
        let (sender, receiver) = mpsc::sync_channel(32);
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => audio_stream::<f32>(&device, &config, receiver, stop),
            cpal::SampleFormat::I16 => audio_stream::<i16>(&device, &config, receiver, stop),
            cpal::SampleFormat::U16 => audio_stream::<u16>(&device, &config, receiver, stop),
            format => anyhow::bail!("Unsupported audio format: {format}"),
        }?;
        Ok(Self {
            stream,
            format: (config.sample_rate.0, config.channels),
            sender,
        })
    }
}

fn audio_stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    receiver: Receiver<AudioChunk>,
    stop: &Arc<AtomicBool>,
) -> Result<cpal::Stream> {
    let stop = stop.clone();
    let mut samples = Vec::new().into_iter();
    Ok(device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            for sample in output {
                *sample = if stop.load(Ordering::Relaxed) {
                    T::EQUILIBRIUM
                } else {
                    if samples.len() == 0
                        && let Ok(chunk) = receiver.try_recv()
                    {
                        samples = chunk.into_iter();
                    }
                    T::from_sample(samples.next().unwrap_or(0.0))
                };
            }
        },
        |_| {},
        None,
    )?)
}

fn read_audio(
    mut pipe: impl Read,
    sender: SyncSender<AudioChunk>,
    samples: usize,
    ready: &AtomicBool,
) -> io::Result<()> {
    let mut bytes = vec![0; samples * 4];
    loop {
        let mut count = 0;
        while count < bytes.len() {
            let read = pipe.read(&mut bytes[count..])?;
            if read == 0 {
                break;
            }
            count += read;
        }
        if count == 0 {
            return Ok(());
        }
        let chunk = bytes[..count]
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(f32::from_le_bytes)
            .collect();
        if sender.send(chunk).is_err() {
            return Ok(());
        }
        ready.store(true, Ordering::Release);
    }
}

fn play(
    path: &Path,
    start: f64,
    end: f64,
    stop: &Arc<AtomicBool>,
    updates: &Mutex<Updates>,
    ctx: &egui::Context,
) -> Result<f64> {
    let probe = media::probe(path)?;
    let streams = probe["streams"].as_array().map_or(&[][..], Vec::as_slice);
    let size = streams
        .iter()
        .find(|stream| stream["codec_type"] == "video")
        .map_or(SIZE, playback_size);
    let mut audio = None;
    let sound = if streams.iter().any(|stream| stream["codec_type"] == "audio") {
        audio = AudioOutput::open(stop).ok();
        if audio.is_some() {
            "Audio on"
        } else {
            "Audio unavailable"
        }
    } else {
        "No audio track"
    };
    updates.lock().unwrap().sound = Some(sound);
    if stop.load(Ordering::Relaxed) {
        return Ok(start);
    }
    let ffmpeg = tools::require("ffmpeg")?;
    let arguments = playback_args(
        path,
        start,
        end,
        size,
        audio.as_ref().map(|audio| audio.format),
    );
    let (_decoder, mut stdout, stderr) = process::piped(&ffmpeg, &arguments, audio.is_some())?;
    let (sender, frames) = mpsc::sync_channel(6);
    thread::spawn(move || {
        loop {
            let mut bytes = vec![0; size[0] * size[1] * 3];
            if stdout.read_exact(&mut bytes).is_err() || sender.send(bytes).is_err() {
                break;
            }
        }
    });
    let audio_ready = Arc::new(AtomicBool::new(false));
    let audio_stream = if let (Some(audio), Some(stderr)) = (audio, stderr) {
        let ready = audio_ready.clone();
        let samples = (audio.format.0 as usize / 50) * audio.format.1 as usize;
        thread::spawn(move || {
            let _ = read_audio(stderr, audio.sender, samples, &ready);
        });
        Some(audio.stream)
    } else {
        None
    };

    let mut frame_index = 0;
    let mut clock = Instant::now();
    let mut last_frame = clock;
    while !stop.load(Ordering::Relaxed) {
        let bytes = match frames.recv_timeout(Duration::from_millis(20)) {
            Ok(bytes) => bytes,
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                ensure!(
                    last_frame.elapsed() < Duration::from_secs(15),
                    "Preview decoder timed out"
                );
                continue;
            }
        };
        if frame_index == 0 {
            if let Some(stream) = &audio_stream {
                let until = Instant::now() + Duration::from_millis(200);
                while Instant::now() < until
                    && !audio_ready.load(Ordering::Acquire)
                    && !stop.load(Ordering::Relaxed)
                {
                    thread::sleep(Duration::from_millis(5));
                }
                stream.play().context("Could not start preview audio")?;
            }
            clock = Instant::now();
        }
        let elapsed = frame_index as f64 / FPS;
        if !wait_until(clock + Duration::from_secs_f64(elapsed), stop) {
            return Ok(start + elapsed);
        }
        let image = ColorImage::from_rgb(size, &bytes);
        updates.lock().unwrap().frame = Some(((start + elapsed).min(end), image));
        ctx.request_repaint();
        frame_index += 1;
        last_frame = Instant::now();
    }
    if stop.load(Ordering::Relaxed) {
        return Ok(start);
    }
    ensure!(frame_index > 0, "No video frames in the selected range");
    let played = (frame_index as f64 / FPS).min(end - start);
    // Keep the output stream alive until the last decoded frame's interval ends.
    wait_until(clock + Duration::from_secs_f64(played), stop);
    let position = start + played;
    Ok(if end - position <= 1.0 / FPS {
        end
    } else {
        position
    })
}

fn wait_until(deadline: Instant, stop: &AtomicBool) -> bool {
    while !stop.load(Ordering::Relaxed) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return true;
        }
        thread::sleep(remaining.min(Duration::from_millis(8)));
    }
    false
}

fn playback_size(stream: &Value) -> [usize; 2] {
    let width = stream["width"].as_f64().unwrap_or_default();
    let height = stream["height"].as_f64().unwrap_or_default();
    if width <= 0.0 || height <= 0.0 {
        return SIZE;
    }
    // FFmpeg applies the display rotation before the scale filter.
    let quarter_turn = stream["side_data_list"].as_array().is_some_and(|data| {
        data.iter()
            .filter_map(|entry| entry["rotation"].as_f64())
            .any(|rotation| (rotation.abs() % 180.0 - 90.0).abs() < 1.0)
    });
    let (width, height) = if quarter_turn {
        (height, width)
    } else {
        (width, height)
    };
    let [max_width, max_height] = if width < height {
        [SIZE[1], SIZE[0]]
    } else {
        SIZE
    };
    let scale = (max_width as f64 / width).min(max_height as f64 / height);
    [
        (width * scale).round().max(1.0) as usize,
        (height * scale).round().max(1.0) as usize,
    ]
}

fn playback_args(
    path: &Path,
    start: f64,
    end: f64,
    [width, height]: [usize; 2],
    audio: Option<(u32, u16)>,
) -> Vec<String> {
    let mut args = media::args(&["-nostdin", "-v", "quiet"]);
    // TS demuxer seeks can skip the only preceding keyframe in a recording.
    let ts = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ["ts", "m2ts", "mts"]
                .iter()
                .any(|candidate| ext.eq_ignore_ascii_case(candidate))
        });
    let seek = || ["-ss".into(), format!("{start:.3}")];
    if !ts {
        args.extend(seek());
    }
    args.extend(["-i".into(), media::text(path)]);
    let length = format!("{:.3}", end - start);
    if ts {
        args.extend(seek());
    }
    args.extend(media::args(&[
        "-t",
        &length,
        "-map",
        "0:v:0",
        "-an",
        "-vf",
        &format!("scale={width}:{height},setsar=1,fps={FPS}"),
        "-threads",
        "1",
        "-pix_fmt",
        "rgb24",
        "-f",
        "rawvideo",
        "pipe:1",
    ]));
    if let Some((rate, channels)) = audio {
        if ts {
            args.extend(seek());
        }
        args.extend(media::args(&[
            "-t",
            &length,
            "-map",
            "0:a:0",
            "-vn",
            "-ac",
            &channels.to_string(),
            "-ar",
            &rate.to_string(),
            "-f",
            "f32le",
            "pipe:2",
        ]));
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_sizes_fit_the_display_orientation_and_aspect_ratio() {
        for (width, height, rotation, expected) in [
            (1920, 1080, 0, SIZE),
            (1080, 1920, 0, [360, 640]),
            (1920, 1080, -90, [360, 640]),
            (1920, 1080, 90, [360, 640]),
            (1080, 1920, 90, SIZE),
            (1920, 1080, 180, SIZE),
            (1080, 1350, 0, [360, 450]),
        ] {
            let stream = serde_json::json!({
                "width": width, "height": height,
                "side_data_list": [{ "rotation": rotation }]
            });
            assert_eq!(playback_size(&stream), expected);
        }
    }

    #[test]
    #[ignore = "Requires FFmpeg with -display_rotation and ffprobe; synthetic media only"]
    fn portrait_preview_plays_at_full_height_for_native_and_rotated_video() {
        let temp = tempfile::tempdir().unwrap();
        let ffmpeg = tools::require("ffmpeg").unwrap();
        for (name, dimensions) in [("portrait", "180x320"), ("phone", "320x180")] {
            let source = temp.path().join(format!("{name}.mp4"));
            process::capture(
                &ffmpeg,
                &media::args(&[
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("testsrc2=size={dimensions}:rate=30"),
                    "-t",
                    "1",
                    "-c:v",
                    "libx264",
                    &media::text(&source),
                ]),
                Duration::from_secs(15),
            )
            .unwrap();
            let source = if name == "phone" {
                let output = temp.path().join("phone-rotated.mp4");
                process::capture(
                    &ffmpeg,
                    &media::args(&[
                        "-v",
                        "error",
                        "-display_rotation:v:0",
                        "90",
                        "-i",
                        &media::text(&source),
                        "-c",
                        "copy",
                        &media::text(&output),
                    ]),
                    Duration::from_secs(15),
                )
                .unwrap();
                output
            } else {
                source
            };
            let bytes = media::preview(&source, 0.0, 1.0).unwrap();
            let still = image::load_from_memory(&bytes).unwrap();
            assert_eq!((still.width(), still.height()), (270, 480));

            let updates = Mutex::new(Updates::default());
            let end = play(
                &source,
                0.0,
                0.5,
                &Arc::new(AtomicBool::new(false)),
                &updates,
                &egui::Context::default(),
            )
            .unwrap();
            assert_eq!(end, 0.5);
            assert_eq!(
                updates.into_inner().unwrap().frame.unwrap().1.size,
                [360, 640]
            );
        }
    }

    #[test]
    #[ignore = "Requires real FFmpeg; synthetic media only"]
    fn one_decoder_streams_video_and_audio_for_the_selected_ts_range() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("preview.ts");
        let ffmpeg = tools::require("ffmpeg").unwrap();
        process::capture(
            &ffmpeg,
            &media::args(&[
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=30",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "2",
                "-c:v",
                "libx264",
                "-c:a",
                "aac",
                &media::text(&source),
            ]),
            Duration::from_secs(15),
        )
        .unwrap();
        let (_decoder, mut video, audio) = process::piped(
            &ffmpeg,
            &playback_args(&source, 1.0, 1.5, SIZE, Some((48000, 2))),
            true,
        )
        .unwrap();
        let audio = thread::spawn(move || {
            let mut bytes = Vec::new();
            audio.unwrap().read_to_end(&mut bytes).unwrap();
            bytes
        });
        let mut pixels = Vec::new();
        video.read_to_end(&mut pixels).unwrap();
        let pcm = audio.join().unwrap();
        assert_eq!(pixels.len(), SIZE[0] * SIZE[1] * 3 * 15);
        let samples = pcm.as_chunks::<4>().0;
        assert!((47_000..=49_000).contains(&samples.len()));
        assert!(samples.iter().any(|&v| f32::from_le_bytes(v).abs() > 0.01));
    }
}

use crate::{media::args, process};
use std::{
    fs,
    ops::Range,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Encoder {
    pub name: &'static str,
    pub device: Option<PathBuf>,
}

pub const CPU: Encoder = Encoder::new("libx264");
pub const NVENC: Encoder = Encoder::new("h264_nvenc");
static FILE: OnceLock<Encoder> = OnceLock::new();
static LIVE: OnceLock<Encoder> = OnceLock::new();
static PROBE: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl Encoder {
    pub const fn new(name: &'static str) -> Self {
        Self { name, device: None }
    }

    pub fn video(&self, preset: &str, quality: &str) -> Vec<String> {
        let mut options = args(&["-c:v", self.name]);
        let extra = match self.name {
            "h264_nvenc" => {
                let tune = if preset == "veryfast" { "ll" } else { "hq" };
                args(&[
                    "-preset", "p5", "-tune", tune, "-rc", "vbr", "-cq", quality, "-b:v", "0",
                ])
            }
            "h264_amf" => args(&[
                "-rc", "cqp", "-qp_i", quality, "-qp_p", quality, "-qp_b", quality,
            ]),
            "h264_qsv" => args(&["-preset", preset, "-global_quality", quality]),
            "h264_vaapi" => args(&["-qp", quality]),
            "h264_videotoolbox" => {
                let bitrate = match quality {
                    "27" => "5000k",
                    "31" => "3200k",
                    "35" => "2000k",
                    "23" => "8000k",
                    _ => "12000k",
                };
                args(&["-allow_sw", "0", "-profile:v", "high", "-b:v", bitrate])
            }
            _ => args(&["-preset", preset, "-crf", quality]),
        };
        options.extend(extra);
        match self.name {
            "h264_vaapi" => {}
            "h264_qsv" | "h264_videotoolbox" => options.extend(args(&["-pix_fmt", "nv12"])),
            _ => options.extend(args(&["-pix_fmt", "yuv420p"])),
        }
        options
    }

    pub fn configure(
        &self,
        options: &mut Vec<String>,
        slot: Range<usize>,
        preset: &str,
        quality: &str,
    ) {
        options.splice(slot, self.video(preset, quality));
        if let Some(device) = &self.device {
            options.splice(0..0, ["-vaapi_device".into(), crate::media::text(device)]);
            if let Some(filter) = options.iter().position(|s| s == "-vf") {
                options[filter + 1].push_str(",format=nv12,hwupload");
            } else {
                let video = options.iter().position(|s| s == "-c:v").unwrap();
                options.splice(video..video, args(&["-vf", "format=nv12,hwupload"]));
            }
        }
    }
}

pub fn auto(ffmpeg: &Path, live: bool) -> &'static Encoder {
    let cache = if live { &LIVE } else { &FILE };
    cache.get_or_init(|| {
        // Probing both profiles concurrently would skew the timings.
        let _probe = PROBE.lock().unwrap();
        benchmark(ffmpeg, live)
    })
}

fn benchmark(ffmpeg: &Path, live: bool) -> Encoder {
    let Ok(list) = process::capture(
        ffmpeg,
        &args(&["-hide_banner", "-encoders"]),
        Duration::from_secs(3),
    ) else {
        return CPU;
    };
    let available = String::from_utf8_lossy(&list);
    let supports = |name| {
        available
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some(name))
    };
    let mut candidates = vec![CPU];
    let gpu_names: &[&str] = if cfg!(target_os = "macos") {
        &["h264_videotoolbox"]
    } else if cfg!(windows) {
        &["h264_nvenc", "h264_amf", "h264_qsv"]
    } else {
        &["h264_nvenc", "h264_qsv", "h264_amf"]
    };
    candidates.extend(gpu_names.iter().map(|&name| Encoder::new(name)));
    if cfg!(target_os = "linux") && supports("h264_vaapi") {
        let mut devices: Vec<_> = fs::read_dir("/dev/dri")
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| name.strip_prefix("renderD"))
                    .is_some_and(|index| index.parse::<u32>().is_ok())
            })
            .collect();
        devices.sort();
        candidates.extend(devices.into_iter().map(|device| Encoder {
            name: "h264_vaapi",
            device: Some(device),
        }));
    }
    fastest(candidates, |encoder| {
        if !supports(encoder.name) {
            return None;
        }
        let mut command = args(&[
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-xerror",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1920x1080:rate=30",
            "-frames:v",
            "90",
            "-an",
        ]);
        let end = command.len();
        let (preset, quality) = if live {
            ("veryfast", "23")
        } else {
            ("medium", "20")
        };
        encoder.configure(&mut command, end..end, preset, quality);
        command.extend(args(&["-f", "null", "-"]));
        let start = Instant::now();
        process::capture(ffmpeg, &command, Duration::from_secs(5)).ok()?;
        Some(start.elapsed())
    })
}

fn fastest(
    candidates: Vec<Encoder>,
    mut measure: impl FnMut(&Encoder) -> Option<Duration>,
) -> Encoder {
    candidates
        .into_iter()
        .filter_map(|encoder| measure(&encoder).map(|elapsed| (elapsed, encoder)))
        .min_by_key(|(elapsed, _)| *elapsed)
        .map_or(CPU, |(_, encoder)| encoder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooses_measured_speed_and_ignores_failed_encoders() {
        let candidates = vec![CPU, NVENC, Encoder::new("h264_videotoolbox")];
        for (cpu_ms, gpu_ms, expected) in [(100, 200, CPU.name), (200, 100, NVENC.name)] {
            let selected = fastest(candidates.clone(), |encoder| match encoder.name {
                "libx264" => Some(Duration::from_millis(cpu_ms)),
                "h264_nvenc" => Some(Duration::from_millis(gpu_ms)),
                _ => None,
            });
            assert_eq!(selected.name, expected);
        }
        assert_eq!(fastest(candidates, |_| None).name, CPU.name);
    }

    #[test]
    fn vaapi_uploads_after_the_existing_resize() {
        let encoder = Encoder {
            name: "h264_vaapi",
            device: Some("/dev/dri/renderD129".into()),
        };
        let mut options = args(&["-i", "in.mp4", "-vf", "scale=1280:720", "out.mp4"]);
        encoder.configure(&mut options, 4..4, "medium", "31");
        assert_eq!(
            &options[..4],
            &["-vaapi_device", "/dev/dri/renderD129", "-i", "in.mp4"]
        );
        assert!(
            options
                .windows(2)
                .any(|pair| pair == ["-vf", "scale=1280:720,format=nv12,hwupload"])
        );
        assert!(options.windows(2).any(|pair| pair == ["-qp", "31"]));
    }
}

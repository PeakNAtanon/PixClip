use crate::{
    encoder, model,
    process::{self, Context},
};
use anyhow::{Context as _, Result, bail, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";

pub fn find(name: &str) -> Option<PathBuf> {
    let names = if cfg!(windows) {
        vec![format!("{name}.exe"), name.into()]
    } else {
        vec![name.into()]
    };
    let root = model::root();
    let mut directories = vec![
        root.join("tools"),
        root.join("tools/streamlink/bin"),
        root.clone(),
        root.join(if cfg!(windows) {
            ".venv/Scripts"
        } else {
            ".venv/bin"
        }),
    ];
    if name == "streamlink" {
        if let Ok(entries) = fs::read_dir(root.join("tools/streamlink")) {
            let mut versions: Vec<_> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.join(".pixclip-ready").is_file())
                .collect();
            versions.sort_by(|a, b| b.cmp(a));
            directories.splice(2..2, versions.into_iter().map(|p| p.join("bin")));
        }
        if cfg!(target_os = "linux") && root.join("tools/streamlink.AppImage").is_file() {
            return Some(root.join("tools/streamlink.AppImage"));
        }
    }
    if let Some(path) = env::var_os("PATH") {
        directories.extend(env::split_paths(&path));
    }
    directories
        .into_iter()
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .find(|path| path.is_file())
}

pub fn require(name: &str) -> Result<PathBuf> {
    find(name)
        .with_context(|| format!("{name} was not found. Use Install tools or add it to PATH."))
}

pub fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()?)
}

pub fn checked_bytes(bytes: &[u8], expected: &str) -> Result<()> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    ensure!(
        actual.eq_ignore_ascii_case(expected.trim()),
        "Checksum verification failed; downloaded file was not installed"
    );
    Ok(())
}

fn checksum(text: &str, filename: &str) -> Result<String> {
    for line in text.lines() {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() >= 2 && parts[1].trim_start_matches('*') == filename && parts[0].len() == 64
        {
            return Ok(parts[0].into());
        }
    }
    let first = text.split_whitespace().next().unwrap_or("");
    if !text.contains('\n') && first.len() == 64 {
        return Ok(first.into());
    }
    bail!("Official checksum was missing for {filename}")
}

fn download(url: &str, ctx: &Context) -> Result<Vec<u8>> {
    request_bytes(url, None, ctx)
}

pub fn request_bytes(url: &str, body: Option<Value>, ctx: &Context) -> Result<Vec<u8>> {
    ctx.check()?;
    let url = url.to_string();
    let background = ctx.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| -> Result<Vec<u8>> {
            let client = client()?;
            let request = if let Some(body) = body {
                client
                    .post(&url)
                    .header("Origin", "https://kick.com")
                    .header("Referer", "https://kick.com/")
                    .json(&body)
            } else {
                client.get(&url)
            };
            let mut response = request.send()?.error_for_status()?;
            let mut bytes = Vec::new();
            let mut buffer = [0; 64 * 1024];
            loop {
                background.check()?;
                let count = response.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..count]);
            }
            Ok(bytes)
        })();
        let _ = sender.send(result);
    });
    loop {
        ctx.check()?;
        ensure!(
            !ctx.deadline.is_some_and(|end| model::now() >= end),
            "Scheduled window ended during network request"
        );
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => return result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

fn install_file(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().context("Missing tools folder")?)?;
    let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))?;
    }
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn extract_tool(bytes: &[u8], filename: &str, target: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        let Some(path) = file.enclosed_name() else {
            bail!("Archive contains an unsafe path");
        };
        if path.file_name().is_some_and(|name| name == filename) {
            let mut content = Vec::new();
            file.read_to_end(&mut content)?;
            install_file(target, &content)?;
            return Ok(());
        }
    }
    bail!("Official archive did not contain {filename}")
}

pub fn install(ctx: &Context, source_check: bool) -> Result<()> {
    let folder = model::root().join("tools");
    let asset = if cfg!(windows) {
        "yt-dlp.exe"
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else if cfg!(target_arch = "aarch64") {
        "yt-dlp_linux_aarch64"
    } else {
        "yt-dlp_linux"
    };
    let sums = download(
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS",
        ctx,
    )?;
    let hash = checksum(std::str::from_utf8(&sums)?, asset)?;
    let ffmpeg_sum = if cfg!(windows) {
        Some(download(
            "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip.sha256",
            ctx,
        )?)
    } else {
        None
    };
    if source_check {
        if cfg!(target_os = "macos") {
            verify(&require("streamlink")?, "--version")?;
            ctx.emit("Official yt-dlp checksum source is reachable. Streamlink is ready.");
        } else {
            streamlink_asset(ctx)?;
            ctx.emit("Official checksum sources are reachable, including Streamlink.");
        }
        return Ok(());
    }
    ctx.emit("PIXCLIP_PHASE=Installing verified yt-dlp");
    let bytes = download(
        &format!("https://github.com/yt-dlp/yt-dlp/releases/latest/download/{asset}"),
        ctx,
    )?;
    checked_bytes(&bytes, &hash)?;
    let yt = folder.join(if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    });
    install_file(&yt, &bytes)?;
    verify(&yt, "--version")?;
    if let Some(sum) = ffmpeg_sum {
        ctx.emit("PIXCLIP_PHASE=Installing verified FFmpeg / ffprobe");
        let bytes = download(
            "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip",
            ctx,
        )?;
        let hash = std::str::from_utf8(&sum)?
            .split_whitespace()
            .next()
            .context("Missing FFmpeg checksum")?;
        checked_bytes(&bytes, hash)?;
        for name in ["ffmpeg.exe", "ffprobe.exe"] {
            let path = folder.join(name);
            extract_tool(&bytes, name, &path)?;
            verify(&path, "-version")?;
        }
    } else {
        for name in ["ffmpeg", "ffprobe"] {
            verify(&require(name)?, "-version")?;
        }
    }
    if cfg!(target_os = "macos") {
        verify(&require("streamlink")?, "--version")?;
    } else {
        install_streamlink(ctx)?;
    }
    ctx.emit("Tools are ready.");
    Ok(())
}

fn streamlink_asset(ctx: &Context) -> Result<(String, String)> {
    let repository = if cfg!(windows) {
        "windows-builds"
    } else {
        "streamlink-appimage"
    };
    let bytes = download(
        &format!("https://api.github.com/repos/streamlink/{repository}/releases/latest"),
        ctx,
    )?;
    let release: Value = serde_json::from_slice(&bytes)?;
    select_streamlink_asset(&release, cfg!(windows), std::env::consts::ARCH)
}

fn select_streamlink_asset(release: &Value, windows: bool, arch: &str) -> Result<(String, String)> {
    ensure!(
        arch == "x86_64" || (!windows && arch == "aarch64"),
        "Streamlink portable builds do not support this CPU architecture"
    );
    let asset = release["assets"]
        .as_array()
        .context("Official Streamlink release did not list assets")?
        .iter()
        .find(|asset| {
            let name = asset["name"].as_str().unwrap_or("");
            if windows {
                name.starts_with("streamlink-") && name.ends_with("-x86_64.zip")
            } else {
                name.starts_with("streamlink-") && name.ends_with(&format!("_{arch}.AppImage"))
            }
        })
        .context("Official Streamlink portable build was not found")?;
    let url = asset["browser_download_url"]
        .as_str()
        .context("Missing Streamlink download URL")?;
    let prefix = if windows {
        "https://github.com/streamlink/windows-builds/releases/download/"
    } else {
        "https://github.com/streamlink/streamlink-appimage/releases/download/"
    };
    ensure!(
        url.starts_with(prefix),
        "Unexpected Streamlink download source"
    );
    let digest = asset["digest"]
        .as_str()
        .and_then(|s| s.strip_prefix("sha256:"))
        .context("Official Streamlink SHA-256 digest is missing")?;
    ensure!(
        digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid Streamlink digest"
    );
    Ok((url.into(), digest.into()))
}

fn unpack_streamlink(bytes: &[u8], folder: &Path, ctx: &Context) -> Result<PathBuf> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    let mut total = 0_u64;
    let mut executable = None;
    for index in 0..archive.len() {
        ctx.check()?;
        let mut file = archive.by_index(index)?;
        let relative = file
            .enclosed_name()
            .context("Streamlink archive contains an unsafe path")?;
        ensure!(
            !file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000),
            "Streamlink archive contains a symlink"
        );
        total = total.saturating_add(file.size());
        ensure!(
            total <= 1024 * 1024 * 1024,
            "Streamlink archive is unexpectedly large"
        );
        let destination = folder.join(&relative);
        if relative.ends_with("bin/streamlink.exe") {
            ensure!(
                executable.replace(destination.clone()).is_none(),
                "Streamlink archive contains multiple launchers"
            );
        }
        if file.is_dir() {
            fs::create_dir_all(&destination)?;
        } else {
            fs::create_dir_all(destination.parent().unwrap())?;
            let mut output = fs::File::create(&destination)?;
            std::io::copy(&mut file, &mut output)?;
        }
    }
    executable.context("Official archive did not contain bin/streamlink.exe")
}

fn install_streamlink(ctx: &Context) -> Result<()> {
    ctx.emit("PIXCLIP_PHASE=Installing verified portable Streamlink");
    let (url, digest) = streamlink_asset(ctx)?;
    let bytes = download(&url, ctx)?;
    checked_bytes(&bytes, &digest)?;
    let folder = model::root().join("tools");
    if cfg!(windows) {
        let versions = folder.join("streamlink");
        fs::create_dir_all(&versions)?;
        let stage = tempfile::Builder::new()
            .prefix(".install-")
            .tempdir_in(&versions)?;
        let executable = unpack_streamlink(&bytes, stage.path(), ctx)?;
        verify(&executable, "--version")?;
        let portable_root = executable
            .parent()
            .and_then(Path::parent)
            .context("Invalid Streamlink archive layout")?;
        let destination = versions.join(format!(
            "release-{}-{}",
            chrono::Local::now().format("%Y%m%d%H%M%S"),
            uuid::Uuid::new_v4().simple()
        ));
        fs::rename(portable_root, &destination)?;
        verify(&destination.join("bin/streamlink.exe"), "--version")?;
        fs::write(
            destination.join(".pixclip-ready"),
            b"verified portable Streamlink",
        )?;
    } else {
        let target = folder.join("streamlink.AppImage");
        install_file(&target, &bytes)?;
        verify(&target, "--version")?;
    }
    ctx.emit("Streamlink is ready.");
    Ok(())
}

pub fn verify(path: &Path, argument: &str) -> Result<()> {
    process::capture(path, &[argument.into()], Duration::from_secs(60)).map(|_| ())
}

#[derive(Clone, Default)]
pub struct Hardware {
    pub gpu: String,
    pub cpu: String,
    pub encoder: String,
    pub file_encoder: String,
}

pub fn hardware() -> Hardware {
    let query = |executable: &str, arguments: &[&str]| -> String {
        process::capture(
            Path::new(executable),
            &crate::media::args(arguments),
            Duration::from_secs(5),
        )
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
        .unwrap_or_default()
    };
    let gpu = if cfg!(windows) {
        query(
            "powershell.exe",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-CimInstance Win32_VideoController -ErrorAction Stop | Select-Object -ExpandProperty Name",
            ],
        )
    } else if cfg!(target_os = "macos") {
        query("system_profiler", &["SPDisplaysDataType"])
            .lines()
            .filter_map(|line| line.trim().strip_prefix("Chipset Model: "))
            .collect::<Vec<_>>()
            .join(" / ")
    } else {
        query("lspci", &["-mm"])
            .lines()
            .filter(|line| line.contains("VGA") || line.contains("Display") || line.contains("3D"))
            .collect::<Vec<_>>()
            .join(" / ")
    };
    let cpu = if cfg!(windows) {
        query(
            "powershell.exe",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-CimInstance Win32_Processor -ErrorAction Stop | Select-Object -ExpandProperty Name",
            ],
        )
    } else if cfg!(target_os = "macos") {
        query("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else {
        fs::read_to_string("/proc/cpuinfo")
            .unwrap_or_default()
            .lines()
            .find_map(|line| {
                line.strip_prefix("model name")
                    .and_then(|s| s.split_once(':'))
                    .map(|(_, v)| v.trim().to_string())
            })
            .unwrap_or_default()
    };
    let ffmpeg = find("ffmpeg");
    let choice = |live| {
        ffmpeg
            .as_deref()
            .map_or("Unavailable", |path| encoder::auto(path, live).name)
    };
    Hardware {
        gpu: if gpu.is_empty() {
            "Unknown".into()
        } else {
            gpu
        },
        cpu: if cpu.is_empty() {
            "Unknown".into()
        } else {
            cpu
        },
        encoder: choice(true).into(),
        file_encoder: choice(false).into(),
    }
}

pub fn kick_playback(video_id: &str, ctx: &Context) -> Result<Value> {
    let bytes = request_bytes(
        &format!("https://web.kick.com/api/v1/stream/{video_id}/playback"),
        Some(
            serde_json::json!({"video_player":{"player":{}},"video_session":{},"user_session":{"non_personalised_ads":true}}),
        ),
        ctx,
    )?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_archive_accepts_the_official_wrapper_folder_and_rejects_traversal() {
        use std::{
            io::{Cursor, Write},
            sync::{Arc, atomic::AtomicBool},
        };
        let zip = |name: &str| {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"fixture launcher").unwrap();
            writer.finish().unwrap().into_inner()
        };
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context {
            cancel: Arc::new(AtomicBool::new(false)),
            deadline: None,
            folder: dir.path().into(),
            reserve: 0.25,
            log: Arc::new(|_| {}),
        };
        let executable =
            unpack_streamlink(&zip("Streamlink/bin/streamlink.exe"), dir.path(), &ctx).unwrap();
        assert_eq!(executable, dir.path().join("Streamlink/bin/streamlink.exe"));
        assert_eq!(fs::read(executable).unwrap(), b"fixture launcher");
        assert!(unpack_streamlink(&zip("../outside.exe"), dir.path(), &ctx).is_err());
        assert!(!dir.path().parent().unwrap().join("outside.exe").exists());
    }
    #[test]
    fn portable_streamlink_assets_are_official_verified_and_match_the_platform() {
        let digest = "a".repeat(64);
        let windows = serde_json::json!({"assets":[{"name":"streamlink-8.6.3-1-py315-x86_64.zip","digest":format!("sha256:{digest}"),"browser_download_url":"https://github.com/streamlink/windows-builds/releases/download/8.6.3-1/portable.zip"}]});
        assert_eq!(
            select_streamlink_asset(&windows, true, "x86_64").unwrap().1,
            digest
        );
        assert!(select_streamlink_asset(&windows, true, "aarch64").is_err());
        let mut wrong = windows.clone();
        wrong["assets"][0]["browser_download_url"] = "https://example.com/portable.zip".into();
        assert!(select_streamlink_asset(&wrong, true, "x86_64").is_err());
        wrong = windows;
        wrong["assets"][0]["digest"] = "sha256:bad".into();
        assert!(select_streamlink_asset(&wrong, true, "x86_64").is_err());
        let linux = serde_json::json!({"assets":[{"name":"streamlink-8.6.3-1-cp315-cp315-manylinux_2_28_aarch64.AppImage","digest":format!("sha256:{digest}"),"browser_download_url":"https://github.com/streamlink/streamlink-appimage/releases/download/8.6.3-1/portable.AppImage"}]});
        assert!(select_streamlink_asset(&linux, false, "aarch64").is_ok());
        assert!(select_streamlink_asset(&linux, false, "x86_64").is_err());
    }
    #[test]
    fn cancelling_http_does_not_wait_for_response_headers() {
        use std::{
            net::TcpListener,
            sync::{
                Arc,
                atomic::{AtomicBool, Ordering},
                mpsc,
            },
            thread,
            time::Instant,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/waiting", listener.local_addr().unwrap());
        let (accepted, ready) = mpsc::channel();
        let server = thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            accepted.send(()).unwrap();
            thread::sleep(Duration::from_millis(700));
        });
        let dir = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let ctx = Context {
            cancel: cancel.clone(),
            deadline: None,
            folder: dir.path().into(),
            reserve: 0.25,
            log: Arc::new(|_| {}),
        };
        let worker = thread::spawn(move || request_bytes(&url, None, &ctx));
        ready.recv_timeout(Duration::from_secs(3)).unwrap();
        let start = Instant::now();
        cancel.store(true, Ordering::Relaxed);
        let error = worker.join().unwrap().err().unwrap();
        assert!(error.to_string().starts_with("Cancelled"), "{error:#}");
        assert!(start.elapsed() < Duration::from_millis(500));
        server.join().unwrap();
    }
    #[test]
    fn checksum_rejects_modified_bytes() {
        let sum = format!("{:x}", Sha256::digest(b"original"));
        assert!(checked_bytes(b"original", &sum).is_ok());
        assert!(checked_bytes(b"changed", &sum).is_err());
    }
}

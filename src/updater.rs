use crate::{model, tools};
use anyhow::{Context as _, Result, ensure};
#[cfg(windows)]
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

pub const RELEASE_PAGE: &str = "https://github.com/PeakNAtanon/PixClip/releases/latest";
const API: &str = "https://api.github.com/repos/PeakNAtanon/PixClip/releases/latest";
const MAX_PACKAGE: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installer {
    Msi,
    Deb,
    Rpm,
    Arch,
    Flatpak,
    Dmg,
}

impl Installer {
    pub fn restarts_automatically(self) -> bool {
        self != Self::Flatpak
    }
    fn suffix(self) -> &'static str {
        match self {
            Self::Msi => "windows-x64.msi",
            Self::Deb => "linux-x64.deb",
            Self::Rpm => "linux-x64.rpm",
            Self::Arch => "linux-x64.pkg.tar.zst",
            Self::Flatpak => "linux-x64.flatpak",
            Self::Dmg => "macos-universal.dmg",
        }
    }
    #[cfg(unix)]
    fn command(self) -> &'static str {
        match self {
            Self::Deb => "deb",
            Self::Rpm => "rpm",
            Self::Arch => "arch",
            Self::Flatpak => "flatpak",
            Self::Dmg => "dmg",
            Self::Msi => "msi",
        }
    }
}

fn installed_kind() -> Option<Installer> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some(Installer::Msi),
        ("macos", "aarch64" | "x86_64") => Some(Installer::Dmg),
        ("linux", "x86_64") => {
            if std::env::var("FLATPAK_ID").as_deref() == Ok("io.github.PeakNAtanon.PixClip") {
                Some(Installer::Flatpak)
            } else if Path::new("/usr/bin/pacman").is_file() {
                Some(Installer::Arch)
            } else if Path::new("/usr/bin/apt-get").is_file() {
                Some(Installer::Deb)
            } else if Path::new("/usr/bin/dnf").is_file() || Path::new("/usr/bin/zypper").is_file()
            {
                Some(Installer::Rpm)
            } else {
                None
            }
        }
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub struct Package {
    pub installer: Installer,
    pub name: String,
    pub url: String,
    pub hash: String,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub package: Option<Package>,
}

fn version(text: &str) -> Result<[u64; 3]> {
    let parts = text
        .strip_prefix('v')
        .unwrap_or(text)
        .split('.')
        .collect::<Vec<_>>();
    ensure!(
        parts.len() == 3
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())),
        "Invalid stable release version"
    );
    Ok([parts[0].parse()?, parts[1].parse()?, parts[2].parse()?])
}

fn release(value: &Value, current: &str, installer: Option<Installer>) -> Result<Option<Release>> {
    if value["draft"].as_bool() == Some(true) || value["prerelease"].as_bool() == Some(true) {
        return Ok(None);
    }
    let tag = value["tag_name"].as_str().context("Missing release tag")?;
    let numbers = version(tag)?;
    if numbers <= version(current)? {
        return Ok(None);
    }
    let display = format!("{}.{}.{}", numbers[0], numbers[1], numbers[2]);
    ensure!(tag == format!("v{display}"), "Unexpected release tag");
    let name = installer.map(|kind| format!("PixClip-{display}-{}", kind.suffix()));
    let asset = value["assets"].as_array().and_then(|assets| {
        assets
            .iter()
            .find(|a| a["name"].as_str() == name.as_deref())
    });
    let package = if let (Some(installer), Some(name), Some(asset)) = (installer, name, asset) {
        let url = format!("https://github.com/PeakNAtanon/PixClip/releases/download/{tag}/{name}");
        ensure!(
            asset["browser_download_url"].as_str() == Some(&url),
            "Unexpected update download source"
        );
        let hash = asset["digest"]
            .as_str()
            .and_then(|s| s.strip_prefix("sha256:"))
            .context("The release is missing a SHA-256 digest")?;
        ensure!(
            hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid update checksum"
        );
        let size = asset["size"].as_u64().context("Missing update size")?;
        ensure!(size > 0 && size <= MAX_PACKAGE, "Unexpected update size");
        Some(Package {
            installer,
            name,
            url,
            hash: hash.to_ascii_lowercase(),
            size,
        })
    } else {
        None
    };
    Ok(Some(Release {
        version: display,
        package,
    }))
}

pub fn check() -> Result<Option<Release>> {
    let response = tools::client()?
        .get(API)
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(15))
        .send()?
        .error_for_status()?;
    let mut bytes = Vec::new();
    response.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 2 * 1024 * 1024,
        "Release response is too large"
    );
    release(
        &serde_json::from_slice(&bytes)?,
        env!("CARGO_PKG_VERSION"),
        installed_kind(),
    )
}

fn verify_file(path: &Path, expected: &str, size: u64) -> Result<()> {
    let mut file = fs::File::open(path)?;
    ensure!(
        file.metadata()?.len() == size,
        "Update download is incomplete"
    );
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    ensure!(
        format!("{:x}", hash.finalize()).eq_ignore_ascii_case(expected),
        "Update checksum verification failed; nothing was installed"
    );
    Ok(())
}

pub fn download(package: &Package, progress: impl Fn(f32)) -> Result<PathBuf> {
    let folder = model::state_dir().join("updates");
    download_into(package, &folder, progress)
}

fn download_into(package: &Package, folder: &Path, progress: impl Fn(f32)) -> Result<PathBuf> {
    model::require_space(folder, 0.25, package.size)?;
    let mut response = tools::client()?
        .get(&package.url)
        .timeout(Duration::from_secs(600))
        .send()?
        .error_for_status()?;
    let mut file = tempfile::NamedTempFile::new_in(folder)?;
    let mut count = 0_u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = response.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        ensure!(
            count <= package.size,
            "Update download exceeded its expected size"
        );
        file.write_all(&buffer[..n])?;
        progress(count as f32 / package.size as f32);
    }
    file.as_file().sync_all()?;
    verify_file(file.path(), &package.hash, package.size)?;
    let target = folder.join(&package.name);
    file.persist(&target).map_err(|e| e.error)?;
    Ok(target)
}

pub fn install(package: &Package, path: &Path) -> Result<()> {
    verify_file(path, &package.hash, package.size)?;
    #[cfg(windows)]
    {
        ensure!(
            package.installer == Installer::Msi,
            "This package is not a Windows installer"
        );
        #[derive(Serialize)]
        struct Plan {
            pid: u32,
            msi: PathBuf,
            hash: String,
            original: PathBuf,
            log: PathBuf,
        }
        let folder = path.parent().context("Missing update directory")?;
        let script = folder.join("install-update.ps1");
        let plan = folder.join("install-update.json");
        fs::write(
            &script,
            include_str!("../scripts/windows/install-update.ps1"),
        )?;
        model::atomic_json(
            &plan,
            &Plan {
                pid: std::process::id(),
                msi: path.into(),
                hash: package.hash.clone(),
                original: std::env::current_exe()?,
                log: folder.join("install-update.log"),
            },
        )?;
        let system = PathBuf::from(
            std::env::var_os("SystemRoot").context("Missing Windows system directory")?,
        )
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        crate::process::command(system)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-WindowStyle",
                "Hidden",
                "-File",
            ])
            .arg(script)
            .arg("-Plan")
            .arg(plan)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("Could not start the update installer")?;
        Ok(())
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        ensure!(
            package.installer != Installer::Msi,
            "This package is not a Unix installer"
        );
        if package.installer == Installer::Flatpak {
            return crate::process::open(path);
        }
        let original = std::env::current_exe()?;
        let destination = if package.installer == Installer::Dmg {
            original
                .ancestors()
                .find(|p| p.extension().is_some_and(|e| e == "app"))
                .map(Path::to_path_buf)
                .unwrap_or_else(|| model::home().join("Applications/PixClip.app"))
        } else {
            PathBuf::from("/usr/bin/pixclip")
        };
        let folder = path.parent().context("Missing update directory")?;
        let script = folder.join("install-update.sh");
        fs::write(&script, include_str!("../scripts/install-update.sh"))?;
        if matches!(
            package.installer,
            Installer::Deb | Installer::Rpm | Installer::Arch
        ) {
            // geteuid only reads the current process identity.
            ensure!(
                unsafe { libc::geteuid() } == 0 || Path::new("/usr/bin/pkexec").is_file(),
                "Install PolicyKit (pkexec) to authorize package updates"
            );
        }
        let mut command = crate::process::command("/bin/sh");
        command
            .arg(script)
            .arg(std::process::id().to_string())
            .arg(package.installer.command())
            .arg(path)
            .arg(&package.hash)
            .arg(original)
            .arg(destination)
            .arg(folder.join("install-update.log"))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()
            .context("Could not start the update installer")?;
        Ok(())
    }
}

pub fn take_install_error() -> Option<String> {
    let path = model::state_dir().join("updates/install-update.log.failed");
    let bytes = fs::read(&path).ok()?;
    let message = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_string();
    let _ = fs::remove_file(path);
    Some(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_platform_selects_its_own_verified_package() {
        let kinds = [
            Installer::Msi,
            Installer::Deb,
            Installer::Rpm,
            Installer::Arch,
            Installer::Flatpak,
            Installer::Dmg,
        ];
        let mut response = fixture();
        response["assets"] = kinds.into_iter().map(|kind| {
            let name = format!("PixClip-1.10.0-{}", kind.suffix());
            serde_json::json!({"browser_download_url":format!("https://github.com/PeakNAtanon/PixClip/releases/download/v1.10.0/{name}"),"name":name,"size":42,"digest":format!("sha256:{}","a".repeat(64))})
        }).collect::<Vec<_>>().into();
        for kind in kinds {
            let result = release(&response, "1.0.0", Some(kind))
                .unwrap()
                .unwrap()
                .package
                .unwrap();
            assert_eq!(result.installer, kind);
            assert!(result.name.ends_with(kind.suffix()));
            assert_eq!(kind.restarts_automatically(), kind != Installer::Flatpak);
        }
    }
    #[test]
    fn http_download_is_verified_before_replacing_a_cached_installer() {
        use std::net::TcpListener;
        for payload in [
            b"new-file".as_slice(),
            b"modified".as_slice(),
            b"short".as_slice(),
            b"too-long-file".as_slice(),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/update.msi", listener.local_addr().unwrap());
            let payload = payload.to_vec();
            let valid = payload == b"new-file";
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 2048];
                assert!(stream.read(&mut request).unwrap() > 0);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                )
                .unwrap();
                stream.write_all(&payload).unwrap();
            });
            let dir = tempfile::tempdir().unwrap();
            let target = dir.path().join("update.msi");
            fs::write(&target, b"existing installer").unwrap();
            let package = Package {
                installer: Installer::Msi,
                name: "update.msi".into(),
                url,
                hash: format!("{:x}", Sha256::digest(b"new-file")),
                size: 8,
            };
            let result = download_into(&package, dir.path(), |_| {});
            assert_eq!(result.is_ok(), valid);
            assert_eq!(
                fs::read(&target).unwrap(),
                if valid {
                    b"new-file".as_slice()
                } else {
                    b"existing installer".as_slice()
                }
            );
            server.join().unwrap();
        }
    }
    fn fixture() -> Value {
        serde_json::json!({"tag_name":"v1.10.0", "assets":[{"name":"PixClip-1.10.0-windows-x64.msi", "browser_download_url":"https://github.com/PeakNAtanon/PixClip/releases/download/v1.10.0/PixClip-1.10.0-windows-x64.msi", "digest":format!("sha256:{}", "a".repeat(64)), "size":42}]})
    }
    #[test]
    fn stable_versions_are_compared_numerically_and_never_downgraded() {
        assert!(
            release(&fixture(), "1.9.0", Some(Installer::Msi))
                .unwrap()
                .is_some()
        );
        assert!(
            release(&fixture(), "1.10.0", Some(Installer::Msi))
                .unwrap()
                .is_none()
        );
        assert!(
            release(&fixture(), "2.0.0", Some(Installer::Msi))
                .unwrap()
                .is_none()
        );
        assert!(version("1.10.0-beta.1").is_err());
        let mut value = fixture();
        value["prerelease"] = true.into();
        assert!(
            release(&value, "1.9.0", Some(Installer::Msi))
                .unwrap()
                .is_none()
        );
        value["prerelease"] = false.into();
        value["draft"] = true.into();
        assert!(
            release(&value, "1.9.0", Some(Installer::Msi))
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn updater_rejects_external_sources_missing_hashes_and_wrong_sizes() {
        for (field, value) in [
            (
                "browser_download_url",
                Value::from("https://example.com/update.msi"),
            ),
            ("digest", Value::Null),
            ("digest", Value::from("sha256:bad")),
            ("size", Value::from(0)),
        ] {
            let mut response = fixture();
            response["assets"][0][field] = value;
            assert!(release(&response, "1.0.0", Some(Installer::Msi)).is_err());
        }
        assert!(
            release(&fixture(), "1.0.0", None)
                .unwrap()
                .unwrap()
                .package
                .is_none()
        );
    }
    #[test]
    fn a_tampered_or_incomplete_installer_is_not_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update.msi");
        fs::write(&path, b"original").unwrap();
        let hash = format!("{:x}", Sha256::digest(b"original"));
        verify_file(&path, &hash, 8).unwrap();
        assert!(verify_file(&path, &hash, 9).is_err());
        fs::write(&path, b"modified").unwrap();
        assert!(verify_file(&path, &hash, 8).is_err());
    }
}

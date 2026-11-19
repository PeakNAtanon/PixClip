use crate::model;
use anyhow::{Context as _, Result, bail, ensure};
use std::{
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, ChildStderr, ChildStdout, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub type Logger = Arc<dyn Fn(String) + Send + Sync>;

#[derive(Clone)]
pub struct Context {
    pub cancel: Arc<AtomicBool>,
    pub deadline: Option<f64>,
    pub folder: PathBuf,
    pub reserve: f64,
    pub log: Logger,
}

pub struct Output {
    pub code: i32,
    pub text: String,
    pub timed_out: bool,
}

pub fn command(executable: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(executable);
    if matches!(
        Path::new(command.get_program())
            .file_stem()
            .and_then(|s| s.to_str()),
        Some("yt-dlp" | "streamlink")
    ) {
        command.env("PYTHONIOENCODING", "utf-8");
    }
    #[cfg(unix)]
    if Path::new(command.get_program())
        .extension()
        .is_some_and(|s| s == "AppImage")
    {
        command.env("APPIMAGE_EXTRACT_AND_RUN", "1");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

fn pipe_lines(reader: impl BufRead) -> impl Iterator<Item = String> {
    reader.split(b'\n').map_while(|bytes| {
        let bytes = bytes.ok()?;
        let text = bytes.strip_suffix(b"\r").unwrap_or(&bytes);
        Some(String::from_utf8_lossy(text).into_owned())
    })
}

pub(crate) struct ManagedChild {
    child: Child,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}

pub(crate) fn piped(
    executable: &Path,
    arguments: &[String],
    pipe_stderr: bool,
) -> Result<(ManagedChild, ChildStdout, Option<ChildStderr>)> {
    let mut command = command(executable);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(if pipe_stderr {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut process = ManagedChild::spawn(command)?;
    let video = process.child.stdout.take().expect("piped stdout");
    let audio = process.child.stderr.take();
    Ok((process, video, audio))
}

impl ManagedChild {
    fn spawn(mut command: Command) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = command.spawn().context("Could not start media tool")?;
        #[cfg(windows)]
        {
            let mut child = child;
            use std::{
                mem::{size_of, zeroed},
                os::windows::io::AsRawHandle,
                ptr::null,
            };
            use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
            unsafe {
                let job = CreateJobObjectW(null(), null());
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if job.is_null()
                    || SetInformationJobObject(
                        job,
                        JobObjectExtendedLimitInformation,
                        &limits as *const _ as _,
                        size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                    ) == 0
                    || AssignProcessToJobObject(job, child.as_raw_handle() as _) == 0
                {
                    let error = std::io::Error::last_os_error();
                    let _ = child.kill();
                    let _ = child.wait();
                    if !job.is_null() {
                        CloseHandle(job);
                    }
                    return Err(error).context("Could not isolate media process tree");
                }
                Ok(Self { child, job })
            }
        }
        #[cfg(not(windows))]
        {
            Ok(Self { child })
        }
    }
    fn stop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 130);
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        self.stop();
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
    }
}

pub fn public_line(line: &str) -> String {
    line.split_whitespace()
        .map(|word| {
            if word.contains("http://") || word.contains("https://") {
                "<media URL>"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Context {
    pub fn check(&self) -> Result<()> {
        ensure!(
            !self.cancel.load(Ordering::Relaxed),
            "Cancelled; partial files kept"
        );
        model::require_space(&self.folder, self.reserve, 0)?;
        Ok(())
    }
    pub fn emit(&self, text: impl Into<String>) {
        (self.log)(text.into());
    }

    pub fn run(
        &self,
        executable: &Path,
        arguments: &[String],
        capture: bool,
        recording: bool,
    ) -> Result<Output> {
        self.check()?;
        ensure!(
            !self.deadline.is_some_and(|d| model::now() >= d),
            "Scheduled Live window ended before capture could start"
        );
        let mut command = command(executable);
        command
            .args(arguments)
            .current_dir(&self.folder)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        self.emit(format!(
            "Running {}",
            executable.file_name().unwrap_or_default().to_string_lossy()
        ));
        let mut process = ManagedChild::spawn(command)?;
        let (sender, receiver) = mpsc::channel();
        let mut readers = vec![];
        for pipe in [
            Box::new(process.child.stdout.take().unwrap()) as Box<dyn Read + Send>,
            Box::new(process.child.stderr.take().unwrap()),
        ] {
            let sender = sender.clone();
            readers.push(thread::spawn(move || {
                for line in pipe_lines(BufReader::new(pipe)) {
                    if sender.send(line).is_err() {
                        break;
                    }
                }
            }));
        }
        drop(sender);
        let start = Instant::now();
        let mut last_space_check = Instant::now();
        let mut text = String::new();
        let mut failure = None;
        let mut timed_out = false;
        let status = loop {
            for line in receiver.try_iter() {
                if capture {
                    if text.len() < 8 * 1024 * 1024 {
                        text.push_str(&line);
                        text.push('\n');
                    }
                } else {
                    self.emit(public_line(&line));
                }
            }
            if let Some(status) = process.child.try_wait()? {
                break Some(status);
            }
            if self.cancel.load(Ordering::Relaxed) {
                failure = Some("Cancelled; partial files kept".to_string());
                break None;
            }
            if self.deadline.is_some_and(|d| model::now() >= d) {
                if recording {
                    timed_out = true;
                } else {
                    failure = Some("Scheduled Live window ended while resolving metadata".into());
                }
                break None;
            }
            if capture && start.elapsed() > Duration::from_secs(90) {
                failure = Some("Metadata request timed out after 90 seconds".into());
                break None;
            }
            if last_space_check.elapsed() >= Duration::from_millis(500) {
                if let Err(error) = self.check() {
                    failure = Some(error.to_string());
                    break None;
                }
                last_space_check = Instant::now();
            }
            thread::sleep(Duration::from_millis(50));
        };
        process.stop();
        for reader in readers {
            let _ = reader.join();
        }
        for line in receiver.try_iter() {
            if capture {
                if text.len() < 8 * 1024 * 1024 {
                    text.push_str(&line);
                    text.push('\n');
                }
            } else {
                self.emit(public_line(&line));
            }
        }
        if let Some(error) = failure {
            bail!("{error}");
        }
        Ok(Output {
            code: if timed_out {
                0
            } else {
                status.and_then(|s| s.code()).unwrap_or(1)
            },
            text,
            timed_out,
        })
    }
}

pub fn capture(executable: &Path, arguments: &[String], timeout: Duration) -> Result<Vec<u8>> {
    let mut command = command(executable);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut process = ManagedChild::spawn(command)?;
    let output = process.child.stdout.take().unwrap();
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        output
            .take(16 * 1024 * 1024)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = process.child.try_wait()? {
            break Some(status);
        }
        if start.elapsed() >= timeout {
            break None;
        }
        thread::sleep(Duration::from_millis(20));
    };
    process.stop();
    let bytes = reader
        .join()
        .map_err(|_| anyhow::anyhow!("Tool output reader failed"))??;
    ensure!(status.is_some(), "Media probe timed out");
    ensure!(status.unwrap().success(), "Media probe failed");
    Ok(bytes)
}

pub fn open(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::{os::windows::ffi::OsStrExt, ptr::null};
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let result = unsafe {
            windows_sys::Win32::UI::Shell::ShellExecuteW(
                std::ptr::null_mut(),
                null(),
                wide.as_ptr(),
                null(),
                null(),
                1,
            )
        };
        ensure!(result as isize > 32, "Could not open {}", path.display());
    }
    #[cfg(not(windows))]
    {
        command(if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        })
        .arg(path)
        .spawn()?;
    }
    Ok(())
}

pub fn open_cli() -> Result<()> {
    let executable = std::env::current_exe()?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        Command::new(executable)
            .arg("--cli")
            .current_dir(model::root())
            .creation_flags(0x00000010)
            .spawn()?;
    }
    #[cfg(not(windows))]
    {
        let terminal = ["x-terminal-emulator", "xterm", "konsole", "gnome-terminal"]
            .into_iter()
            .find_map(|name| crate::tools::find(name).map(|path| (name, path)))
            .context("No terminal was found. Run PixClip --cli from your terminal.")?;
        command(&terminal.1)
            .current_dir(model::root())
            .arg(if terminal.0 == "gnome-terminal" {
                "--"
            } else {
                "-e"
            })
            .arg(executable)
            .arg("--cli")
            .spawn()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_non_utf8_log_line_does_not_stop_reading_later_output() {
        let bytes = b"Downloading Caf\xe9\r\nprogress=100\nFinished";
        assert_eq!(
            pipe_lines(bytes.as_slice()).collect::<Vec<_>>(),
            ["Downloading Caf\u{fffd}", "progress=100", "Finished"]
        );
    }

    #[test]
    fn cancellation_stops_a_waiting_process() {
        let dir = tempfile::tempdir().unwrap();
        let flag = Arc::new(AtomicBool::new(false));
        let ctx = Context {
            cancel: flag.clone(),
            deadline: None,
            folder: dir.path().into(),
            reserve: 0.25,
            log: Arc::new(|_| {}),
        };
        let start = Instant::now();
        let thread = thread::spawn(move || {
            if cfg!(windows) {
                ctx.run(
                    Path::new("powershell.exe"),
                    &[
                        "-NoProfile".into(),
                        "-NonInteractive".into(),
                        "-Command".into(),
                        "Start-Sleep -Seconds 30".into(),
                    ],
                    true,
                    false,
                )
            } else {
                ctx.run(
                    Path::new("sh"),
                    &["-c".into(), "sleep 30 & wait".into()],
                    true,
                    false,
                )
            }
        });
        thread::sleep(Duration::from_millis(250));
        flag.store(true, Ordering::Relaxed);
        let error = thread
            .join()
            .unwrap()
            .err()
            .expect("Cancellation must stop the command");
        assert!(error.to_string().starts_with("Cancelled"), "{error:#}");
        assert!(start.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn signed_urls_are_not_logged() {
        assert!(!public_line("Failed https://host/video?token=secret").contains("secret"));
    }
}

use super::*;
use crate::updater::{self, Package, Release};

enum Event {
    Checked(std::result::Result<Option<Release>, String>),
    Progress(f32),
    Downloaded(std::result::Result<PathBuf, String>),
}

pub(super) struct Updater {
    sender: mpsc::Sender<Event>,
    receiver: mpsc::Receiver<Event>,
    release: Option<Release>,
    downloaded: Option<PathBuf>,
    checking: bool,
    downloading: bool,
    progress: f32,
    message: String,
    open: bool,
    last_check: Instant,
}

impl Updater {
    pub fn new(ctx: &egui::Context) -> Self {
        let (sender, receiver) = mpsc::channel();
        let mut state = Self {
            sender,
            receiver,
            release: None,
            downloaded: None,
            checking: false,
            downloading: false,
            progress: 0.0,
            message: String::new(),
            open: false,
            last_check: Instant::now(),
        };
        if !cfg!(any(test, feature = "screenshot")) {
            if let Some(error) = updater::take_install_error() {
                state.message = error;
                state.open = true;
            } else {
                state.check(ctx);
            }
        }
        state
    }

    fn check(&mut self, ctx: &egui::Context) {
        if self.checking || self.downloading {
            return;
        }
        self.checking = true;
        self.message.clear();
        self.last_check = Instant::now();
        let sender = self.sender.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = updater::check().map_err(|e| format!("{e:#}"));
            let _ = sender.send(Event::Checked(result));
            ctx.request_repaint();
        });
    }

    fn download(&mut self, ctx: &egui::Context, package: Package) {
        self.downloading = true;
        self.progress = 0.0;
        self.message.clear();
        let sender = self.sender.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = updater::download(&package, |value| {
                let _ = sender.send(Event::Progress(value));
                ctx.request_repaint();
            })
            .map_err(|e| format!("{e:#}"));
            let _ = sender.send(Event::Downloaded(result));
            ctx.request_repaint();
        });
    }

    pub fn help_button(&mut self, ui: &mut egui::Ui, thai: bool) {
        ui.label(format!("PixClip {}", env!("CARGO_PKG_VERSION")));
        if ui
            .add_enabled(
                !self.checking && !self.downloading,
                egui::Button::new(if thai {
                    "ตรวจอัปเดต PixClip"
                } else {
                    "Check PixClip updates"
                }),
            )
            .clicked()
        {
            self.open = true;
            self.check(ui.ctx());
        }
        ui.separator();
    }

    pub fn show(&mut self, ctx: &egui::Context, jobs_pending: bool, thai: bool) -> bool {
        let mut install = false;
        for event in self.receiver.try_iter().collect::<Vec<_>>() {
            match event {
                Event::Checked(result) => {
                    self.checking = false;
                    match result {
                        Ok(Some(release)) => {
                            self.release = Some(release);
                            self.downloaded = None;
                            self.open = true;
                        }
                        Ok(None) => {
                            self.release = None;
                            self.downloaded = None;
                            self.message = if thai {
                                "คุณใช้เวอร์ชันล่าสุดแล้ว"
                            } else {
                                "PixClip is up to date"
                            }
                            .into();
                        }
                        Err(error) => self.message = error,
                    }
                }
                Event::Progress(value) => self.progress = value,
                Event::Downloaded(result) => {
                    self.downloading = false;
                    match result {
                        Ok(path) => {
                            self.downloaded = Some(path);
                            install = !jobs_pending;
                        }
                        Err(error) => {
                            self.message = error;
                            self.open = true;
                        }
                    }
                }
            }
        }
        if !cfg!(any(test, feature = "screenshot"))
            && !self.open
            && self.last_check.elapsed() >= Duration::from_secs(6 * 60 * 60)
        {
            self.check(ctx);
        }
        if self.open {
            let mut open = true;
            egui::Window::new(if thai { "อัปเดต PixClip" } else { "PixClip update" })
                .id(egui::Id::new("pixclip-update"))
                .open(&mut open).resizable(false).default_width(420.0)
                .show(ctx, |ui| {
                    ui.label(format!("PixClip {}", env!("CARGO_PKG_VERSION")));
                    if self.checking { ui.spinner(); ui.label(if thai { "กำลังตรวจเวอร์ชันใหม่…" } else { "Checking for updates…" }); }
                    if let Some(release) = self.release.clone() {
                        ui.label(RichText::new(format!("{} {}", if thai { "มีเวอร์ชันใหม่:" } else { "New version:" }, release.version)).color(MINT).strong());
                        if jobs_pending { ui.label(if thai { "ให้งานในคิวเสร็จก่อนอัปเดต โปรแกรมจะปิดและเปิดใหม่" } else { "Finish queued jobs before updating. PixClip will restart." }); }
                        if self.downloading {
                            ui.add(egui::ProgressBar::new(self.progress).show_percentage());
                        } else if let Some(package) = release.package {
                            ui.label(if thai { "ดาวน์โหลดและตรวจ SHA-256 ก่อนติดตั้ง ประวัติและการตั้งค่าจะยังอยู่ ระบบอาจขอรหัสผ่านหรือยืนยันสิทธิ์ก่อนติดตั้ง" } else { "The download is verified before installation. History and settings are kept. Your system may ask for installation permission." });
                            let label = if package.installer.restarts_automatically() {
                                if thai { "อัปเดตและเปิดใหม่" } else { "Update and restart" }
                            } else if thai { "ดาวน์โหลดและเปิดตัวติดตั้ง" } else { "Download and open installer" };
                            if ui.add_enabled(!jobs_pending && !self.checking, egui::Button::new(label)).clicked() {
                                if self.downloaded.is_some() { install = true; } else { self.download(ctx, package); }
                            }
                        } else {
                            ui.label(if thai { "ยังไม่มีแพ็กเกจอัปเดตที่ตรงกับระบบนี้ กรุณาตรวจหน้า Releases" } else { "No matching update package is published for this system. Check Releases." });
                        }
                        if ui.button(if thai { "เปิดหน้า Releases" } else { "Open Releases" }).clicked()
                            && let Err(error) = process::open(Path::new(updater::RELEASE_PAGE)) {
                                self.message = error.to_string();
                            }
                    }
                    if !self.message.is_empty() { ui.label(&self.message); }
                });
            self.open = open;
        }
        install
    }

    pub fn install(&mut self) -> Result<bool> {
        let package = self
            .release
            .as_ref()
            .and_then(|r| r.package.as_ref())
            .context("No compatible installer is available")?;
        let restart = package.installer.restarts_automatically();
        updater::install(
            package,
            self.downloaded
                .as_deref()
                .context("Download the update first")?,
        )?;
        if !restart {
            self.release = None;
            self.message = "Flatpak installer opened. Confirm installation in your system, then reopen PixClip.".into();
            self.open = true;
        }
        Ok(restart)
    }

    pub fn install_failed(&mut self, error: impl std::fmt::Display) {
        self.message = error.to_string();
        self.open = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_download_never_closes_an_app_with_queued_jobs() {
        for jobs_pending in [false, true] {
            let ctx = egui::Context::default();
            let mut updater = Updater::new(&ctx);
            updater
                .sender
                .send(Event::Downloaded(Ok(PathBuf::from("verified.msi"))))
                .unwrap();
            assert_eq!(updater.show(&ctx, jobs_pending, true), !jobs_pending);
            assert!(updater.downloaded.is_some());
        }
    }
    #[test]
    fn failed_download_stays_in_the_app_and_can_be_retried() {
        let ctx = egui::Context::default();
        let mut updater = Updater::new(&ctx);
        updater.downloading = true;
        updater
            .sender
            .send(Event::Downloaded(Err("network unavailable".into())))
            .unwrap();
        let mut install = false;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            install = updater.show(ctx, false, false)
        });
        assert!(!install);
        assert!(!updater.downloading);
        assert!(updater.downloaded.is_none());
        assert!(updater.open);
        assert!(updater.message.contains("network unavailable"));
    }
}

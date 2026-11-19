use super::*;
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct Item {
    index: usize,
    title: String,
    selected: bool,
}

pub(super) struct Review {
    job: Job,
    receiver: mpsc::Receiver<Result<(String, Vec<Item>)>>,
    cancel: Arc<AtomicBool>,
    title: String,
    items: Vec<Item>,
    loading: bool,
    error: Option<String>,
    mode: usize,
    quality: usize,
    audio_format: usize,
    audio_quality: usize,
}

impl Drop for Review {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn read_playlist(job: &Job, context: &process::Context) -> Result<(String, Vec<Item>)> {
    let mut args = media::args(&[
        "--ignore-config",
        "--no-warnings",
        "--yes-playlist",
        "--flat-playlist",
        "--skip-download",
        "--dump-single-json",
        "--socket-timeout",
        "10",
    ]);
    if let Some(cookies) = &job.cookies_file {
        args.extend(["--cookies".into(), media::text(cookies)]);
    }
    args.extend(["--".into(), job.url.clone()]);
    let output = context.run(&tools::require("yt-dlp")?, &args, true, false)?;
    ensure!(
        output.code == 0,
        "Could not read the playlist. Check the link and Cookies."
    );
    let playlist: Value = output
        .text
        .lines()
        .find_map(|line| serde_json::from_str(line).ok())
        .context("Could not read the playlist list")?;
    let entries = playlist["entries"]
        .as_array()
        .context("Use a playlist link")?;
    let items: Vec<_> = entries
        .iter()
        .enumerate()
        .filter_map(|(position, entry)| {
            let title = entry["title"].as_str()?;
            let index = entry["playlist_index"]
                .as_u64()
                .and_then(|number| usize::try_from(number).ok())
                .filter(|number| *number > 0)
                .unwrap_or(position + 1);
            Some(Item {
                index,
                title: title.into(),
                selected: false,
            })
        })
        .collect();
    ensure!(!items.is_empty(), "No items were visible in the playlist");
    Ok((
        playlist["title"].as_str().unwrap_or("Playlist").into(),
        items,
    ))
}

impl App {
    pub(super) fn start_playlist_review(&mut self, url: &str) -> Result<()> {
        ensure!(
            self.playlist_review.is_none(),
            "Finish the current playlist selection first"
        );
        let job = self.job_template(url.into());
        job.validate()?;
        let cancel = Arc::new(AtomicBool::new(false));
        let context = process::Context {
            cancel: cancel.clone(),
            deadline: None,
            folder: job.folder.clone(),
            reserve: job.reserve_gb,
            log: Arc::new(|_| {}),
        };
        let (sender, receiver) = mpsc::channel();
        let source = job.clone();
        thread::spawn(move || {
            let _ = sender.send(read_playlist(&source, &context));
        });
        self.playlist_review = Some(Review {
            job,
            receiver,
            cancel,
            title: "Loading playlist…".into(),
            items: vec![],
            loading: true,
            error: None,
            mode: 0,
            quality: self.quality.min(4),
            audio_format: self.audio_format,
            audio_quality: self.audio_quality,
        });
        Ok(())
    }

    pub(super) fn show_playlist_review(&mut self, ctx: &egui::Context) {
        let Some(review) = &mut self.playlist_review else {
            return;
        };
        if let Ok(result) = review.receiver.try_recv() {
            review.loading = false;
            match result {
                Ok((title, items)) => {
                    review.title = title;
                    review.items = items;
                }
                Err(error) => {
                    review.title = "Playlist selection".into();
                    review.error = Some(process::public_line(&format!("{error:#}")));
                }
            }
        }
        if review.loading {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let mut close = false;
        let mut submit = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("playlist-review"),
            egui::ViewportBuilder::default()
                .with_title("PixClip - Select playlist items")
                .with_inner_size([820.0, 570.0])
                .with_min_inner_size([560.0, 350.0]),
            |ctx, _| {
                close = ctx.input(|i| i.viewport().close_requested());
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(BG).inner_margin(16))
                    .show(ctx, |ui| {
                        flow_heading(ui, &review.title, "file");
                        if review.loading {
                            pixel_progress(ui, None);
                        }
                        if let Some(error) = &review.error {
                            ui.colored_label(AMBER, error);
                        }
                        if !review.items.is_empty() {
                            ui.horizontal(|ui| {
                                combo(
                                    ui,
                                    "playlist-mode",
                                    &mut review.mode,
                                    &["MP4", "Compatible MP4", "Audio"],
                                    144.0,
                                );
                                if review.mode == 2 {
                                    combo(
                                        ui,
                                        "playlist-format",
                                        &mut review.audio_format,
                                        &AUDIO_FORMATS,
                                        90.0,
                                    );
                                    if media::audio_encoder(AUDIO_FORMATS[review.audio_format])
                                        .is_some()
                                    {
                                        combo(
                                            ui,
                                            "playlist-bitrate",
                                            &mut review.audio_quality,
                                            &["Best quality", "320 kbps", "192 kbps", "128 kbps"],
                                            128.0,
                                        );
                                    }
                                } else {
                                    combo(
                                        ui,
                                        "playlist-quality",
                                        &mut review.quality,
                                        &QUALITIES[..5],
                                        128.0,
                                    );
                                }
                            });
                            ui.horizontal(|ui| {
                                if flow_button(ui, "Select all", "", 105.0, SURFACE_RAISED, true)
                                    .clicked()
                                {
                                    for item in &mut review.items {
                                        item.selected = true;
                                    }
                                }
                                if flow_button(ui, "Clear", "clear", 92.0, SURFACE_RAISED, true)
                                    .clicked()
                                {
                                    for item in &mut review.items {
                                        item.selected = false;
                                    }
                                }
                            });
                            let height = (ui.available_height() - 65.0).max(36.0);
                            list_panel(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .max_height(height)
                                    .show(ui, |ui| {
                                        for item in &mut review.items {
                                            ui.checkbox(
                                                &mut item.selected,
                                                format!("{}. {}", item.index, item.title),
                                            );
                                        }
                                    });
                            });
                        }
                        ui.horizontal(|ui| {
                            let count = review.items.iter().filter(|item| item.selected).count();
                            submit = flow_button(
                                ui,
                                &format!("Add {count} to queue"),
                                "download",
                                200.0,
                                GREEN,
                                count > 0,
                            )
                            .clicked();
                            close |=
                                flow_button(ui, "Close", "", 92.0, SURFACE_RAISED, true).clicked();
                        });
                    });
            },
        );
        if submit {
            let mut job = review.job.clone();
            job.mode = media::MODES[4 + review.mode].into();
            job.quality = QUALITIES[review.quality].into();
            job.playlist_items = review
                .items
                .iter()
                .filter(|item| item.selected)
                .map(|item| item.index)
                .collect();
            if media::audio_mode(&job.mode) {
                let format = AUDIO_FORMATS[review.audio_format];
                job.audio_format = format.into();
                job.audio_bitrate =
                    media::audio_encoder(format).and(AUDIO_BITRATES[review.audio_quality]);
            }
            match self.engine.add(job) {
                Ok(id) => {
                    self.selected = Some(id);
                    self.queue_open = true;
                    self.status = "Added selected playlist items to queue".into();
                    close = true;
                }
                Err(error) => self.error(error),
            }
        }
        if close {
            self.playlist_review = None;
        }
    }
}

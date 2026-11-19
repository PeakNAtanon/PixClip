use super::*;
use crate::spotify::{self, Candidate, Link, Track};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

enum Update {
    Tracks(String, Vec<Track>),
    Matches(usize, Result<Vec<Candidate>, String>),
    Finished(Result<(), String>),
}

struct Row {
    track: Track,
    matches: Option<Result<Vec<Candidate>, String>>,
    selected: usize,
    queued: bool,
}

pub(super) struct Review {
    job: Job,
    receiver: mpsc::Receiver<Update>,
    cancel: Arc<AtomicBool>,
    rows: Vec<Row>,
    title: String,
    busy: bool,
    error: Option<String>,
    format: usize,
    quality: usize,
}

impl Drop for Review {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl App {
    pub(super) fn start_spotify_review(&mut self, url: &str, link: Link) -> Result<()> {
        ensure!(
            self.spotify_review.is_none(),
            "Finish the current Spotify review first"
        );
        let yt_dlp = tools::require("yt-dlp")?;
        let mut job = self.job_template(url.into());
        job.mode = media::MODES[if link.is_collection() { 6 } else { 3 }].into();
        let cancel = Arc::new(AtomicBool::new(false));
        let context = process::Context {
            cancel: cancel.clone(),
            deadline: None,
            folder: job.folder.clone(),
            reserve: job.reserve_gb,
            log: Arc::new(|_| {}),
        };
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = (|| -> Result<()> {
                let (title, tracks) = link.fetch(&context)?;
                let _ = sender.send(Update::Tracks(title, tracks.clone()));
                let next = AtomicUsize::new(0);
                thread::scope(|scope| {
                    for _ in 0..tracks.len().min(3) {
                        scope.spawn(|| {
                            while !context.cancel.load(Ordering::Relaxed) {
                                let index = next.fetch_add(1, Ordering::Relaxed);
                                let Some(track) = tracks.get(index) else {
                                    break;
                                };
                                let matches = spotify::search(track, &yt_dlp, &context)
                                    .map_err(|error| process::public_line(&format!("{error:#}")));
                                if sender.send(Update::Matches(index, matches)).is_err() {
                                    break;
                                }
                            }
                        });
                    }
                });
                Ok(())
            })();
            let _ = sender.send(Update::Finished(
                result.map_err(|error| process::public_line(&format!("{error:#}"))),
            ));
        });
        self.spotify_review = Some(Review {
            job,
            receiver,
            cancel,
            rows: Vec::new(),
            title: "Reading Spotify…".into(),
            busy: true,
            error: None,
            format: self.audio_format,
            quality: self.audio_quality,
        });
        self.status = "Review Spotify matches before adding to the queue".into();
        Ok(())
    }

    pub(super) fn show_spotify_review(&mut self, ctx: &egui::Context) {
        let Some(review) = &mut self.spotify_review else {
            return;
        };
        for event in review.receiver.try_iter() {
            match event {
                Update::Tracks(title, tracks) => {
                    review.title = title;
                    review.rows = tracks
                        .into_iter()
                        .map(|track| Row {
                            track,
                            matches: None,
                            selected: 0,
                            queued: false,
                        })
                        .collect();
                }
                Update::Matches(index, matches) => review.rows[index].matches = Some(matches),
                Update::Finished(result) => {
                    review.busy = false;
                    review.error = result.err();
                }
            }
        }
        if review.busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let mut close = false;
        let mut submit = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("spotify-review"),
            egui::ViewportBuilder::default()
                .with_title("PixClip - Spotify matches")
                .with_inner_size([920.0, 600.0])
                .with_min_inner_size([580.0, 340.0]),
            |ctx, _| {
                close = ctx.input(|i| i.viewport().close_requested());
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(BG).inner_margin(16))
                    .show(ctx, |ui| {
                        flow_heading(ui, &review.title, "play");
                        ui.hyperlink_to("Spotify source", &review.job.url);
                        if let Some(error) = &review.error {
                            ui.colored_label(RED, error);
                        }
                        ui.horizontal(|ui| {
                            combo(
                                ui,
                                "spotify-format",
                                &mut review.format,
                                &AUDIO_FORMATS,
                                110.0,
                            );
                            if media::audio_encoder(AUDIO_FORMATS[review.format]).is_some() {
                                combo(
                                    ui,
                                    "spotify-bitrate",
                                    &mut review.quality,
                                    &["Best quality", "320 kbps", "192 kbps", "128 kbps"],
                                    128.0,
                                );
                            } else {
                                ui.label("Lossless");
                            }
                            if flow_button(
                                ui,
                                "Select first matches",
                                "",
                                190.0,
                                SURFACE_RAISED,
                                true,
                            )
                            .clicked()
                            {
                                for row in &mut review.rows {
                                    if !row.queued
                                        && row.matches.as_ref().is_some_and(|matches| {
                                            matches.as_ref().is_ok_and(|items| !items.is_empty())
                                        })
                                    {
                                        row.selected = 1;
                                    }
                                }
                            }
                        });
                        if review.busy {
                            let checked = review
                                .rows
                                .iter()
                                .filter(|row| row.matches.is_some())
                                .count();
                            ui.label(format!(
                                "Searching YouTube ({checked}/{})",
                                review.rows.len()
                            ));
                            if !review.rows.is_empty() {
                                pixel_progress(ui, Some(checked as f32 / review.rows.len() as f32));
                            }
                        }
                        let height = (ui.available_height() - 65.0).max(36.0);
                        list_panel(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .max_height(height)
                                .show(ui, |ui| {
                                    for (index, row) in review.rows.iter_mut().enumerate() {
                                        ui.separator();
                                        ui.label(format!(
                                            "{}. {} — {}{}",
                                            index + 1,
                                            row.track.title,
                                            row.track.artist,
                                            spotify::duration_label(row.track.duration)
                                        ));
                                        if row.queued {
                                            ui.label("Queued");
                                            continue;
                                        }
                                        match &row.matches {
                                            Some(Ok(candidates)) if !candidates.is_empty() => {
                                                let selected = row
                                                    .selected
                                                    .checked_sub(1)
                                                    .and_then(|index| candidates.get(index));
                                                dropdown(("spotify-match", index))
                                                    .selected_text(
                                                        selected.map_or(
                                                            "Skip this song",
                                                            |candidate| candidate.label.as_str(),
                                                        ),
                                                    )
                                                    .width(
                                                        (ui.available_width() - 140.0).max(220.0),
                                                    )
                                                    .show_ui(ui, |ui| {
                                                        ui.selectable_value(
                                                            &mut row.selected,
                                                            0,
                                                            "Skip this song",
                                                        );
                                                        for (candidate, result) in
                                                            candidates.iter().enumerate()
                                                        {
                                                            ui.selectable_value(
                                                                &mut row.selected,
                                                                candidate + 1,
                                                                &result.label,
                                                            );
                                                        }
                                                    });
                                                if let Some(match_index) =
                                                    row.selected.checked_sub(1)
                                                    && let Some(candidate) =
                                                        candidates.get(match_index)
                                                {
                                                    ui.hyperlink_to(
                                                        "Check on YouTube",
                                                        &candidate.url,
                                                    );
                                                }
                                            }
                                            Some(Err(error)) => {
                                                ui.colored_label(AMBER, error);
                                            }
                                            Some(Ok(_)) => {
                                                ui.label("No match found");
                                            }
                                            None => {
                                                ui.label("Waiting for search");
                                            }
                                        }
                                    }
                                });
                        });
                        ui.horizontal(|ui| {
                            let selected = review
                                .rows
                                .iter()
                                .filter(|row| !row.queued && row.selected > 0)
                                .count();
                            submit = flow_button(
                                ui,
                                &format!("Add {selected} to queue"),
                                "download",
                                200.0,
                                GREEN,
                                selected > 0,
                            )
                            .clicked();
                            close |=
                                flow_button(ui, "Close", "", 92.0, SURFACE_RAISED, true).clicked();
                        });
                    });
            },
        );
        if submit && let Err(error) = self.queue_spotify_matches() {
            self.error(error);
        }
        if close {
            self.spotify_review = None;
        }
    }

    fn queue_spotify_matches(&mut self) -> Result<()> {
        let review = self
            .spotify_review
            .as_mut()
            .context("Open a Spotify review first")?;
        let group = model::organized_folder(&review.job)?;
        let collection = review.job.mode.starts_with("Playlist");
        let format = AUDIO_FORMATS[review.format];
        for (index, row) in review.rows.iter_mut().enumerate() {
            if row.queued {
                continue;
            }
            let Some(candidate) = row
                .selected
                .checked_sub(1)
                .and_then(|index| row.matches.as_ref()?.as_ref().ok()?.get(index))
            else {
                continue;
            };
            let folder = if collection {
                group
                    .join(model::safe_component(&review.title))
                    .join(format!(
                        "{:03} - {}",
                        index + 1,
                        model::safe_component(&row.track.title)
                    ))
            } else {
                group.clone()
            };
            let job = Job {
                url: candidate.url.clone(),
                mode: media::MODES[3].into(),
                output_folder: Some(folder),
                audio_format: format.into(),
                audio_bitrate: media::audio_encoder(format).and(AUDIO_BITRATES[review.quality]),
                spotify_url: Some(review.job.url.clone()),
                folder: review.job.folder.clone(),
                cookies_file: review.job.cookies_file.clone(),
                reserve_gb: review.job.reserve_gb,
                ..Job::default()
            };
            self.selected = Some(self.engine.add(job)?);
            row.queued = true;
        }
        self.queue_open = true;
        if !review.busy
            && review
                .rows
                .iter()
                .all(|row| row.queued || row.selected == 0)
        {
            self.spotify_review = None;
        }
        self.status = if self.engine.history.paused {
            "Queue paused - press Resume queue to start."
        } else {
            "Added Spotify matches to queue"
        }
        .into();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spotify_confirmation_queues_only_selected_youtube_sources() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(
            &egui::Context::default(),
            Engine::new(dir.path()).unwrap(),
            Settings {
                output_directory: dir.path().into(),
                ..Settings::default()
            },
        );
        let (_, receiver) = mpsc::channel();
        let url = "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M";
        let source = "https://www.youtube.com/watch?v=abcdefghijk";
        let mut job = app.job_template(url.into());
        job.mode = media::MODES[6].into();
        app.spotify_review = Some(Review {
            job,
            receiver,
            cancel: Arc::new(AtomicBool::new(false)),
            rows: [1, 0]
                .into_iter()
                .map(|selected| Row {
                    track: Track {
                        title: "Song".into(),
                        artist: "Artist".into(),
                        duration: None,
                    },
                    matches: Some(Ok(vec![Candidate {
                        url: source.into(),
                        label: "Official".into(),
                    }])),
                    selected,
                    queued: false,
                })
                .collect(),
            title: "Test playlist".into(),
            busy: false,
            error: None,
            format: 1,
            quality: 2,
        });
        assert!(app.engine.history.jobs.is_empty());
        app.queue_spotify_matches().unwrap();
        assert_eq!(app.engine.history.jobs.len(), 1);
        let job = &app.engine.history.jobs[0];
        assert_eq!(job.url, source);
        assert_eq!(job.spotify_url.as_deref(), Some(url));
        assert_eq!(job.audio_format, "M4A");
        assert_eq!(job.audio_bitrate, Some(192));
        assert!(
            job.directory()
                .starts_with(dir.path().join("Playlists").join("Spotify"))
        );
    }
}

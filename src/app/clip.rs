use super::playback::Playback;
use super::*;
use egui::{CursorIcon, Key, Pos2, Rect, Sense, Stroke};

pub(super) const SIZE: [f32; 2] = [960.0, 760.0];
pub(super) const MIN_SIZE: [f32; 2] = [760.0, 660.0];

#[derive(Clone, Copy)]
enum Marker {
    Start,
    End,
}

pub(super) struct Clip {
    source: PathBuf,
    start: [String; 3],
    end: [String; 3],
    mode: usize,
    position: f64,
    duration: Option<f64>,
    preview: Option<TextureHandle>,
    frame_position: f64,
    note: String,
    generation: u64,
    pending: Option<Instant>,
    busy: bool,
    playing: Option<Playback>,
    playback_note: Option<String>,
}

impl Default for Clip {
    fn default() -> Self {
        Self {
            source: PathBuf::new(),
            start: time_fields(0.0),
            end: ["00".into(), "00".into(), String::new()],
            mode: 1,
            position: 0.0,
            duration: None,
            preview: None,
            frame_position: 0.0,
            note: "Choose a video to preview".into(),
            generation: 0,
            pending: None,
            busy: false,
            playing: None,
            playback_note: None,
        }
    }
}

impl Clip {
    pub(super) fn reset(&mut self) {
        *self = Self {
            generation: self.generation.wrapping_add(1),
            busy: self.busy,
            ..Self::default()
        };
    }

    fn range(&self) -> Result<(f64, f64)> {
        let start = model::parse_time(&self.start.join(":")).context("Enter a valid start time")?;
        let end = model::parse_time(&self.end.join(":")).context("Enter a valid end time")?;
        ensure!(end > start, "End must be after Start");
        ensure!(
            self.duration.is_none_or(|duration| end <= duration + 0.001),
            "End exceeds the video duration"
        );
        Ok((start, end))
    }

    fn seek(&mut self, position: f64) {
        self.playing = None;
        self.playback_note = None;
        self.position = position.clamp(0.0, self.duration.unwrap_or(0.0));
        self.generation = self.generation.wrapping_add(1);
        self.pending = Some(Instant::now());
    }

    fn trim(&mut self, marker: Marker, position: f64, duration: f64) {
        // The fields and export use millisecond precision, including at the end of the file.
        let duration = (duration * 1000.0).floor() / 1000.0;
        let position = (position * 1000.0).round() / 1000.0;
        let start = model::parse_time(&self.start.join(":")).unwrap_or(0.0);
        let end = model::parse_time(&self.end.join(":")).unwrap_or(duration);
        let position = match marker {
            Marker::Start => {
                let value = position.clamp(0.0, (end.min(duration) - 0.001).max(0.0));
                self.start = time_fields(value);
                value
            }
            Marker::End => {
                let value = position.clamp((start + 0.001).min(duration), duration);
                self.end = time_fields(value);
                value
            }
        };
        self.seek(position);
    }

    fn toggle_play(&mut self, start: f64, end: f64, ctx: &egui::Context) {
        if self.playing.is_some() {
            self.playing = None;
            self.generation = self.generation.wrapping_add(1);
            self.pending = None;
            self.playback_note = None;
            if self.preview.is_some() {
                self.position = self.frame_position;
            }
        } else {
            let position = if (start..end).contains(&self.position) {
                self.position
            } else {
                start
            };
            self.position = position;
            self.generation = self.generation.wrapping_add(1);
            self.pending = None;
            self.playback_note = Some("Starting preview…".into());
            self.playing = Some(Playback::start(
                self.source.clone(),
                position,
                end,
                ctx.clone(),
            ));
        }
    }
}

impl App {
    pub(super) fn load_cut_source(&mut self, path: PathBuf) {
        self.clip.reset();
        self.clip.source = path;
        self.clip.note = "Loading preview...".into();
        self.clip.seek(0.0);
    }

    pub(super) fn cut_ui(&mut self, ui: &mut egui::Ui) {
        let width = ui.max_rect().width();
        let height = ui.max_rect().height();
        let rail_width = width - 48.0;
        pixel_heading(
            ui,
            rect(24.0, 10.0, rail_width, 40.0),
            "Cut Clip",
            "cut",
            22.0,
        );

        let source = rect(24.0, 58.0, width - 236.0, 40.0);
        pixel_frame(ui.painter(), source, SURFACE, BORDER);
        let filename = self.clip.source.file_name().and_then(|name| name.to_str());
        label_at(
            ui,
            source.shrink(8.0),
            filename.unwrap_or("No video selected"),
            MUTED,
            13.3,
            false,
            false,
        );
        if let Some(path) = self.clip.source.to_str().filter(|path| !path.is_empty()) {
            ui.interact(source, ui.id().with("clip-source"), Sense::hover())
                .on_hover_text(path);
        }
        if pixel_button(
            ui,
            rect(width - 204.0, 58.0, 180.0, 40.0),
            "Choose video",
            "folder",
            SURFACE_RAISED,
            true,
            13.3,
        )
        .clicked()
            && let Some(path) = video_dialog()
                .set_title("Choose a video file to cut")
                .pick_file()
        {
            self.load_cut_source(path);
        }

        let preview = rect(24.0, 110.0, rail_width, height - 368.0);
        pixel_frame(ui.painter(), preview, INPUT, BORDER);
        if let Some(image) = &self.clip.preview {
            let size = image.size_vec2();
            let scale = ((preview.width() - 4.0) / size.x).min((preview.height() - 4.0) / size.y);
            egui::Image::new(image)
                .paint_at(ui, Rect::from_center_size(preview.center(), size * scale));
        } else {
            label_at(
                ui,
                preview.shrink(8.0),
                &self.clip.note,
                MUTED,
                13.3,
                false,
                true,
            );
        }

        self.clip_transport(ui, width, height - 246.0);
        self.clip_timeline(ui, rect(24.0, height - 200.0, rail_width, 63.0));

        let field_y = height - 128.0;
        let start_fields = rect(24.0, field_y, 178.0, 50.0);
        let end_fields = rect(width / 2.0, field_y, 178.0, 50.0);
        let start_changed = edit_time(ui, start_fields, "Start", MINT, &mut self.clip.start);
        let end_changed = edit_time(ui, end_fields, "End", AMBER, &mut self.clip.end);
        if (start_changed || end_changed) && self.clip.playing.is_some() {
            self.clip.seek(self.clip.position);
        }
        for (marker, label, left) in [
            (Marker::Start, "Set start here", start_fields.right() + 9.0),
            (Marker::End, "Set end here", end_fields.right() + 9.0),
        ] {
            if pixel_button(
                ui,
                rect(left, field_y + 12.0, 137.0, 34.0),
                label,
                "",
                SURFACE_RAISED,
                self.clip.duration.is_some(),
                12.0,
            )
            .clicked()
                && let Some(duration) = self.clip.duration
            {
                self.clip.trim(marker, self.clip.position, duration);
            }
        }

        let range = self.clip.range();
        let (summary, color) = match &range {
            Ok((start, end)) => (format!("Selected: {}", display_time(end - start)), INFO),
            Err(error)
                if !self.clip.source.as_os_str().is_empty() && self.clip.duration.is_some() =>
            {
                (error.to_string(), AMBER)
            }
            _ => (
                "Drag the Start and End handles, or enter the times below.".into(),
                MUTED,
            ),
        };
        label_at(
            ui,
            rect(24.0, height - 151.0, rail_width - 320.0, 22.0),
            &summary,
            color,
            12.0,
            false,
            false,
        );
        if self.clip.duration.is_some() {
            label_at(
                ui,
                rect(width - 330.0, height - 151.0, 306.0, 22.0),
                "Drag handles  |  Click track to seek",
                MUTED,
                12.0,
                false,
                false,
            );
        }

        let bottom = height - 59.0;
        combo_at(
            ui,
            "cut-mode",
            rect(24.0, bottom, width - 363.0, 40.0),
            &mut self.clip.mode,
            &media::CUT_MODES,
            true,
        );
        if pixel_button(
            ui,
            rect(width - 329.0, bottom, 176.0, 40.0),
            "Save MP4",
            "save",
            TEAL,
            range.is_ok() && self.clip.source.is_file(),
            13.3,
        )
        .clicked()
            && let Ok((start, end)) = range
        {
            self.clip.seek(self.clip.position);
            if let Err(error) = self.save_clip(start, end) {
                self.error(error);
            }
        }
        if pixel_button(
            ui,
            rect(width - 143.0, bottom, 119.0, 40.0),
            "Cancel",
            "",
            SURFACE_RAISED,
            true,
            13.3,
        )
        .clicked()
        {
            self.cut_open = false;
        }
    }

    fn clip_transport(&mut self, ui: &mut egui::Ui, width: f32, y: f32) {
        let playable = self.clip.duration.is_some() && self.clip.range().is_ok();
        let (shortcut, seek_seconds) = if self.clip.duration.is_some() {
            transport_shortcuts(ui)
        } else {
            (false, 0.0)
        };
        if seek_seconds != 0.0 {
            self.clip.seek(self.clip.position + seek_seconds);
        }
        if shortcut && let Ok((start, end)) = self.clip.range() {
            self.clip.toggle_play(start, end, ui.ctx());
        }
        let (label, icon) = if self.clip.playing.is_some() {
            ("Pause", "pause")
        } else {
            ("Play", "play")
        };
        if pixel_button(
            ui,
            rect(24.0, y, 112.0, 36.0),
            label,
            icon,
            GREEN,
            playable,
            13.3,
        )
        .on_hover_text("Preview the selected range (Space)")
        .clicked()
            && !shortcut
            && let Ok((start, end)) = self.clip.range()
        {
            self.clip.toggle_play(start, end, ui.ctx());
        }
        for (x, label, icon, seconds, hint) in [
            (144.0, "−1s", "left", -1.0, "Back one second (Left Arrow)"),
            (
                213.0,
                "+1s",
                "right",
                1.0,
                "Forward one second (Right Arrow)",
            ),
        ] {
            if pixel_button(
                ui,
                rect(x, y, 61.0, 36.0),
                label,
                icon,
                SURFACE_RAISED,
                self.clip.duration.is_some(),
                12.0,
            )
            .on_hover_text(hint)
            .clicked()
            {
                self.clip.seek(self.clip.position + seconds);
            }
        }
        let duration = self
            .clip
            .duration
            .map(transport_time)
            .unwrap_or_else(|| "--:--:--".into());
        label_at(
            ui,
            rect(290.0, y, width - 490.0, 36.0),
            &format!(
                "{} / {duration}{}",
                transport_time(self.clip.position),
                self.clip
                    .playback_note
                    .as_ref()
                    .map(|note| format!("  |  {note}"))
                    .unwrap_or_default()
            ),
            TEXT,
            13.3,
            true,
            false,
        );
        label_at(
            ui,
            rect(width - 196.0, y, 172.0, 36.0),
            "Space: Play / Pause",
            MUTED,
            12.0,
            false,
            true,
        );
    }

    fn clip_timeline(&mut self, ui: &mut egui::Ui, bounds: Rect) {
        let rail = rect(
            bounds.left() + 13.0,
            bounds.top() + 26.0,
            bounds.width() - 26.0,
            22.0,
        );
        pixel_frame(ui.painter(), rail, SURFACE, BORDER);
        let Some(duration) = self.clip.duration else {
            return;
        };
        let start = model::parse_time(&self.clip.start.join(":")).unwrap_or(0.0);
        let end = model::parse_time(&self.clip.end.join(":")).unwrap_or(duration);
        let x_at =
            |time: f64| rail.left() + rail.width() * (time / duration).clamp(0.0, 1.0) as f32;
        let time_at =
            |x: f32| duration * f64::from(((x - rail.left()) / rail.width()).clamp(0.0, 1.0));

        if end > start {
            ui.painter().rect_filled(
                Rect::from_min_max(
                    Pos2::new(x_at(start), rail.top()),
                    Pos2::new(x_at(end), rail.bottom()),
                ),
                0.0,
                TEAL,
            );
        }
        for tick in 0..=4 {
            let seconds = duration * f64::from(tick) / 4.0;
            let x = x_at(seconds);
            let label_x = (x - 51.0).clamp(bounds.left(), bounds.right() - 102.0);
            label_at(
                ui,
                rect(label_x, bounds.top(), 102.0, 19.0),
                &display_time(seconds),
                MUTED,
                11.0,
                false,
                true,
            );
            ui.painter().vline(
                x,
                rail.top() - 5.0..=rail.top(),
                Stroke::new(1.0_f32, BORDER),
            );
        }

        let seek = timeline_input(
            ui,
            "clip-seek",
            "Preview position",
            rail,
            self.clip.position,
            time_at,
        );
        if let Some(position) = seek {
            self.clip.seek(position);
        }
        for (marker, id, label, time, color, offset) in [
            (
                Marker::Start,
                "clip-start",
                "Trim start",
                start,
                MINT,
                -12.0,
            ),
            (Marker::End, "clip-end", "Trim end", end, AMBER, 0.0),
        ] {
            let x = x_at(time);
            let handle = rect(x + offset, rail.top() - 6.0, 12.0, rail.height() + 12.0);
            ui.painter().rect_filled(handle, 0.0, color);
            ui.painter().vline(
                handle.center().x,
                rail.top() + 5.0..=rail.bottom() - 5.0,
                Stroke::new(1.0_f32, BG),
            );
            if let Some(position) = timeline_input(ui, id, label, handle, time, |pointer| {
                time_at(pointer - offset - 6.0)
            }) {
                self.clip.trim(marker, position, duration);
            }
        }
        let x = x_at(self.clip.position);
        ui.painter().vline(
            x,
            rail.top()..=rail.bottom() + 4.0,
            Stroke::new(2.0_f32, TEXT),
        );
    }

    fn save_clip(&mut self, start: f64, end: f64) -> Result<()> {
        let source = &self.clip.source;
        let filename = format!(
            "{}_clip_{}-{}.mp4",
            source.file_stem().unwrap_or_default().to_string_lossy(),
            display_time(start).replace(':', "-"),
            display_time(end).replace(':', "-"),
        );
        let Some(output) = rfd::FileDialog::new()
            .set_title("Save clipped video as")
            .set_directory(source.parent().unwrap_or(Path::new(".")))
            .add_filter("MP4", &["mp4"])
            .set_file_name(filename)
            .save_file()
        else {
            return Ok(());
        };
        let arguments = media::cut(source, &output, start, end, self.clip.mode == 1)?;
        self.add_edit(
            "Cutting clip",
            &output,
            arguments,
            Default::default(),
            vec![source.clone()],
        )?;
        self.cut_open = false;
        Ok(())
    }

    pub(super) fn clip_preview_received(
        &mut self,
        ctx: &egui::Context,
        generation: u64,
        duration: Option<f64>,
        position: f64,
        frame: Result<Vec<u8>, String>,
    ) {
        self.clip.busy = false;
        if generation != self.clip.generation || !self.cut_open {
            return;
        }
        if let Some(duration) = duration {
            self.clip.duration = Some(duration);
            if self.clip.end[2].is_empty() {
                self.clip.end = time_fields((duration * 1000.0).floor() / 1000.0);
            }
        }
        match frame {
            Ok(bytes) => {
                self.clip.frame_position = position;
                self.clip.preview =
                    texture(ctx, "clip-preview", &bytes, egui::TextureOptions::LINEAR).ok();
                self.clip.note = "Preview unavailable. Enter the times manually.".into();
            }
            Err(error) => {
                self.clip.playing = None;
                self.clip.preview = None;
                self.clip.note = format!("Preview unavailable: {error}");
            }
        }
    }

    pub(super) fn tick_clip(&mut self, ctx: &egui::Context) {
        if !self.cut_open {
            self.clip.playing = None;
            self.clip.pending = None;
            return;
        }
        if let Some(playing) = &self.clip.playing {
            let updates = playing.take_updates();
            if let Some(sound) = updates.sound {
                self.clip.playback_note = Some(sound.into());
            }
            if let Some((position, image)) = updates.frame {
                self.clip.position = position;
                self.clip.frame_position = position;
                let options = egui::TextureOptions::LINEAR;
                if let Some(texture) = &mut self.clip.preview {
                    texture.set(image, options);
                } else {
                    self.clip.preview = Some(ctx.load_texture("clip-playback", image, options));
                }
            }
            if let Some(finished) = updates.finished {
                self.clip.playing = None;
                match finished {
                    Ok(end) => {
                        self.clip.position = end;
                        self.clip.playback_note = None;
                    }
                    Err(error) => self.clip.playback_note = Some(error),
                }
            }
        }
        if self.clip.pending.is_some() {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
        if self.clip.busy
            || !self
                .clip
                .pending
                .is_some_and(|pending| pending.elapsed() >= Duration::from_millis(90))
        {
            return;
        }
        self.clip.pending = None;
        self.clip.busy = true;
        let sender = self.sender.clone();
        let ctx = ctx.clone();
        let path = self.clip.source.clone();
        let position = self.clip.position;
        let generation = self.clip.generation;
        let duration = self.clip.duration;
        thread::spawn(move || {
            let duration = duration.map_or_else(|| media::duration(&path), Ok);
            let frame = duration
                .as_ref()
                .map_err(ToString::to_string)
                .and_then(|duration| {
                    media::preview(&path, position, *duration).map_err(|error| error.to_string())
                });
            let _ = sender.send(UiEvent::Preview(generation, duration.ok(), position, frame));
            ctx.request_repaint();
        });
    }
}

fn transport_shortcuts(ui: &egui::Ui) -> (bool, f64) {
    let editing_time = ui.memory(|memory| {
        ["Start", "End"]
            .into_iter()
            .any(|name| (0..3).any(|index| memory.has_focus(time_id(ui, name, index))))
    });
    if editing_time || egui::Popup::is_any_open(ui.ctx()) {
        return (false, 0.0);
    }
    ui.input_mut(|input| {
        let pressed = input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: Key::Space,
                    modifiers: egui::Modifiers::NONE,
                    pressed: true,
                    repeat: false,
                    ..
                }
            )
        });
        input.consume_key(egui::Modifiers::NONE, Key::Space);
        let forward = input.count_and_consume_key(egui::Modifiers::NONE, Key::ArrowRight);
        let backward = input.count_and_consume_key(egui::Modifiers::NONE, Key::ArrowLeft);
        (pressed, forward as f64 - backward as f64)
    })
}

fn time_id(ui: &egui::Ui, name: &str, index: usize) -> egui::Id {
    ui.id().with(("clip-time", name, index))
}

fn timeline_input(
    ui: &mut egui::Ui,
    id: &str,
    name: &str,
    rect: Rect,
    time: f64,
    time_at: impl Fn(f32) -> f64,
) -> Option<f64> {
    let response = ui
        .interact(rect, ui.id().with(id), Sense::click_and_drag())
        .on_hover_cursor(CursorIcon::ResizeHorizontal)
        .on_hover_text(format!("{name}: {}", display_time(time)));
    response.widget_info(|| egui::WidgetInfo::slider(true, time, name));
    if (response.dragged() || response.clicked())
        && let Some(pointer) = response.interact_pointer_pos()
    {
        response.request_focus();
        return Some(time_at(pointer.x));
    }
    None
}

fn transport_time(seconds: f64) -> String {
    model::timecode(seconds.floor())
        .trim_end_matches(".000")
        .into()
}

fn display_time(seconds: f64) -> String {
    model::timecode(seconds)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .into()
}

fn time_fields(seconds: f64) -> [String; 3] {
    let time = display_time(seconds);
    let mut fields = time.split(':');
    std::array::from_fn(|_| fields.next().unwrap_or("00").into())
}

fn edit_time(
    ui: &mut egui::Ui,
    group: Rect,
    name: &str,
    color: Color32,
    values: &mut [String; 3],
) -> bool {
    label_at(
        ui,
        rect(group.left(), group.top(), group.width(), 18.0),
        &format!("{name}  (HH:MM:SS)"),
        color,
        12.0,
        true,
        false,
    );
    let mut changed = false;
    for (index, value) in values.iter_mut().enumerate() {
        let x = group.left() + index as f32 * 54.0;
        let field = rect(
            x,
            group.top() + 23.0,
            if index == 2 { 70.0 } else { 40.0 },
            26.0,
        );
        let response = ui.put(
            field,
            egui::TextEdit::singleline(value)
                .id(time_id(ui, name, index))
                .horizontal_align(egui::Align::Center),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::TextEdit,
                true,
                format!("{name} {}", ["hours", "minutes", "seconds"][index]),
            )
        });
        changed |= response.changed();
        if index < 2 {
            label_at(
                ui,
                rect(field.right(), field.top(), 14.0, field.height()),
                ":",
                MUTED,
                13.3,
                true,
                true,
            );
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_keys_are_scoped_to_the_editor_and_skip_time_fields() {
        for (key, action) in [
            (Key::Space, (true, 0.0)),
            (Key::ArrowLeft, (false, -1.0)),
            (Key::ArrowRight, (false, 1.0)),
        ] {
            for editing in [false, true] {
                let ctx = egui::Context::default();
                let _ = ctx.run(
                    egui::RawInput {
                        events: vec![egui::Event::Key {
                            key,
                            physical_key: Some(key),
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        }],
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            if editing {
                                let id = time_id(ui, "Start", 2);
                                ui.memory_mut(|memory| memory.request_focus(id));
                            }
                            assert_eq!(
                                transport_shortcuts(ui),
                                if editing { (false, 0.0) } else { action }
                            );
                            assert_eq!(ui.input(|input| input.key_pressed(key)), editing);
                        });
                    },
                );
            }
        }
    }

    #[test]
    fn editor_arrows_seek_one_second_within_the_clip() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            crate::jobs::Engine::new(dir.path()).unwrap(),
            crate::model::Settings {
                output_directory: dir.path().into(),
                ..Default::default()
            },
        );
        app.clip.duration = Some(8.0);
        app.clip.start = time_fields(2.0);
        app.clip.end = time_fields(6.0);
        for (position, key, expected) in [
            (2.25, Key::ArrowRight, 3.25),
            (3.25, Key::ArrowLeft, 2.25),
            (0.25, Key::ArrowLeft, 0.0),
            (7.75, Key::ArrowRight, 8.0),
        ] {
            app.clip.position = position;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(rect(0.0, 0.0, 960.0, 760.0)),
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: Some(key),
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.cut_ui(ui));
                },
            );
            assert_eq!(app.clip.position, expected);
            assert_eq!(app.clip.range().unwrap(), (2.0, 6.0));
        }
    }

    #[test]
    fn trim_handles_keep_a_valid_range() {
        let mut clip = Clip {
            duration: Some(4.5126),
            end: time_fields(4.512),
            ..Clip::default()
        };
        clip.trim(Marker::Start, 2.75, 4.5126);
        assert_eq!(clip.range().unwrap(), (2.75, 4.512));
        assert_eq!(clip.position, 2.75);
        assert!(clip.pending.is_some());
        clip.trim(Marker::End, 1.0, 4.5126);
        assert_eq!(clip.range().unwrap(), (2.75, 2.751));
        clip.trim(Marker::End, 4.5126, 4.5126);
        assert_eq!(clip.range().unwrap(), (2.75, 4.512));
        clip.trim(Marker::Start, 10.0, 4.5126);
        assert_eq!(clip.range().unwrap(), (4.511, 4.512));
    }
}

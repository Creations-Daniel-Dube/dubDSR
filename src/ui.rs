use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, ColorImage, Context, DragValue, Frame, Key, Layout, Rect, RichText, ScrollArea, Sense, Stroke, TextureHandle, TextureOptions, Vec2, pos2, vec2,
};
use rfd::FileDialog;

use crate::error::PipelineError;
use crate::gpu::{probe_gpu, GpuInfo, ProbeError};
use crate::i18n::{cycle_language, Catalog};
use crate::pipeline::avi::AviVideo;
use crate::pipeline::rawset::{PreviewImage, RawFolder};
use crate::pipeline::{stack_video, StackResult, StackSettings};
use crate::settings::{AppSettings, ProfileSettings};

enum Status {
    Ready,
    Busy,
    Stage { key: String, args: Vec<String> },
    Done {
        kept: usize,
        total: usize,
        bayer: Option<String>,
        path: String,
    },
    Failed,
}

enum Msg {
    Progress { key: String, args: Vec<String>, value: f32 },
    Done(StackResult),
    Fail(PipelineError),
}

enum Source {
    Avi(AviVideo),
    Raw(RawFolder),
}

pub struct DubApp {
    catalog: Catalog,
    settings: AppSettings,
    path: String,
    section: bool,
    range_start: usize,
    range_end: usize,
    video: Option<Source>,
    video_frames: usize,
    video_fps: f64,
    gpu: Result<GpuInfo, ProbeError>,
    warn_open: bool,
    missing_open: bool,
    error_open: bool,
    error_body: String,
    status: Status,
    progress: f32,
    started: Option<Instant>,
    frozen_time: String,
    job: Option<Receiver<Msg>>,
    preview: Option<TextureHandle>,
    preview_size: Vec2,
    pending_preview: Option<(usize, Instant, bool)>,
    content_height: f32,
    fit_frames: u8,
    window_fitted: bool,
    help_open: bool,
    help_shot: Option<TextureHandle>,
}

impl DubApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(theme());
        install_stack_font(&cc.egui_ctx);
        let settings = AppSettings::load();
        let path = if settings.active == "moon" {
            settings.moon.last_file.clone()
        } else {
            settings.sun.last_file.clone()
        };
        let gpu = probe_gpu();
        let warn_open = gpu.is_err();
        let mut app = Self {
            catalog: Catalog::load(&settings.language),
            settings,
            path,
            section: false,
            range_start: 0,
            range_end: 0,
            video: None,
            video_frames: 0,
            video_fps: 0.0,
            gpu,
            warn_open,
            missing_open: false,
            error_open: false,
            error_body: String::new(),
            status: Status::Ready,
            progress: 0.0,
            started: None,
            frozen_time: String::new(),
            job: None,
            preview: None,
            preview_size: Vec2::ZERO,
            pending_preview: None,
            content_height: 0.0,
            fit_frames: 0,
            window_fitted: false,
            help_open: false,
            help_shot: None,
        };
        app.set_title(&cc.egui_ctx);
        app.probe_video();
        app
    }

    fn t(&self, key: &str) -> String {
        self.catalog.text(key)
    }

    fn profile(&self) -> &ProfileSettings {
        if self.settings.active == "moon" { &self.settings.moon } else { &self.settings.sun }
    }

    fn profile_mut(&mut self) -> &mut ProfileSettings {
        if self.settings.active == "moon" { &mut self.settings.moon } else { &mut self.settings.sun }
    }

    fn save(&mut self) {
        self.settings.save();
    }

    fn set_title(&self, ctx: &Context) {
        let title = self.catalog.fmt("window.title", &[env!("CARGO_PKG_VERSION")]);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }

    fn gpu_line(&self) -> String {
        let detail = match &self.gpu {
            Ok(info) => self.catalog.fmt(
                "gpu.available",
                &[&info.name, &format!("{}.{}", info.major, info.minor), info.kernels, &info.gib],
            ),
            Err(ProbeError::Cuda(err)) => self.catalog.fmt("gpu.unavailable", &[err]),
            Err(ProbeError::Unsupported { major, minor }) => self.catalog.fmt("gpu.unsupported", &[&format!("{major}.{minor}")]),
        };
        self.catalog.fmt("gpu.prefix", &[&detail])
    }

    fn sync_active_path(&mut self) {
        let path = self.path.clone();
        let path_buf = PathBuf::from(&path);
        let dir = if path_buf.is_dir() {
            path.clone()
        } else {
            path_buf.parent().map(|parent| parent.display().to_string()).unwrap_or_default()
        };
        let profile = self.profile_mut();
        profile.last_file = path;
        if !dir.is_empty() {
            profile.last_dir = dir;
        }
    }

    fn set_target(&mut self, active: &str) {
        if self.settings.active == active {
            return;
        }
        self.sync_active_path();
        self.settings.active = if active == "moon" { "moon".to_string() } else { "sun".to_string() };
        self.path = self.profile().last_file.clone();
        self.probe_video();
        self.save();
    }

    fn probe_video(&mut self) {
        let path = PathBuf::from(self.path.trim());
        if path.is_dir() {
            if let Ok(folder) = RawFolder::open(&path) {
                if !folder.files.is_empty() {
                    let last = folder.files.len().saturating_sub(1);
                    self.video_frames = folder.files.len();
                    self.video_fps = 0.0;
                    self.video = Some(Source::Raw(folder));
                    self.range_start = 0;
                    self.range_end = last;
                    let middle = (self.video_frames / 2).min(last);
                    self.pending_preview = Some((middle, Instant::now(), true));
                    return;
                }
            }
            self.clear_source();
            return;
        }
        match AviVideo::open(&path) {
            Ok(video) => {
                let probe = video.probe();
                self.video_frames = probe.frame_count;
                self.video_fps = probe.fps;
                self.video = Some(Source::Avi(video));
                let last = probe.frame_count.saturating_sub(1);
                self.range_start = 0;
                self.range_end = last;
                let middle = (probe.frame_count / 2).min(last);
                self.pending_preview = Some((middle, Instant::now(), true));
            }
            Err(_) => self.clear_source(),
        }
    }

    fn clear_source(&mut self) {
        self.video = None;
        self.video_frames = 0;
        self.video_fps = 0.0;
        self.section = false;
        self.range_start = 0;
        self.range_end = 0;
        self.preview = None;
        self.preview_size = Vec2::ZERO;
        self.pending_preview = None;
    }

    fn handle_language(&mut self, ctx: &Context) {
        let cycle = ctx.input(|input| input.key_pressed(Key::L) && input.modifiers.command);
        if cycle {
            self.settings.language = cycle_language(&self.settings.language).to_string();
            self.catalog = Catalog::load(&self.settings.language);
            self.set_title(ctx);
            self.save();
        }
    }

    fn poll(&mut self, ctx: &Context) {
        let Some(rx) = self.job.take() else {
            return;
        };
        let mut restore = true;
        loop {
            match rx.try_recv() {
                Ok(Msg::Progress { key, args, value }) => {
                    self.progress = value;
                    self.status = Status::Stage { key, args };
                }
                Ok(Msg::Done(result)) => {
                    self.finish_ok(ctx, result);
                    restore = false;
                    break;
                }
                Ok(Msg::Fail(err)) => {
                    self.finish_err(err);
                    restore = false;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.finish_err(PipelineError::new("error.gpu"));
                    restore = false;
                    break;
                }
            }
        }
        if restore {
            self.job = Some(rx);
        }
    }

    fn finish_ok(&mut self, ctx: &Context, result: StackResult) {
        let elapsed = self.stop_timer();
        self.frozen_time = self.catalog.fmt("total", &[&self.format_duration(elapsed)]);
        self.progress = 1.0;
        self.status = Status::Done {
            kept: result.kept_frames,
            total: result.frame_count,
            bayer: result.bayer_pattern.clone(),
            path: result.output_path.display().to_string(),
        };
        self.show_result(ctx, &result);
    }

    fn finish_err(&mut self, err: PipelineError) {
        let elapsed = self.stop_timer();
        self.frozen_time = self.catalog.fmt("stopped", &[&self.format_duration(elapsed)]);
        self.status = Status::Failed;
        let args: Vec<&str> = err.args.iter().map(String::as_str).collect();
        self.error_body = self.catalog.fmt(err.key, &args);
        self.error_open = true;
    }

    fn stop_timer(&mut self) -> f64 {
        let elapsed = self.started.take().map(|start| start.elapsed().as_secs_f64()).unwrap_or(0.0);
        elapsed
    }

    fn flush_preview(&mut self, ctx: &Context) {
        let Some((index, since, immediate)) = self.pending_preview else {
            return;
        };
        if immediate || since.elapsed() >= Duration::from_millis(40) {
            self.pending_preview = None;
            self.show_source(ctx, index);
        }
    }

    fn show_source(&mut self, ctx: &Context, index: usize) {
        if self.job.is_some() {
            return;
        }
        let Some(image) = self.preview_image(index) else {
            return;
        };
        let (width, height, texture) = match image {
            PreviewImage::Gray { width, height, pixels } => {
                let image = ColorImage::from_gray([width, height], &pixels);
                (width, height, image)
            }
            PreviewImage::Rgb { width, height, pixels } => {
                let image = ColorImage::from_rgb([width, height], &pixels);
                (width, height, image)
            }
        };
        self.preview = Some(ctx.load_texture("preview", texture, TextureOptions::LINEAR));
        self.preview_size = vec2(width as f32, height as f32);
    }

    fn preview_image(&self, index: usize) -> Option<PreviewImage> {
        match self.video.as_ref()? {
            Source::Avi(video) => {
                let frame = video.read_frame(index).ok()?;
                let probe = video.probe();
                let width = probe.width as usize;
                let height = probe.height as usize;
                if frame.len() != width * height || width == 0 || height == 0 {
                    return None;
                }
                let peak = frame.iter().copied().fold(0.0f32, f32::max).max(1.0);
                let pixels: Vec<u8> = frame.iter().map(|value| ((value / peak) * 255.0).clamp(0.0, 255.0) as u8).collect();
                Some(PreviewImage::Gray { width, height, pixels })
            }
            Source::Raw(folder) => folder.preview(index).ok(),
        }
    }

    fn show_result(&mut self, ctx: &Context, result: &StackResult) {
        if result.width == 0 || result.height == 0 {
            return;
        }
        let bytes: Vec<u8> = result.pixels.iter().map(|value| (value / 257) as u8).collect();
        let expected = result.width * result.height * if result.rgb { 3 } else { 1 };
        if bytes.len() != expected {
            return;
        }
        let image = if result.rgb {
            ColorImage::from_rgb([result.width, result.height], &bytes)
        } else {
            ColorImage::from_gray([result.width, result.height], &bytes)
        };
        self.preview = Some(ctx.load_texture("preview", image, TextureOptions::LINEAR));
        self.preview_size = vec2(result.width as f32, result.height as f32);
    }

    fn format_duration(&self, seconds: f64) -> String {
        if seconds >= 60.0 {
            let minutes = (seconds / 60.0) as u64;
            let secs = seconds - minutes as f64 * 60.0;
            self.catalog.fmt("duration.min", &[&minutes.to_string(), &format!("{secs:04.1}")])
        } else {
            self.catalog.fmt("duration.s", &[&format!("{seconds:.1}")])
        }
    }

    fn time_label(&self) -> String {
        if let Some(start) = self.started {
            self.catalog.fmt("elapsed", &[&self.format_duration(start.elapsed().as_secs_f64())])
        } else {
            self.frozen_time.clone()
        }
    }

    fn status_text(&self) -> String {
        match &self.status {
            Status::Ready => self.t("ready"),
            Status::Busy => self.t("processing"),
            Status::Stage { key, args } => {
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                self.catalog.fmt(key, &refs)
            }
            Status::Done { kept, total, bayer, path } => {
                let info = bayer.as_ref().map(|name| self.catalog.fmt("bayer.info", &[name])).unwrap_or_default();
                self.catalog.fmt("done", &[&kept.to_string(), &total.to_string(), &info, path])
            }
            Status::Failed => self.t("error"),
        }
    }

    fn timecode(&self, frame: usize) -> String {
        if let Some(Source::Raw(folder)) = &self.video {
            return folder
                .files
                .get(frame)
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| frame.to_string());
        }
        if self.video_fps <= 0.0 {
            return frame.to_string();
        }
        let seconds = frame as f64 / self.video_fps;
        let minutes = (seconds / 60.0) as u64;
        let secs = seconds - minutes as f64 * 60.0;
        format!("{minutes}:{secs:05.2}")
    }

    fn browse(&mut self) {
        let title = self.t("file.dialog");
        let filter = self.t("file.filter");
        let dir = self.profile().last_dir.clone();
        let mut dialog = FileDialog::new().set_title(title).add_filter(filter, &["avi", "AVI"]);
        if !dir.is_empty() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.path = path.display().to_string();
            self.sync_active_path();
            self.probe_video();
            self.save();
        }
    }

    fn browse_folder(&mut self) {
        let title = self.t("file.folder_dialog");
        let dir = self.profile().last_dir.clone();
        let mut dialog = FileDialog::new().set_title(title);
        if !dir.is_empty() {
            dialog = dialog.set_directory(&dir);
        }
        if let Some(path) = dialog.pick_folder() {
            self.path = path.display().to_string();
            self.sync_active_path();
            self.probe_video();
            self.save();
        }
    }

    fn start_stack(&mut self) {
        let path = PathBuf::from(self.path.trim());
        if !path.is_file() && !path.is_dir() {
            self.missing_open = true;
            return;
        }
        self.sync_active_path();
        self.save();
        let settings = self.stack_settings();
        let (tx, rx) = mpsc::channel();
        self.job = Some(rx);
        self.started = Some(Instant::now());
        self.frozen_time.clear();
        self.progress = 0.0;
        self.status = match &self.gpu {
            Ok(info) => Status::Stage {
                key: "gpu.available".to_string(),
                args: vec![info.name.clone(), format!("{}.{}", info.major, info.minor), info.kernels.to_string(), info.gib.clone()],
            },
            Err(_) => Status::Busy,
        };
        thread::spawn(move || {
            let outcome = stack_video(&path, &settings, |stage, args, value| {
                let _ = tx.send(Msg::Progress {
                    key: stage.to_string(),
                    args: args.iter().map(|value| (*value).to_string()).collect(),
                    value,
                });
            });
            match outcome {
                Ok(result) => {
                    let _ = tx.send(Msg::Done(result));
                }
                Err(err) => {
                    let _ = tx.send(Msg::Fail(err));
                }
            }
        });
    }

    fn stack_settings(&self) -> StackSettings {
        let profile = self.profile();
        let section = self.section && self.video_frames > 0;
        StackSettings {
            keep_ratio: profile.keep_ratio / 100.0,
            debayer: profile.debayer,
            white_balance: profile.white_balance,
            auto_crop: profile.auto_crop,
            crop_margin_ratio: profile.crop_margin / 100.0,
            drizzle_scale: if profile.drizzle { 3 } else { 1 },
            bayer_pattern: profile.bayer_pattern.clone(),
            black_point: profile.black_point,
            white_point: if profile.protect_highlights { 100.0 } else { 99.7 },
            gamma: profile.gamma,
            subtract_background: profile.subtract_background,
            solar_stretch: profile.solar_stretch,
            sharpen_amount: profile.sharpen_amount,
            sharpen_radius: profile.sharpen_radius,
            flatten_strength: profile.flatten_strength / 100.0,
            max_frames: profile.max_frames,
            start_frame: if section { self.range_start } else { 0 },
            end_frame: if section { Some(self.range_end) } else { None },
        }
    }

    fn contents(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
            let height = ui.available_height();
            ui.allocate_ui_with_layout(vec2(500.0, height), Layout::top_down(Align::Min), |ui| {
                let output = ScrollArea::vertical()
                    .id_salt("controls")
                    .auto_shrink([false, true])
                    .show(ui, |ui| self.controls(ui));
                self.content_height = output.content_size.y;
            });
            ui.vertical(|ui| self.preview_panel(ui));
        });
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        let gpu = ui.label(self.gpu_line());
        self.hover(gpu, "help.gpu");
        self.target_box(ui);
        self.file_box(ui);
        self.range_box(ui);
        self.settings_box(ui);
        self.tone_box(ui);
        ui.add_space(8.0);
        let running = self.job.is_some();
        let label = RichText::new(self.t("stack"))
            .size(22.0)
            .family(egui::FontFamily::Name("stack-bold".into()));
        let bar_width = ui.available_width();
        let button_width = if bar_width.is_finite() { bar_width } else { 160.0 };
        let clicked = ui
            .with_layout(Layout::top_down(Align::Center), |ui| {
                ui.set_width(button_width);
                let button = ui.add_enabled(!running, egui::Button::new(label).min_size(vec2(button_width, 44.0)));
                let clicked = button.clicked();
                self.hover(button, "help.stack");
                clicked
            })
            .inner;
        if clicked {
            self.start_stack();
        }
        ui.add_space(6.0);
        let progress = ui.add(egui::ProgressBar::new(self.progress).show_percentage().desired_width(button_width));
        self.hover(progress, "help.progress");
        let status = ui.label(self.status_text());
        self.hover(status, "help.status");
        let time = self.time_label();
        if !time.is_empty() {
            let time_label = ui.label(time);
            self.hover(time_label, "help.time");
        }
    }

    fn target_box(&mut self, ui: &mut egui::Ui) {
        let title = self.t("target.box");
        let label = self.t("target.label");
        let sun = self.t("target.sun");
        let moon = self.t("target.moon");
        let mut active = self.settings.active.clone();
        let selected = if active == "moon" { moon.clone() } else { sun.clone() };
        group(ui, &title, |ui| {
            let row = ui.horizontal(|ui| {
                ui.label(label);
                egui::ComboBox::from_id_salt("target").selected_text(selected).show_ui(ui, |ui| {
                    ui.selectable_value(&mut active, "sun".to_string(), sun);
                    ui.selectable_value(&mut active, "moon".to_string(), moon);
                });
            });
            self.hover(row.response, "help.target");
        });
        if active != self.settings.active {
            self.set_target(&active);
        }
    }

    fn file_box(&mut self, ui: &mut egui::Ui) {
        let title = self.t("file.box");
        let hint = self.t("file.hint");
        let browse = self.t("file.browse");
        let folder = self.t("file.folder");
        let mut browse_clicked = false;
        let mut folder_clicked = false;
        group(ui, &title, |ui| {
            ui.horizontal(|ui| {
                let response = ui.add(egui::TextEdit::singleline(&mut self.path).hint_text(hint).desired_width(f32::INFINITY));
                if response.lost_focus() {
                    self.sync_active_path();
                    self.probe_video();
                    self.save();
                }
                self.hover(response, "help.path");
                let browse_button = ui.button(browse);
                if browse_button.clicked() {
                    browse_clicked = true;
                }
                self.hover(browse_button, "help.browse");
                let folder_button = ui.button(folder);
                if folder_button.clicked() {
                    folder_clicked = true;
                }
                self.hover(folder_button, "help.folder");
            });
        });
        if browse_clicked {
            self.browse();
        }
        if folder_clicked {
            self.browse_folder();
        }
    }

    fn range_box(&mut self, ui: &mut egui::Ui) {
        let title = self.t("range.box");
        let full = self.t("range.full");
        let section = self.t("range.section");
        let has_video = self.video_frames > 0;
        let mut section_on = self.section && has_video;
        let before = section_on;
        group(ui, &title, |ui| {
            ui.add_enabled_ui(has_video, |ui| {
                let full_button = ui.radio_value(&mut section_on, false, full);
                self.hover(full_button, "help.full");
                let section_button = ui.radio_value(&mut section_on, true, section);
                self.hover(section_button, "help.section");
            });
            if !has_video {
                ui.label(self.t("range.empty"));
            } else if section_on {
                let last = self.video_frames.saturating_sub(1);
                let (moved, slider) = range_slider(ui, &mut self.range_start, &mut self.range_end, last);
                self.hover(slider, "help.slider");
                let start_code = self.timecode(self.range_start);
                let end_code = self.timecode(self.range_end);
                ui.label(self.catalog.fmt(
                    "range.labels",
                    &[&self.range_start.to_string(), &start_code, &self.range_end.to_string(), &end_code],
                ));
                if let Some((index, released)) = moved {
                    self.pending_preview = Some((index, Instant::now(), released));
                }
            } else {
                ui.label(self.t("range.empty"));
            }
        });
        if section_on && !before {
            let last = self.video_frames.saturating_sub(1);
            self.range_start = 0;
            self.range_end = last;
        }
        self.section = section_on;
    }

    fn settings_box(&mut self, ui: &mut egui::Ui) {
        let title = self.t("settings.box");
        group(ui, &title, |ui| {
            split_options(ui, |left, right| {
                self.drag_f32(left, "keep", |profile| &mut profile.keep_ratio, 10.0..=100.0, 5.0, 0, " %", |value| {
                    ((value / 5.0).round() * 5.0).clamp(10.0, 100.0)
                });
                let limit_label = left.label(self.t("max_frames"));
                self.hover(limit_label, "help.limit");
                let mut limit = self.profile().max_frames;
                let limit_drag = left.add(wide_drag(left, DragValue::new(&mut limit).range(0u32..=100_000)));
                if limit_drag.changed() {
                    self.profile_mut().max_frames = limit;
                    self.save();
                }
                self.hover(limit_drag, "help.limit");
                self.bayer_combo(left);
                self.drag_f32(left, "margin", |profile| &mut profile.crop_margin, 1.0..=30.0, 1.0, 0, " %", |value| value.clamp(1.0, 30.0));

                self.toggle(right, "debayer", |profile| &mut profile.debayer);
                self.toggle(right, "wb", |profile| &mut profile.white_balance);
                self.toggle(right, "crop", |profile| &mut profile.auto_crop);
                self.toggle(right, "drizzle", |profile| &mut profile.drizzle);
            });
        });
    }

    fn tone_box(&mut self, ui: &mut egui::Ui) {
        let title = self.t("tone.box");
        group(ui, &title, |ui| {
            split_options(ui, |left, right| {
                self.drag_f32(left, "flatten", |profile| &mut profile.flatten_strength, 0.0..=100.0, 10.0, 0, " %", |value| value.clamp(0.0, 100.0));
                self.drag_f32(left, "gamma", |profile| &mut profile.gamma, 0.5..=3.0, 0.05, 2, "", |value| value.clamp(0.5, 3.0));
                self.drag_f32(left, "sharpen", |profile| &mut profile.sharpen_amount, 0.0..=6.0, 0.25, 2, "", |value| value.clamp(0.0, 6.0));
                self.drag_f32(left, "radius", |profile| &mut profile.sharpen_radius, 0.5..=8.0, 0.5, 1, "", |value| value.clamp(0.5, 8.0));
                self.drag_f32(left, "solar", |profile| &mut profile.solar_stretch, 0.0..=50.0, 1.0, 0, "", |value| value.clamp(0.0, 50.0));
                self.drag_f32(left, "black", |profile| &mut profile.black_point, 0.0..=20.0, 0.1, 1, " %", |value| value.clamp(0.0, 20.0));

                self.toggle(right, "background", |profile| &mut profile.subtract_background);
                self.toggle(right, "protect", |profile| &mut profile.protect_highlights);
            });
        });
    }

    fn bayer_combo(&mut self, ui: &mut egui::Ui) {
        let label = ui.label(self.t("bayer"));
        self.hover(label, "help.bayer");
        let auto = self.t("bayer.auto");
        let mut pattern = self.profile().bayer_pattern.clone();
        let selected = if pattern == "AUTO" { auto.clone() } else { pattern.clone() };
        let width = ui.available_width();
        let mut combo = egui::ComboBox::from_id_salt("bayer").selected_text(selected);
        if width.is_finite() {
            combo = combo.width(width);
        }
        let shown = combo.show_ui(ui, |ui| {
            ui.selectable_value(&mut pattern, "AUTO".to_string(), auto);
            for name in ["GRBG", "GBRG", "BGGR", "RGGB"] {
                ui.selectable_value(&mut pattern, name.to_string(), name);
            }
        });
        self.hover(shown.response, "help.bayer");
        if pattern != self.profile().bayer_pattern {
            self.profile_mut().bayer_pattern = pattern;
            self.save();
        }
    }

    fn toggle(&mut self, ui: &mut egui::Ui, key: &str, field: impl Fn(&mut ProfileSettings) -> &mut bool) {
        let label = self.t(key);
        let mut value = *field(self.profile_mut());
        let response = ui.checkbox(&mut value, label);
        if response.changed() {
            *field(self.profile_mut()) = value;
            self.save();
        }
        self.hover(response, help_for(key));
    }

    fn drag_f32(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        field: impl Fn(&mut ProfileSettings) -> &mut f32,
        range: std::ops::RangeInclusive<f32>,
        speed: f32,
        decimals: usize,
        suffix: &str,
        snap: impl Fn(f32) -> f32,
    ) {
        let label = ui.label(self.t(key));
        self.hover(label, help_for(key));
        let mut value = *field(self.profile_mut());
        let drag = DragValue::new(&mut value).range(range).speed(speed).fixed_decimals(decimals).suffix(suffix);
        let response = ui.add(wide_drag(ui, drag));
        if response.changed() {
            *field(self.profile_mut()) = snap(value);
            self.save();
        }
        self.hover(response, help_for(key));
    }

    fn hover(&self, response: egui::Response, key: &str) {
        let text = self.t(key);
        let disabled = text.clone();
        response
            .on_hover_ui(|ui| tip_label(ui, &text))
            .on_disabled_hover_ui(|ui| tip_label(ui, &disabled));
    }

    fn help_window(&mut self, ctx: &Context) {
        if !self.help_open {
            return;
        }
        let mut close = false;
        let title = self.t("help.title");
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("dubdsr-help"),
            egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size([900.0, 960.0])
                .with_min_inner_size([520.0, 420.0])
                .with_icon(crate::icon::app_icon()),
            |ui, class| {
                ui.ctx().set_visuals(theme());
                if ui.input(|input| input.viewport().close_requested()) {
                    close = true;
                }
                if class == egui::ViewportClass::EmbeddedWindow {
                    self.help_body(ui);
                } else {
                    egui::CentralPanel::default().show(ui, |ui| self.help_body(ui));
                }
            },
        );
        if close {
            self.help_open = false;
        }
    }

    fn help_body(&mut self, ui: &mut egui::Ui) {
        ui.style_mut().interaction.selectable_labels = true;
        ScrollArea::vertical().id_salt("help-scroll").auto_shrink([false, false]).show(ui, |ui| {
            if let Some(texture) = self.help_texture(ui.ctx()) {
                let size = texture.size_vec2();
                let width = ui.available_width();
                if width > 1.0 && size.x > 1.0 {
                    let draw = vec2(width, width * size.y / size.x);
                    ui.add(egui::Image::from_texture(&texture).fit_to_exact_size(draw));
                }
            }
            ui.add_space(8.0);
            ui.label(self.t("help.intro"));
            ui.add_space(12.0);
            for (number, title_key, body_key) in HELP_ITEMS {
                let title = self.t(title_key);
                let title = title.trim().trim_end_matches(':').trim();
                ui.label(RichText::new(format!("{number}. {title}")).strong());
                ui.label(self.t(body_key));
                ui.add_space(10.0);
            }
        });
    }

    fn help_texture(&mut self, ctx: &Context) -> Option<TextureHandle> {
        if let Some(texture) = &self.help_shot {
            return Some(texture.clone());
        }
        let decoded = image::load_from_memory_with_format(HELP_PNG, image::ImageFormat::Png).ok()?;
        let rgba = decoded.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let color = ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        let texture = ctx.load_texture("help-shot", color, TextureOptions::LINEAR);
        self.help_shot = Some(texture.clone());
        Some(texture)
    }

    fn fit_window(&mut self, ctx: &Context) {
        if self.window_fitted || self.content_height < 200.0 {
            return;
        }
        self.fit_frames = self.fit_frames.saturating_add(1);
        if self.fit_frames < 2 {
            ctx.request_repaint();
            return;
        }
        self.window_fitted = true;
        let margin = egui::Frame::central_panel(&ctx.global_style()).inner_margin.sum().y;
        let height = (self.content_height + margin + 4.0).ceil().max(MIN_WINDOW_HEIGHT);
        let width = ctx.content_rect().width().max(900.0);
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(width, height)));
    }

    fn preview_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let title = ui.label(RichText::new(self.t("preview")).strong());
            self.hover(title, "help.preview");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let button = ui.add(egui::Button::new(RichText::new("?").strong()).min_size(vec2(28.0, 24.0)));
                if button.clicked() {
                    self.help_open = true;
                }
                self.hover(button, "help.open");
            });
        });
        let available = ui.available_size();
        let (rect, response) = ui.allocate_exact_size(available, Sense::hover());
        self.hover(response, "help.preview");
        ui.painter().rect_filled(rect, 4.0, Color32::from_rgb(0x0e, 0x03, 0x03));
        let Some(texture) = &self.preview else {
            return;
        };
        let size = self.preview_size;
        if size.x <= 1.0 || size.y <= 1.0 {
            return;
        }
        let scale = ((rect.width() - 16.0) / size.x).min((rect.height() - 16.0) / size.y).max(0.0);
        let draw = size * scale;
        let origin = rect.center() - draw * 0.5;
        ui.painter().image(
            texture.id(),
            Rect::from_min_size(origin, draw),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    fn dialogs(&mut self, ctx: &Context) {
        let ok = self.t("ok");
        if self.warn_open {
            let title = self.t("preflight.title");
            let detail = match self.gpu.as_ref() {
                Err(ProbeError::Cuda(detail)) => detail.clone(),
                Err(ProbeError::Unsupported { major, minor }) => self.catalog.fmt("gpu.unsupported", &[&format!("{major}.{minor}")]),
                Ok(_) => String::new(),
            };
            let body = self.catalog.fmt("preflight.body", &[&detail]);
            let mut open = self.warn_open;
            modal(ctx, &title, &body, &ok, &mut open);
            self.warn_open = open;
        }
        if self.missing_open {
            let title = self.t("missing.title");
            let body = self.t("missing.body");
            let mut open = self.missing_open;
            modal(ctx, &title, &body, &ok, &mut open);
            self.missing_open = open;
        }
        if self.error_open {
            let title = self.t("fail.title");
            let body = self.error_body.clone();
            let mut open = self.error_open;
            modal(ctx, &title, &body, &ok, &mut open);
            self.error_open = open;
        }
    }
}

impl eframe::App for DubApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_language(&ctx);
        self.poll(&ctx);
        self.flush_preview(&ctx);
        egui::CentralPanel::default().show(ui, |ui| self.contents(ui));
        self.fit_window(&ctx);
        self.dialogs(&ctx);
        self.help_window(&ctx);
        if self.pending_preview.is_some_and(|(_, _, immediate)| immediate) {
            ctx.request_repaint();
        } else if self.job.is_some() || self.pending_preview.is_some() || self.started.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn on_exit(&mut self) {
        self.sync_active_path();
        self.settings.save();
    }
}

const MIN_WINDOW_HEIGHT: f32 = 941.0;

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, MIN_WINDOW_HEIGHT])
            .with_min_inner_size([900.0, MIN_WINDOW_HEIGHT])
            .with_icon(crate::icon::app_icon()),
        centered: true,
        ..Default::default()
    };
    eframe::run_native("dubdsr", options, Box::new(|cc| Ok(Box::new(DubApp::new(cc)))))
}

fn theme() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    let label = Color32::from_rgb(0xff, 0xe1, 0x4d);
    let control = Color32::from_rgb(0xff, 0x93, 0x20);
    visuals.window_fill = Color32::from_rgb(0x16, 0x05, 0x05);
    visuals.panel_fill = Color32::from_rgb(0x16, 0x05, 0x05);
    visuals.extreme_bg_color = Color32::from_rgb(0x0e, 0x03, 0x03);
    visuals.faint_bg_color = Color32::from_rgb(0x3d, 0x12, 0x10);
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(0x3d, 0x12, 0x10);
    visuals.widgets.noninteractive.fg_stroke.color = label;
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(0x62, 0x20, 0x18);
    visuals.widgets.inactive.fg_stroke.color = control;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x7a, 0x28, 0x20);
    visuals.widgets.hovered.fg_stroke.color = control;
    visuals.widgets.active.bg_fill = Color32::from_rgb(0x4a, 0x14, 0x14);
    visuals.widgets.active.fg_stroke.color = control;
    visuals.widgets.open.bg_fill = Color32::from_rgb(0x62, 0x20, 0x18);
    visuals.widgets.open.fg_stroke.color = control;
    visuals.selection.bg_fill = Color32::from_rgb(0xc4, 0x3a, 0x18);
    visuals.selection.stroke.color = control;
    visuals
}

fn install_stack_font(ctx: &egui::Context) {
    let candidates = [r"C:\Windows\Fonts\segoeuib.ttf", r"C:\Windows\Fonts\arialbd.ttf"];
    let Some(bytes) = candidates.iter().find_map(|path| std::fs::read(path).ok()) else {
        return;
    };
    ctx.add_font(egui::FontInsert::new(
        "stack-bold",
        egui::FontData::from_owned(bytes),
        vec![egui::InsertFontFamily {
            family: egui::FontFamily::Name("stack-bold".into()),
            priority: egui::FontPriority::Highest,
        }],
    ));
}

fn group(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).strong());
    Frame::NONE
        .fill(Color32::from_rgb(0x3d, 0x12, 0x10))
        .stroke(Stroke::new(1.0, Color32::from_rgb(0x8e, 0x32, 0x24)))
        .inner_margin(8.0)
        .show(ui, |ui| {
            let width = ui.available_width();
            if width.is_finite() {
                ui.set_min_width(width);
            }
            add(ui);
        });
}

fn wide_drag<'a>(ui: &egui::Ui, drag: egui::DragValue<'a>) -> egui::DragValue<'a> {
    let width = ui.available_width();
    if width.is_finite() {
        drag.min_size(vec2(width, ui.spacing().interact_size.y))
    } else {
        drag
    }
}

fn split_options(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui, &mut egui::Ui)) {
    let gap = 18.0;
    let total = ui.available_width();
    let col = ((total - gap) * 0.5).max(40.0);
    let top = ui.cursor().min;
    let height_budget = 4096.0;
    let left_rect = Rect::from_min_size(top, vec2(col, height_budget));
    let right_rect = Rect::from_min_size(pos2(top.x + col + gap, top.y), vec2(col, height_budget));

    let mut left = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    left.set_width(col);
    left.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);

    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(right_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    right.set_width(col);
    right.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
    right.spacing_mut().item_spacing.y = 8.0;

    add(&mut left, &mut right);

    let used = left.min_rect().union(right.min_rect());
    let bottom = used.bottom().max(top.y);
    ui.advance_cursor_after_rect(Rect::from_min_max(top, pos2(top.x + total, bottom)));
    if bottom > top.y {
        ui.painter().vline(
            top.x + col + gap * 0.5,
            top.y..=bottom,
            Stroke::new(1.0, Color32::from_rgb(0x8e, 0x32, 0x24)),
        );
    }
}

fn modal(ctx: &Context, title: &str, body: &str, ok: &str, open: &mut bool) {
    if !*open {
        return;
    }
    let mut shown = true;
    let mut dismiss = false;
    egui::Window::new(title)
        .open(&mut shown)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(body);
            if ui.button(ok).clicked() {
                dismiss = true;
            }
        });
    if !shown || dismiss {
        *open = false;
    }
}

fn range_slider(ui: &mut egui::Ui, start: &mut usize, end: &mut usize, last: usize) -> (Option<(usize, bool)>, egui::Response) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click_and_drag());
    if last == 0 {
        return (None, response);
    }
    let x_of = |frame: usize| rect.left() + (frame as f32 / last as f32) * rect.width();
    let painter = ui.painter();
    let track = Rect::from_min_max(pos2(rect.left(), rect.center().y - 3.0), pos2(rect.right(), rect.center().y + 3.0));
    painter.rect_filled(track, 3.0, Color32::from_rgb(0x62, 0x20, 0x18));
    let span = Rect::from_min_max(pos2(x_of(*start), rect.center().y - 3.0), pos2(x_of(*end), rect.center().y + 3.0));
    painter.rect_filled(span, 3.0, Color32::from_rgb(0xc4, 0x3a, 0x18));
    let handle = Color32::from_rgb(0xff, 0x93, 0x20);
    painter.circle_filled(pos2(x_of(*start), rect.center().y), 7.0, handle);
    painter.circle_filled(pos2(x_of(*end), rect.center().y), 7.0, handle);

    let id = ui.scope_id().with("range-handle");
    if response.drag_started()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let which = if (pointer.x - x_of(*start)).abs() <= (pointer.x - x_of(*end)).abs() { 0u8 } else { 1 };
        ui.data_mut(|data| data.insert_temp(id, which));
    }
    if !response.dragged() && !response.drag_stopped() {
        return (None, response);
    }
    let Some(pointer) = response.interact_pointer_pos().or(ui.input(|input| input.pointer.interact_pos())) else {
        return (None, response);
    };
    let which = ui.data(|data| data.get_temp::<u8>(id).unwrap_or(0));
    let frame = ((pointer.x - rect.left()) / rect.width().max(1.0) * last as f32).round().clamp(0.0, last as f32) as usize;
    if which == 0 {
        *start = frame.min(*end);
    } else {
        *end = frame.max(*start);
    }
    let index = if which == 0 { *start } else { *end };
    (Some((index, response.drag_stopped())), response)
}

const HELP_PNG: &[u8] = include_bytes!("../manuals/fenetre.png");

const HELP_ITEMS: &[(&str, &str, &str)] = &[
    ("1", "help.t.gpu", "help.gpu"),
    ("2", "target.label", "help.target"),
    ("3", "help.t.path", "help.path"),
    ("4", "file.browse", "help.browse"),
    ("5", "file.folder", "help.folder"),
    ("6", "range.full", "help.full"),
    ("7", "range.section", "help.section"),
    ("8", "keep", "help.keep"),
    ("9", "max_frames", "help.limit"),
    ("10", "bayer", "help.bayer"),
    ("11", "margin", "help.margin"),
    ("12", "debayer", "help.debayer"),
    ("13", "wb", "help.wb"),
    ("14", "crop", "help.crop"),
    ("15", "drizzle", "help.drizzle"),
    ("16", "flatten", "help.flatten"),
    ("17", "gamma", "help.gamma"),
    ("18", "sharpen", "help.sharpen"),
    ("19", "radius", "help.radius"),
    ("20", "solar", "help.solar"),
    ("21", "black", "help.black"),
    ("22", "background", "help.background"),
    ("23", "protect", "help.protect"),
    ("24", "stack", "help.stack"),
    ("25", "help.t.progress", "help.progress"),
    ("26", "help.t.status", "help.status"),
    ("27", "preview", "help.preview"),
    ("28", "help.t.slider", "help.slider"),
    ("29", "help.t.time", "help.time"),
];

fn help_for(label: &str) -> &'static str {
    match label {
        "keep" => "help.keep",
        "margin" => "help.margin",
        "flatten" => "help.flatten",
        "gamma" => "help.gamma",
        "sharpen" => "help.sharpen",
        "radius" => "help.radius",
        "solar" => "help.solar",
        "black" => "help.black",
        "debayer" => "help.debayer",
        "wb" => "help.wb",
        "crop" => "help.crop",
        "drizzle" => "help.drizzle",
        "background" => "help.background",
        "protect" => "help.protect",
        _ => "help.open",
    }
}

fn tip_label(ui: &mut egui::Ui, text: &str) {
    ui.set_max_width(420.0);
    ui.label(text);
}

#[cfg(test)]
mod tests {
    use super::HELP_ITEMS;
    use crate::i18n::Catalog;

    #[test]
    fn help_texts_exist_in_every_language() {
        for language in ["fr", "en", "es", "de"] {
            let catalog = Catalog::load(language);
            for key in ["help.open", "help.title", "help.intro"] {
                let text = catalog.text(key);
                assert_ne!(text, key, "{language} {key}");
                assert!(!text.is_empty(), "{language} {key}");
            }
            for (_, title, body) in HELP_ITEMS {
                let title_text = catalog.text(title);
                assert_ne!(&title_text, title, "{language} {title}");
                let body_text = catalog.text(body);
                assert_ne!(&body_text, body, "{language} {body}");
            }
        }
    }
}

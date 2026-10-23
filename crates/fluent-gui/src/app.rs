use std::time::{Duration, Instant};

use eframe::egui;
use fluent_core::{
    estimate_memory, find_language, languages, popular_languages, DeviceChoice, EngineConfig,
    EngineStatus, Language, Mode, Precision, Progress, ProjectConfig, TranslationRequest,
    TranslationResult,
};

use crate::settings::{settings_path, Settings, ThemeChoice, ACCENTS};
use crate::theme::{self, Palette};
use crate::widgets;
use crate::worker::{Command, Event, WorkerHandle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerTarget {
    Source,
    Target,
}

pub struct FluentApp {
    project: ProjectConfig,
    settings: Settings,
    worker: WorkerHandle,
    palette: Palette,
    applied_theme: Option<(ThemeChoice, [u8; 3])>,

    source_text: String,
    result: Option<TranslationResult>,
    preview: String,
    error: Option<String>,
    notice: Option<(String, Instant)>,
    copied_at: Option<Instant>,

    busy: bool,
    loading: bool,
    pending_translation: bool,
    progress: Option<Progress>,
    status: Option<EngineStatus>,
    job: u64,

    settings_open: bool,
    picker: Option<PickerTarget>,
    picker_search: String,
    picker_focus: bool,
}

impl FluentApp {
    pub fn new(cc: &eframe::CreationContext<'_>, project: ProjectConfig) -> Self {
        let settings = Settings::load(&project);
        let palette = theme::apply(&cc.egui_ctx, settings.theme, settings.accent);
        let worker = WorkerHandle::spawn(cc.egui_ctx.clone());
        let source_text = settings.last_source.clone();

        Self {
            project,
            applied_theme: Some((settings.theme, settings.accent)),
            settings,
            worker,
            palette,
            source_text,
            result: None,
            preview: String::new(),
            error: None,
            notice: None,
            copied_at: None,
            busy: false,
            loading: false,
            pending_translation: false,
            progress: None,
            status: None,
            job: 0,
            settings_open: false,
            picker: None,
            picker_search: String::new(),
            picker_focus: false,
        }
    }

    // ------------------------------------------------------------------
    // Worker
    // ------------------------------------------------------------------

    fn drain_events(&mut self) {
        while let Ok(event) = self.worker.rx.try_recv() {
            match event {
                Event::Started { job, loading } => {
                    if job != self.job {
                        continue;
                    }
                    self.busy = true;
                    self.loading = loading;
                    self.error = None;
                    self.preview.clear();
                    if loading {
                        self.status = None;
                    }
                    self.progress = Some(Progress::new(
                        if loading {
                            fluent_core::ProgressStage::Preparing
                        } else {
                            fluent_core::ProgressStage::Encoding
                        },
                        Some(0.0),
                        if loading {
                            "Loading model…"
                        } else {
                            "Preparing translation…"
                        },
                    ));
                }
                Event::Progress { job, progress } => {
                    if job != self.job {
                        continue;
                    }
                    if let Some(preview) = &progress.preview {
                        self.preview = preview.clone();
                    }
                    self.progress = Some(progress);
                }
                Event::Loaded { job, status } => {
                    if job == self.job {
                        self.loading = false;
                    }
                    self.status = Some(status);
                    self.progress = None;
                    if !self.pending_translation {
                        self.busy = false;
                    }
                }
                Event::Unloaded => {
                    self.status = None;
                    self.busy = false;
                    self.loading = false;
                    self.progress = None;
                }
                Event::Translated { job, result } => {
                    if job != self.job {
                        continue;
                    }
                    self.busy = false;
                    self.loading = false;
                    self.progress = None;
                    self.preview.clear();
                    self.error = None;
                    self.result = Some(result);
                    self.settings.last_source = self.source_text.clone();
                    self.settings.save();
                }
                Event::Failed {
                    job,
                    message,
                    cancelled,
                } => {
                    if job != self.job {
                        continue;
                    }
                    self.busy = false;
                    self.loading = false;
                    self.progress = None;
                    self.preview.clear();
                    if cancelled {
                        self.notice = Some(("Translation cancelled".to_string(), Instant::now()));
                    } else {
                        self.error = Some(message);
                    }
                }
            }
        }
    }

    fn engine_config(&self) -> EngineConfig {
        let mut config = EngineConfig::from_project(&self.project, self.settings.mode);
        config.device = self.settings.device_choice();
        config.precision = self.settings.precision;
        config.num_beams = self.settings.num_beams;
        config.max_length = self.settings.max_length;
        config.max_new_tokens = self.settings.max_length;
        config
    }

    fn start_translation(&mut self) {
        if self.busy || self.source_text.trim().is_empty() {
            return;
        }
        self.job += 1;
        self.busy = true;
        self.loading = self.status.is_none();
        self.error = None;
        self.preview.clear();
        self.result = None;
        self.copied_at = None;
        self.progress = Some(Progress::new(
            fluent_core::ProgressStage::Preparing,
            Some(0.0),
            "Preparing translation…",
        ));

        let request = TranslationRequest::new(
            self.source_text.trim().to_string(),
            self.settings.source_language.clone(),
            self.settings.target_language.clone(),
        );
        self.settings.last_source = self.source_text.clone();
        self.settings.save();

        self.worker.submit(Command::Translate {
            job: self.job,
            request,
            config: self.engine_config(),
        });
    }

    fn load_model(&mut self) {
        if self.busy {
            return;
        }
        self.job += 1;
        self.busy = true;
        self.loading = true;
        self.error = None;
        self.status = None;
        self.progress = Some(Progress::new(
            fluent_core::ProgressStage::Preparing,
            Some(0.0),
            "Loading model…",
        ));
        self.worker.submit(Command::Load {
            job: self.job,
            config: self.engine_config(),
        });
    }

    fn unload_model(&mut self) {
        if self.busy {
            return;
        }
        self.worker.submit(Command::Unload);
        self.status = None;
        self.result = None;
        self.preview.clear();
    }

    fn request_cancel(&mut self) {
        self.worker.request_cancel();
        self.notice = Some(("Cancelling…".to_string(), Instant::now()));
    }

    fn swap_languages(&mut self) {
        if self.settings.source_language == "auto" {
            return;
        }
        std::mem::swap(
            &mut self.settings.source_language,
            &mut self.settings.target_language,
        );
        if let Some(result) = &self.result {
            if !result.text.trim().is_empty() {
                self.source_text = result.text.clone();
            }
        }
        self.result = None;
        self.settings.save();
    }

    // ------------------------------------------------------------------
    // Header
    // ------------------------------------------------------------------

    fn header(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette;
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(38.0, 38.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::same(12), palette.accent);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "F",
                egui::FontId::proportional(21.0),
                palette.on_accent,
            );

            ui.add_space(2.0);
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("Fluent")
                        .size(21.0)
                        .strong()
                        .color(palette.text),
                );
                ui.label(
                    egui::RichText::new("Local translation · NLLB-200")
                        .size(11.5)
                        .color(palette.muted),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, &palette, "⚙", "Settings").clicked() {
                    self.settings_open = true;
                }
                let theme_icon = if palette.dark { "☀" } else { "☾" };
                if widgets::icon_button(ui, &palette, theme_icon, "Change theme").clicked() {
                    self.settings.theme = match self.settings.theme {
                        ThemeChoice::System => {
                            if palette.dark {
                                ThemeChoice::Light
                            } else {
                                ThemeChoice::Dark
                            }
                        }
                        ThemeChoice::Light => ThemeChoice::Dark,
                        ThemeChoice::Dark => ThemeChoice::Light,
                    };
                    self.settings.save();
                }
                ui.add_space(4.0);
                self.status_chip(ui);
            });
        });
    }

    fn status_chip(&self, ui: &mut egui::Ui) {
        let palette = &self.palette;
        let fraction = self
            .progress
            .as_ref()
            .and_then(|progress| progress.fraction)
            .unwrap_or(0.0);
        let (color, label) = if self.busy && self.loading {
            (
                palette.warning,
                format!("Loading · {:.0} %", fraction * 100.0),
            )
        } else if self.busy {
            (palette.accent, "Translating".to_string())
        } else if self.error.is_some() {
            (palette.danger, "Error".to_string())
        } else if let Some(notice) = &self.notice {
            if notice.1.elapsed() < Duration::from_secs(4) {
                (palette.warning, notice.0.clone())
            } else {
                self.ready_label()
            }
        } else {
            self.ready_label()
        };

        egui::Frame::NONE
            .fill(palette.card_alt)
            .stroke(egui::Stroke::new(1.0, palette.border))
            .corner_radius(egui::CornerRadius::same(255))
            .inner_margin(egui::Margin::symmetric(10, 5))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    widgets::status_dot(ui, color);
                    ui.label(egui::RichText::new(label).size(11.5).color(palette.muted));
                });
            });
    }

    fn ready_label(&self) -> (egui::Color32, String) {
        match &self.status {
            Some(status) => (
                self.palette.success,
                format!("Ready · {} · {}", status.mode.model_label(), status.device),
            ),
            None => (self.palette.faint, "Model not loaded".to_string()),
        }
    }

    // ------------------------------------------------------------------
    // Language bar
    // ------------------------------------------------------------------

    fn language_bar(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                widgets::section_label(ui, &palette, "Source language");
                let automatic = self.settings.source_language == "auto";
                let code = if automatic {
                    None
                } else {
                    Some(self.settings.source_language.as_str())
                };
                if widgets::language_button(
                    ui,
                    &palette,
                    code,
                    "Automatic detection",
                    automatic,
                    230.0,
                )
                .clicked()
                {
                    self.open_picker(PickerTarget::Source);
                }
            });

            ui.add_space(2.0);
            ui.vertical(|ui| {
                ui.add_space(15.0);
                if widgets::icon_button(ui, &palette, "⇄", "Swap languages").clicked() {
                    self.swap_languages();
                }
            });

            ui.vertical(|ui| {
                widgets::section_label(ui, &palette, "Target language");
                if widgets::language_button(
                    ui,
                    &palette,
                    Some(self.settings.target_language.as_str()),
                    "Choose",
                    false,
                    230.0,
                )
                .clicked()
                {
                    self.open_picker(PickerTarget::Target);
                }
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                widgets::section_label(ui, &palette, "Mode");
                for mode in [Mode::Accurate, Mode::Fast] {
                    let selected = self.settings.mode == mode;
                    let fill = if selected {
                        palette.accent
                    } else {
                        palette.card_alt
                    };
                    let color = if selected {
                        palette.on_accent
                    } else {
                        palette.muted
                    };
                    let response = ui.add(
                        egui::Button::new(
                            egui::RichText::new(mode.label()).size(12.5).color(color),
                        )
                        .fill(fill)
                        .stroke(egui::Stroke::new(1.0, palette.border))
                        .corner_radius(egui::CornerRadius::same(255))
                        .min_size(egui::vec2(72.0, 30.0)),
                    );
                    if response.clicked() && !selected && !self.busy {
                        self.settings.mode = mode;
                        self.settings.save();
                    }
                }
            });
        });
    }

    fn open_picker(&mut self, target: PickerTarget) {
        self.picker = Some(target);
        self.picker_search.clear();
        self.picker_focus = true;
    }

    // ------------------------------------------------------------------
    // Cards
    // ------------------------------------------------------------------

    fn main_area(&mut self, ui: &mut egui::Ui) {
        self.language_bar(ui);
        ui.add_space(12.0);

        if let Some(error) = self.error.clone() {
            widgets::error_banner(ui, &self.palette, &error);
            ui.add_space(10.0);
        }

        let height = ui.available_height();
        let width = ui.available_width();
        if width >= 700.0 {
            ui.horizontal_top(|ui| {
                let column = ((width - 26.0) / 2.0).max(300.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(column, height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.source_card(ui, height),
                );
                ui.add_space(6.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(column, height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.target_card(ui, height),
                );
            });
        } else {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.source_card(ui, 350.0);
                    ui.add_space(12.0);
                    self.target_card(ui, 350.0);
                });
        }
    }

    fn source_card(&mut self, ui: &mut egui::Ui, height: f32) {
        let palette = self.palette;
        let width = ui.available_width();
        palette.card_frame().show(ui, |ui| {
            ui.set_min_size(egui::vec2(
                (width - 38.0).max(200.0),
                (height - 38.0).max(220.0),
            ));

            ui.horizontal(|ui| {
                widgets::section_label(ui, &palette, "Source text");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let count = self.source_text.chars().count();
                    if ui
                        .add_enabled(
                            !self.source_text.is_empty(),
                            egui::Button::new(
                                egui::RichText::new("Clear").size(12.0).color(palette.muted),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        self.source_text.clear();
                    }
                    ui.label(
                        egui::RichText::new(format!("{count} characters"))
                            .size(11.5)
                            .color(palette.faint),
                    );
                });
            });
            widgets::divider(ui, &palette);

            let footer_height = if self.busy { 58.0 } else { 44.0 };
            let text_height = (ui.available_height() - footer_height).max(90.0);
            // Make the editor grow with its content inside a scroll area, instead of
            // clipping the text when it exceeds the card height.
            let font = egui::FontId::proportional(16.0);
            let line_height = ui
                .painter()
                .layout_no_wrap("Xg".to_owned(), font.clone(), palette.text)
                .size()
                .y
                .max(1.0);
            let rows = ((text_height / line_height).floor() as usize).max(3);
            egui::ScrollArea::vertical()
                .id_salt("source_text_scroll")
                .auto_shrink([false, false])
                .max_height(text_height)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.source_text)
                            .frame(egui::Frame::NONE)
                            .hint_text(
                                egui::RichText::new("Type or paste your text…")
                                    .size(16.0)
                                    .color(palette.faint),
                            )
                            .desired_width(f32::INFINITY)
                            .desired_rows(rows)
                            .font(font)
                            .text_color(palette.text)
                            .margin(egui::Margin::same(0)),
                    );
                });
            ui.add_space(4.0);

            let compact = ui.available_width() < 430.0;
            ui.horizontal(|ui| {
                if let Some(result) = &self.result {
                    if let Some(detected) = &result.detected {
                        widgets::badge(
                            ui,
                            &palette,
                            &format!("{} · {:.0} %", detected.name, detected.confidence * 100.0),
                            palette.accent,
                        );
                    }
                } else if self.status.is_none() && !compact {
                    widgets::hint(
                        ui,
                        &palette,
                        "The model will load on the first translation.",
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.busy {
                        let cancel = egui::Button::new(
                            egui::RichText::new("Cancel")
                                .color(egui::Color32::WHITE)
                                .strong(),
                        )
                        .fill(palette.danger)
                        .stroke(egui::Stroke::NONE)
                        .corner_radius(egui::CornerRadius::same(11))
                        .min_size(egui::vec2(0.0, 38.0));
                        if ui.add(cancel).clicked() {
                            self.request_cancel();
                        }
                        if let Some(progress) = &self.progress {
                            ui.label(
                                egui::RichText::new(progress.stage.label())
                                    .size(11.5)
                                    .color(palette.muted),
                            );
                        }
                    } else {
                        let enabled = !self.source_text.trim().is_empty();
                        if widgets::primary_button(ui, &palette, "Translate", enabled).clicked() {
                            self.start_translation();
                        }
                        if !compact {
                            ui.label(
                                egui::RichText::new("Ctrl + Enter")
                                    .size(10.5)
                                    .color(palette.faint),
                            );
                        }
                    }
                });
            });
        });
    }

    fn target_card(&mut self, ui: &mut egui::Ui, height: f32) {
        let palette = self.palette;
        let width = ui.available_width();
        palette.card_frame().show(ui, |ui| {
            ui.set_min_size(egui::vec2(
                (width - 38.0).max(200.0),
                (height - 38.0).max(220.0),
            ));

            ui.horizontal(|ui| {
                widgets::section_label(ui, &palette, "Translation");
                if let Some(language) = find_language(&self.settings.target_language) {
                    widgets::badge(ui, &palette, language.name, palette.accent);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let copied = self
                        .copied_at
                        .map(|time| time.elapsed() < Duration::from_millis(1600))
                        .unwrap_or(false);
                    let label = if copied { "Copied ✓" } else { "Copy" };
                    let color = if copied {
                        palette.success
                    } else {
                        palette.muted
                    };
                    if ui
                        .add_enabled(
                            self.result.is_some(),
                            egui::Button::new(egui::RichText::new(label).size(12.0).color(color))
                                .frame(false),
                        )
                        .clicked()
                    {
                        if let Some(result) = &self.result {
                            ui.ctx().copy_text(result.text.clone());
                            self.copied_at = Some(Instant::now());
                        }
                    }
                });
            });
            widgets::divider(ui, &palette);

            let footer_height = if self.busy { 64.0 } else { 40.0 };
            let text_height = (ui.available_height() - footer_height).max(90.0);
            let body = self.result.as_ref().map(|result| result.text.as_str());
            let shown = if self.busy {
                self.preview.trim()
            } else {
                body.map(str::trim).unwrap_or("")
            };

            if shown.is_empty() {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), text_height),
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(egui::RichText::new("◍").size(30.0).color(palette.faint));
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(if self.busy {
                                    "Translation in progress…"
                                } else {
                                    "The translation will appear here."
                                })
                                .size(14.0)
                                .color(palette.muted),
                            );
                            if self.busy && self.loading {
                                if let Some(progress) = &self.progress {
                                    ui.add_space(6.0);
                                    ui.label(
                                        egui::RichText::new(&progress.message)
                                            .size(11.5)
                                            .color(palette.faint),
                                    );
                                }
                            }
                        });
                    },
                );
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("target_text_scroll")
                    .auto_shrink([false, false])
                    .max_height(text_height)
                    .scroll_bar_visibility(
                        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                    )
                    .show(ui, |ui| {
                        let color = if self.busy {
                            palette.muted
                        } else {
                            palette.text
                        };
                        ui.add(
                            egui::Label::new(egui::RichText::new(shown).size(16.5).color(color))
                                .wrap()
                                .selectable(!self.busy),
                        );
                    });
            }

            ui.add_space(4.0);
            if self.busy {
                let fraction = self
                    .progress
                    .as_ref()
                    .and_then(|progress| progress.fraction)
                    .unwrap_or(0.0);
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .desired_height(6.0)
                        .corner_radius(egui::CornerRadius::same(6))
                        .fill(palette.accent)
                        .animate(true),
                );
                ui.horizontal(|ui| {
                    if let Some(progress) = &self.progress {
                        ui.label(
                            egui::RichText::new(&progress.message)
                                .size(11.0)
                                .color(palette.muted),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{:.0} %", fraction * 100.0))
                                .size(11.0)
                                .color(palette.faint),
                        );
                    });
                });
            } else if let Some(result) = &self.result {
                let compact = ui.available_width() < 430.0;
                ui.horizontal(|ui| {
                    let language = find_language(&result.target_language)
                        .map(|language| language.name)
                        .unwrap_or(&result.target_language);
                    if !compact {
                        ui.label(
                            egui::RichText::new(language)
                                .size(11.5)
                                .color(palette.muted),
                        );
                        ui.label(egui::RichText::new("·").size(11.5).color(palette.faint));
                    }
                    ui.label(
                        egui::RichText::new(format!(
                            "{} tokens · {:.1} s",
                            result.generated_tokens,
                            result.duration.as_secs_f64()
                        ))
                        .size(11.5)
                        .color(palette.faint),
                    );
                    if result.truncated {
                        if compact {
                            widgets::badge(ui, &palette, "Text truncated", palette.warning);
                        } else {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    widgets::badge(ui, &palette, "Text truncated", palette.warning);
                                },
                            );
                        }
                    }
                });
            } else if self.status.is_none() {
                widgets::hint(
                    ui,
                    &palette,
                    "Choose a target language, then start translating.",
                );
            } else {
                widgets::hint(ui, &palette, "Ready to translate.");
            }
        });
    }

    // ------------------------------------------------------------------
    // Settings modal
    // ------------------------------------------------------------------

    fn settings_modal(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let palette = self.palette;
        let before = self.settings.clone();
        let mut close = false;
        let mut load_clicked = false;
        let mut unload_clicked = false;

        let frame = popup_frame(ctx, &palette);

        let response = egui::Modal::new(egui::Id::unique("fluent_settings"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(560.0);
                ui.set_max_height(ctx.content_rect().height() * 0.86);

                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Settings")
                            .size(22.0)
                            .strong()
                            .color(palette.text),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("✕").size(16.0).color(palette.muted),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            close = true;
                        }
                    });
                });
                ui.add_space(2.0);
                widgets::divider(ui, &palette);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(ctx.content_rect().height() * 0.62)
                    .show(ui, |ui| {
                        widgets::section_label(ui, &palette, "Translation model");
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let mut selected_mode = self.settings.mode;
                            for mode in [Mode::Fast, Mode::Accurate] {
                                let selected = self.settings.mode == mode;
                                let fill = if selected {
                                    palette.accent_soft
                                } else {
                                    palette.card_alt
                                };
                                let stroke = egui::Stroke::new(
                                    if selected { 1.5 } else { 1.0 },
                                    if selected { palette.accent } else { palette.border },
                                );
                                let card = egui::Frame::NONE
                                    .fill(fill)
                                    .stroke(stroke)
                                    .corner_radius(egui::CornerRadius::same(12))
                                    .inner_margin(egui::Margin::same(12))
                                    .show(ui, |ui| {
                                        ui.set_min_width(230.0);
                                        ui.label(
                                            egui::RichText::new(mode.model_label())
                                                .size(14.0)
                                                .strong()
                                                .color(if selected {
                                                    palette.accent
                                                } else {
                                                    palette.text
                                                }),
                                        );
                                        ui.label(
                                            egui::RichText::new(match mode {
                                                Mode::Fast => {
                                                    "Fast · 5.2 GB · recommended on CPU"
                                                }
                                                Mode::Accurate => {
                                                    "Accurate · 17.5 GB · GPU or lots of RAM"
                                                }
                                            })
                                            .size(11.0)
                                            .color(palette.muted),
                                        );
                                    })
                                    .response
                                    .interact(egui::Sense::click())
                                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                                if card.clicked() {
                                    selected_mode = mode;
                                }
                            }
                            self.settings.mode = selected_mode;
                        });

                        ui.add_space(8.0);
                        let model_dir = self.project.model_path(self.settings.mode);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Folder")
                                    .size(12.0)
                                    .color(palette.muted),
                            );
                            ui.label(
                                egui::RichText::new(model_dir.display().to_string())
                                    .monospace()
                                    .size(11.0)
                                    .color(palette.faint),
                            );
                        });

                        if !model_dir.is_dir() {
                            widgets::error_banner(
                                ui,
                                &palette,
                                "This model folder cannot be found.",
                            );
                        } else if let Ok(estimate) = estimate_memory(model_dir, self.settings.precision)
                        {
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "Weights ≈ {:.1} GB · memory needed ≈ {:.1} GB",
                                    estimate.weights_mb as f64 / 1024.0,
                                    estimate.needed_mb as f64 / 1024.0
                                ))
                                .size(11.5)
                                .color(palette.muted),
                            );
                            if let Some(available) = estimate.info.available_with_swap_mb() {
                                let memory_color = if estimate.needed_mb <= available {
                                    palette.success
                                } else {
                                    palette.danger
                                };
                                ui.label(
                                    egui::RichText::new(format!(
                                        "Available ≈ {:.1} GB (including {:.1} GB swap)",
                                        available as f64 / 1024.0,
                                        estimate.info.swap_free_mb.unwrap_or(0) as f64 / 1024.0
                                    ))
                                    .size(11.5)
                                    .color(memory_color),
                                );
                            }
                            if !estimate.fits() {
                                ui.add_space(4.0);
                                widgets::error_banner(
                                    ui,
                                    &palette,
                                    "Not enough memory: choose Fast mode, f16 precision, or free up memory.",
                                );
                            } else if estimate
                                .info
                                .available_mb
                                .is_some_and(|ram| estimate.needed_mb > ram)
                            {
                                ui.add_space(4.0);
                                widgets::info_banner(
                                    ui,
                                    &palette,
                                    "RAM is tight: the system will use swap, which will be very slow.",
                                );
                            }
                        }

                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Device")
                                    .size(13.0)
                                    .color(palette.text),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let current = self.settings.device_choice();
                                    egui::ComboBox::from_id_salt("device_combo")
                                        .selected_text(current.label())
                                        .width(230.0)
                                        .show_ui(ui, |ui| {
                                            for choice in [
                                                DeviceChoice::Auto,
                                                DeviceChoice::Cpu,
                                                DeviceChoice::Cuda(0),
                                                DeviceChoice::Metal(0),
                                            ] {
                                                if ui
                                                    .selectable_label(current == choice, choice.label())
                                                    .clicked()
                                                {
                                                    self.settings.set_device(choice);
                                                }
                                            }
                                        });
                                },
                            );
                        });
                        widgets::hint(
                            ui,
                            &palette,
                            if cfg!(feature = "cuda") {
                                "CUDA is compiled into this build."
                            } else {
                                "CUDA / Metal require a dedicated build (--features cuda or metal)."
                            },
                        );

                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Precision")
                                    .size(13.0)
                                    .color(palette.text),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    egui::ComboBox::from_id_salt("precision_combo")
                                        .selected_text(self.settings.precision.label())
                                        .width(230.0)
                                        .show_ui(ui, |ui| {
                                            for precision in
                                                [Precision::Auto, Precision::F32, Precision::F16]
                                            {
                                                if ui
                                                    .selectable_label(
                                                        self.settings.precision == precision,
                                                        precision.label(),
                                                    )
                                                    .clicked()
                                                {
                                                    self.settings.precision = precision;
                                                }
                                            }
                                        });
                                },
                            );
                        });

                        ui.add_space(8.0);
                        ui.add(
                            egui::Slider::new(&mut self.settings.num_beams, 1..=8)
                                .text("Beam search"),
                        );
                        ui.add(
                            egui::Slider::new(&mut self.settings.max_length, 64..=512)
                                .step_by(32.0)
                                .text("Maximum length (tokens)"),
                        );
                        widgets::hint(
                            ui,
                            &palette,
                            "More beams = better quality but slower generation. \
                             Changing the model, device or precision reloads the model.",
                        );

                        ui.add_space(12.0);
                        widgets::divider(ui, &palette);
                        widgets::section_label(ui, &palette, "Appearance");
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            for choice in
                                [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark]
                            {
                                let selected = self.settings.theme == choice;
                                let button = egui::Button::new(
                                    egui::RichText::new(choice.label()).size(12.5).color(
                                        if selected {
                                            palette.on_accent
                                        } else {
                                            palette.muted
                                        },
                                    ),
                                )
                                .fill(if selected {
                                    palette.accent
                                } else {
                                    palette.card_alt
                                })
                                .stroke(egui::Stroke::new(1.0, palette.border))
                                .corner_radius(egui::CornerRadius::same(9));
                                if ui.add(button).clicked() {
                                    self.settings.theme = choice;
                                }
                            }
                        });

                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            for (name, rgb) in ACCENTS {
                                let color = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                                let selected = self.settings.accent == *rgb;
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(34.0, 34.0),
                                    egui::Sense::click(),
                                );
                                ui.painter().circle_filled(rect.center(), 11.0, color);
                                if selected {
                                    ui.painter().circle_stroke(
                                        rect.center(),
                                        14.0,
                                        egui::Stroke::new(2.0, palette.text),
                                    );
                                }
                                if response
                                    .on_hover_text(*name)
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    self.settings.accent = *rgb;
                                }
                            }
                        });

                        ui.add_space(12.0);
                        widgets::divider(ui, &palette);
                        widgets::section_label(ui, &palette, "About");
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "Fluent 0.1.0 · {} {} · NLLB-200 engine in Rust",
                                std::env::consts::OS,
                                std::env::consts::ARCH
                            ))
                            .size(12.0)
                            .color(palette.muted),
                        );
                        ui.label(
                            egui::RichText::new(
                                "All translations are computed on this device. \
                                 No text is sent over the Internet.",
                            )
                            .size(12.0)
                            .color(palette.muted),
                        );
                        if let Some(path) = settings_path() {
                            ui.label(
                                egui::RichText::new(format!("Settings: {}", path.display()))
                                    .monospace()
                                    .size(10.5)
                                    .color(palette.faint),
                            );
                        }
                    });

                ui.add_space(8.0);
                widgets::divider(ui, &palette);
                ui.horizontal(|ui| {
                    if self.status.is_some() {
                        if widgets::ghost_button(ui, &palette, "Unload model").clicked() {
                            unload_clicked = true;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::primary_button(ui, &palette, "Close", true).clicked() {
                            close = true;
                        }
                        if widgets::accent_button(
                            ui,
                            &palette,
                            "Load now",
                            !self.busy,
                        )
                        .clicked()
                        {
                            load_clicked = true;
                        }
                    });
                });
            });

        if response.should_close() {
            close = true;
        }
        if close {
            self.settings_open = false;
        }
        if unload_clicked {
            self.unload_model();
        }
        if load_clicked {
            self.load_model();
        }
        if self.settings != before {
            self.settings.sanitize();
            self.settings.save();
        }
    }

    // ------------------------------------------------------------------
    // Language picker
    // ------------------------------------------------------------------

    fn picker_modal(&mut self, ctx: &egui::Context) {
        let Some(target) = self.picker else {
            return;
        };
        let palette = self.palette;
        let mut close = false;
        let mut selected: Option<&'static str> = None;
        let mut select_auto = false;

        let frame = popup_frame(ctx, &palette);

        let screen = ctx.content_rect();
        let modal_width = 540.0_f32.min((screen.width() - 48.0).max(320.0));
        let modal_height = (screen.height() * 0.84).clamp(300.0, 640.0);

        let response = egui::Modal::new(egui::Id::unique("fluent_language_picker"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(modal_width);
                ui.set_height((modal_height - 40.0).max(260.0));

                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(match target {
                            PickerTarget::Source => "Source language",
                            PickerTarget::Target => "Target language",
                        })
                        .size(20.0)
                        .strong()
                        .color(palette.text),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("✕").size(16.0).color(palette.muted),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            close = true;
                        }
                    });
                });

                ui.add_space(4.0);
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.picker_search)
                        .hint_text("Search for a language…")
                        .desired_width(f32::INFINITY),
                );
                if self.picker_focus {
                    search.request_focus();
                    self.picker_focus = false;
                }
                ui.add_space(6.0);

                if target == PickerTarget::Source {
                    let active = self.settings.source_language == "auto";
                    if picker_row(
                        ui,
                        &palette,
                        "Automatic detection",
                        "Detect the language from the text",
                        "auto",
                        active,
                    )
                    .clicked()
                    {
                        select_auto = true;
                    }
                    ui.add_space(2.0);
                }

                let query = self.picker_search.trim().to_lowercase();
                let list_height = (ui.available_height() - 8.0).max(120.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .min_scrolled_height(list_height)
                    .max_height(list_height)
                    .show(ui, |ui| {
                        let current = match target {
                            PickerTarget::Source => self.settings.source_language.as_str(),
                            PickerTarget::Target => self.settings.target_language.as_str(),
                        };

                        if query.is_empty() {
                            for language in popular_languages() {
                                if picker_language_row(ui, &palette, language, current) {
                                    selected = Some(language.code);
                                }
                            }
                            ui.add_space(6.0);
                            widgets::section_label(ui, &palette, "All languages");
                            ui.add_space(2.0);
                            for language in languages() {
                                if language.popular {
                                    continue;
                                }
                                if picker_language_row(ui, &palette, language, current) {
                                    selected = Some(language.code);
                                }
                            }
                        } else {
                            let mut count = 0usize;
                            for language in languages() {
                                let haystack = language.search_haystack();
                                if !haystack.contains(&query) {
                                    continue;
                                }
                                count += 1;
                                if picker_language_row(ui, &palette, language, current) {
                                    selected = Some(language.code);
                                }
                            }
                            if count == 0 {
                                ui.add_space(16.0);
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        egui::RichText::new("No languages found")
                                            .size(13.0)
                                            .color(palette.muted),
                                    );
                                });
                            }
                        }
                    });
            });

        if response.should_close() {
            close = true;
        }
        if select_auto {
            self.settings.source_language = "auto".to_string();
            close = true;
        }
        if let Some(code) = selected {
            match target {
                PickerTarget::Source => self.settings.source_language = code.to_string(),
                PickerTarget::Target => self.settings.target_language = code.to_string(),
            }
            self.result = None;
            close = true;
        }
        if close {
            self.picker = None;
            self.picker_search.clear();
            self.settings.save();
        }
    }
}

impl eframe::App for FluentApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.drain_events();

        let key = (self.settings.theme, self.settings.accent);
        let effective_dark = ctx.theme() == egui::Theme::Dark;
        if self.applied_theme != Some(key)
            || (self.settings.theme == ThemeChoice::System && self.palette.dark != effective_dark)
        {
            self.palette = theme::apply(&ctx, self.settings.theme, self.settings.accent);
            self.applied_theme = Some(key);
        }

        if !self.settings_open && self.picker.is_none() {
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter))
            {
                self.start_translation();
            }
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                && self.busy
            {
                self.request_cancel();
            }
        }

        let palette = self.palette;
        egui::Panel::top("fluent_header")
            .exact_size(74.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::NONE
                    .fill(palette.bg)
                    .inner_margin(egui::Margin::symmetric(22, 14)),
            )
            .show(ui, |ui| self.header(ui));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(palette.bg)
                    .inner_margin(egui::Margin::symmetric(22, 10)),
            )
            .show(ui, |ui| self.main_area(ui));

        self.settings_modal(&ctx);
        self.picker_modal(&ctx);

        // The status bar must refresh even without interaction during
        // generation (worker events already request a repaint).
        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }
}

fn popup_frame(ctx: &egui::Context, palette: &Palette) -> egui::Frame {
    egui::Frame::popup(&ctx.style_of(ctx.theme()))
        .fill(palette.card)
        .stroke(egui::Stroke::new(1.0, palette.border))
        .corner_radius(egui::CornerRadius::same(18))
        .inner_margin(egui::Margin::same(20))
}

fn picker_language_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    language: &'static Language,
    current: &str,
) -> bool {
    picker_row(
        ui,
        palette,
        language.name,
        language.native,
        language.code,
        current == language.code,
    )
    .clicked()
}

fn picker_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    name: &str,
    native: &str,
    code: &str,
    selected: bool,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 40.0), egui::Sense::click());

    if selected {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(9), palette.accent_soft);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(9), palette.card_alt);
    }

    let inner = rect.shrink2(egui::vec2(10.0, 0.0));
    let clip = ui.clip_rect();
    let code_width = 72.0_f32.min(inner.width() * 0.35);
    let left_rect = egui::Rect::from_min_max(
        inner.min,
        egui::pos2(
            (inner.right() - code_width - 8.0).max(inner.left()),
            inner.bottom(),
        ),
    );
    let code_rect = egui::Rect::from_min_max(
        egui::pos2((inner.right() - code_width).max(inner.left()), inner.top()),
        inner.max,
    );

    let mut left = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    left.set_clip_rect(clip.intersect(left_rect));
    left.add(
        egui::Label::new(
            egui::RichText::new(name)
                .size(13.5)
                .strong()
                .color(if selected {
                    palette.accent
                } else {
                    palette.text
                }),
        )
        .truncate(),
    );
    if !native.is_empty() && native != name {
        left.add_space(6.0);
        left.add(
            egui::Label::new(egui::RichText::new(native).size(11.5).color(palette.muted))
                .truncate(),
        );
    }

    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(code_rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    right.set_clip_rect(clip.intersect(code_rect));
    right.label(
        egui::RichText::new(code)
            .monospace()
            .size(10.5)
            .color(palette.faint),
    );

    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

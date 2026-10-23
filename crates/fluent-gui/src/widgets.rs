use egui::{Color32, CornerRadius, Margin, Response, RichText, Ui};

use crate::theme::{with_alpha, Palette};

pub fn section_label(ui: &mut Ui, palette: &Palette, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(10.5)
            .strong()
            .color(palette.faint),
    );
}

pub fn badge(ui: &mut Ui, palette: &Palette, text: &str, color: Color32) -> Response {
    let alpha = if palette.dark { 42 } else { 24 };
    egui::Frame::NONE
        .fill(with_alpha(color, alpha))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(9, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.5).color(color).strong());
        })
        .response
}

pub fn status_dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

pub fn primary_button(ui: &mut Ui, palette: &Palette, text: &str, enabled: bool) -> Response {
    let button = egui::Button::new(
        RichText::new(text)
            .color(palette.on_accent)
            .strong()
            .size(15.0),
    )
    .fill(palette.accent)
    .stroke(egui::Stroke::NONE)
    .corner_radius(CornerRadius::same(11))
    .min_size(egui::vec2(0.0, 38.0));
    ui.add_enabled(enabled, button)
}

pub fn accent_button(ui: &mut Ui, palette: &Palette, text: &str, enabled: bool) -> Response {
    let fill = if palette.dark {
        palette.accent_soft
    } else {
        palette.card_alt
    };
    let color = if enabled {
        palette.accent
    } else {
        palette.faint
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(color).strong())
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, palette.border))
            .corner_radius(CornerRadius::same(10))
            .min_size(egui::vec2(0.0, 34.0)),
    )
}

pub fn ghost_button(ui: &mut Ui, palette: &Palette, text: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(palette.muted))
            .frame(false)
            .min_size(egui::vec2(0.0, 28.0)),
    )
}

pub fn icon_button(ui: &mut Ui, palette: &Palette, glyph: &str, tooltip: &str) -> Response {
    let response = ui.add(
        egui::Button::new(RichText::new(glyph).size(17.0).color(palette.muted))
            .fill(palette.card_alt)
            .stroke(egui::Stroke::new(1.0, palette.border))
            .corner_radius(CornerRadius::same(10))
            .min_size(egui::vec2(36.0, 34.0)),
    );
    response.on_hover_text(tooltip)
}

/// Compact language selector: name + code, with a chevron.
pub fn language_button(
    ui: &mut Ui,
    palette: &Palette,
    code: Option<&str>,
    label: &str,
    automatic: bool,
    width: f32,
) -> Response {
    let (main, sub) = match code {
        Some(code) => {
            let language = fluent_core::find_language(code);
            (
                language.map(|l| l.name).unwrap_or(code).to_string(),
                if automatic {
                    format!("Automatic detection · {code}")
                } else {
                    code.to_string()
                },
            )
        }
        None => (label.to_string(), String::new()),
    };

    let text = if sub.is_empty() {
        RichText::new(main).color(palette.text).size(15.0)
    } else {
        RichText::new(format!("{main}\n{sub}"))
            .color(palette.text)
            .size(14.5)
    };

    let button = egui::Button::new(text)
        .fill(palette.card_alt)
        .stroke(egui::Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(11))
        .min_size(egui::vec2(width, 44.0));

    let response = ui.add_sized([width, 44.0], button);
    // Small chevron drawn on the right.
    let rect = response.rect;
    let center = egui::pos2(rect.right() - 16.0, rect.center().y);
    ui.painter().text(
        center,
        egui::Align2::CENTER_CENTER,
        "▾",
        egui::FontId::proportional(11.0),
        palette.muted,
    );
    response
}

pub fn hint(ui: &mut Ui, palette: &Palette, text: &str) {
    ui.label(RichText::new(text).size(11.5).color(palette.faint));
}

pub fn divider(ui: &mut Ui, palette: &Palette) {
    ui.add_space(6.0);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::ZERO, palette.border);
    ui.add_space(6.0);
}

pub fn error_banner(ui: &mut Ui, palette: &Palette, message: &str) {
    egui::Frame::NONE
        .fill(with_alpha(
            palette.danger,
            if palette.dark { 34 } else { 20 },
        ))
        .stroke(egui::Stroke::new(
            1.0,
            with_alpha(palette.danger, if palette.dark { 110 } else { 70 }),
        ))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("⚠").color(palette.danger).size(14.0));
                ui.label(RichText::new(message).size(13.0).color(palette.danger));
            });
        });
}

pub fn info_banner(ui: &mut Ui, palette: &Palette, message: &str) {
    egui::Frame::NONE
        .fill(palette.accent_soft)
        .stroke(egui::Stroke::new(1.0, with_alpha(palette.accent, 90)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.label(RichText::new(message).size(12.5).color(palette.text));
        });
}

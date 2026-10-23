use egui::{Color32, CornerRadius, Margin, Stroke, Visuals};

use crate::settings::ThemeChoice;

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub card: Color32,
    pub card_alt: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub input_bg: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
}

impl Palette {
    pub fn new(dark: bool, accent: [u8; 3]) -> Self {
        let accent = Color32::from_rgb(accent[0], accent[1], accent[2]);
        let (bg, card, card_alt, border, text, muted, faint, input_bg) = if dark {
            (
                Color32::from_rgb(13, 16, 19),
                Color32::from_rgb(22, 26, 30),
                Color32::from_rgb(29, 34, 39),
                Color32::from_rgb(42, 49, 56),
                Color32::from_rgb(236, 241, 244),
                Color32::from_rgb(150, 162, 172),
                Color32::from_rgb(110, 122, 132),
                Color32::from_rgb(16, 20, 23),
            )
        } else {
            (
                Color32::from_rgb(243, 245, 247),
                Color32::from_rgb(255, 255, 255),
                Color32::from_rgb(247, 249, 250),
                Color32::from_rgb(226, 232, 235),
                Color32::from_rgb(19, 25, 29),
                Color32::from_rgb(103, 116, 124),
                Color32::from_rgb(150, 160, 168),
                Color32::from_rgb(246, 248, 249),
            )
        };

        let accent_soft = mix(card, accent, if dark { 0.20 } else { 0.10 });
        let on_accent = if luminance(accent) > 0.62 {
            Color32::from_rgb(20, 24, 28)
        } else {
            Color32::WHITE
        };

        Self {
            dark,
            bg,
            card,
            card_alt,
            border,
            text,
            muted,
            faint,
            input_bg,
            accent,
            accent_soft,
            on_accent,
            success: Color32::from_rgb(22, 163, 74),
            warning: Color32::from_rgb(217, 119, 6),
            danger: Color32::from_rgb(220, 38, 38),
        }
    }

    pub fn card_frame(&self) -> egui::Frame {
        egui::Frame::NONE
            .fill(self.card)
            .stroke(Stroke::new(1.0, self.border))
            .corner_radius(CornerRadius::same(16))
            .inner_margin(Margin::same(18))
    }
}

/// Applies the theme and returns the effective palette.
pub fn apply(ctx: &egui::Context, preference: ThemeChoice, accent: [u8; 3]) -> Palette {
    ctx.set_theme(egui::ThemePreference::from(preference));
    let dark = ctx.theme() == egui::Theme::Dark;
    let palette = Palette::new(dark, accent);

    let mut visuals = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    visuals.override_text_color = Some(palette.text);
    visuals.weak_text_color = Some(palette.muted);
    visuals.panel_fill = palette.bg;
    visuals.window_fill = palette.card;
    visuals.window_stroke = Stroke::new(1.0, palette.border);
    visuals.window_corner_radius = CornerRadius::same(16);
    visuals.menu_corner_radius = CornerRadius::same(12);
    visuals.extreme_bg_color = palette.input_bg;
    visuals.text_edit_bg_color = Some(palette.input_bg);
    visuals.faint_bg_color = palette.card_alt;
    visuals.code_bg_color = palette.card_alt;
    visuals.hyperlink_color = palette.accent;
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(
        palette.accent.r(),
        palette.accent.g(),
        palette.accent.b(),
        72,
    );
    visuals.selection.stroke = Stroke::new(1.0, palette.text);
    visuals.button_frame = true;
    visuals.striped = false;
    visuals.slider_trailing_fill = true;
    visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);

    let w = &mut visuals.widgets;
    w.noninteractive.bg_fill = palette.card;
    w.noninteractive.weak_bg_fill = palette.card;
    w.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    w.noninteractive.corner_radius = CornerRadius::same(10);

    w.inactive.bg_fill = palette.card_alt;
    w.inactive.weak_bg_fill = palette.card_alt;
    w.inactive.bg_stroke = Stroke::new(1.0, palette.border);
    // Also used as the scroll-handle color, which must stand out from the track.
    w.inactive.fg_stroke = Stroke::new(1.0, palette.faint);
    w.inactive.corner_radius = CornerRadius::same(10);
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = palette.accent_soft;
    w.hovered.weak_bg_fill = palette.accent_soft;
    w.hovered.bg_stroke = Stroke::new(1.0, palette.accent);
    w.hovered.fg_stroke = Stroke::new(1.5, palette.text);
    w.hovered.corner_radius = CornerRadius::same(10);
    w.hovered.expansion = 0.0;

    w.active.bg_fill = palette.accent;
    w.active.weak_bg_fill = palette.accent;
    w.active.bg_stroke = Stroke::new(1.0, palette.accent);
    w.active.fg_stroke = Stroke::new(1.5, palette.on_accent);
    w.active.corner_radius = CornerRadius::same(10);
    w.active.expansion = 0.0;

    w.open.bg_fill = palette.accent_soft;
    w.open.weak_bg_fill = palette.accent_soft;
    w.open.bg_stroke = Stroke::new(1.0, palette.accent);
    w.open.fg_stroke = Stroke::new(1.0, palette.text);
    w.open.corner_radius = CornerRadius::same(10);

    let theme = ctx.theme();
    let mut style = (*ctx.style_of(theme)).clone();
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size = egui::vec2(40.0, 34.0);
    style.spacing.slider_width = 190.0;
    style.spacing.combo_width = 190.0;
    style.spacing.indent = 18.0;
    style.spacing.scroll.bar_width = 8.0;
    style.spacing.scroll.floating = false;
    // Draw the handle with the (muted) foreground color so it stays visible on
    // both light and dark tracks.
    style.spacing.scroll.foreground_color = true;
    style.spacing.scroll.bar_inner_margin = 8.0;
    style.text_styles = [
        (egui::TextStyle::Small, egui::FontId::proportional(12.0)),
        (egui::TextStyle::Body, egui::FontId::proportional(15.0)),
        (egui::TextStyle::Button, egui::FontId::proportional(15.0)),
        (egui::TextStyle::Heading, egui::FontId::proportional(23.0)),
        (egui::TextStyle::Monospace, egui::FontId::monospace(13.0)),
    ]
    .into();
    ctx.set_style_of(theme, style);

    palette
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let channel = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t).round() as u8;
    Color32::from_rgb(
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
    )
}

pub fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn luminance(color: Color32) -> f32 {
    let channel = |value: u8| {
        let value = value as f32 / 255.0;
        if value <= 0.039_28 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

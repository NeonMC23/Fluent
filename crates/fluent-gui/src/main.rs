mod app;
mod settings;
mod theme;
mod widgets;
mod worker;

use eframe::egui;
use fluent_core::ProjectConfig;

fn main() -> eframe::Result<()> {
    let project = ProjectConfig::load();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Fluent — Local Translation")
            .with_app_id("io.fluent.Fluent")
            .with_inner_size([1140.0, 730.0])
            .with_min_inner_size([780.0, 560.0])
            .with_icon(app_icon()),
        renderer: eframe::Renderer::Glow,
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "Fluent",
        options,
        Box::new(move |cc| Ok(Box::new(app::FluentApp::new(cc, project)))),
    )
}

/// Generated icon: turquoise dot and white text lines.
fn app_icon() -> egui::IconData {
    const SIZE: usize = 64;
    let accent = (15u8, 159u8, 143u8);
    let mut rgba = vec![0u8; SIZE * SIZE * 4];

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - 31.5;
            let dy = y as f32 - 31.5;
            let inside = dx * dx + dy * dy <= 29.5 * 29.5;
            if !inside {
                continue;
            }
            let index = (y * SIZE + x) * 4;
            let mut color = (accent.0, accent.1, accent.2);
            let line_one = (18..=46).contains(&x) && (22..=26).contains(&y);
            let line_two = (18..=38).contains(&x) && (32..=36).contains(&y);
            if line_one || line_two {
                color = (255, 255, 255);
            }
            rgba[index] = color.0;
            rgba[index + 1] = color.1;
            rgba[index + 2] = color.2;
            rgba[index + 3] = 255;
        }
    }

    egui::IconData {
        rgba,
        width: SIZE as u32,
        height: SIZE as u32,
    }
}

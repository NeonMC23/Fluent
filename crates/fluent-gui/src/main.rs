mod app;
mod chat_worker;
mod settings;
mod theme;
mod widgets;
mod worker;

use eframe::egui;
use fluent_core::ProjectConfig;

const HELP: &str = "\
Fluent - local NLLB-200 translation

Usage:
  fluent                     Open the graphical interface
  fluent --version, -V       Print the application version and exit
  fluent --self-test         Run non-interactive runtime checks and exit
  fluent --smoke-test-gui    Open a window briefly, paint, and exit
  fluent --help, -h          Show this help

Environment:
  FLUENT_CONFIG              Path to config.json
  XDG_CONFIG_HOME, XDG_DATA_HOME, HOME
                             Standard per-user configuration locations
";

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        print_version();
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{HELP}");
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--self-test") {
        std::process::exit(run_self_test());
    }

    let smoke_gui = args.iter().any(|arg| arg == "--smoke-test-gui");
    if let Some(unknown) = args.iter().find(|arg| arg.as_str() != "--smoke-test-gui") {
        eprintln!("Fluent: unknown argument: {unknown}");
        eprint!("{HELP}");
        std::process::exit(2);
    }

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
        Box::new(move |cc| {
            let mut app = app::FluentApp::new(cc, project);
            if smoke_gui {
                app.set_smoke_frames(6);
                cc.egui_ctx.request_repaint();
            }
            Ok(Box::new(app))
        }),
    )
}

fn print_version() {
    println!(
        "Fluent {} ({} {})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
}

/// Verify the pieces that must work without a display, GPU or model files.
/// Missing models are reported but are not a failure: release archives
/// deliberately exclude them.
fn run_self_test() -> i32 {
    print_version();

    let mut ok = true;

    // The window icon is compiled into the binary. A truncated or absent
    // asset is a packaging/build defect.
    const ICON_SIZE: usize = 256 * 256 * 4;
    let icon = app_icon();
    let has_opaque_pixel = icon.rgba.chunks_exact(4).any(|pixel| pixel[3] > 0);
    if icon.width == 256 && icon.height == 256 && icon.rgba.len() == ICON_SIZE && has_opaque_pixel {
        println!("window-icon: OK (256x256 RGBA, {} bytes)", icon.rgba.len());
    } else {
        eprintln!("window-icon: FAIL");
        ok = false;
    }

    match std::env::current_exe() {
        Ok(path) if path.is_file() => println!("executable: OK ({})", path.display()),
        Ok(path) => {
            eprintln!("executable: FAIL (missing: {})", path.display());
            ok = false;
        }
        Err(error) => {
            eprintln!("executable: FAIL ({error})");
            ok = false;
        }
    }

    let config = ProjectConfig::load();
    match &config.config_path {
        Some(path) => println!("config: OK ({})", path.display()),
        None => println!("config: defaults (no config.json found)"),
    }

    let language_count = fluent_core::languages().len();
    if language_count > 0 && fluent_core::find_language(&config.default_target_language).is_some() {
        println!("languages: OK ({language_count} codes)");
    } else {
        eprintln!(
            "languages: FAIL (count {language_count}, default target {})",
            config.default_target_language
        );
        ok = false;
    }

    for (label, path) in [
        ("fast-model", &config.fast_model),
        ("accurate-model", &config.accurate_model),
        ("madlad-model", &config.madlad_model),
        ("small-model", &config.small_model),
        ("chat-model", &config.chat_model),
    ] {
        if path.is_dir() {
            let entries = std::fs::read_dir(path)
                .map(|entries| entries.flatten().count())
                .unwrap_or(0);
            if entries > 0 {
                println!("{label}: present ({}, {entries} entries)", path.display());
            } else {
                println!("{label}: empty directory ({})", path.display());
            }
        } else {
            println!("{label}: not installed ({})", path.display());
        }
    }

    if ok {
        println!("SELF-TEST OK");
        0
    } else {
        eprintln!("SELF-TEST FAILED");
        1
    }
}

/// Official Fluent window icon, embedded at compile time as raw RGBA.
///
/// Generated by `scripts/generate-branding-assets.py` from the 1024×1024
/// master. Embedding avoids an image-decoder dependency and any runtime
/// filesystem lookup.
fn app_icon() -> egui::IconData {
    const SIZE: u32 = 256;
    const RGBA: &[u8] = include_bytes!("../../../src/logo/generated/window/fluent-256.rgba");
    const _: () = assert!(RGBA.len() == (SIZE * SIZE * 4) as usize);

    egui::IconData {
        rgba: RGBA.to_vec(),
        width: SIZE,
        height: SIZE,
    }
}

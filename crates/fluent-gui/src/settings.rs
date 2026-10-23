use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use fluent_core::{DeviceChoice, Mode, Precision, ProjectConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
}

impl Default for ThemeChoice {
    fn default() -> Self {
        Self::System
    }
}

impl ThemeChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

impl From<ThemeChoice> for egui::ThemePreference {
    fn from(value: ThemeChoice) -> Self {
        match value {
            ThemeChoice::System => Self::System,
            ThemeChoice::Light => Self::Light,
            ThemeChoice::Dark => Self::Dark,
        }
    }
}

/// Accent colors offered in the settings.
pub const ACCENTS: &[(&str, [u8; 3])] = &[
    ("Turquoise", [15, 159, 143]),
    ("Indigo", [79, 70, 229]),
    ("Blue", [37, 99, 235]),
    ("Violet", [147, 51, 234]),
    ("Rose", [225, 29, 116]),
    ("Amber", [217, 119, 6]),
    ("Emerald", [5, 150, 105]),
    ("Slate", [71, 85, 105]),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub accent: [u8; 3],
    pub mode: Mode,
    pub source_language: String,
    pub target_language: String,
    /// `auto`, `cpu`, `cuda:0`, `metal:0`…
    pub device: String,
    pub precision: Precision,
    pub num_beams: usize,
    pub max_length: usize,
    /// Last entered text, restored on startup.
    pub last_source: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            accent: [15, 159, 143],
            mode: Mode::Fast,
            source_language: "auto".to_string(),
            target_language: "fra_Latn".to_string(),
            device: "auto".to_string(),
            precision: Precision::Auto,
            num_beams: 4,
            max_length: 512,
            last_source: String::new(),
        }
    }
}

impl Settings {
    pub fn load(project: &ProjectConfig) -> Self {
        // `config.json` provides the initial values; an existing settings file
        // then overrides all of them.
        let mut settings = Self {
            mode: project.default_mode,
            source_language: project.default_source_language.clone(),
            target_language: project.default_target_language.clone(),
            ..Self::default()
        };
        if let Some(path) = settings_path() {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                match serde_json::from_str::<Self>(&raw) {
                    Ok(parsed) => settings = parsed,
                    Err(error) => {
                        eprintln!(
                            "Fluent: could not read settings in {} ({error}), using defaults.",
                            path.display()
                        );
                    }
                }
            }
        }
        if settings.last_source.is_empty() {
            settings.last_source = String::new();
        }
        settings.sanitize();
        settings
    }

    pub fn sanitize(&mut self) {
        self.num_beams = self.num_beams.clamp(1, 8);
        self.max_length = self.max_length.clamp(64, 1024);
        if self.source_language.is_empty() {
            self.source_language = "auto".to_string();
        }
        if self.target_language.is_empty() || self.target_language == "auto" {
            self.target_language = "eng_Latn".to_string();
        }
        if self.device.is_empty() {
            self.device = "auto".to_string();
        }
    }

    pub fn save(&self) {
        let Some(path) = settings_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                eprintln!("Fluent: could not create {} ({error})", parent.display());
                return;
            }
        }
        match serde_json::to_string_pretty(self) {
            Ok(raw) => {
                if let Err(error) = std::fs::write(&path, raw) {
                    eprintln!("Fluent: could not write {} ({error})", path.display());
                }
            }
            Err(error) => eprintln!("Fluent: could not serialize settings ({error})"),
        }
    }

    pub fn device_choice(&self) -> DeviceChoice {
        let value = self.device.trim().to_ascii_lowercase();
        if value == "cpu" {
            return DeviceChoice::Cpu;
        }
        if let Some(rest) = value.strip_prefix("cuda:") {
            return DeviceChoice::Cuda(rest.parse().unwrap_or(0));
        }
        if value == "cuda" {
            return DeviceChoice::Cuda(0);
        }
        if let Some(rest) = value.strip_prefix("metal:") {
            return DeviceChoice::Metal(rest.parse().unwrap_or(0));
        }
        if value == "metal" {
            return DeviceChoice::Metal(0);
        }
        DeviceChoice::Auto
    }

    pub fn set_device(&mut self, choice: DeviceChoice) {
        self.device = match choice {
            DeviceChoice::Auto => "auto".to_string(),
            DeviceChoice::Cpu => "cpu".to_string(),
            DeviceChoice::Cuda(index) => format!("cuda:{index}"),
            DeviceChoice::Metal(index) => format!("metal:{index}"),
        };
    }
}

pub fn settings_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("io", "fluent", "Fluent")
        .map(|dirs| dirs.config_dir().join("settings.json"))
}

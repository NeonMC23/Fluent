use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use fluent_core::{DeviceChoice, Mode, OutputLimit, Precision, ProjectConfig};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputLengthSetting {
    /// Use `max_new_tokens` as a finite ceiling.
    Limited,
    /// Continue until EOS, cancellation or the model's positional capacity.
    Unlimited,
    /// Preserve the input-derived budget used by earlier releases. Unknown
    /// persisted values fall back here instead of discarding the whole file.
    #[default]
    #[serde(other)]
    Automatic,
}

impl OutputLengthSetting {
    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::Limited => "Custom limit",
            Self::Unlimited => "No limit",
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
    /// Maximum source tokens.
    pub max_length: usize,
    pub output_length: OutputLengthSetting,
    /// Finite output ceiling used when `output_length` is `Limited`.
    pub max_new_tokens: usize,
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
            output_length: OutputLengthSetting::Automatic,
            max_new_tokens: 512,
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
        self.max_new_tokens = self.max_new_tokens.clamp(64, 1024);
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

    /// Maps the persisted GUI choice to the inference-engine policy.
    pub fn output_limit(&self) -> OutputLimit {
        match self.output_length {
            OutputLengthSetting::Automatic => OutputLimit::Auto,
            OutputLengthSetting::Limited => OutputLimit::Tokens(self.max_new_tokens),
            OutputLengthSetting::Unlimited => OutputLimit::UntilEos,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_strings_round_trip() {
        for choice in [
            DeviceChoice::Auto,
            DeviceChoice::Cpu,
            DeviceChoice::Cuda(0),
            DeviceChoice::Cuda(1),
            DeviceChoice::Metal(0),
        ] {
            let mut settings = Settings::default();
            settings.set_device(choice);
            assert_eq!(settings.device_choice(), choice);
        }
    }

    #[test]
    fn unknown_device_strings_fall_back_to_automatic() {
        let mut settings = Settings {
            device: "quantum".to_string(),
            ..Settings::default()
        };
        assert_eq!(settings.device_choice(), DeviceChoice::Auto);
        settings.device = "cuda:not-a-number".to_string();
        assert_eq!(settings.device_choice(), DeviceChoice::Cuda(0));
    }

    #[test]
    fn sanitize_clamps_generation_settings() {
        let mut settings = Settings {
            num_beams: 0,
            max_length: 10,
            max_new_tokens: 1,
            source_language: String::new(),
            target_language: "auto".to_string(),
            device: String::new(),
            ..Settings::default()
        };
        settings.sanitize();
        assert_eq!(settings.num_beams, 1);
        assert_eq!(settings.max_length, 64);
        assert_eq!(settings.source_language, "auto");
        assert_eq!(settings.target_language, "eng_Latn");
        assert_eq!(settings.device, "auto");

        settings.num_beams = 99;
        settings.max_length = 9_000;
        settings.sanitize();
        assert_eq!(settings.num_beams, 8);
        assert_eq!(settings.max_length, 1024);
        assert_eq!(settings.max_new_tokens, 64);

        settings.max_new_tokens = 9_000;
        settings.sanitize();
        assert_eq!(settings.max_new_tokens, 1024);
    }

    #[test]
    fn output_length_choices_map_to_the_inference_policy() {
        let mut settings = Settings::default();
        assert_eq!(settings.output_length, OutputLengthSetting::Automatic);
        assert_eq!(settings.output_limit(), OutputLimit::Auto);

        settings.output_length = OutputLengthSetting::Limited;
        settings.max_new_tokens = 600;
        assert_eq!(settings.output_limit(), OutputLimit::Tokens(600));

        settings.output_length = OutputLengthSetting::Unlimited;
        assert_eq!(settings.output_limit(), OutputLimit::UntilEos);
    }

    #[test]
    fn madlad_mode_round_trips_in_settings() {
        let parsed: Settings =
            serde_json::from_str(r#"{"mode":"madlad","target_language":"fra_Latn"}"#).unwrap();
        assert_eq!(parsed.mode, Mode::Madlad);
        assert!(Mode::Madlad.supports_language("fra_Latn"));
        assert!(!Mode::Madlad.supports_language("not_a_language"));
    }

    #[test]
    fn unknown_output_choice_falls_back_to_automatic() {
        let parsed: Settings =
            serde_json::from_str(r#"{"output_length":"something-new","num_beams":3}"#).unwrap();
        assert_eq!(parsed.output_length, OutputLengthSetting::Automatic);
        assert_eq!(parsed.num_beams, 3);
    }

    #[test]
    fn older_settings_files_default_to_automatic_output() {
        let parsed: Settings = serde_json::from_str(r#"{"theme":"dark","num_beams":2}"#).unwrap();
        assert_eq!(parsed.output_length, OutputLengthSetting::Automatic);
        assert_eq!(parsed.max_new_tokens, 512);
    }

    #[test]
    fn output_length_round_trips_through_json() {
        for choice in [
            OutputLengthSetting::Automatic,
            OutputLengthSetting::Limited,
            OutputLengthSetting::Unlimited,
        ] {
            let settings = Settings {
                output_length: choice,
                max_new_tokens: 768,
                ..Settings::default()
            };
            let raw = serde_json::to_string(&settings).unwrap();
            let parsed: Settings = serde_json::from_str(&raw).unwrap();
            assert_eq!(parsed.output_length, choice);
            assert_eq!(parsed.max_new_tokens, 768);
        }
    }
}

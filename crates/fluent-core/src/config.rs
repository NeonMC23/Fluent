use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::model::chat_registry::{chat_model_spec, default_chat_model};
use crate::{Error, Mode, OutputLimit, Result};

#[derive(Debug, Clone)]
pub struct ProjectConfig {
    pub config_path: Option<PathBuf>,
    /// Set when a populated portable model directory was adopted because no
    /// `config.json` existed. Used for first-run guidance; no files are moved.
    pub detected_model_root: Option<PathBuf>,
    pub base_dir: PathBuf,
    pub default_source_language: String,
    pub default_target_language: String,
    pub default_mode: Mode,
    pub theme: String,
    pub fast_model: PathBuf,
    pub accurate_model: PathBuf,
    pub madlad_model: PathBuf,
    pub small_model: PathBuf,
    /// Chat model directory for the default (1.5B) chat model. Other chat
    /// models live next to it.
    pub chat_model: PathBuf,
    pub max_length: usize,
    /// Generated-token policy. `Auto` derives a finite budget from the
    /// encoded source; `Tokens(n)` uses a finite ceiling; `UntilEos` removes
    /// the Fluent ceiling and runs until EOS, cancellation or model capacity.
    pub output_limit: OutputLimit,
    pub num_beams: usize,
}

const MODEL_DIRS: [&str; 6] = [
    "nllb-200-1.3b",
    "nllb-200-3.3b",
    "madlad400-3b-mt",
    "small100",
    "qwen2.5-1.5b-instruct",
    "qwen2.5-0.5b-instruct",
];

impl Default for ProjectConfig {
    fn default() -> Self {
        Self::for_model_root(&default_model_root())
    }
}

impl ProjectConfig {
    /// Build the default configuration with every model directory under one
    /// root. Used by the platform default and by tests; keeping this pure
    /// avoids relying on the process working directory.
    fn for_model_root(root: &Path) -> Self {
        let base_dir = root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf());
        Self {
            config_path: None,
            detected_model_root: None,
            fast_model: root.join("nllb-200-1.3b"),
            accurate_model: root.join("nllb-200-3.3b"),
            madlad_model: root.join("madlad400-3b-mt"),
            small_model: root.join("small100"),
            chat_model: root.join("qwen2.5-1.5b-instruct"),
            base_dir,
            default_source_language: "auto".to_string(),
            default_target_language: "eng_Latn".to_string(),
            default_mode: Mode::Fast,
            theme: "system".to_string(),
            max_length: 512,
            output_limit: OutputLimit::Auto,
            num_beams: 4,
        }
    }

    fn with_model_root(mut self, root: &Path) -> Self {
        self.detected_model_root = Some(root.to_path_buf());
        self.fast_model = root.join("nllb-200-1.3b");
        self.accurate_model = root.join("nllb-200-3.3b");
        self.madlad_model = root.join("madlad400-3b-mt");
        self.small_model = root.join("small100");
        self.chat_model = root.join("qwen2.5-1.5b-instruct");
        self.base_dir = root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf());
        self
    }
}

/// Platform-native model root used when no `config.json` exists.
///
/// `directories` resolves the user data directory per platform:
/// Linux follows XDG, Windows uses per-user application data, and macOS uses
/// Application Support. The model subdirectory is kept outside the
/// application bundle and never depends on the process working directory.
pub fn default_model_root() -> PathBuf {
    let data_dir = directories::ProjectDirs::from("io", "fluent", "Fluent")
        .expect("Fluent requires a user data directory")
        .data_dir()
        .to_path_buf();
    data_dir.join("models")
}

/// Create a storage root and prove that it is writable.
///
/// The probe is removed before returning. The caller receives the underlying
/// I/O error so it can show the exact path and cause.
pub fn ensure_model_root(root: &Path) -> std::io::Result<()> {
    fs::create_dir_all(root)?;
    let probe = root.join(format!(".fluent-write-test-{}", std::process::id()));
    fs::write(&probe, b"ok")?;
    fs::remove_file(&probe)
}

#[derive(Debug, Deserialize, Default)]
struct RawConfig {
    default_source_language: Option<String>,
    default_target_language: Option<String>,
    default_mode: Option<String>,
    theme: Option<String>,
    models: Option<RawModels>,
    generation: Option<RawGeneration>,
}

#[derive(Debug, Deserialize, Default)]
struct RawModels {
    fast: Option<String>,
    accurate: Option<String>,
    madlad: Option<String>,
    small: Option<String>,
    chat: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct RawGeneration {
    max_length: Option<usize>,
    max_new_tokens: Option<usize>,
    /// Explicit opt-in for the no-Fluent-ceiling mode. Kept separate from
    /// `max_new_tokens` so older files with a numeric value are unchanged.
    unlimited_output: Option<bool>,
    num_beams: Option<usize>,
}

impl ProjectConfig {
    /// Loads `config.json`, searching in order:
    /// `FLUENT_CONFIG`, the executable directory, its parent, the platform
    /// configuration directory, then the current directory.
    ///
    /// The platform directory is the same one the GUI uses for
    /// `settings.json` (`~/.config/fluent` on Linux,
    /// `~/Library/Application Support/io.fluent.Fluent` on macOS, and
    /// `%APPDATA%\fluent\Fluent\config` on Windows). Keeping the installed
    /// configuration there lets application updates replace the program files
    /// without touching user-owned configuration.
    pub fn load() -> Self {
        for candidate in config_candidates() {
            if candidate.is_file() {
                match Self::load_from(&candidate) {
                    Ok(config) => return config,
                    Err(error) => {
                        eprintln!(
                            "Fluent: could not read {} ({error}), using defaults.",
                            candidate.display()
                        );
                    }
                }
            }
        }

        // No config.json: use the platform data directory by default. A
        // populated portable model directory from an earlier run is adopted
        // only when the platform root is empty, so existing users keep access
        // without moving or duplicating model files.
        let mut config = Self::default();
        if let Some(root) = discover_portable_model_root() {
            eprintln!("Fluent: using existing model directory {}", root.display());
            config = config.with_model_root(&root);
        }
        config
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let raw_text = std::fs::read_to_string(path)?;
        let raw: RawConfig = serde_json::from_str(&raw_text)?;
        let base_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        let mut config = Self::for_model_root(&base_dir.join("models"));
        config.config_path = Some(path.to_path_buf());

        if let Some(value) = raw.default_source_language {
            config.default_source_language = value;
        }
        if let Some(value) = raw.default_target_language {
            config.default_target_language = value;
        }
        if let Some(value) = raw.default_mode {
            if let Ok(mode) = value.parse() {
                config.default_mode = mode;
            }
        }
        if let Some(value) = raw.theme {
            config.theme = value;
        }
        if let Some(models) = raw.models {
            if let Some(fast) = models.fast {
                config.fast_model = resolve_model_path(&base_dir, &fast);
            }
            if let Some(accurate) = models.accurate {
                config.accurate_model = resolve_model_path(&base_dir, &accurate);
            }
            if let Some(madlad) = models.madlad {
                config.madlad_model = resolve_model_path(&base_dir, &madlad);
            }
            if let Some(small) = models.small {
                config.small_model = resolve_model_path(&base_dir, &small);
            }
            if let Some(chat) = models.chat {
                config.chat_model = resolve_model_path(&base_dir, &chat);
            }
        }
        if let Some(generation) = raw.generation {
            if let Some(max_length) = generation.max_length {
                config.max_length = max_length.clamp(16, 1024);
            }
            if generation.unlimited_output.unwrap_or(false) {
                config.output_limit = OutputLimit::UntilEos;
            } else if let Some(max_new_tokens) = generation.max_new_tokens {
                config.output_limit = OutputLimit::Tokens(max_new_tokens.clamp(16, 1024));
            }
            if let Some(num_beams) = generation.num_beams {
                config.num_beams = num_beams.clamp(1, 8);
            }
        }

        if !config.fast_model.is_dir() {
            let fallback = base_dir.join("models/nllb-200-1.3b");
            if fallback.is_dir() {
                config.fast_model = fallback;
            }
        }
        if !config.accurate_model.is_dir() {
            let fallback = base_dir.join("models/nllb-200-3.3b");
            if fallback.is_dir() {
                config.accurate_model = fallback;
            }
        }
        if !config.madlad_model.is_dir() {
            let fallback = base_dir.join("models/madlad400-3b-mt");
            if fallback.is_dir() {
                config.madlad_model = fallback;
            }
        }
        if !config.madlad_model.is_dir() {
            if let Some(dirs) = directories::ProjectDirs::from("io", "fluent", "Fluent") {
                let fallback = dirs.data_dir().join("models/madlad400-3b-mt");
                if fallback.is_dir() {
                    config.madlad_model = fallback;
                }
            }
        }
        if !config.small_model.is_dir() {
            let fallback = base_dir.join("models/small100");
            if fallback.is_dir() {
                config.small_model = fallback;
            }
        }
        if !config.small_model.is_dir() {
            if let Some(dirs) = directories::ProjectDirs::from("io", "fluent", "Fluent") {
                let fallback = dirs.data_dir().join("models/small100");
                if fallback.is_dir() {
                    config.small_model = fallback;
                }
            }
        }
        if !config.chat_model.is_dir() {
            let fallback = base_dir.join("models/qwen2.5-1.5b-instruct");
            if fallback.is_dir() {
                config.chat_model = fallback;
            }
        }
        if !config.chat_model.is_dir() {
            if let Some(dirs) = directories::ProjectDirs::from("io", "fluent", "Fluent") {
                let fallback = dirs.data_dir().join("models/qwen2.5-1.5b-instruct");
                if fallback.is_dir() {
                    config.chat_model = fallback;
                }
            }
        }

        Ok(config)
    }

    /// Directory for a chat model id. The default 1.5B model uses the explicit
    /// `chat_model` path; other chat models live beside it.
    pub fn chat_model_path(&self, id: &str) -> PathBuf {
        let spec = chat_model_spec(id).unwrap_or_else(default_chat_model);
        if spec.default_dir == default_chat_model().default_dir {
            self.chat_model.clone()
        } else {
            self.chat_model
                .parent()
                .map(|parent| parent.join(spec.default_dir))
                .unwrap_or_else(|| self.base_dir.join(spec.default_dir))
        }
    }

    pub fn model_path(&self, mode: Mode) -> &Path {
        match mode {
            Mode::Fast => &self.fast_model,
            Mode::Accurate => &self.accurate_model,
            Mode::Madlad => &self.madlad_model,
            Mode::Small => &self.small_model,
        }
    }

    pub fn validate(&self) -> Result<()> {
        for mode in [Mode::Fast, Mode::Accurate] {
            let path = self.model_path(mode);
            if !path.exists() {
                return Err(Error::ModelNotFound(path.to_path_buf()));
            }
        }
        Ok(())
    }
}

fn config_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(value) = std::env::var("FLUENT_CONFIG") {
        if !value.trim().is_empty() {
            candidates.push(PathBuf::from(value));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("config.json"));
            if let Some(parent) = dir.parent() {
                candidates.push(parent.join("config.json"));
            }
        }
    }
    if let Some(dirs) = directories::ProjectDirs::from("io", "fluent", "Fluent") {
        candidates.push(dirs.config_dir().join("config.json"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("config.json"));
    }
    candidates
}

/// Existing portable model roots, in priority order. These are read-only
/// discovery candidates: nothing is moved or deleted.
fn portable_model_candidates(
    exe: Option<&Path>,
    cwd: Option<&Path>,
    home: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut push = |path: PathBuf| {
        if !candidates.contains(&path) {
            candidates.push(path);
        }
    };

    if let Some(exe) = exe.filter(|path| !is_app_bundle_path(path)) {
        if let Some(dir) = exe.parent() {
            push(dir.join("models"));
            if let Some(parent) = dir.parent() {
                push(parent.join("models"));
            }
        }
    }

    // Preserve the old portable launch pattern only when the process was
    // started from the executable directory or the archive root. An
    // arbitrary terminal working directory is not evidence of a portable
    // installation.
    if let (Some(exe), Some(cwd)) = (exe, cwd) {
        if !is_app_bundle_path(cwd) {
            if let Some(exe_dir) = exe.parent() {
                let exe_parent = exe_dir.parent();
                if cwd == exe_dir || exe_parent == Some(cwd) {
                    push(cwd.join("models"));
                }
            }
        }
    }

    // Historical installer layout. It is only adopted when populated and no
    // newer location wins.
    if let Some(home) = home {
        push(home.join(".fluent/models"));
    }

    candidates
}

fn is_app_bundle_path(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str().to_string_lossy().ends_with(".app"))
}

fn is_safe_portable_root(root: &Path) -> bool {
    // Reject /models, C:\models and other paths whose parent is a filesystem
    // root. They are not user-owned portable locations.
    root.parent().and_then(Path::parent).is_some() && !is_app_bundle_path(root)
}

fn model_root_is_populated(root: &Path) -> bool {
    MODEL_DIRS.iter().any(|name| {
        let dir = root.join(name);
        dir.is_dir()
            && fs::read_dir(&dir)
                .map(|mut entries| entries.next().is_some())
                .unwrap_or(false)
    })
}

fn populated_portable_root(
    default_root: &Path,
    candidates: impl IntoIterator<Item = PathBuf>,
) -> Option<PathBuf> {
    let default_canonical = default_root
        .canonicalize()
        .unwrap_or_else(|_| default_root.to_path_buf());
    if model_root_is_populated(&default_canonical) {
        return None;
    }
    for candidate in candidates {
        let Ok(canonical) = candidate.canonicalize() else {
            continue;
        };
        if canonical == default_canonical || !is_safe_portable_root(&canonical) {
            continue;
        }
        if model_root_is_populated(&canonical) {
            return Some(canonical);
        }
    }
    None
}

fn discover_portable_model_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok();
    let cwd = std::env::current_dir().ok();
    let home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
    populated_portable_root(
        &default_model_root(),
        portable_model_candidates(exe.as_deref(), cwd.as_deref(), home.as_deref()),
    )
}

fn resolve_model_path(base_dir: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let unique = format!(
            "fluent-config-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn candidates_include_the_platform_config_directory() {
        // The installer writes config.json to the platform configuration
        // directory so that application updates do not touch user-owned files.
        let expected = directories::ProjectDirs::from("io", "fluent", "Fluent")
            .expect("home directory")
            .config_dir()
            .join("config.json");
        assert!(
            config_candidates().contains(&expected),
            "platform config candidate missing: {}",
            expected.display()
        );
    }

    #[test]
    fn relative_paths_resolve_against_the_config_directory() {
        let dir = temp_dir("relative");
        fs::create_dir_all(dir.join("models/nllb-200-1.3b")).unwrap();
        fs::create_dir_all(dir.join("models/nllb-200-3.3b")).unwrap();
        fs::create_dir_all(dir.join("models/madlad400-3b-mt")).unwrap();
        fs::write(
            dir.join("config.json"),
            r#"{
                "default_source_language": "auto",
                "default_target_language": "fra_Latn",
                "models": {"fast": "models/nllb-200-1.3b", "accurate": "models/nllb-200-3.3b", "madlad": "models/madlad400-3b-mt"},
                "generation": {"max_length": 5000, "max_new_tokens": 5000, "num_beams": 42}
            }"#,
        )
        .unwrap();

        let config = ProjectConfig::load_from(&dir.join("config.json")).unwrap();
        assert_eq!(config.base_dir, dir);
        assert_eq!(config.fast_model, dir.join("models/nllb-200-1.3b"));
        assert_eq!(config.accurate_model, dir.join("models/nllb-200-3.3b"));
        assert_eq!(config.madlad_model, dir.join("models/madlad400-3b-mt"));
        assert_eq!(config.default_target_language, "fra_Latn");
        // Out-of-range values are clamped.
        assert_eq!(config.max_length, 1024);
        assert_eq!(config.output_limit, OutputLimit::Tokens(1024));
        assert_eq!(config.num_beams, 8);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_model_directory_falls_back_to_the_conventional_name() {
        let dir = temp_dir("fallback");
        fs::create_dir_all(dir.join("models/nllb-200-1.3b")).unwrap();
        fs::create_dir_all(dir.join("models/nllb-200-3.3b")).unwrap();
        fs::create_dir_all(dir.join("models/madlad400-3b-mt")).unwrap();
        fs::write(
            dir.join("config.json"),
            r#"{"models": {"fast": "elsewhere", "accurate": "elsewhere-too"}}"#,
        )
        .unwrap();

        let config = ProjectConfig::load_from(&dir.join("config.json")).unwrap();
        assert_eq!(config.fast_model, dir.join("models/nllb-200-1.3b"));
        assert_eq!(config.accurate_model, dir.join("models/nllb-200-3.3b"));
        assert_eq!(config.madlad_model, dir.join("models/madlad400-3b-mt"));
        assert_eq!(config.output_limit, OutputLimit::Auto);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn invalid_generation_values_are_ignored() {
        let dir = temp_dir("generation");
        fs::write(
            dir.join("config.json"),
            r#"{"default_mode": "unknown", "generation": {"max_length": 1, "max_new_tokens": 1, "num_beams": 0}}"#,
        )
        .unwrap();

        let config = ProjectConfig::load_from(&dir.join("config.json")).unwrap();
        assert_eq!(config.default_mode, Mode::Fast);
        assert_eq!(config.max_length, 16);
        assert_eq!(config.output_limit, OutputLimit::Tokens(16));
        assert_eq!(config.num_beams, 1);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn unlimited_output_is_an_explicit_opt_in() {
        let dir = temp_dir("unlimited");
        fs::write(
            dir.join("config.json"),
            r#"{"generation": {"unlimited_output": true, "max_new_tokens": 256}}"#,
        )
        .unwrap();
        let config = ProjectConfig::load_from(&dir.join("config.json")).unwrap();
        assert_eq!(config.output_limit, OutputLimit::UntilEos);
        fs::remove_dir_all(dir).ok();
    }

    fn create_model_marker(root: &Path) {
        let dir = root.join("nllb-200-1.3b");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), b"{}").unwrap();
    }

    #[test]
    fn default_model_paths_are_platform_native_and_cwd_independent() {
        let root = default_model_root();
        assert!(root.is_absolute(), "default root is not absolute: {root:?}");
        assert!(root.ends_with("models"), "unexpected root: {root:?}");

        let config = ProjectConfig::default();
        assert_eq!(config.fast_model, root.join("nllb-200-1.3b"));
        assert_eq!(config.accurate_model, root.join("nllb-200-3.3b"));
        assert_eq!(config.madlad_model, root.join("madlad400-3b-mt"));
        assert_eq!(config.small_model, root.join("small100"));
        assert_eq!(config.chat_model, root.join("qwen2.5-1.5b-instruct"));
        assert_eq!(
            config.fast_model.parent(),
            Some(root.as_path()),
            "model paths must not be under the process working directory"
        );
    }

    #[test]
    fn model_paths_preserve_spaces_and_unicode() {
        let dir = temp_dir("paths");
        let root = dir.join("Models with spaces").join("日本語");
        let config = ProjectConfig::for_model_root(&root);
        assert_eq!(config.fast_model, root.join("nllb-200-1.3b"));
        assert_eq!(config.chat_model, root.join("qwen2.5-1.5b-instruct"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn absolute_configured_paths_take_precedence_and_keep_unicode() {
        let dir = temp_dir("configured");
        let fast = dir.join("models with spaces").join("日本語");
        fs::create_dir_all(&fast).unwrap();
        let raw = serde_json::json!({"models": {"fast": fast}});
        fs::write(dir.join("config.json"), serde_json::to_vec(&raw).unwrap()).unwrap();

        let config = ProjectConfig::load_from(&dir.join("config.json")).unwrap();
        assert_eq!(config.fast_model, fast);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn portable_archive_root_is_detected_from_the_executable() {
        let archive = temp_dir("portable");
        let models = archive.join("models");
        create_model_marker(&models);
        let exe = archive.join("bin/fluent");
        let candidates = portable_model_candidates(Some(&exe), Some(&archive), None);
        assert!(candidates.contains(&models));

        let default_root = temp_dir("platform-empty");
        let adopted = models.canonicalize().unwrap();
        assert_eq!(
            populated_portable_root(&default_root, candidates),
            Some(adopted.clone())
        );
        let config = ProjectConfig::default().with_model_root(&adopted);
        assert_eq!(config.detected_model_root, Some(adopted));
        fs::remove_dir_all(archive).ok();
        fs::remove_dir_all(default_root).ok();
    }

    #[test]
    fn arbitrary_working_directory_is_not_adopted() {
        let cwd = temp_dir("arbitrary-cwd");
        let exe_dir = temp_dir("other-exe");
        create_model_marker(&cwd.join("models"));
        let exe = exe_dir.join("fluent");
        let candidates = portable_model_candidates(Some(&exe), Some(&cwd), None);
        assert!(
            !candidates.contains(&cwd.join("models")),
            "an unrelated working directory must not become a model root"
        );
        fs::remove_dir_all(cwd).ok();
        fs::remove_dir_all(exe_dir).ok();
    }

    #[test]
    fn app_bundle_executables_do_not_use_bundle_or_finder_paths() {
        let exe = Path::new("/Applications/Fluent.app/Contents/MacOS/fluent");
        let candidates = portable_model_candidates(Some(exe), Some(Path::new("/")), None);
        assert!(
            candidates.is_empty(),
            "unexpected candidates: {candidates:?}"
        );
        assert!(!is_safe_portable_root(Path::new("/models")));
    }

    #[test]
    fn populated_platform_root_wins_over_portable_roots() {
        let platform = temp_dir("platform-populated");
        create_model_marker(&platform);
        let portable = temp_dir("portable-other");
        create_model_marker(&portable.join("models"));
        assert_eq!(
            populated_portable_root(&platform, vec![portable.join("models")]),
            None
        );
        fs::remove_dir_all(platform).ok();
        fs::remove_dir_all(portable).ok();
    }

    #[test]
    fn legacy_home_models_is_a_candidate() {
        let home = temp_dir("legacy-home");
        let legacy = home.join(".fluent/models");
        create_model_marker(&legacy);
        let candidates = portable_model_candidates(None, None, Some(&home));
        assert!(candidates.contains(&legacy));
        fs::remove_dir_all(home).ok();
    }

    #[test]
    fn ensure_model_root_creates_and_probes_the_directory() {
        let dir = temp_dir("ensure");
        let root = dir.join("nested with spaces").join("日本語");
        ensure_model_root(&root).unwrap();
        assert!(root.is_dir());
        assert!(
            fs::read_dir(&root).unwrap().next().is_none(),
            "write probe was not removed"
        );

        let blocked = dir.join("not-a-directory");
        fs::write(&blocked, b"file").unwrap();
        assert!(ensure_model_root(&blocked).is_err());
        fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn ensure_model_root_reports_a_read_only_parent() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("readonly");
        let parent = dir.join("read-only");
        fs::create_dir_all(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o500)).unwrap();

        // Root can bypass permission bits. Skip the assertion there rather
        // than making the test depend on the CI user.
        let probe = parent.join(".probe");
        let can_write = fs::write(&probe, b"x").is_ok();
        if can_write {
            fs::remove_file(probe).ok();
        } else {
            assert!(ensure_model_root(&parent.join("models")).is_err());
        }

        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(dir).ok();
    }
}

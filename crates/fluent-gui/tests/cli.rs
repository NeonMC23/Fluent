use std::path::PathBuf;
use std::process::Command;

fn temp_dir() -> PathBuf {
    let unique = format!(
        "fluent-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let path = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn version_flag_reports_the_workspace_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_fluent"))
        .arg("--version")
        .output()
        .expect("run fluent --version");
    assert!(output.status.success(), "exit status: {}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("Fluent "), "unexpected output: {stdout}");
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "version missing from: {stdout}"
    );
}

#[test]
fn self_test_succeeds_without_models_or_user_configuration() {
    let dir = temp_dir();
    let output = Command::new(env!("CARGO_BIN_EXE_fluent"))
        .arg("--self-test")
        .current_dir(&dir)
        .env("HOME", &dir)
        .env("XDG_CONFIG_HOME", dir.join(".config"))
        .env("XDG_DATA_HOME", dir.join(".local/share"))
        .env_remove("FLUENT_CONFIG")
        .output()
        .expect("run fluent --self-test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "self-test failed: {stdout}{stderr}"
    );
    assert!(stdout.contains("window-icon: OK"), "{stdout}");
    assert!(stdout.contains("languages: OK"), "{stdout}");
    assert!(stdout.contains("SELF-TEST OK"), "{stdout}");
    std::fs::remove_dir_all(&dir).ok();
}
#[test]
fn help_describes_translation_and_local_chat() {
    let output = Command::new(env!("CARGO_BIN_EXE_fluent"))
        .arg("--help")
        .output()
        .expect("run fluent --help");
    assert!(output.status.success(), "exit status: {}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("local multilingual translation and chat"),
        "product description missing from help:\n{stdout}"
    );
    assert!(
        stdout.contains("local instruction models"),
        "local chat description missing from help:\n{stdout}"
    );
    assert!(
        stdout.contains("Model Manager"),
        "model acquisition description missing from help:\n{stdout}"
    );
    assert!(
        stdout.contains("inference runs locally"),
        "local inference description missing from help:\n{stdout}"
    );
}

#[test]
fn self_test_uses_xdg_data_home_instead_of_the_working_directory() {
    let dir = temp_dir();
    let home = dir.join("home");
    let data = dir.join("data");
    let cwd = dir.join("cwd");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(cwd.join("models/nllb-200-1.3b")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_fluent"))
        .arg("--self-test")
        .current_dir(&cwd)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", &data)
        .env_remove("FLUENT_CONFIG")
        .output()
        .expect("run fluent --self-test");

    assert!(output.status.success(), "self-test failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected = data.join("fluent/models/nllb-200-1.3b");
    assert!(
        stdout.contains(&expected.display().to_string()),
        "XDG model path missing from self-test:\n{stdout}"
    );
    assert!(
        !stdout.contains(&cwd.join("models").display().to_string()),
        "working-directory models must not be used:\n{stdout}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(target_os = "linux")]
#[test]
fn self_test_falls_back_to_home_local_share_without_xdg_data_home() {
    let dir = temp_dir();
    let home = dir.join("home");
    let cwd = dir.join("cwd");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_fluent"))
        .arg("--self-test")
        .current_dir(&cwd)
        .env("HOME", &home)
        .env_remove("XDG_DATA_HOME")
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env_remove("FLUENT_CONFIG")
        .output()
        .expect("run fluent --self-test");

    assert!(output.status.success(), "self-test failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected = home.join(".local/share/fluent/models/nllb-200-1.3b");
    assert!(
        stdout.contains(&expected.display().to_string()),
        "HOME fallback missing from self-test:\n{stdout}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(target_os = "macos")]
#[test]
fn self_test_uses_application_support_on_macos() {
    let dir = temp_dir();
    let home = dir.join("home");
    let cwd = dir.join("cwd");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_fluent"))
        .arg("--self-test")
        .current_dir(&cwd)
        .env("HOME", &home)
        .env_remove("FLUENT_CONFIG")
        .output()
        .expect("run fluent --self-test");

    assert!(output.status.success(), "self-test failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Library/Application Support/io.fluent.Fluent/models/nllb-200-1.3b"),
        "Application Support model path missing from self-test:\n{stdout}"
    );
    assert!(
        !stdout.contains(&cwd.join("models").display().to_string()),
        "working-directory models must not be used:\n{stdout}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(target_os = "windows")]
#[test]
fn self_test_uses_per_user_appdata_on_windows() {
    let dir = temp_dir();
    let cwd = dir.join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_fluent"))
        .arg("--self-test")
        .current_dir(&cwd)
        .env_remove("FLUENT_CONFIG")
        .output()
        .expect("run fluent --self-test");

    assert!(output.status.success(), "self-test failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected_suffix = std::path::Path::new("Fluent")
        .join("data")
        .join("models")
        .join("nllb-200-1.3b");
    assert!(
        stdout.contains(&expected_suffix.display().to_string()),
        "per-user AppData model path missing from self-test:\n{stdout}"
    );
    assert!(
        !stdout.contains(&cwd.join("models").display().to_string()),
        "working-directory models must not be used:\n{stdout}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn explicit_config_model_root_takes_precedence_over_defaults() {
    let dir = temp_dir();
    let models = dir.join("models with spaces").join("日本語");
    let cwd = dir.join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&models).unwrap();
    let config = dir.join("fluent config.json");
    let raw = serde_json::json!({"models": {"fast": models}});
    std::fs::write(&config, serde_json::to_vec(&raw).unwrap()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_fluent"))
        .arg("--self-test")
        .current_dir(&cwd)
        .env("FLUENT_CONFIG", &config)
        .env("HOME", dir.join("home"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .output()
        .expect("run fluent --self-test");

    assert!(output.status.success(), "self-test failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&models.display().to_string()),
        "explicit config path missing from self-test:\n{stdout}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

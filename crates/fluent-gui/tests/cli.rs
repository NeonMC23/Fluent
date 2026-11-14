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

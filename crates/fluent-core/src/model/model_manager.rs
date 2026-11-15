//! Native model installation, verification, storage and removal.
//!
//! The catalogue lives in [`super::registry`]; this module only uses the
//! registered required-file manifests. Downloads are resumable, verified before
//! they replace a final file, and never write inside the Git repository.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use fs2::available_space;
use sha2::{Digest, Sha256};

use crate::model::registry::{model_spec, ModelFile, ModelSpec};
use crate::{Error, Mode, Result};

const DISK_MARGIN: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallState {
    NotInstalled,
    Incomplete,
    Installed,
    Corrupt,
}

impl InstallState {
    pub fn label(self) -> &'static str {
        match self {
            Self::NotInstalled => "Not installed",
            Self::Incomplete => "Incomplete",
            Self::Installed => "Installed",
            Self::Corrupt => "Needs repair",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileState {
    pub file: ModelFile,
    pub actual_size: Option<u64>,
    pub sha_ok: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ModelInstallation {
    pub mode: Mode,
    pub dir: PathBuf,
    pub state: InstallState,
    pub files: Vec<FileState>,
    pub installed_bytes: u64,
    pub required_bytes: u64,
    /// True only if every required file's SHA-256 was checked in this call.
    pub verified: bool,
}

impl ModelInstallation {
    pub fn missing_files(&self) -> Vec<&ModelFile> {
        self.files
            .iter()
            .filter(|file| file.actual_size != Some(file.file.size))
            .map(|file| &file.file)
            .collect()
    }

    pub fn bad_hashes(&self) -> Vec<&ModelFile> {
        self.files
            .iter()
            .filter(|file| file.sha_ok == Some(false))
            .map(|file| &file.file)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStage {
    Downloading,
    Verifying,
}

impl TaskStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Downloading => "Downloading",
            Self::Verifying => "Verifying",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaskProgress {
    pub mode: Mode,
    pub stage: TaskStage,
    pub file: Option<String>,
    pub file_index: usize,
    pub file_count: usize,
    pub completed_bytes: u64,
    pub total_bytes: u64,
    pub file_bytes: u64,
    pub file_total: u64,
    pub speed_bps: f64,
    pub eta_seconds: Option<u64>,
}

pub fn inspect_model(mode: Mode, dir: &Path) -> ModelInstallation {
    let spec = model_spec(mode);
    let mut files = Vec::with_capacity(spec.files.len());
    let mut installed_bytes = 0u64;
    let mut all_present = true;
    for file in spec.files {
        let path = dir.join(file.name);
        let actual_size = fs::metadata(&path)
            .ok()
            .map(|meta| meta.len())
            .filter(|_| path.is_file());
        if let Some(size) = actual_size {
            installed_bytes = installed_bytes.saturating_add(size);
        }
        if actual_size != Some(file.size) {
            all_present = false;
        }
        files.push(FileState {
            file: *file,
            actual_size,
            sha_ok: None,
        });
    }
    let state =
        if !dir.is_dir() || installed_bytes == 0 && files.iter().all(|f| f.actual_size.is_none()) {
            InstallState::NotInstalled
        } else if all_present {
            InstallState::Installed
        } else {
            InstallState::Incomplete
        };
    ModelInstallation {
        mode,
        dir: dir.to_path_buf(),
        state,
        files,
        installed_bytes,
        required_bytes: spec.download_bytes(),
        verified: false,
    }
}

pub fn verify_model(
    mode: Mode,
    dir: &Path,
    progress: &mut dyn FnMut(TaskProgress),
    cancel: &AtomicBool,
) -> Result<ModelInstallation> {
    let spec = model_spec(mode);
    let mut installation = inspect_model(mode, dir);
    if installation.state == InstallState::NotInstalled {
        return Ok(installation);
    }
    let total_bytes = spec.download_bytes();
    let mut completed = 0u64;
    let started = Instant::now();
    for (index, file) in spec.files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let path = dir.join(file.name);
        let Some(state) = installation
            .files
            .iter_mut()
            .find(|state| state.file.name == file.name)
        else {
            continue;
        };
        if !path.is_file() || fs::metadata(&path).map(|m| m.len()).ok() != Some(file.size) {
            state.sha_ok = Some(false);
            completed = completed.saturating_add(file.size);
            continue;
        }
        let hash = hash_file(
            &path,
            completed,
            total_bytes,
            file,
            index,
            spec.files.len(),
            mode,
            TaskStage::Verifying,
            started,
            progress,
            cancel,
        )?;
        let ok = hash.eq_ignore_ascii_case(file.sha256);
        state.sha_ok = Some(ok);
        completed = completed.saturating_add(file.size);
    }
    installation.verified = true;
    installation.state = if installation
        .files
        .iter()
        .all(|file| file.actual_size == Some(file.file.size) && file.sha_ok == Some(true))
    {
        InstallState::Installed
    } else if installation
        .files
        .iter()
        .all(|file| file.actual_size == Some(file.file.size))
    {
        InstallState::Corrupt
    } else {
        InstallState::Incomplete
    };
    Ok(installation)
}

pub fn install_model(
    mode: Mode,
    dir: &Path,
    progress: &mut dyn FnMut(TaskProgress),
    cancel: &AtomicBool,
) -> Result<ModelInstallation> {
    let spec = model_spec(mode);
    fs::create_dir_all(dir)?;
    let total_bytes = spec.download_bytes();
    let mut completed = 0u64;

    // Check which files are already valid, and preserve invalid files instead
    // of deleting any user bytes. A failed download leaves the valid set
    // intact.
    let mut plan: Vec<(&ModelFile, PathBuf, u64)> = Vec::new();
    for file in spec.files {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let path = dir.join(file.name);
        if path.is_file() && fs::metadata(&path).map(|m| m.len()).ok() == Some(file.size) {
            let actual = hash_path(&path)?;
            if actual.eq_ignore_ascii_case(file.sha256) {
                completed = completed.saturating_add(file.size);
                continue;
            }
        }
        if path.is_file() {
            move_to_invalid(&path)?;
        }
        let part = part_path(&path);
        let part_size = fs::metadata(&part)
            .map(|m| m.len())
            .unwrap_or(0)
            .min(file.size);
        completed = completed.saturating_add(part_size);
        plan.push((file, path, part_size));
    }

    let remaining: u64 = plan
        .iter()
        .map(|(file, _, part)| file.size.saturating_sub(*part))
        .sum();
    let free = available_space(dir)?;
    if remaining.saturating_add(DISK_MARGIN) > free {
        return Err(Error::Other(format!(
            "not enough disk space: need at least {} MB, only {} MB free",
            remaining.saturating_add(DISK_MARGIN) / (1024 * 1024),
            free / (1024 * 1024)
        )));
    }

    for (index, (file, target, part_size)) in plan.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        download_file(
            spec,
            file,
            target,
            *part_size,
            completed.saturating_sub(*part_size),
            total_bytes,
            index,
            plan.len(),
            progress,
            cancel,
        )?;
        completed = completed.saturating_add(file.size.saturating_sub(*part_size));
    }

    // A complete hash pass is the only state that may be called installed.
    let installation = verify_model(mode, dir, progress, cancel)?;
    if installation.state != InstallState::Installed {
        return Err(Error::Other(format!(
            "model verification failed after download: {}",
            installation.state.label()
        )));
    }
    Ok(installation)
}

pub fn remove_model(mode: Mode, dir: &Path, models_root: &Path) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    let root = fs::canonicalize(models_root)?;
    let target = fs::canonicalize(dir)?;
    let spec = model_spec(mode);
    if target == root || !target.starts_with(&root) {
        return Err(Error::Other(format!(
            "refusing to remove a model outside the managed storage root: {}",
            target.display()
        )));
    }
    if target.file_name().and_then(|name| name.to_str()) != Some(spec.default_dir) {
        return Err(Error::Other(format!(
            "refusing to remove an unexpected model directory: {}",
            target.display()
        )));
    }
    fs::remove_dir_all(&target)?;
    Ok(())
}

pub fn directory_size(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        } else if meta.is_dir() {
            total = total.saturating_add(directory_size(&path));
        } else if meta.is_file() {
            total = total.saturating_add(meta.len());
        }
    }
    total
}

pub fn available_bytes(path: &Path) -> Result<u64> {
    Ok(available_space(path)?)
}

pub fn model_dir(root: &Path, mode: Mode) -> PathBuf {
    root.join(model_spec(mode).default_dir)
}

fn part_path(path: &Path) -> PathBuf {
    path.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            "{}.part",
            path.file_name().unwrap_or_default().to_string_lossy()
        ))
}

fn move_to_invalid(path: &Path) -> Result<()> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let backup = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            "{}.invalid-{stamp}",
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
    fs::rename(path, backup)?;
    Ok(())
}

fn hash_path(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[allow(clippy::too_many_arguments)]
fn hash_file(
    path: &Path,
    completed: u64,
    total: u64,
    file: &ModelFile,
    index: usize,
    file_count: usize,
    mode: Mode,
    stage: TaskStage,
    started: Instant,
    progress: &mut dyn FnMut(TaskProgress),
    cancel: &AtomicBool,
) -> Result<String> {
    let mut handle = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut done = 0u64;
    while done < file.size {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let count = handle.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        done = done.saturating_add(count as u64);
        let elapsed = started.elapsed().as_secs_f64().max(0.001);
        progress(TaskProgress {
            mode,
            stage,
            file: Some(file.name.to_string()),
            file_index: index,
            file_count,
            completed_bytes: completed.saturating_add(done),
            total_bytes: total,
            file_bytes: done,
            file_total: file.size,
            speed_bps: (completed + done) as f64 / elapsed,
            eta_seconds: if done < file.size {
                Some(
                    ((file.size - done) as f64 / ((completed + done) as f64 / elapsed).max(1.0))
                        as u64,
                )
            } else {
                None
            },
        });
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[allow(clippy::too_many_arguments)]
fn download_file(
    spec: &ModelSpec,
    file: &ModelFile,
    target: &Path,
    part_size: u64,
    completed: u64,
    total: u64,
    file_index: usize,
    file_count: usize,
    progress: &mut dyn FnMut(TaskProgress),
    cancel: &AtomicBool,
) -> Result<()> {
    let url = format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        spec.repo, spec.revision, file.name
    );
    download_url(
        spec, file, target, part_size, completed, total, file_index, file_count, &url, progress,
        cancel,
    )
}

#[allow(clippy::too_many_arguments)]
fn download_url(
    spec: &ModelSpec,
    file: &ModelFile,
    target: &Path,
    part_size: u64,
    completed: u64,
    total: u64,
    file_index: usize,
    file_count: usize,
    url: &str,
    progress: &mut dyn FnMut(TaskProgress),
    cancel: &AtomicBool,
) -> Result<()> {
    let part = part_path(target);
    // Large checkpoints can legitimately take many minutes, so there is no
    // global deadline. Connect/response and per-read inactivity limits still
    // prevent a stalled connection from hanging the manager indefinitely.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(std::time::Duration::from_secs(30)))
        .timeout_recv_response(Some(std::time::Duration::from_secs(30)))
        .timeout_per_read(Some(std::time::Duration::from_secs(60)))
        .build()
        .into();
    let mut request = agent.get(url);
    if part_size > 0 {
        request = request.header("Range", &format!("bytes={part_size}-"));
    }
    let mut response = request
        .call()
        .map_err(|error| Error::Other(format!("download failed for {}: {error}", file.name)))?;
    let status = response.status().as_u16();
    let append = status == 206 && part_size > 0;
    if status == 416 {
        if fs::metadata(&part).map(|m| m.len()).unwrap_or(0) != file.size {
            return Err(Error::Other(format!(
                "server rejected resume for {}; remove the .part file and retry",
                file.name
            )));
        }
        // The complete .part is verified and promoted without touching it.
        let actual = hash_path(&part)?;
        if actual.eq_ignore_ascii_case(file.sha256) {
            fs::rename(&part, target)?;
            return Ok(());
        }
        return Err(Error::Other(format!(
            "checksum mismatch for {}; the complete .part file was kept",
            file.name
        )));
    } else if status != 200 && status != 206 {
        return Err(Error::Other(format!(
            "download failed for {}: HTTP {status}",
            file.name
        )));
    }
    if status == 200 && part_size > 0 {
        // The server ignored Range; restart this file safely.
        fs::remove_file(&part).ok();
    }

    let mut handle = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&part)?;
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(file.size + 1)
        .reader();
    let mut done = if append { part_size } else { 0 };
    let mut buffer = vec![0u8; 1024 * 1024];
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let count = reader.read(&mut buffer).map_err(|error| {
            Error::Other(format!("download interrupted for {}: {error}", file.name))
        })?;
        if count == 0 {
            break;
        }
        handle.write_all(&buffer[..count])?;
        done = done.saturating_add(count as u64);
        if done > file.size {
            return Err(Error::Other(format!(
                "server sent too much data for {}",
                file.name
            )));
        }
        let elapsed = started.elapsed().as_secs_f64().max(0.001);
        let total_done = completed.saturating_add(done);
        let speed = (done.saturating_sub(if append { part_size } else { 0 })) as f64 / elapsed;
        progress(TaskProgress {
            mode: spec.mode,
            stage: TaskStage::Downloading,
            file: Some(file.name.to_string()),
            file_index,
            file_count,
            completed_bytes: total_done,
            total_bytes: total,
            file_bytes: done,
            file_total: file.size,
            speed_bps: speed,
            eta_seconds: if speed > 0.0 {
                Some((total.saturating_sub(total_done) as f64 / speed) as u64)
            } else {
                None
            },
        });
    }
    handle.flush()?;
    drop(handle);
    if fs::metadata(&part)?.len() != file.size {
        return Err(Error::Other(format!(
            "incomplete download for {}: expected {} bytes, got {}",
            file.name,
            file.size,
            fs::metadata(&part)?.len()
        )));
    }
    let actual = hash_path(&part)?;
    if !actual.eq_ignore_ascii_case(file.sha256) {
        return Err(Error::Other(format!(
            "checksum mismatch for {}; the .part file was kept for inspection",
            file.name
        )));
    }
    fs::rename(&part, target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MODELS;
    use std::io::Write;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "fluent-manager-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn missing_files_are_incomplete_not_installed() {
        let dir = temp_dir("incomplete");
        let spec = model_spec(Mode::Fast);
        let first = spec.files[0];
        let file = File::create(dir.join(first.name)).unwrap();
        file.set_len(first.size).unwrap();
        let installation = inspect_model(Mode::Fast, &dir);
        assert_eq!(installation.state, InstallState::Incomplete);
        assert_eq!(installation.required_bytes, spec.download_bytes());
        assert!(!installation.missing_files().is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn complete_sizes_are_quick_detected_as_installed() {
        let dir = temp_dir("installed");
        for file in model_spec(Mode::Fast).files {
            let handle = File::create(dir.join(file.name)).unwrap();
            handle.set_len(file.size).unwrap();
        }
        let installation = inspect_model(Mode::Fast, &dir);
        assert_eq!(installation.state, InstallState::Installed);
        assert!(!installation.verified);
        assert_eq!(installation.installed_bytes, installation.required_bytes);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn remove_is_confined_to_the_managed_model_directory() {
        let root = temp_dir("remove-root");
        let model = root.join(model_spec(Mode::Fast).default_dir);
        fs::create_dir_all(&model).unwrap();
        fs::write(model.join("marker"), b"x").unwrap();
        remove_model(Mode::Fast, &model, &root).unwrap();
        assert!(!model.exists());

        let outside = temp_dir("remove-outside");
        let error = remove_model(Mode::Fast, &outside, &root).unwrap_err();
        assert!(error.to_string().contains("outside"));
        assert!(outside.exists());
        fs::remove_dir_all(root).ok();
        fs::remove_dir_all(outside).ok();
    }

    #[test]
    fn directory_usage_and_hashes_are_reported() {
        let dir = temp_dir("usage");
        let mut file = File::create(dir.join("payload")).unwrap();
        file.write_all(b"hello").unwrap();
        assert_eq!(directory_size(&dir), 5);
        assert_eq!(
            hash_path(&dir.join("payload")).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn registry_manifests_are_unique_and_sized() {
        for spec in MODELS {
            assert!(!spec.files.is_empty());
            let mut names = std::collections::HashSet::new();
            for file in spec.files {
                assert!(names.insert(file.name), "duplicate file in {}", spec.name);
                assert!(file.size > 0);
                assert_eq!(file.sha256.len(), 64);
            }
            assert!(spec.download_bytes() > 0);
        }
    }
}

#[cfg(test)]
mod download_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    const TEST_FILE: ModelFile = ModelFile {
        name: "test.bin",
        size: 5,
        sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    };

    fn serve_once(partial: bool, body: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let count = socket.read(&mut request).unwrap_or(0);
            let text = String::from_utf8_lossy(&request[..count]).to_ascii_lowercase();
            let (status, extra) = if partial && text.contains("range: bytes=2-") {
                ("206 Partial Content", "Content-Range: bytes 2-4/5\r\n")
            } else {
                ("200 OK", "")
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(response.as_bytes()).unwrap();
            socket.write_all(body).unwrap();
        });
        format!("http://{address}/test.bin")
    }

    #[test]
    fn resumes_a_partial_download_and_verifies_the_hash() {
        let dir = std::env::temp_dir().join(format!(
            "fluent-download-resume-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("test.bin");
        fs::write(part_path(&target), b"he").unwrap();
        let url = serve_once(true, b"llo");
        let spec = model_spec(Mode::Fast);
        let cancel = AtomicBool::new(false);
        download_url(
            spec,
            &TEST_FILE,
            &target,
            2,
            0,
            5,
            0,
            1,
            &url,
            &mut |_| {},
            &cancel,
        )
        .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"hello");
        assert!(!part_path(&target).exists());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_bad_hash_keeps_the_part_file_and_does_not_create_the_target() {
        let dir = std::env::temp_dir().join(format!(
            "fluent-download-badhash-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("test.bin");
        let url = serve_once(false, b"world");
        let spec = model_spec(Mode::Fast);
        let cancel = AtomicBool::new(false);
        let error = download_url(
            spec,
            &TEST_FILE,
            &target,
            0,
            0,
            5,
            0,
            1,
            &url,
            &mut |_| {},
            &cancel,
        )
        .unwrap_err();
        assert!(error.to_string().contains("checksum mismatch"));
        assert!(part_path(&target).exists());
        assert!(!target.exists());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn cancellation_is_reported() {
        let dir = std::env::temp_dir().join(format!(
            "fluent-download-cancel-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("test.bin");
        let url = serve_once(false, b"hello");
        let spec = model_spec(Mode::Fast);
        let cancel = AtomicBool::new(true);
        let error = download_url(
            spec,
            &TEST_FILE,
            &target,
            0,
            0,
            5,
            0,
            1,
            &url,
            &mut |_| {},
            &cancel,
        )
        .unwrap_err();
        assert!(matches!(error, Error::Cancelled));
        fs::remove_dir_all(dir).ok();
    }
}

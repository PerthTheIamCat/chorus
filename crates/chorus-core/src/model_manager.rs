//! 1-Click Local In-Process Model Storage & Lifecycle Manager.
//!
//! Manages local GGUF model storage in `~/Library/Application Support/Chorus/models/`,
//! pre-download disk space checks via Darwin `statvfs`, atomic temporary file
//! finalization, integrity checks, and deletion lifecycle.

use crate::{ChorusError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Specification for a curated downloadable local model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSpec {
    pub id: String,
    pub display_name: String,
    pub filename: String,
    pub download_url: String,
    pub expected_size_bytes: u64,
    pub blake3_hash: Option<String>,
    #[serde(default)]
    pub sha256_hash: Option<String>,
    pub min_disk_free_bytes: u64,
    pub purpose: ModelPurpose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelPurpose {
    Router,
    Compaction,
    DualUse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ModelStatus {
    NotInstalled {
        spec: ModelSpec,
        free_disk_bytes: u64,
        can_download: bool,
    },
    Ready {
        spec: ModelSpec,
        path: PathBuf,
        size_bytes: u64,
    },
    Corrupted {
        spec: ModelSpec,
        path: PathBuf,
        reason: String,
    },
}

pub struct ModelManager {
    models_dir: PathBuf,
}

impl Default for ModelManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelManager {
    /// Create a model manager with the default macOS Application Support path.
    pub fn new() -> Self {
        let dir = Self::default_models_dir();
        Self { models_dir: dir }
    }

    /// Create a model manager pointing to a custom directory (useful for testing).
    pub fn with_directory(dir: impl Into<PathBuf>) -> Self {
        Self {
            models_dir: dir.into(),
        }
    }

    /// Resolve the default storage path: `~/Library/Application Support/Chorus/models/`
    pub fn default_models_dir() -> PathBuf {
        if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("Chorus")
                .join("models")
        } else {
            PathBuf::from(".chorus").join("models")
        }
    }

    /// Returns the models directory, ensuring it exists.
    pub fn ensure_models_dir(&self) -> Result<&Path> {
        if !self.models_dir.exists() {
            fs::create_dir_all(&self.models_dir).map_err(|e| {
                ChorusError::Invalid(format!(
                    "failed to create models directory {:?}: {e}",
                    self.models_dir
                ))
            })?;
        }
        Ok(&self.models_dir)
    }

    /// Returns the curated list of officially supported 1-click models.
    pub fn curated_models() -> Vec<ModelSpec> {
        vec![
            ModelSpec {
                id: "openthai-systemone:0.8b-q4_k_m".to_string(),
                display_name: "OpenThai System One 0.8B (Q4_K_M)".to_string(),
                filename: "OpenThai-SystemOne-v0.3-Q4_K_M.gguf".to_string(),
                download_url: "https://huggingface.co/iapp/OpenThai-SystemOne-GGUF/resolve/020b22d01232e29783da229dbd75ee1f9fe8dba8/OpenThai-SystemOne-v0.3-Q4_K_M.gguf".to_string(),
                expected_size_bytes: 529_313_920, // ~529 MB
                blake3_hash: None,
                sha256_hash: Some("9be793559b815eb73cd2478bff9944b11e9e855e9ac9ff04efbdf6a344512da2".into()),
                min_disk_free_bytes: 1_500_000_000, // 1.5 GB free required
                purpose: ModelPurpose::Router,
            },
            ModelSpec {
                id: "qwen2.5-coder:1.5b-instruct-q3_k_m".to_string(),
                display_name: "Qwen 2.5 Coder 1.5B (Q3_K_M)".to_string(),
                filename: "qwen2.5-coder-1.5b-instruct-q3_k_m.gguf".to_string(),
                download_url: "https://huggingface.co/Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF/resolve/f86cb2c1fa58255f8052cc32aeede1b7482d4361/qwen2.5-coder-1.5b-instruct-q3_k_m.gguf".to_string(),
                expected_size_bytes: 924_456_000,
                blake3_hash: None,
                sha256_hash: Some("d281a3a0010df03c8a0e3ffebd7f9444a95244fb518f132c5475e4b48d9adb5e".into()),
                min_disk_free_bytes: 2_500_000_000, // 2.5 GB free required
                purpose: ModelPurpose::Compaction,
            },
        ]
    }

    /// Look up a curated model spec by model ID.
    pub fn find_spec(model_id: &str) -> Option<ModelSpec> {
        Self::curated_models()
            .into_iter()
            .find(|m| m.id == model_id)
    }

    /// Check the status of a specific model.
    pub fn status(&self, spec: &ModelSpec) -> ModelStatus {
        let target_path = match self.model_path(spec) {
            Ok(path) => path,
            Err(e) => {
                return ModelStatus::Corrupted {
                    spec: spec.clone(),
                    path: self.models_dir.clone(),
                    reason: e.to_string(),
                }
            }
        };
        if matches!(fs::symlink_metadata(&target_path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
        {
            let free_disk = self.free_disk_space().unwrap_or(0);
            let can_download = free_disk >= spec.min_disk_free_bytes;
            return ModelStatus::NotInstalled {
                spec: spec.clone(),
                free_disk_bytes: free_disk,
                can_download,
            };
        }

        // Status is polled by the UI: validate only metadata/header here, not a full hash scan.
        match validate_model_file(&target_path, spec, false) {
            Ok(actual_size) => ModelStatus::Ready {
                spec: spec.clone(),
                path: target_path,
                size_bytes: actual_size,
            },
            Err(e) => ModelStatus::Corrupted {
                spec: spec.clone(),
                path: target_path,
                reason: format!("failed to read metadata: {e}"),
            },
        }
    }

    /// Query available free disk space on the volume housing the models directory.
    pub fn free_disk_space(&self) -> Result<u64> {
        let probe_dir = if self.models_dir.exists() {
            &self.models_dir
        } else if let Some(parent) = self.models_dir.parent() {
            if parent.exists() {
                parent
            } else {
                Path::new("/")
            }
        } else {
            Path::new("/")
        };

        free_disk_space_bytes(probe_dir)
            .map_err(|e| ChorusError::Invalid(format!("failed to query disk space: {e}")))
    }

    /// Atomically verify and install a downloaded temporary file into the models directory.
    pub fn finalize_download(&self, temp_file_path: &Path, spec: &ModelSpec) -> Result<PathBuf> {
        let target_path = self.model_path(spec)?;
        self.ensure_models_dir()?;
        validate_model_file(temp_file_path, spec, true)?;

        // Atomic move
        fs::rename(temp_file_path, &target_path).map_err(|e| {
            ChorusError::Invalid(format!(
                "failed to move model file from {:?} to {:?}: {e}",
                temp_file_path, target_path
            ))
        })?;

        Ok(target_path)
    }

    /// Delete a model file from disk to reclaim storage.
    pub fn delete_model(&self, spec: &ModelSpec) -> Result<bool> {
        let target_path = self.model_path(spec)?;
        match fs::symlink_metadata(&target_path) {
            Ok(_) => {
                fs::remove_file(&target_path).map_err(|e| {
                    ChorusError::Invalid(format!(
                        "failed to delete model file {:?}: {e}",
                        target_path
                    ))
                })?;
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(ChorusError::Invalid(format!(
                "failed to inspect model file: {e}"
            ))),
        }
    }

    fn model_path(&self, spec: &ModelSpec) -> Result<PathBuf> {
        let mut components = Path::new(&spec.filename).components();
        if !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
            || spec.filename.contains(['/', '\\'])
            || spec.expected_size_bytes < 4
        {
            return Err(ChorusError::Invalid(
                "invalid model filename or expected size".into(),
            ));
        }
        Ok(self.models_dir.join(&spec.filename))
    }
}

fn validate_model_file(path: &Path, spec: &ModelSpec, verify_hash: bool) -> Result<u64> {
    let invalid = |e| ChorusError::Invalid(format!("model validation failed: {e}"));
    let meta = fs::symlink_metadata(path).map_err(|e| invalid(e.to_string()))?;
    if !meta.is_file() {
        return Err(invalid(
            "expected a regular file, not a symlink or directory".into(),
        ));
    }
    if meta.len() != spec.expected_size_bytes {
        return Err(invalid(format!(
            "expected {} bytes, received {}",
            spec.expected_size_bytes,
            meta.len()
        )));
    }
    let mut file = fs::File::open(path).map_err(|e| invalid(e.to_string()))?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic)
        .map_err(|e| invalid(e.to_string()))?;
    if &magic != b"GGUF" {
        return Err(invalid("invalid GGUF header".into()));
    }
    if verify_hash && (spec.blake3_hash.is_some() || spec.sha256_hash.is_some()) {
        let expected_blake3 = spec
            .blake3_hash
            .as_ref()
            .map(|h| blake3::Hash::from_hex(h).map_err(|e| invalid(e.to_string())))
            .transpose()?;
        if let Some(expected) = &spec.sha256_hash {
            if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(invalid("invalid SHA-256 digest".into()));
            }
        }
        let mut blake3 = expected_blake3.map(|_| blake3::Hasher::new());
        let mut sha256 = spec.sha256_hash.as_ref().map(|_| Sha256::new());
        if let Some(h) = &mut blake3 {
            h.update(&magic);
        }
        if let Some(h) = &mut sha256 {
            h.update(magic);
        }
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buffer).map_err(|e| invalid(e.to_string()))?;
            if n == 0 {
                break;
            }
            if let Some(h) = &mut blake3 {
                h.update(&buffer[..n]);
            }
            if let Some(h) = &mut sha256 {
                h.update(&buffer[..n]);
            }
        }
        if let (Some(h), Some(expected)) = (blake3, expected_blake3) {
            if h.finalize() != expected {
                return Err(invalid("BLAKE3 checksum mismatch".into()));
            }
        }
        if let (Some(h), Some(expected)) = (sha256, &spec.sha256_hash) {
            if !format!("{:x}", h.finalize()).eq_ignore_ascii_case(expected) {
                return Err(invalid("SHA-256 checksum mismatch".into()));
            }
        }
    }
    Ok(meta.len())
}

/// Query free disk space in bytes on macOS using POSIX `statvfs`.
#[cfg(target_os = "macos")]
fn free_disk_space_bytes(path: &Path) -> std::io::Result<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

    unsafe {
        #[repr(C)]
        struct libc_statvfs {
            f_bsize: std::os::raw::c_ulong,
            f_frsize: std::os::raw::c_ulong,
            f_blocks: u32,
            f_bfree: u32,
            f_bavail: u32,
            f_files: u32,
            f_ffree: u32,
            f_favail: u32,
            f_fsid: std::os::raw::c_ulong,
            f_flag: std::os::raw::c_ulong,
            f_namemax: std::os::raw::c_ulong,
        }

        extern "C" {
            fn statvfs(
                path: *const std::os::raw::c_char,
                buf: *mut libc_statvfs,
            ) -> std::os::raw::c_int;
        }

        let mut stat: libc_statvfs = std::mem::zeroed();
        if statvfs(c_path.as_ptr(), &mut stat) == 0 {
            let frsize = if stat.f_frsize > 0 {
                stat.f_frsize
            } else {
                stat.f_bsize
            };
            Ok(stat.f_bavail as u64 * frsize as u64)
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn free_disk_space_bytes(_path: &Path) -> std::io::Result<u64> {
    Ok(100 * 1024 * 1024 * 1024) // Default fallback 100 GB
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_curated_models_registry() {
        let models = ModelManager::curated_models();
        assert_eq!(models.len(), 2);
        assert!(models.iter().any(|m| m.id.contains("openthai")));
    }

    #[test]
    fn test_model_lifecycle_not_installed_to_ready_to_deleted() {
        let dir = tempdir().unwrap();
        let mgr = ModelManager::with_directory(dir.path());
        let mut spec = ModelManager::find_spec("openthai-systemone:0.8b-q4_k_m").unwrap();
        let data = b"GGUFfixture model";
        spec.sha256_hash = Some(format!("{:x}", Sha256::digest(data)));
        spec.expected_size_bytes = data.len() as u64;
        spec.blake3_hash = Some(blake3::hash(data).to_hex().to_string());

        // 1. Initial status: NotInstalled
        let st = mgr.status(&spec);
        match st {
            ModelStatus::NotInstalled { can_download, .. } => {
                assert!(can_download);
            }
            _ => panic!("expected NotInstalled"),
        }

        // 2. Simulate temp download file
        let temp_file = dir.path().join("download.tmp");
        fs::write(&temp_file, data).unwrap();

        // 3. Finalize download
        let installed_path = mgr.finalize_download(&temp_file, &spec).unwrap();
        assert!(installed_path.exists());
        assert!(!temp_file.exists());

        // 4. Verify Ready status
        let st2 = mgr.status(&spec);
        match st2 {
            ModelStatus::Ready { size_bytes, .. } => {
                assert_eq!(size_bytes, data.len() as u64);
            }
            _ => panic!("expected Ready"),
        }

        // 5. Delete model
        let deleted = mgr.delete_model(&spec).unwrap();
        assert!(deleted);
        assert!(!installed_path.exists());

        // 6. Return to NotInstalled
        let st3 = mgr.status(&spec);
        assert!(matches!(st3, ModelStatus::NotInstalled { .. }));
    }

    #[test]
    fn test_free_disk_space_positive() {
        let mgr = ModelManager::new();
        let free = mgr.free_disk_space().unwrap();
        assert!(free > 0, "free disk space must be greater than zero");
    }

    fn fixture_spec() -> ModelSpec {
        let mut spec = ModelManager::curated_models().remove(0);
        spec.expected_size_bytes = 16;
        spec.blake3_hash = None;
        spec.sha256_hash = None;
        spec
    }

    #[test]
    fn sha256_rejects_same_size_tampering_and_preserves_installed_model() {
        let dir = tempdir().unwrap();
        let mgr = ModelManager::with_directory(dir.path());
        let mut spec = fixture_spec();
        // Independent hashlib/NIST-compatible SHA-256 reference for this synthetic fixture.
        spec.sha256_hash =
            Some("909a56789df9568ce0b0785e4752d7092317131e8849ce5ed0ca80da687822f0".into());
        let download = dir.path().join("download.tmp");
        fs::write(&download, b"GGUFtrusted file").unwrap();
        let installed = mgr.finalize_download(&download, &spec).unwrap();
        fs::write(&download, b"GGUFtamperedfile").unwrap();
        let error = mgr.finalize_download(&download, &spec).unwrap_err();
        assert!(error.to_string().contains("SHA-256 checksum mismatch"));
        assert_eq!(fs::read(&installed).unwrap(), b"GGUFtrusted file");
        assert!(download.exists());
        spec.sha256_hash = Some("invalid".into());
        assert!(mgr
            .finalize_download(&download, &spec)
            .unwrap_err()
            .to_string()
            .contains("invalid SHA-256 digest"));
    }

    #[test]
    fn sha256_checks_bytes_beyond_the_first_buffer() {
        let dir = tempdir().unwrap();
        let mgr = ModelManager::with_directory(dir.path());
        let mut spec = fixture_spec();
        let mut data = vec![0u8; 131_072];
        data[..4].copy_from_slice(b"GGUF");
        spec.expected_size_bytes = data.len() as u64;
        spec.sha256_hash =
            Some("6d4f5a9997e8e16a12c19a863509bc426358de266ee4d5c6a24b142f780e6cad".into());
        let download = dir.path().join("download.tmp");
        fs::write(&download, &data).unwrap();
        let installed = mgr.finalize_download(&download, &spec).unwrap();
        data[131_000] = 1;
        fs::write(&download, &data).unwrap();
        assert!(mgr.finalize_download(&download, &spec).is_err());
        assert_eq!(fs::read(installed).unwrap()[131_000], 0);
    }

    #[test]
    fn invalid_downloads_do_not_replace_an_installed_model() {
        let dir = tempdir().unwrap();
        let mgr = ModelManager::with_directory(dir.path());
        let mut spec = fixture_spec();
        let installed = dir.path().join(&spec.filename);
        fs::write(&installed, b"GGUFtrusted file").unwrap();
        for data in [
            b"GGUFshort".as_slice(),
            b"GGUFoversized payload".as_slice(),
            b"<html>not a gguf!".as_slice(),
        ] {
            let download = dir.path().join("download.tmp");
            fs::write(&download, data).unwrap();
            assert!(mgr.finalize_download(&download, &spec).is_err());
            assert_eq!(fs::read(&installed).unwrap(), b"GGUFtrusted file");
            assert!(download.exists());
        }
        let download = dir.path().join("download.tmp");
        fs::write(&download, b"GGUFtamperedfile").unwrap();
        spec.blake3_hash = Some(blake3::hash(b"GGUFtrusted file").to_hex().to_string());
        assert!(mgr.finalize_download(&download, &spec).is_err());
        assert_eq!(fs::read(&installed).unwrap(), b"GGUFtrusted file");
        spec.blake3_hash = Some("invalid hash".into());
        assert!(mgr.finalize_download(&download, &spec).is_err());
    }

    #[test]
    fn model_filenames_cannot_escape_the_storage_directory() {
        let dir = tempdir().unwrap();
        let models = dir.path().join("models");
        let mgr = ModelManager::with_directory(&models);
        let outside = dir.path().join("outside.gguf");
        fs::write(&outside, b"GGUFtrusted file").unwrap();
        let download = dir.path().join("download.tmp");
        fs::write(&download, b"GGUFtrusted file").unwrap();
        for name in [
            "../outside.gguf".to_owned(),
            outside.to_string_lossy().into_owned(),
            "nested/model.gguf".into(),
            "..\\outside.gguf".into(),
            "".into(),
            ".".into(),
        ] {
            let mut spec = fixture_spec();
            spec.filename = name;
            assert!(mgr.finalize_download(&download, &spec).is_err());
            assert!(mgr.delete_model(&spec).is_err());
            assert!(matches!(mgr.status(&spec), ModelStatus::Corrupted { .. }));
            assert_eq!(fs::read(&outside).unwrap(), b"GGUFtrusted file");
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_downloads_are_rejected_and_model_link_deletion_preserves_target() {
        let dir = tempdir().unwrap();
        let mgr = ModelManager::with_directory(dir.path().join("models"));
        mgr.ensure_models_dir().unwrap();
        let outside = dir.path().join("outside.gguf");
        fs::write(&outside, b"GGUFtrusted file").unwrap();
        let download = dir.path().join("download.tmp");
        std::os::unix::fs::symlink(&outside, &download).unwrap();
        let spec = fixture_spec();
        assert!(mgr.finalize_download(&download, &spec).is_err());
        let installed = mgr.models_dir.join(&spec.filename);
        std::os::unix::fs::symlink(&outside, &installed).unwrap();
        assert!(matches!(mgr.status(&spec), ModelStatus::Corrupted { .. }));
        assert!(mgr.delete_model(&spec).unwrap());
        assert_eq!(fs::read(&outside).unwrap(), b"GGUFtrusted file");
        std::os::unix::fs::symlink(dir.path().join("missing"), &installed).unwrap();
        assert!(mgr.delete_model(&spec).unwrap());
        assert!(fs::symlink_metadata(&installed).is_err());
    }
}

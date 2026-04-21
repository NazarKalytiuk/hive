//! Immutable per-run artifact directories under
//! `<workspace-root>/.tarn/runs/<run_id>/`.
//!
//! Every Tarn run is assigned a stable `run_id` derived from its start
//! timestamp plus a short random suffix, and its artifacts (the full
//! JSON report, the condensed `state.json`, and any follow-up files
//! from sibling tickets) are written into that directory. The old
//! `.tarn/last-run.json` and `.tarn/state.json` paths stay as copies
//! of the most recent run, so tooling that already reads those paths
//! keeps working.
//!
//! The goal is to make debugging context durable: a second run does
//! not destroy the artifacts from the previous run, so users can
//! compare runs, open a prior run after the fact, or share a run id
//! with an agent.

use chrono::{DateTime, Utc};
use rand::RngCore;
use std::io;
use std::path::{Path, PathBuf};

/// Build a stable run identifier from the run's start timestamp and a
/// short random suffix. Format: `YYYYmmdd-HHMMSS-xxxxxx` where
/// `xxxxxx` is 6 lowercase hex characters.
///
/// The timestamp prefix keeps directory listings chronologically
/// sortable; the random suffix prevents collisions when two runs
/// start inside the same second (e.g. watch mode firing repeatedly).
pub fn generate_run_id(started_at: DateTime<Utc>) -> String {
    let mut rng = rand::rng();
    let mut bytes = [0u8; 3];
    rng.fill_bytes(&mut bytes);
    format!(
        "{}-{:02x}{:02x}{:02x}",
        started_at.format("%Y%m%d-%H%M%S"),
        bytes[0],
        bytes[1],
        bytes[2]
    )
}

/// Return `<workspace_root>/.tarn/runs/<run_id>/` without creating it.
pub fn run_directory(workspace_root: &Path, run_id: &str) -> PathBuf {
    workspace_root.join(".tarn").join("runs").join(run_id)
}

/// Create `<workspace_root>/.tarn/runs/<run_id>/` (and parents) and
/// return its absolute path.
pub fn ensure_run_directory(workspace_root: &Path, run_id: &str) -> io::Result<PathBuf> {
    let dir = run_directory(workspace_root, run_id);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Copy a freshly-written run artifact (e.g. `report.json`) to a
/// pointer path (e.g. `.tarn/last-run.json`) so legacy consumers that
/// read the pointer path keep working. The pointer is overwritten
/// atomically via write-then-rename so a reader never sees a
/// half-populated file.
pub fn copy_to_pointer(src: &Path, pointer: &Path) -> io::Result<()> {
    if !src.exists() {
        return Ok(());
    }
    if let Some(parent) = pointer.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = std::fs::read(src)?;
    let tmp = pointer.with_extension("tmp");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, pointer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn generate_run_id_has_timestamp_and_suffix() {
        let started = Utc::now();
        let id = generate_run_id(started);
        // Format: YYYYmmdd-HHMMSS-xxxxxx -> 8+1+6+1+6 = 22 chars
        assert_eq!(id.len(), 22, "unexpected run_id format: {}", id);
        assert!(id.starts_with(&started.format("%Y%m%d-%H%M%S").to_string()));
    }

    #[test]
    fn generate_run_id_is_unique_under_same_second() {
        let started = Utc::now();
        let a = generate_run_id(started);
        let b = generate_run_id(started);
        assert_ne!(a, b);
    }

    #[test]
    fn run_directory_path_is_under_workspace_tarn_runs() {
        let path = run_directory(Path::new("/tmp/ws"), "20260101-000000-aabbcc");
        assert_eq!(
            path,
            PathBuf::from("/tmp/ws/.tarn/runs/20260101-000000-aabbcc")
        );
    }

    #[test]
    fn ensure_run_directory_creates_nested_path() {
        let tmp = TempDir::new().unwrap();
        let dir = ensure_run_directory(tmp.path(), "20260101-000000-abcdef").unwrap();
        assert!(dir.is_dir());
        assert!(dir.ends_with(".tarn/runs/20260101-000000-abcdef"));
    }

    #[test]
    fn copy_to_pointer_overwrites_existing_file_atomically() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("report.json");
        let pointer = tmp.path().join(".tarn").join("last-run.json");
        std::fs::write(&src, b"v2").unwrap();
        std::fs::create_dir_all(pointer.parent().unwrap()).unwrap();
        std::fs::write(&pointer, b"v1").unwrap();
        copy_to_pointer(&src, &pointer).unwrap();
        assert_eq!(std::fs::read(&pointer).unwrap(), b"v2");
        assert!(
            !pointer.with_extension("tmp").exists(),
            "tmp must be renamed away"
        );
    }

    #[test]
    fn copy_to_pointer_noop_when_source_missing() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("missing.json");
        let pointer = tmp.path().join(".tarn").join("last-run.json");
        copy_to_pointer(&src, &pointer).unwrap();
        assert!(!pointer.exists());
    }
}

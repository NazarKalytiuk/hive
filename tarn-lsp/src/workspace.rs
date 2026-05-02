//! Bounded, best-effort cache of every `.tarn.yaml` file in the workspace.
//!
//! L2.2 (`textDocument/references`) is the first feature in `tarn-lsp` that
//! needs to look at files other than the one currently under the cursor: an
//! `{{ env.x }}` reference query has to walk every test file in the workspace
//! and report every interpolation that mentions `env.x`. The data structure
//! that powers that walk lives here.
//!
//! ## Design
//!
//! `WorkspaceIndex` is a *cache*, not a watcher. The lifecycle is:
//!
//!   1. The first reference query (or any future cross-file feature) calls
//!      [`WorkspaceIndex::ensure_scanned`], which performs a one-shot
//!      recursive walk of the workspace root and caches the parsed
//!      [`tarn::outline::Outline`] plus the raw source text for every
//!      `.tarn.yaml` it finds.
//!   2. The walk is bounded at [`WORKSPACE_FILE_LIMIT`] files. If we hit
//!      that limit we log a warning to stderr (`eprintln!`) and return the
//!      partial result. The references handler still works — it just may
//!      miss tail entries in pathologically large repositories.
//!   3. The server's `didChange` / `didSave` / `didClose` notification
//!      handlers call [`WorkspaceIndex::invalidate`] for the affected URL
//!      so the next reference query will re-read the freshest content from
//!      that file (either from the document store, if open, or from disk).
//!
//! There is intentionally **no** filesystem watcher, no background thread,
//! and no eviction policy beyond the per-URL invalidation hook. The
//! references feature is the first cross-file feature in `tarn-lsp`, and a
//! future ticket can replace this with a smarter cache once we have real
//! usage data. For NAZ-298 the cache is the simplest thing that works.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use lsp_types::Url;
use tarn::outline::{outline_document, Outline};

/// Maximum number of `.tarn.yaml` files [`WorkspaceIndex::ensure_scanned`]
/// will walk in a single pass.
///
/// This is a safety net so a stray `node_modules` symlink in a giant
/// monorepo cannot pin a single LSP request. Hitting the cap is rare in
/// practice — even tarn workspaces with hundreds of test files stay well
/// inside it. When the cap is hit we log a warning and continue with the
/// partial result rather than erroring out, because returning *some*
/// references is strictly better than returning none.
pub const WORKSPACE_FILE_LIMIT: usize = 5000;

/// One cached file. Pairs the raw source text with the best-effort
/// outline so callers that need either one don't have to re-parse.
///
/// `mtime` is captured for two reasons: (a) it lets future tickets add a
/// staleness check without changing the public surface, and (b) it makes
/// the cache shape consistent with file-system-watcher patterns from other
/// LSP implementations, which means future swaps stay drop-in.
#[derive(Debug, Clone)]
pub struct CachedFile {
    /// Raw UTF-8 source text of the file.
    pub source: String,
    /// Outline produced by [`tarn::outline::outline_document`]. May be
    /// `None` when the file does not parse — that branch is still cached
    /// so we don't repeatedly retry a broken file inside a single query.
    pub outline: Option<Outline>,
    /// Filesystem mtime at the time we read the file. Best-effort —
    /// platforms that fail to report mtime fall back to `UNIX_EPOCH`.
    pub mtime: SystemTime,
}

/// Errors the workspace walker can surface to its caller.
///
/// Today there is exactly one variant — the walker treats per-file
/// failures as warnings logged to stderr rather than errors, so the only
/// thing that can fail the entire scan is a missing or unreadable root.
#[derive(Debug)]
pub enum WorkspaceError {
    /// The configured workspace root could not be opened (missing,
    /// permission denied, not a directory, …).
    RootUnreadable(PathBuf, std::io::Error),
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkspaceError::RootUnreadable(path, err) => {
                write!(f, "workspace root unreadable {}: {}", path.display(), err)
            }
        }
    }
}

impl std::error::Error for WorkspaceError {}

/// Bounded cache of every `.tarn.yaml` file in the workspace.
///
/// The struct holds a single `HashMap<Url, CachedFile>` and a one-shot
/// "have we walked yet?" flag — that flag is what makes
/// [`WorkspaceIndex::ensure_scanned`] cheap on every request after the
/// first. Tests construct an empty index, call `ensure_scanned` against a
/// `tempfile::TempDir`, and then assert against `iter()` / `get()`.
#[derive(Debug, Default)]
pub struct WorkspaceIndex {
    /// Configured workspace root, if the LSP client provided one. When
    /// `None`, [`WorkspaceIndex::ensure_scanned`] is a no-op and the
    /// references feature degrades gracefully to single-file behaviour.
    root: Option<Url>,
    /// Per-URL cache populated by the walker.
    cache: HashMap<Url, CachedFile>,
    /// Set to `true` after the first successful walk so subsequent
    /// invocations of [`WorkspaceIndex::ensure_scanned`] short-circuit.
    /// Cleared by [`WorkspaceIndex::reset`] (test-only today).
    scanned: bool,
}

impl WorkspaceIndex {
    /// Build an empty index. The root is set later via
    /// [`WorkspaceIndex::set_root`] once the server has the
    /// `InitializeParams::root_uri`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct an index already pinned to a workspace root. Used by
    /// tests that need to skip the bootstrap-style two-step setup.
    pub fn with_root(root: Url) -> Self {
        Self {
            root: Some(root),
            ..Self::default()
        }
    }

    /// Replace the workspace root. Resets the cache so the next
    /// `ensure_scanned` call walks the new tree.
    pub fn set_root(&mut self, root: Option<Url>) {
        self.root = root;
        self.cache.clear();
        self.scanned = false;
    }

    /// Workspace root URL the index was configured with, if any.
    pub fn root(&self) -> Option<&Url> {
        self.root.as_ref()
    }

    /// Drop the cached entry for `uri`.
    ///
    /// Called from the server's `didChange` / `didSave` / `didClose`
    /// notification handlers so the next reference query re-reads the
    /// fresh content (from the document store if the buffer is still
    /// open, otherwise from disk via [`WorkspaceIndex::ensure_scanned`]).
    pub fn invalidate(&mut self, uri: &Url) {
        self.cache.remove(uri);
        // We deliberately leave `scanned = true` so we don't redo the
        // entire workspace walk just because one file changed. The next
        // query that needs the invalidated file will re-read it through
        // [`WorkspaceIndex::refresh_one`] (driven by the references
        // handler).
    }

    /// Force the cache to forget what it has walked. Used by tests.
    #[cfg(test)]
    pub fn reset(&mut self) {
        self.cache.clear();
        self.scanned = false;
    }

    /// Look up a cached file by URL.
    pub fn get(&self, uri: &Url) -> Option<&CachedFile> {
        self.cache.get(uri)
    }

    /// Iterate every cached `(Url, CachedFile)` pair in arbitrary order.
    pub fn iter(&self) -> impl Iterator<Item = (&Url, &CachedFile)> {
        self.cache.iter()
    }

    /// Number of files currently in the cache.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// `true` when the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    /// Insert (or replace) a cached file from already-loaded source text.
    ///
    /// Used by the references handler to splice in the freshest copy of
    /// an open buffer before walking the cache. The mtime field falls
    /// back to "now" because we are not reading from disk.
    pub fn insert_from_source(&mut self, uri: Url, source: String) {
        let outline = outline_document(&uri_to_path(&uri), &source);
        self.cache.insert(
            uri,
            CachedFile {
                source,
                outline,
                mtime: SystemTime::now(),
            },
        );
    }

    /// Walk the workspace root once and populate the cache.
    ///
    /// Cheap to call repeatedly: subsequent invocations short-circuit on
    /// the `scanned` flag. Returns `Ok(())` when the root was unset (the
    /// references feature degrades gracefully) or after a successful
    /// walk; only a missing/unreadable root surfaces as
    /// [`WorkspaceError::RootUnreadable`].
    pub fn ensure_scanned(&mut self) -> Result<(), WorkspaceError> {
        if self.scanned {
            return Ok(());
        }
        let Some(root) = self.root.clone() else {
            // No workspace root configured (the LSP client is operating
            // in single-file mode). Mark as scanned so we never retry,
            // and let the references handler fall back to per-file
            // behaviour.
            self.scanned = true;
            return Ok(());
        };
        let Ok(root_path) = root.to_file_path() else {
            // Non-`file://` root. Mark scanned and degrade gracefully.
            self.scanned = true;
            return Ok(());
        };
        if !root_path.exists() {
            return Err(WorkspaceError::RootUnreadable(
                root_path.clone(),
                std::io::Error::new(std::io::ErrorKind::NotFound, "workspace root not found"),
            ));
        }
        let mut visited = 0usize;
        let mut hit_cap = false;
        walk_dir(&root_path, &mut |path| {
            if visited >= WORKSPACE_FILE_LIMIT {
                hit_cap = true;
                return false;
            }
            if !is_tarn_yaml(path) {
                return true;
            }
            visited += 1;
            // Per-file failures are warnings, never errors. The walker
            // continues so one unreadable file does not break references
            // for the rest of the workspace.
            match read_and_cache(path) {
                Ok((url, cached)) => {
                    self.cache.insert(url, cached);
                }
                Err(err) => {
                    eprintln!(
                        "tarn-lsp: failed to read {} for workspace index: {}",
                        path.display(),
                        err
                    );
                }
            }
            true
        });
        if hit_cap {
            eprintln!(
                "tarn-lsp: workspace walk stopped at {} files; references may be incomplete",
                WORKSPACE_FILE_LIMIT
            );
        }
        self.scanned = true;
        Ok(())
    }
}

/// Recursive directory walker.
///
/// Calls `visit` once per *file* (not directory). The closure returns
/// `false` to abort the walk early — used by [`WorkspaceIndex::ensure_scanned`]
/// to stop scanning once it has hit the file cap.
fn walk_dir(root: &Path, visit: &mut dyn FnMut(&Path) -> bool) {
    // Use std::fs::read_dir directly so the implementation has zero
    // unconditional dependencies on `walkdir`. The recursion is bounded
    // by the directory hierarchy itself plus the explicit file cap; we
    // do not protect against pathological symlink loops because Tarn
    // workspaces in practice live inside ordinary git checkouts.
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            // Skip the usual heavy directories so a workspace walk does
            // not get derailed by a transitive `node_modules` or
            // `target` checkout. We could push these into a configurable
            // ignore list later, but the obvious offenders cover 99% of
            // real workspaces.
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    "node_modules" | "target" | ".git" | ".svn" | "dist" | "build"
                ) {
                    continue;
                }
            }
            walk_dir(&path, visit);
        } else if file_type.is_file() && !visit(&path) {
            // Visitor asked us to stop the walk.
            return;
        }
    }
}

/// `true` when `path` looks like a Tarn test file: ends in `.tarn.yaml`
/// or `.tarn.yml`. The double-extension check matches the file pattern
/// the rest of the LSP advertises (`*.tarn.yaml`).
fn is_tarn_yaml(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name.ends_with(".tarn.yaml") || name.ends_with(".tarn.yml")
}

/// Read a file from disk, parse its outline, and pair it with a
/// freshly-derived `Url`.
fn read_and_cache(path: &Path) -> std::io::Result<(Url, CachedFile)> {
    let source = std::fs::read_to_string(path)?;
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let outline = outline_document(path, &source);
    let url = Url::from_file_path(path).map_err(|()| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("path not convertible to file:// url: {}", path.display()),
        )
    })?;
    Ok((
        url,
        CachedFile {
            source,
            outline,
            mtime,
        },
    ))
}

fn uri_to_path(uri: &Url) -> PathBuf {
    uri.to_file_path()
        .unwrap_or_else(|_| PathBuf::from(uri.path()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&path, body).expect("write file");
        path
    }

    fn root_url(dir: &TempDir) -> Url {
        Url::from_directory_path(dir.path()).expect("dir url")
    }

    const FIXTURE_A: &str = "name: a\nsteps:\n  - name: ping\n    request:\n      method: GET\n      url: \"{{ env.base_url }}/a\"\n";
    const FIXTURE_B: &str = "name: b\nsteps:\n  - name: ping\n    request:\n      method: GET\n      url: \"{{ env.base_url }}/b\"\n";

    #[test]
    fn ensure_scanned_with_no_root_is_a_noop() {
        let mut idx = WorkspaceIndex::new();
        idx.ensure_scanned().expect("ensure_scanned");
        assert!(idx.is_empty());
    }

    #[test]
    fn ensure_scanned_caches_every_tarn_yaml_in_root() {
        let dir = TempDir::new().unwrap();
        write(dir.path(), "alpha.tarn.yaml", FIXTURE_A);
        write(dir.path(), "beta.tarn.yaml", FIXTURE_B);
        // A non-tarn file should be ignored.
        write(dir.path(), "ignore.txt", "not a tarn file");

        let mut idx = WorkspaceIndex::with_root(root_url(&dir));
        idx.ensure_scanned().expect("ensure_scanned");
        assert_eq!(idx.len(), 2);
    }

    #[test]
    fn ensure_scanned_recurses_into_subdirectories() {
        let dir = TempDir::new().unwrap();
        write(dir.path(), "top.tarn.yaml", FIXTURE_A);
        write(dir.path(), "nested/inner.tarn.yaml", FIXTURE_B);

        let mut idx = WorkspaceIndex::with_root(root_url(&dir));
        idx.ensure_scanned().expect("ensure_scanned");
        assert_eq!(idx.len(), 2);
    }

    #[test]
    fn ensure_scanned_skips_node_modules_and_target() {
        let dir = TempDir::new().unwrap();
        write(dir.path(), "real.tarn.yaml", FIXTURE_A);
        write(dir.path(), "node_modules/lib.tarn.yaml", FIXTURE_B);
        write(dir.path(), "target/release/x.tarn.yaml", FIXTURE_B);

        let mut idx = WorkspaceIndex::with_root(root_url(&dir));
        idx.ensure_scanned().expect("ensure_scanned");
        assert_eq!(idx.len(), 1);
    }

    #[test]
    fn ensure_scanned_short_circuits_on_repeat_calls() {
        let dir = TempDir::new().unwrap();
        write(dir.path(), "alpha.tarn.yaml", FIXTURE_A);

        let mut idx = WorkspaceIndex::with_root(root_url(&dir));
        idx.ensure_scanned().unwrap();
        assert_eq!(idx.len(), 1);

        // Add a second file *after* the first walk. Without `reset()`
        // the second `ensure_scanned` should NOT pick it up — that's
        // the whole point of the short-circuit.
        write(dir.path(), "beta.tarn.yaml", FIXTURE_B);
        idx.ensure_scanned().unwrap();
        assert_eq!(idx.len(), 1);

        idx.reset();
        idx.ensure_scanned().unwrap();
        assert_eq!(idx.len(), 2);
    }

    #[test]
    fn invalidate_drops_one_entry_without_clearing_others() {
        let dir = TempDir::new().unwrap();
        let alpha = write(dir.path(), "alpha.tarn.yaml", FIXTURE_A);
        write(dir.path(), "beta.tarn.yaml", FIXTURE_B);

        let mut idx = WorkspaceIndex::with_root(root_url(&dir));
        idx.ensure_scanned().unwrap();
        assert_eq!(idx.len(), 2);

        let alpha_url = Url::from_file_path(&alpha).unwrap();
        idx.invalidate(&alpha_url);
        assert_eq!(idx.len(), 1);
        assert!(idx.get(&alpha_url).is_none());
    }

    #[test]
    fn insert_from_source_overwrites_cached_entry() {
        let dir = TempDir::new().unwrap();
        let alpha = write(dir.path(), "alpha.tarn.yaml", FIXTURE_A);

        let mut idx = WorkspaceIndex::with_root(root_url(&dir));
        idx.ensure_scanned().unwrap();
        let alpha_url = Url::from_file_path(&alpha).unwrap();

        let new_source = "name: a-overridden\nsteps: []\n".to_owned();
        idx.insert_from_source(alpha_url.clone(), new_source.clone());

        let cached = idx.get(&alpha_url).expect("cached entry");
        assert_eq!(cached.source, new_source);
    }

    #[test]
    fn missing_root_returns_error() {
        // Build the URL from a platform-valid absolute path that is
        // guaranteed not to exist. A hard-coded `/this/path/...` works
        // on Unix but `Url::from_directory_path` rejects it on Windows
        // (no drive letter), which makes `unwrap()` panic before the
        // function under test even runs.
        let nonexistent = std::env::temp_dir()
            .join("tarn-lsp-tests-missing-root-d8a3c1e7")
            .join("does-not-exist");
        let bogus = Url::from_directory_path(&nonexistent).expect("path is absolute");
        let mut idx = WorkspaceIndex::with_root(bogus);
        let result = idx.ensure_scanned();
        assert!(matches!(result, Err(WorkspaceError::RootUnreadable(_, _))));
    }

    #[test]
    fn set_root_clears_existing_cache() {
        let dir1 = TempDir::new().unwrap();
        write(dir1.path(), "x.tarn.yaml", FIXTURE_A);
        let dir2 = TempDir::new().unwrap();

        let mut idx = WorkspaceIndex::with_root(root_url(&dir1));
        idx.ensure_scanned().unwrap();
        assert_eq!(idx.len(), 1);

        idx.set_root(Some(root_url(&dir2)));
        assert_eq!(idx.len(), 0);
        idx.ensure_scanned().unwrap();
        assert_eq!(idx.len(), 0);
    }
}

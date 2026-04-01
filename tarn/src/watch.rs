use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Run in watch mode: execute, then re-execute on file changes.
/// The `run_fn` closure is called for each run and returns the exit code.
pub fn run_watch_loop(watch_paths: &[String], run_fn: impl Fn(&[String]) -> i32) -> ! {
    let mut dependency_map = build_dependency_map(watch_paths);

    // Initial run
    clear_screen();
    run_fn(watch_paths);

    let (tx, rx) = mpsc::channel();
    let mut watcher =
        RecommendedWatcher::new(tx, Config::default()).expect("Failed to create file watcher");

    watch_directories(&mut watcher, watch_paths, &dependency_map);
    // Also watch cwd for env/config files
    let _ = watcher.watch(Path::new("."), RecursiveMode::NonRecursive);

    eprintln!("\n  Watching for changes... (Ctrl+C to stop)\n");

    let debounce = Duration::from_millis(300);
    let mut last_run = Instant::now();

    loop {
        match rx.recv() {
            Ok(Ok(event)) => {
                let changed_paths: Vec<PathBuf> = event
                    .paths
                    .into_iter()
                    .filter_map(|path| canonical_or_original(&path))
                    .collect();
                let rerun_targets = rerun_targets(&changed_paths, watch_paths, &dependency_map);
                if !rerun_targets.is_empty() && last_run.elapsed() > debounce {
                    last_run = Instant::now();
                    clear_screen();
                    run_fn(&rerun_targets);
                    dependency_map = build_dependency_map(watch_paths);
                    eprintln!("\n  Watching for changes... (Ctrl+C to stop)\n");
                }
            }
            Ok(Err(e)) => eprintln!("Watch error: {}", e),
            Err(_) => {
                std::process::exit(3);
            }
        }
    }
}

fn build_dependency_map(watch_paths: &[String]) -> HashMap<String, HashSet<PathBuf>> {
    watch_paths
        .iter()
        .map(|file_path| {
            let dependencies = crate::parser::include_dependencies(Path::new(file_path))
                .map(|deps| deps.into_iter().collect())
                .unwrap_or_default();
            (file_path.clone(), dependencies)
        })
        .collect()
}

fn watch_directories(
    watcher: &mut RecommendedWatcher,
    watch_paths: &[String],
    dependency_map: &HashMap<String, HashSet<PathBuf>>,
) {
    let mut watched = HashSet::new();
    for file_path in watch_paths {
        if let Some(dir) = Path::new(file_path).parent() {
            if watched.insert(dir.to_path_buf()) {
                let _ = watcher.watch(dir, RecursiveMode::Recursive);
            }
        }
        if let Some(dependencies) = dependency_map.get(file_path) {
            for dependency in dependencies {
                if let Some(dir) = dependency.parent() {
                    if watched.insert(dir.to_path_buf()) {
                        let _ = watcher.watch(dir, RecursiveMode::Recursive);
                    }
                }
            }
        }
    }
}

fn rerun_targets(
    changed_paths: &[PathBuf],
    watch_paths: &[String],
    dependency_map: &HashMap<String, HashSet<PathBuf>>,
) -> Vec<String> {
    if changed_paths
        .iter()
        .any(|path| is_global_rerun_trigger(path))
    {
        return watch_paths.to_vec();
    }

    let mut impacted = Vec::new();
    for file_path in watch_paths {
        let root = canonical_or_original(Path::new(file_path));
        let dependencies = dependency_map.get(file_path);
        let matches = changed_paths.iter().any(|changed| {
            root.as_ref().is_some_and(|root| root == changed)
                || dependencies
                    .is_some_and(|deps| deps.iter().any(|dependency| dependency == changed))
        });
        if matches {
            impacted.push(file_path.clone());
        }
    }

    impacted
}

fn is_global_rerun_trigger(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    name.starts_with("tarn.env") || name == "tarn.config.yaml"
}

fn canonical_or_original(path: &Path) -> Option<PathBuf> {
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
    }
}

#[cfg(test)]
fn should_watch_event(paths: &[PathBuf]) -> bool {
    !rerun_targets(paths, &[], &HashMap::new()).is_empty()
        || paths.iter().any(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            name.ends_with(".tarn.yaml") || is_global_rerun_trigger(path)
        })
}

fn clear_screen() {
    eprint!("\x1B[2J\x1B[1;1H");
}

#[cfg(test)]
mod tests {
    use super::{is_global_rerun_trigger, rerun_targets, should_watch_event};
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    #[test]
    fn reruns_for_test_files_and_env_files() {
        assert!(should_watch_event(&[PathBuf::from(
            "tests/health.tarn.yaml"
        )]));
        assert!(is_global_rerun_trigger(&PathBuf::from(
            "tarn.env.local.yaml"
        )));
        assert!(is_global_rerun_trigger(&PathBuf::from("tarn.config.yaml")));
    }

    #[test]
    fn ignores_unrelated_files() {
        assert!(!should_watch_event(&[PathBuf::from("README.md")]));
    }

    #[test]
    fn reruns_only_impacted_roots_for_include_changes() {
        let watch_paths = vec![
            "tests/a.tarn.yaml".to_string(),
            "tests/b.tarn.yaml".to_string(),
        ];
        let dependency_map = HashMap::from([
            (
                "tests/a.tarn.yaml".to_string(),
                HashSet::from([PathBuf::from("/tmp/shared-auth.tarn.yaml")]),
            ),
            ("tests/b.tarn.yaml".to_string(), HashSet::new()),
        ]);

        let impacted = rerun_targets(
            &[PathBuf::from("/tmp/shared-auth.tarn.yaml")],
            &watch_paths,
            &dependency_map,
        );
        assert_eq!(impacted, vec!["tests/a.tarn.yaml".to_string()]);
    }
}

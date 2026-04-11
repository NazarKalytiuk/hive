use crate::config::NamedEnvironmentConfig;
use crate::error::TarnError;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Origin of a resolved environment variable. Mirrors the layers applied in
/// [`resolve_env_with_profiles`] so editors and LSP clients can surface
/// *where* a value came from in hover tooltips.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvSource {
    /// From the inline `env:` block in the test file itself.
    InlineEnvBlock,
    /// From the default `tarn.env.yaml` (or a caller-configured equivalent).
    DefaultEnvFile {
        /// Display path of the env file that supplied the value.
        path: String,
    },
    /// From the `tarn.env.{name}.yaml` file for a named environment.
    NamedEnvFile {
        /// Display path of the env file that supplied the value.
        path: String,
        /// Name of the active environment (e.g. "staging").
        env_name: String,
    },
    /// From a `NamedEnvironmentConfig.vars` entry declared in `tarn.config.yaml`.
    NamedProfileVars {
        /// Name of the active environment profile.
        env_name: String,
    },
    /// From the `tarn.env.local.yaml` file (gitignored, secrets).
    LocalEnvFile {
        /// Display path of the local env file that supplied the value.
        path: String,
    },
    /// From a `--var KEY=VALUE` CLI override.
    CliVar,
}

impl EnvSource {
    /// Short human-readable label for this source. Stable — do not rename
    /// without bumping the LSP hover tests.
    pub fn label(&self) -> &str {
        match self {
            EnvSource::InlineEnvBlock => "inline env: block",
            EnvSource::DefaultEnvFile { .. } => "default env file",
            EnvSource::NamedEnvFile { .. } => "named env file",
            EnvSource::NamedProfileVars { .. } => "named profile vars",
            EnvSource::LocalEnvFile { .. } => "local env file",
            EnvSource::CliVar => "CLI --var",
        }
    }

    /// Display path of the file that supplied the value, if any. Returns
    /// `None` for sources that are not backed by a file on disk (inline,
    /// CLI overrides).
    pub fn source_file(&self) -> Option<&str> {
        match self {
            EnvSource::DefaultEnvFile { path }
            | EnvSource::NamedEnvFile { path, .. }
            | EnvSource::LocalEnvFile { path } => Some(path.as_str()),
            _ => None,
        }
    }
}

/// One resolved environment variable plus the layer that supplied it.
/// Returned by [`resolve_env_with_sources`] so LSP hovers can show the
/// effective value *and* where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvEntry {
    /// Final effective value after shell-variable expansion.
    pub value: String,
    /// The winning layer for this key.
    pub source: EnvSource,
}

/// Resolve environment variables with the priority chain:
/// CLI --var > shell env ${VAR} > tarn.env.local.yaml > tarn.env.{name}.yaml > tarn.env.yaml > inline env: block
pub fn resolve_env(
    inline_env: &HashMap<String, String>,
    env_name: Option<&str>,
    cli_vars: &[(String, String)],
    base_dir: &Path,
) -> Result<HashMap<String, String>, TarnError> {
    resolve_env_with_profiles(
        inline_env,
        env_name,
        cli_vars,
        base_dir,
        "tarn.env.yaml",
        &HashMap::new(),
    )
}

/// Resolve environment variables using a configurable env file name.
pub fn resolve_env_with_file(
    inline_env: &HashMap<String, String>,
    env_name: Option<&str>,
    cli_vars: &[(String, String)],
    base_dir: &Path,
    env_file_name: &str,
) -> Result<HashMap<String, String>, TarnError> {
    resolve_env_with_profiles(
        inline_env,
        env_name,
        cli_vars,
        base_dir,
        env_file_name,
        &HashMap::new(),
    )
}

/// Resolve environment variables using a configurable env file name and named profiles.
pub fn resolve_env_with_profiles(
    inline_env: &HashMap<String, String>,
    env_name: Option<&str>,
    cli_vars: &[(String, String)],
    base_dir: &Path,
    env_file_name: &str,
    profiles: &HashMap<String, NamedEnvironmentConfig>,
) -> Result<HashMap<String, String>, TarnError> {
    let mut env = HashMap::new();

    // Layer 1 (lowest): inline env: block from test file
    for (k, v) in inline_env {
        env.insert(k.clone(), v.clone());
    }

    // Layer 2: tarn.env.yaml (default env file)
    let default_env_file = base_dir.join(env_file_name);
    if default_env_file.exists() {
        let file_env = load_env_file(&default_env_file)?;
        for (k, v) in file_env {
            env.insert(k, v);
        }
    }

    // Layer 3: tarn.env.{name}.yaml (environment-specific)
    if let Some(name) = env_name {
        let named_env_file = profiles
            .get(name)
            .and_then(|profile| profile.env_file.as_ref().map(|path| base_dir.join(path)))
            .unwrap_or_else(|| base_dir.join(env_variant_filename(env_file_name, name)));
        if named_env_file.exists() {
            let file_env = load_env_file(&named_env_file)?;
            for (k, v) in file_env {
                env.insert(k, v);
            }
        }
        if let Some(profile) = profiles.get(name) {
            for (k, v) in &profile.vars {
                env.insert(k.clone(), v.clone());
            }
        }
    }

    // Layer 4: tarn.env.local.yaml (gitignored, secrets)
    let local_env_file = base_dir.join(env_variant_filename(env_file_name, "local"));
    if local_env_file.exists() {
        let file_env = load_env_file(&local_env_file)?;
        for (k, v) in file_env {
            env.insert(k, v);
        }
    }

    // Layer 5: shell environment variables (resolve ${VAR} references)
    let resolved: HashMap<String, String> = env
        .into_iter()
        .map(|(k, v)| {
            let resolved_v = resolve_shell_vars(&v);
            (k, resolved_v)
        })
        .collect();
    env = resolved;

    // Layer 6 (highest): CLI --var overrides
    for (k, v) in cli_vars {
        env.insert(k.clone(), v.clone());
    }

    Ok(env)
}

/// Resolve environment variables while preserving per-key provenance.
///
/// This is the LSP-facing counterpart to [`resolve_env_with_profiles`]: it
/// applies the exact same priority chain and shell-variable expansion, but
/// returns a map from key to [`EnvEntry`] so callers can report *which*
/// layer supplied each final value. The natural consumer is `tarn-lsp`'s
/// hover provider, which shows the effective value together with the file
/// path it came from.
///
/// The priority chain (lowest to highest — later layers overwrite earlier
/// ones) is identical to [`resolve_env_with_profiles`]:
///
///   1. inline `env:` block from the test file
///   2. `tarn.env.yaml` (or the caller-configured equivalent)
///   3. `tarn.env.{name}.yaml` for a named environment (if configured)
///   4. `NamedEnvironmentConfig.vars` from `tarn.config.yaml`
///   5. `tarn.env.local.yaml` (gitignored, secrets)
///   6. CLI `--var KEY=VALUE`
///
/// Between layers 5 and 6 every value is passed through
/// [`resolve_shell_vars`] so `${VAR}` placeholders in env-file values
/// expand to their shell values.
pub fn resolve_env_with_sources(
    inline_env: &HashMap<String, String>,
    env_name: Option<&str>,
    cli_vars: &[(String, String)],
    base_dir: &Path,
    env_file_name: &str,
    profiles: &HashMap<String, NamedEnvironmentConfig>,
) -> Result<BTreeMap<String, EnvEntry>, TarnError> {
    let mut env: BTreeMap<String, EnvEntry> = BTreeMap::new();

    // Layer 1: inline env block from the test file.
    for (k, v) in inline_env {
        env.insert(
            k.clone(),
            EnvEntry {
                value: v.clone(),
                source: EnvSource::InlineEnvBlock,
            },
        );
    }

    // Layer 2: default env file (tarn.env.yaml).
    let default_env_file = base_dir.join(env_file_name);
    if default_env_file.exists() {
        let file_env = load_env_file(&default_env_file)?;
        let display = default_env_file.display().to_string();
        for (k, v) in file_env {
            env.insert(
                k,
                EnvEntry {
                    value: v,
                    source: EnvSource::DefaultEnvFile {
                        path: display.clone(),
                    },
                },
            );
        }
    }

    // Layer 3: named env file (tarn.env.{name}.yaml or profile override).
    if let Some(name) = env_name {
        let named_env_file = profiles
            .get(name)
            .and_then(|profile| profile.env_file.as_ref().map(|path| base_dir.join(path)))
            .unwrap_or_else(|| base_dir.join(env_variant_filename(env_file_name, name)));
        if named_env_file.exists() {
            let file_env = load_env_file(&named_env_file)?;
            let display = named_env_file.display().to_string();
            for (k, v) in file_env {
                env.insert(
                    k,
                    EnvEntry {
                        value: v,
                        source: EnvSource::NamedEnvFile {
                            path: display.clone(),
                            env_name: name.to_owned(),
                        },
                    },
                );
            }
        }
        if let Some(profile) = profiles.get(name) {
            for (k, v) in &profile.vars {
                env.insert(
                    k.clone(),
                    EnvEntry {
                        value: v.clone(),
                        source: EnvSource::NamedProfileVars {
                            env_name: name.to_owned(),
                        },
                    },
                );
            }
        }
    }

    // Layer 4: tarn.env.local.yaml (gitignored, secrets).
    let local_env_file = base_dir.join(env_variant_filename(env_file_name, "local"));
    if local_env_file.exists() {
        let file_env = load_env_file(&local_env_file)?;
        let display = local_env_file.display().to_string();
        for (k, v) in file_env {
            env.insert(
                k,
                EnvEntry {
                    value: v,
                    source: EnvSource::LocalEnvFile {
                        path: display.clone(),
                    },
                },
            );
        }
    }

    // Layer 5: shell variable expansion of ${VAR} placeholders in each
    // already-resolved value. The *source* of the entry does not change —
    // shell expansion is a post-processing step, not a new layer.
    for entry in env.values_mut() {
        entry.value = resolve_shell_vars(&entry.value);
    }

    // Layer 6 (highest): CLI --var overrides.
    for (k, v) in cli_vars {
        env.insert(
            k.clone(),
            EnvEntry {
                value: v.clone(),
                source: EnvSource::CliVar,
            },
        );
    }

    Ok(env)
}

fn env_variant_filename(env_file_name: &str, suffix: &str) -> PathBuf {
    let path = Path::new(env_file_name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(env_file_name);

    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => PathBuf::from(format!("{stem}.{suffix}.{ext}")),
        None => PathBuf::from(format!("{stem}.{suffix}")),
    }
}

/// Load an env file (YAML key-value pairs).
fn load_env_file(path: &Path) -> Result<HashMap<String, String>, TarnError> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        TarnError::Config(format!("Failed to read env file {}: {}", path.display(), e))
    })?;

    let map: HashMap<String, serde_yaml::Value> = serde_yaml::from_str(&content).map_err(|e| {
        TarnError::Config(format!(
            "Failed to parse env file {}: {}",
            path.display(),
            e
        ))
    })?;

    Ok(map
        .into_iter()
        .map(|(k, v)| {
            let s = match v {
                serde_yaml::Value::String(s) => s,
                serde_yaml::Value::Number(n) => n.to_string(),
                serde_yaml::Value::Bool(b) => b.to_string(),
                serde_yaml::Value::Null => String::new(),
                other => format!("{:?}", other),
            };
            (k, s)
        })
        .collect())
}

/// Resolve ${VAR_NAME} references in a string using shell environment variables.
fn resolve_shell_vars(value: &str) -> String {
    let mut result = value.to_string();
    // Find all ${VAR} patterns
    while let Some(start) = result.find("${") {
        if let Some(end) = result[start..].find('}') {
            let var_name = &result[start + 2..start + end];
            let replacement = std::env::var(var_name).unwrap_or_default();
            result = format!(
                "{}{}{}",
                &result[..start],
                replacement,
                &result[start + end + 1..]
            );
        } else {
            break;
        }
    }
    result
}

/// Parse CLI --var arguments from "key=value" format.
pub fn parse_cli_vars(vars: &[String]) -> Result<Vec<(String, String)>, TarnError> {
    vars.iter()
        .map(|s| {
            let parts: Vec<&str> = s.splitn(2, '=').collect();
            if parts.len() != 2 {
                Err(TarnError::Config(format!(
                    "Invalid --var format '{}': expected key=value",
                    s
                )))
            } else {
                Ok((parts[0].to_string(), parts[1].to_string()))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn setup_env_files(dir: &TempDir, files: &[(&str, &str)]) {
        for (name, content) in files {
            let path = dir.path().join(name);
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(content.as_bytes()).unwrap();
        }
    }

    // --- Priority chain ---

    #[test]
    fn inline_env_is_base_layer() {
        let dir = TempDir::new().unwrap();
        let mut inline = HashMap::new();
        inline.insert("base_url".into(), "http://localhost:3000".into());

        let env = resolve_env(&inline, None, &[], dir.path()).unwrap();
        assert_eq!(env.get("base_url").unwrap(), "http://localhost:3000");
    }

    #[test]
    fn env_file_overrides_inline() {
        let dir = TempDir::new().unwrap();
        setup_env_files(&dir, &[("tarn.env.yaml", "base_url: http://from-file")]);

        let mut inline = HashMap::new();
        inline.insert("base_url".into(), "http://inline".into());

        let env = resolve_env(&inline, None, &[], dir.path()).unwrap();
        assert_eq!(env.get("base_url").unwrap(), "http://from-file");
    }

    #[test]
    fn named_env_overrides_default() {
        let dir = TempDir::new().unwrap();
        setup_env_files(
            &dir,
            &[
                ("tarn.env.yaml", "base_url: http://default"),
                ("tarn.env.staging.yaml", "base_url: http://staging"),
            ],
        );

        let env = resolve_env(&HashMap::new(), Some("staging"), &[], dir.path()).unwrap();
        assert_eq!(env.get("base_url").unwrap(), "http://staging");
    }

    #[test]
    fn named_profile_uses_custom_env_file_and_vars() {
        let dir = TempDir::new().unwrap();
        setup_env_files(
            &dir,
            &[
                ("tarn.env.yaml", "base_url: http://default\nregion: local"),
                ("env.staging.yaml", "base_url: http://from-profile-file"),
            ],
        );

        let mut profiles = HashMap::new();
        profiles.insert(
            "staging".into(),
            NamedEnvironmentConfig {
                env_file: Some("env.staging.yaml".into()),
                vars: HashMap::from([("region".into(), "eu-west-1".into())]),
            },
        );

        let env = resolve_env_with_profiles(
            &HashMap::new(),
            Some("staging"),
            &[],
            dir.path(),
            "tarn.env.yaml",
            &profiles,
        )
        .unwrap();
        assert_eq!(env.get("base_url").unwrap(), "http://from-profile-file");
        assert_eq!(env.get("region").unwrap(), "eu-west-1");
    }

    #[test]
    fn local_env_overrides_named() {
        let dir = TempDir::new().unwrap();
        setup_env_files(
            &dir,
            &[
                ("tarn.env.yaml", "base_url: http://default"),
                ("tarn.env.staging.yaml", "base_url: http://staging"),
                ("tarn.env.local.yaml", "base_url: http://local"),
            ],
        );

        let env = resolve_env(&HashMap::new(), Some("staging"), &[], dir.path()).unwrap();
        assert_eq!(env.get("base_url").unwrap(), "http://local");
    }

    #[test]
    fn cli_var_overrides_all() {
        let dir = TempDir::new().unwrap();
        setup_env_files(
            &dir,
            &[
                ("tarn.env.yaml", "base_url: http://default"),
                ("tarn.env.local.yaml", "base_url: http://local"),
            ],
        );

        let mut inline = HashMap::new();
        inline.insert("base_url".into(), "http://inline".into());

        let cli_vars = vec![("base_url".into(), "http://cli".into())];
        let env = resolve_env(&inline, None, &cli_vars, dir.path()).unwrap();
        assert_eq!(env.get("base_url").unwrap(), "http://cli");
    }

    // --- Shell variable resolution ---

    #[test]
    fn resolve_shell_variable() {
        std::env::set_var("HIVE_TEST_SECRET", "s3cret");
        let result = resolve_shell_vars("password is ${HIVE_TEST_SECRET}");
        assert_eq!(result, "password is s3cret");
        std::env::remove_var("HIVE_TEST_SECRET");
    }

    #[test]
    fn resolve_missing_shell_variable_becomes_empty() {
        let result = resolve_shell_vars("${HIVE_NONEXISTENT_VAR}");
        assert_eq!(result, "");
    }

    #[test]
    fn resolve_multiple_shell_variables() {
        std::env::set_var("HIVE_TEST_A", "alpha");
        std::env::set_var("HIVE_TEST_B", "beta");
        let result = resolve_shell_vars("${HIVE_TEST_A} and ${HIVE_TEST_B}");
        assert_eq!(result, "alpha and beta");
        std::env::remove_var("HIVE_TEST_A");
        std::env::remove_var("HIVE_TEST_B");
    }

    #[test]
    fn no_shell_vars_unchanged() {
        let result = resolve_shell_vars("no variables here");
        assert_eq!(result, "no variables here");
    }

    // --- Env file parsing ---

    #[test]
    fn load_env_file_with_various_types() {
        let dir = TempDir::new().unwrap();
        setup_env_files(
            &dir,
            &[(
                "test.yaml",
                "string_val: hello\nnumber_val: 42\nbool_val: true\nnull_val: null",
            )],
        );

        let env = load_env_file(&dir.path().join("test.yaml")).unwrap();
        assert_eq!(env.get("string_val").unwrap(), "hello");
        assert_eq!(env.get("number_val").unwrap(), "42");
        assert_eq!(env.get("bool_val").unwrap(), "true");
        assert_eq!(env.get("null_val").unwrap(), "");
    }

    #[test]
    fn load_env_file_missing() {
        let result = load_env_file(Path::new("/nonexistent/file.yaml"));
        assert!(result.is_err());
    }

    #[test]
    fn load_env_file_invalid_yaml() {
        let dir = TempDir::new().unwrap();
        setup_env_files(&dir, &[("bad.yaml", "not: [valid: yaml")]);
        let result = load_env_file(&dir.path().join("bad.yaml"));
        assert!(result.is_err());
    }

    // --- CLI var parsing ---

    #[test]
    fn parse_cli_vars_valid() {
        let vars = vec!["key=value".into(), "another=with=equals".into()];
        let result = parse_cli_vars(&vars).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], ("key".into(), "value".into()));
        assert_eq!(result[1], ("another".into(), "with=equals".into()));
    }

    #[test]
    fn parse_cli_vars_invalid() {
        let vars = vec!["no_equals_sign".into()];
        let result = parse_cli_vars(&vars);
        assert!(result.is_err());
    }

    #[test]
    fn parse_cli_vars_empty() {
        let vars: Vec<String> = vec![];
        let result = parse_cli_vars(&vars).unwrap();
        assert!(result.is_empty());
    }

    // --- Merging ---

    #[test]
    fn variables_from_different_layers_merge() {
        let dir = TempDir::new().unwrap();
        setup_env_files(
            &dir,
            &[("tarn.env.yaml", "file_only: from_file\nbase_url: from_file")],
        );

        let mut inline = HashMap::new();
        inline.insert("inline_only".into(), "from_inline".into());
        inline.insert("base_url".into(), "from_inline".into());

        let env = resolve_env(&inline, None, &[], dir.path()).unwrap();
        assert_eq!(env.get("inline_only").unwrap(), "from_inline");
        assert_eq!(env.get("file_only").unwrap(), "from_file");
        assert_eq!(env.get("base_url").unwrap(), "from_file"); // file overrides inline
    }

    #[test]
    fn missing_named_env_file_is_ok() {
        let dir = TempDir::new().unwrap();
        // No tarn.env.staging.yaml exists
        let env = resolve_env(&HashMap::new(), Some("staging"), &[], dir.path()).unwrap();
        assert!(env.is_empty());
    }

    #[test]
    fn custom_env_file_supports_named_and_local_variants() {
        let dir = TempDir::new().unwrap();
        setup_env_files(
            &dir,
            &[
                ("custom.env.yaml", "base_url: http://default"),
                ("custom.env.staging.yaml", "base_url: http://staging"),
                ("custom.env.local.yaml", "token: secret"),
            ],
        );

        let env = resolve_env_with_file(
            &HashMap::new(),
            Some("staging"),
            &[],
            dir.path(),
            "custom.env.yaml",
        )
        .unwrap();

        assert_eq!(env.get("base_url").unwrap(), "http://staging");
        assert_eq!(env.get("token").unwrap(), "secret");
    }

    #[test]
    fn env_variant_filename_inserts_suffix_before_extension() {
        assert_eq!(
            env_variant_filename("tarn.env.yaml", "local"),
            PathBuf::from("tarn.env.local.yaml")
        );
        assert_eq!(
            env_variant_filename("custom.env.yaml", "staging"),
            PathBuf::from("custom.env.staging.yaml")
        );
    }

    // --- resolve_env_with_sources ---

    #[test]
    fn resolve_env_with_sources_tags_inline_entries() {
        let dir = TempDir::new().unwrap();
        let mut inline = HashMap::new();
        inline.insert("base_url".into(), "http://localhost:3000".into());

        let env = resolve_env_with_sources(
            &inline,
            None,
            &[],
            dir.path(),
            "tarn.env.yaml",
            &HashMap::new(),
        )
        .unwrap();
        let entry = env.get("base_url").unwrap();
        assert_eq!(entry.value, "http://localhost:3000");
        assert!(matches!(entry.source, EnvSource::InlineEnvBlock));
    }

    #[test]
    fn resolve_env_with_sources_reports_default_env_file_path() {
        let dir = TempDir::new().unwrap();
        setup_env_files(&dir, &[("tarn.env.yaml", "base_url: http://from-file")]);

        let env = resolve_env_with_sources(
            &HashMap::new(),
            None,
            &[],
            dir.path(),
            "tarn.env.yaml",
            &HashMap::new(),
        )
        .unwrap();
        let entry = env.get("base_url").unwrap();
        assert_eq!(entry.value, "http://from-file");
        match &entry.source {
            EnvSource::DefaultEnvFile { path } => {
                assert!(path.ends_with("tarn.env.yaml"), "got path: {}", path);
            }
            other => panic!("expected DefaultEnvFile source, got {:?}", other),
        }
    }

    #[test]
    fn resolve_env_with_sources_named_env_file_wins_over_default() {
        let dir = TempDir::new().unwrap();
        setup_env_files(
            &dir,
            &[
                ("tarn.env.yaml", "base_url: http://default"),
                ("tarn.env.staging.yaml", "base_url: http://staging"),
            ],
        );

        let env = resolve_env_with_sources(
            &HashMap::new(),
            Some("staging"),
            &[],
            dir.path(),
            "tarn.env.yaml",
            &HashMap::new(),
        )
        .unwrap();
        let entry = env.get("base_url").unwrap();
        assert_eq!(entry.value, "http://staging");
        match &entry.source {
            EnvSource::NamedEnvFile { env_name, path } => {
                assert_eq!(env_name, "staging");
                assert!(path.ends_with("tarn.env.staging.yaml"));
            }
            other => panic!("expected NamedEnvFile source, got {:?}", other),
        }
    }

    #[test]
    fn resolve_env_with_sources_cli_var_has_highest_priority() {
        let dir = TempDir::new().unwrap();
        setup_env_files(
            &dir,
            &[
                ("tarn.env.yaml", "base_url: http://default"),
                ("tarn.env.local.yaml", "base_url: http://local"),
            ],
        );

        let cli_vars = vec![("base_url".into(), "http://cli".into())];
        let env = resolve_env_with_sources(
            &HashMap::new(),
            None,
            &cli_vars,
            dir.path(),
            "tarn.env.yaml",
            &HashMap::new(),
        )
        .unwrap();
        let entry = env.get("base_url").unwrap();
        assert_eq!(entry.value, "http://cli");
        assert!(matches!(entry.source, EnvSource::CliVar));
    }

    #[test]
    fn resolve_env_with_sources_local_env_file_overrides_named() {
        let dir = TempDir::new().unwrap();
        setup_env_files(
            &dir,
            &[
                ("tarn.env.yaml", "base_url: http://default"),
                ("tarn.env.staging.yaml", "base_url: http://staging"),
                ("tarn.env.local.yaml", "base_url: http://local"),
            ],
        );

        let env = resolve_env_with_sources(
            &HashMap::new(),
            Some("staging"),
            &[],
            dir.path(),
            "tarn.env.yaml",
            &HashMap::new(),
        )
        .unwrap();
        let entry = env.get("base_url").unwrap();
        assert_eq!(entry.value, "http://local");
        assert!(matches!(entry.source, EnvSource::LocalEnvFile { .. }));
    }

    #[test]
    fn env_source_label_and_source_file_accessors_are_stable() {
        assert_eq!(EnvSource::InlineEnvBlock.label(), "inline env: block");
        assert_eq!(EnvSource::CliVar.label(), "CLI --var");
        assert_eq!(EnvSource::InlineEnvBlock.source_file(), None);
        assert_eq!(EnvSource::CliVar.source_file(), None);
        assert_eq!(
            EnvSource::DefaultEnvFile {
                path: "/tmp/tarn.env.yaml".into()
            }
            .source_file(),
            Some("/tmp/tarn.env.yaml")
        );
    }
}

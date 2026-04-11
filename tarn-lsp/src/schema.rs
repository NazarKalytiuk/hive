//! Shared JSON-Schema accessors for `schemas/v1/testfile.json`.
//!
//! L1.3 (hover) needs top-level key descriptions. L1.4 (completion)
//! needs both the descriptions and the set of valid keys per YAML
//! scope — root, test-group, step. Rather than duplicate schema
//! parsing in every feature, both features read from this module.
//!
//! The schema file is baked in with `include_str!`, so clients do not
//! need the workspace on disk at runtime. Parsing is done exactly once
//! per process behind a `OnceLock`, so every subsequent lookup is an
//! in-memory map read.
//!
//! Scope note: nested-object completion (e.g. `assert.body.*`,
//! `request.headers.*`) is deliberately out of scope for Phase L1 and
//! NAZ-293. The cache exposes the top-level / test / step key sets
//! only. Phase L3 may revisit this when the VS Code provider grows
//! nested-object completion.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Shared schema key cache. Populated lazily on first access.
///
/// The `get_*` helpers return borrowed slices into the cache so
/// callers never need to clone the descriptions.
#[derive(Debug)]
pub struct SchemaKeyCache {
    /// Map of every `properties` entry anywhere in the schema →
    /// its `description` (with local `$ref` chains resolved).
    descriptions: HashMap<String, String>,
    /// Top-level root keys of a `.tarn.yaml` test file.
    root_keys: Vec<&'static str>,
    /// Keys allowed on a named test group (`tests.<name>.*`).
    test_keys: Vec<&'static str>,
    /// Keys allowed on a single step (`steps[*]` / `setup[*]` / …).
    step_keys: Vec<&'static str>,
}

impl SchemaKeyCache {
    pub fn description(&self, key: &str) -> Option<&str> {
        self.descriptions.get(key).map(String::as_str)
    }

    pub fn descriptions(&self) -> &HashMap<String, String> {
        &self.descriptions
    }

    pub fn root_keys(&self) -> &[&'static str] {
        &self.root_keys
    }

    pub fn test_keys(&self) -> &[&'static str] {
        &self.test_keys
    }

    pub fn step_keys(&self) -> &[&'static str] {
        &self.step_keys
    }
}

/// Top-level keys of a `.tarn.yaml` test file. Kept in source as a
/// stable hard-coded list (rather than derived from the schema at
/// runtime) so the set the completion provider offers is obvious by
/// inspection and has a single place to update when the schema grows.
const ROOT_KEYS: &[&str] = &[
    "version",
    "name",
    "description",
    "tags",
    "env",
    "cookies",
    "redaction",
    "defaults",
    "setup",
    "teardown",
    "tests",
    "steps",
];

/// Keys allowed on a named test group inside `tests:`. Driven by the
/// `$defs/TestGroup.properties` block in the schema.
const TEST_KEYS: &[&str] = &["description", "tags", "steps"];

/// Keys allowed on a single step. Driven by `$defs/Step.properties`.
///
/// `include` is offered as a sibling because `StepOrInclude` allows
/// either a step or an include directive at the same array slot —
/// completion surfaces both so the user can pick.
const STEP_KEYS: &[&str] = &[
    "name",
    "request",
    "capture",
    "assert",
    "retries",
    "timeout",
    "connect_timeout",
    "follow_redirects",
    "max_redirs",
    "delay",
    "poll",
    "script",
    "cookies",
    "include",
];

/// Return the process-wide cached [`SchemaKeyCache`]. Parses the
/// schema on first call; every subsequent call is an atomic load.
pub fn schema_key_cache() -> &'static SchemaKeyCache {
    static CACHE: OnceLock<SchemaKeyCache> = OnceLock::new();
    CACHE.get_or_init(|| {
        let raw = include_str!("../../schemas/v1/testfile.json");
        let descriptions = parse_schema_descriptions(raw).unwrap_or_default();
        SchemaKeyCache {
            descriptions,
            root_keys: ROOT_KEYS.to_vec(),
            test_keys: TEST_KEYS.to_vec(),
            step_keys: STEP_KEYS.to_vec(),
        }
    })
}

/// Top-level + nested descriptions from `testfile.json`.
///
/// Walks the whole schema document; the first description encountered
/// for a given key wins. Top-level `properties` are visited before
/// `$defs`, so `status`, `body`, `headers` resolve to the top-level
/// `Assertion` descriptions rather than their inner re-definitions.
fn parse_schema_descriptions(raw: &str) -> Option<HashMap<String, String>> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let mut out = HashMap::new();
    collect_descriptions_recursive(&value, &value, &mut out);
    Some(out)
}

fn collect_descriptions_recursive(
    root: &serde_json::Value,
    value: &serde_json::Value,
    out: &mut HashMap<String, String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::Object(props)) = map.get("properties") {
                for (key, prop) in props {
                    if let Some(desc) = extract_description(root, prop) {
                        out.entry(key.clone()).or_insert(desc);
                    }
                }
            }
            for v in map.values() {
                collect_descriptions_recursive(root, v, out);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                collect_descriptions_recursive(root, v, out);
            }
        }
        _ => {}
    }
}

/// Inline `description` wins; otherwise follow a local `$ref` and use
/// the referenced schema's `description`. External refs are ignored.
fn extract_description(root: &serde_json::Value, prop: &serde_json::Value) -> Option<String> {
    if let Some(serde_json::Value::String(desc)) = prop.get("description") {
        return Some(desc.clone());
    }
    if let Some(serde_json::Value::String(r)) = prop.get("$ref") {
        if let Some(target) = resolve_local_ref(root, r) {
            if let Some(serde_json::Value::String(desc)) = target.get("description") {
                return Some(desc.clone());
            }
        }
    }
    None
}

/// Resolve `#/path/to/schema`. External refs return `None`.
fn resolve_local_ref<'a>(
    root: &'a serde_json::Value,
    reference: &str,
) -> Option<&'a serde_json::Value> {
    let path = reference.strip_prefix("#/")?;
    let mut current = root;
    for segment in path.split('/') {
        let unescaped = segment.replace("~1", "/").replace("~0", "~");
        current = current.get(&unescaped)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_root_keys_match_known_top_level_fields() {
        let cache = schema_key_cache();
        let roots = cache.root_keys();
        assert!(roots.contains(&"name"));
        assert!(roots.contains(&"env"));
        assert!(roots.contains(&"tests"));
        assert!(roots.contains(&"steps"));
        assert!(roots.contains(&"defaults"));
    }

    #[test]
    fn cache_step_keys_match_step_schema() {
        let cache = schema_key_cache();
        let steps = cache.step_keys();
        assert!(steps.contains(&"name"));
        assert!(steps.contains(&"request"));
        assert!(steps.contains(&"capture"));
        assert!(steps.contains(&"assert"));
        assert!(steps.contains(&"poll"));
        assert!(steps.contains(&"include"));
    }

    #[test]
    fn cache_test_keys_match_testgroup_schema() {
        let cache = schema_key_cache();
        let tests = cache.test_keys();
        assert!(tests.contains(&"description"));
        assert!(tests.contains(&"steps"));
        assert!(tests.contains(&"tags"));
    }

    #[test]
    fn descriptions_include_top_level_keys() {
        let cache = schema_key_cache();
        for key in &["name", "env", "defaults", "setup", "tests"] {
            assert!(
                cache.description(key).is_some(),
                "missing description for `{key}`"
            );
        }
    }

    #[test]
    fn descriptions_include_nested_assertion_keys() {
        let cache = schema_key_cache();
        // Walked out of `$defs/Assertion.properties`.
        assert!(cache.description("status").is_some());
        assert!(cache.description("body").is_some());
        assert!(cache.description("headers").is_some());
    }

    #[test]
    fn schema_parses_successfully() {
        // If the bundled schema ever gets corrupted this is the
        // fastest signal — the cache would return an empty map.
        let cache = schema_key_cache();
        assert!(!cache.descriptions().is_empty());
    }
}

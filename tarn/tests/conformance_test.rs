use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct ConformanceManifest {
    schema_version: u32,
    cases: Vec<ConformanceCase>,
}

#[derive(Debug, Deserialize)]
struct ConformanceCase {
    id: String,
    mode: String,
    path: String,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn load_manifest() -> ConformanceManifest {
    let path = repo_root().join("tarn/tests/conformance/manifest.json");
    let content = std::fs::read_to_string(path).unwrap();
    serde_json::from_str(&content).unwrap()
}

#[test]
fn conformance_manifest_is_valid() {
    let manifest = load_manifest();
    assert_eq!(manifest.schema_version, 1);
    assert!(!manifest.cases.is_empty());
}

#[test]
fn conformance_cases_pass() {
    let manifest = load_manifest();
    let root = repo_root();

    for case in manifest.cases {
        let path = root.join(&case.path);
        assert!(
            path.exists(),
            "Missing conformance case file: {}",
            case.path
        );

        match case.mode.as_str() {
            "parse" => {
                tarn::parser::parse_file(&path).unwrap_or_else(|error| {
                    panic!("Conformance case '{}' failed to parse: {}", case.id, error)
                });
            }
            "format" => {
                let original = std::fs::read_to_string(&path).unwrap();
                let formatted =
                    tarn::parser::format_str(&original, &path).unwrap_or_else(|error| {
                        panic!("Conformance case '{}' failed to format: {}", case.id, error)
                    });
                tarn::parser::parse_str(&formatted, &path).unwrap_or_else(|error| {
                    panic!(
                        "Conformance case '{}' failed to round-trip: {}",
                        case.id, error
                    )
                });
            }
            other => panic!("Unknown conformance mode '{}'", other),
        }
    }
}

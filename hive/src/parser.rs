use crate::error::HiveError;
use crate::model::TestFile;
use std::path::Path;

/// Parse a .hive.yaml file into a TestFile struct.
pub fn parse_file(path: &Path) -> Result<TestFile, HiveError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| HiveError::Parse(format!("Failed to read {}: {}", path.display(), e)))?;
    parse_str(&content, path)
}

/// Parse YAML content string into a TestFile struct.
pub fn parse_str(content: &str, path: &Path) -> Result<TestFile, HiveError> {
    let test_file: TestFile = serde_yaml::from_str(content)
        .map_err(|e| HiveError::Parse(format!("Failed to parse {}: {}", path.display(), e)))?;
    validate_test_file(&test_file, path)?;
    Ok(test_file)
}

/// Validate semantic constraints on a parsed TestFile.
fn validate_test_file(tf: &TestFile, path: &Path) -> Result<(), HiveError> {
    // Must have either steps or tests (or both for setup+tests pattern)
    if tf.steps.is_empty() && tf.tests.is_empty() {
        return Err(HiveError::Parse(format!(
            "{}: Test file must have either 'steps' or 'tests'",
            path.display()
        )));
    }

    // Validate each step has a non-empty name
    let all_steps = tf
        .setup
        .iter()
        .chain(tf.teardown.iter())
        .chain(tf.steps.iter())
        .chain(tf.tests.values().flat_map(|t| t.steps.iter()));

    for step in all_steps {
        if step.name.trim().is_empty() {
            return Err(HiveError::Parse(format!(
                "{}: Step name cannot be empty",
                path.display()
            )));
        }
        if step.request.method.trim().is_empty() {
            return Err(HiveError::Parse(format!(
                "{}: Step '{}' has empty HTTP method",
                path.display(),
                step.name
            )));
        }
        if step.request.url.trim().is_empty() {
            return Err(HiveError::Parse(format!(
                "{}: Step '{}' has empty URL",
                path.display(),
                step.name
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn parse_yaml(yaml: &str) -> Result<TestFile, HiveError> {
        parse_str(yaml, Path::new("test.hive.yaml"))
    }

    #[test]
    fn parse_minimal_yaml() {
        let tf = parse_yaml(
            r#"
name: Health check
steps:
  - name: GET /health
    request:
      method: GET
      url: "http://localhost:3000/health"
    assert:
      status: 200
"#,
        )
        .unwrap();
        assert_eq!(tf.name, "Health check");
        assert_eq!(tf.steps.len(), 1);
    }

    #[test]
    fn parse_file_from_disk() {
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            r#"
name: Disk test
steps:
  - name: Check
    request:
      method: GET
      url: "http://localhost:3000"
    assert:
      status: 200
"#
        )
        .unwrap();

        let tf = parse_file(file.path()).unwrap();
        assert_eq!(tf.name, "Disk test");
    }

    #[test]
    fn error_on_missing_file() {
        let result = parse_file(Path::new("/nonexistent/test.hive.yaml"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, HiveError::Parse(_)));
        assert!(err.to_string().contains("Failed to read"));
    }

    #[test]
    fn error_on_invalid_yaml() {
        let result = parse_yaml("not: [valid: yaml: content");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to parse"));
    }

    #[test]
    fn error_on_missing_name_field() {
        let result = parse_yaml(
            r#"
steps:
  - name: test
    request:
      method: GET
      url: "http://localhost:3000"
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn error_on_no_steps_or_tests() {
        let result = parse_yaml(
            r#"
name: Empty test
"#,
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must have either 'steps' or 'tests'"));
    }

    #[test]
    fn error_on_empty_step_name() {
        let result = parse_yaml(
            r#"
name: Bad step
steps:
  - name: ""
    request:
      method: GET
      url: "http://localhost:3000"
"#,
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Step name cannot be empty"));
    }

    #[test]
    fn error_on_empty_method() {
        let result = parse_yaml(
            r#"
name: Bad method
steps:
  - name: test
    request:
      method: ""
      url: "http://localhost:3000"
"#,
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("empty HTTP method"));
    }

    #[test]
    fn error_on_empty_url() {
        let result = parse_yaml(
            r#"
name: Bad url
steps:
  - name: test
    request:
      method: GET
      url: ""
"#,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty URL"));
    }

    #[test]
    fn parse_file_with_tests_map() {
        let tf = parse_yaml(
            r#"
name: Test map
tests:
  login:
    description: "Login test"
    steps:
      - name: Login
        request:
          method: POST
          url: "http://localhost:3000/login"
"#,
        )
        .unwrap();
        assert_eq!(tf.tests.len(), 1);
        assert!(tf.tests.contains_key("login"));
    }

    #[test]
    fn validates_setup_and_teardown_steps() {
        let result = parse_yaml(
            r#"
name: Bad setup
setup:
  - name: ""
    request:
      method: GET
      url: "http://localhost:3000"
steps:
  - name: OK step
    request:
      method: GET
      url: "http://localhost:3000"
"#,
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Step name cannot be empty"));
    }
}

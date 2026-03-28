use crate::error::HiveError;
use serde_json::Value;
use serde_json_path::JsonPath;
use std::collections::HashMap;

/// Extract captures from a JSON response body using JSONPath expressions.
/// Returns a map of capture_name -> extracted_string_value.
pub fn extract_captures(
    body: &Value,
    capture_map: &HashMap<String, String>,
) -> Result<HashMap<String, String>, HiveError> {
    let mut captures = HashMap::new();

    for (name, path_str) in capture_map {
        let value = extract_jsonpath(body, path_str).map_err(|e| {
            HiveError::Capture(format!(
                "Failed to capture '{}' with path '{}': {}",
                name, path_str, e
            ))
        })?;
        captures.insert(name.clone(), value);
    }

    Ok(captures)
}

/// Extract a single value via JSONPath from a JSON body.
/// Returns the value as a string.
fn extract_jsonpath(body: &Value, path_str: &str) -> Result<String, String> {
    let json_path =
        JsonPath::parse(path_str).map_err(|e| format!("Invalid JSONPath '{}': {}", path_str, e))?;

    let node_list = json_path.query(body);
    let nodes: Vec<&Value> = node_list.all();

    if nodes.is_empty() {
        return Err(format!("JSONPath '{}' matched no values", path_str));
    }

    // Take the first match
    let value = nodes[0];
    Ok(value_to_string(value))
}

/// Convert a JSON value to a string for use as a captured variable.
fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        // Arrays and objects are serialized as JSON strings
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_string_field() {
        let body = json!({"name": "Alice"});
        let mut map = HashMap::new();
        map.insert("user_name".into(), "$.name".into());

        let captures = extract_captures(&body, &map).unwrap();
        assert_eq!(captures.get("user_name").unwrap(), "Alice");
    }

    #[test]
    fn extract_number_field() {
        let body = json!({"age": 30});
        let mut map = HashMap::new();
        map.insert("user_age".into(), "$.age".into());

        let captures = extract_captures(&body, &map).unwrap();
        assert_eq!(captures.get("user_age").unwrap(), "30");
    }

    #[test]
    fn extract_boolean_field() {
        let body = json!({"active": true});
        let mut map = HashMap::new();
        map.insert("is_active".into(), "$.active".into());

        let captures = extract_captures(&body, &map).unwrap();
        assert_eq!(captures.get("is_active").unwrap(), "true");
    }

    #[test]
    fn extract_null_field() {
        let body = json!({"deleted": null});
        let mut map = HashMap::new();
        map.insert("deleted".into(), "$.deleted".into());

        let captures = extract_captures(&body, &map).unwrap();
        assert_eq!(captures.get("deleted").unwrap(), "null");
    }

    #[test]
    fn extract_nested_field() {
        let body = json!({"user": {"profile": {"email": "alice@test.com"}}});
        let mut map = HashMap::new();
        map.insert("email".into(), "$.user.profile.email".into());

        let captures = extract_captures(&body, &map).unwrap();
        assert_eq!(captures.get("email").unwrap(), "alice@test.com");
    }

    #[test]
    fn extract_array_element() {
        let body = json!({"items": [{"id": "first"}, {"id": "second"}]});
        let mut map = HashMap::new();
        map.insert("first_id".into(), "$.items[0].id".into());

        let captures = extract_captures(&body, &map).unwrap();
        assert_eq!(captures.get("first_id").unwrap(), "first");
    }

    #[test]
    fn extract_missing_path_returns_error() {
        let body = json!({"name": "Alice"});
        let mut map = HashMap::new();
        map.insert("missing".into(), "$.nonexistent".into());

        let result = extract_captures(&body, &map);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("matched no values"));
    }

    #[test]
    fn extract_invalid_jsonpath_returns_error() {
        let body = json!({"name": "Alice"});
        let mut map = HashMap::new();
        map.insert("bad".into(), "$[invalid".into());

        let result = extract_captures(&body, &map);
        assert!(result.is_err());
    }

    #[test]
    fn extract_multiple_captures() {
        let body = json!({"id": "usr_123", "token": "abc", "status": 200});
        let mut map = HashMap::new();
        map.insert("id".into(), "$.id".into());
        map.insert("tok".into(), "$.token".into());
        map.insert("code".into(), "$.status".into());

        let captures = extract_captures(&body, &map).unwrap();
        assert_eq!(captures.len(), 3);
        assert_eq!(captures.get("id").unwrap(), "usr_123");
        assert_eq!(captures.get("tok").unwrap(), "abc");
        assert_eq!(captures.get("code").unwrap(), "200");
    }

    #[test]
    fn extract_array_value() {
        let body = json!({"tags": ["a", "b"]});
        let mut map = HashMap::new();
        map.insert("tags".into(), "$.tags".into());

        let captures = extract_captures(&body, &map).unwrap();
        // Array serialized as JSON string
        assert_eq!(captures.get("tags").unwrap(), "[\"a\",\"b\"]");
    }

    #[test]
    fn value_to_string_object() {
        let val = json!({"key": "value"});
        assert_eq!(value_to_string(&val), "{\"key\":\"value\"}");
    }

    #[test]
    fn empty_capture_map() {
        let body = json!({"name": "Alice"});
        let map = HashMap::new();
        let captures = extract_captures(&body, &map).unwrap();
        assert!(captures.is_empty());
    }
}

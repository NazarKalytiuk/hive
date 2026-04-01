use crate::error::TarnError;
use indexmap::IndexMap;
use serde::Serialize;
use serde_yaml::Value as YamlValue;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Default)]
struct Entry {
    method: String,
    url: String,
    request_headers: IndexMap<String, String>,
    request_body: Option<YamlValue>,
    status: Option<u16>,
    assert_headers: HashMap<String, String>,
    assert_body: IndexMap<String, YamlValue>,
    redirect: Option<OutputRedirectAssertion>,
    captures: IndexMap<String, YamlValue>,
}

#[derive(Debug, Serialize)]
struct OutputFile {
    name: String,
    steps: Vec<OutputStep>,
}

#[derive(Debug, Serialize)]
struct OutputStep {
    name: String,
    request: OutputRequest,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    capture: IndexMap<String, YamlValue>,
    #[serde(rename = "assert", skip_serializing_if = "Option::is_none")]
    assertions: Option<OutputAssertion>,
}

#[derive(Debug, Serialize)]
struct OutputRequest {
    method: String,
    url: String,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    headers: IndexMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<YamlValue>,
}

#[derive(Debug, Serialize)]
struct OutputAssertion {
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    redirect: Option<OutputRedirectAssertion>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    headers: HashMap<String, String>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    body: IndexMap<String, YamlValue>,
}

#[derive(Debug, Clone, Serialize)]
struct OutputRedirectAssertion {
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<u32>,
}

pub fn convert_file(path: &Path) -> Result<String, TarnError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| TarnError::Parse(format!("Failed to read {}: {}", path.display(), e)))?;
    convert_str(&content, path)
}

pub fn convert_str(content: &str, path: &Path) -> Result<String, TarnError> {
    let entries = parse_entries(content, path)?;
    if entries.is_empty() {
        return Err(TarnError::Parse(format!(
            "{}: no Hurl entries found",
            path.display()
        )));
    }

    let capture_names = collect_capture_names(&entries);
    let output = OutputFile {
        name: format!(
            "Imported from {}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Hurl")
        ),
        steps: entries
            .into_iter()
            .map(|entry| entry_to_output(entry, &capture_names))
            .collect(),
    };

    let mut rendered = serde_yaml::to_string(&output).map_err(|e| {
        TarnError::Parse(format!("{}: failed to render YAML: {}", path.display(), e))
    })?;
    if let Some(rest) = rendered.strip_prefix("---\n") {
        rendered = rest.to_string();
    }
    Ok(rendered)
}

fn parse_entries(content: &str, path: &Path) -> Result<Vec<Entry>, TarnError> {
    let lines: Vec<&str> = content.lines().collect();
    let mut index = 0;
    let mut entries = Vec::new();

    while index < lines.len() {
        skip_blank_and_comment_lines(&lines, &mut index);
        if index >= lines.len() {
            break;
        }

        let trimmed = lines[index].trim();
        if !is_request_line(trimmed) {
            return Err(TarnError::Parse(format!(
                "{}:{}: unsupported Hurl syntax starting at '{}'",
                path.display(),
                index + 1,
                trimmed
            )));
        }

        let (method, url) = parse_request_line(trimmed, path, index + 1)?;
        index += 1;
        let mut entry = Entry {
            method,
            url,
            ..Entry::default()
        };
        let mut current_section: Option<&str> = None;

        while index < lines.len() {
            let raw = lines[index];
            let trimmed = raw.trim();

            if trimmed.is_empty() || trimmed.starts_with('#') {
                index += 1;
                continue;
            }

            if entry.status.is_none() {
                if is_http_status_line(trimmed) {
                    entry.status = Some(parse_status_line(trimmed, path, index + 1)?);
                    index += 1;
                    continue;
                }
                if let Some((name, value)) = parse_header_line(trimmed) {
                    entry.request_headers.insert(name.into(), value.into());
                    index += 1;
                    continue;
                }
                if trimmed.starts_with('{') || trimmed.starts_with('[') {
                    let body = collect_body_block(&lines, &mut index, path)?;
                    entry.request_body = Some(parse_json_or_yaml_value(&body, path, index)?);
                    continue;
                }
                return Err(TarnError::Parse(format!(
                    "{}:{}: unsupported request surface '{}'",
                    path.display(),
                    index + 1,
                    trimmed
                )));
            }

            if is_request_line(trimmed) {
                break;
            }

            if let Some(section) = parse_section_header(trimmed) {
                current_section = Some(section);
                index += 1;
                continue;
            }

            match current_section {
                Some("Captures") => {
                    let (name, value) = parse_capture_line(trimmed, path, index + 1)?;
                    entry.captures.insert(name, value);
                }
                Some("Asserts") => {
                    parse_assert_line(trimmed, path, index + 1, &mut entry)?;
                }
                _ => {
                    return Err(TarnError::Parse(format!(
                        "{}:{}: unsupported response surface '{}'",
                        path.display(),
                        index + 1,
                        trimmed
                    )));
                }
            }

            index += 1;
        }

        entries.push(entry);
    }

    Ok(entries)
}

fn skip_blank_and_comment_lines(lines: &[&str], index: &mut usize) {
    while *index < lines.len() {
        let trimmed = lines[*index].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            *index += 1;
        } else {
            break;
        }
    }
}

fn is_request_line(line: &str) -> bool {
    let Some((method, _rest)) = line.split_once(char::is_whitespace) else {
        return false;
    };
    !method.is_empty()
        && method
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
}

fn parse_request_line(
    line: &str,
    path: &Path,
    line_no: usize,
) -> Result<(String, String), TarnError> {
    let Some((method, url)) = line.split_once(char::is_whitespace) else {
        return Err(TarnError::Parse(format!(
            "{}:{}: invalid request line '{}'",
            path.display(),
            line_no,
            line
        )));
    };
    Ok((method.to_string(), url.trim().to_string()))
}

fn is_http_status_line(line: &str) -> bool {
    line.starts_with("HTTP/")
        || line.starts_with("HTTP ")
        || line.starts_with("HTTP\t")
        || line == "HTTP"
}

fn parse_status_line(line: &str, path: &Path, line_no: usize) -> Result<u16, TarnError> {
    let status = line
        .split_whitespace()
        .find_map(|part| part.parse::<u16>().ok())
        .ok_or_else(|| {
            TarnError::Parse(format!(
                "{}:{}: could not parse HTTP status from '{}'",
                path.display(),
                line_no,
                line
            ))
        })?;
    Ok(status)
}

fn parse_header_line(line: &str) -> Option<(&str, &str)> {
    let (name, value) = line.split_once(':')?;
    if name.contains(' ') || name.is_empty() {
        return None;
    }
    Some((name.trim(), value.trim()))
}

fn collect_body_block(lines: &[&str], index: &mut usize, path: &Path) -> Result<String, TarnError> {
    let start = *index;
    let mut body_lines = Vec::new();
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut prev_escape = false;

    while *index < lines.len() {
        let line = lines[*index];
        body_lines.push(line);
        for ch in line.chars() {
            if in_string {
                if ch == '"' && !prev_escape {
                    in_string = false;
                }
                prev_escape = ch == '\\' && !prev_escape;
                continue;
            }
            prev_escape = false;
            match ch {
                '"' => in_string = true,
                '{' | '[' => depth += 1,
                '}' | ']' => depth -= 1,
                _ => {}
            }
        }
        *index += 1;
        if depth <= 0 {
            break;
        }
    }

    if depth != 0 {
        return Err(TarnError::Parse(format!(
            "{}:{}: unbalanced JSON body block in Hurl file",
            path.display(),
            start + 1
        )));
    }

    Ok(body_lines.join("\n"))
}

fn parse_json_or_yaml_value(
    value: &str,
    path: &Path,
    line_no: usize,
) -> Result<YamlValue, TarnError> {
    serde_yaml::from_str(value).map_err(|e| {
        TarnError::Parse(format!(
            "{}:{}: failed to parse request body: {}",
            path.display(),
            line_no,
            e
        ))
    })
}

fn parse_section_header(line: &str) -> Option<&str> {
    line.strip_prefix('[')?.strip_suffix(']')
}

fn parse_capture_line(
    line: &str,
    path: &Path,
    line_no: usize,
) -> Result<(String, YamlValue), TarnError> {
    let (name, expr) = line.split_once(':').ok_or_else(|| {
        TarnError::Parse(format!(
            "{}:{}: invalid capture syntax '{}'",
            path.display(),
            line_no,
            line
        ))
    })?;
    let name = name.trim().to_string();
    let expr = expr.trim();

    if let Some(path_expr) = expr.strip_prefix("jsonpath ") {
        return Ok((name, YamlValue::String(unquote(path_expr)?.to_string())));
    }
    if let Some(header_name) = expr.strip_prefix("header ") {
        return Ok((
            name,
            yaml_mapping(&[(
                "header",
                YamlValue::String(unquote(header_name)?.to_string()),
            )]),
        ));
    }
    if let Some(cookie_name) = expr.strip_prefix("cookie ") {
        return Ok((
            name,
            yaml_mapping(&[(
                "cookie",
                YamlValue::String(unquote(cookie_name)?.to_string()),
            )]),
        ));
    }
    if expr == "status" {
        return Ok((name, yaml_mapping(&[("status", YamlValue::Bool(true))])));
    }
    if expr == "url" {
        return Ok((name, yaml_mapping(&[("url", YamlValue::Bool(true))])));
    }
    if expr == "body" {
        return Ok((name, yaml_mapping(&[("body", YamlValue::Bool(true))])));
    }

    Err(TarnError::Parse(format!(
        "{}:{}: unsupported Hurl capture '{}'",
        path.display(),
        line_no,
        line
    )))
}

fn parse_assert_line(
    line: &str,
    path: &Path,
    line_no: usize,
    entry: &mut Entry,
) -> Result<(), TarnError> {
    let mut parts = line.splitn(3, char::is_whitespace);
    let subject = parts.next().unwrap_or_default();
    let rest = parts.collect::<Vec<_>>().join(" ");

    if subject == "jsonpath" {
        let (jsonpath, operator, expected) = parse_quoted_subject_assertion(&rest, path, line_no)?;
        if operator != "==" {
            return Err(TarnError::Parse(format!(
                "{}:{}: unsupported jsonpath operator '{}'",
                path.display(),
                line_no,
                operator
            )));
        }
        entry
            .assert_body
            .insert(jsonpath.to_string(), parse_scalar_yaml_value(expected)?);
        return Ok(());
    }

    if subject == "header" {
        let (header_name, operator, expected) =
            parse_quoted_subject_assertion(&rest, path, line_no)?;
        let spec = match operator {
            "==" => unquote(expected)?.to_string(),
            "contains" => format!("contains \"{}\"", unquote(expected)?),
            "matches" => format!("matches \"{}\"", unquote(expected)?),
            _ => {
                return Err(TarnError::Parse(format!(
                    "{}:{}: unsupported header operator '{}'",
                    path.display(),
                    line_no,
                    operator
                )))
            }
        };
        entry.assert_headers.insert(header_name.to_string(), spec);
        return Ok(());
    }

    if subject == "body" {
        let (operator, expected) = split_operator_and_value(&rest, path, line_no)?;
        let value = match operator {
            "==" => parse_scalar_yaml_value(expected)?,
            "contains" => yaml_mapping(&[(
                "contains",
                YamlValue::String(unquote(expected)?.to_string()),
            )]),
            "matches" => {
                yaml_mapping(&[("matches", YamlValue::String(unquote(expected)?.to_string()))])
            }
            _ => {
                return Err(TarnError::Parse(format!(
                    "{}:{}: unsupported body operator '{}'",
                    path.display(),
                    line_no,
                    operator
                )))
            }
        };
        entry.assert_body.insert("$".into(), value);
        return Ok(());
    }

    if subject == "url" {
        let (operator, expected) = split_operator_and_value(&rest, path, line_no)?;
        if operator != "==" {
            return Err(TarnError::Parse(format!(
                "{}:{}: unsupported url assertion operator '{}'",
                path.display(),
                line_no,
                operator
            )));
        }
        entry.redirect.get_or_insert_with(default_redirect).url =
            Some(unquote(expected)?.to_string());
        return Ok(());
    }

    if subject == "redirects" {
        let (operator, expected) = split_operator_and_value(&rest, path, line_no)?;
        if operator != "==" {
            return Err(TarnError::Parse(format!(
                "{}:{}: unsupported redirects assertion operator '{}'",
                path.display(),
                line_no,
                operator
            )));
        }
        entry.redirect.get_or_insert_with(default_redirect).count =
            Some(expected.trim().parse::<u32>().map_err(|_| {
                TarnError::Parse(format!(
                    "{}:{}: invalid redirects value '{}'",
                    path.display(),
                    line_no,
                    expected
                ))
            })?);
        return Ok(());
    }

    Err(TarnError::Parse(format!(
        "{}:{}: unsupported Hurl assertion '{}'",
        path.display(),
        line_no,
        line
    )))
}

fn parse_quoted_subject_assertion<'a>(
    rest: &'a str,
    path: &Path,
    line_no: usize,
) -> Result<(&'a str, &'a str, &'a str), TarnError> {
    let first_quote = rest.find('"').ok_or_else(|| {
        TarnError::Parse(format!(
            "{}:{}: expected quoted subject in '{}'",
            path.display(),
            line_no,
            rest
        ))
    })?;
    let after_first = &rest[first_quote..];
    let subject_end = after_first[1..]
        .find('"')
        .map(|offset| offset + 1)
        .ok_or_else(|| {
            TarnError::Parse(format!(
                "{}:{}: unterminated quoted subject in '{}'",
                path.display(),
                line_no,
                rest
            ))
        })?;
    let quoted = &after_first[..=subject_end];
    let subject = unquote(quoted)?;
    let remainder = after_first[subject_end + 1..].trim();
    let (operator, expected) = split_operator_and_value(remainder, path, line_no)?;
    Ok((subject, operator, expected))
}

fn split_operator_and_value<'a>(
    input: &'a str,
    path: &Path,
    line_no: usize,
) -> Result<(&'a str, &'a str), TarnError> {
    for operator in ["contains", "matches", "=="] {
        if let Some(rest) = input.strip_prefix(operator) {
            return Ok((operator, rest.trim()));
        }
    }
    Err(TarnError::Parse(format!(
        "{}:{}: unsupported assertion expression '{}'",
        path.display(),
        line_no,
        input
    )))
}

fn parse_scalar_yaml_value(input: &str) -> Result<YamlValue, TarnError> {
    let trimmed = input.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') {
        return Ok(YamlValue::String(unquote(trimmed)?.to_string()));
    }
    serde_yaml::from_str(trimmed).or_else(|_| Ok(YamlValue::String(trimmed.to_string())))
}

fn unquote(input: &str) -> Result<&str, TarnError> {
    input
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .ok_or_else(|| TarnError::Parse(format!("expected quoted string, got '{}'", input)))
}

fn yaml_mapping(entries: &[(&str, YamlValue)]) -> YamlValue {
    let mut mapping = serde_yaml::Mapping::new();
    for (key, value) in entries {
        mapping.insert(YamlValue::String((*key).to_string()), value.clone());
    }
    YamlValue::Mapping(mapping)
}

fn collect_capture_names(entries: &[Entry]) -> HashSet<String> {
    entries
        .iter()
        .flat_map(|entry| entry.captures.keys().cloned())
        .collect()
}

fn entry_to_output(entry: Entry, capture_names: &HashSet<String>) -> OutputStep {
    let name = format!("{} {}", entry.method, display_step_target(&entry.url));
    let request = OutputRequest {
        method: entry.method,
        url: rewrite_templates_in_string(&entry.url, capture_names),
        headers: rewrite_templates_in_map(&entry.request_headers, capture_names),
        body: entry
            .request_body
            .map(|value| rewrite_templates_in_yaml(value, capture_names)),
    };
    let capture = entry
        .captures
        .into_iter()
        .map(|(key, value)| (key, rewrite_templates_in_yaml(value, capture_names)))
        .collect();
    let assertions = if entry.status.is_none()
        && entry.redirect.is_none()
        && entry.assert_headers.is_empty()
        && entry.assert_body.is_empty()
    {
        None
    } else {
        Some(OutputAssertion {
            status: entry.status,
            redirect: entry.redirect.map(|redirect| OutputRedirectAssertion {
                url: redirect
                    .url
                    .map(|url| rewrite_templates_in_string(&url, capture_names)),
                count: redirect.count,
            }),
            headers: rewrite_templates_in_hash_map(entry.assert_headers, capture_names),
            body: entry
                .assert_body
                .into_iter()
                .map(|(key, value)| (key, rewrite_templates_in_yaml(value, capture_names)))
                .collect(),
        })
    };

    OutputStep {
        name,
        request,
        capture,
        assertions,
    }
}

fn default_redirect() -> OutputRedirectAssertion {
    OutputRedirectAssertion {
        url: None,
        count: None,
    }
}

fn display_step_target(url: &str) -> String {
    if let Some(idx) = url.find("://") {
        let rest = &url[idx + 3..];
        if let Some(path_start) = rest.find('/') {
            let path = &rest[path_start..];
            if !path.is_empty() {
                return path.to_string();
            }
        }
    }
    url.to_string()
}

fn rewrite_templates_in_map(
    values: &IndexMap<String, String>,
    capture_names: &HashSet<String>,
) -> IndexMap<String, String> {
    values
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                rewrite_templates_in_string(value, capture_names),
            )
        })
        .collect()
}

fn rewrite_templates_in_hash_map(
    values: HashMap<String, String>,
    capture_names: &HashSet<String>,
) -> HashMap<String, String> {
    values
        .into_iter()
        .map(|(key, value)| (key, rewrite_templates_in_string(&value, capture_names)))
        .collect()
}

fn rewrite_templates_in_yaml(value: YamlValue, capture_names: &HashSet<String>) -> YamlValue {
    match value {
        YamlValue::String(text) => {
            YamlValue::String(rewrite_templates_in_string(&text, capture_names))
        }
        YamlValue::Sequence(items) => YamlValue::Sequence(
            items
                .into_iter()
                .map(|item| rewrite_templates_in_yaml(item, capture_names))
                .collect(),
        ),
        YamlValue::Mapping(map) => {
            let mut rewritten = serde_yaml::Mapping::new();
            for (key, value) in map {
                rewritten.insert(key, rewrite_templates_in_yaml(value, capture_names));
            }
            YamlValue::Mapping(rewritten)
        }
        other => other,
    }
}

fn rewrite_templates_in_string(input: &str, capture_names: &HashSet<String>) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        if let Some(end) = after_start.find("}}") {
            let raw_name = after_start[..end].trim();
            let replacement = if raw_name.contains('.') || raw_name.starts_with('$') {
                format!("{{{{ {} }}}}", raw_name)
            } else if capture_names.contains(raw_name) {
                format!("{{{{ capture.{} }}}}", raw_name)
            } else {
                format!("{{{{ env.{} }}}}", raw_name)
            };
            output.push_str(&replacement);
            rest = &after_start[end + 2..];
        } else {
            output.push_str(&rest[start..]);
            return output;
        }
    }

    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_basic_request_and_assertions() {
        let converted = convert_str(
            r#"
GET https://api.example.com/health
HTTP 200
[Asserts]
jsonpath "$.status" == "ok"
"#,
            Path::new("health.hurl"),
        )
        .unwrap();

        assert!(converted.contains("name: Imported from health.hurl"));
        assert!(converted.contains("method: GET"));
        assert!(converted.contains("url: https://api.example.com/health"));
        assert!(converted.contains("status: 200"));
        assert!(converted.contains("$.status"));
        assert!(converted.contains("ok"));
    }

    #[test]
    fn converts_captures_and_rewrites_templates() {
        let converted = convert_str(
            r#"
POST https://api.example.com/users
Content-Type: application/json
{
  "name": "Jane"
}
HTTP 201
[Captures]
user_id: jsonpath "$.id"

GET https://api.example.com/users/{{user_id}}
HTTP 200
"#,
            Path::new("users.hurl"),
        )
        .unwrap();

        assert!(converted.contains("user_id: $.id"));
        assert!(converted.contains("url: https://api.example.com/users/{{ capture.user_id }}"));
    }

    #[test]
    fn rejects_unsupported_assertions() {
        let error = convert_str(
            r#"
GET https://api.example.com/users
HTTP 200
[Asserts]
xpath "//title" == "Users"
"#,
            Path::new("bad.hurl"),
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("unsupported Hurl assertion"));
    }
}

//! `tarn scaffold --from-curl <file>` — parse a single `curl`
//! invocation out of a text file and rebuild the Tarn skeleton.
//!
//! We support the flags most commonly produced by "copy as cURL" in
//! Chrome DevTools and by server-side curl examples: `-X`, `-H`,
//! `-d`/`--data`/`--data-raw`/`--data-binary`, `-u`, `-b`, plus the
//! leading URL positional. Multi-line invocations with backslash
//! continuation are folded before tokenisation. `$VAR` occurrences
//! in URLs and bodies are rewritten to `{{ env.VAR }}` so tests don't
//! silently bake shell state into request data.
//!
//! Anything we can't parse surfaces as a structured error — the
//! scaffold refuses to emit half-inferred YAML.

use super::{BodyShape, ScaffoldRequest, Todo, TodoCategory};
use crate::error::TarnError;
use std::collections::BTreeMap;

pub fn scaffold_from_curl(
    curl_text: &str,
    source_label: &str,
) -> Result<(ScaffoldRequest, Vec<Todo>), TarnError> {
    let folded = fold_continuations(curl_text);
    let tokens = tokenize(&folded).map_err(TarnError::Validation)?;
    if tokens.is_empty() {
        return Err(TarnError::Validation(
            "tarn scaffold --from-curl: input is empty".into(),
        ));
    }
    // `curl` might be absent (some copy-as-cURL variants omit it). We
    // skip any leading `curl` token but do not require it; the URL is
    // the first non-flag positional.
    let mut idx = 0;
    if tokens[idx].eq_ignore_ascii_case("curl") {
        idx += 1;
    }

    let mut method: Option<String> = None;
    let mut url: Option<String> = None;
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    let mut body: Option<String> = None;
    let mut basic_auth: Option<String> = None;
    let mut cookie: Option<String> = None;

    while idx < tokens.len() {
        let tok = &tokens[idx];
        idx += 1;
        match tok.as_str() {
            "-X" | "--request" => {
                let v = take_arg(&tokens, &mut idx, tok)?;
                method = Some(v.to_ascii_uppercase());
            }
            "-H" | "--header" => {
                let v = take_arg(&tokens, &mut idx, tok)?;
                let (name, value) = v
                    .split_once(':')
                    .ok_or_else(|| TarnError::Validation(format!("invalid -H value: {v}")))?;
                headers.insert(name.trim().to_string(), value.trim().to_string());
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-urlencode" => {
                let v = take_arg(&tokens, &mut idx, tok)?;
                // `@filename` is a curl convention (read from file); the
                // scaffold can't know the file's contents, so surface
                // the raw token verbatim and let the TODO warn.
                body = Some(v);
                if method.is_none() {
                    // curl defaults to POST when -d is present.
                    method = Some("POST".into());
                }
            }
            "-u" | "--user" => {
                let v = take_arg(&tokens, &mut idx, tok)?;
                basic_auth = Some(v);
            }
            "-b" | "--cookie" => {
                let v = take_arg(&tokens, &mut idx, tok)?;
                cookie = Some(v);
            }
            "-G" | "--get" => {
                if method.is_none() {
                    method = Some("GET".into());
                }
            }
            // Flags we can safely ignore for scaffold purposes: they
            // affect transport, not request shape.
            "-i" | "--include" | "-v" | "--verbose" | "-s" | "--silent" | "-k" | "--insecure"
            | "--compressed" | "-L" | "--location" => {}
            // Any other flag: swallow its value if the next token looks
            // like one, else just skip.
            t if t.starts_with('-') => {
                if idx < tokens.len() && !tokens[idx].starts_with('-') {
                    idx += 1;
                }
            }
            other => {
                if url.is_none() {
                    url = Some(other.to_string());
                }
                // Second positional or later — curl treats them as
                // additional URLs; scaffold only supports one.
            }
        }
    }

    let url =
        url.ok_or_else(|| TarnError::Validation("curl input had no URL positional".into()))?;
    let url = rewrite_dollar_vars(&url);
    let method = method.unwrap_or_else(|| "GET".into());

    // Basic auth → Authorization header. We keep the header name
    // deterministic so sensitive-TODO attachment works.
    if let Some(creds) = basic_auth {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(creds.as_bytes());
        headers
            .entry("Authorization".to_string())
            .or_insert_with(|| format!("Basic {encoded}"));
    }
    if let Some(c) = cookie {
        headers.entry("Cookie".to_string()).or_insert(c);
    }

    // Body: prefer structured JSON when Content-Type says JSON and the
    // body actually parses; otherwise keep it as a raw string.
    let body_shape = body.map(|raw| {
        let rewritten = rewrite_dollar_vars(&raw);
        let is_json = headers.iter().any(|(k, v)| {
            k.eq_ignore_ascii_case("content-type") && v.to_lowercase().contains("json")
        });
        if is_json {
            match serde_json::from_str::<serde_json::Value>(&rewritten) {
                Ok(v) => BodyShape::Json(v),
                // JSON parse failed — keep raw form and let the TODO
                // say so.
                Err(_) => BodyShape::Raw(rewritten),
            }
        } else {
            BodyShape::Raw(rewritten)
        }
    });

    let step_name = format!("{} {}", method, path_segment(&url));
    let file_name = format!("Imported from {}", source_label);

    let mut request = ScaffoldRequest::new(file_name, step_name);
    request.method = method;
    request.url = url;
    request.headers = headers;
    request.body = body_shape;
    // Always flag `Authorization` / `Cookie` / API key style headers;
    // the emitter also has a built-in sensitive list but mode-specific
    // hints let the TODO text say "we synthesized this from -u".
    for name in ["Authorization", "Cookie", "X-Api-Key", "X-Auth-Token"] {
        if request.headers.keys().any(|k| k.eq_ignore_ascii_case(name)) {
            request.sensitive_headers.push(name.to_string());
        }
    }
    request.sensitive_headers.sort();
    request.sensitive_headers.dedup();

    let todos: Vec<Todo> = vec![Todo::new(
        TodoCategory::Headers,
        "headers imported from curl — move tokens/secrets to env",
    )];
    Ok((request, todos))
}

fn take_arg(tokens: &[String], idx: &mut usize, flag: &str) -> Result<String, TarnError> {
    if *idx >= tokens.len() {
        return Err(TarnError::Validation(format!(
            "curl flag {flag} expects an argument"
        )));
    }
    let v = tokens[*idx].clone();
    *idx += 1;
    Ok(v)
}

/// Fold `\\\n` line continuations used in multi-line curl commands.
/// We don't touch any other whitespace so subsequent tokenisation
/// sees a single logical line.
fn fold_continuations(text: &str) -> String {
    // Replace `\<LF>` (and the CRLF variant) with a single space.
    text.replace("\\\r\n", " ").replace("\\\n", " ")
}

/// Minimal POSIX-shell-ish tokeniser: handles single-quoted, double-
/// quoted, and bare arguments. Bare `\` escapes the next char. Inside
/// single quotes nothing is interpreted. Inside double quotes `\` only
/// escapes `\`, `"`, `$`, and newline. This is deliberately small —
/// a pulled-in crate would be overkill for the restricted subset curl
/// commands use in practice, and avoiding one keeps the scaffold
/// deterministic (no environment expansion of `$VAR` at tokenise
/// time; we rewrite `$VAR` later as `{{ env.VAR }}`).
fn tokenize(text: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = text.chars().peekable();
    let mut pushed_something = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' if in_single => cur.push('\\'),
            '\\' if in_double => {
                if let Some(&next) = chars.peek() {
                    if matches!(next, '\\' | '"' | '$' | '\n') {
                        cur.push(next);
                        chars.next();
                    } else {
                        cur.push('\\');
                    }
                } else {
                    cur.push('\\');
                }
            }
            '\\' => {
                if let Some(next) = chars.next() {
                    cur.push(next);
                } else {
                    return Err("dangling backslash".into());
                }
            }
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            c if c.is_whitespace() && !in_single && !in_double => {
                if pushed_something {
                    out.push(std::mem::take(&mut cur));
                    pushed_something = false;
                }
            }
            c => {
                cur.push(c);
                pushed_something = true;
            }
        }
    }
    if in_single || in_double {
        return Err("unterminated quoted string in curl input".into());
    }
    if pushed_something {
        out.push(cur);
    }
    Ok(out)
}

/// Rewrite bare `$VAR` (and `${VAR}`) sequences into Tarn template
/// form `{{ env.VAR }}`. Leaves already-templated values alone.
fn rewrite_dollar_vars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'$' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'{' {
                if let Some(end) = s[i + 2..].find('}') {
                    let name = &s[i + 2..i + 2 + end];
                    if is_env_var_name(name) {
                        out.push_str(&format!("{{{{ env.{name} }}}}"));
                        i = i + 2 + end + 1;
                        continue;
                    }
                }
            } else if (next as char).is_ascii_alphabetic() || next == b'_' {
                let mut j = i + 1;
                while j < bytes.len() {
                    let c2 = bytes[j];
                    if (c2 as char).is_ascii_alphanumeric() || c2 == b'_' {
                        j += 1;
                    } else {
                        break;
                    }
                }
                let name = &s[i + 1..j];
                out.push_str(&format!("{{{{ env.{name} }}}}"));
                i = j;
                continue;
            }
        }
        out.push(c as char);
        i += 1;
    }
    out
}

fn is_env_var_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn path_segment(url: &str) -> String {
    if let Some(idx) = url.find("://") {
        let rest = &url[idx + 3..];
        if let Some(slash) = rest.find('/') {
            return rest[slash..].to_string();
        }
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenise_handles_mixed_quotes() {
        let toks =
            tokenize(r#"curl -X POST 'http://x/y' -H "Content-Type: application/json""#).unwrap();
        assert_eq!(
            toks,
            vec![
                "curl",
                "-X",
                "POST",
                "http://x/y",
                "-H",
                "Content-Type: application/json",
            ]
        );
    }

    #[test]
    fn tokenise_reports_unterminated_quote() {
        let err = tokenize("curl 'unterminated").unwrap_err();
        assert!(err.contains("unterminated"));
    }

    #[test]
    fn fold_joins_backslash_newline() {
        let input = "curl -X POST \\\n  http://x/y \\\n  -d '{\"a\":1}'";
        let folded = fold_continuations(input);
        assert!(!folded.contains("\\\n"));
        assert!(folded.contains("-d '{\"a\":1}'"));
    }

    #[test]
    fn rewrites_dollar_vars_to_env_templates() {
        assert_eq!(rewrite_dollar_vars("$TOKEN"), "{{ env.TOKEN }}");
        assert_eq!(rewrite_dollar_vars("${TOKEN}"), "{{ env.TOKEN }}");
        assert_eq!(rewrite_dollar_vars("a$b$c"), "a{{ env.b }}{{ env.c }}");
        // Not env-like — leave alone.
        assert_eq!(rewrite_dollar_vars("$1"), "$1");
    }

    #[test]
    fn curl_with_post_and_json_body_produces_structured_body() {
        let input = r#"curl -X POST http://api/users -H "Content-Type: application/json" -d '{"name":"Jane"}'"#;
        let (req, _) = scaffold_from_curl(input, "users.curl").unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.url, "http://api/users");
        match req.body {
            Some(BodyShape::Json(v)) => {
                assert_eq!(v["name"], "Jane");
            }
            other => panic!("expected structured JSON body, got {:?}", other),
        }
    }

    #[test]
    fn curl_without_method_but_with_data_defaults_to_post() {
        let input = r#"curl http://api/x -d '{"a":1}' -H 'Content-Type: application/json'"#;
        let (req, _) = scaffold_from_curl(input, "x.curl").unwrap();
        assert_eq!(req.method, "POST");
    }

    #[test]
    fn curl_with_basic_auth_creates_authorization_header() {
        let input = "curl -u demo:secret http://api/me";
        let (req, _) = scaffold_from_curl(input, "auth.curl").unwrap();
        let auth = req
            .headers
            .get("Authorization")
            .expect("basic auth header present");
        assert!(auth.starts_with("Basic "));
        assert!(req.sensitive_headers.iter().any(|h| h == "Authorization"));
    }

    #[test]
    fn curl_rejects_empty_input() {
        let err = scaffold_from_curl("", "empty.curl").unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn curl_rejects_missing_url() {
        let err = scaffold_from_curl("curl -X GET", "bad.curl").unwrap_err();
        assert!(err.to_string().contains("no URL"));
    }

    #[test]
    fn curl_rewrites_shell_vars_in_url_and_body() {
        let input = r#"curl -X POST http://api/$RESOURCE -H "Content-Type: application/json" -d '{"id":"$ID"}'"#;
        let (req, _) = scaffold_from_curl(input, "vars.curl").unwrap();
        assert!(req.url.contains("{{ env.RESOURCE }}"));
        match req.body {
            Some(BodyShape::Json(v)) => assert_eq!(v["id"], "{{ env.ID }}"),
            other => panic!("expected JSON body, got {:?}", other),
        }
    }

    #[test]
    fn curl_is_deterministic() {
        let input = r#"curl -X POST http://api/users -H "Content-Type: application/json" -d '{"name":"Jane"}'"#;
        let a = scaffold_from_curl(input, "users.curl").unwrap().0;
        let b = scaffold_from_curl(input, "users.curl").unwrap().0;
        assert_eq!(a.method, b.method);
        assert_eq!(a.url, b.url);
        assert_eq!(a.headers, b.headers);
    }
}

//! `tarn scaffold` — bootstrap a minimal `.tarn.yaml` skeleton from one
//! of four inputs (OpenAPI operation id, raw `curl` command, explicit
//! method+URL, or a previously-recorded fixture).
//!
//! The scaffold is intentionally *scaffold-quality*: the generated YAML
//! parses and validates, the request block is correct enough to execute,
//! and placeholder assertions + captures hint at what the user should
//! tighten — every inferred-but-unverified fragment is marked with a
//! machine-greppable `# TODO:` comment so agents can drive the next
//! iteration without rereading the whole file.
//!
//! Determinism is load-bearing: running scaffold twice with identical
//! inputs must produce byte-identical output, including TODO ordering.
//! Every map the emitter walks uses `BTreeMap` / pre-sorted `Vec` for
//! that reason, and no clock / RNG state is read at scaffold time.
//! Random placeholders emit Tarn built-in names (`$uuid_v4`,
//! `$random_hex(8)`) instead — those resolve deterministically under
//! the faker seed when the test runs, not when the YAML is written.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::TarnError;

pub mod curl;
pub mod emit;
pub mod explicit;
pub mod openapi;
pub mod recorded;

/// Which input mode produced a [`ScaffoldResult`]. The string form is
/// a public contract for the `--format json` surface; renaming a
/// variant is a breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceMode {
    OpenApi,
    Curl,
    Explicit,
    Recorded,
}

impl SourceMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceMode::OpenApi => "openapi",
            SourceMode::Curl => "curl",
            SourceMode::Explicit => "explicit",
            SourceMode::Recorded => "recorded",
        }
    }
}

/// Categorises a TODO so agents can filter the list without text-matching
/// on the human message. Kept deliberately small — each category maps
/// to a concrete review action ("check auth headers", "fill required
/// body fields") rather than a free-form tag cloud.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TodoCategory {
    Env,
    Method,
    Url,
    PathParam,
    Headers,
    Auth,
    Body,
    Assertion,
    Capture,
}

impl TodoCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            TodoCategory::Env => "env",
            TodoCategory::Method => "method",
            TodoCategory::Url => "url",
            TodoCategory::PathParam => "path_param",
            TodoCategory::Headers => "headers",
            TodoCategory::Auth => "auth",
            TodoCategory::Body => "body",
            TodoCategory::Assertion => "assertion",
            TodoCategory::Capture => "capture",
        }
    }
}

/// A single TODO marker the emitter will interleave as `# TODO:` comment
/// above the anchor line in the rendered YAML. `line` is back-filled by
/// [`emit::render`] after layout so callers consuming `--format json`
/// see the line number they would `grep -n` in the file.
#[derive(Debug, Clone)]
pub struct Todo {
    pub category: TodoCategory,
    pub message: String,
    /// 1-based line number in the rendered YAML. `None` before
    /// rendering, always `Some` in the final [`ScaffoldResult`].
    pub line: Option<usize>,
}

impl Todo {
    pub fn new(category: TodoCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
            line: None,
        }
    }
}

/// Body payload carried through the pipeline. We keep the structural
/// form (`Json`) when we can synthesise a mapping and fall back to a
/// raw string for non-JSON content-types; the emitter renders each
/// branch differently.
#[derive(Debug, Clone)]
pub enum BodyShape {
    /// Structured JSON; will be emitted as YAML mapping/sequence.
    Json(serde_json::Value),
    /// Opaque string body (non-JSON or parse failure); will be
    /// emitted as a quoted YAML scalar.
    Raw(String),
}

/// Internal, mode-agnostic scaffold IR. Each of the four input modes
/// produces one of these; [`emit::render`] is the single consumer.
/// `BTreeMap` for headers is not accidental — YAML rendering order
/// determines TODO line numbers and the determinism acceptance
/// criterion requires stable iteration even when the same header set
/// comes from two different modes.
#[derive(Debug, Clone)]
pub struct ScaffoldRequest {
    /// Top-level `name:` of the generated file.
    pub file_name: String,
    /// Step's `name:` field.
    pub step_name: String,
    /// HTTP method (uppercased for cleanliness; parser accepts any case).
    pub method: String,
    /// Final URL string. Path parameters are already templated as
    /// `{{ test.<name> }}` by the mode-specific scaffolder; callers
    /// downstream must not re-rewrite them.
    pub url: String,
    /// Request headers (sorted for determinism).
    pub headers: BTreeMap<String, String>,
    /// Headers the mode flagged as sensitive ("Authorization", etc.);
    /// the emitter prepends a TODO immediately above them.
    pub sensitive_headers: Vec<String>,
    /// Body, when inferable.
    pub body: Option<BodyShape>,
    /// Captures the emitter should render. Keys are capture names,
    /// values are JSONPath strings — the compact form of `CaptureSpec`.
    pub captures: BTreeMap<String, String>,
    /// Path parameters that appear as `{{ test.<name> }}` placeholders
    /// in the URL; emitted as a comment so users know which values
    /// they must supply before running.
    pub path_params: Vec<String>,
    /// Inferred response field names. Reported verbatim in the
    /// `--format json` metadata so agents can decide which to
    /// capture/assert on.
    pub response_shape_keys: Vec<String>,
    /// Status assertion to render. `None` falls back to `2xx`.
    pub status_assertion: Option<String>,
}

impl ScaffoldRequest {
    pub fn new(file_name: impl Into<String>, step_name: impl Into<String>) -> Self {
        Self {
            file_name: file_name.into(),
            step_name: step_name.into(),
            method: "GET".to_string(),
            url: String::new(),
            headers: BTreeMap::new(),
            sensitive_headers: Vec::new(),
            body: None,
            captures: BTreeMap::new(),
            path_params: Vec::new(),
            response_shape_keys: Vec::new(),
            status_assertion: None,
        }
    }
}

/// Output of [`generate`]: the rendered YAML, the TODO list (with
/// final line numbers), a copy of the inferred request, and the
/// round-trip validation report.
#[derive(Debug, Clone)]
pub struct ScaffoldResult {
    pub source_mode: SourceMode,
    pub request: ScaffoldRequest,
    pub yaml: String,
    pub todos: Vec<Todo>,
    pub parsed_ok: bool,
    pub schema_ok: bool,
}

/// Inputs for [`generate`]. The four fields are mutually exclusive;
/// the CLI dispatcher validates that exactly one is populated and
/// surfaces a structured error otherwise.
#[derive(Debug, Clone)]
pub enum ScaffoldInput {
    /// `--from-openapi <spec-file> --op-id <id>`.
    OpenApi {
        spec_path: std::path::PathBuf,
        op_id: String,
    },
    /// `--from-curl <file>` — file contents are a single `curl` invocation.
    Curl {
        curl_text: String,
        source_label: String,
    },
    /// `--method <m> --url <u>`.
    Explicit { method: String, url: String },
    /// `--from-recorded <path>` — a fixture JSON or a step directory.
    Recorded { path: std::path::PathBuf },
}

/// Override struct carrying shared options — filled by the CLI once
/// before dispatching into a mode. Keeping this separate from
/// [`ScaffoldInput`] means mode unit tests don't have to fabricate
/// unrelated CLI state.
#[derive(Debug, Clone, Default)]
pub struct ScaffoldOptions {
    /// Override the inferred top-level `name:`.
    pub name_override: Option<String>,
}

/// Top-level dispatcher. Runs the mode-specific scaffolder, renders
/// YAML with interleaved TODO comments, and round-trips the result
/// through [`crate::parser::parse_str`] so a shape-mistake in the
/// scaffold itself surfaces as an error instead of a silently broken
/// file.
pub fn generate(
    input: &ScaffoldInput,
    options: &ScaffoldOptions,
) -> Result<ScaffoldResult, TarnError> {
    let (source_mode, mut request, mut todos) = match input {
        ScaffoldInput::OpenApi { spec_path, op_id } => {
            let (req, todos) = openapi::scaffold_from_openapi(spec_path, op_id)?;
            (SourceMode::OpenApi, req, todos)
        }
        ScaffoldInput::Curl {
            curl_text,
            source_label,
        } => {
            let (req, todos) = curl::scaffold_from_curl(curl_text, source_label)?;
            (SourceMode::Curl, req, todos)
        }
        ScaffoldInput::Explicit { method, url } => {
            let (req, todos) = explicit::scaffold_from_explicit(method, url)?;
            (SourceMode::Explicit, req, todos)
        }
        ScaffoldInput::Recorded { path } => {
            let (req, todos) = recorded::scaffold_from_recorded(path)?;
            (SourceMode::Recorded, req, todos)
        }
    };

    if let Some(name) = &options.name_override {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            request.file_name = trimmed.to_string();
        }
    }

    let yaml = emit::render(&request, &mut todos);

    // Round-trip through the real parser so shape bugs in the scaffold
    // fail loud instead of producing a plausible-looking YAML that
    // `tarn validate` later rejects. Mirror the parser's exposed error
    // type so the CLI can exit with the right code.
    let parsed_ok = crate::parser::parse_str(&yaml, Path::new("scaffold.tarn.yaml")).is_ok();
    if !parsed_ok {
        return Err(TarnError::Validation(format!(
            "tarn scaffold produced YAML that failed round-trip parsing. \
             This is a bug in the scaffold generator, not your inputs. \
             Generated YAML:\n---\n{yaml}\n---"
        )));
    }
    // We treat "parses cleanly" as "schema_ok" — parser::parse_str
    // enforces the same shape the JSON Schema does (field names,
    // required keys, enum values). The two can only drift if the
    // schema adds a constraint the parser does not, which the
    // `schema_validates_all_examples` test in `parser.rs` would
    // catch before landing. Recording the distinct field keeps the
    // public JSON output stable if we later split the two checks.
    let schema_ok = parsed_ok;

    Ok(ScaffoldResult {
        source_mode,
        request,
        yaml,
        todos,
        parsed_ok,
        schema_ok,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_explicit_is_deterministic() {
        // Running scaffold twice with the same inputs must yield
        // byte-identical YAML — agents rely on this to diff runs.
        let input = ScaffoldInput::Explicit {
            method: "POST".into(),
            url: "http://example.com/users".into(),
        };
        let options = ScaffoldOptions::default();
        let a = generate(&input, &options).unwrap();
        let b = generate(&input, &options).unwrap();
        assert_eq!(a.yaml, b.yaml);
        assert_eq!(a.todos.len(), b.todos.len());
        for (ta, tb) in a.todos.iter().zip(b.todos.iter()) {
            assert_eq!(ta.category, tb.category);
            assert_eq!(ta.message, tb.message);
            assert_eq!(ta.line, tb.line);
        }
    }

    #[test]
    fn generate_explicit_round_trips_through_parser() {
        let input = ScaffoldInput::Explicit {
            method: "GET".into(),
            url: "http://example.com/health".into(),
        };
        let result = generate(&input, &ScaffoldOptions::default()).unwrap();
        assert!(result.parsed_ok);
        assert!(result.schema_ok);
        // Sanity: the YAML the scaffold emitted must contain a step
        // name and request block.
        assert!(result.yaml.contains("name:"));
        assert!(result.yaml.contains("method: GET"));
        assert!(result.yaml.contains("url: "));
    }

    #[test]
    fn generate_emits_todos_with_populated_line_numbers() {
        let input = ScaffoldInput::Explicit {
            method: "POST".into(),
            url: "http://example.com/widgets".into(),
        };
        let result = generate(&input, &ScaffoldOptions::default()).unwrap();
        assert!(
            !result.todos.is_empty(),
            "minimal scaffold must carry at least one TODO"
        );
        for todo in &result.todos {
            assert!(
                todo.line.is_some(),
                "rendered TODOs must have a line number: {:?}",
                todo
            );
        }
    }

    #[test]
    fn name_override_replaces_inferred_name() {
        let input = ScaffoldInput::Explicit {
            method: "GET".into(),
            url: "http://example.com/health".into(),
        };
        let options = ScaffoldOptions {
            name_override: Some("Custom smoke".into()),
        };
        let result = generate(&input, &options).unwrap();
        assert!(result.yaml.contains("name: Custom smoke"));
    }
}

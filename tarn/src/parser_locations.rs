//! Source-location extraction for Tarn test files.
//!
//! `serde_yaml` throws away line/column information once it produces a
//! `Value`, so we make a second pass over the original YAML text with
//! `yaml-rust2` (which exposes markers on every event) to collect the
//! locations of step `name:` keys and assertion operator keys.
//!
//! These locations are attached to each `Step` after deserialization so
//! the runner can propagate them into `StepResult` / `AssertionResult`
//! and downstream consumers (VS Code extension, MCP clients, CI
//! dashboards) can anchor runtime results on the exact source range
//! without re-parsing the YAML themselves.

use crate::model::Location;
use std::collections::HashMap;
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust2::scanner::Marker;

/// Locations for a single step entry in a YAML sequence.
///
/// `name` is `None` for sequence entries that are not plain step mappings
/// (e.g. `- include: ./other.tarn.yaml`), since there is no `name:` key to
/// point at.
#[derive(Debug, Clone, Default)]
pub(crate) struct StepLocations {
    /// Location of the `name:` key for this step (if any).
    pub(crate) name: Option<Location>,
    /// Locations of individual assertion operator keys, indexed by the
    /// same label `AssertionResult::assertion` uses (e.g. `"status"`,
    /// `"duration"`, `"redirect.url"`, `"header Content-Type"`,
    /// `"body $.name"`).
    pub(crate) assertions: HashMap<String, Location>,
}

/// Locations collected from a single YAML file, grouped by the section
/// the step lives in.
#[derive(Debug, Clone, Default)]
pub(crate) struct FileLocations {
    pub(crate) setup: Vec<StepLocations>,
    pub(crate) teardown: Vec<StepLocations>,
    pub(crate) flat_steps: Vec<StepLocations>,
    pub(crate) tests: HashMap<String, Vec<StepLocations>>,
}

/// Extract step-name and assertion-key locations from raw Tarn YAML.
///
/// `file` is the absolute (or canonical) path used in the resulting
/// `Location.file` field so downstream consumers can anchor on the
/// same path tarn already emits in other report fields.
///
/// Returns `None` if the YAML cannot be scanned (malformed input, etc).
/// `serde_yaml` is the source of truth for validation errors — this
/// function is strictly best-effort enrichment, never a gate.
pub(crate) fn extract(content: &str, file: &str) -> Option<FileLocations> {
    let mut sink = EventSink { events: Vec::new() };
    let mut parser = Parser::new_from_str(content);
    parser.load(&mut sink, true).ok()?;

    let mut cursor = Cursor {
        events: &sink.events,
        pos: 0,
        file,
    };
    cursor.walk_document()
}

/// Collects every `(Event, Marker)` pair the parser emits so we can walk
/// them recursively with random access. Event streams for Tarn files are
/// tiny relative to test runtimes, so the allocation overhead is
/// negligible.
struct EventSink {
    events: Vec<(Event, Marker)>,
}

impl MarkedEventReceiver for EventSink {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        self.events.push((ev, mark));
    }
}

struct Cursor<'a> {
    events: &'a [(Event, Marker)],
    pos: usize,
    file: &'a str,
}

impl<'a> Cursor<'a> {
    fn peek(&self) -> Option<&'a (Event, Marker)> {
        self.events.get(self.pos)
    }

    fn advance(&mut self) -> Option<&'a (Event, Marker)> {
        let event = self.events.get(self.pos);
        if event.is_some() {
            self.pos += 1;
        }
        event
    }

    fn location_from(&self, mark: &Marker) -> Location {
        // yaml-rust2 markers are 1-based on `line` and 0-based on `col`
        // (its own `fmt::Display` impl bumps the column by one before
        // printing). We surface 1-based values for both so the JSON
        // report matches what editors and error messages already use.
        Location {
            file: self.file.to_string(),
            line: mark.line(),
            column: mark.col() + 1,
        }
    }

    /// Walk the root of the YAML document, returning the per-file
    /// location map. Expects the current position to be at
    /// `StreamStart`.
    fn walk_document(&mut self) -> Option<FileLocations> {
        // StreamStart
        match self.advance()? {
            (Event::StreamStart, _) => {}
            _ => return None,
        }
        // DocumentStart
        match self.advance()? {
            (Event::DocumentStart, _) => {}
            _ => return None,
        }
        // Root is expected to be a mapping.
        match self.advance()? {
            (Event::MappingStart(_, _), _) => {}
            _ => return None,
        }

        let mut locations = FileLocations::default();

        loop {
            match self.peek()? {
                (Event::MappingEnd, _) => {
                    self.advance();
                    break;
                }
                _ => {
                    let key = self.read_scalar_key()?;
                    match key.as_str() {
                        "setup" => {
                            locations.setup = self.walk_step_sequence()?;
                        }
                        "teardown" => {
                            locations.teardown = self.walk_step_sequence()?;
                        }
                        "steps" => {
                            locations.flat_steps = self.walk_step_sequence()?;
                        }
                        "tests" => {
                            locations.tests = self.walk_tests_mapping()?;
                        }
                        _ => {
                            self.skip_node()?;
                        }
                    }
                }
            }
        }

        Some(locations)
    }

    /// Read a scalar node that we know is a mapping key. Returns its
    /// string form. Non-string keys abort the walk (Tarn schema never
    /// uses them at the positions we care about).
    fn read_scalar_key(&mut self) -> Option<String> {
        let (event, _) = self.advance()?;
        match event {
            Event::Scalar(value, _, _, _) => Some(value.clone()),
            _ => None,
        }
    }

    /// Read a mapping key and return it along with its marker.
    fn read_scalar_key_with_mark(&mut self) -> Option<(String, Marker)> {
        let (event, mark) = self.advance()?;
        match event {
            Event::Scalar(value, _, _, _) => Some((value.clone(), *mark)),
            _ => None,
        }
    }

    /// Skip the next value node (scalar, sequence, or mapping) in a
    /// balanced way, advancing past its closing event.
    fn skip_node(&mut self) -> Option<()> {
        let (event, _) = self.advance()?;
        match event {
            Event::Scalar(_, _, _, _) | Event::Alias(_) => Some(()),
            Event::SequenceStart(_, _) => loop {
                match self.peek()? {
                    (Event::SequenceEnd, _) => {
                        self.advance();
                        return Some(());
                    }
                    _ => {
                        self.skip_node()?;
                    }
                }
            },
            Event::MappingStart(_, _) => loop {
                match self.peek()? {
                    (Event::MappingEnd, _) => {
                        self.advance();
                        return Some(());
                    }
                    _ => {
                        // key
                        self.skip_node()?;
                        // value
                        self.skip_node()?;
                    }
                }
            },
            _ => None,
        }
    }

    /// Walk a sequence of steps, returning per-item location records.
    /// Expects the current position to be just before `SequenceStart`.
    fn walk_step_sequence(&mut self) -> Option<Vec<StepLocations>> {
        match self.advance()? {
            (Event::SequenceStart(_, _), _) => {}
            // Not actually a sequence — skip gracefully. Shouldn't
            // happen for files that passed `validate_yaml_shape`, but
            // being defensive here keeps location extraction strictly
            // best-effort.
            _ => return Some(Vec::new()),
        }

        let mut items = Vec::new();
        loop {
            match self.peek()? {
                (Event::SequenceEnd, _) => {
                    self.advance();
                    return Some(items);
                }
                (Event::MappingStart(_, _), _) => {
                    items.push(self.walk_step_mapping()?);
                }
                _ => {
                    // Unknown shape inside a step sequence — still
                    // allocate an empty slot so positional alignment
                    // with `Vec<Step>` stays correct.
                    items.push(StepLocations::default());
                    self.skip_node()?;
                }
            }
        }
    }

    /// Walk a single step mapping, recording the `name:` key location
    /// and diving into `assert:` if present. Expects position at the
    /// opening `MappingStart`.
    fn walk_step_mapping(&mut self) -> Option<StepLocations> {
        match self.advance()? {
            (Event::MappingStart(_, _), _) => {}
            _ => return Some(StepLocations::default()),
        }

        let mut locations = StepLocations::default();
        loop {
            match self.peek()? {
                (Event::MappingEnd, _) => {
                    self.advance();
                    return Some(locations);
                }
                _ => {
                    let (key, mark) = self.read_scalar_key_with_mark()?;
                    match key.as_str() {
                        "name" => {
                            locations.name = Some(self.location_from(&mark));
                            // Consume the name value.
                            self.skip_node()?;
                        }
                        "assert" => {
                            self.walk_assert_mapping(&mut locations.assertions)?;
                        }
                        _ => {
                            self.skip_node()?;
                        }
                    }
                }
            }
        }
    }

    /// Walk an `assert:` mapping and record locations for each
    /// assertion operator key. Keys mirror `AssertionResult::assertion`
    /// so the runner can look them up cheaply by label.
    fn walk_assert_mapping(&mut self, out: &mut HashMap<String, Location>) -> Option<()> {
        match self.advance()? {
            (Event::MappingStart(_, _), _) => {}
            _ => return Some(()),
        }

        loop {
            match self.peek()? {
                (Event::MappingEnd, _) => {
                    self.advance();
                    return Some(());
                }
                _ => {
                    let (key, mark) = self.read_scalar_key_with_mark()?;
                    match key.as_str() {
                        "status" => {
                            out.insert("status".to_string(), self.location_from(&mark));
                            self.skip_node()?;
                        }
                        "duration" => {
                            out.insert("duration".to_string(), self.location_from(&mark));
                            self.skip_node()?;
                        }
                        "redirect" => {
                            self.walk_redirect_assertions(&mark, out)?;
                        }
                        "headers" => {
                            self.walk_header_assertions(&mark, out)?;
                        }
                        "body" => {
                            self.walk_body_assertions(&mark, out)?;
                        }
                        _ => {
                            self.skip_node()?;
                        }
                    }
                }
            }
        }
    }

    /// Record `redirect.url` / `redirect.count` locations. Falls back to
    /// the `redirect:` key marker when the nested sub-keys aren't
    /// scalars (shouldn't normally happen).
    fn walk_redirect_assertions(
        &mut self,
        fallback: &Marker,
        out: &mut HashMap<String, Location>,
    ) -> Option<()> {
        let fallback_loc = self.location_from(fallback);
        out.insert("redirect.url".to_string(), fallback_loc.clone());
        out.insert("redirect.count".to_string(), fallback_loc);

        match self.advance()? {
            (Event::MappingStart(_, _), _) => {}
            _ => return Some(()),
        }

        loop {
            match self.peek()? {
                (Event::MappingEnd, _) => {
                    self.advance();
                    return Some(());
                }
                _ => {
                    let (key, mark) = self.read_scalar_key_with_mark()?;
                    match key.as_str() {
                        "url" => {
                            out.insert("redirect.url".to_string(), self.location_from(&mark));
                            self.skip_node()?;
                        }
                        "count" => {
                            out.insert("redirect.count".to_string(), self.location_from(&mark));
                            self.skip_node()?;
                        }
                        _ => {
                            self.skip_node()?;
                        }
                    }
                }
            }
        }
    }

    /// Record one `header <name>` location per header assertion key.
    fn walk_header_assertions(
        &mut self,
        _fallback: &Marker,
        out: &mut HashMap<String, Location>,
    ) -> Option<()> {
        match self.advance()? {
            (Event::MappingStart(_, _), _) => {}
            _ => return Some(()),
        }

        loop {
            match self.peek()? {
                (Event::MappingEnd, _) => {
                    self.advance();
                    return Some(());
                }
                _ => {
                    let (name, mark) = self.read_scalar_key_with_mark()?;
                    out.insert(format!("header {}", name), self.location_from(&mark));
                    self.skip_node()?;
                }
            }
        }
    }

    /// Record `body <path>` locations. Each JSONPath key can appear once,
    /// and an operator map under it can yield several assertions that
    /// all share the same source line.
    fn walk_body_assertions(
        &mut self,
        _fallback: &Marker,
        out: &mut HashMap<String, Location>,
    ) -> Option<()> {
        match self.advance()? {
            (Event::MappingStart(_, _), _) => {}
            _ => return Some(()),
        }

        loop {
            match self.peek()? {
                (Event::MappingEnd, _) => {
                    self.advance();
                    return Some(());
                }
                _ => {
                    let (path, mark) = self.read_scalar_key_with_mark()?;
                    out.insert(format!("body {}", path), self.location_from(&mark));
                    self.skip_node()?;
                }
            }
        }
    }

    /// Walk the `tests:` mapping, one named group per key. Each group's
    /// `steps:` sequence produces a vector of per-step locations.
    fn walk_tests_mapping(&mut self) -> Option<HashMap<String, Vec<StepLocations>>> {
        match self.advance()? {
            (Event::MappingStart(_, _), _) => {}
            _ => return Some(HashMap::new()),
        }

        let mut groups = HashMap::new();
        loop {
            match self.peek()? {
                (Event::MappingEnd, _) => {
                    self.advance();
                    return Some(groups);
                }
                _ => {
                    let name = self.read_scalar_key()?;
                    let group_steps = self.walk_test_group_mapping()?;
                    groups.insert(name, group_steps);
                }
            }
        }
    }

    /// Walk a single test group mapping, extracting its `steps:`
    /// sequence if present.
    fn walk_test_group_mapping(&mut self) -> Option<Vec<StepLocations>> {
        match self.advance()? {
            (Event::MappingStart(_, _), _) => {}
            _ => {
                // Primitive — no steps.
                return Some(Vec::new());
            }
        }

        let mut group_steps = Vec::new();
        loop {
            match self.peek()? {
                (Event::MappingEnd, _) => {
                    self.advance();
                    return Some(group_steps);
                }
                _ => {
                    let key = self.read_scalar_key()?;
                    if key == "steps" {
                        group_steps = self.walk_step_sequence()?;
                    } else {
                        self.skip_node()?;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_simple_flat_steps() {
        let yaml = "\
name: Simple
steps:
  - name: first
    request:
      method: GET
      url: http://localhost/
  - name: second
    request:
      method: GET
      url: http://localhost/
    assert:
      status: 200
      duration: \"< 500ms\"
";
        let locs = extract(yaml, "/abs/path/file.tarn.yaml").expect("extract");
        assert_eq!(locs.flat_steps.len(), 2);
        assert!(locs.flat_steps[0].name.is_some());
        assert!(locs.flat_steps[1].name.is_some());
        let second = &locs.flat_steps[1];
        let status_loc = second.assertions.get("status").expect("status loc");
        assert_eq!(status_loc.file, "/abs/path/file.tarn.yaml");
        assert!(status_loc.line > 0);
        assert!(second.assertions.contains_key("duration"));
    }

    #[test]
    fn extract_named_tests_and_headers() {
        let yaml = "\
name: Named
tests:
  group_a:
    steps:
      - name: alpha
        request:
          method: GET
          url: http://localhost/
        assert:
          headers:
            Content-Type: application/json
          body:
            $.user.name: \"Alice\"
";
        let locs = extract(yaml, "f.yaml").expect("extract");
        let group = locs.tests.get("group_a").expect("group_a");
        assert_eq!(group.len(), 1);
        let step = &group[0];
        assert!(step.name.is_some());
        assert!(step.assertions.contains_key("header Content-Type"));
        assert!(step.assertions.contains_key("body $.user.name"));
    }

    #[test]
    fn extract_setup_and_teardown() {
        let yaml = "\
name: Hooks
setup:
  - name: login
    request:
      method: POST
      url: http://localhost/auth
teardown:
  - name: cleanup
    request:
      method: POST
      url: http://localhost/cleanup
steps:
  - name: main
    request:
      method: GET
      url: http://localhost/
";
        let locs = extract(yaml, "f.yaml").expect("extract");
        assert_eq!(locs.setup.len(), 1);
        assert_eq!(locs.teardown.len(), 1);
        assert_eq!(locs.flat_steps.len(), 1);
        assert!(locs.setup[0].name.is_some());
        assert!(locs.teardown[0].name.is_some());
        assert!(locs.flat_steps[0].name.is_some());
    }

    #[test]
    fn extract_include_entries_leave_none_name() {
        let yaml = "\
name: With include
setup:
  - include: ./other.tarn.yaml
  - name: real
    request:
      method: GET
      url: http://localhost/
";
        let locs = extract(yaml, "f.yaml").expect("extract");
        assert_eq!(locs.setup.len(), 2);
        // The include entry has no `name:` key, so `name` should be None.
        assert!(locs.setup[0].name.is_none());
        assert!(locs.setup[1].name.is_some());
    }

    #[test]
    fn extract_redirect_assertions() {
        let yaml = "\
name: Redirects
steps:
  - name: follow
    request:
      method: GET
      url: http://localhost/
    assert:
      redirect:
        url: http://localhost/final
        count: 2
";
        let locs = extract(yaml, "f.yaml").expect("extract");
        let step = &locs.flat_steps[0];
        assert!(step.assertions.contains_key("redirect.url"));
        assert!(step.assertions.contains_key("redirect.count"));
    }

    #[test]
    fn extract_malformed_yaml_returns_none() {
        let yaml = "name: broken\n  bad-indent: true\n  - list-here: oops\n";
        // We don't care whether this is Some or None — just that it
        // never panics on invalid input.
        let _ = extract(yaml, "f.yaml");
    }
}

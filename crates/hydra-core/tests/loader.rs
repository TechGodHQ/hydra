//! Loader-boundary tests for API definition parsing: malformed declarations
//! must fail at `load_api_definition` with an error that names the
//! definition source, because that is the only place the file is in scope.

use hydra_core::load_api_definition;

const BASE: &str = r"
operations:
  - name: status
    description: Read status.
    method: GET
    path: /status
    read: true
    output_type: Status
    parameters: []
    http_error_responses:
      - name: unavailable
        status: 503
        fields:
          - name: error
            type: string
            required: true
";

/// Write a definition whose single error-response field carries the given
/// `const` declaration line, and return its path.
fn write_definition(dir: &std::path::Path, const_line: Option<&str>) -> std::path::PathBuf {
    let path = dir.join("operations.yaml");
    let yaml = const_line.map_or_else(
        || BASE.to_owned(),
        |line| format!("{BASE}            {line}\n"),
    );
    std::fs::write(&path, yaml).expect("write definition fixture");
    path
}

#[test]
fn load_api_definition_rejects_non_string_constants_naming_the_source() {
    for line in [
        "const: 5",
        "const: true",
        "const: null",
        "const:",
        "const: [a]",
    ] {
        let dir = tempfile::tempdir().expect("create temp directory");
        let path = write_definition(dir.path(), Some(line));
        let error =
            load_api_definition(&path).expect_err("malformed constants must fail at load time");
        let rendered = format!("{error:#}");
        assert!(
            rendered.contains("parse API definition from"),
            "error must name the parse step for {line:?}: {rendered}"
        );
        assert!(
            rendered.contains(&path.display().to_string()),
            "error must name the definition source file for {line:?}: {rendered}"
        );
        assert!(
            rendered.contains("fields[0].const"),
            "error must name the offending field for {line:?}: {rendered}"
        );
    }
}

#[test]
fn load_api_definition_keeps_dynamic_and_string_constants() {
    let dir = tempfile::tempdir().expect("create temp directory");

    let definition =
        load_api_definition(write_definition(dir.path(), None)).expect("omitted const key loads");
    assert_eq!(
        definition.operations[0].http_error_responses[0].fields[0].constant,
        None
    );

    for (line, expected) in [("const: unavailable", "unavailable"), (r#"const: """#, "")] {
        let definition = load_api_definition(write_definition(dir.path(), Some(line)))
            .unwrap_or_else(|e| panic!("string declaration {line:?} must load: {e:#}"));
        assert_eq!(
            definition.operations[0].http_error_responses[0].fields[0].constant,
            Some(expected.to_owned()),
            "declaration {line:?} must load as the exact string {expected:?}"
        );
    }
}

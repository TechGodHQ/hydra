//! Loader-boundary tests for declared HTTP-error constants.
//!
//! The typed model cannot represent the malformed `const` forms these tests
//! pin: YAML null is indistinguishable from an omitted key once parsed into
//! `Option<String>`, and `serde_yaml` deserializes unquoted booleans and
//! numbers into strings. The loader's raw-value guard is the only layer that
//! sees those forms, so every case here runs through `load_api_definition`
//! with a real file path and asserts the rejection diagnostic names the
//! path and the field's `const` key.

use std::fs;

use hydra_core::{ApiDefinition, load_api_definition};

/// Minimal but complete definition whose error-response field declares the
/// constant under test. `REQUIRED` and `CONST` are substituted per case.
const DEFINITION: &str = r"
operations:
  - name: read_events
    description: Read an event page through the generated unary route.
    method: GET
    path: /events
    read: true
    output_type: EventPage
    parameters: []
    http_error_responses:
      - name: rate_limited
        status: 429
        fields:
          - name: error
            type: string
            required: REQUIRED
            const: CONST
          - name: retry_hint
            type: string
            required: false
";

/// Load a definition whose `error` field is `required: true` and declares
/// `constant_declaration` as its `const` value. Returns the file path
/// alongside the result so rejection diagnostics can be checked for it.
fn load_required(
    constant_declaration: &str,
) -> (std::path::PathBuf, anyhow::Result<ApiDefinition>) {
    load("true", Some(constant_declaration))
}

/// Load a definition whose `error` field is optional (`required: false`).
fn load_optional(
    constant_declaration: &str,
) -> (std::path::PathBuf, anyhow::Result<ApiDefinition>) {
    load("false", Some(constant_declaration))
}

/// Load a definition whose `error` field omits the `const` key entirely.
fn load_omitted() -> (std::path::PathBuf, anyhow::Result<ApiDefinition>) {
    load("true", None)
}

fn load(
    required: &str,
    constant_declaration: Option<&str>,
) -> (std::path::PathBuf, anyhow::Result<ApiDefinition>) {
    let yaml = constant_declaration.map_or_else(
        || {
            DEFINITION
                .replace("REQUIRED", required)
                .replace("            const: CONST\n", "")
        },
        |declaration| {
            DEFINITION
                .replace("REQUIRED", required)
                .replace("const: CONST", &format!("const: {declaration}"))
        },
    );
    let file = tempfile::NamedTempFile::new().expect("temporary definition file");
    fs::write(file.path(), yaml).expect("write temporary definition");
    let path = file.path().to_owned();
    (path.clone(), load_api_definition(&path))
}

/// The rejection diagnostic for a `const` declaration, rendered as the
/// single message a caller sees from `Error::to_string()`.
fn rejection(result: anyhow::Result<ApiDefinition>) -> String {
    result
        .expect_err("malformed const declarations must be rejected at load")
        .to_string()
}

/// Assert the diagnostic names the definition file path, the field's
/// `const` key, and the operation, response, and field it belongs to.
fn assert_names_definition_and_field(rendered: &str, path: &std::path::Path) {
    assert!(
        rendered.contains(&path.display().to_string()),
        "rejection must name the definition file path: {rendered}"
    );
    assert!(
        rendered.contains("`const`"),
        "rejection must name the field's const key: {rendered}"
    );
    assert!(
        rendered.contains("operation read_events HTTP error response rate_limited field error"),
        "rejection must name the operation, response, and field: {rendered}"
    );
}

#[test]
fn loader_rejects_null_constants_including_unquoted_empty() {
    for declaration in ["null", "~", ""] {
        let (path, result) = load_required(declaration);
        let rendered = rejection(result);
        assert_names_definition_and_field(&rendered, &path);
        assert_eq!(
            rendered,
            format!(
                "{}: operation read_events HTTP error response rate_limited field error \
                 declares `const` with a null value; write a quoted string (an empty \"\" is \
                 allowed) or omit the key to keep the field dynamic",
                path.display()
            ),
            "unquoted empty, null, and ~ are the same YAML null form"
        );
    }
}

#[test]
fn loader_rejects_boolean_and_number_constants() {
    for (declaration, kind) in [
        ("true", "boolean value (true)"),
        ("false", "boolean value (false)"),
        ("42", "number value (42)"),
        ("4.2", "number value (4.2)"),
        ("-3", "number value (-3)"),
    ] {
        let (path, result) = load_required(declaration);
        let rendered = rejection(result);
        assert_names_definition_and_field(&rendered, &path);
        assert!(
            rendered.contains(&format!("declares `const` with a {kind};")),
            "{declaration} rejection must name the offending form: {rendered}"
        );
    }
}

#[test]
fn loader_rejects_list_and_mapping_constants() {
    for (declaration, kind) in [("[unexpected]", "list"), ("{unexpected: value}", "mapping")] {
        let (path, result) = load_required(declaration);
        let rendered = rejection(result);
        assert_names_definition_and_field(&rendered, &path);
        assert!(
            rendered.contains(&format!(
                "declares `const` with a {kind}; `const` must be a string"
            )),
            "{declaration} rejection must name the offending form: {rendered}"
        );
    }
}

#[test]
fn loader_rejects_const_on_optional_fields() {
    for declaration in ["fixed", "\"\""] {
        let (path, result) = load_optional(declaration);
        let rendered = rejection(result);
        assert_names_definition_and_field(&rendered, &path);
        assert!(
            rendered.contains("declares `const` but is optional; constants require required: true"),
            "optional {declaration:?} const rejection must state the required-field rule: {rendered}"
        );
    }
}

#[test]
fn loader_accepts_quoted_and_plain_string_constants() {
    for (declaration, expected) in [
        ("\"rate limited\"", "rate limited"),
        ("\"\"", ""),
        ("\"42\"", "42"),
        ("\"true\"", "true"),
        ("\"null\"", "null"),
        ("'single quoted'", "single quoted"),
        ("rate_limited", "rate_limited"),
    ] {
        let (_, result) = load_required(declaration);
        let definition =
            result.unwrap_or_else(|error| panic!("{declaration:?} must load: {error:#}"));
        assert_eq!(
            definition.operations[0].http_error_responses[0].fields[0]
                .constant
                .as_deref(),
            Some(expected),
            "{declaration:?} must parse as the exact string {expected:?}"
        );
        assert!(
            definition.operations[0].http_error_responses[0].fields[1]
                .constant
                .is_none(),
            "the adjacent optional field stays dynamic without a const key"
        );
    }
}

#[test]
fn loader_keeps_omitted_const_dynamic() {
    let (_, result) = load_omitted();
    let definition = result.expect("an omitted const key loads as a dynamic field");
    assert!(
        definition.operations[0].http_error_responses[0].fields[0]
            .constant
            .is_none(),
        "omitted const must stay dynamic, not become an error"
    );

    let (_, optional) = load("false", None);
    optional.expect("an omitted const on an optional field also loads");
}

#[test]
fn typed_parse_backstop_rejects_supplied_non_string_constants() {
    // Direct typed parsing bypasses the loader's raw-value guard; the
    // deserialization backstop must still refuse a supplied non-string.
    for declaration in ["null", "true", "42", "[unexpected]", "{unexpected: value}"] {
        let yaml = DEFINITION
            .replace("REQUIRED", "true")
            .replace("const: CONST", &format!("const: {declaration}"));
        let error = serde_yaml::from_str::<ApiDefinition>(&yaml)
            .expect_err("{declaration:?} must fail typed parsing");
        let rendered = error.to_string();
        assert!(
            rendered.contains("const"),
            "typed rejection must point at the const key: {rendered}"
        );
        assert!(
            rendered.contains("must be a string when supplied"),
            "typed rejection must state the string rule: {rendered}"
        );
    }

    // Omission still deserializes to a dynamic field.
    let yaml = DEFINITION
        .replace("REQUIRED", "true")
        .replace("            const: CONST\n", "");
    let definition: ApiDefinition = serde_yaml::from_str(&yaml).expect("omission parses");
    assert!(
        definition.operations[0].http_error_responses[0].fields[0]
            .constant
            .is_none()
    );
}

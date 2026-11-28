//! Integration coverage for [`hydra_core::load_api_definition`], the product
//! YAML boundary.
//!
//! HTTP-error `const` declarations are gated on the raw YAML form, where an
//! omitted key, an unquoted empty value, and quoted scalars stay
//! distinguishable in a way typed parsing cannot preserve. Every rejection
//! must name the definition file path and the offending field's `const` key,
//! so these rules are exercised through the loader rather than the semantic
//! `validate_definition` gate, which only sees already-typed values.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static FILE_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Write `yaml` to a unique temp file so rejections can name a real path.
fn definition_file(yaml: &str) -> PathBuf {
    let name = format!(
        "hydra-loader-const-{}-{}.yaml",
        std::process::id(),
        FILE_COUNTER.fetch_add(1, Ordering::SeqCst)
    );
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, yaml).expect("write temporary definition");
    path
}

/// Remove a temp definition; cleanup failures must not mask assertions.
fn discard(path: &PathBuf) {
    let _ = std::fs::remove_file(path);
}

/// Minimal valid single-operation definition whose only error response owns
/// the given `fields` block.
fn definition_with_fields(fields_yaml: &str) -> String {
    format!(
        r"operations:
  - name: read_events
    description: Read an event page.
    method: GET
    path: /events
    read: true
    output_type: EventPage
    parameters: []
    surfaces: [http, cli, mcp]
    http_error_responses:
      - name: rate_limited
        status: 429
        fields:
{fields_yaml}
"
    )
}

/// A required string field declaring `const_line`.
fn required_field_with_const(const_line: &str) -> String {
    format!(
        "          - name: error\n            type: string\n            required: true\n            {const_line}\n"
    )
}

#[test]
fn load_rejects_non_string_const_values() {
    let cases = [
        "const:",
        "const: true",
        "const: 30",
        "const: [boom]",
        "const: {code: boom}",
    ];
    for const_line in cases {
        let path = definition_file(&definition_with_fields(&required_field_with_const(
            const_line,
        )));
        let outcome = hydra_core::load_api_definition(&path);
        discard(&path);
        let error = outcome.expect_err(&format!(
            "const {const_line:?} must be rejected at load time"
        ));
        let message = format!("{error:#}");
        let path_text = path.display().to_string();
        assert!(message.contains(&path_text), "path missing: {message}");
        assert!(message.contains("operation read_events"), "{message}");
        assert!(
            message.contains("HTTP error response rate_limited"),
            "{message}"
        );
        assert!(message.contains("field error"), "{message}");
        assert!(
            message.contains("declares const with a non-string value"),
            "{message}"
        );
        assert!(
            message.contains("const must be a string when supplied"),
            "{message}"
        );
    }
}

#[test]
fn load_rejects_quoted_empty_const_values() {
    let const_line = format!("const: {}{}", '"', '"');
    let path = definition_file(&definition_with_fields(&required_field_with_const(
        &const_line,
    )));
    let outcome = hydra_core::load_api_definition(&path);
    discard(&path);
    let error = outcome.expect_err("an empty string constant must be rejected at load time");
    let message = format!("{error:#}");
    let path_text = path.display().to_string();
    assert!(message.contains(&path_text), "path missing: {message}");
    assert!(message.contains("operation read_events"), "{message}");
    assert!(
        message.contains("HTTP error response rate_limited"),
        "{message}"
    );
    assert!(message.contains("field error"), "{message}");
    assert!(
        message.contains("declares const as an empty string"),
        "{message}"
    );
    assert!(
        message.contains("omit the key to leave the field dynamic"),
        "{message}"
    );
}

#[test]
fn direct_typed_deserialization_rejects_non_string_const_values() {
    let yaml = definition_with_fields(&required_field_with_const("const: true"));
    let error = serde_yaml::from_str::<hydra_core::ApiDefinition>(&yaml)
        .expect_err("direct typed deserialization must preserve strict const parsing");
    assert!(
        error
            .to_string()
            .contains("HTTP error field const must be a string when supplied"),
        "unexpected deserializer diagnostic: {error}"
    );
}

#[test]
fn load_rejects_const_on_optional_fields() {
    let path = definition_file(&definition_with_fields(
        "          - name: hint\n            type: string\n            required: false\n            const: \"backoff\"\n",
    ));
    let outcome = hydra_core::load_api_definition(&path);
    discard(&path);
    let error = outcome.expect_err("const on an optional field must be rejected");
    let message = format!("{error:#}");
    let path_text = path.display().to_string();
    assert!(message.contains(&path_text), "path missing: {message}");
    assert!(message.contains("operation read_events"), "{message}");
    assert!(message.contains("field hint"), "{message}");
    assert!(
        message.contains("declares const but is optional"),
        "{message}"
    );
    assert!(
        message.contains("constants require required: true"),
        "{message}"
    );
}

#[test]
fn load_accepts_nonempty_quoted_scalars() {
    let cases = [
        (r#"const: "true""#, "true"),
        (r#"const: "30""#, "30"),
        (r#"const: "backoff""#, "backoff"),
    ];
    for (const_line, expected) in cases {
        let path = definition_file(&definition_with_fields(&required_field_with_const(
            const_line,
        )));
        let definition = hydra_core::load_api_definition(&path).unwrap_or_else(|error| {
            panic!("quoted scalar {const_line:?} must stay valid declaration data: {error}")
        });
        discard(&path);
        let constant = definition.operations[0].http_error_responses[0].fields[0]
            .constant
            .as_deref()
            .expect("quoted const parses as a string constant");
        assert_eq!(constant, expected);
    }
}

#[test]
fn load_accepts_plain_string_constants_and_keeps_omitted_const_dynamic() {
    // Fixture parity: the committed http-error fixtures use unquoted plain
    // string constants, and optional fields without `const` stay dynamic.
    let path = definition_file(&definition_with_fields(
        "          - name: error\n            type: string\n            required: true\n            const: invalid_replay_cursor\n          - name: oldest_cursor\n            type: string\n            required: false\n",
    ));
    let definition = hydra_core::load_api_definition(&path)
        .expect("plain string constants and dynamic optional fields must load");
    discard(&path);
    let fields = &definition.operations[0].http_error_responses[0].fields;
    assert_eq!(fields[0].constant.as_deref(), Some("invalid_replay_cursor"));
    assert_eq!(fields[1].constant, None);
}

//! CLI-boundary tests for the loader's HTTP-error const gate.
//!
//! `hydra-codegen check` resolves `api/operations.yaml` from the working
//! directory and loads it through `hydra_core::load_api_definition`. These
//! tests run the real binary inside a temporary consumer project so a
//! malformed `const` declaration is rejected with the named diagnostic at
//! the public CLI boundary, before artifact comparison.

use std::{fs, process::Command};

/// A minimal consumer project with one required error field whose `const`
/// declaration is under test.
fn write_project(constant_declaration: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temporary consumer project");
    fs::create_dir(dir.path().join("api")).expect("create api directory");
    fs::write(
        dir.path().join("api/operations.yaml"),
        format!(
            "operations:\n  \
             - name: read_events\n    \
             description: Read an event page through the generated unary route.\n    \
             method: GET\n    \
             path: /events\n    \
             read: true\n    \
             output_type: EventPage\n    \
             parameters: []\n    \
             http_error_responses:\n      \
             - name: rate_limited\n        \
             status: 429\n        \
             fields:\n          \
             - name: error\n            \
             type: string\n            \
             required: true\n            \
             const: {constant_declaration}\n"
        ),
    )
    .expect("write consumer definition");
    dir
}

/// Run `hydra-codegen check` inside `dir`, returning stderr and success.
fn run_check(dir: &tempfile::TempDir) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_hydra-codegen"))
        .arg("check")
        .current_dir(dir.path())
        .output()
        .expect("run hydra-codegen check");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn cli_check_rejects_malformed_const_with_named_diagnostic() {
    let dir = write_project("42");
    let (success, stderr) = run_check(&dir);
    assert!(!success, "a malformed const must fail the CLI check");
    assert!(
        stderr.contains(
            "api/operations.yaml: operation read_events HTTP error response \
rate_limited field error declares `const` with a number value (42)"
        ),
        "the CLI rejection must name the definition path and the field's const key: {stderr}"
    );
}

#[test]
fn cli_check_accepts_quoted_constants_including_empty() {
    for declaration in ["\"rate limited\"", "\"\""] {
        let dir = write_project(declaration);
        let (success, stderr) = run_check(&dir);
        // The definition must load; the project fails later only because no
        // generated artifacts exist yet, which proves the const was accepted.
        assert!(
            !success && stderr.contains("generated artifacts are stale"),
            "{declaration:?} must load through the CLI boundary and reach the \
             artifact comparison: {stderr}"
        );
        assert!(
            !stderr.contains("`const` must be a string"),
            "a quoted string constant must never be rejected: {stderr}"
        );
    }
}

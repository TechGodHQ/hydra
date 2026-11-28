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

/// Run a `hydra-codegen` command inside `dir`, returning success and stderr.
fn run_codegen(dir: &tempfile::TempDir, command: &str) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_hydra-codegen"))
        .arg(command)
        .current_dir(dir.path())
        .output()
        .unwrap_or_else(|err| panic!("run hydra-codegen {command}: {err}"));
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn cli_check_rejects_malformed_const_with_named_diagnostic() {
    let dir = write_project("42");
    let (success, stderr) = run_codegen(&dir, "check");
    assert!(!success, "a malformed const must fail the CLI check");
    assert!(
        stderr.contains("api/operations.yaml"),
        "the CLI rejection must name the definition file: {stderr}"
    );
    assert!(
        stderr.contains("operation read_events HTTP error response rate_limited field error"),
        "the CLI rejection must name the declaration: {stderr}"
    );
    assert!(
        stderr.contains(
            "declares const with a non-string value; const must be a string when supplied"
        ),
        "the CLI rejection must explain the current validation rule: {stderr}"
    );
}

#[test]
fn cli_write_and_check_accept_nonempty_quoted_constants() {
    for declaration in ["\"rate limited\"", "\"42\"", "\"true\""] {
        let dir = write_project(declaration);
        for command in ["write", "check"] {
            let (success, stderr) = run_codegen(&dir, command);
            assert!(
                success,
                "hydra-codegen {command} failed with {declaration:?}: {stderr}"
            );
        }
    }
}

#[test]
fn cli_check_rejects_quoted_empty_constant_with_named_diagnostic() {
    let dir = write_project("\"\"");
    let (success, stderr) = run_codegen(&dir, "check");
    assert!(!success, "the current contract rejects an empty const");
    assert!(
        stderr.contains("api/operations.yaml"),
        "the CLI diagnostic must identify the definition file: {stderr}"
    );
    assert!(
        stderr.contains("operation read_events HTTP error response rate_limited field error"),
        "the CLI diagnostic must identify the declaration: {stderr}"
    );
    assert!(
        stderr.contains("declares const as an empty string"),
        "the CLI diagnostic must explain the current non-empty rule: {stderr}"
    );
}

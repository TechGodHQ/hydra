//! Shared helpers for scanning `{placeholder}` segments in HTTP paths.

use anyhow::Result;

/// Extract ordered path parameter names from a `/resource/{id}` style path.
pub fn path_parameters(path: &str) -> Result<Vec<String>> {
    let mut parameters = Vec::new();
    let mut chars = path.char_indices();
    while let Some((_, ch)) = chars.next() {
        if ch != '{' {
            anyhow::ensure!(ch != '}', "unmatched closing brace in path: {path}");
            continue;
        }

        let mut name = String::new();
        let mut closed = false;
        for (_, inner) in chars.by_ref() {
            if inner == '}' {
                closed = true;
                break;
            }
            anyhow::ensure!(inner != '{', "nested opening brace in path: {path}");
            name.push(inner);
        }
        anyhow::ensure!(closed, "unmatched opening brace in path: {path}");
        anyhow::ensure!(!name.is_empty(), "empty path parameter in path: {path}");
        anyhow::ensure!(
            super::validate::is_valid_identifier(&name),
            "invalid path parameter {{{name}}} in path: {path}"
        );
        parameters.push(name);
    }
    Ok(parameters)
}

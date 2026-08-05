//! Variable expansion for custom slash command templates.
//!
//! Custom commands defined in config may contain `{env:VAR}` and `{file:path}`
//! placeholders that are expanded at dispatch time:
//!
//! - `{env:VAR}` — replaced by the value of the environment variable `VAR`.
//!   If the variable is unset, the placeholder is left literal.
//! - `{file:path}` — replaced by the contents of the file at `path` (relative
//!   to the current working directory). If the file cannot be read, the
//!   placeholder is left literal.

use std::collections::HashMap;

/// Expand `{env:VAR}` and `{file:path}` placeholders in `template`.
///
/// Unknown placeholders and unreadable files/env vars are left literal.
pub fn expand_vars(template: &str, env: &HashMap<String, String>) -> String {
    let mut result = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        // Find the matching closing brace.
        let Some(end_rel) = after.find('}') else {
            // Unclosed brace — keep the remainder literal.
            result.push_str(&rest[start..]);
            return result;
        };
        let inner = &after[..end_rel];
        let placeholder = &rest[start..=start + 1 + end_rel];

        if let Some(var) = inner.strip_prefix("env:") {
            match env.get(var) {
                Some(value) => result.push_str(value),
                None => result.push_str(placeholder),
            }
        } else if let Some(path) = inner.strip_prefix("file:") {
            match std::fs::read_to_string(path) {
                Ok(contents) => result.push_str(contents.trim_end()),
                Err(_) => result.push_str(placeholder),
            }
        } else {
            // Unknown placeholder — keep literal.
            result.push_str(placeholder);
        }

        rest = &after[end_rel + 1..];
    }

    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("USER".to_string(), "lorenzo".to_string());
        m
    }

    #[test]
    fn test_expand_env_var() {
        assert_eq!(expand_vars("hello {env:USER}", &env()), "hello lorenzo");
    }

    #[test]
    fn test_expand_env_var_missing_stays_literal() {
        assert_eq!(
            expand_vars("hello {env:MISSING}", &env()),
            "hello {env:MISSING}"
        );
    }

    #[test]
    fn test_expand_file_var() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("note.txt");
        std::fs::write(&path, "file contents\n").unwrap();
        let tpl = format!("read: {{file:{}}}", path.display());
        assert_eq!(expand_vars(&tpl, &env()), "read: file contents");
    }

    #[test]
    fn test_expand_file_missing_stays_literal() {
        let tpl = "read: {file:/nonexistent/xyz.txt}";
        assert_eq!(expand_vars(tpl, &env()), tpl);
    }

    #[test]
    fn test_unknown_placeholder_stays_literal() {
        assert_eq!(expand_vars("a {foo} b", &env()), "a {foo} b");
    }

    #[test]
    fn test_unclosed_brace_stays_literal() {
        assert_eq!(expand_vars("a {env:USER", &env()), "a {env:USER");
    }

    #[test]
    fn test_multiple_placeholders() {
        assert_eq!(
            expand_vars("{env:USER}-{env:USER}", &env()),
            "lorenzo-lorenzo"
        );
    }

    #[test]
    fn test_plain_text_passthrough() {
        assert_eq!(expand_vars("no vars", &env()), "no vars");
    }
}

//! System-prompt templating.
//!
//! `AgentConfig.system_prompt` may be a template containing `{var}` placeholders
//! that are substituted at runtime with per-model/per-agent values. Supported
//! variables are `{model}`, `{workspace}` and `{tools}`.

use std::collections::HashMap;

/// Render a template by substituting every `{var}` placeholder with the value
/// from `vars`.
///
/// Unknown placeholders (not present in `vars`) are left as-is so that a
/// template can safely reference variables that are not always available.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use anacleto::llm::template::render_template;
///
/// let mut vars = HashMap::new();
/// vars.insert("model".to_string(), "claude-sonnet-4".to_string());
/// vars.insert("workspace".to_string(), "/data/proj".to_string());
///
/// let out = render_template("You are {model} in {workspace}.", &vars);
/// assert_eq!(out, "You are claude-sonnet-4 in /data/proj.");
/// ```
pub fn render_template(template: &str, vars: &HashMap<String, String>) -> String {
    let mut result = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        result.push_str(&rest[..open]);
        rest = &rest[open + 1..];
        if let Some(close) = rest.find('}') {
            let key = &rest[..close];
            if let Some(value) = vars.get(key) {
                result.push_str(value);
            } else {
                // Unknown variable — keep the placeholder literal.
                result.push('{');
                result.push_str(key);
                result.push('}');
            }
            rest = &rest[close + 1..];
        } else {
            // Unclosed `{` — keep the rest literal.
            result.push('{');
            result.push_str(rest);
            return result;
        }
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> HashMap<String, String> {
        let mut v = HashMap::new();
        v.insert("model".to_string(), "claude-sonnet-4".to_string());
        v.insert("workspace".to_string(), "/data/proj".to_string());
        v.insert("tools".to_string(), "read, grep, glob".to_string());
        v
    }

    #[test]
    fn test_substitutes_all_variables() {
        let out = render_template(
            "Model: {model}\nWorkspace: {workspace}\nTools: {tools}",
            &vars(),
        );
        assert_eq!(
            out,
            "Model: claude-sonnet-4\nWorkspace: /data/proj\nTools: read, grep, glob"
        );
    }

    #[test]
    fn test_unknown_variable_left_literal() {
        let out = render_template("Hello {model} and {missing}", &vars());
        assert_eq!(out, "Hello claude-sonnet-4 and {missing}");
    }

    #[test]
    fn test_no_placeholders_passthrough() {
        let out = render_template("plain text", &vars());
        assert_eq!(out, "plain text");
    }

    #[test]
    fn test_empty_template() {
        assert_eq!(render_template("", &vars()), "");
    }

    #[test]
    fn test_unclosed_brace_kept_literal() {
        let out = render_template("prefix {model and more", &vars());
        assert_eq!(out, "prefix {model and more");
    }

    #[test]
    fn test_repeated_variable() {
        let out = render_template("{model} is {model}", &vars());
        assert_eq!(out, "claude-sonnet-4 is claude-sonnet-4");
    }
}

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::hook::HookActionConfig;

use super::types::Skill;

/// Load a skill from a Markdown file with YAML frontmatter.
///
/// Format:
/// ```markdown
/// ---
/// name: my-skill
/// description: Does something useful
/// ---
///
/// Skill instructions here...
/// ```
pub fn load_skill(path: &Path) -> Result<Skill> {
    let contents = std::fs::read_to_string(path).map_err(|e| {
        Error::Skill(format!(
            "Failed to read skill file {}: {}",
            path.display(),
            e
        ))
    })?;

    parse_skill(&contents)
}

/// Parse a skill from a Markdown string with YAML frontmatter.
pub fn parse_skill(content: &str) -> Result<Skill> {
    let content = content.trim();

    // Check for frontmatter delimiters
    if !content.starts_with("---") {
        return Err(Error::Skill(
            "Skill file must start with YAML frontmatter (---)".into(),
        ));
    }

    // Find the closing ---
    let end_frontmatter = content[3..]
        .find("\n---")
        .map(|pos| pos + 3) // +3 for the \n--- we matched
        .ok_or_else(|| Error::Skill("Missing closing --- in frontmatter".into()))?;

    let frontmatter_str = &content[3..end_frontmatter].trim();
    let instructions = content[end_frontmatter + 4..].trim().to_string();

    // Parse frontmatter as YAML
    #[derive(serde::Deserialize)]
    struct Frontmatter {
        name: String,
        description: String,
        #[serde(default)]
        metadata: Option<serde_yaml::Value>,
        #[serde(default)]
        hooks: std::collections::HashMap<String, Vec<HookActionConfig>>,
    }

    let frontmatter: Frontmatter = serde_yaml::from_str(frontmatter_str)
        .map_err(|e| Error::Skill(format!("Invalid frontmatter: {}", e)))?;

    let metadata = extract_string_metadata(frontmatter.metadata);

    Ok(Skill {
        name: frontmatter.name,
        description: frontmatter.description,
        instructions,
        metadata,
        hooks: frontmatter.hooks,
    })
}

/// Extract only string-valued entries from an optional YAML mapping value.
/// Non-string values (maps, sequences, numbers, booleans, null) are silently skipped.
fn extract_string_metadata(
    value: Option<serde_yaml::Value>,
) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if let Some(serde_yaml::Value::Mapping(mapping)) = value {
        for (k, v) in mapping {
            if let (serde_yaml::Value::String(key), serde_yaml::Value::String(val)) = (k, v) {
                map.insert(key, val);
            }
        }
    }
    map
}

/// Load all skills from a directory (non-recursive).
pub fn load_skills_from_dir(dir: &Path) -> Result<Vec<Skill>> {
    let mut skills = Vec::new();

    if !dir.exists() {
        return Ok(skills);
    }

    for entry in std::fs::read_dir(dir)
        .map_err(|e| Error::Skill(format!("Failed to read skill directory: {e}")))?
    {
        let entry = entry.map_err(|e| Error::Skill(format!("Failed to read entry: {e}")))?;
        let path = entry.path();

        if path.file_name().is_some_and(|name| name == "SKILL.md") {
            match load_skill(&path) {
                Ok(skill) => skills.push(skill),
                Err(e) => {
                    eprintln!("Warning: Failed to load skill {}: {}", path.display(), e);
                }
            }
        }
    }

    Ok(skills)
}

/// Load a skill from a path that may be a file or a directory.
pub fn load_single_or_dir(path: &Path) -> Result<Vec<Skill>> {
    if path.is_dir() {
        load_skills_from_dir(path)
    } else if path.is_file() {
        load_skill(path).map(|s| vec![s])
    } else {
        Err(Error::Skill(format!(
            "Skill path does not exist: {}",
            path.display()
        )))
    }
}

/// Load all skills for an agent from its list of skill paths.
pub fn load_agent_skills(paths: &[PathBuf]) -> Vec<Skill> {
    let mut skills = Vec::new();
    for path in paths {
        match load_single_or_dir(path) {
            Ok(mut s) => skills.append(&mut s),
            Err(e) => eprintln!("Warning: {e}"),
        }
    }
    skills
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::HookAction;

    #[test]
    fn test_parse_skill() {
        let content = r#"---
name: code-review
description: Reviews code for quality and correctness
---

Review the provided code and check for:
1. Correctness
2. Performance
3. Style
"#;
        let skill = parse_skill(content).unwrap();
        assert_eq!(skill.name, "code-review");
        assert_eq!(
            skill.description,
            "Reviews code for quality and correctness"
        );
        assert!(skill.instructions.contains("Review the provided code"));
    }

    #[test]
    fn test_parse_skill_no_frontmatter() {
        let content = "Just some text without frontmatter";
        let result = parse_skill(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_skill_with_metadata() {
        let content = r#"---
name: test
description: A test skill
metadata:
  version: "1.0"
  author: test
---

Instructions here
"#;
        let skill = parse_skill(content).unwrap();
        assert_eq!(skill.metadata.get("version").unwrap(), "1.0");
        assert_eq!(skill.metadata.get("author").unwrap(), "test");
    }

    #[test]
    fn test_parse_skill_nested_metadata() {
        let content = r#"---
name: blog-avoid-ai
description: A skill with nested metadata
metadata:
  openclaw:
    emoji: "✍️"
  version: "1.0"
  author: test
  count: 42
  enabled: true
  tags:
    - writing
    - blog
---
Instructions here
"#;
        let skill = parse_skill(content).unwrap();
        // String values are preserved
        assert_eq!(skill.metadata.get("version").unwrap(), "1.0");
        assert_eq!(skill.metadata.get("author").unwrap(), "test");
        // Non-string values (maps, numbers, booleans, sequences) are skipped
        assert!(!skill.metadata.contains_key("openclaw"));
        assert!(!skill.metadata.contains_key("count"));
        assert!(!skill.metadata.contains_key("enabled"));
        assert!(!skill.metadata.contains_key("tags"));
        // Only the 2 string entries
        assert_eq!(skill.metadata.len(), 2);
    }

    #[test]
    fn test_skill_hooks_default_empty() {
        let yaml = r#"---
name: test-skill
description: A test skill
---
Some instructions"#;
        let skill = parse_skill(yaml).unwrap();
        assert!(skill.hooks.is_empty());
    }

    #[test]
    fn test_skill_hooks_parsed() {
        let yaml = r#"---
name: codegraph
description: Sync codegraph
hooks:
  after_apply:
    - type: shell
      command: "codegraph sync"
      timeout_secs: 60
---
Sync codegraph"#;
        let skill = parse_skill(yaml).unwrap();
        assert_eq!(skill.hooks.len(), 1);
        let actions = skill.hooks.get("after_apply").unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0].action {
            HookAction::Shell { command } => assert_eq!(command, "codegraph sync"),
        }
    }
}
